use super::*;
use crate::{api, dom, management, model as m, session};
use serde_json::{json, Value as V};
use std::collections::BTreeMap;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

pub(super) fn editor_lock_state(scanning: bool, dirty: bool) -> (&'static str, &'static str, bool) {
    if scanning {
        (
            "scanning",
            "An AI scan is filling the system editor on Services. Planning and launching wait until it ends.",
            false,
        )
    } else if dirty {
        (
            "dirty",
            "The system editor on Services has unsaved changes. Save or discard them to plan or launch a change.",
            true,
        )
    } else {
        ("", "", false)
    }
}

pub(super) fn sync_editor_lock(state: &str, notice: &str, show_discard: bool, locked: bool) {
    let Some(lock) = dom::get("system-editor-lock") else {
        return;
    };
    let current = lock.get_attribute("data-lock-state").unwrap_or_default();
    if current != state {
        let _ = lock.set_attribute("data-lock-state", state);
        lock.set_text_content(None);
        if !notice.is_empty() {
            dom::add(&lock, &dom::el("span", "", notice));
            if show_discard {
                dom::add(&lock, &dom::el("span", "", " "));
                let btn = dom::button("Discard draft", "system-editor-lock-discard", V::Null);
                btn.set_class_name("button compact");
                let _ = btn.set_attribute("id", "system-editor-lock-discard");
                if locked {
                    let _ = btn.set_attribute("disabled", "");
                }
                dom::add(&lock, &btn);
            }
        }
    } else if show_discard {
        dom::disabled("system-editor-lock-discard", locked);
    }
    dom::hide(&lock, notice.is_empty());
}

pub(super) fn discard_editor_draft(from_lock_banner: bool) {
    if m::yes(&m::data("editor-pending")) {
        return;
    }
    let lock_had_focus = from_lock_banner
        && dom::document()
            .active_element()
            .zip(dom::get("system-editor-lock"))
            .is_some_and(|(active, lock)| lock.contains(Some(&active)));
    clear_editor_notices();
    remember_discovery(None);
    m::next("discovery");
    dom::cancel_timer("discovery-poll");
    crate::activity::reset("system-discovery-agent");
    m::take("discovery-owner");
    m::next("editor-scope");
    m::take("editor");
    m::take("editor-generation");
    m::set("editor-dirty", json!(false));
    dom::set_value("system-editor-name", "");
    dom::text("system-editor-location", "");
    dom::clear("system-editor-services");
    dom::clear("system-editor-shared");
    dom::hidden("system-editor", true);
    controls();
    if from_lock_banner {
        dom::text(
            "system-run-message",
            "Discarded unsaved system editor draft.",
        );
        if lock_had_focus {
            for candidate in [
                "system-prepare",
                "system-execute",
                "system-new-change",
                "system-runs",
            ] {
                let can_focus = dom::get(candidate).is_some_and(|el| {
                    !el.has_attribute("hidden")
                        && !dom::is_disabled(&el)
                        && el.closest("[hidden]").ok().flatten().is_none()
                });
                if can_focus {
                    dom::focus(candidate);
                    break;
                }
            }
        }
    }
}

