use super::registered;
use super::skills::{create_skill_controls, skills_controls, validate_create_skill_inputs};
use crate::{api, dom, model as m, session, systems};
use serde_json::{json, Value as V};

pub async fn folder_open(purpose: &str) {
    folder_open_at(purpose, "", "Choose a project folder").await;
}
/// Opens the folder browser at `start` (the home folder when empty) under a title
/// that names what the chosen folder becomes.
pub async fn folder_open_at(purpose: &str, start: &str, title: &str) {
    m::set("folder-purpose", json!(purpose));
    dom::text("folder-picker-title", title);
    dom::error("folder-picker-error", "");
    dom::dialog("folder-picker", true);
    // A start folder that no longer exists opens the home folder instead.
    if !folder(start, false).await && !start.is_empty() {
        dom::error("folder-picker-error", "");
        folder("", false).await;
    }
}
/// Lists `path`; returns whether the listing was shown.
pub(super) async fn folder(path: &str, query: bool) -> bool {
    let body = json!({"query":if query{dom::value("folder-picker-query").trim().to_owned()}else{String::new()},"hidden":dom::checked("folder-picker-hidden")});
    let mut body = body;
    if !path.is_empty() {
        body["path"] = json!(path);
    }
    let epoch = m::next("folder");
    // A path typed while this listing loads is the person's next request; keep it.
    let typed = dom::value("folder-picker-location");
    dom::text("folder-picker-status", "Reading folders…");
    match api::post("/api/projects/browse", body).await {
        Ok(v) if m::current("folder", epoch) => {
            m::set("folder", v.clone());
            if dom::value("folder-picker-location") == typed {
                dom::set_value("folder-picker-location", &m::text(&v, "path"));
            }
            dom::text("folder-picker-current", &m::text(&v, "path"));
            dom::clear("folder-picker-list");
            for entry in m::array(&v["entries"]) {
                dom::append(
                    "folder-picker-list",
                    &dom::button(&m::summary(&entry), "folder-enter", entry["path"].clone()),
                );
            }
            dom::text(
                "folder-picker-status",
                &format!(
                    "{} folders{}",
                    m::array(&v["entries"]).len(),
                    if m::yes(&v["truncated"]) {
                        " · listing limited"
                    } else {
                        ""
                    }
                ),
            );
            dom::disabled("folder-picker-use", v["valid"] == false);
            dom::disabled("folder-picker-parent", v["parent"].is_null());
            true
        }
        Ok(_) => false,
        Err(e) => {
            dom::error("folder-picker-error", &e.message);
            false
        }
    }
}
pub(super) async fn folder_use() {
    let data = m::data("folder");
    let path = m::text(&data, "path");
    if path.is_empty() || data["valid"] == false {
        return;
    }
    match api::post("/api/projects", json!({"path":path})).await {
        Ok(v) => {
            registered(v["projects"].clone());
            let project = m::text(&v["project"], "id");
            let purpose = m::text(
                &m::data("folder-purpose")
                    .as_str()
                    .map(|s| json!({"v":s}))
                    .unwrap_or(V::Null),
                "v",
            );
            dom::dialog("folder-picker", false);
            if purpose.starts_with("service:") {
                systems::add_service(
                    &project,
                    purpose
                        .strip_prefix("service:")
                        .and_then(|s| s.parse().ok()),
                )
                .await;
            } else {
                session::new_session(Some(&project));
                if purpose == "system-project" {
                    systems::editor_open().await;
                } else {
                    session::load_git().await;
                }
            }
        }
        Err(e) => dom::error("folder-picker-error", &e.message),
    }
}
pub fn invalidate(scope: &str) {
    m::next(&format!("{scope}-preview"));
    m::take(&format!("{scope}-preview"));
    if cfg!(target_arch = "wasm32") {
        dom::hidden(&format!("{scope}-preview"), true);
        let button = match scope {
            "setup" => "setup-install-button",
            "skills" => "skills-install",
            "create-skill" => "create-skill-save",
            _ => "",
        };
        dom::disabled(button, true);
    }
}
pub(super) fn selection(scope: &str) -> V {
    match scope {
        "setup" => {
            let mut tools = dom::selected_checks("setup-tools");
            tools.sort_by_key(V::to_string);
            json!({"project_id":dom::value("setup-project"),"edition":dom::value("setup-edition"),"tools":tools})
        }
        "skills" => {
            let mut skills = dom::selected_checks("skills-list");
            skills.sort_by_key(V::to_string);
            let mut agents = dom::selected_checks("skills-agents");
            agents.sort_by_key(V::to_string);
            json!({"project_id":dom::value("skills-project"),"source_id":dom::value("skills-source"),"skills":skills,"agents":agents})
        }
        _ => {
            json!({"project_id":dom::value("create-skill-project"),"name":dom::value("create-skill-name").trim(),"description":dom::value("create-skill-description").trim(),"instructions":dom::value("create-skill-instructions"),"agents":dom::selected_checks("create-skill-agents")})
        }
    }
}
pub(super) fn preview_controls(scope: &str) {
    let preview = m::data(&format!("{scope}-preview"));
    let key = selection(scope).to_string();
    let same = preview["selection"] == key;
    let expires = preview["expires_at"]
        .as_f64()
        .map(|at| at * 1000. > js_sys::Date::now())
        .unwrap_or(true);
    let ready = m::yes(&preview["can_install"])
        && same
        && expires
        && !m::yes(&m::data(&format!("{scope}-pending")));
    dom::disabled(
        match scope {
            "setup" => "setup-install-button",
            "skills" => "skills-install",
            _ => "create-skill-save",
        },
        !ready,
    );
    if scope == "skills" {
        skills_controls();
    } else if scope == "create-skill" {
        create_skill_controls();
    }
    crate::session::sync_working_indicator();
}
pub(super) fn setup_file_key(file: &V, duplicate_index: usize) -> String {
    let action = file["action"]
        .as_str()
        .or_else(|| file["status"].as_str())
        .unwrap_or("File");
    let path = m::text(file, "path");
    if duplicate_index == 0 {
        format!("file:{action}:{path}")
    } else {
        format!("file:{action}:{path}#{duplicate_index}")
    }
}

