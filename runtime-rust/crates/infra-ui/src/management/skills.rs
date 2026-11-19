use super::setup::{file_list, invalidate};
use crate::{api, dom, model as m};
use serde_json::{json, Value as V};
use wasm_bindgen::JsCast;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CreateSkillValidation {
    pub(super) valid: bool,
    pub(super) name_error: &'static str,
    pub(super) description_error: &'static str,
    pub(super) instructions_error: &'static str,
    pub(super) agents_error: &'static str,
    pub(super) summary_error: &'static str,
}

pub(super) fn valid_skill_slug(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub(super) fn validate_create_skill_inputs(
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

pub(super) fn skills_controls() {
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

pub(super) fn create_skill_controls() {
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
pub(super) fn startup_table(data: &V, index: u64) {
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
pub(super) async fn discover(refresh: bool) {
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
pub(super) fn render_skills() {
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
        let id = m::text(skill, "id");
        let name = {
            let n = m::text(skill, "name");
            if n.is_empty() {
                id.clone()
            } else {
                n
            }
        };
        let description = m::text(skill, "description");
        let text = format!("{id} {name} {description}");
        if !text.to_lowercase().contains(&query) {
            continue;
        }
        count += 1;
        let row = dom::el("label", "skill-choice", "");
        let input = dom::el("input", "", "");
        let _ = input.set_attribute("type", "checkbox");
        let _ = input.set_attribute("data-action", "skills-select");
        let _ = input.set_attribute("value", &id);
        if let Some(el) = input.dyn_ref::<web_sys::HtmlInputElement>() {
            el.set_value(&id);
            el.set_checked(selected.iter().any(|v| v == &id));
        }
        let copy = dom::el("span", "skill-choice-copy", "");
        dom::add(&copy, &dom::el("span", "skill-choice-name", &name));
        dom::add(&copy, &dom::el("span", "skill-choice-id", &id));
        if !description.is_empty() {
            dom::add(
                &copy,
                &dom::el("span", "skill-choice-description", &description),
            );
        }
        dom::add(&row, &input);
        dom::add(&row, &copy);
        dom::append("skills-list", &row);
    }
    dom::text("skills-visible-count", &format!("{count} matching"));
    dom::text(
        "skills-selection-count",
        &format!("{} selected of {}", selected.len(), items.len()),
    );
}
pub(super) async fn installed_skills() {
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
pub(super) fn render_installed() {
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
        let e = dom::el("div", "skills-file installed-skill", "");
        let meta = dom::el("div", "", "");
        let title_row = dom::el("div", "skills-installed-name", "");
        let name_text = {
            let n = m::text(&skill, "name");
            if n.is_empty() {
                m::text(&skill, "id")
            } else {
                n
            }
        };
        dom::add(&title_row, &dom::el("span", "", &name_text));
        let chips = dom::el("span", "installed-tools", "");
        let agent_name = m::text(&skill, "agent");
        if !agent_name.is_empty() {
            let chip_cls = if m::yes(&skill["managed"]) {
                "tool-chip managed"
            } else {
                "tool-chip"
            };
            dom::add(&chips, &dom::el("span", chip_cls, &agent_name));
        }
        let state_label = m::text(&skill, "state");
        if !state_label.is_empty() {
            dom::add(&chips, &dom::el("span", "tool-chip", &state_label));
        }
        dom::add(&title_row, &chips);
        dom::add(&meta, &title_row);
        let path_text = m::text(&skill, "path");
        if !path_text.is_empty() {
            dom::add(&meta, &dom::el("div", "skills-installed-path", &path_text));
        }
        dom::add(&e, &meta);
        if m::yes(&skill["managed"]) {
            let actions = dom::el("div", "skills-source-actions", "");
            for operation in ["update", "remove"] {
                if operation == "update" && skill["source_id"] == "local" {
                    continue;
                }
                let mut p = skill.clone();
                p["operation"] = json!(operation);
                let btn = dom::button(
                    if operation == "update" {
                        "Check update"
                    } else {
                        "Preview removal"
                    },
                    "skill-change-preview",
                    p,
                );
                let _ = btn.class_list().add_1("compact");
                dom::add(&actions, &btn);
            }
            dom::add(&e, &actions);
        }
        dom::append("skills-installed-list", &e);
    }
    dom::text(
        "skills-installed-status",
        &format!("{count} installed copies"),
    );
}
pub(super) async fn change_preview(skill: &V) {
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
pub(super) async fn apply_change() {
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
