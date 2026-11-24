use serde_json::{json, Value as V};
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{Element, HtmlElement, HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement};
pub fn window() -> web_sys::Window {
    web_sys::window().expect("browser window")
}
pub fn document() -> web_sys::Document {
    window().document().expect("browser document")
}
pub fn get(id: &str) -> Option<Element> {
    document().get_element_by_id(id)
}
pub fn all(selector: &str) -> Vec<Element> {
    document()
        .query_selector_all(selector)
        .map(|l| {
            (0..l.length())
                .filter_map(|i| l.item(i)?.dyn_into().ok())
                .collect()
        })
        .unwrap_or_default()
}
pub fn children(e: &Element) -> Vec<Element> {
    let c = e.children();
    (0..c.length()).filter_map(|i| c.item(i)).collect()
}
pub fn el(tag: &str, class: &str, text: &str) -> Element {
    let e = document().create_element(tag).expect("valid element");
    if !class.is_empty() {
        e.set_class_name(class);
    }
    e.set_text_content(Some(text));
    e
}
pub fn add(parent: &Element, child: &Element) {
    let _ = parent.append_child(child);
}
pub fn append(id: &str, e: &Element) {
    if let Some(p) = get(id) {
        add(&p, e);
    }
}
pub fn clear(id: &str) {
    if let Some(e) = get(id) {
        e.set_text_content(None);
    }
}
pub fn text(id: &str, value: &str) {
    if let Some(e) = get(id) {
        e.set_text_content(Some(value));
    }
}
pub const MODAL_OVERLAY_IDS: [&str; 3] = ["command-palette", "approvals-drawer", "run-file-card"];
pub const EXPANDED_WORKSPACE_IDS: [&str; 3] = [
    "session-graph-workspace",
    "system-map-workspace",
    "system-change-map-workspace",
];
const FALLBACK_RESTORE_SELECTOR: &str = "#view-tabs [aria-selected=\"true\"], #new-session";
const MODAL_FOCUSABLE_SELECTOR: &str = "button:not([disabled]):not([tabindex=\"-1\"]), a[href]:not([tabindex=\"-1\"]), input:not([disabled]):not([type=\"hidden\"]):not([tabindex=\"-1\"]), select:not([disabled]):not([tabindex=\"-1\"]), textarea:not([disabled]):not([tabindex=\"-1\"]), details > summary:not([tabindex=\"-1\"]), [tabindex]:not([tabindex=\"-1\"]):not([disabled])";

pub const MOBILE_BREAKPOINT_QUERY: &str = "(max-width: 920px)";

pub fn is_mobile_viewport() -> bool {
    window()
        .match_media(MOBILE_BREAKPOINT_QUERY)
        .ok()
        .flatten()
        .is_some_and(|list| list.matches())
}

pub(crate) fn should_register_viewport_listener(existing_keys: &[String], guard_key: &str) -> bool {
    !guard_key.is_empty() && !existing_keys.iter().any(|k| k == guard_key)
}

type ResizeCallback = std::rc::Rc<std::cell::RefCell<Box<dyn FnMut()>>>;

#[derive(Default)]
struct ViewportResizeHub {
    wired: bool,
    mobile_media: Option<web_sys::MediaQueryList>,
    listeners: Vec<(String, i32, ResizeCallback)>,
}

thread_local! {
    static VIEWPORT_RESIZE_HUB: std::cell::RefCell<ViewportResizeHub> =
        std::cell::RefCell::new(ViewportResizeHub::default());
}

fn dispatch_viewport_resize() {
    let entries: Vec<(String, i32, ResizeCallback)> = VIEWPORT_RESIZE_HUB.with(|hub| {
        hub.borrow()
            .listeners
            .iter()
            .map(|(k, ms, cb)| (k.clone(), *ms, std::rc::Rc::clone(cb)))
            .collect()
    });
    for (key, debounce_ms, cb) in entries {
        if debounce_ms > 0 {
            later(&format!("viewport-resize:{key}"), debounce_ms, move || {
                if let Ok(mut f) = cb.try_borrow_mut() {
                    f();
                }
            });
        } else if let Ok(mut f) = cb.try_borrow_mut() {
            f();
        }
    }
}

/// Registers an idempotent viewport `resize` + `MOBILE_BREAKPOINT_QUERY` listener on the shared hub.
/// When `debounce_ms > 0`, rapid resize events coalesce via `dom::later`.
pub fn on_viewport_resize(guard_key: &str, debounce_ms: i32, on_resize: impl FnMut() + 'static) {
    let should_wire = VIEWPORT_RESIZE_HUB.with(|hub| {
        let mut hub = hub.borrow_mut();
        let existing: Vec<String> = hub.listeners.iter().map(|(k, _, _)| k.clone()).collect();
        if !should_register_viewport_listener(&existing, guard_key) {
            return false;
        }
        hub.listeners.push((
            guard_key.to_owned(),
            debounce_ms,
            std::rc::Rc::new(std::cell::RefCell::new(Box::new(on_resize))),
        ));
        if hub.wired {
            return false;
        }
        hub.wired = true;
        true
    });
    if !should_wire {
        return;
    }
    if let Ok(Some(media)) = window().match_media(MOBILE_BREAKPOINT_QUERY) {
        let media_cb = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
            dispatch_viewport_resize();
        });
        let _ = media.add_event_listener_with_callback("change", media_cb.as_ref().unchecked_ref());
        media_cb.forget();
        VIEWPORT_RESIZE_HUB.with(|hub| {
            hub.borrow_mut().mobile_media = Some(media);
        });
    }
    let resize_cb = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
        dispatch_viewport_resize();
    });
    if window()
        .add_event_listener_with_callback("resize", resize_cb.as_ref().unchecked_ref())
        .is_ok()
    {
        resize_cb.forget();
    }
}

pub(crate) fn overlay_stack_priority(
    palette_open: bool,
    drawer_open: bool,
    top_dialog_open: bool,
    file_card_open: bool,
    mobile_sidebar_open: bool,
    expanded_ws_open: bool,
) -> Option<&'static str> {
    if palette_open {
        Some("command-palette")
    } else if drawer_open {
        Some("approvals-drawer")
    } else if top_dialog_open {
        Some("dialog")
    } else if file_card_open {
        Some("run-file-card")
    } else if mobile_sidebar_open {
        Some("sidebar")
    } else if expanded_ws_open {
        Some("expanded-workspace")
    } else {
        None
    }
}

fn overlay_inert_targets(
    palette_open: bool,
    drawer_open: bool,
    file_card_open: bool,
    sidebar_open: bool,
    mobile: bool,
) -> [(&'static str, bool); 5] {
    if palette_open {
        [
            ("sidebar", true),
            ("main", true),
            ("approvals-drawer", true),
            ("run-file-card", true),
            ("command-palette", false),
        ]
    } else if drawer_open {
        [
            ("sidebar", true),
            ("main", true),
            ("approvals-drawer", false),
            ("run-file-card", true),
            ("command-palette", false),
        ]
    } else if file_card_open {
        [
            ("sidebar", true),
            ("main", true),
            ("approvals-drawer", false),
            ("run-file-card", false),
            ("command-palette", false),
        ]
    } else if mobile {
        [
            ("sidebar", !sidebar_open),
            ("main", sidebar_open),
            ("approvals-drawer", false),
            ("run-file-card", false),
            ("command-palette", false),
        ]
    } else {
        [
            ("sidebar", false),
            ("main", false),
            ("approvals-drawer", false),
            ("run-file-card", false),
            ("command-palette", false),
        ]
    }
}

pub(crate) fn standalone_body_inert_targets(
    modal_open: bool,
    mobile_sidebar_open: bool,
) -> [(&'static str, bool); 3] {
    if modal_open {
        [
            ("#graph-explain-action", true),
            ("#scrim", true),
            (".skip", true),
        ]
    } else if mobile_sidebar_open {
        [
            ("#graph-explain-action", true),
            ("#scrim", false),
            (".skip", true),
        ]
    } else {
        [
            ("#graph-explain-action", false),
            ("#scrim", false),
            (".skip", false),
        ]
    }
}

fn set_element_inert(e: &Element, inert: bool) {
    let has_inert = e.has_attribute("inert");
    if inert && !has_inert {
        let _ = e.set_attribute("inert", "");
    } else if !inert && has_inert {
        let _ = e.remove_attribute("inert");
    }
}

fn set_inert_attr(id: &str, inert: bool) {
    if let Some(e) = get(id) {
        set_element_inert(&e, inert);
    }
}

fn is_standalone_body_sibling(el: &Element) -> bool {
    el.parent_element()
        .is_some_and(|p| p.tag_name().eq_ignore_ascii_case("BODY"))
}

fn set_selector_inert_attr(selector: &str, inert: bool) {
    for el in all(selector) {
        let effective = if selector == "#graph-explain-action" {
            inert && is_standalone_body_sibling(&el)
        } else {
            inert
        };
        set_element_inert(&el, effective);
    }
}

fn clear_workspace_inert() {
    for el in all("[data-workspace-inert=\"true\"]") {
        let _ = el.remove_attribute("data-workspace-inert");
        let _ = el.remove_attribute("inert");
    }
}

fn active_expanded_workspace() -> Option<Element> {
    EXPANDED_WORKSPACE_IDS.iter().find_map(|id| {
        let el = get(id)?;
        (el.get_attribute("data-expanded").as_deref() == Some("true") && !element_hidden(&el, None))
            .then_some(el)
    })
}

fn apply_workspace_sibling_inert(ws: &Element) {
    let Some(main_el) = get("main") else {
        return;
    };
    let mut cur = ws.clone();
    loop {
        if cur == main_el {
            break;
        }
        let Some(parent) = cur.parent_element() else {
            break;
        };
        for sibling in children(&parent) {
            if sibling != cur && !sibling.has_attribute("inert") {
                let _ = sibling.set_attribute("data-workspace-inert", "true");
                let _ = sibling.set_attribute("inert", "");
            }
        }
        if parent == main_el {
            break;
        }
        cur = parent;
    }
}

pub(crate) fn push_overlay_id(stack: &mut Vec<String>, id: &str) {
    if id.is_empty() {
        return;
    }
    stack.retain(|item| item != id);
    stack.push(id.to_owned());
}

pub(crate) fn remove_overlay_id(stack: &mut Vec<String>, id: &str) {
    if !id.is_empty() {
        stack.retain(|item| item != id);
    }
}

pub(crate) fn top_overlay_from_stack<'a>(
    stack: &'a [String],
    open_ids: &[String],
) -> Option<&'a str> {
    stack
        .iter()
        .rev()
        .find(|id| open_ids.iter().any(|open| open == *id))
        .map(String::as_str)
}