pub(super) fn build_file_preview_details(item: &V) -> web_sys::Element {
    let label = m::text(item, "label");
    let body = m::text(item, "body");
    let key = m::text(item, "key");
    let d = dom::details(&label, &body);
    let _ = d.set_attribute("data-restore-key", &key);
    if let Some(summary) = d.first_element_child() {
        let _ = summary.set_attribute("data-focus-key", &format!("summary:{key}"));
    }
    d
}

pub(super) fn file_preview_item(file: &V, duplicate_index: usize) -> V {
    let action = file["action"]
        .as_str()
        .or_else(|| file["status"].as_str())
        .unwrap_or("File");
    let path = m::text(file, "path");
    let label = format!("{action} · {path}");
    let before = m::string(&file["before_bytes"]);
    let after = m::string(&file["after_bytes"]);
    let bytes = if !before.is_empty() || !after.is_empty() {
        format!("{before} → {after} bytes")
    } else {
        String::new()
    };
    let reason = m::text(file, "reason");
    let diff = m::text(file, "diff");
    let truncated = if m::yes(&file["diff_truncated"]) {
        "Diff truncated; inspect the complete file locally."
    } else {
        ""
    };
    let body = [bytes.as_str(), reason.as_str(), diff.as_str(), truncated]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    json!({
        "key": setup_file_key(file, duplicate_index),
        "label": label,
        "body": body,
    })
}

pub(super) fn build_keyed_file_items(files: &[V]) -> Vec<V> {
    let mut seen = std::collections::BTreeMap::<String, usize>::new();
    files
        .iter()
        .map(|file| {
            let base = setup_file_key(file, 0);
            let idx = *seen
                .entry(base.clone())
                .and_modify(|n| *n += 1)
                .or_insert(0);
            file_preview_item(file, idx)
        })
        .collect()
}