pub async fn editor_open() {
    if m::yes(&m::data("editor-pending")) {
        return;
    }
    if !sync_editor_target() {
        return;
    }
    if m::yes(&m::data("editor-dirty")) && editor_ready() {
        dom::hidden("system-editor", false);
        layout();
        return;
    }
    clear_editor_notices();
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let generation = m::bind_scope("editor-scope", &project, &config);
    m::bind_pending("editor-pending", generation);
    controls();
    match api::post(
        "/api/systems/editor",
        json!({"project_id":project,"config_path":config}),
    )
    .await
    {
        Ok(value)
            if editor_current(&project, &config, generation)
                && value["project_id"] == project
                && value["config_path"] == config =>
        {
            m::set("editor-generation", json!(generation));
            m::set("editor", value);
            m::set("editor-dirty", json!(false));
            render_editor();
            restore_discovery().await;
        }
        Err(error) if editor_current(&project, &config, generation) => {
            dom::hidden("system-editor", false);
            dom::error("system-editor-error", &error.message);
        }
        _ => {}
    }
    if m::finish_pending("editor-pending", generation) {
        m::set("editor-pending", m::data("discovery-active"));
        controls();
    }
}
pub(super) fn field(
    parent: &web_sys::Element,
    label: &str,
    value: &V,
    path: &str,
    kind: &str,
    choices: &[&str],
    constraints: (bool, usize),
) {
    let (required, limit) = constraints;
    let wrap = dom::el(
        "label",
        if kind == "checkbox" {
            "checkbox"
        } else if kind == "lines" || kind == "textarea" {
            "field wide"
        } else {
            "field"
        },
        if kind == "checkbox" { "" } else { label },
    );
    let input = dom::el(
        if !choices.is_empty() {
            "select"
        } else if kind == "lines" || kind == "textarea" {
            "textarea"
        } else {
            "input"
        },
        "",
        "",
    );
    let _ = input.set_attribute("data-field", &format!("{path}|{kind}"));
    let focus_key = if let Some((_, tail)) = path.split_once("/passport/") {
        format!("field:passport/{tail}")
    } else {
        format!("field:{path}")
    };
    let _ = input.set_attribute("data-focus-key", &focus_key);
    if required {
        let _ = input.set_attribute("required", "");
    }
    if choices.is_empty() {
        let _ = input.set_attribute("maxlength", &limit.to_string());
    }
    if kind == "checkbox" {
        let _ = input.set_attribute("type", "checkbox");
        if let Some(i) = input.dyn_ref::<web_sys::HtmlInputElement>() {
            i.set_checked(m::yes(value));
        }
    } else {
        for choice in choices {
            let e = dom::el("option", "", choice);
            let _ = e.set_attribute("value", choice);
            dom::add(&input, &e);
        }
        dom::set_element_value(
            &input,
            &if kind == "lines" {
                m::array(value)
                    .iter()
                    .map(m::string)
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                m::string(value)
            },
        );
    }
    dom::add(&wrap, &input);
    if kind == "checkbox" {
        // A checkbox reads as one line: the box, then its statement.
        dom::add(&wrap, &dom::el("span", "", label));
    }
    dom::add(parent, &wrap);
}
/// A service card is named by its ID; before one is entered, by its folder.
pub(super) fn service_title(passport: &V, path: &str) -> String {
    let id = m::text(passport, "id");
    if !id.trim().is_empty() {
        return id;
    }
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .map(|name| format!("New service · {name}"))
        .unwrap_or_else(|| "New service".into())
}
/// One-line singular name of a row in an editor section, used in headings and
/// button names so that "Remove" and "Add" say what they act on.
pub(super) fn row_name(kind: &str) -> &'static str {
    match kind {
        "capabilities" => "Capability",
        "provides" => "Provided contract",
        "consumes" => "Consumed contract",
        _ => "Source",
    }
}
pub(super) fn build_rows_details(
    title: &str,
    values: &V,
    path: &str,
    kind: &str,
    restore_key: &str,
) -> web_sys::Element {
    let d = dom::el("details", "system-editor-section", "");
    let _ = d.set_attribute("data-restore-key", restore_key);
    let summary = dom::el(
        "summary",
        "",
        &format!("{title} ({})", m::array(values).len()),
    );
    let _ = summary.set_attribute("data-focus-key", &format!("summary:{restore_key}"));
    dom::add(&d, &summary);
    let name = row_name(kind);
    for (index, value) in m::array(values).into_iter().enumerate() {
        let item = dom::el("div", "system-editor-row", "");
        let head = dom::el("div", "system-editor-row-head", "");
        let base = format!("{path}/{index}");
        dom::add(&head, &dom::el("h6", "", &format!("{name} {}", index + 1)));
        let remove = dom::button(
            "Remove",
            "editor-row-remove",
            json!({"path":path,"index":index}),
        );
        let _ = remove.set_attribute("class", "button quiet");
        let _ = remove.set_attribute(
            "aria-label",
            &format!("Remove {} {}", name.to_lowercase(), index + 1),
        );
        let _ = remove.set_attribute("data-focus-key", &format!("remove:{kind}:{index}"));
        dom::add(&head, &remove);
        dom::add(&item, &head);
        // Field helpers below write into the row's two-column grid.
        let row = dom::el("div", "system-editor-grid", "");
        dom::add(&item, &row);
        match kind {
            "capabilities" => {
                field(
                    &row,
                    "Capability ID",
                    &value["id"],
                    &format!("{base}/id"),
                    "text",
                    &[],
                    (true, 80),
                );
                field(
                    &row,
                    "Status",
                    &value["status"],
                    &format!("{base}/status"),
                    "text",
                    &["unknown", "planned", "partial", "implemented"],
                    (false, 200),
                );
                field(
                    &row,
                    "Description",
                    &value["description"],
                    &format!("{base}/description"),
                    "textarea",
                    &[],
                    (true, 2000),
                );
                field(
                    &row,
                    "Source paths (one per line)",
                    &value["sources"],
                    &format!("{base}/sources"),
                    "lines",
                    &[],
                    (false, 102400),
                );
                field(
                    &row,
                    "Routing keywords (one per line)",
                    &value["keywords"],
                    &format!("{base}/keywords"),
                    "lines",
                    &[],
                    (false, 102400),
                );
            }
            "provides" => {
                field(
                    &row,
                    "Contract ID",
                    &value["id"],
                    &format!("{base}/id"),
                    "text",
                    &[],
                    (true, 80),
                );
                field(
                    &row,
                    "Contract type",
                    &value["kind"],
                    &format!("{base}/kind"),
                    "text",
                    &["http", "event", "rpc", "graphql", "other"],
                    (false, 200),
                );
                field(
                    &row,
                    "Contract version",
                    &value["version"],
                    &format!("{base}/version"),
                    "text",
                    &[],
                    (true, 100),
                );
                field(
                    &row,
                    "Source paths (one per line)",
                    &value["sources"],
                    &format!("{base}/sources"),
                    "lines",
                    &[],
                    (false, 102400),
                );
            }
            "consumes" => {
                for (key, label, limit) in [
                    ("service", "Provider service ID", 80),
                    ("contract", "Consumed contract ID", 80),
                    ("version", "Contract version", 100),
                ] {
                    field(
                        &row,
                        label,
                        &value[key],
                        &format!("{base}/{key}"),
                        "text",
                        &[],
                        (true, limit),
                    );
                }
            }
            _ => {
                field(
                    &row,
                    "Source path",
                    &value["path"],
                    &format!("{base}/path"),
                    "text",
                    &[],
                    (true, 1024),
                );
                field(
                    &row,
                    "Source kind",
                    &value["kind"],
                    &format!("{base}/kind"),
                    "text",
                    &["policy", "spec", "contract", "code", "test", "memory"],
                    (false, 200),
                );
            }
        }
        dom::add(&d, &item);
    }
    let add_btn = dom::button(
        &format!("Add {}", name.to_lowercase()),
        "editor-row-add",
        json!({"path":path,"kind":kind}),
    );
    let _ = add_btn.set_attribute("data-focus-key", &format!("add:{kind}"));
    dom::add(&d, &add_btn);
    d
}

