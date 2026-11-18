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
        x.set_value(value);
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

fn tree_item_collapsed(e: &Element, tree: &Element) -> bool {
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
        if &parent == tree {
            break;
        }
        child = parent.clone();
        cur = parent.parent_element();
    }
    false
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
    query_elements(
        card,
        "button:not([disabled]), a[href], input:not([disabled]):not([type=\"hidden\"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex=\"-1\"])",
    )
    .into_iter()
    .filter(|el| !is_disabled(el) && !element_hidden(el, Some(card)))
    .collect()
}

fn handle_modal_trap(event: &web_sys::KeyboardEvent) -> bool {
    if event.key() != "Tab" || event.ctrl_key() || event.meta_key() || event.alt_key() {
        return false;
    }
    let Some(card) = get("run-file-card") else {
        return false;
    };
    if card.has_attribute("hidden") {
        return false;
    }
    let items = modal_focusables(&card);
    if items.is_empty() {
        focus_element(&card);
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
        return false;
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
}
