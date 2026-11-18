use crate::{api, dom, model as m, run, session, systems};
use serde_json::{json, Value as V};
use wasm_bindgen_futures::spawn_local;
pub fn initialize() {
    for k in ["setup", "skills", "create-skill"] {
        invalidate(k);
    }
    dom::set_value("creator-count", "3");
}
pub fn creator_models() {
    let provider = m::provider(&dom::value("creator-provider"));
    let metadata = &provider["model_options"];
    let models = m::array(&metadata["models"]);
    dom::clear("creator-models");
    for model in &models {
        let option = dom::el("option", "", "");
        let _ = option.set_attribute("value", &m::text(model, "id"));
        let _ = option.set_attribute("label", &m::text(model, "label"));
        dom::append("creator-models", &option);
    }
    let selected = dom::value("creator-model").trim().to_owned();
    let model = models.iter().find(|model| model["id"] == selected);
    let efforts = model
        .filter(|model| model["efforts"].is_array())
        .map(|model| m::array(&model["efforts"]))
        .unwrap_or_else(|| m::array(&metadata["efforts"]));
    let mut options = vec![json!({"id":"","name":"Provider default"})];
    options.extend(
        efforts
            .into_iter()
            .map(|effort| json!({"id":effort,"name":m::label(effort.as_str().unwrap_or(""))})),
    );
    dom::options("creator-effort", &options, m::summary, None);
    dom::text(
        "creator-model-detail",
        metadata["detail"].as_str().unwrap_or(""),
    );
    creator_permissions_hint();
}

