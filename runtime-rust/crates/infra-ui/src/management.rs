use crate::{api, dom, model as m, session, systems};
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
    m::set("folder-purpose", json!(purpose));
    dom::dialog("folder-picker", true);
    folder("", false).await;
}
async fn folder(path: &str, query: bool) {
    let body = json!({"query":if query{dom::value("folder-picker-query").trim().to_owned()}else{String::new()},"hidden":dom::checked("folder-picker-hidden")});
    let mut body = body;
    if !path.is_empty() {
        body["path"] = json!(path);
    }
    let epoch = m::next("folder");
    dom::text("folder-picker-status", "Reading folders…");
    match api::post("/api/projects/browse", body).await {
        Ok(v) if m::current("folder", epoch) => {
            m::set("folder", v.clone());
            dom::set_value("folder-picker-location", &m::text(&v, "path"));
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
        }
        Ok(_) => {}
        Err(e) => dom::error("folder-picker-error", &e.message),
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
        let body = format!(
            "{} → {} bytes\n{}\n{}{}",
            m::string(&file["before_bytes"]),
            m::string(&file["after_bytes"]),
            m::text(file, "reason"),
            m::text(file, "diff"),
            if m::yes(&file["diff_truncated"]) {
                "\nDiff truncated; inspect the complete file locally."
            } else {
                ""
            }
        );
        dom::append(id, &dom::details(&label, &body));
    }
}
pub async fn load_setup() {
    invalidate("setup");
    let project = dom::value("setup-project");
    if project.is_empty() {
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
                dom::error(
                    "setup-readiness-error",
                    "Incomplete project setup information. Refresh before installing.",
                );
                return;
            }
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
            dom::rows("setup-readiness-grid", &m::array(&v["readiness"]));
            dom::rows("setup-providers", &m::array(&v["providers"]));
            dom::rows("setup-diagnostics", &m::array(&v["diagnostics"]));
            dom::text(
                "setup-readiness-status",
                if v["available"] == false {
                    "This registered project is currently unavailable."
                } else {
                    "Project inspected."
                },
            );
            dom::disabled("setup-preview-button", v["available"] == false);
        }
        Ok(_) => {}
        Err(e) if !e.aborted => dom::error("setup-readiness-error", &e.message),
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
async fn preview(scope: &str) {
    let draft = selection(scope);
    if scope == "create-skill" {
        let name = m::text(&draft, "name");
        let valid = !name.is_empty()
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !name.starts_with('-')
            && !name.ends_with('-');
        let error = if !valid {
            "Use a lowercase hyphen-separated skill name."
        } else if m::text(&draft, "description").trim().is_empty()
            || m::text(&draft, "description").chars().count() > 1024
        {
            "Use a description of 1–1024 characters."
        } else if m::text(&draft, "instructions").trim().is_empty()
            || m::text(&draft, "instructions").len() > 32000
        {
            "Use instructions of 1–32,000 UTF-8 bytes."
        } else if m::array(&draft["agents"]).is_empty() {
            "Choose at least one tool."
        } else {
            ""
        };
        if !error.is_empty() {
            dom::error("create-skill-action-error", error);
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
fn setup_files() {
    let v = m::data("setup-preview");
    let query = dom::value("setup-preview-path").to_lowercase();
    let filter = dom::value("setup-preview-actions");
    let files = m::array(&v["files"])
        .into_iter()
        .filter(|f| {
            m::text(f, "path").to_lowercase().contains(&query)
                && (filter == "all"
                    || if filter == "collision" {
                        f["action"] == "collision"
                    } else {
                        f["action"] != "unchanged" && f["action"] != "kept"
                    })
        })
        .collect::<Vec<_>>();
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
        }
        Ok(_) => {}
        Err(e) if !e.aborted => dom::error("skills-catalog-error", &e.message),
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
    if locked {
        for action in dom::all("#creator-actions button") {
            let _ = action.set_attribute("disabled", "");
        }
    }
}
fn creator_reset_detail() {
    m::take("creator-detail");
    dom::hidden("creator-detail", true);
    dom::clear("creator-actions");
    dom::options("creator-runs", &[], m::summary, None);
}
pub fn reset_creator_project() {
    m::next("creator-list");
    m::next("creator-read");
    dom::cancel_timer("creator-poll");
    m::take("creator-list");
    creator_reset_detail();
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
                dom::set_checked("creator-agents", m::yes(&run["agents_enabled"]));
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
    dom::text(
        "creator-status",
        &m::label(run["status"].as_str().unwrap_or("")),
    );
    dom::text(
        "creator-target",
        &format!(
            "{} · {} · {} / {}",
            m::text(run, "target"),
            m::text(run, "workspace"),
            m::text(run, "provider"),
            m::text(run, "model")
        ),
    );
    dom::text("creator-message", &m::text(run, "message"));
    session::restore_budgets("creator-run-budgets", &run["budgets"]);
    dom::rows("creator-reports", &m::array(&v["reports"]));
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
    let body = json!({"budgets":session::budgets("creator-budgets"),"project_id":project,"provider":dom::value("creator-provider"),"model":session::nullable(dom::value("creator-model").trim().into()),"thinking_effort":session::nullable(dom::value("creator-effort")),"operation":dom::value("creator-operation"),"workspace":dom::value("creator-workspace"),"worktree_branch":if dom::value("creator-workspace")=="worktree"{dom::value("creator-branch").trim().to_owned()}else{String::new()},"agents_enabled":dom::checked("creator-agents"),"agent_count":dom::num("creator-count"),"goal":dom::value("creator-goal"),"tools":dom::all("[name=creator-tool]:checked").iter().map(dom::value_of).collect::<Vec<_>>()});
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
        body["routing"] = json!({"model":session::nullable(dom::value("creator-model").trim().into()),"thinking_effort":session::nullable(dom::value("creator-effort")),"agents_enabled":dom::checked("creator-agents"),"agent_count":dom::num("creator-count")});
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
            return true;
        }
        if action == "skills-agent" {
            invalidate("skills");
            return true;
        }
        if id == "skills-source" {
            invalidate("skills");
            m::take("skills-discovery");
            render_skills();
            return true;
        }
        if id == "skills-project" {
            invalidate("skills");
            m::take("skills-change-preview");
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
        "folder-enter" => folder(payload.as_str().unwrap_or(""), false).await,
        "folder-picker-home" => folder("", false).await,
        "folder-picker-parent" => folder(&m::text(&m::data("folder"), "parent"), false).await,
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
            dom::focus("creator-goal");
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
            dom::set_value("setup-edition", &dom::value(payload.as_str().unwrap_or("")));
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