fn push_overlay_stack(id: &str) {
    OVERLAY_STACK.with(|stack| push_overlay_id(&mut stack.borrow_mut(), id));
}

fn remove_overlay_stack(id: &str) {
    OVERLAY_STACK.with(|stack| remove_overlay_id(&mut stack.borrow_mut(), id));
}

fn all_dialog_elements() -> Vec<Element> {
    all("dialog")
}

fn is_dialog_open_el(el: &Element) -> bool {
    let id = el.id();
    el.has_attribute("open")
        || (MODAL_OVERLAY_IDS.contains(&id.as_str()) && !el.has_attribute("hidden"))
}

fn top_open_modal_id() -> Option<String> {
    let dialogs = all_dialog_elements();
    let open_ids: Vec<String> = dialogs
        .iter()
        .filter(|el| is_dialog_open_el(el))
        .map(Element::id)
        .filter(|id| !id.is_empty())
        .collect();
    if open_ids.is_empty() {
        return None;
    }
    let from_stack = OVERLAY_STACK
        .with(|stack| top_overlay_from_stack(&stack.borrow(), &open_ids).map(str::to_owned));
    if from_stack.is_some() {
        return from_stack;
    }
    let has_open = |id: &str| open_ids.iter().any(|open| open == id);
    let fallback_dialog = dialogs
        .iter()
        .rev()
        .find(|el| {
            is_dialog_open_el(el)
                && !matches!(
                    el.id().as_str(),
                    "command-palette" | "approvals-drawer" | "run-file-card"
                )
        })
        .map(Element::id);
    match overlay_stack_priority(
        has_open("command-palette"),
        has_open("approvals-drawer"),
        fallback_dialog.is_some(),
        has_open("run-file-card"),
        false,
        false,
    ) {
        Some("command-palette") => Some("command-palette".into()),
        Some("approvals-drawer") => Some("approvals-drawer".into()),
        Some("dialog") => fallback_dialog,
        Some("run-file-card") => Some("run-file-card".into()),
        _ => None,
    }
}

/// Returns the ID of the top-most open `<dialog>` overlay according to open order.
pub(crate) fn top_open_dialog_id() -> Option<String> {
    top_open_modal_id()
}

/// Synchronizes `inert` attributes across the shell, sidebar, expanded workspaces, and stacked modal overlays.
pub fn sync_overlay_inert() {
    clear_workspace_inert();
    let dialogs = all_dialog_elements();
    let top_modal = top_open_modal_id();
    let sidebar_open = get("sidebar").is_some_and(|e| e.class_list().contains("open"));
    let mobile = is_mobile_viewport();

    if let Some(ref top_id) = top_modal {
        set_inert_attr("sidebar", true);
        set_inert_attr("main", true);
        for (selector, inert) in standalone_body_inert_targets(true, sidebar_open && mobile) {
            set_selector_inert_attr(selector, inert);
        }
        for d in dialogs {
            let should_inert = is_dialog_open_el(&d) && d.id() != *top_id;
            let has_inert = d.has_attribute("inert");
            if should_inert && !has_inert {
                let _ = d.set_attribute("inert", "");
            } else if !should_inert && has_inert {
                let _ = d.remove_attribute("inert");
            }
        }
        return;
    }

    for d in dialogs {
        if d.has_attribute("inert") {
            let _ = d.remove_attribute("inert");
        }
    }

    let expanded_ws = if !(mobile && sidebar_open) {
        active_expanded_workspace()
    } else {
        None
    };

    for (selector, mut inert) in standalone_body_inert_targets(false, sidebar_open && mobile) {
        if selector == ".skip" && expanded_ws.is_some() {
            inert = true;
        }
        set_selector_inert_attr(selector, inert);
    }
    for (id, mut inert) in overlay_inert_targets(false, false, false, sidebar_open, mobile) {
        if expanded_ws.is_some() && id == "sidebar" {
            inert = true;
        }
        set_inert_attr(id, inert);
    }
    if let Some(ws) = expanded_ws {
        apply_workspace_sibling_inert(&ws);
    }
}

pub fn hidden(id: &str, v: bool) {
    hidden_with_restore(id, v, true);
}

/// Shows or hides element `id`, restoring saved opener focus on hide only when `restore_focus` is true.
pub fn hidden_with_restore(id: &str, v: bool, restore_focus: bool) {
    if let Some(e) = get(id) {
        hide_with_restore(&e, v, restore_focus);
    }
}

pub fn hide(e: &Element, v: bool) {
    hide_with_restore(e, v, true);
}

/// Shows or hides element `e`, restoring saved opener focus on hide only when `restore_focus` is true.
pub fn hide_with_restore(e: &Element, v: bool, restore_focus: bool) {
    let id = e.id();
    let is_tracked_modal = MODAL_OVERLAY_IDS.contains(&id.as_str()) || e.tag_name() == "DIALOG";
    let was_open = !e.has_attribute("hidden") || e.has_attribute("open");
    let mut restored_already = false;
    if let Ok(dialog) = e.clone().dyn_into::<web_sys::HtmlDialogElement>() {
        if v {
            remove_overlay_stack(&id);
            let _ = e.set_attribute("hidden", "");
            if !restore_focus {
                clear_saved_focus_for_overlay(&id);
            }
            if dialog.open() {
                mark_suppress_close_restore(&id);
                dialog.close();
                if is_tracked_modal {
                    sync_overlay_inert();
                    if restore_focus {
                        restore_focus_for_overlay(&id);
                    }
                    restored_already = true;
                }
            }
        } else {
            if is_tracked_modal && !dialog.open() {
                save_focus_for_overlay(&id);
            }
            let _ = e.remove_attribute("hidden");
            if !dialog.open() {
                let _ = dialog.show_modal();
            }
            if is_tracked_modal && !id.is_empty() {
                push_overlay_stack(&id);
            }
        }
    } else if v {
        remove_overlay_stack(&id);
        let _ = e.set_attribute("hidden", "");
        if !restore_focus {
            clear_saved_focus_for_overlay(&id);
        }
    } else {
        if MODAL_OVERLAY_IDS.contains(&id.as_str()) && !was_open {
            save_focus_for_overlay(&id);
        }
        let _ = e.remove_attribute("hidden");
        if MODAL_OVERLAY_IDS.contains(&id.as_str()) && !id.is_empty() {
            push_overlay_stack(&id);
        }
    }
    if (is_tracked_modal && !restored_already) || id == "graph-explain-action" {
        sync_overlay_inert();
        if is_tracked_modal && v && restore_focus && was_open {
            restore_focus_for_overlay(&id);
        }
    }
}
pub fn disabled(id: &str, v: bool) {
    if let Some(e) = get(id) {
        if v {
            let _ = e.set_attribute("disabled", "");
        } else {
            let _ = e.remove_attribute("disabled");
        }
    }
}
fn sync_expanded_workspace_modal(el: &Element, id: &str, was_expanded: bool, now_expanded: bool) {
    if was_expanded == now_expanded {
        return;
    }
    if now_expanded {
        save_focus_for_overlay(id);
        if let Some(prev_role) = el.get_attribute("role") {
            let _ = el.set_attribute("data-prev-role", &prev_role);
        } else {
            let _ = el.remove_attribute("data-prev-role");
        }
        let _ = el.set_attribute("role", "dialog");
        let _ = el.set_attribute("aria-modal", "true");
        sync_overlay_inert();
    } else {
        let _ = el.remove_attribute("aria-modal");
        if let Some(prev_role) = el.get_attribute("data-prev-role") {
            let _ = el.set_attribute("role", &prev_role);
            let _ = el.remove_attribute("data-prev-role");
        } else {
            let _ = el.remove_attribute("role");
        }
        sync_overlay_inert();
        restore_focus_for_overlay(id);
    }
}

