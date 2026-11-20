mod creator;
mod setup;
mod skills;
#[cfg(test)]
mod tests;

pub use creator::{creator_list, creator_models, creator_read, reset_creator_project};
#[allow(unused_imports)]
pub(crate) use creator::{on_sse_creator_resync, on_sse_creator_run};
#[allow(unused_imports)]
pub use setup::invalidate;
pub use setup::{folder_open, folder_open_at, load_attachment, load_setup};
pub use skills::{accelerators, load_skills};

use crate::{activity::events_card, api, dom, model as m, run, session};
use creator::*;
use serde_json::{json, Value as V};
use setup::*;
use skills::*;
use wasm_bindgen_futures::spawn_local;

pub fn initialize() {
    for k in ["setup", "skills", "create-skill"] {
        invalidate(k);
    }
    dom::set_value("creator-count", "3");
    if !m::yes(&m::data("management-initialized")) {
        m::set("management-initialized", json!(true));
        crate::api::on_sse("creator_run", crate::management::on_sse_creator_run);
        crate::api::on_sse("resync", crate::management::on_sse_creator_resync);
        crate::session::on_project_change(crate::management::on_global_project_change);
    }
}

#[allow(dead_code)]
pub(crate) fn on_global_project_change(project: &str) {
    let project = project.trim();
    if project.is_empty() {
        return;
    }
    let active_view = m::read(|s| s.view.clone());
    for scope_id in [
        "setup-project",
        "skills-project",
        "create-skill-project",
        "accelerator-project",
    ] {
        if cfg!(target_arch = "wasm32") && dom::value(scope_id) != project {
            dom::set_value(scope_id, project);
        }
    }
    for k in ["setup", "skills", "create-skill"] {
        invalidate(k);
    }
    m::take("skills-change-preview");
    if !m::yes(&m::data("creator-pending")) {
        let prev_creator = if cfg!(target_arch = "wasm32") {
            dom::value("creator-project")
        } else {
            m::string(&m::data("creator-project-sync"))
        };
        if prev_creator != project {
            m::set("creator-project-sync", json!(project));
            if cfg!(target_arch = "wasm32") {
                dom::set_value("creator-project", project);
            }
            reset_creator_project();
            if cfg!(target_arch = "wasm32") && active_view == "creator" {
                spawn_local(async {
                    creator_list().await;
                });
            }
        }
    }
    if cfg!(target_arch = "wasm32") {
        if active_view == "setup" {
            spawn_local(async {
                load_setup().await;
            });
        } else if active_view == "skills" {
            skills_controls();
            spawn_local(async {
                installed_skills().await;
            });
        }
    }
}
pub fn registered(projects: V) {
    let old = dom::value("project");
    m::write(|s| s.boot["projects"] = projects);
    session::populate();
    if !m::project(&old).is_null() {
        session::sync_project(&old);
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
            if value.trim().is_empty() {
                dom::cancel_timer("skills-filter-debounce");
                render_skills();
            } else {
                dom::later("skills-filter-debounce", 80, || {
                    render_skills();
                });
            }
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
        if action == "structured-events-search" && m::text(payload, "scope") == "creator-events" {
            events_card::set_event_filter_query("creator-events", value);
            render_creator_events_only();
            return true;
        }
        if action == "creator-replace" {
            handle_creator_replace_toggle(value, checked);
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
        "structured-events-kind" if m::text(payload, "scope") == "creator-events" => {
            events_card::set_event_filter_kind("creator-events", &m::text(payload, "kind"));
            render_creator_events_only();
        }
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
            session::select_with_view(payload.as_str().unwrap_or(""), Some("changes")).await;
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
#[allow(unused_imports)]
mod test_exports {
    pub(super) use super::creator::*;
    pub(super) use super::setup::*;
    pub(super) use super::skills::*;
}