pub(super) fn rows(parent: &web_sys::Element, title: &str, values: &V, path: &str, kind: &str) {
    dom::add(parent, &build_rows_details(title, values, path, kind, kind));
}

pub(super) fn build_editor_service_items(services: &[V]) -> Vec<V> {
    let mut seen_paths = BTreeMap::<String, usize>::new();
    services
        .iter()
        .enumerate()
        .map(|(index, service)| {
            let path = m::text(service, "path");
            let base = if path.is_empty() {
                format!("idx:{index}")
            } else {
                format!("path:{path}")
            };
            let count = seen_paths.entry(base.clone()).or_insert(0);
            let key = if *count == 0 {
                base
            } else {
                format!("{base}#{}", *count)
            };
            *count += 1;
            json!({
                "key": key,
                "index": index,
                "service": service,
            })
        })
        .collect()
}

pub(super) fn build_editor_service_card(item: &V) -> web_sys::Element {
    let index = item["index"].as_u64().unwrap_or(0) as usize;
    let service = &item["service"];
    let base = format!("/services/{index}/passport");
    let p = &service["passport"];
    let path = m::text(service, "path");
    let card = dom::el("section", "system-editor-service", "");
    let head = dom::el("div", "system-editor-service-head", "");
    let title = dom::el("div", "", "");
    dom::add(&title, &dom::el("h5", "", &service_title(p, &path)));
    dom::add(&title, &dom::el("p", "system-note", &path));
    dom::add(&head, &title);
    let actions = dom::el("div", "system-editor-service-actions", "");
    for (label, action, payload, focus_key) in [
        (
            "Choose another folder",
            "editor-service-folder",
            json!(index),
            "service-folder",
        ),
        (
            "Remove service",
            "editor-row-remove",
            json!({"path":"/services","index":index}),
            "service-remove",
        ),
    ] {
        let button = dom::button(label, action, payload);
        let _ = button.set_attribute("class", "button quiet");
        let _ = button.set_attribute("data-focus-key", focus_key);
        dom::add(&actions, &button);
    }
    dom::add(&head, &actions);
    dom::add(&card, &head);
    let grid = dom::el("div", "system-editor-grid", "");
    for (key, label, kind, required, limit) in [
        ("id", "Service ID", "text", true, 80),
        ("owner", "Owning team", "text", true, 200),
        ("description", "Service description", "textarea", true, 2000),
    ] {
        field(
            &grid,
            label,
            &p[key],
            &format!("{base}/{key}"),
            kind,
            &[],
            (required, limit),
        );
    }
    dom::add(&card, &grid);
    field(
        &card,
        "I have described all known dependencies",
        &p["relationships_complete"],
        &format!("{base}/relationships_complete"),
        "checkbox",
        &[],
        (false, 200),
    );
    for (key, title) in [
        ("capabilities", "Capabilities"),
        ("provides", "Provided contracts"),
        ("consumes", "Consumed contracts"),
        ("sources", "Sources of context"),
    ] {
        rows(&card, title, &p[key], &format!("{base}/{key}"), key);
    }
    card
}