pub fn attr(id: &str, key: &str, value: &str) {
    if let Some(e) = get(id) {
        let was_expanded =
            key == "data-expanded" && e.get_attribute("data-expanded").as_deref() == Some("true");
        let _ = e.set_attribute(key, value);
        if key == "data-expanded" && EXPANDED_WORKSPACE_IDS.contains(&id) {
            sync_expanded_workspace_modal(&e, id, was_expanded, value == "true");
        }
    }
}
pub fn class(id: &str, key: &str, v: bool) {
    if let Some(e) = get(id) {
        let _ = e.class_list().toggle_with_force(key, v);
        if id == "sidebar" && key == "open" {
            sync_overlay_inert();
        }
    }
}
pub fn value_of(e: &Element) -> String {
    if let Some(x) = e.dyn_ref::<HtmlInputElement>() {
        x.value()
    } else if let Some(x) = e.dyn_ref::<HtmlSelectElement>() {
        x.value()
    } else if let Some(x) = e.dyn_ref::<HtmlTextAreaElement>() {
        x.value()
    } else {
        e.get_attribute("value").unwrap_or_default()
    }
}
pub fn value(id: &str) -> String {
    if crate::model::is_project_field_id(id) {
        if id == "project-switcher" {
            if let Some(e) = get(id) {
                return value_of(&e);
            }
            return crate::model::current_project();
        }
        if let Some(e) = get(id) {
            return value_of(&e);
        }
        let scoped = crate::model::scoped_project(id);
        if !scoped.is_empty() {
            return scoped;
        }
        return get("project-switcher")
            .map(|e| value_of(&e))
            .unwrap_or_default();
    }
    get(id).map(|e| value_of(&e)).unwrap_or_default()
}
pub fn set_value(id: &str, v: &str) {
    if crate::model::is_project_field_id(id) {
        if matches!(id, "project" | "project-switcher") {
            crate::model::set_current_project(v);
            if let Some(e) = get("project-switcher") {
                set_element_value(&e, v);
            }
            if id == "project" {
                if let Some(e) = get(id) {
                    set_element_value(&e, v);
                }
            }
            sync_workspace_project_badge(v);
        } else {
            crate::model::set_scoped_project(id, v);
            if let Some(e) = get(id) {
                set_element_value(&e, v);
            }
        }
        return;
    }
    if let Some(e) = get(id) {
        set_element_value(&e, v);
    }
}
fn sync_workspace_project_badge(project_id: &str) {
    if let Some(badge) = get("workspace-project-badge") {
        let project = crate::model::project(project_id);
        let name = crate::model::text(&project, "name");
        let label = if !name.is_empty() {
            name
        } else if !project_id.is_empty() {
            project_id.to_owned()
        } else {
            "No project".into()
        };
        badge.set_text_content(Some(&label));
        hide(&badge, label == "No project");
    }
}
pub fn set_element_value(e: &Element, v: &str) {
    if let Some(x) = e.dyn_ref::<HtmlInputElement>() {
        x.set_value(v);
    } else if let Some(x) = e.dyn_ref::<HtmlSelectElement>() {
        x.set_value(v);
    } else if let Some(x) = e.dyn_ref::<HtmlTextAreaElement>() {
        x.set_value(v);
    } else {
        let _ = e.set_attribute("value", v);
    }
}
pub fn checked_of(e: &Element) -> bool {
    e.dyn_ref::<HtmlInputElement>()
        .map(|x| x.checked())
        .unwrap_or(false)
}
pub fn checked(id: &str) -> bool {
    get(id).map(|e| checked_of(&e)).unwrap_or(false)
}
pub fn set_checked(id: &str, v: bool) {
    if let Some(e) = get(id).and_then(|e| e.dyn_into::<HtmlInputElement>().ok()) {
        e.set_checked(v);
    }
}
pub fn is_disabled(e: &Element) -> bool {
    e.has_attribute("disabled") || e.closest("[inert]").ok().flatten().is_some()
}
pub fn error(id: &str, v: &str) {
    text(id, v);
    hidden(id, v.is_empty());
}
pub fn focus(id: &str) {
    if let Some(e) = get(id).and_then(|e| e.dyn_into::<HtmlElement>().ok()) {
        let _ = e.focus();
    }
}

thread_local! {
    static OVERLAY_STACK: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static FOCUS_RETURN_STACK: std::cell::RefCell<Vec<(String, Element)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static SUPPRESS_CLOSE_FOCUS_RESTORE: std::cell::RefCell<std::collections::BTreeSet<String>> =
        const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
}

fn clear_saved_focus_for_overlay(overlay_id: &str) {
    if overlay_id.is_empty() {
        return;
    }
    FOCUS_RETURN_STACK.with(|stack| {
        stack.borrow_mut().retain(|(id, _)| id != overlay_id);
    });
}

fn mark_suppress_close_restore(overlay_id: &str) {
    if !overlay_id.is_empty() {
        SUPPRESS_CLOSE_FOCUS_RESTORE.with(|set| {
            set.borrow_mut().insert(overlay_id.to_owned());
        });
    }
}

pub(crate) fn take_suppress_close_restore(
    suppressed: &mut std::collections::BTreeSet<String>,
    overlay_id: &str,
) -> bool {
    !overlay_id.is_empty() && suppressed.remove(overlay_id)
}

pub fn save_focus_for_overlay(overlay_id: &str) {
    let Some(active) = document().active_element() else {
        return;
    };
    if get(overlay_id).is_some_and(|overlay| overlay.contains(Some(&active))) {
        return;
    }
    FOCUS_RETURN_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        stack.retain(|(id, _)| id != overlay_id);
        stack.push((overlay_id.to_owned(), active));
    });
}

pub fn restore_focus_for_overlay(overlay_id: &str) {
    let mut had_saved = false;
    let saved = FOCUS_RETURN_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        let pos = stack.iter().rposition(|(id, _)| id == overlay_id)?;
        had_saved = true;
        Some(stack.remove(pos).1)
    });
    if let Some(el) = saved {
        if el.is_connected() {
            focus_element(&el);
            return;
        }
    }
    let active_lost = document().active_element().is_none_or(|active| {
        active.tag_name() == "BODY"
            || get(overlay_id).is_some_and(|overlay| overlay.contains(Some(&active)))
    });
    if !had_saved && !active_lost {
        return;
    }
    if let Some(fallback) = document()
        .query_selector(FALLBACK_RESTORE_SELECTOR)
        .ok()
        .flatten()
    {
        focus_element(&fallback);
    }
}

pub fn click_outside_element(el: &Element, mouse: &web_sys::MouseEvent) -> bool {
    let rect = el.get_bounding_client_rect();
    let x = f64::from(mouse.client_x());
    let y = f64::from(mouse.client_y());
    if mouse.detail() == 0 && x == 0.0 && y == 0.0 {
        return false;
    }
    x < rect.left() || x > rect.right() || y < rect.top() || y > rect.bottom()
}

pub fn dialog(id: &str, open: bool) {
    dialog_with_restore(id, open, true);
}

/// Opens or closes a `<dialog>` element, restoring opener focus on close only when `restore_focus` is true.
pub(crate) fn dialog_with_restore(id: &str, open: bool, restore_focus: bool) {
    if let Some(d) = get(id).and_then(|e| e.dyn_into::<web_sys::HtmlDialogElement>().ok()) {
        if open {
            let _ = d.remove_attribute("hidden");
            if !d.open() {
                save_focus_for_overlay(id);
                let _ = d.show_modal();
                push_overlay_stack(id);
                if let Some(el) = modal_focusables(d.as_ref()).first() {
                    focus_element(el);
                }
            }
            sync_overlay_inert();
        } else {
            remove_overlay_stack(id);
            let was_open = d.open();
            if was_open {
                mark_suppress_close_restore(id);
                d.close();
            }
            if MODAL_OVERLAY_IDS.contains(&id) {
                let _ = d.set_attribute("hidden", "");
            }
            if !restore_focus {
                clear_saved_focus_for_overlay(id);
            }
            sync_overlay_inert();
            if restore_focus && was_open {
                restore_focus_for_overlay(id);
            }
        }
    }
}

/// Closes any open non-shell `<dialog>` elements (without focus theft) when navigating away.
pub(crate) fn close_other_open_dialogs() {
    for el in all_dialog_elements() {
        let id = el.id();
        if !matches!(
            id.as_str(),
            "command-palette" | "approvals-drawer" | "run-file-card"
        ) && is_dialog_open_el(&el)
        {
            dialog_with_restore(&id, false, false);
        }
    }
    sync_overlay_inert();
}
pub fn validity(id: &str) -> bool {
    get(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlFormElement>().ok())
        .map(|f| f.report_validity())
        .unwrap_or(true)
}
pub fn button(label: &str, action: &str, payload: V) -> Element {
    let e = el("button", "button", label);
    let _ = e.set_attribute("type", "button");
    let _ = e.set_attribute("data-action", action);
    let _ = e.set_attribute("data-payload", &payload.to_string());
    e
}
pub fn check(label: &str, id: &str, value: &str, checked: bool) -> Element {
    let e = el("label", "checkbox", "");
    let i = el("input", "", "");
    let _ = i.set_attribute("type", "checkbox");
    let _ = i.set_attribute("data-action", id);
    let _ = i.set_attribute("value", value);
    if let Some(x) = i.dyn_ref::<HtmlInputElement>() {
        x.set_value(value);
        x.set_checked(checked);
    }
    add(&e, &i);
    add(&e, &el("span", "", label));
    e
}
pub fn options(id: &str, items: &[V], label: impl Fn(&V) -> String, previous: Option<&str>) {
    if get(id).is_none() {
        return;
    }
    let previous = previous.map(str::to_owned).unwrap_or_else(|| value(id));
    clear(id);
    for v in items {
        let e = el("option", "", &label(v));
        let key = crate::model::text(v, "id");
        let _ = e.set_attribute("value", &key);
        if v["available"].as_bool() == Some(false) {
            let _ = e.set_attribute("disabled", "");
        }
        append(id, &e);
    }
    if items.is_empty() {
        let e = el("option", "", "None available");
        let _ = e.set_attribute("value", "");
        append(id, &e);
    }
    if items
        .iter()
        .any(|v| crate::model::text(v, "id") == previous && v["available"].as_bool() != Some(false))
    {
        set_value(id, &previous);
    }
}
pub fn selected_checks(container: &str) -> Vec<V> {
    all(&format!("#{container} input:checked"))
        .iter()
        .map(|e| V::String(value_of(e)))
        .collect()
}
pub fn pretty(id: &str, v: &V) {
    text(id, &serde_json::to_string_pretty(v).unwrap_or_default());
}
pub fn details(title: &str, body: &str) -> Element {
    let d = el("details", "setup-file", "");
    add(&d, &el("summary", "", title));
    add(&d, &el("pre", "knowledge-result", body));
    d
}
pub fn rows(id: &str, values: &[V]) {
    let keyed=values.iter().enumerate().map(|(index,value)|json!({"key":format!("{}:{}",index,crate::model::summary(value)),"value":value})).collect::<Vec<_>>();
    keyed_render(
        id,
        &keyed,
        |v| crate::model::text(v, "key"),
        |row| {
            let v = &row["value"];
            let title = crate::model::summary(v);
            let detail = crate::model::text(v, "diff");
            let body = if detail.is_empty() {
                serde_json::to_string_pretty(v).unwrap_or_default()
            } else {
                detail
            };
            details(&title, &body)
        },
    );
}
pub fn enc(s: &str) -> String {
    js_sys::encode_uri_component(s)
        .as_string()
        .unwrap_or_default()
}
pub fn query(values: &[(&str, String)]) -> String {
    let parts: Vec<_> = values
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}
pub fn storage(key: &str) -> Option<String> {
    window()
        .local_storage()
        .ok()
        .flatten()?
        .get_item(key)
        .ok()
        .flatten()
}
pub fn save(key: &str, value: Option<&str>) {
    if let Ok(Some(s)) = window().local_storage() {
        if let Some(v) = value {
            let _ = s.set_item(key, v);
        } else {
            let _ = s.remove_item(key);
        }
    }
}
pub fn json_storage(key: &str) -> V {
    storage(key)
        .and_then(|v| {
            if v.len() <= 500000 {
                serde_json::from_str(&v).ok()
            } else {
                None
            }
        })
        .unwrap_or(json!({}))
}