pub fn registered(projects: V) {
    let old = dom::value("project");
    m::write(|s| s.boot["projects"] = projects);
    session::populate();
    if !m::project(&old).is_null() {
        session::sync_project(&old);
    }
}
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
async fn folder(path: &str, query: bool) -> bool {
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
async fn folder_use() {
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
    dom::hidden(&format!("{scope}-preview"), true);
    let button = match scope {
        "setup" => "setup-install-button",
        "skills" => "skills-install",
        "create-skill" => "create-skill-save",
        _ => "",
    };
    dom::disabled(button, true);
}
fn selection(scope: &str) -> V {
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
fn preview_controls(scope: &str) {
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
fn file_list(id: &str, files: &[V]) {
    dom::clear(id);
    for file in files {
        let label = format!(
            "{} · {}",
            file["action"]
                .as_str()
                .or_else(|| file["status"].as_str())
                .unwrap_or("File"),
            m::text(file, "path")
        );
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
        dom::append(id, &dom::details(&label, &body));
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct SetupCheckCard {
    key: String,
    title: String,
    state: String,
    detail: String,
    ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CreatorReportView {
    key: String,
    summary: String,
    text: String,
}

fn readiness_cards(readiness: &V) -> Vec<SetupCheckCard> {
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

fn setup_provider_card(item: &V, idx: usize) -> SetupCheckCard {
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

fn format_setup_diagnostic(item: &V) -> String {
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

fn creator_report_view(item: &V, idx: usize) -> CreatorReportView {
    let raw_name = m::text(item, "name");
    let name = if raw_name.trim().is_empty() {
        format!("report-{}", idx + 1)
    } else {
        raw_name.trim().to_owned()
    };
    let text = if !m::text(item, "text").is_empty() {
        m::text(item, "text")
    } else {
        m::summary(item)
    };
    let mut meta = vec![format!("{} chars", text.chars().count())];
    if m::yes(&item["truncated"]) {
        meta.push("truncated".to_owned());
    }
    let hash = m::text(item, "hash");
    if !hash.trim().is_empty() {
        let short: String = hash.trim().chars().take(8).collect();
        meta.push(format!("#{short}"));
    }
    CreatorReportView {
        key: format!("{idx}:{name}"),
        summary: format!("{name} ({})", meta.join(" · ")),
        text,
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
async fn attach(action: &str) {
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
async fn hooks(trust: bool) {
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
#[derive(Debug, Clone, PartialEq, Eq)]
struct CreateSkillValidation {
    valid: bool,
    name_error: &'static str,
    description_error: &'static str,
    instructions_error: &'static str,
    agents_error: &'static str,
    summary_error: &'static str,
}

fn valid_skill_slug(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn validate_create_skill_inputs(
    project: &str,
    name: &str,
    description: &str,
    instructions: &str,
    agent_count: usize,
) -> CreateSkillValidation {
    let name_valid = valid_skill_slug(name);
    let desc_trimmed = description.trim();
    let desc_chars = desc_trimmed.chars().count();
    let desc_valid = !desc_trimmed.is_empty() && desc_chars <= 1024;
    let inst_trimmed = instructions.trim();
    let inst_bytes = instructions.len();
    let inst_valid = !inst_trimmed.is_empty() && inst_bytes <= 32000;
    let agents_valid = agent_count > 0;

    let name_error = if !name.is_empty() && !name_valid {
        "Use lowercase letters, digits, and single hyphens (up to 64 chars)."
    } else {
        ""
    };
    let description_error = if desc_chars > 1024 {
        "Use a description of at most 1,024 characters."
    } else {
        ""
    };
    let instructions_error = if inst_bytes > 32000 {
        "Use instructions of at most 32,000 UTF-8 bytes."
    } else {
        ""
    };
    let agents_error = if agents_valid {
        ""
    } else {
        "Choose at least one tool."
    };
    let summary_error = if project.is_empty() {
        "Choose a project first."
    } else if !name_valid {
        "Use a lowercase hyphen-separated skill name."
    } else if !desc_valid {
        "Use a description of 1–1024 characters."
    } else if !inst_valid {
        "Use instructions of 1–32,000 UTF-8 bytes."
    } else if !agents_valid {
        "Choose at least one tool."
    } else {
        ""
    };
    let valid = summary_error.is_empty();
    CreateSkillValidation {
        valid,
        name_error,
        description_error,
        instructions_error,
        agents_error,
        summary_error,
    }
}

fn skills_controls() {
    let catalog = m::data("skills-catalog");
    let loaded = catalog.is_object();
    let available = loaded && catalog["available"] != false;
    let has_sources = !m::array(&catalog["sources"]).is_empty();
    let has_source = available && has_sources && !dom::value("skills-source").is_empty();
    let discovery = m::data("skills-discovery");
    let has_items = discovery["skills"]
        .as_array()
        .or_else(|| discovery["items"].as_array())
        .is_some_and(|items| !items.is_empty());
    let pending = m::yes(&m::data("skills-pending"));
    let selected_skills = m::array(&m::data("skills-selection"));
    let selected_agents = dom::selected_checks("skills-agents");

    dom::disabled("skills-source", !available || !has_sources);
    dom::disabled("skills-discover", !has_source || pending);
    dom::disabled("skills-source-refresh", !has_source || pending);
    dom::disabled("skills-filter", !available || !has_items);
    dom::disabled(
        "skills-preview-button",
        !available
            || pending
            || dom::value("skills-project").is_empty()
            || selected_skills.is_empty()
            || selected_agents.is_empty(),
    );
    dom::text(
        "skills-selection-summary",
        &if selected_skills.is_empty() || selected_agents.is_empty() {
            "Select at least one skill and one tool.".to_owned()
        } else {
            format!(
                "{} skill{} for {} tool{} ready to preview.",
                selected_skills.len(),
                if selected_skills.len() == 1 { "" } else { "s" },
                selected_agents.len(),
                if selected_agents.len() == 1 { "" } else { "s" }
            )
        },
    );
}

fn create_skill_controls() {
    let catalog = m::data("skills-catalog");
    let loaded = catalog.is_object();
    let available = loaded && catalog["available"] != false;
    let agent_total = m::array(&catalog["agents"]).len();
    if loaded {
        dom::text(
            "create-skill-catalog-status",
            &if available {
                format!("Supported tools ready ({agent_total}).")
            } else {
                catalog["detail"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("Skill creation is currently unavailable.")
                    .to_owned()
            },
        );
    }
    let name = dom::value("create-skill-name");
    let trimmed_name = name.trim();
    let desc = dom::value("create-skill-description");
    let inst = dom::value("create-skill-instructions");
    let agents = dom::selected_checks("create-skill-agents");
    let project = dom::value("create-skill-project");
    let check = validate_create_skill_inputs(&project, trimmed_name, &desc, &inst, agents.len());

    dom::error("create-skill-name-error", check.name_error);
    dom::error("create-skill-description-error", check.description_error);
    dom::error("create-skill-instructions-error", check.instructions_error);
    dom::error(
        "create-skill-agents-error",
        if loaded && available {
            check.agents_error
        } else {
            ""
        },
    );
    let pending = m::yes(&m::data("create-skill-pending"));
    dom::disabled(
        "create-skill-preview-button",
        !available || pending || !check.valid,
    );
}

async fn preview(scope: &str) {
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
async fn install(scope: &str) {
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
        installed_skills().await;
    }
}
fn filtered_setup_files(preview: &V, query: &str, filter: &str) -> Vec<V> {
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
fn setup_files() {
    let v = m::data("setup-preview");
    let query = dom::value("setup-preview-path");
    let filter = dom::value("setup-preview-actions");
    let files = filtered_setup_files(&v, &query, &filter);
    dom::text(
        "setup-preview-count",
        &format!("{} files shown", files.len()),
    );
    file_list("setup-preview-files", &files);
}
pub fn accelerators() {
    let boot = m::read(|s| s.boot.clone());
    dom::clear("accelerators");
    let kits = m::array(&boot["accelerators"]);
    for (index, item) in kits.iter().enumerate() {
        let card = dom::el("article", "accelerator", "");
        dom::add(
            &card,
            &dom::el("div", "kit-number", &format!("Kit {}", index + 1)),
        );
        let content = dom::el("div", "", "");
        dom::add(&content, &dom::el("h3", "", &m::summary(item)));
        dom::add(&content, &dom::el("p", "", &m::text(item, "description")));
        let actions = dom::el("div", "accelerator-actions", "");
        let identity = format!("{} {}", m::text(item, "id"), m::text(item, "name")).to_lowercase();
        if item["editions"].is_array() {
            let label = dom::el("label", "field", "Edition");
            let select = dom::el("select", "", "");
            let select_id = format!("preview-edition-{index}");
            let _ = select.set_attribute("id", &select_id);
            for edition in m::array(&item["editions"]) {
                let id = edition
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| m::text(&edition, "id"));
                let name = edition
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| m::summary(&edition));
                let option = dom::el("option", "", &name);
                let _ = option.set_attribute("value", &id);
                dom::add(&select, &option);
            }
            dom::add(&label, &select);
            dom::add(&actions, &label);
            dom::add(
                &actions,
                &dom::button(
                    "Install into the project",
                    "accelerator-setup",
                    json!(select_id),
                ),
            );
            dom::add(&content, &actions);
            dom::add(&content,&dom::el("p","small-note","Sessions already use this edition without installing it. Install to put its files in the project's Git: choose target tools and review file changes under Install into project."));
            let details = dom::el("details", "fleet-report startup-context", "");
            dom::add(
                &details,
                &dom::el("summary", "", "Startup context per edition"),
            );
            dom::add(
                &details,
                &dom::button(
                    "Measure startup context",
                    "accelerator-startup",
                    json!(index),
                ),
            );
            let output = dom::el("div", "", "");
            let _ = output.set_attribute("id", &format!("startup-context-{index}"));
            dom::add(&details, &output);
            dom::add(&content, &details);
        } else {
            let catalog = identity.contains("kit3")
                || identity.contains("open-source")
                || identity.contains("open source");
            dom::add(
                &actions,
                &dom::button(
                    if catalog {
                        "Explore the catalog"
                    } else {
                        "Open Infrastructure Creator"
                    },
                    if catalog {
                        "accelerator-catalog"
                    } else {
                        "accelerator-creator"
                    },
                    V::Null,
                ),
            );
            dom::add(&content, &actions);
        }
        dom::add(&card, &content);
        dom::append("accelerators", &card);
    }
    if kits.is_empty() {
        dom::append(
            "accelerators",
            &dom::el(
                "p",
                "empty-note",
                "No accelerators were reported by this runner.",
            ),
        );
    }
}
fn startup_table(data: &V, index: u64) {
    let id = format!("startup-context-{index}");
    dom::clear(&id);
    let table = dom::el("table", "memory-table", "");
    let head = dom::el("tr", "", "");
    let labels = data["labels"].as_object().cloned().unwrap_or_default();
    for name in std::iter::once("Edition".to_owned())
        .chain(labels.values().map(|v| m::label(v.as_str().unwrap_or(""))))
        .chain(
            ["Startup total", "≈ tokens", "CI ceiling"]
                .into_iter()
                .map(str::to_owned),
        )
    {
        let cell = dom::el("th", "", &name);
        let _ = cell.set_attribute("scope", "col");
        dom::add(&head, &cell);
    }
    let thead = dom::el("thead", "", "");
    dom::add(&thead, &head);
    dom::add(&table, &thead);
    let body = dom::el("tbody", "", "");
    for row in m::array(&data["editions"]) {
        let tr = dom::el("tr", "", "");
        let name = dom::el("th", "", &m::text(&row, "edition"));
        let _ = name.set_attribute("scope", "row");
        dom::add(&tr, &name);
        if let Some(error) = row["error"].as_str() {
            let cell = dom::el("td", "", error);
            let _ = cell.set_attribute("colspan", &(labels.len() + 3).to_string());
            dom::add(&tr, &cell);
        } else {
            for key in labels.keys() {
                dom::add(
                    &tr,
                    &dom::el(
                        "td",
                        "context-number",
                        &format!("{} B", m::string(&row["bytes"][key])),
                    ),
                );
            }
            let total = dom::el(
                "td",
                "context-number startup-total",
                &format!("{} B", m::string(&row["total"])),
            );
            let bar = dom::el("span", "startup-bar", "");
            let fill = dom::el("span", "", "");
            let ceiling = row["ceiling_total"].as_f64().unwrap_or(0.);
            let used = if ceiling > 0. {
                row["total"].as_f64().unwrap_or(0.) / ceiling * 100.
            } else {
                0.
            };
            let _ = fill.set_attribute("style", &format!("width:{}%", used.min(100.)));
            dom::add(&bar, &fill);
            dom::add(&total, &bar);
            dom::add(&tr, &total);
            dom::add(
                &tr,
                &dom::el(
                    "td",
                    "context-number",
                    &format!("≈ {} tokens", m::string(&row["tokens"])),
                ),
            );
            dom::add(
                &tr,
                &dom::el(
                    "td",
                    "context-number",
                    &format!("{} B · {used:.0}% used", m::string(&row["ceiling_total"])),
                ),
            );
        }
        dom::add(&body, &tr);
    }
    dom::add(&table, &body);
    dom::append(&id, &table);
    dom::append(&id,&dom::el("p","knowledge-note",&format!("Exact startup bytes per edition. Token counts use the measured calibration {}; CI gates the configured ceilings.",m::text(data,"calibration"))));
}

pub async fn load_skills() {
    match api::scoped("skills-catalog", "/api/skills").await {
        Ok((e, v)) if m::current("skills-catalog", e) => {
            if !v["sources"].is_array() || !v["agents"].is_array() {
                dom::error("skills-catalog-error", "Incomplete skill catalog.");
                return;
            }
            m::set("skills-catalog", v.clone());
            dom::options("skills-source", &m::array(&v["sources"]), m::summary, None);
            dom::text(
                "skills-availability",
                v["detail"]
                    .as_str()
                    .unwrap_or("Discover a source before selecting skills."),
            );
            for id in ["skills-agents", "create-skill-agents"] {
                let selected = dom::selected_checks(id);
                dom::clear(id);
                for a in m::array(&v["agents"]) {
                    let name = m::text(&a, "id");
                    let preferred = if selected.is_empty() {
                        name == dom::value("provider")
                    } else {
                        selected.iter().any(|v| v == &name)
                    };
                    dom::append(
                        id,
                        &dom::check(
                            &m::summary(&a),
                            if id == "skills-agents" {
                                "skills-agent"
                            } else {
                                "create-skill-agent"
                            },
                            &name,
                            preferred,
                        ),
                    );
                }
            }
            dom::options(
                "skills-installed-agent",
                &{
                    let mut a = vec![json!({"id":"","name":"All tools"})];
                    a.extend(m::array(&v["agents"]));
                    a
                },
                m::summary,
                None,
            );
            installed_skills().await;
            skills_controls();
            create_skill_controls();
        }
        Ok(_) => {}
        Err(e) if !e.aborted => {
            dom::error("skills-catalog-error", &e.message);
            dom::text("create-skill-catalog-status", &e.message);
            skills_controls();
            create_skill_controls();
        }
        Err(_) => {}
    }
}
async fn discover(refresh: bool) {
    invalidate("skills");
    let source = dom::value("skills-source");
    let epoch = m::next("skills-discovery");
    dom::text(
        "skills-availability",
        "Loading source; this can take minutes…",
    );
    match api::post(
        "/api/skills/discover",
        json!({"source_id":source,"refresh":refresh}),
    )
    .await
    {
        Ok(v) if m::current("skills-discovery", epoch) && dom::value("skills-source") == source => {
            m::set("skills-discovery", v.clone());
            m::set("skills-selection", json!([]));
            render_skills();
            dom::text("skills-availability", &m::summary(&v));
        }
        Ok(_) => {}
        Err(e) => dom::error("skills-catalog-error", &e.message),
    }
    skills_controls();
}
fn render_skills() {
    let query = dom::value("skills-filter").to_lowercase();
    let selected = m::array(&m::data("skills-selection"));
    let v = m::data("skills-discovery");
    let items = if v["skills"].is_array() {
        m::array(&v["skills"])
    } else {
        m::array(&v["items"])
    };
    dom::clear("skills-list");
    let mut count = 0;
    for skill in &items {
        let text = format!(
            "{} {} {}",
            m::text(skill, "id"),
            m::text(skill, "name"),
            m::text(skill, "description")
        );
        if !text.to_lowercase().contains(&query) {
            continue;
        }
        count += 1;
        let id = m::text(skill, "id");
        dom::append(
            "skills-list",
            &dom::check(
                &text,
                "skills-select",
                &id,
                selected.iter().any(|v| v == &id),
            ),
        );
    }
    dom::text("skills-visible-count", &format!("{count} matching"));
    dom::text(
        "skills-selection-count",
        &format!("{} selected of {}", selected.len(), items.len()),
    );
}
async fn installed_skills() {
    let p = dom::value("skills-project");
    if p.is_empty() {
        return;
    }
    match api::scoped(
        "skills-installed",
        &format!("/api/projects/{}/skills", dom::enc(&p)),
    )
    .await
    {
        Ok((e, v)) if m::current("skills-installed", e) && dom::value("skills-project") == p => {
            m::set("skills-installed", v);
            render_installed();
        }
        Ok(_) => {}
        Err(e) if !e.aborted => dom::error("skills-installed-error", &e.message),
        Err(_) => {}
    }
}
fn render_installed() {
    let v = m::data("skills-installed");
    let items = m::array(if v["skills"].is_array() {
        &v["skills"]
    } else {
        &v["installed"]
    });
    let agent = dom::value("skills-installed-agent");
    dom::clear("skills-installed-list");
    let mut count = 0;
    for skill in items {
        if !agent.is_empty() && skill["agent"] != agent {
            continue;
        }
        count += 1;
        let e = dom::el(
            "div",
            "skills-file installed-skill",
            &format!(
                "{} · {} · {} · {}",
                m::text(&skill, "name"),
                m::text(&skill, "agent"),
                m::text(&skill, "path"),
                m::text(&skill, "state")
            ),
        );
        if m::yes(&skill["managed"]) {
            for operation in ["update", "remove"] {
                if operation == "update" && skill["source_id"] == "local" {
                    continue;
                }
                let mut p = skill.clone();
                p["operation"] = json!(operation);
                dom::add(
                    &e,
                    &dom::button(
                        if operation == "update" {
                            "Check update"
                        } else {
                            "Preview removal"
                        },
                        "skill-change-preview",
                        p,
                    ),
                );
            }
        }
        dom::append("skills-installed-list", &e);
    }
    dom::text(
        "skills-installed-status",
        &format!("{count} installed copies"),
    );
}
async fn change_preview(skill: &V) {
    m::take("skills-change-preview");
    dom::disabled("skills-change-apply", true);
    let p = dom::value("skills-project");
    let body = json!({"project_id":p,"agent":skill["agent"],"name":skill["name"],"operation":skill["operation"]});
    match api::post("/api/skills/change-preview", body).await {
        Ok(v) => {
            if v["project_id"] != p
                || v["path"] != skill["path"]
                || v["operation"] != skill["operation"]
                || !v["preview_id"].is_string()
            {
                dom::error(
                    "skills-change-error",
                    "Incomplete or mismatched skill change preview.",
                );
                return;
            }
            m::set("skills-change-preview", v.clone());
            dom::hidden("skills-change", false);
            dom::text("skills-change-summary", &m::summary(&v));
            file_list("skills-change-files", &m::array(&v["files"]));
            dom::disabled("skills-change-apply", !m::yes(&v["can_apply"]));
            if let Some(at) = v["expires_at"].as_f64() {
                dom::later(
                    "skills-change-expiry",
                    ((at * 1000. - js_sys::Date::now())
                        .max(0.)
                        .min(i32::MAX as f64) as i32)
                        + 5,
                    || {
                        m::take("skills-change-preview");
                        dom::disabled("skills-change-apply", true);
                        dom::text(
                            "skills-change-note",
                            "This preview expired. Check the skill again.",
                        );
                    },
                );
            }
        }
        Err(e) => dom::error("skills-change-error", &e.message),
    }
}
async fn apply_change() {
    let preview = m::take("skills-change-preview");
    dom::disabled("skills-change-apply", true);
    if !m::yes(&preview["can_apply"])
        || !preview["preview_id"].is_string()
        || preview["expires_at"]
            .as_f64()
            .map(|v| v * 1000. <= js_sys::Date::now())
            .unwrap_or(true)
    {
        return;
    }
    match api::post(
        "/api/skills/apply",
        json!({"preview_id":preview["preview_id"]}),
    )
    .await
    {
        Ok(v) => {
            dom::pretty("skills-change-result", &v);
            dom::hidden("skills-change-result", false);
            if v["ok"] != true {
                dom::error(
                    "skills-change-error",
                    v["error"].as_str().unwrap_or(
                        "The operation stopped after a partial change. Inspect the reported files.",
                    ),
                );
            }
        }
        Err(e) => dom::error(
            "skills-change-error",
            &format!(
                "{} Some files may have changed. Refresh before retrying.",
                e.message
            ),
        ),
    }
    installed_skills().await;
}
fn normalized_creator_permission(value: Option<&str>) -> &'static str {
    match value {
        Some("manual") => "manual",
        Some("bypass") => "bypass",
        _ => "auto",
    }
}

fn creator_restored_permissions(run: &V) -> &'static str {
    session::restored_value(
        "permissions",
        normalized_creator_permission(run["permissions"].as_str()),
    )
}

fn creator_permissions_hint_text(provider: &str, permission: &str) -> &'static str {
    session::permissions_hint(provider, "edit", permission)
}

fn creator_permissions_hint() {
    let provider = dom::value("creator-provider");
    let permission = dom::value("creator-permissions");
    dom::text(
        "creator-permissions-hint",
        creator_permissions_hint_text(&provider, &permission),
    );
    dom::attr(
        "creator-permissions-hint",
        "data-mode",
        session::permissions_hint_mode(false, "edit", &permission),
    );
}

fn creator_target_summary(run: &V) -> String {
    let perm = match normalized_creator_permission(run["permissions"].as_str()) {
        "manual" => "Manual",
        "bypass" => "Bypass",
        _ => "Auto",
    };
    format!(
        "{} · {} · {} / {} · Permissions: {perm}",
        m::text(run, "target"),
        m::text(run, "workspace"),
        m::text(run, "provider"),
        m::text(run, "model")
    )
}

fn creator_status_text(run: &V, waiting: &[V]) -> String {
    let base = m::label(run["status"].as_str().unwrap_or(""));
    if creator_busy(run) && !waiting.is_empty() {
        format!("{base} · Waiting for your answer to a permission request…")
    } else {
        base
    }
}

fn creator_permissions_panel_state(v: &V) -> (bool, String) {
    let waiting = m::array(&v["waiting_permissions"]);
    (!waiting.is_empty(), run::permission_panel_lead(&waiting))
}

#[allow(clippy::too_many_arguments)]
fn creator_start_body(
    budgets: V,
    project: &str,
    provider: &str,
    model: V,
    thinking_effort: V,
    permissions: &str,
    operation: &str,
    workspace: &str,
    worktree_branch: String,
    agents_enabled: bool,
    agent_count: V,
    goal: &str,
    tools: Vec<String>,
) -> V {
    json!({
        "budgets": budgets,
        "project_id": project,
        "provider": provider,
        "model": model,
        "thinking_effort": thinking_effort,
        "permissions": normalized_creator_permission(Some(permissions)),
        "operation": operation,
        "workspace": workspace,
        "worktree_branch": worktree_branch,
        "agents_enabled": agents_enabled,
        "agent_count": agent_count,
        "goal": goal,
        "tools": tools,
    })
}

fn creator_routing_payload(
    model: V,
    thinking_effort: V,
    permissions: &str,
    agents_enabled: bool,
    agent_count: V,
) -> V {
    json!({
        "model": model,
        "thinking_effort": thinking_effort,
        "permissions": normalized_creator_permission(Some(permissions)),
        "agents_enabled": agents_enabled,
        "agent_count": agent_count,
    })
}

fn creator_current(project: &str, id: &str, generation: u64) -> bool {
    dom::value("creator-project") == project
        && m::scope_current("creator-scope", generation, project, id)
}
fn creator_mutation_controls(locked: bool) {
    for id in [
        "creator-project",
        "creator-runs",
        "creator-refresh",
        "creator-new-run",
    ] {
        dom::disabled(id, locked);
    }
    for input in dom::all(
        "#creator-form input,#creator-form select,#creator-form textarea,#creator-form button",
    ) {
        if locked {
            let _ = input.set_attribute("disabled", "");
        } else {
            let _ = input.remove_attribute("disabled");
        }
    }
    let busy = creator_busy(&m::data("creator-detail")["run"]);
    dom::disabled("creator-permissions", locked || busy);
    creator_permissions_hint();
    if locked {
        for action in dom::all("#creator-actions button") {
            let _ = action.set_attribute("disabled", "");
        }
    }
    crate::session::sync_working_indicator();
}
fn creator_reset_detail() {
    m::take("creator-detail");
    dom::hidden("creator-detail", true);
    dom::hidden("creator-permissions-panel", true);
    dom::text("creator-permissions-lead", "");
    run::sync_waiting_permissions("creator-permissions-list", &[], false);
    dom::clear("creator-actions");
    dom::options("creator-runs", &[], m::summary, None);
    creator_permissions_hint();
    crate::session::sync_working_indicator();
}
pub fn reset_creator_project() {
    m::next("creator-list");
    m::next("creator-read");
    dom::cancel_timer("creator-poll");
    m::take("creator-list");
    dom::set_value("creator-permissions", "auto");
    creator_reset_detail();
    creator_mutation_controls(false);
}
pub async fn creator_list() {
    dom::cancel_timer("creator-poll");
    m::next("creator-read");
    let project = dom::value("creator-project");
    let generation = m::bind_scope("creator-scope", &project, "");
    if m::data("creator-detail")["run"]["project_id"] != project {
        creator_reset_detail();
    }
    if project.is_empty() {
        creator_reset_detail();
        return;
    }
    match api::scoped(
        "creator-list",
        &format!("/api/creator?project_id={}", dom::enc(&project)),
    )
    .await
    {
        Ok((epoch, value))
            if m::current("creator-list", epoch) && creator_current(&project, "", generation) =>
        {
            m::set("creator-list", value.clone());
            dom::options(
                "creator-runs",
                &m::array(&value["runs"]),
                |run| {
                    format!(
                        "{} · {} · {}",
                        m::text(run, "created_at"),
                        m::text(run, "operation"),
                        m::text(run, "status")
                    )
                },
                None,
            );
            dom::disabled(
                "creator-start",
                value["available"] == false || m::yes(&m::data("creator-pending")),
            );
            let id = dom::value("creator-runs");
            if id.is_empty() {
                creator_reset_detail();
                creator_mutation_controls(false);
            } else {
                creator_read(&id).await;
            }
        }
        Ok(_) => {}
        Err(error) if !error.aborted && creator_current(&project, "", generation) => {
            dom::error("creator-error", &error.message)
        }
        Err(_) => {}
    }
}
pub async fn creator_read(id: &str) {
    dom::cancel_timer("creator-poll");
    let project = dom::value("creator-project");
    let generation = m::bind_scope("creator-scope", &project, id);
    creator_read_scoped(id, &project, generation).await;
}
async fn creator_read_scoped(id: &str, project: &str, generation: u64) {
    if id.is_empty()
        || !creator_current(project, id, generation)
        || !m::read(|s| s.view == "creator")
    {
        return;
    }
    dom::cancel_timer("creator-poll");
    match api::scoped("creator-read", &format!("/api/creator/{}", dom::enc(id))).await {
        Ok((epoch, mut value))
            if m::current("creator-read", epoch)
                && creator_current(project, id, generation)
                && value["run"]["id"] == id
                && value["run"]["project_id"] == project
                && m::read(|s| s.view == "creator") =>
        {
            let prior = m::data("creator-detail");
            if prior["run"]["id"] != value["run"]["id"] {
                let run = &value["run"];
                dom::set_value("creator-provider", &m::text(run, "provider"));
                dom::set_value("creator-model", &m::text(run, "model"));
                creator_models();
                for (key, target) in [
                    ("provider", "creator-provider"),
                    ("model", "creator-model"),
                    ("thinking_effort", "creator-effort"),
                    ("agent_count", "creator-count"),
                ] {
                    dom::set_value(target, &m::string(&run[key]));
                }
                dom::set_value("creator-permissions", creator_restored_permissions(run));
                dom::set_checked("creator-agents", m::yes(&run["agents_enabled"]));
                creator_permissions_hint();
            }
            if prior["run"]["revision"] != value["run"]["revision"] {
                dom::set_value("creator-answers", "");
            }
            value["view_generation"] = json!(generation);
            m::set("creator-detail", value.clone());
            render_creator();
            if creator_busy(&value["run"]) {
                let id = id.to_owned();
                let project = project.to_owned();
                dom::later("creator-poll", 2000, move || {
                    spawn_local(async move {
                        creator_read_scoped(&id, &project, generation).await;
                    });
                });
            }
        }
        Ok(_) => {}
        Err(error) if !error.aborted && creator_current(project, id, generation) => {
            dom::error("creator-action-error", &error.message)
        }
        Err(_) => {}
    }
}
fn creator_busy(v: &V) -> bool {
    matches!(
        v["status"].as_str(),
        Some("scanning" | "generating" | "applying" | "rolling_back")
    )
}
fn render_creator() {
    let v = m::data("creator-detail");
    let run = &v["run"];
    if !run.is_object() {
        return;
    }
    dom::hidden("creator-detail", false);
    let waiting = m::array(&v["waiting_permissions"]);
    dom::text("creator-status", &creator_status_text(run, &waiting));
    dom::text("creator-target", &creator_target_summary(run));
    dom::text("creator-message", &m::text(run, "message"));
    session::restore_budgets("creator-run-budgets", &run["budgets"]);
    let active = creator_busy(run) || m::active(&v["session"]);
    let (show_waiting, lead) = creator_permissions_panel_state(&v);
    run::sync_waiting_permissions("creator-permissions-list", &waiting, active);
    dom::hidden("creator-permissions-panel", !show_waiting);
    dom::text("creator-permissions-lead", &lead);
    let reports = m::array(&v["reports"])
        .into_iter()
        .enumerate()
        .map(|(idx, item)| json!({"idx": idx, "item": item}))
        .collect::<Vec<_>>();
    dom::keyed_render(
        "creator-reports",
        &reports,
        |entry| {
            let idx = entry["idx"].as_u64().unwrap_or(0) as usize;
            creator_report_view(&entry["item"], idx).key
        },
        |entry| {
            let idx = entry["idx"].as_u64().unwrap_or(0) as usize;
            let report = creator_report_view(&entry["item"], idx);
            let details = dom::el("details", "setup-file creator-report", "");
            let _ = details.set_attribute("data-restore-key", &report.key);
            dom::add(&details, &dom::el("summary", "", &report.summary));
            dom::add(&details, &dom::el("pre", "knowledge-result", &report.text));
            details
        },
    );
    if reports.is_empty() {
        dom::append(
            "creator-reports",
            &dom::el("p", "fleet-note", "No reports recorded yet."),
        );
    }
    dom::text(
        "creator-events",
        &m::array(&v["events"])
            .iter()
            .map(|v| format!("[{}] {}", m::text(v, "kind"), m::text(v, "text")))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    file_list("creator-files", &m::array(&v["preview"]["files"]));
    dom::hidden("creator-preview", !v["preview"].is_object());
    for file in m::array(&v["preview"]["files"]) {
        if m::yes(&file["requires_decision"]) {
            dom::append(
                "creator-files",
                &dom::check(
                    &format!(
                        "Approve {}: {}",
                        m::text(&file, "action"),
                        m::text(&file, "path")
                    ),
                    "creator-replace",
                    &m::text(&file, "path"),
                    false,
                ),
            );
        }
    }
    dom::clear("creator-actions");
    let mut actions = vec![];
    if m::yes(&run["recovery_available"]) {
        actions.push(("rollback", "Recover interrupted publication"));
    } else {
        match run["status"].as_str().unwrap_or("") {
            "review" => {
                actions.push(("generate", "Approve profile & generate"));
                actions.push(("revise", "Revise scan"));
            }
            "preview" => {
                actions.push(("apply", "Apply reviewed files"));
                actions.push(("revise", "Revise generated files"));
            }
            "needs_input" | "failed" | "cancelled" | "interrupted" | "rolled_back" => {
                actions.push(("revise", "Continue with answers / corrections"))
            }
            _ => {}
        }
        if !creator_busy(run) && run["status"] != "complete" {
            actions.push(("rescan", "Scan again for review"));
        }
        if ["scanning", "generating"].contains(&run["status"].as_str().unwrap_or("")) {
            actions.push(("cancel", "Cancel phase"));
        }
    }
    for (action, label) in actions {
        let b = dom::button(label, "creator-action", json!(action));
        let _ = b.set_attribute("id", &format!("creator-{action}"));
        if action == "apply"
            && (!m::yes(&v["preview"]["can_apply"])
                || m::array(&v["preview"]["files"])
                    .iter()
                    .any(|v| m::yes(&v["requires_decision"])))
        {
            let _ = b.set_attribute("disabled", "");
        }
        dom::append("creator-actions", &b);
    }
    if let Some(sid) = v["session"]["id"].as_str() {
        dom::append(
            "creator-actions",
            &dom::button("View results & run usage", "creator-session", json!(sid)),
        );
        m::upsert(v["session"].clone());
    }
    dom::hidden(
        "creator-corrections",
        creator_busy(run) || run["status"] == "complete" || m::yes(&run["recovery_available"]),
    );
    creator_mutation_controls(m::yes(&m::data("creator-pending")));
}
async fn creator_start() {
    if !dom::validity("creator-form") || m::yes(&m::data("creator-pending")) {
        return;
    }
    let project = dom::value("creator-project");
    if project.is_empty() {
        return;
    }
    let body = creator_start_body(
        session::budgets("creator-budgets"),
        &project,
        &dom::value("creator-provider"),
        session::nullable(dom::value("creator-model").trim().into()),
        session::nullable(dom::value("creator-effort")),
        &dom::value("creator-permissions"),
        &dom::value("creator-operation"),
        &dom::value("creator-workspace"),
        if dom::value("creator-workspace") == "worktree" {
            dom::value("creator-branch").trim().to_owned()
        } else {
            String::new()
        },
        dom::checked("creator-agents"),
        dom::num("creator-count"),
        &dom::value("creator-goal"),
        dom::all("[name=creator-tool]:checked")
            .iter()
            .map(dom::value_of)
            .collect::<Vec<_>>(),
    );
    let generation = m::bind_scope("creator-scope", &project, "");
    dom::cancel_timer("creator-poll");
    m::next("creator-read");
    m::bind_pending("creator-pending", generation);
    creator_mutation_controls(true);
    match api::post("/api/creator", body).await {
        Ok(mut value)
            if creator_current(&project, "", generation)
                && value["run"]["project_id"] == project =>
        {
            value["view_generation"] = json!(generation);
            m::set("creator-detail", value.clone());
            render_creator();
            dom::hidden("creator-form", true);
            let id = m::text(&value["run"], "id");
            let read_generation = m::bind_scope("creator-scope", &project, &id);
            creator_read_scoped(&id, &project, read_generation).await;
            session::refresh().await;
        }
        Ok(_) => {}
        Err(error) if creator_current(&project, "", generation) => {
            dom::error("creator-error", &error.message)
        }
        Err(_) => {}
    }
    if m::finish_pending("creator-pending", generation) {
        creator_mutation_controls(false);
        render_creator();
    }
}
async fn creator_action(action: &str) {
    if m::yes(&m::data("creator-pending")) {
        return;
    }
    let data = m::data("creator-detail");
    let run = &data["run"];
    let project = dom::value("creator-project");
    let id = m::text(run, "id");
    if id.is_empty()
        || run["project_id"] != project
        || !creator_current(&project, &id, data["view_generation"].as_u64().unwrap_or(0))
    {
        dom::error(
            "creator-action-error",
            "Select the current project and run again before continuing.",
        );
        return;
    }
    let mut body = json!({"action":action,"revision":run["revision"]});
    if action == "budgets" {
        body["budgets"] = session::budgets("creator-run-budgets");
    }
    if ["revise", "rescan"].contains(&action) {
        body["answers"] = json!(dom::value("creator-answers"));
    }
    if ["revise", "rescan", "generate"].contains(&action) {
        if session::budgets("creator-run-budgets") != run["budgets"] {
            dom::error(
                "creator-action-error",
                "Save budget changes before launching another phase.",
            );
            return;
        }
        body["routing"] = creator_routing_payload(
            session::nullable(dom::value("creator-model").trim().into()),
            session::nullable(dom::value("creator-effort")),
            &dom::value("creator-permissions"),
            dom::checked("creator-agents"),
            dom::num("creator-count"),
        );
    }
    if action == "apply" {
        let approved = dom::selected_checks("creator-files");
        if !m::yes(&data["preview"]["can_apply"])
            || m::array(&data["preview"]["files"])
                .iter()
                .filter(|file| m::yes(&file["requires_decision"]))
                .any(|file| !approved.contains(&file["path"]))
        {
            return;
        }
        body["preview_id"] = data["preview"]["preview_id"].clone();
        body["replace"] = json!(approved);
    }
    let generation = m::bind_scope("creator-scope", &project, &id);
    dom::cancel_timer("creator-poll");
    m::next("creator-read");
    m::bind_pending("creator-pending", generation);
    creator_mutation_controls(true);
    match api::post(&format!("/api/creator/{}", dom::enc(&id)), body).await {
        Ok(mut value)
            if creator_current(&project, &id, generation)
                && value["run"]["id"] == id
                && value["run"]["project_id"] == project =>
        {
            value["view_generation"] = json!(generation);
            m::set("creator-detail", value.clone());
            if action != "budgets" {
                dom::set_value("creator-answers", "");
            }
            render_creator();
            if creator_busy(&value["run"]) {
                let project = project.clone();
                let id = id.clone();
                dom::later("creator-poll", 1000, move || {
                    spawn_local(async move {
                        creator_read_scoped(&id, &project, generation).await;
                    });
                });
            }
        }
        Ok(_) => {}
        Err(error) if creator_current(&project, &id, generation) => {
            dom::error("creator-action-error", &error.message)
        }
        Err(_) => {}
    }
    if m::finish_pending("creator-pending", generation) {
        creator_mutation_controls(false);
        render_creator();
    }
}
pub async fn event(
    kind: &str,
    id: &str,
    action: &str,
    payload: &V,
    value: &str,
    checked: bool,
) -> bool {
    if kind == "change" || kind == "input" {
        if action == "setup-tool" || id == "setup-edition" {
            invalidate("setup");
            return true;
        }
        if id == "setup-project" {
            load_setup().await;
            return true;
        }
        if id == "setup-preview-actions" || id == "setup-preview-path" {
            setup_files();
            return true;
        }
        if action == "skills-select" {
            let mut selected = m::array(&m::data("skills-selection"));
            selected.retain(|v| v != value);
            if checked {
                selected.push(json!(value));
            }
            m::set("skills-selection", json!(selected));
            invalidate("skills");
            skills_controls();
            return true;
        }
        if action == "skills-agent" {
            invalidate("skills");
            skills_controls();
            return true;
        }
        if id == "skills-source" {
            invalidate("skills");
            m::take("skills-discovery");
            render_skills();
            skills_controls();
            return true;
        }
        if id == "skills-project" {
            invalidate("skills");
            m::take("skills-change-preview");
            skills_controls();
            installed_skills().await;
            return true;
        }
        if id == "skills-filter" {
            render_skills();
            return true;
        }
        if id == "skills-installed-agent" {
            render_installed();
            return true;
        }
        if id.starts_with("create-skill-") || action == "create-skill-agent" {
            invalidate("create-skill");
            dom::text(
                "create-skill-bytes",
                &format!(
                    "{} / 32,000 UTF-8 bytes",
                    dom::value("create-skill-instructions").len()
                ),
            );
            create_skill_controls();
            return true;
        }
        if id == "creator-project" {
            creator_list().await;
            return true;
        }
        if id == "creator-runs" {
            creator_read(value).await;
            return true;
        }
        if action == "creator-replace" {
            let v = m::data("creator-detail");
            let approved = dom::selected_checks("creator-files");
            dom::disabled(
                "creator-apply",
                !m::yes(&v["preview"]["can_apply"])
                    || m::array(&v["preview"]["files"])
                        .iter()
                        .filter(|v| m::yes(&v["requires_decision"]))
                        .any(|v| !approved.contains(&v["path"])),
            );
            return true;
        }
        if id == "creator-workspace" {
            dom::hidden("creator-branch-field", value != "worktree");
            return true;
        }
        if id == "creator-provider" {
            creator_models();
            return true;
        }
        if id == "creator-permissions" {
            creator_permissions_hint();
            return true;
        }
    }
    if kind == "input" && id == "creator-model" {
        creator_models();
        return true;
    }
    if kind == "submit" {
        match id {
            "folder-picker-location-form" => {
                folder(&dom::value("folder-picker-location"), false).await;
                return true;
            }
            "folder-picker-search-form" => {
                folder(&m::text(&m::data("folder"), "path"), true).await;
                return true;
            }
            "create-skill-form" => {
                preview("create-skill").await;
                return true;
            }
            "creator-form" => {
                creator_start().await;
                return true;
            }
            _ => {}
        }
    }
    if kind != "click" {
        return false;
    }
    match action {
        "project-choose-folder" | "welcome-choose-folder" => folder_open("session").await,
        "folder-enter" => {
            folder(payload.as_str().unwrap_or(""), false).await;
        }
        "folder-picker-home" => {
            folder("", false).await;
        }
        "folder-picker-parent" => {
            folder(&m::text(&m::data("folder"), "parent"), false).await;
        }
        "folder-picker-clear" => {
            dom::set_value("folder-picker-query", "");
            folder(&m::text(&m::data("folder"), "path"), false).await;
        }
        "folder-picker-close" => {
            m::next("folder");
            dom::dialog("folder-picker", false);
        }
        "folder-picker-use" => folder_use().await,
        "setup-refresh-projects" => {
            if let Ok(v) = api::get("/api/projects").await {
                registered(v["projects"].clone());
            }
            load_setup().await;
        }
        "setup-preview-button" => preview("setup").await,
        "setup-install-button" => install("setup").await,
        "setup-attach-button" => attach("attach").await,
        "setup-detach-button" => attach("detach").await,
        "setup-attach-codex-trust" => hooks(true).await,
        "setup-start-session" => {
            let p = dom::value("setup-project");
            session::new_session(Some(&p));
            session::open_view("sessions", true).await;
        }
        "setup-result-start" => {
            let p = m::string(&m::data("setup-result-project"));
            session::new_session(Some(&p));
            session::open_view("sessions", true).await;
        }
        "skills-discover" => discover(false).await,
        "skills-source-refresh" => discover(true).await,
        "skills-preview-button" => preview("skills").await,
        "skills-install" => install("skills").await,
        "skills-installed-refresh" => installed_skills().await,
        "skill-change-preview" => change_preview(payload).await,
        "skills-change-apply" => apply_change().await,
        "create-skill-save" => install("create-skill").await,
        "create-skill-view-installed" => {
            dom::set_value(
                "skills-project",
                &m::string(&m::data("create-skill-result-project")),
            );
            session::open_view("skills", true).await;
        }
        "creator-refresh" => creator_list().await,
        "creator-new-run" => {
            dom::hidden("creator-form", false);
            dom::set_value(
                "creator-permissions",
                session::restored_value("permissions", &dom::value("creator-permissions")),
            );
            creator_permissions_hint();
            dom::focus("creator-goal");
        }
        "permission-decide" => {
            if run::permission_event_container(payload) != "creator-permissions-list" {
                return false;
            }
            let detail = m::data("creator-detail");
            let sid = detail["session"]["id"]
                .as_str()
                .or_else(|| detail["run"]["session_id"].as_str())
                .unwrap_or("")
                .to_owned();
            let run_id = m::text(&detail["run"], "id");
            if !sid.is_empty()
                && run::decide_permission_for("creator-permissions-list", &sid, payload).await
                && !run_id.is_empty()
            {
                creator_read(&run_id).await;
            }
        }
        "creator-action" => creator_action(payload.as_str().unwrap_or("")).await,
        "creator-run-budgets-save" => creator_action("budgets").await,
        "creator-session" => {
            session::select(payload.as_str().unwrap_or("")).await;
            session::open_view("changes", true).await;
        }
        "accelerator-catalog" => session::open_view("kit3", true).await,
        "accelerator-creator" => {
            dom::set_value("creator-project", &dom::value("accelerator-project"));
            session::open_view("creator", true).await;
        }
        "accelerator-setup" => {
            dom::set_value("setup-project", &dom::value("accelerator-project"));
            dom::set_value("setup-edition", payload.as_str().unwrap_or(""));
            session::open_view("setup", true).await;
        }
        "accelerator-startup" => {
            let index = payload.as_u64().unwrap_or(0);
            match api::get("/api/accelerators/startup").await {
                Ok(v) => startup_table(&v, index),
                Err(e) => dom::append(
                    &format!("startup-context-{index}"),
                    &dom::el("p", "error-text", &e.message),
                ),
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{
        creator_report_view, format_setup_diagnostic, readiness_cards, setup_provider_card,
    };
    use serde_json::json;

    #[test]
    fn setup_readiness_providers_diagnostics_and_creator_reports_format_cleanly() {
        let readiness = json!({
            "policy": true,
            "memory_runtime": false,
            "brain_runtime": true
        });
        let checks = readiness_cards(&readiness);
        assert_eq!(checks.len(), 3);
        assert_eq!(checks[0].title, "Policy guide");
        assert!(checks[0].ok);
        assert_eq!(checks[0].state, "Ready");
        assert_eq!(checks[1].title, "Memory runtime");
        assert!(!checks[1].ok);
        assert_eq!(checks[1].state, "Not detected");

        let provider = json!({
            "id": "codex",
            "name": "Codex",
            "available": true,
            "detail": "CLI ready",
            "agent_control_detail": "Interactive TTY"
        });
        let provider_card = setup_provider_card(&provider, 0);
        assert_eq!(provider_card.title, "Codex");
        assert!(provider_card.ok);
        assert_eq!(provider_card.state, "Available");
        assert_eq!(provider_card.detail, "CLI ready · Interactive TTY");

        assert_eq!(
            format_setup_diagnostic(&json!("Missing PHP binary on PATH")),
            "Missing PHP binary on PATH"
        );

        let report = json!({
            "name": "infra-scan-project-profile.md",
            "text": "# Scan\nAll good",
            "truncated": true,
            "hash": "abcdef123456"
        });
        let view = creator_report_view(&report, 0);
        assert!(view
            .summary
            .contains("infra-scan-project-profile.md (15 chars · truncated · #abcdef12)"));
        assert_eq!(view.text, "# Scan\nAll good");
    }

    #[test]
    fn creator_permissions_hints_payloads_restoration_and_inline_cards_preserve_contracts() {
        assert!(super::creator_permissions_hint_text("claude", "auto").contains("auto mode"));
        assert!(super::creator_permissions_hint_text("codex", "manual").contains("Allow and Deny"));
        assert!(super::creator_permissions_hint_text("cursor", "bypass")
            .contains("without the sandbox"));

        assert_eq!(
            super::creator_restored_permissions(&json!({"permissions": "bypass"})),
            "auto"
        );
        assert_eq!(
            super::creator_restored_permissions(&json!({"permissions": "manual"})),
            "manual"
        );
        assert_eq!(super::creator_restored_permissions(&json!({})), "auto");

        let run = json!({
            "target": "/srv/app",
            "workspace": "project",
            "provider": "claude",
            "model": "sonnet",
            "permissions": "manual",
            "status": "scanning"
        });
        assert_eq!(
            super::creator_target_summary(&run),
            "/srv/app · project · claude / sonnet · Permissions: Manual"
        );
        assert!(super::creator_busy(&run));

        let waiting = vec![json!({
            "request_id": "cr-1",
            "provider": "claude",
            "tool": "Bash",
            "summary": "composer test",
            "detail": "Manual approval requested",
            "session_option": true,
            "rule_summary": "Allow Bash(composer test)",
        })];
        assert_eq!(
            super::creator_status_text(&run, &waiting),
            "scanning · Waiting for your answer to a permission request…"
        );
        assert_eq!(super::creator_status_text(&run, &[]), "scanning");

        let detail = json!({"run": run, "waiting_permissions": waiting});
        assert_eq!(
            super::creator_permissions_panel_state(&detail),
            (
                true,
                "Waiting for your answer to a permission request…".to_owned()
            )
        );
        assert_eq!(
            super::creator_permissions_panel_state(&json!({"waiting_permissions": []})),
            (false, String::new())
        );

        let start = super::creator_start_body(
            json!({"usd": 5}),
            "proj-1",
            "claude",
            json!(null),
            json!("high"),
            "manual",
            "generate",
            "project",
            String::new(),
            true,
            json!(3),
            "Build PHP kit",
            vec!["claude".to_owned()],
        );
        assert_eq!(start["permissions"], "manual");
        assert_eq!(start["project_id"], "proj-1");

        let routing =
            super::creator_routing_payload(json!("opus"), json!(null), "bypass", false, json!(3));
        assert_eq!(routing["permissions"], "bypass");
        assert_eq!(routing["model"], "opus");

        let html = crate::run::render_permission_card_html(
            "creator-permissions-list",
            &detail["waiting_permissions"][0],
            true,
        );
        crate::test_support::assert_no_inline_hex_colors(&html);
        assert!(html.contains("data-permission-id=\"cr-1\""));
        assert!(html.contains("Allow for this session: Allow Bash(composer test)"));
        let payloads = crate::test_support::payloads(&html, "permission-decide");
        assert_eq!(payloads.len(), 3);
        assert_eq!(payloads[0]["container"], "creator-permissions-list");
        assert_eq!(payloads[0]["request_id"], "cr-1");
        assert_eq!(payloads[1]["decision"], "allow_session");
    }

    #[test]
    fn create_skill_validation_enforces_slug_bounds_and_tools() {
        let bad_name = super::validate_create_skill_inputs(
            "proj-1",
            "BAD NAME",
            "Valid trigger description",
            "# Steps",
            1,
        );
        assert!(!bad_name.valid);
        assert!(!bad_name.name_error.is_empty());

        let double_hyphen = super::validate_create_skill_inputs(
            "proj-1",
            "bad--name",
            "Valid trigger description",
            "# Steps",
            1,
        );
        assert!(!double_hyphen.valid);
        assert!(!double_hyphen.name_error.is_empty());

        let no_tools = super::validate_create_skill_inputs(
            "proj-1",
            "check-tax-rounding",
            "Valid trigger description",
            "# Steps",
            0,
        );
        assert!(!no_tools.valid);
        assert!(!no_tools.agents_error.is_empty());

        let ok = super::validate_create_skill_inputs(
            "proj-1",
            "check-tax-rounding",
            "Use when verifying tax calculations",
            "# Steps\nVerify 2-decimal precision.",
            2,
        );
        assert!(ok.valid);
        assert!(ok.name_error.is_empty());
        assert!(ok.summary_error.is_empty());
    }

    #[test]
    fn setup_preview_filter_surfaces_reported_collisions_and_all_files() {
        let preview = json!({
            "files": [
                {"path": "AGENTS.md", "action": "merge"},
                {"path": ".gitignore", "action": "unchanged"},
            ],
            "collisions": [
                {"path": "ACCELERATOR.md", "reason": "accelerator-readme-exists"},
            ],
        });

        let changed = super::filtered_setup_files(&preview, "", "changed");
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0]["path"], "AGENTS.md");

        let collisions = super::filtered_setup_files(&preview, "", "collision");
        assert_eq!(collisions.len(), 1);
        assert_eq!(collisions[0]["path"], "ACCELERATOR.md");
        assert_eq!(collisions[0]["action"], "collision");
        assert_eq!(collisions[0]["reason"], "accelerator-readme-exists");

        let all = super::filtered_setup_files(&preview, "", "all");
        assert_eq!(all.len(), 3);

        let searched = super::filtered_setup_files(&preview, "accelerator", "collision");
        assert_eq!(searched.len(), 1);
        assert_eq!(searched[0]["path"], "ACCELERATOR.md");
    }
}