pub(super) fn file_list(id: &str, files: &[V]) {
    let items = build_keyed_file_items(files);
    dom::keyed_render(
        id,
        &items,
        |item| m::text(item, "key"),
        build_file_preview_details,
    );
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SetupCheckCard {
    pub(super) key: String,
    pub(super) title: String,
    pub(super) state: String,
    pub(super) detail: String,
    pub(super) ok: bool,
}

pub(super) fn readiness_cards(readiness: &V) -> Vec<SetupCheckCard> {
    const KNOWN: [(&str, &str, &str); 3] = [
        ("policy", "Policy guide", "AGENTS.md and stack policy rules"),
        (
            "memory_runtime",
            "Memory runtime",
            "memory-bank runtime & database",
        ),
        (
            "brain_runtime",
            "Project brain",
            "project-brain runtime & state",
        ),
    ];
    if let Some(obj) = readiness.as_object() {
        let mut cards = Vec::new();
        for (key, title, detail) in KNOWN {
            if let Some(val) = obj.get(key) {
                let ok = val.as_bool().unwrap_or_else(|| m::yes(&val["ok"]));
                let state = if let Some(s) = val
                    .get("status")
                    .and_then(|s| s.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    m::label(s)
                } else if ok {
                    "Ready".to_owned()
                } else {
                    "Not detected".to_owned()
                };
                cards.push(SetupCheckCard {
                    key: key.to_owned(),
                    title: title.to_owned(),
                    state,
                    detail: detail.to_owned(),
                    ok,
                });
            }
        }
        let mut extras = obj
            .keys()
            .filter(|k| !KNOWN.iter().any(|(known, _, _)| known == &k.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        extras.sort();
        for key in extras {
            if let Some(val) = obj.get(&key) {
                let ok = val.as_bool().unwrap_or_else(|| m::yes(&val["ok"]));
                cards.push(SetupCheckCard {
                    key: key.clone(),
                    title: m::label(&key),
                    state: if ok {
                        "Ready".to_owned()
                    } else {
                        "Not detected".to_owned()
                    },
                    detail: m::text(val, "detail"),
                    ok,
                });
            }
        }
        return cards;
    }
    m::array(readiness)
        .into_iter()
        .enumerate()
        .map(|(idx, item)| {
            let key = m::text(&item, "id");
            let key = if key.trim().is_empty() {
                format!("check-{idx}")
            } else {
                key
            };
            let title = [
                m::text(&item, "title"),
                m::text(&item, "name"),
                m::text(&item, "label"),
            ]
            .into_iter()
            .find(|s| !s.trim().is_empty())
            .unwrap_or_else(|| m::label(&key));
            let ok = item["ok"]
                .as_bool()
                .unwrap_or_else(|| m::yes(&item["ready"]) || m::yes(&item["installed"]));
            let status = m::text(&item, "status");
            let state = if !status.trim().is_empty() {
                m::label(status.trim())
            } else if ok {
                "Ready".to_owned()
            } else {
                "Not detected".to_owned()
            };
            let detail = m::text(&item, "detail");
            SetupCheckCard {
                key,
                title,
                state,
                detail,
                ok,
            }
        })
        .collect()
}

pub(super) fn setup_provider_card(item: &V, idx: usize) -> SetupCheckCard {
    let id = m::text(item, "id");
    let name = m::text(item, "name");
    let title = if !name.trim().is_empty() {
        name.trim().to_owned()
    } else if !id.trim().is_empty() {
        id.trim().to_owned()
    } else if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        s.to_owned()
    } else {
        format!("Provider {}", idx + 1)
    };
    let ok = item["available"]
        .as_bool()
        .unwrap_or_else(|| m::yes(&item["ok"]));
    let state = if ok {
        "Available".to_owned()
    } else {
        "Unavailable".to_owned()
    };
    let detail = [
        m::text(item, "detail"),
        m::text(item, "agent_control_detail"),
    ]
    .into_iter()
    .map(|s| s.trim().to_owned())
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    SetupCheckCard {
        key: if id.trim().is_empty() {
            title.clone()
        } else {
            id
        },
        title,
        state,
        detail,
        ok,
    }
}

pub(super) fn format_setup_diagnostic(item: &V) -> String {
    if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return s.to_owned();
    }
    let head = [
        m::text(item, "severity"),
        m::text(item, "code"),
        m::text(item, "title"),
    ]
    .into_iter()
    .map(|s| s.trim().to_owned())
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    let body = [
        m::text(item, "message"),
        m::text(item, "detail"),
        m::text(item, "summary"),
    ]
    .into_iter()
    .map(|s| s.trim().to_owned())
    .find(|s| !s.is_empty())
    .unwrap_or_default();
    match (head.is_empty(), body.is_empty()) {
        (false, false) => format!("{head}: {body}"),
        (false, true) => head,
        (true, false) => body,
        (true, true) => m::summary(item),
    }
}

fn render_setup_card(class: &str, card: &SetupCheckCard) -> web_sys::Element {
    let el = dom::el("div", class, "");
    let _ = el.set_attribute("data-ok", if card.ok { "true" } else { "false" });
    dom::add(&el, &dom::el("strong", "", &card.title));
    dom::add(&el, &dom::el("span", "setup-check-state", &card.state));
    if !card.detail.trim().is_empty() {
        dom::add(&el, &dom::el("span", "", &card.detail));
    }
    el
}

pub async fn load_setup() {
    invalidate("setup");
    let project = dom::value("setup-project");
    if project.is_empty() {
        dom::hidden("setup-readiness", true);
        return;
    }
    match api::scoped(
        "setup",
        &format!("/api/projects/{}/setup", dom::enc(&project)),
    )
    .await
    {
        Ok((e, v)) if m::current("setup", e) && dom::value("setup-project") == project => {
            if v["project_id"] != project || !v["editions"].is_array() || !v["tools"].is_array() {
                dom::hidden("setup-readiness", true);
                dom::error(
                    "setup-readiness-error",
                    "Incomplete project setup information. Refresh before installing.",
                );
                return;
            }
            dom::text("setup-readiness-error", "");
            dom::hidden("setup-readiness-error", true);
            m::set("setup", v.clone());
            dom::text("setup-project-location", &m::text(&v, "path"));
            dom::options(
                "setup-edition",
                &m::array(&v["editions"]),
                m::summary,
                Some(&dom::value("setup-edition")),
            );
            dom::clear("setup-tools");
            let preferred = dom::value("provider");
            for tool in m::array(&v["tools"]) {
                let id = m::text(&tool, "id");
                dom::append(
                    "setup-tools",
                    &dom::check(
                        &format!(
                            "{}{}",
                            m::summary(&tool),
                            if m::yes(&tool["installed"]) {
                                " · files detected"
                            } else {
                                ""
                            }
                        ),
                        "setup-tool",
                        &id,
                        id == preferred,
                    ),
                );
            }
            dom::clear("setup-readiness-grid");
            let readiness = readiness_cards(&v["readiness"]);
            if readiness.is_empty() {
                dom::append(
                    "setup-readiness-grid",
                    &dom::el("p", "fleet-note", "No readiness checks reported."),
                );
            } else {
                for card in &readiness {
                    dom::append(
                        "setup-readiness-grid",
                        &render_setup_card("setup-check", card),
                    );
                }
            }
            dom::clear("setup-providers");
            let providers = m::array(&v["providers"]);
            if providers.is_empty() {
                dom::append(
                    "setup-providers",
                    &dom::el("p", "fleet-note", "No providers reported."),
                );
            } else {
                for (idx, item) in providers.iter().enumerate() {
                    let card = setup_provider_card(item, idx);
                    dom::append(
                        "setup-providers",
                        &render_setup_card("setup-provider", &card),
                    );
                }
            }
            dom::clear("setup-diagnostics");
            for item in m::array(&v["diagnostics"]) {
                let text = format_setup_diagnostic(&item);
                if !text.trim().is_empty() {
                    let callout =
                        dom::el("div", "system-side-callout system-side-callout-warn", "");
                    dom::add(&callout, &dom::el("p", "", text.trim()));
                    dom::append("setup-diagnostics", &callout);
                }
            }
            dom::hidden("setup-readiness", false);
            dom::text(
                "setup-readiness-status",
                if v["available"] == false {
                    "This registered project is currently unavailable."
                } else {
                    "Project inspected."
                },
            );
            let no_editions = m::array(&v["editions"]).is_empty();
            dom::disabled("setup-edition", v["available"] == false || no_editions);
            dom::disabled("setup-preview-button", v["available"] == false);
        }
        Ok(_) => {}
        Err(e) if !e.aborted => {
            dom::hidden("setup-readiness", true);
            dom::error("setup-readiness-error", &e.message);
        }
        Err(_) => {}
    }
}
pub async fn load_attachment() {
    let p = dom::value("project");
    if p.is_empty() {
        return;
    }
    let path = format!("/api/projects/{}/accelerator", dom::enc(&p));
    if let Ok((e, v)) = api::scoped("attachment", &path).await {
        if m::current("attachment", e) && p == dom::value("project") {
            let editions = m::array(&v["editions"])
                .into_iter()
                .filter_map(|edition| edition.as_str().map(|id| json!({"id":id,"name":id})))
                .collect::<Vec<_>>();
            let preferred = v["edition"]
                .as_str()
                .or_else(|| v["detected"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| dom::value("setup-attach-edition"));
            dom::options(
                "setup-attach-edition",
                &editions,
                m::summary,
                Some(&preferred),
            );
            m::set("attachment", v.clone());
            dom::text("setup-attach-summary", &m::summary(&v));
            dom::text("project-panel-path", &m::text(&m::project(&p), "path"));
            dom::disabled("setup-detach-button", v["mode"] != "attached");
            dom::text("setup-attach-codex-status", &m::summary(&v["codex_hooks"]));
        }
    }
}
pub(super) async fn attach(action: &str) {
    let p = dom::value("project");
    let mut body = json!({"action":action});
    if action == "attach" {
        body["edition"] = json!(dom::value("setup-attach-edition"));
    }
    match api::post(&format!("/api/projects/{}/accelerator", dom::enc(&p)), body).await {
        Ok(v) => {
            if v["projects"].is_array() {
                registered(v["projects"].clone());
            }
            m::set("attachment", v);
            load_attachment().await;
        }
        Err(e) => dom::error("setup-attach-error", &e.message),
    }
}
pub(super) async fn hooks(trust: bool) {
    let p = dom::value("project");
    let url = format!("/api/projects/{}/accelerator/codex-hooks", dom::enc(&p));
    let result = if trust {
        api::post(&url, json!({})).await
    } else {
        api::get(&url).await
    };
    match result {
        Ok(v) => dom::text("setup-attach-codex-status", &m::summary(&v)),
        Err(e) => dom::error("setup-attach-error", &e.message),
    }
}
pub(super) async fn preview(scope: &str) {
    let draft = selection(scope);
    if scope == "create-skill" {
        let check = validate_create_skill_inputs(
            &m::text(&draft, "project_id"),
            &m::text(&draft, "name"),
            &m::text(&draft, "description"),
            &m::text(&draft, "instructions"),
            m::array(&draft["agents"]).len(),
        );
        if !check.valid {
            dom::error("create-skill-action-error", check.summary_error);
            create_skill_controls();
            return;
        }
    }
    let epoch = m::next(&format!("{scope}-preview"));
    let signature = draft.to_string();
    m::set(&format!("{scope}-pending"), json!(true));
    preview_controls(scope);
    let endpoint = match scope {
        "setup" => "/api/accelerators/preview",
        "skills" => "/api/skills/preview",
        _ => "/api/skills/create-preview",
    };
    match api::post(endpoint, draft.clone()).await {
        Ok(mut v)
            if m::current(&format!("{scope}-preview"), epoch) && {
                let current_signature = selection(scope).to_string();
                current_signature == signature
            } =>
        {
            if !v["preview_id"].is_string()
                || !v["files"].is_array()
                || !v["can_install"].is_boolean()
            {
                dom::error(
                    &format!("{scope}-action-error"),
                    "Incomplete preview. Prepare a new preview before installing.",
                );
            } else {
                v["selection"] = json!(signature);
                m::set(&format!("{scope}-preview"), v.clone());
                dom::hidden(&format!("{scope}-preview"), false);
                dom::text(&format!("{scope}-preview-summary"), &m::summary(&v));
                if scope == "setup" {
                    let edition = m::text(&draft, "edition");
                    let edition = if edition == "Cms/wordpress" {
                        "WordPress"
                    } else {
                        &edition
                    };
                    let tools = m::array(&draft["tools"])
                        .iter()
                        .filter_map(V::as_str)
                        .map(|tool| match tool {
                            "claude" => "Claude Code",
                            "codex" => "Codex",
                            "cursor" => "Cursor",
                            other => other,
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    dom::text(
                        "setup-preview-target",
                        &format!("{edition} · Tools: {tools}"),
                    );
                } else {
                    dom::text(&format!("{scope}-preview-context"), &draft.to_string());
                }
                dom::text(
                    &format!("{scope}-preview-note"),
                    if m::yes(&v["can_install"]) {
                        "Review this exact file preview; it can be used once."
                    } else {
                        "Blocked. Resolve reported issues and prepare a new preview."
                    },
                );
                file_list(&format!("{scope}-preview-files"), &m::array(&v["files"]));
                if scope == "setup" {
                    dom::error(
                        "setup-collisions",
                        &if m::array(&v["collisions"]).is_empty() {
                            String::new()
                        } else {
                            format!(
                                "{} collisions block installation.",
                                m::array(&v["collisions"]).len()
                            )
                        },
                    );
                }
                if scope == "create-skill" {
                    dom::text("create-skill-content", &m::text(&v, "content"));
                }
                if let Some(at) = v["expires_at"].as_f64() {
                    let scope = scope.to_owned();
                    let ms = (at * 1000. - js_sys::Date::now())
                        .max(0.)
                        .min(i32::MAX as f64) as i32;
                    dom::later(&format!("{scope}-expiry"), ms + 5, move || {
                        preview_controls(&scope)
                    });
                }
            }
        }
        Ok(_) => {}
        Err(e) => dom::error(&format!("{scope}-action-error"), &e.message),
    }
    m::set(&format!("{scope}-pending"), json!(false));
    preview_controls(scope);
}
pub(super) async fn install(scope: &str) {
    let preview = m::data(&format!("{scope}-preview"));
    let signature = selection(scope).to_string();
    if preview["selection"] != signature
        || !m::yes(&preview["can_install"])
        || !preview["preview_id"].is_string()
        || preview["expires_at"]
            .as_f64()
            .map(|n| n * 1000. <= js_sys::Date::now())
            .unwrap_or(false)
    {
        invalidate(scope);
        return;
    }
    m::take(&format!("{scope}-preview"));
    preview_controls(scope);
    m::set(&format!("{scope}-pending"), json!(true));
    let project = dom::value(&format!("{scope}-project"));
    let url = if scope == "setup" {
        "/api/accelerators/install"
    } else {
        "/api/skills/install"
    };
    dom::text(
        &format!("{scope}-preview-note"),
        "Preview consumed. Create a new preview before another installation.",
    );
    match api::post(url,json!({"preview_id":preview["preview_id"]})).await{Ok(v)=>{
  m::set(&format!("{scope}-result"),v.clone());m::set(&format!("{scope}-result-project"),json!(project));dom::hidden(&format!("{scope}-result"),false);dom::text(&format!("{scope}-result-summary"),&m::summary(&v));
  let mut rows=vec![];for kind in ["installed","merged","repaired","unchanged","changed"]{for path in m::array(&v[kind]){rows.push(json!({"path":path,"status":kind}));}}file_list(if scope=="setup"{"setup-result-paths"}else if scope=="skills"{"skills-result-files"}else{"create-skill-result-files"},&rows);
  if v["ok"]!=true{dom::error(&format!("{scope}-action-error"),v["error"].as_str().unwrap_or("Installation may be partial. Inspect the reported files before retrying."));}
 },Err(e)=>dom::error(&format!("{scope}-action-error"),&format!("{} The preview is consumed; installation may have partially completed. Refresh before preparing another preview.",e.message))}
    m::set(&format!("{scope}-pending"), json!(false));
    dom::hidden(&format!("{scope}-preview"), true);
    if scope == "setup" {
        load_setup().await;
    } else {
        super::skills::installed_skills().await;
    }
}
pub(super) fn filtered_setup_files(preview: &V, query: &str, filter: &str) -> Vec<V> {
    let needle = query.trim().to_lowercase();
    let matches_query = |path: &str| needle.is_empty() || path.to_lowercase().contains(&needle);
    let mut rows = Vec::new();
    let mut seen_collision_paths = std::collections::BTreeSet::new();
    for f in m::array(&preview["files"]) {
        let path = m::text(&f, "path");
        if !matches_query(&path) {
            continue;
        }
        let action = f["action"].as_str().unwrap_or("");
        let include = match filter {
            "all" => true,
            "collision" => action == "collision",
            _ => action != "unchanged" && action != "kept",
        };
        if include {
            if action == "collision" {
                seen_collision_paths.insert(path);
            }
            rows.push(f);
        }
    }
    if matches!(filter, "all" | "collision") {
        for c in m::array(&preview["collisions"]) {
            let path = m::text(&c, "path");
            if path.is_empty() || !matches_query(&path) || seen_collision_paths.contains(&path) {
                continue;
            }
            let reason = m::text(&c, "reason");
            let detail = if reason.is_empty() {
                "Collision blocks installation.".to_owned()
            } else {
                format!("Collision blocks installation ({reason}).")
            };
            rows.push(json!({
                "path": path,
                "action": "collision",
                "reason": reason,
                "diff": detail,
            }));
        }
    }
    rows
}
pub(super) fn setup_preview_visibility(
    preview: &V,
    query: &str,
    filter: &str,
) -> (Vec<V>, std::collections::BTreeSet<String>) {
    let all_rows = filtered_setup_files(preview, "", "all");
    let visible_rows = filtered_setup_files(preview, query, filter);
    let items = build_keyed_file_items(&all_rows);
    let visible_keys: std::collections::BTreeSet<String> = build_keyed_file_items(&visible_rows)
        .into_iter()
        .map(|it| m::text(&it, "key"))
        .collect();
    (items, visible_keys)
}

pub(super) fn setup_files() {
    let v = m::data("setup-preview");
    let query = dom::value("setup-preview-path");
    let filter = dom::value("setup-preview-actions");
    let (items, visible_keys) = setup_preview_visibility(&v, &query, &filter);
    dom::text(
        "setup-preview-count",
        &format!("{} files shown", visible_keys.len()),
    );
    dom::keyed_render(
        "setup-preview-files",
        &items,
        |item| m::text(item, "key"),
        build_file_preview_details,
    );
    if let Some(container) = dom::get("setup-preview-files") {
        for child in dom::children(&container) {
            if let Some(key) = child.get_attribute("data-key") {
                dom::hide(&child, !visible_keys.contains(&key));
            }
        }
    }
}