/// Reads a per-tab value from `sessionStorage` so drafts stay isolated across tabs.
pub fn session_storage(key: &str) -> Option<String> {
    window()
        .session_storage()
        .ok()
        .flatten()?
        .get_item(key)
        .ok()
        .flatten()
}

/// Writes or deletes a per-tab value in `sessionStorage`.
pub fn save_session(key: &str, value: Option<&str>) {
    if let Ok(Some(s)) = window().session_storage() {
        if let Some(v) = value {
            let _ = s.set_item(key, v);
        } else {
            let _ = s.remove_item(key);
        }
    }
}

/// Parses JSON from per-tab `sessionStorage` with size guard.
pub fn json_session_storage(key: &str) -> V {
    session_storage(key)
        .and_then(|v| {
            if v.len() <= 500000 {
                serde_json::from_str(&v).ok()
            } else {
                None
            }
        })
        .unwrap_or(json!({}))
}

/// Politely announces a status change into an `aria-live` status container.
pub fn announce(id: &str, message: &str) {
    if let Some(el) = get(id) {
        let msg = message.trim();
        el.set_text_content(Some(""));
        if !msg.is_empty() {
            el.set_text_content(Some(msg));
        }
    }
}

/// Parsed URL hash deep-link route (`#/<view>?project=...&session=...&run=...&path=...`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HashRoute {
    pub view: String,
    pub project: String,
    pub session: String,
    pub run: String,
    pub path: String,
}

fn percent_encode_ascii(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xf) as usize] as char);
        }
    }
    out
}

fn percent_decode_bytes(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Parses a `#/<view>?project=...&session=...&run=...&path=...` fragment in pure Rust.
pub fn parse_hash_str(raw: &str) -> HashRoute {
    let trimmed = raw.trim().trim_start_matches('#').trim_start_matches('/');
    let (view_raw, query_raw) = trimmed.split_once('?').unwrap_or((trimmed, ""));
    let mut route = HashRoute {
        view: percent_decode_bytes(view_raw),
        ..Default::default()
    };
    for pair in query_raw.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode_bytes(k);
        let val = percent_decode_bytes(v);
        match key.as_str() {
            "project" => route.project = val,
            "session" => route.session = val,
            "run" => route.run = val,
            "path" => route.path = val,
            _ => {}
        }
    }
    route
}

/// Formats a `HashRoute` back to `#/<view>?...` preserving only non-empty query fields.
pub fn format_hash_str(route: &HashRoute) -> String {
    let view = if route.view.is_empty() {
        "sessions"
    } else {
        route.view.as_str()
    };
    let mut params = Vec::new();
    for (k, v) in [
        ("project", route.project.as_str()),
        ("session", route.session.as_str()),
        ("run", route.run.as_str()),
        ("path", route.path.as_str()),
    ] {
        if !v.is_empty() {
            params.push(format!("{k}={}", percent_encode_ascii(v)));
        }
    }
    if params.is_empty() {
        format!("#/{view}")
    } else {
        format!("#/{view}?{}", params.join("&"))
    }
}

/// Parses a URL search string (`?project=...`) for an optional `project` override.
pub fn parse_search_project(search: &str) -> String {
    let query = search.strip_prefix('?').unwrap_or(search);
    for pair in query.split('&') {
        if let Some((key, val)) = pair.split_once('=') {
            if key == "project" {
                let decoded = percent_decode_bytes(val);
                if !decoded.is_empty() {
                    return decoded;
                }
            }
        }
    }
    String::new()
}

/// Reads and parses the current `location.hash` deep-link route, falling back to `?project=` search param.
pub fn hash_route() -> HashRoute {
    let mut route = parse_hash_str(&window().location().hash().unwrap_or_default());
    if route.project.is_empty() {
        let search_project =
            parse_search_project(&window().location().search().unwrap_or_default());
        if !search_project.is_empty() {
            route.project = search_project;
        }
    }
    route
}

#[allow(dead_code)]
pub fn hash_view() -> String {
    hash_route().view
}

/// Updates browser history hash without duplicating identical entries.
pub fn update_hash_route(route: &HashRoute, push: bool) {
    let target = format_hash_str(route);
    let current = window().location().hash().unwrap_or_default();
    if current == target {
        return;
    }
    if let Ok(history) = window().history() {
        if push {
            let _ = history.push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&target));
        } else {
            let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&target));
        }
    }
}
enum FramePending {
    Raf(i32),
    Timer,
}

struct RetainedCallback<H> {
    token: u64,
    handle: i32,
    closure: H,
}

struct CallbackRegistry<H> {
    next_token: u64,
    active: std::collections::BTreeMap<String, RetainedCallback<H>>,
    retired: Vec<H>,
    executing: usize,
}

impl<H> Default for CallbackRegistry<H> {
    fn default() -> Self {
        Self {
            next_token: 0,
            active: std::collections::BTreeMap::new(),
            retired: Vec::new(),
            executing: 0,
        }
    }
}

impl<H> CallbackRegistry<H> {
    fn drain_retired_if_idle(&mut self) {
        if self.executing == 0 && !self.retired.is_empty() {
            self.retired.clear();
        }
    }

    fn allocate_token(&mut self) -> u64 {
        self.drain_retired_if_idle();
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.next_token
    }

    fn insert_with_token(
        &mut self,
        key: &str,
        token: u64,
        handle: i32,
        closure: H,
    ) -> Option<RetainedCallback<H>> {
        self.drain_retired_if_idle();
        self.active.insert(
            key.to_owned(),
            RetainedCallback {
                token,
                handle,
                closure,
            },
        )
    }

    fn cancel(&mut self, key: &str) -> Option<i32> {
        self.drain_retired_if_idle();
        self.active.remove(key).map(|entry| entry.handle)
    }

    fn begin_fire(&mut self, key: &str, token: u64) -> bool {
        self.drain_retired_if_idle();
        let matches = self
            .active
            .get(key)
            .is_some_and(|entry| entry.token == token);
        if !matches {
            return false;
        }
        let Some(entry) = self.active.remove(key) else {
            return false;
        };
        // Keep the invoked Closure alive until the JS trampoline finishes returning.
        self.retired.push(entry.closure);
        self.executing = self.executing.saturating_add(1);
        true
    }

    fn end_fire(&mut self) {
        self.executing = self.executing.saturating_sub(1);
    }
}

thread_local! {
    static PENDING_FRAMES: std::cell::RefCell<std::collections::BTreeMap<String, FramePending>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    static TIMER_CLOSURES: std::cell::RefCell<CallbackRegistry<Closure<dyn FnMut()>>> =
        std::cell::RefCell::new(CallbackRegistry::default());
    static FRAME_CLOSURES: std::cell::RefCell<CallbackRegistry<Closure<dyn FnMut()>>> =
        std::cell::RefCell::new(CallbackRegistry::default());
}

struct TimerFireGuard;
impl Drop for TimerFireGuard {
    fn drop(&mut self) {
        TIMER_CLOSURES.with(|reg| reg.borrow_mut().end_fire());
    }
}

struct FrameFireGuard;
impl Drop for FrameFireGuard {
    fn drop(&mut self) {
        FRAME_CLOSURES.with(|reg| reg.borrow_mut().end_fire());
    }
}

pub fn later(key: &str, ms: i32, callback: impl FnOnce() + 'static) {
    cancel_timer(key);
    let name = key.to_owned();
    let token = TIMER_CLOSURES.with(|reg| reg.borrow_mut().allocate_token());
    let mut cb = Some(callback);
    let c = Closure::new(move || {
        let should_run = TIMER_CLOSURES.with(|reg| reg.borrow_mut().begin_fire(&name, token));
        if !should_run {
            return;
        }
        let _guard = TimerFireGuard;
        crate::model::write(|s| {
            s.timers.remove(&name);
        });
        if let Some(f) = cb.take() {
            f();
        }
    });
    if let Ok(id) = window()
        .set_timeout_with_callback_and_timeout_and_arguments_0(c.as_ref().unchecked_ref(), ms)
    {
        TIMER_CLOSURES.with(|reg| {
            let _ = reg.borrow_mut().insert_with_token(key, token, id, c);
        });
        crate::model::write(|s| {
            s.timers.insert(key.into(), id);
        });
    }
}
pub fn cancel_timer(key: &str) {
    let registry_id = TIMER_CLOSURES.with(|reg| reg.borrow_mut().cancel(key));
    let model_id = crate::model::write(|s| s.timers.remove(key));
    if let Some(id) = model_id.or(registry_id) {
        window().clear_timeout_with_handle(id);
    }
}

