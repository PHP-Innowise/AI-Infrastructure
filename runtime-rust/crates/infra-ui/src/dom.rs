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
    get(id).map(|e| value_of(&e)).unwrap_or_default()
}
pub fn set_value(id: &str, v: &str) {
    if let Some(e) = get(id) {
        set_element_value(&e, v);
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
pub fn dialog(id: &str, open: bool) {
    if let Some(d) = get(id).and_then(|e| e.dyn_into::<web_sys::HtmlDialogElement>().ok()) {
        if open {
            let _ = d.show_modal();
        } else {
            d.close();
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
        x.set_checked(checked);
    }
    add(&e, &i);
    add(&e, &el("span", "", label));
    e
}
pub fn options(id: &str, items: &[V], label: impl Fn(&V) -> String, previous: Option<&str>) {
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
pub fn hash_view() -> String {
    window()
        .location()
        .hash()
        .unwrap_or_default()
        .trim_start_matches("#/")
        .to_owned()
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
