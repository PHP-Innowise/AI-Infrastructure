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
const FALLBACK_RESTORE_SELECTOR: &str = "#view-tabs [aria-selected=\"true\"], #new-session";
const MODAL_FOCUSABLE_SELECTOR: &str = "button:not([disabled]), a[href], input:not([disabled]):not([type=\"hidden\"]), select:not([disabled]), textarea:not([disabled]), details > summary:not([tabindex=\"-1\"]), [tabindex]:not([tabindex=\"-1\"])";

pub fn is_mobile_viewport() -> bool {
    window()
        .match_media("(max-width: 920px)")
        .ok()
        .flatten()
        .is_some_and(|list| list.matches())
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

fn set_inert_attr(id: &str, inert: bool) {
    if let Some(e) = get(id) {
        if inert {
            let _ = e.set_attribute("inert", "");
        } else {
            let _ = e.remove_attribute("inert");
        }
    }
}

/// Synchronizes `inert` attributes across the shell, sidebar, and stacked modal overlays.
pub fn sync_overlay_inert() {
    let is_open = |id: &str| get(id).is_some_and(|e| !e.has_attribute("hidden"));
    let sidebar_open = get("sidebar").is_some_and(|e| e.class_list().contains("open"));
    for (id, inert) in overlay_inert_targets(
        is_open("command-palette"),
        is_open("approvals-drawer"),
        is_open("run-file-card"),
        sidebar_open,
        is_mobile_viewport(),
    ) {
        set_inert_attr(id, inert);
    }
}

pub fn hidden(id: &str, v: bool) {
    if let Some(e) = get(id) {
        hide(&e, v);
    }
}
pub fn hide(e: &Element, v: bool) {
    if v {
        let _ = e.set_attribute("hidden", "");
    } else {
        let _ = e.remove_attribute("hidden");
    }
    let id = e.id();
    if MODAL_OVERLAY_IDS.contains(&id.as_str()) {
        sync_overlay_inert();
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
pub fn attr(id: &str, key: &str, value: &str) {
    if let Some(e) = get(id) {
        let _ = e.set_attribute(key, value);
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
    static FOCUS_RETURN_STACK: std::cell::RefCell<Vec<(String, Element)>> =
        const { std::cell::RefCell::new(Vec::new()) };
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
    let saved = FOCUS_RETURN_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        let pos = stack.iter().rposition(|(id, _)| id == overlay_id)?;
        Some(stack.remove(pos).1)
    });
    if let Some(el) = saved {
        if el.is_connected() {
            focus_element(&el);
            return;
        }
    }
    if let Some(fallback) = document()
        .query_selector(FALLBACK_RESTORE_SELECTOR)
        .ok()
        .flatten()
    {
        focus_element(&fallback);
    }
}

pub fn dialog(id: &str, open: bool) {
    if let Some(d) = get(id).and_then(|e| e.dyn_into::<web_sys::HtmlDialogElement>().ok()) {
        if open {
            if !d.open() {
                save_focus_for_overlay(id);
                let _ = d.show_modal();
                if let Some(el) = modal_focusables(d.as_ref()).first() {
                    focus_element(el);
                }
            }
        } else {
            if d.open() {
                d.close();
            }
            restore_focus_for_overlay(id);
        }
    }
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
pub fn later(key: &str, ms: i32, callback: impl FnOnce() + 'static) {
    cancel_timer(key);
    let name = key.to_owned();
    let c = Closure::once_into_js(move || {
        crate::model::write(|s| {
            s.timers.remove(&name);
        });
        callback();
    });
    if let Ok(id) =
        window().set_timeout_with_callback_and_timeout_and_arguments_0(c.unchecked_ref(), ms)
    {
        crate::model::write(|s| {
            s.timers.insert(key.into(), id);
        });
    }
}
pub fn cancel_timer(key: &str) {
    if let Some(id) = crate::model::write(|s| s.timers.remove(key)) {
        window().clear_timeout_with_handle(id);
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
        }
    }
    for (index, item) in items.iter().enumerate() {
        let key = key_of(item);
        let signature = item.to_string();
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

fn element_hidden(e: &Element, stop_at: Option<&Element>) -> bool {
    let mut cur = Some(e.clone());
    while let Some(node) = cur {
        if node.has_attribute("hidden") {
            return true;
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
    for id in MODAL_OVERLAY_IDS {
        if let Some(el) = get(id) {
            if !el.has_attribute("hidden") {
                return Some(el);
            }
        }
    }
    if let Ok(Some(dialog)) = document().query_selector("dialog[open]") {
        return Some(dialog);
    }
    if is_mobile_viewport() {
        if let Some(sidebar) = get("sidebar") {
            if sidebar.class_list().contains("open") {
                return Some(sidebar);
            }
        }
    }
    None
}

fn handle_modal_trap(event: &web_sys::KeyboardEvent) -> bool {
    if event.key() != "Tab" || event.ctrl_key() || event.meta_key() || event.alt_key() {
        return false;
    }
    let Some(container) = active_overlay_container() else {
        return false;
    };
    let items = modal_focusables(&container);
    if items.is_empty() {
        focus_element(&container);
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

fn sync_widget_from_target(target: &Element) {
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

pub fn wire_a11y_widgets() {
    sync_overlay_inert();
    for widget in all("[role=\"tablist\"],[role=\"toolbar\"],[role=\"radiogroup\"],[role=\"tree\"]")
    {
        sync_roving_widget(&widget);
    }
    if crate::model::yes(&crate::model::data("a11y-widgets-wired")) {
        return;
    }
    crate::model::set("a11y-widgets-wired", json!(true));

    let keydown_cb = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        let Some(key_event) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        if handle_modal_trap(key_event) || handle_widget_keydown(key_event) {
            key_event.prevent_default();
        }
    });
    let _ =
        document().add_event_listener_with_callback("keydown", keydown_cb.as_ref().unchecked_ref());
    keydown_cb.forget();

    let focus_cb = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        if let Some(target) = event.target().and_then(|v| v.dyn_into::<Element>().ok()) {
            sync_widget_from_target(&target);
        }
    });
    let _ =
        document().add_event_listener_with_callback("focusin", focus_cb.as_ref().unchecked_ref());
    focus_cb.forget();

    let click_cb = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        if let Some(target) = event.target().and_then(|v| v.dyn_into::<Element>().ok()) {
            sync_widget_from_target(&target);
        }
    });
    let _ = document().add_event_listener_with_callback("click", click_cb.as_ref().unchecked_ref());
    click_cb.forget();

    let toggle_cb = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        if let Some(target) = event.target().and_then(|v| v.dyn_into::<Element>().ok()) {
            if let Ok(Some(tree)) = target.closest("[role=\"tree\"]") {
                sync_roving_widget(&tree);
            }
        }
    });
    let _ = document().add_event_listener_with_callback_and_bool(
        "toggle",
        toggle_cb.as_ref().unchecked_ref(),
        true,
    );
    toggle_cb.forget();

    let close_cb = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        if let Some(target) = event.target().and_then(|v| v.dyn_into::<Element>().ok()) {
            if target.tag_name() == "DIALOG" {
                let id = target.id();
                if !id.is_empty() {
                    restore_focus_for_overlay(&id);
                }
            }
        }
    });
    let _ = document().add_event_listener_with_callback_and_bool(
        "close",
        close_cb.as_ref().unchecked_ref(),
        true,
    );
    close_cb.forget();
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
}