/// Coalesces repeated work under `key` onto the next animation frame (falling back to a short timer while hidden).
pub fn schedule_frame(key: &str, callback: impl FnOnce() + 'static) {
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let already_pending = PENDING_FRAMES.with(|frames| frames.borrow().contains_key(key));
    if already_pending {
        return;
    }
    let name = key.to_owned();
    if document().hidden() {
        PENDING_FRAMES.with(|frames| {
            frames
                .borrow_mut()
                .insert(name.clone(), FramePending::Timer);
        });
        let timer_key = format!("frame:{key}");
        later(&timer_key, 16, move || {
            let active = PENDING_FRAMES.with(|frames| frames.borrow_mut().remove(&name).is_some());
            if active {
                callback();
            }
        });
        return;
    }
    let name_for_cb = name.clone();
    let token = FRAME_CLOSURES.with(|reg| reg.borrow_mut().allocate_token());
    let mut cb = Some(callback);
    let c = Closure::new(move || {
        let should_run =
            FRAME_CLOSURES.with(|reg| reg.borrow_mut().begin_fire(&name_for_cb, token));
        if !should_run {
            return;
        }
        let _guard = FrameFireGuard;
        let active =
            PENDING_FRAMES.with(|frames| frames.borrow_mut().remove(&name_for_cb).is_some());
        if active {
            if let Some(f) = cb.take() {
                f();
            }
        }
    });
    if let Ok(id) = window().request_animation_frame(c.as_ref().unchecked_ref()) {
        FRAME_CLOSURES.with(|reg| {
            let _ = reg.borrow_mut().insert_with_token(&name, token, id, c);
        });
        PENDING_FRAMES.with(|frames| {
            frames.borrow_mut().insert(name, FramePending::Raf(id));
        });
    }
}

/// Cancels any queued frame callback under `key`.
pub fn cancel_frame(key: &str) {
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let pending = PENDING_FRAMES.with(|frames| frames.borrow_mut().remove(key));
    let reg_id = FRAME_CLOSURES.with(|reg| reg.borrow_mut().cancel(key));
    match pending {
        Some(FramePending::Raf(id)) => {
            let _ = window().cancel_animation_frame(id);
        }
        Some(FramePending::Timer) => {
            cancel_timer(&format!("frame:{key}"));
        }
        None => {
            if let Some(id) = reg_id {
                let _ = window().cancel_animation_frame(id);
            }
        }
    }
}
pub fn num(id: &str) -> V {
    let raw = value(id);
    let raw = raw.trim();
    if let Ok(n) = raw.parse::<i64>() {
        return json!(n);
    }
    if let Ok(n) = raw.parse::<u64>() {
        return json!(n);
    }
    let Some(n) = raw.parse::<f64>().ok().filter(|n| n.is_finite()) else {
        return V::Null;
    };
    if n.fract() == 0. && n >= i64::MIN as f64 && n < i64::MAX as f64 {
        return json!(n as i64);
    }
    serde_json::Number::from_f64(n)
        .map(V::Number)
        .unwrap_or(V::Null)
}
fn fnv1a_feed(h1: &mut u64, h2: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h1 ^= u64::from(b);
        *h1 = h1.wrapping_mul(0x0000_0100_0000_01B3);
        *h2 = h2.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ u64::from(b).wrapping_add(0x517C_C1B7_2722_0A95);
    }
}

fn feed_json_value(h1: &mut u64, h2: &mut u64, v: &V) {
    match v {
        V::Null => fnv1a_feed(h1, h2, b"N"),
        V::Bool(true) => fnv1a_feed(h1, h2, b"T"),
        V::Bool(false) => fnv1a_feed(h1, h2, b"F"),
        V::Number(n) => {
            fnv1a_feed(h1, h2, b"#");
            fnv1a_feed(h1, h2, n.to_string().as_bytes());
            fnv1a_feed(h1, h2, b"\x00");
        }
        V::String(s) => {
            fnv1a_feed(h1, h2, b"$");
            fnv1a_feed(h1, h2, &s.len().to_le_bytes());
            fnv1a_feed(h1, h2, s.as_bytes());
        }
        V::Array(arr) => {
            fnv1a_feed(h1, h2, b"[");
            fnv1a_feed(h1, h2, &arr.len().to_le_bytes());
            for item in arr {
                feed_json_value(h1, h2, item);
            }
            fnv1a_feed(h1, h2, b"]");
        }
        V::Object(map) => {
            fnv1a_feed(h1, h2, b"{");
            fnv1a_feed(h1, h2, &map.len().to_le_bytes());
            for (k, val) in map {
                fnv1a_feed(h1, h2, &k.len().to_le_bytes());
                fnv1a_feed(h1, h2, k.as_bytes());
                feed_json_value(h1, h2, val);
            }
            fnv1a_feed(h1, h2, b"}");
        }
    }
}

/// Computes a compact 128-bit structural digest of a JSON value without serializing the full payload into DOM attributes.
pub fn json_signature(v: &V) -> String {
    let mut h1: u64 = 0xCBF2_9CE4_8422_2325;
    let mut h2: u64 = 0x517C_C1B7_2722_0A95;
    feed_json_value(&mut h1, &mut h2, v);
    format!("{h1:016x}{h2:016x}")
}

/// Computes a 128-bit digest of rendered HTML markup for cheap unchanged-DOM skipping.
pub fn markup_signature(html: &str) -> String {
    let mut h1: u64 = 0xCBF2_9CE4_8422_2325;
    let mut h2: u64 = 0x517C_C1B7_2722_0A95;
    fnv1a_feed(&mut h1, &mut h2, &html.len().to_le_bytes());
    fnv1a_feed(&mut h1, &mut h2, html.as_bytes());
    format!("{h1:016x}{h2:016x}")
}

pub(crate) fn should_apply_markup(current_sig: Option<&str>, next_sig: &str) -> bool {
    current_sig != Some(next_sig)
}

/// Replaces `el`'s inner HTML only when its markup digest changed, preserving DOM nodes and `<details>` state otherwise.
pub fn set_markup(el: &Element, html: &str) -> bool {
    let sig = markup_signature(html);
    let current = el.get_attribute("data-markup-sig");
    if !should_apply_markup(current.as_deref(), &sig) {
        return false;
    }
    el.set_inner_html(html);
    let _ = el.set_attribute("data-markup-sig", &sig);
    true
}

/// Replaces `#id`'s inner HTML only when its markup digest changed.
pub fn set_markup_id(id: &str, html: &str) -> bool {
    let Some(el) = get(id) else {
        return false;
    };
    set_markup(&el, html)
}

pub(crate) fn should_remove_keyless_child(tag_name: &str, has_data_key: bool) -> bool {
    !has_data_key && !tag_name.eq_ignore_ascii_case("TEMPLATE")
}