pub(super) fn render_editor() {
    let data = m::data("editor");
    if !data.is_object() {
        return;
    }
    dom::hidden("system-editor", false);
    let name = m::text(&data, "name");
    if dom::value("system-editor-name") != name {
        dom::set_value("system-editor-name", &name);
    }
    let location = format!(
        "{} / {}",
        m::text(&m::project(&m::text(&data, "project_id")), "path"),
        m::text(&data, "config_path")
    );
    if dom::get("system-editor-location")
        .and_then(|e| e.text_content())
        .as_deref()
        != Some(location.as_str())
    {
        dom::text("system-editor-location", &location);
    }
    let service_items = build_editor_service_items(&m::array(&data["services"]));
    dom::keyed_render(
        "system-editor-services",
        &service_items,
        |item| m::text(item, "key"),
        build_editor_service_card,
    );
    let shared_items = [json!({
        "key": "shared_sources",
        "values": data["shared_sources"],
    })];
    dom::keyed_render(
        "system-editor-shared",
        &shared_items,
        |item| m::text(item, "key"),
        |item| {
            build_rows_details(
                "Shared sources",
                &item["values"],
                "/shared_sources",
                "sources",
                "shared_sources",
            )
        },
    );
    controls();
}
pub(super) fn pointer_token(token: &str) -> Option<String> {
    let mut decoded = String::new();
    let mut chars = token.chars();
    while let Some(ch) = chars.next() {
        if ch == '~' {
            decoded.push(match chars.next()? {
                '0' => '~',
                '1' => '/',
                _ => return None,
            });
        } else {
            decoded.push(ch);
        }
    }
    Some(decoded)
}
pub(super) fn set_editor_value(data: &mut V, path: &str, value: V) -> bool {
    let Some(tokens) = path.strip_prefix('/') else {
        return false;
    };
    if tokens
        .split('/')
        .any(|token| pointer_token(token).is_none())
    {
        return false;
    }
    let Some((parent, leaf)) = path.rsplit_once('/') else {
        return false;
    };
    let Some(leaf) = pointer_token(leaf) else {
        return false;
    };
    if let Some(slot) = data.pointer_mut(path) {
        *slot = value;
        return true;
    }
    let Some(object) = data.pointer_mut(parent).and_then(V::as_object_mut) else {
        return false;
    };
    object.insert(leaf, value);
    true
}
pub fn edit_field(field: &str, value: &str, checked: bool) {
    if !editor_ready() || m::yes(&m::data("editor-pending")) {
        return;
    }
    if m::yes(&m::data("editor-pending")) {
        return;
    }
    let (path, kind) = field.rsplit_once('|').unwrap_or((field, "text"));
    let mut data = m::data("editor");
    let v = if kind == "checkbox" {
        json!(checked)
    } else if kind == "lines" {
        json!(value
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>())
    } else {
        json!(value)
    };
    if set_editor_value(&mut data, path, v) {
        m::set("editor", data);
        changed();
    }
}
pub(super) fn changed() {
    m::set("editor-dirty", json!(true));
    dom::text("system-editor-status", "Unsaved changes");
    controls();
}
pub(super) async fn service_folder(index: Option<u64>) {
    if !editor_ready() || m::yes(&m::data("editor-pending")) {
        return;
    }
    let generation = m::read(|s| s.epochs.get("editor-scope").copied().unwrap_or(0));
    m::set(
        "service-folder-owner",
        json!({"project":dom::value("system-project"),"config":dom::value("system-config"),"generation":generation}),
    );
    let purpose = index
        .map(|i| format!("service:{i}"))
        .unwrap_or_else(|| "service:".into());
    management::folder_open_at(
        &purpose,
        &service_folder_start(index),
        "Choose a service folder",
    )
    .await;
}
/// Where the service folder browser opens: the replaced service's own folder; for a
/// new service, the folder that holds the last added one (services are usually
/// siblings); for the first, the system folder.
pub(super) fn service_folder_start(index: Option<u64>) -> String {
    let services = m::array(&m::data("editor")["services"]);
    if let Some(service) = index.and_then(|index| services.get(index as usize)) {
        return m::text(service, "path");
    }
    if let Some(last) = services.last() {
        let path = m::text(last, "path");
        if let Some((parent, _)) = path.trim_end_matches(['/', '\\']).rsplit_once(['/', '\\']) {
            if !parent.is_empty() {
                return parent.to_owned();
            }
        }
    }
    m::text(&m::project(&dom::value("system-project")), "path")
}
pub async fn add_service(project: &str, index: Option<usize>) {
    let selected = m::take("service-folder-owner");
    if !editor_current(
        &m::text(&selected, "project"),
        &m::text(&selected, "config"),
        selected["generation"].as_u64().unwrap_or(0),
    ) {
        dom::error(
            "system-editor-error",
            "The selected system changed. Choose the service folder again.",
        );
        return;
    }
    if m::yes(&m::data("editor-pending")) || !editor_ready() {
        return;
    }
    let owner = dom::value("system-project");
    let config = dom::value("system-config");
    let generation = m::bind_scope("editor-scope", &owner, &config);
    m::set("editor-generation", json!(generation));
    m::bind_pending("editor-pending", generation);
    controls();
    match api::post("/api/systems/service", json!({"project_id":project})).await {
        Ok(mut service) if editor_current(&owner, &config, generation) && editor_ready() => {
            let mut data = m::data("editor");
            if let Some(services) = data["services"].as_array_mut() {
                if let Some(index) = index {
                    if index < services.len() {
                        if service["fingerprint"].is_null() {
                            service["passport"] = services[index]["passport"].clone();
                        }
                        services[index] = service;
                    }
                } else {
                    services.push(service);
                }
            }
            m::set("editor", data);
            changed();
            render_editor();
        }
        Err(error) if editor_current(&owner, &config, generation) => {
            dom::error("system-editor-error", &error.message)
        }
        _ => {}
    }
    if m::finish_pending("editor-pending", generation) {
        controls();
    }
}
pub(super) fn editor_payload() -> V {
    let mut data = m::data("editor");
    if let Some(items) = data["services"].as_array_mut() {
        for item in items {
            if let Some(map) = item.as_object_mut() {
                map.remove("path");
            }
        }
    }
    data
}
pub(super) async fn save_editor() {
    if m::yes(&m::data("editor-pending")) || !dom::validity("system-editor") {
        return;
    }
    if !editor_ready() {
        dom::error(
            "system-editor-error",
            "Load the current project's system before saving.",
        );
        return;
    }
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let payload = editor_payload();
    let generation = m::bind_scope("editor-scope", &project, &config);
    m::set("editor-generation", json!(generation));
    m::next("discovery");
    m::bind_pending("editor-pending", generation);
    controls();
    let mut saved = false;
    match api::post("/api/systems/preview", payload).await {
        Ok(preview) if editor_current(&project, &config, generation) && editor_ready() => {
            if let Some(id) = preview["preview_id"].as_str() {
                // The file preview is reviewed for this same bound target.
                if editor_current(&project, &config, generation) && editor_ready() {
                    match api::post("/api/systems/apply", json!({"preview_id":id})).await {
                        Ok(value)
                            if editor_current(&project, &config, generation)
                                && value["editor"]["project_id"] == project
                                && value["editor"]["config_path"] == config =>
                        {
                            m::set("editor", value["editor"].clone());
                            m::set("editor-dirty", json!(false));
                            m::set("system-catalog", value.clone());
                            render_catalog(&value);
                            remember_discovery(None);
                            saved = true;
                        }
                        Err(error) if editor_current(&project, &config, generation) => {
                            dom::error("system-editor-error", &error.message)
                        }
                        _ => {}
                    }
                }
            } else {
                dom::error(
                    "system-editor-error",
                    "The runner returned no system preview.",
                );
            }
        }
        Err(error) if editor_current(&project, &config, generation) => {
            dom::error("system-editor-error", &error.message)
        }
        _ => {}
    }
    if m::finish_pending("editor-pending", generation) {
        if editor_current(&project, &config, generation) {
            if saved {
                dom::hidden("system-editor", true);
                dom::text(
                    "system-message",
                    "System saved. The map shows the saved services.",
                );
            } else {
                dom::text(
                    "system-editor-status",
                    "Save did not complete. The form is retained; reload metadata before retrying if files changed.",
                );
            }
        }
        controls();
    }
}
pub(super) fn discovery_key() -> String {
    let d = m::data("editor");
    format!(
        "harness-discovery:{}:{}",
        m::text(&d, "project_id"),
        m::text(&d, "config_path")
    )
}
pub(super) fn remember_discovery(id: Option<&str>) {
    dom::save(&discovery_key(), id);
}
pub(super) async fn discovery_start() {
    if m::yes(&m::data("editor-pending")) || !editor_ready() {
        return;
    }
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let generation = m::bind_scope("editor-scope", &project, &config);
    m::set("editor-generation", json!(generation));
    let epoch = m::next("discovery");
    m::set(
        "discovery-owner",
        json!({"project":project,"config":config,"generation":generation}),
    );
    m::set("editor-pending", json!(true));
    m::set("discovery-active", json!(true));
    controls();
    dom::text(
        "system-discovery-status",
        "Capturing service evidence and queueing AI scan…",
    );
    match api::post("/api/system-discoveries",json!({"editor":editor_payload(),"provider":dom::value("system-discovery-provider"),"timeout":dom::num("system-discovery-timeout")})).await {
        Ok(value) if discovery_current(epoch) => {
            let id=m::text(&value,"id");
            m::set("discovery-id",json!(id));
            remember_discovery(Some(&id));
            session::refresh().await;
            if discovery_current(epoch) { discovery_poll(epoch).await; }
        }
        Err(error) if discovery_current(epoch) => {
            dom::error("system-editor-error",&error.message);
            m::set("editor-pending",json!(false));
            m::set("discovery-active",json!(false));
            controls();
        }
        _ => {}
    }
}
/// What a proposed passport changes from the one the form held before the
/// scan: the proposal replaces the whole form, so a scan of a system whose
/// passports were edited by hand says what Save would overwrite.
pub(super) fn passport_changes(before: &V, after: &V) -> String {
    if !before.is_object() {
        return "new passport".into();
    }
    let changed = [
        "description",
        "owner",
        "relationships_complete",
        "capabilities",
        "provides",
        "consumes",
        "sources",
    ]
    .iter()
    .filter(|key| before[**key] != after[**key])
    .map(
        |key| match (before[*key].as_array(), after[*key].as_array()) {
            (Some(old), Some(new)) if old.len() != new.len() => {
                format!("{} {} → {}", m::label(key), old.len(), new.len())
            }
            _ => m::label(key),
        },
    )
    .collect::<Vec<_>>();
    if changed.is_empty() {
        "same as saved".into()
    } else {
        format!("changes {}", changed.join(", "))
    }
}
pub(super) fn proposal(v: &V, draft: &V) {
    if !editor_ready()
        || v["editor"]["project_id"] != dom::value("system-project")
        || v["editor"]["config_path"] != dom::value("system-config")
    {
        return;
    }
    if v["editor"].is_object() {
        m::set("editor", v["editor"].clone());
        changed();
        render_editor();
    }
    dom::clear("system-discovery-report");
    dom::hidden("system-discovery-report", false);
    dom::append(
        "system-discovery-report",
        &dom::el(
            "p",
            "system-note",
            "AI filled the forms. Verify sources, ownership and dependencies before saving. Complete dependency coverage remains unconfirmed.",
        ),
    );
    for warning in m::array(&v["warnings"]) {
        dom::append(
            "system-discovery-report",
            &dom::el("p", "system-warning", &m::string(&warning)),
        );
    }
    for report in m::array(&v["reports"]) {
        let saved = m::array(&draft["services"])
            .into_iter()
            .find(|service| service["passport"]["id"] == report["id"])
            .map(|service| service["passport"].clone())
            .unwrap_or(V::Null);
        let d = dom::details(
            &format!(
                "{} · {} capabilities · {} provided · {} consumed contracts · {}",
                m::text(&report, "id"),
                m::array(&report["passport"]["capabilities"]).len(),
                m::array(&report["passport"]["provides"]).len(),
                m::array(&report["passport"]["consumes"]).len(),
                passport_changes(&saved, &report["passport"])
            ),
            &serde_json::to_string_pretty(&report).unwrap_or_default(),
        );
        dom::add(
            &d,
            &dom::details(
                "Captured evidence inventory",
                &serde_json::to_string_pretty(&v["inventory"][m::text(&report, "id")])
                    .unwrap_or_default(),
            ),
        );
        dom::append("system-discovery-report", &d);
    }
}
pub(super) async fn discovery_poll(epoch: u64) {
    if !discovery_current(epoch) {
        return;
    }
    dom::cancel_timer("discovery-poll");
    let id = m::string(&m::data("discovery-id"));
    if id.is_empty() {
        return;
    }
    match api::get(&format!("/api/system-discoveries/{}", dom::enc(&id))).await {
        Ok(job) if discovery_current(epoch) && job["id"] == id => {
            let active = m::yes(&job["active"]);
            m::set("discovery-active", json!(active));
            m::set("editor-pending", json!(active));
            dom::text(
                "system-discovery-status",
                &format!(
                    "AI scan · {} · {}",
                    m::text(&job, "provider"),
                    m::label(job["status"].as_str().unwrap_or(""))
                ),
            );
            dom::hidden("system-discovery-agents", false);
            dom::hidden("system-discovery-live", !active);
            discovery_agents(&job).await;
            if !discovery_current(epoch) {
                return;
            }
            if active {
                dom::later("discovery-poll", 1000, move || {
                    spawn_local(async move {
                        discovery_poll(epoch).await;
                    })
                });
            } else {
                if job["proposal"].is_object() {
                    proposal(&job["proposal"], &job["draft"]);
                    dom::text(
                        "system-editor-status",
                        "AI filled service details. Review sources and uncertainties, then Save system.",
                    );
                } else {
                    dom::text(
                        "system-discovery-status",
                        &format!("{} Your original form is retained.", scan_end(&job)),
                    );
                }
                session::refresh().await;
            }
            if discovery_current(epoch) {
                controls();
            }
        }
        Err(error) if discovery_current(epoch) => {
            if [400, 404].contains(&error.status) {
                let unavailable = match api::get(&format!("/api/sessions/{}", dom::enc(&id))).await
                {
                    Ok(value) => !m::active(&value["session"]),
                    Err(missing) => [400, 404].contains(&missing.status),
                };
                if !discovery_current(epoch) {
                    return;
                }
                if unavailable {
                    m::set("discovery-active", json!(false));
                    m::set("editor-pending", json!(false));
                    changed();
                    remember_discovery(None);
                    dom::text(
                        "system-discovery-status",
                        "AI scan is no longer available. The original form is retained.",
                    );
                    controls();
                    return;
                }
            }
            if !discovery_current(epoch) {
                return;
            }
            dom::text(
                "system-discovery-status",
                &format!("Reconnecting to AI scan: {}", error.message),
            );
            dom::later("discovery-poll", 3000, move || {
                spawn_local(async move {
                    discovery_poll(epoch).await;
                })
            });
        }
        _ => {}
    }
}
pub(super) async fn restore_discovery() {
    if !editor_ready() {
        return;
    }
    let key = discovery_key();
    let Some(id) = dom::storage(&key) else {
        return;
    };
    let owner = m::data("editor-scope-owner");
    let project = m::text(&owner, "owner");
    let config = m::text(&owner, "item");
    let generation = m::read(|s| s.epochs.get("editor-scope").copied().unwrap_or(0));
    let epoch = m::next("discovery");
    m::set(
        "discovery-owner",
        json!({"project":project,"config":config,"generation":generation}),
    );
    match api::get(&format!("/api/system-discoveries/{}", dom::enc(&id))).await {
        Ok(job) if discovery_current(epoch) && key == discovery_key() && job["id"] == id => {
            m::set("discovery-id", json!(id));
            discovery_agents(&job).await;
            if !discovery_current(epoch) {
                return;
            }
            let active = m::yes(&job["active"]);
            m::set("discovery-active", json!(active));
            m::set("editor-pending", json!(active));
            let draft = &job["draft"];
            let same_target = draft["project_id"] == project && draft["config_path"] == config;
            if active && same_target {
                m::set("editor", draft.clone());
                changed();
                render_editor();
                discovery_poll(epoch).await;
            } else if job["proposal"].is_object() {
                proposal(&job["proposal"], &job["draft"]);
            } else if same_target && draft["revision"] == m::data("editor")["revision"] {
                m::set("editor", draft.clone());
                changed();
                render_editor();
            }
        }
        Err(error) if discovery_current(epoch) => dom::text(
            "system-discovery-status",
            &format!("Previous AI scan unavailable: {}", error.message),
        ),
        _ => {}
    }
}