pub fn keyed_render(
    id: &str,
    items: &[V],
    key_of: impl Fn(&V) -> String,
    mut build: impl FnMut(&V) -> Element,
) {
    let Some(container) = get(id) else { return };
    let mut previous = std::collections::BTreeMap::new();
    for e in children(&container) {
        if let Some(key) = e.get_attribute("data-key") {
            previous.insert(key, e);
        } else if should_remove_keyless_child(&e.tag_name(), false) {
            e.remove();
        }
    }
    for (index, item) in items.iter().enumerate() {
        let key = key_of(item);
        let signature = json_signature(item);
        let old = previous.remove(&key);
        let node = if old
            .as_ref()
            .and_then(|e| e.get_attribute("data-signature"))
            .as_deref()
            == Some(signature.as_str())
        {
            old.expect("existing keyed node")
        } else {
            let fresh = build(item);
            let _ = fresh.set_attribute("data-key", &key);
            let _ = fresh.set_attribute("data-signature", &signature);
            if let Some(old) = old {
                if old.tag_name() == "DETAILS" && old.has_attribute("open") {
                    let _ = fresh.set_attribute("open", "");
                }
                let old_open = old
                    .query_selector_all("details[open]")
                    .ok()
                    .map(|nodes| {
                        (0..nodes.length())
                            .filter_map(|i| nodes.item(i)?.dyn_into::<Element>().ok())
                            .filter_map(|e| e.get_attribute("data-restore-key"))
                            .collect::<std::collections::BTreeSet<_>>()
                    })
                    .unwrap_or_default();
                if let Ok(nodes) = fresh.query_selector_all("details") {
                    for i in 0..nodes.length() {
                        if let Some(e) = nodes.item(i).and_then(|v| v.dyn_into::<Element>().ok()) {
                            if e.get_attribute("data-restore-key")
                                .is_some_and(|k| old_open.contains(&k))
                            {
                                let _ = e.set_attribute("open", "");
                            }
                        }
                    }
                }
                let focus = document()
                    .active_element()
                    .filter(|f| old.contains(Some(f)))
                    .and_then(|f| f.get_attribute("data-focus-key"));
                let _ = container.replace_child(&fresh, &old);
                if let Some(key) = focus {
                    if let Ok(nodes) = fresh.query_selector_all("[data-focus-key]") {
                        for i in 0..nodes.length() {
                            if let Some(e) =
                                nodes.item(i).and_then(|v| v.dyn_into::<Element>().ok())
                            {
                                if e.get_attribute("data-focus-key").as_deref()
                                    == Some(key.as_str())
                                {
                                    if let Some(e) = e.dyn_ref::<HtmlElement>() {
                                        let _ = e.focus();
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            fresh
        };
        let current = container.children().item(index as u32);
        if current.as_ref() != Some(&node) {
            let _ = container.insert_before(&node, current.as_ref().map(|e| e.as_ref()));
        }
    }
    for node in previous.into_values() {
        node.remove();
    }
}

/// Renders keyed items into `#id`, or a keyed `__empty__` placeholder when `items` is empty.
pub fn keyed_render_or_empty(
    id: &str,
    items: &[V],
    key_of: impl Fn(&V) -> String,
    build: impl FnMut(&V) -> Element,
    empty_tag: &str,
    empty_class: &str,
    empty_text: &str,
) {
    if items.is_empty() {
        let placeholder = [json!({
            "__empty__": true,
            "tag": empty_tag,
            "class": empty_class,
            "text": empty_text,
        })];
        keyed_render(
            id,
            &placeholder,
            |_| "__empty__".into(),
            |_| el(empty_tag, empty_class, empty_text),
        );
    } else {
        keyed_render(id, items, key_of, build);
    }
}

pub fn numeric_valid(id: &str) -> bool {
    get(id)
        .and_then(|e| e.dyn_into::<HtmlInputElement>().ok())
        .map(|e| e.check_validity())
        .unwrap_or(true)
}

pub(crate) fn next_roving_index(
    current: usize,
    len: usize,
    key: &str,
    vertical: bool,
    wrap: bool,
) -> Option<usize> {
    if len == 0 || current >= len {
        return None;
    }
    let step = |forward: bool| {
        if forward {
            if current + 1 < len {
                current + 1
            } else if wrap {
                0
            } else {
                len - 1
            }
        } else if current > 0 {
            current - 1
        } else if wrap {
            len - 1
        } else {
            0
        }
    };
    match key {
        "Home" => Some(0),
        "End" => Some(len - 1),
        "ArrowRight" if !vertical => Some(step(true)),
        "ArrowLeft" if !vertical => Some(step(false)),
        "ArrowDown" => Some(step(true)),
        "ArrowUp" => Some(step(false)),
        _ => None,
    }
}

pub(crate) fn next_trap_index(current: Option<usize>, len: usize, shift: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match (current, shift) {
        (None, true) => Some(len - 1),
        (None, false) => Some(0),
        (Some(0), true) => Some(len - 1),
        (Some(idx), true) => Some(idx.saturating_sub(1)),
        (Some(idx), false) if idx + 1 >= len => Some(0),
        (Some(idx), false) => Some(idx + 1),
    }
}

#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub(crate) fn is_computed_hidden(display: &str, visibility: &str, check_visibility: bool) -> bool {
    display == "none" || (check_visibility && matches!(visibility, "hidden" | "collapse"))
}

fn element_hidden(e: &Element, stop_at: Option<&Element>) -> bool {
    let mut cur = Some(e.clone());
    while let Some(node) = cur {
        if node.has_attribute("hidden") {
            return true;
        }
        #[cfg(target_arch = "wasm32")]
        if node.is_connected() {
            if let Ok(Some(style)) = window().get_computed_style(&node) {
                let display = style.get_property_value("display").unwrap_or_default();
                let visibility = style.get_property_value("visibility").unwrap_or_default();
                if is_computed_hidden(&display, &visibility, &node == e) {
                    return true;
                }
            }
        }
        if let Some(stop) = stop_at {
            if &node == stop {
                break;
            }
        }
        cur = node.parent_element();
    }
    false
}

fn inside_closed_details(e: &Element, stop_at: &Element) -> bool {
    let mut cur = e.parent_element();
    let mut child = e.clone();
    while let Some(parent) = cur {
        if parent.tag_name() == "DETAILS" && !parent.has_attribute("open") {
            let is_direct_summary =
                child.tag_name() == "SUMMARY" && child.parent_element().as_ref() == Some(&parent);
            if !is_direct_summary {
                return true;
            }
        }
        if &parent == stop_at {
            break;
        }
        child = parent.clone();
        cur = parent.parent_element();
    }
    false
}

fn tree_item_collapsed(e: &Element, tree: &Element) -> bool {
    inside_closed_details(e, tree)
}

fn query_elements(root: &Element, selector: &str) -> Vec<Element> {
    root.query_selector_all(selector)
        .ok()
        .map(|list| {
            (0..list.length())
                .filter_map(|i| list.item(i)?.dyn_into::<Element>().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn widget_items(container: &Element, role: &str) -> Vec<Element> {
    let selector = match role {
        "tablist" => "[role=\"tab\"]",
        "radiogroup" => "[role=\"radio\"]",
        "toolbar" => "button, a[href], [role=\"button\"], select, input:not([type=\"hidden\"])",
        "tree" => "[role=\"treeitem\"]",
        _ => return Vec::new(),
    };
    query_elements(container, selector)
        .into_iter()
        .filter(|el| {
            !is_disabled(el)
                && !element_hidden(el, Some(container))
                && (role != "tree" || !tree_item_collapsed(el, container))
        })
        .collect()
}

fn set_roving_active(items: &[Element], active_index: usize) {
    for (idx, item) in items.iter().enumerate() {
        let value = if idx == active_index { "0" } else { "-1" };
        if item.get_attribute("tabindex").as_deref() != Some(value) {
            let _ = item.set_attribute("tabindex", value);
        }
    }
}

pub fn sync_tree_expanded(tree: &Element) {
    for folder in query_elements(tree, "details.run-tree-folder") {
        let open = folder.has_attribute("open");
        if let Ok(Some(summary)) = folder.query_selector(":scope > summary") {
            let expanded = if open { "true" } else { "false" };
            if summary.get_attribute("aria-expanded").as_deref() != Some(expanded) {
                let _ = summary.set_attribute("aria-expanded", expanded);
            }
        }
    }
}

pub fn sync_roving_widget(container: &Element) {
    let Some(role) = container.get_attribute("role") else {
        return;
    };
    if role == "tree" {
        sync_tree_expanded(container);
    }
    let items = widget_items(container, &role);
    if items.is_empty() {
        return;
    }
    let active = match role.as_str() {
        "tablist" => items
            .iter()
            .position(|e| e.get_attribute("aria-selected").as_deref() == Some("true")),
        "radiogroup" => items
            .iter()
            .position(|e| e.get_attribute("aria-checked").as_deref() == Some("true")),
        _ => None,
    }
    .or_else(|| {
        items
            .iter()
            .position(|e| e.get_attribute("tabindex").as_deref() == Some("0"))
    })
    .unwrap_or(0);
    set_roving_active(&items, active);
    if role == "tree" {
        for btn in query_elements(
            container,
            ".run-row-actions > button:not([role=\"treeitem\"])",
        ) {
            if btn.get_attribute("tabindex").as_deref() != Some("-1") {
                let _ = btn.set_attribute("tabindex", "-1");
            }
        }
    }
}

fn focus_element(el: &Element) {
    if let Some(html) = el.dyn_ref::<HtmlElement>() {
        let _ = html.focus();
    }
}

fn modal_focusables(card: &Element) -> Vec<Element> {
    query_elements(card, MODAL_FOCUSABLE_SELECTOR)
        .into_iter()
        .filter(|el| {
            !is_disabled(el) && !element_hidden(el, Some(card)) && !inside_closed_details(el, card)
        })
        .collect()
}

fn active_overlay_container() -> Option<Element> {
    if let Some(id) = top_open_modal_id() {
        return get(&id);
    }
    let mobile_sidebar_el = is_mobile_viewport()
        .then(|| get("sidebar").filter(|el| el.class_list().contains("open")))
        .flatten();
    if mobile_sidebar_el.is_some() {
        return mobile_sidebar_el;
    }
    active_expanded_workspace()
}

pub(crate) fn collapse_active_expanded_workspace() -> bool {
    if active_overlay_container()
        .is_some_and(|el| !EXPANDED_WORKSPACE_IDS.contains(&el.id().as_str()))
    {
        return false;
    }
    let Some(ws) = active_expanded_workspace() else {
        return false;
    };
    let selector = match ws.id().as_str() {
        "session-graph-workspace" => "#session-graph-expand, [data-graph-id=\"expand\"]",
        "system-map-workspace" => "#system-map-expand",
        "system-change-map-workspace" => "#system-change-topology-expand",
        _ => return false,
    };
    if let Ok(Some(btn)) = ws.query_selector(selector) {
        if let Some(html) = btn.dyn_ref::<HtmlElement>() {
            html.click();
            return true;
        }
    }
    false
}

pub(crate) fn trap_tab_in_container(container: &Element, event: &web_sys::KeyboardEvent) -> bool {
    if event.key() != "Tab" || event.ctrl_key() || event.meta_key() || event.alt_key() {
        return false;
    }
    let items = modal_focusables(container);
    if items.is_empty() {
        focus_element(container);
        return true;
    }
    let current = document()
        .active_element()
        .and_then(|active| items.iter().position(|el| el == &active));
    if let Some(next) = next_trap_index(current, items.len(), event.shift_key()) {
        focus_element(&items[next]);
    }
    true
}

fn handle_modal_trap(event: &web_sys::KeyboardEvent) -> bool {
    if event.key() != "Tab" || event.ctrl_key() || event.meta_key() || event.alt_key() {
        return false;
    }
    let Some(container) = active_overlay_container() else {
        return false;
    };
    trap_tab_in_container(&container, event)
}

fn handle_tree_keydown(tree: &Element, target: &Element, key: &str) -> bool {
    let items = widget_items(tree, "tree");
    if items.is_empty() {
        return false;
    }
    let row_button = target
        .closest(".run-row-actions")
        .ok()
        .flatten()
        .filter(|row| tree.contains(Some(row)));
    let current_item = if target.get_attribute("role").as_deref() == Some("treeitem") {
        Some(target.clone())
    } else {
        row_button
            .as_ref()
            .and_then(|row| row.query_selector("[role=\"treeitem\"]").ok().flatten())
    };
    let Some(current_el) = current_item else {
        return false;
    };
    let Some(current_idx) = items.iter().position(|e| e == &current_el) else {
        return false;
    };
    let on_secondary = target != &current_el;
    match key {
        "ArrowDown" | "ArrowUp" | "Home" | "End" => {
            let Some(next) = next_roving_index(current_idx, items.len(), key, true, false) else {
                return false;
            };
            set_roving_active(&items, next);
            focus_element(&items[next]);
            true
        }
        "ArrowRight" => {
            if current_el.tag_name() == "SUMMARY" {
                if let Some(details) = current_el
                    .parent_element()
                    .filter(|p| p.tag_name() == "DETAILS")
                {
                    if !details.has_attribute("open") {
                        let _ = details.set_attribute("open", "");
                        sync_tree_expanded(tree);
                        return true;
                    }
                    let updated = widget_items(tree, "tree");
                    if let Some(pos) = updated.iter().position(|e| e == &current_el) {
                        if pos + 1 < updated.len() && details.contains(Some(&updated[pos + 1])) {
                            set_roving_active(&updated, pos + 1);
                            focus_element(&updated[pos + 1]);
                            return true;
                        }
                    }
                }
                return false;
            }
            if !on_secondary {
                if let Some(row) = row_button {
                    let extras =
                        query_elements(&row, "button:not([role=\"treeitem\"]):not([disabled])");
                    if let Some(next_btn) =
                        extras.first().filter(|e| !element_hidden(e, Some(tree)))
                    {
                        focus_element(next_btn);
                        return true;
                    }
                }
            }
            false
        }
        "ArrowLeft" => {
            if on_secondary {
                set_roving_active(&items, current_idx);
                focus_element(&current_el);
                return true;
            }
            if current_el.tag_name() == "SUMMARY" {
                if let Some(details) = current_el
                    .parent_element()
                    .filter(|p| p.tag_name() == "DETAILS")
                {
                    if details.has_attribute("open") {
                        let _ = details.remove_attribute("open");
                        sync_tree_expanded(tree);
                        set_roving_active(&items, current_idx);
                        return true;
                    }
                }
                return false;
            }
            if let Ok(Some(details)) = current_el.closest("details.run-tree-folder") {
                if let Ok(Some(summary)) =
                    details.query_selector(":scope > summary[role=\"treeitem\"]")
                {
                    if let Some(pos) = items.iter().position(|e| e == &summary) {
                        set_roving_active(&items, pos);
                        focus_element(&summary);
                        return true;
                    }
                }
            }
            false
        }
        _ => false,
    }
}

fn handle_widget_keydown(event: &web_sys::KeyboardEvent) -> bool {
    if event.default_prevented()
        || event.is_composing()
        || event.ctrl_key()
        || event.meta_key()
        || event.alt_key()
    {
        return false;
    }
    let Some(target) = event.target().and_then(|v| v.dyn_into::<Element>().ok()) else {
        return false;
    };
    if matches!(target.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT") {
        return false;
    }
    let Some(container) = target
        .closest("[role=\"tablist\"],[role=\"toolbar\"],[role=\"radiogroup\"],[role=\"tree\"]")
        .ok()
        .flatten()
    else {
        let Some(nav) = target.closest("nav.navigation").ok().flatten() else {
            return false;
        };
        let key = event.key();
        let items: Vec<Element> = query_elements(&nav, ".nav-item")
            .into_iter()
            .filter(|el| !is_disabled(el) && !element_hidden(el, Some(&nav)))
            .collect();
        if items.is_empty() {
            return false;
        }
        let current_el = target
            .closest(".nav-item")
            .ok()
            .flatten()
            .filter(|el| nav.contains(Some(el)))
            .unwrap_or_else(|| target.clone());
        let Some(index) = items.iter().position(|el| el == &current_el) else {
            return false;
        };
        if matches!(key.as_str(), "ArrowLeft" | "ArrowRight") {
            return false;
        }
        let Some(next) = next_roving_index(index, items.len(), &key, true, true) else {
            return false;
        };
        focus_element(&items[next]);
        return true;
    };
    let role = container.get_attribute("role").unwrap_or_default();
    let key = event.key();
    if role == "tree" {
        return handle_tree_keydown(&container, &target, &key);
    }
    let items = widget_items(&container, &role);
    if items.is_empty() {
        return false;
    }
    let item_selector = match role.as_str() {
        "tablist" => "[role=\"tab\"]",
        "radiogroup" => "[role=\"radio\"]",
        _ => "button, a[href], [role=\"button\"]",
    };
    let current_el = target
        .closest(item_selector)
        .ok()
        .flatten()
        .filter(|el| container.contains(Some(el)))
        .unwrap_or_else(|| target.clone());
    let Some(index) = items.iter().position(|el| el == &current_el) else {
        return false;
    };
    let vertical = container.get_attribute("aria-orientation").as_deref() == Some("vertical");
    let Some(next) = next_roving_index(index, items.len(), &key, vertical, true) else {
        return false;
    };
    set_roving_active(&items, next);
    focus_element(&items[next]);
    if matches!(role.as_str(), "tablist" | "radiogroup") {
        if let Some(html) = items[next].dyn_ref::<HtmlElement>() {
            html.click();
        }
    }
    true
}

pub(crate) fn sync_widget_from_target(target: &Element) {
    let Some(container) = target
        .closest("[role=\"tablist\"],[role=\"toolbar\"],[role=\"radiogroup\"],[role=\"tree\"]")
        .ok()
        .flatten()
    else {
        return;
    };
    let role = container.get_attribute("role").unwrap_or_default();
    let items = widget_items(&container, &role);
    if items.is_empty() {
        return;
    }
    let selector = match role.as_str() {
        "tablist" => "[role=\"tab\"]",
        "radiogroup" => "[role=\"radio\"]",
        "tree" => "[role=\"treeitem\"]",
        _ => "button, a[href], [role=\"button\"], select, input:not([type=\"hidden\"])",
    };
    if let Ok(Some(item)) = target.closest(selector) {
        if let Some(pos) = items.iter().position(|el| el == &item) {
            set_roving_active(&items, pos);
        }
    }
}

/// Handles modal `Tab` focus trapping and roving keyboard navigation for ARIA widgets.
pub(crate) fn handle_a11y_keydown(event: &web_sys::KeyboardEvent) -> bool {
    handle_modal_trap(event) || handle_widget_keydown(event)
}

/// Re-syncs roving tabindex when a `<details>` tree node is toggled open or closed.
pub(crate) fn handle_a11y_toggle(target: &Element) {
    if let Ok(Some(tree)) = target.closest("[role=\"tree\"]") {
        sync_roving_widget(&tree);
    }
}

fn should_apply_dialog_close(dialog_still_open: bool) -> bool {
    !dialog_still_open
}

/// Keeps `hidden`, `inert`, and focus restoration synchronized when a `<dialog>` closes.
pub(crate) fn handle_a11y_dialog_close(target: &Element) {
    if target.tag_name() != "DIALOG" {
        return;
    }
    let id = target.id();
    let suppressed = SUPPRESS_CLOSE_FOCUS_RESTORE
        .with(|set| take_suppress_close_restore(&mut set.borrow_mut(), &id));
    let dialog_still_open = target
        .dyn_ref::<web_sys::HtmlDialogElement>()
        .map_or_else(|| target.has_attribute("open"), |dialog| dialog.open());
    // Native <dialog>.close() queues the `close` event asynchronously; ignore stale
    // events that arrive after the same dialog has already been reopened.
    if !should_apply_dialog_close(dialog_still_open) {
        return;
    }
    remove_overlay_stack(&id);
    if MODAL_OVERLAY_IDS.contains(&id.as_str()) {
        let _ = target.set_attribute("hidden", "");
    }
    sync_overlay_inert();
    if !id.is_empty() && !suppressed {
        restore_focus_for_overlay(&id);
    }
}

/// Synchronizes overlay `inert` attributes and roving widget `tabindex` states in the DOM.
pub fn sync_a11y_widgets() {
    sync_overlay_inert();
    for widget in all("[role=\"tablist\"],[role=\"toolbar\"],[role=\"radiogroup\"],[role=\"tree\"]")
    {
        sync_roving_widget(&widget);
    }
}

/// Strips Unicode bidirectional override control characters from text.
pub fn clean_bidi(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(
                *c,
                '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}'
            )
        })
        .collect()
}

/// Strips bidirectional override controls and escapes `&`, `<`, `>`, `"`, `'` for HTML interpolation.
pub fn escape_html(raw: &str) -> String {
    let cleaned = clean_bidi(raw);
    let mut out = String::with_capacity(cleaned.len());
    for ch in cleaned.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escapes a value for safe interpolation inside a CSS attribute selector (`[attr="..."]`).
pub fn escape_css_attr_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\a "),
            '\r' => out.push_str("\\d "),
            '\t' => out.push_str("\\9 "),
            '\0' => out.push('\u{fffd}'),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roving_index_handles_horizontal_vertical_wrapping_and_boundaries() {
        assert_eq!(next_roving_index(0, 3, "ArrowRight", false, true), Some(1));
        assert_eq!(next_roving_index(2, 3, "ArrowRight", false, true), Some(0));
        assert_eq!(next_roving_index(0, 3, "ArrowLeft", false, true), Some(2));
        assert_eq!(next_roving_index(2, 3, "Home", false, true), Some(0));
        assert_eq!(next_roving_index(0, 3, "End", false, true), Some(2));
        assert_eq!(next_roving_index(1, 3, "ArrowUp", false, true), Some(0));
        assert_eq!(next_roving_index(1, 3, "ArrowDown", false, true), Some(2));
        // Vertical non-wrapping tree behavior
        assert_eq!(next_roving_index(0, 3, "ArrowUp", true, false), Some(0));
        assert_eq!(next_roving_index(2, 3, "ArrowDown", true, false), Some(2));
        assert_eq!(next_roving_index(1, 3, "ArrowLeft", true, false), None);
        assert_eq!(next_roving_index(0, 0, "ArrowRight", false, true), None);
    }

    #[test]
    fn modal_trap_index_cycles_forward_and_backward() {
        assert_eq!(next_trap_index(None, 0, false), None);
        assert_eq!(next_trap_index(None, 3, false), Some(0));
        assert_eq!(next_trap_index(None, 3, true), Some(2));
        assert_eq!(next_trap_index(Some(0), 3, false), Some(1));
        assert_eq!(next_trap_index(Some(2), 3, false), Some(0));
        assert_eq!(next_trap_index(Some(0), 3, true), Some(2));
        assert_eq!(next_trap_index(Some(2), 3, true), Some(1));
        assert_eq!(next_trap_index(Some(0), 1, false), Some(0));
        assert_eq!(next_trap_index(Some(0), 1, true), Some(0));
    }

    #[test]
    fn hash_route_parses_and_formats_all_fields() {
        let parsed =
            parse_hash_str("#/sessions?project=proj%201&session=s_2&run=r%2F9&path=src%2Flib.rs");
        assert_eq!(parsed.view, "sessions");
        assert_eq!(parsed.project, "proj 1");
        assert_eq!(parsed.session, "s_2");
        assert_eq!(parsed.run, "r/9");
        assert_eq!(parsed.path, "src/lib.rs");

        let encoded = format_hash_str(&parsed);
        assert_eq!(parse_hash_str(&encoded), parsed);

        assert_eq!(
            format_hash_str(&HashRoute {
                view: "systems".into(),
                ..Default::default()
            }),
            "#/systems"
        );
    }

    #[test]
    fn parse_search_project_and_vertical_nav_roving_wrap_cleanly() {
        assert_eq!(
            parse_search_project("?view=sessions&project=my%20app"),
            "my app"
        );
        assert_eq!(parse_search_project("foo=bar"), "");
        assert_eq!(next_roving_index(0, 6, "ArrowUp", true, true), Some(5));
        assert_eq!(next_roving_index(5, 6, "ArrowDown", true, true), Some(0));
        assert_eq!(next_roving_index(2, 6, "ArrowRight", true, true), None);
    }

    #[test]
    fn overlay_focus_and_inert_policies_match_viewport_and_stacking() {
        assert!(FALLBACK_RESTORE_SELECTOR.contains("#new-session"));
        assert!(!FALLBACK_RESTORE_SELECTOR.contains("#new-session-button"));
        assert!(MODAL_FOCUSABLE_SELECTOR.contains("details > summary:not([tabindex=\"-1\"])"));

        // Desktop: open sidebar does not inert main or sidebar.
        assert_eq!(
            overlay_inert_targets(false, false, false, true, false),
            [
                ("sidebar", false),
                ("main", false),
                ("approvals-drawer", false),
                ("run-file-card", false),
                ("command-palette", false),
            ]
        );
        // Mobile closed sidebar: sidebar inert, main active.
        assert_eq!(
            overlay_inert_targets(false, false, false, false, true),
            [
                ("sidebar", true),
                ("main", false),
                ("approvals-drawer", false),
                ("run-file-card", false),
                ("command-palette", false),
            ]
        );
        // Mobile -> desktop with closed sidebar clears `inert` on `#sidebar`.
        assert_eq!(
            overlay_inert_targets(false, false, false, false, false),
            [
                ("sidebar", false),
                ("main", false),
                ("approvals-drawer", false),
                ("run-file-card", false),
                ("command-palette", false),
            ]
        );
        // Mobile open sidebar: main inert, sidebar active.
        assert_eq!(
            overlay_inert_targets(false, false, false, true, true),
            [
                ("sidebar", false),
                ("main", true),
                ("approvals-drawer", false),
                ("run-file-card", false),
                ("command-palette", false),
            ]
        );
        // Mobile -> desktop with open sidebar clears `inert` on both `#sidebar` and `#main`.
        assert_eq!(
            overlay_inert_targets(false, false, false, true, false),
            [
                ("sidebar", false),
                ("main", false),
                ("approvals-drawer", false),
                ("run-file-card", false),
                ("command-palette", false),
            ]
        );
        // Command palette sits above drawer and file card, marking lower overlays inert.
        assert_eq!(
            overlay_inert_targets(true, true, true, false, false),
            [
                ("sidebar", true),
                ("main", true),
                ("approvals-drawer", true),
                ("run-file-card", true),
                ("command-palette", false),
            ]
        );
        // Approvals drawer leaves itself active and inerts file card + shell.
        assert_eq!(
            overlay_inert_targets(false, true, true, false, false),
            [
                ("sidebar", true),
                ("main", true),
                ("approvals-drawer", false),
                ("run-file-card", true),
                ("command-palette", false),
            ]
        );
    }

    #[test]
    fn overlay_stack_priority_and_close_restore_dedup_hold() {
        // Native top-layer <dialog open> wins over non-top-layer #run-file-card.
        assert_eq!(
            overlay_stack_priority(false, false, true, true, false, true),
            Some("dialog")
        );
        assert_eq!(
            overlay_stack_priority(true, true, true, true, true, true),
            Some("command-palette")
        );
        assert_eq!(
            overlay_stack_priority(false, false, false, true, true, true),
            Some("run-file-card")
        );
        assert_eq!(
            overlay_stack_priority(false, false, false, false, false, true),
            Some("expanded-workspace")
        );

        let mut suppressed = std::collections::BTreeSet::from(["command-palette".to_owned()]);
        assert!(take_suppress_close_restore(
            &mut suppressed,
            "command-palette"
        ));
        assert!(!take_suppress_close_restore(
            &mut suppressed,
            "command-palette"
        ));
        assert!(!take_suppress_close_restore(&mut suppressed, ""));

        // Open-order OVERLAY_STACK picks the most recently opened open modal regardless of DOM order.
        let mut stack = Vec::new();
        push_overlay_id(&mut stack, "omni-settings-dialog");
        push_overlay_id(&mut stack, "graph-map");
        push_overlay_id(&mut stack, "omni-settings-dialog");
        assert_eq!(stack, vec!["graph-map", "omni-settings-dialog"]);
        assert_eq!(
            top_overlay_from_stack(&stack, &["graph-map".into(), "omni-settings-dialog".into()]),
            Some("omni-settings-dialog")
        );
        remove_overlay_id(&mut stack, "omni-settings-dialog");
        assert_eq!(
            top_overlay_from_stack(&stack, &["graph-map".into()]),
            Some("graph-map")
        );
        assert_eq!(top_overlay_from_stack(&stack, &[]), None);
    }

    #[test]
    fn escape_html_json_signature_and_resize_guard_work() {
        assert_eq!(
            escape_html("<a href=\"x\" title='y'>\u{202e}a&b</a>"),
            "&lt;a href=&quot;x&quot; title=&#39;y&#39;&gt;a&amp;b&lt;/a&gt;"
        );
        assert_eq!(escape_css_attr_value("a\"b\\c\nd"), "a\\\"b\\\\c\\a d");

        let s1 = json_signature(&json!({"id": "c1", "output": "ok"}));
        let s2 = json_signature(&json!({"id": "c1", "output": "ok2"}));
        let s3 = json_signature(&json!({"id": "c1", "output": "ok"}));
        assert_eq!(s1.len(), 32);
        assert_eq!(s1, s3);
        assert_ne!(s1, s2);

        let existing = vec!["session-responsive-viewport".to_owned()];
        assert!(!should_register_viewport_listener(
            &existing,
            "session-responsive-viewport"
        ));
        assert!(should_register_viewport_listener(
            &existing,
            "knowledge-responsive-viewport"
        ));
        assert!(!should_register_viewport_listener(&existing, ""));
    }

    #[test]
    fn markup_signature_and_keyless_child_filter_prevent_redundant_writes_and_leaks() {
        let sig_a = markup_signature("<p>Hello</p>");
        let sig_b = markup_signature("<p>World</p>");
        assert_eq!(sig_a.len(), 32);
        assert_eq!(sig_a, markup_signature("<p>Hello</p>"));
        assert_ne!(sig_a, sig_b);
        assert!(should_apply_markup(None, &sig_a));
        assert!(!should_apply_markup(Some(&sig_a), &sig_a));
        assert!(should_apply_markup(Some(&sig_a), &sig_b));

        assert!(should_remove_keyless_child("P", false));
        assert!(should_remove_keyless_child("li", false));
        assert!(!should_remove_keyless_child("TEMPLATE", false));
        assert!(!should_remove_keyless_child("template", false));
        assert!(!should_remove_keyless_child("P", true));
    }

    #[test]
    fn callback_registry_drops_cancelled_handles_and_survives_reentrant_rearm() {
        use std::cell::Cell;
        use std::rc::Rc;

        struct DropCounter(Rc<Cell<usize>>);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let drops = Rc::new(Cell::new(0));
        let mut reg = CallbackRegistry::default();

        // Cancel drops closure immediately.
        let t1 = reg.allocate_token();
        assert!(reg
            .insert_with_token("poll", t1, 10, DropCounter(drops.clone()))
            .is_none());
        assert_eq!(reg.cancel("poll"), Some(10));
        assert_eq!(drops.get(), 1);
        assert!(reg.active.is_empty());

        // Firing retires closure across execution; re-arming during execution must not drop
        // the currently executing closure mid-frame, and next idle cycle frees it.
        let t2 = reg.allocate_token();
        reg.insert_with_token("countdown", t2, 20, DropCounter(drops.clone()));
        assert!(reg.begin_fire("countdown", t2));
        assert_eq!(
            drops.get(),
            1,
            "active callback stays alive during execution"
        );

        let t3 = reg.allocate_token();
        reg.insert_with_token("countdown", t3, 21, DropCounter(drops.clone()));
        assert_eq!(
            drops.get(),
            1,
            "retired closure stays alive while executing > 0"
        );

        reg.end_fire();
        assert_eq!(reg.cancel("countdown"), Some(21));
        assert_eq!(
            drops.get(),
            3,
            "both retired and cancelled closures are dropped once idle"
        );
        assert!(reg.active.is_empty());
        assert!(reg.retired.is_empty());
    }

    #[test]
    fn stale_async_dialog_close_is_ignored_when_reopened() {
        assert!(should_apply_dialog_close(false));
        assert!(!should_apply_dialog_close(true));
        assert_eq!(
            standalone_body_inert_targets(true, false),
            [
                ("#graph-explain-action", true),
                ("#scrim", true),
                (".skip", true),
            ]
        );
        assert_eq!(
            standalone_body_inert_targets(false, true),
            [
                ("#graph-explain-action", true),
                ("#scrim", false),
                (".skip", true),
            ]
        );
        assert_eq!(
            standalone_body_inert_targets(false, false),
            [
                ("#graph-explain-action", false),
                ("#scrim", false),
                (".skip", false),
            ]
        );
        assert!(is_computed_hidden("none", "visible", false));
        assert!(is_computed_hidden("block", "hidden", true));
        assert!(is_computed_hidden("block", "collapse", true));
        assert!(!is_computed_hidden("block", "hidden", false));
        assert!(!is_computed_hidden("block", "visible", true));
        assert!(MODAL_FOCUSABLE_SELECTOR
            .contains("input:not([disabled]):not([type=\"hidden\"]):not([tabindex=\"-1\"])"));
        assert!(MODAL_FOCUSABLE_SELECTOR.contains("button:not([disabled]):not([tabindex=\"-1\"])"));
    }
}
