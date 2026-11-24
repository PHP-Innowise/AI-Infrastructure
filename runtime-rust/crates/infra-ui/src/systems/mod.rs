mod catalog_map;
mod editor;
mod runs;
#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use catalog_map::poll_system_graph_status;
#[allow(unused_imports)]
pub use catalog_map::{
    adopt_sidebar_project, aggregate_graphs_pill, build_changes_overlay, catalog_autoload,
    drill_down_service_map, drill_down_service_target, focus_system_graph_workspace,
    leave_systems_view, monotonic_service_graph_status, reset_system_project,
    switch_changes_to_topology, switch_services_to_topology,
};
pub use editor::{add_service, edit_field, editor_open};
pub(crate) use runs::read as read_system_run;
#[allow(unused_imports)]
pub(crate) use runs::{on_sse_system_resync, on_sse_system_run};
#[allow(unused_imports)]
pub use runs::{overlap_rows, receipt_changed_files, OverlapRow};

use crate::{activity::events_card, api, dom, management, model as m, run, session, system_map};
#[allow(unused_imports)]
use catalog_map::*;
#[allow(unused_imports)]
use editor::*;
#[allow(unused_imports)]
use runs::*;
use serde_json::{json, Value as V};
use wasm_bindgen_futures::spawn_local;

fn editor_current(project: &str, config: &str, generation: u64) -> bool {
    dom::value("system-project") == project
        && dom::value("system-config") == config
        && m::scope_current("editor-scope", generation, project, config)
}
pub(super) fn editor_ready() -> bool {
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let editor = m::data("editor");
    editor["project_id"] == project
        && editor["config_path"] == config
        && m::scope_current(
            "editor-scope",
            m::data("editor-generation").as_u64().unwrap_or(0),
            &project,
            &config,
        )
}
pub(super) fn sync_editor_target() -> bool {
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    if m::owns_scope("editor-scope", &project, &config) {
        return true;
    }
    let previous = m::data("editor-scope-owner");
    if previous["owner"].is_string()
        && (m::yes(&m::data("editor-dirty")) || m::yes(&m::data("editor-pending")))
    {
        dom::set_value("system-project", &m::text(&previous, "owner"));
        dom::set_value("system-config", &m::text(&previous, "item"));
        dom::error(
            "system-editor-error",
            "Save or discard the current system draft before changing its project or file.",
        );
        return false;
    }
    reset_system_target(&project, &config);
    true
}
pub(super) fn clear_editor_notices() {
    dom::text("system-discovery-status", "");
    dom::text("system-editor-status", "");
    dom::error("system-editor-error", "");
    dom::clear("system-discovery-report");
    for id in [
        "system-discovery-report",
        "system-discovery-agents",
        "system-discovery-live",
        "system-discovery-cancel",
    ] {
        dom::hidden(id, true);
    }
}
pub(super) fn reset_system_target(project: &str, config: &str) {
    if cfg!(target_arch = "wasm32") {
        clear_editor_notices();
        dom::hidden("system-plan-form", true);
        dom::clear("system-plan-services");
        dom::set_checked("system-reviewed", false);
        for (error, message) in [
            ("system-error", "system-message"),
            ("system-run-error", "system-run-message"),
        ] {
            dom::error(error, "");
            dom::text(message, "");
        }
    }
    m::bind_scope("editor-scope", project, config);
    m::next("system-read");
    m::next("system-list");
    m::next("discovery");
    m::bind_scope("system-scope", project, "");
    if cfg!(target_arch = "wasm32") {
        dom::options("system-runs", &[], m::summary, None);
        crate::activity::reset("system-agents");
        crate::activity::reset("system-discovery-agent");
        dom::cancel_timer("system-poll");
        dom::cancel_timer("system-sse-refresh");
        dom::cancel_timer("system-sse-list");
        dom::cancel_timer("system-graph-poll");
        dom::cancel_timer("discovery-poll");
    }
    for key in [
        "editor",
        "editor-generation",
        "system-run",
        "system-permissions-owner",
        "system-catalog",
        "system-catalog-probed",
        "system-graph-status",
        "system-graph-monotonic",
        "system-live-focus",
        "system-member-selected",
        "system-change-selected-service",
        "discovery-id",
        "discovery-owner",
        "system-poll-errors",
        "system-poll-error-shown",
        "system-failed-services",
    ] {
        m::take(key);
    }
    if cfg!(target_arch = "wasm32") {
        dom::set_value("system-permissions", "auto");
        dom::set_value("system-recovery-permissions", "auto");
        run::sync_waiting_permissions("system-permissions-list", &[], false);
        dom::text("system-permissions-lead", "");
    }
    for key in ["editor-dirty", "editor-pending", "discovery-active"] {
        m::set(key, json!(false));
    }
    if cfg!(target_arch = "wasm32") {
        for id in [
            "system-editor",
            "system-run",
            "system-permissions-panel",
            "system-catalog",
            "system-change-map-workspace",
            "system-discovery-agents",
        ] {
            dom::hidden(id, true);
        }
    }
}

#[allow(dead_code)]
pub fn initialize() {
    if m::yes(&m::data("systems-initialized")) {
        return;
    }
    m::set("systems-initialized", json!(true));
    crate::api::on_sse("system_run", crate::systems::on_sse_system_run);
    crate::api::on_sse("system_graph_status", |payload| {
        let project = dom::value("system-project");
        let ev_project = m::text(payload, "project_id");
        if !project.is_empty() && (ev_project.is_empty() || ev_project == project) {
            spawn_local(async {
                poll_system_graph_status(false).await;
            });
        }
    });
    crate::api::on_sse("resync", crate::systems::on_sse_system_resync);
    crate::session::on_project_change(crate::systems::on_global_project_change);
}

pub(super) fn can_switch_system_project_now() -> bool {
    !m::yes(&m::data("editor-dirty"))
        && !m::yes(&m::data("editor-pending"))
        && !m::yes(&m::data("system-pending"))
        && !m::yes(&m::data("discovery-active"))
}

pub(super) fn consume_pending_project_sync() -> bool {
    if !can_switch_system_project_now() {
        return false;
    }
    let pending_project = m::take("system-pending-project-sync");
    let Some(project) = pending_project
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    on_global_project_change(project);
    true
}

#[allow(dead_code)]
pub(crate) fn on_global_project_change(project: &str) {
    let project = project.trim();
    if project.is_empty() {
        return;
    }
    if !can_switch_system_project_now() {
        m::set("system-pending-project-sync", json!(project));
        return;
    }
    m::take("system-pending-project-sync");
    let prev = if cfg!(target_arch = "wasm32") {
        dom::value("system-project")
    } else {
        m::string(&m::data("system-project-sync"))
    };
    m::set("system-project-sync", json!(project));
    if cfg!(target_arch = "wasm32") {
        dom::set_value("system-project", project);
    }
    if prev != project {
        reset_system_target(project, "");
        if cfg!(target_arch = "wasm32") {
            crate::graph_map::load_system_embedded(
                crate::graph_map::SystemEmbeddedKind::Member,
                None,
                false,
            );
            crate::graph_map::load_system_embedded(
                crate::graph_map::SystemEmbeddedKind::Change,
                None,
                false,
            );
            let active_view = m::read(|s| s.view.clone());
            if runs::is_systems_view(&active_view) {
                spawn_local(async {
                    load().await;
                });
            }
        }
    }
}
pub(super) fn system_current(project: &str, id: &str, generation: u64) -> bool {
    dom::value("system-project") == project
        && m::scope_current("system-scope", generation, project, id)
}
pub(super) fn discovery_current(epoch: u64) -> bool {
    let owner = m::data("discovery-owner");
    m::current("discovery", epoch)
        && editor_current(
            &m::text(&owner, "project"),
            &m::text(&owner, "config"),
            owner["generation"].as_u64().unwrap_or(0),
        )
}

pub async fn load() {
    initialize();
    if !sync_editor_target() {
        return;
    }
    dom::cancel_timer("system-poll");
    m::next("system-read");
    let project = dom::value("system-project");
    let generation = m::bind_scope("system-scope", &project, "");
    m::take("system-run");
    dom::hidden("system-run", true);
    crate::activity::reset("system-agents");
    if project.is_empty() {
        return;
    }
    if m::data("system-catalog").is_null() && !m::yes(&m::data("editor-dirty")) {
        catalog_fetch(true).await;
    } else {
        poll_system_graph_status(false).await;
    }
    let path = format!("/api/system-runs?project_id={}", dom::enc(&project));
    match api::scoped("system-list", &path).await {
        Ok((epoch, value))
            if m::current("system-list", epoch) && system_current(&project, "", generation) =>
        {
            dom::options("system-runs", &m::array(&value["runs"]), run_label, None);
            dom::hidden("system-runs-empty", !m::array(&value["runs"]).is_empty());
            let id = dom::value("system-runs");
            if !id.is_empty() {
                read(&id).await;
            } else {
                sync_changes_map_workspace();
                controls();
            }
        }
        Err(error) if !error.aborted && system_current(&project, "", generation) => {
            dom::error("system-run-error", &error.message)
        }
        _ => {}
    }
}

pub(super) fn controls() {
    let _ = consume_pending_project_sync();
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let pending = m::yes(&m::data("system-pending"));
    let dirty = m::yes(&m::data("editor-dirty"));
    let locked = m::yes(&m::data("editor-pending"));
    let v = m::data("system-run");
    let active = m::yes(&v["active"]);
    let provider = m::array(&v["providers"])
        .into_iter()
        .find(|v| v["id"] == dom::value("system-provider"))
        .unwrap_or(V::Null);
    for id in [
        "system-project",
        "system-config",
        "system-load",
        "system-choose-project",
    ] {
        dom::disabled(id, pending || locked || dirty);
    }
    for e in dom::all(
        "#system-editor input,#system-editor select,#system-editor textarea,#system-editor button",
    ) {
        // The scan's own Cancel stays usable while the scan locks the form.
        if e.id() != "system-discovery-cancel"
            && e.closest("#system-discovery-agents")
                .ok()
                .flatten()
                .is_none()
        {
            if locked {
                let _ = e.set_attribute("disabled", "");
            } else {
                let _ = e.remove_attribute("disabled");
            }
        }
    }
    // A plan is computed from the saved system files, so a draft or an AI scan
    // in the editor holds planning and launching until it is saved or discarded.
    let scanning = m::yes(&m::data("discovery-active"));
    let held = dirty || scanning;
    let (lock_state, notice, show_discard) = editor_lock_state(scanning, dirty);
    sync_editor_lock(lock_state, notice, show_discard, locked);
    dom::disabled("system-prepare", pending || held);
    let system_mode = dom::value("system-mode");
    let system_provider = dom::value("system-provider");
    let system_perm = dom::value("system-permissions");
    dom::disabled(
        "system-permissions",
        pending || held || active || v["session_id"].is_string() || system_mode == "read-only",
    );
    let launch_editing = if system_mode == "edit" {
        "edit"
    } else {
        "plan"
    };
    dom::text(
        "system-permissions-hint",
        &system_permissions_hint(&system_provider, &system_mode, &system_perm),
    );
    dom::attr(
        "system-permissions-hint",
        "data-mode",
        session::permissions_hint_mode(system_mode != "edit", launch_editing, &system_perm),
    );
    let saved_mode = v["mode"]
        .as_str()
        .or_else(|| v["execution"]["mode"].as_str())
        .unwrap_or("read-only");
    let saved_provider = v["provider"].as_str().unwrap_or("");
    let recovery_perm = dom::value("system-recovery-permissions");
    dom::disabled(
        "system-recovery-permissions",
        pending
            || active
            || !v["session_id"].is_string()
            || v["status"] == "completed"
            || saved_mode != "edit",
    );
    let recovery_editing = if saved_mode == "edit" { "edit" } else { "plan" };
    dom::text(
        "system-recovery-permissions-hint",
        &system_permissions_hint(saved_provider, saved_mode, &recovery_perm),
    );
    dom::attr(
        "system-recovery-permissions-hint",
        "data-mode",
        session::permissions_hint_mode(saved_mode != "edit", recovery_editing, &recovery_perm),
    );
    dom::disabled(
        "system-execute",
        pending
            || held
            || active
            || v["session_id"].is_string()
            || v["plan"]["status"] != "needs_review"
            || !dom::checked("system-reviewed")
            || !m::yes(&provider["available"]),
    );
    dom::disabled(
        "system-resume",
        pending || active || !v["session_id"].is_string() || v["status"] == "completed",
    );
    dom::disabled("system-cancel", pending || !active);
    dom::hidden(
        "system-recovery",
        !v["session_id"].is_string() || active || v["status"] == "completed",
    );
    dom::hidden("system-launch", v["session_id"].is_string());
    dom::hidden("system-cancel", !active);
    dom::hidden("system-archive", !v.is_object() || active);
    dom::hidden("system-replan", !v.is_object() || active);
    dom::disabled("system-archive", pending || active);
    let editor = m::data("editor");
    dom::disabled("system-edit", pending || locked);
    dom::disabled("system-editor-save", locked || !editor.is_object());
    dom::disabled("system-editor-add", locked || !editor.is_object());
    dom::disabled(
        "system-discovery-start",
        locked
            || m::array(&editor["services"]).is_empty()
            || !m::yes(&m::provider(&dom::value("system-discovery-provider"))["available"]),
    );
    dom::hidden(
        "system-discovery-cancel",
        !m::yes(&m::data("discovery-active")),
    );
    dom::disabled(
        "system-discovery-cancel",
        !m::yes(&m::data("discovery-active")),
    );
    layout();
    crate::session::sync_working_indicator();
}
/// The empty state stands in for the map and the editor, so it hides while either shows.
pub(super) fn layout() {
    let shown = |id: &str| dom::get(id).is_some_and(|e| !e.has_attribute("hidden"));
    dom::hidden(
        "system-empty",
        shown("system-editor") || shown("system-catalog"),
    );
}

pub async fn event(
    kind: &str,
    id: &str,
    action: &str,
    payload: &V,
    value: &str,
    checked: bool,
) -> bool {
    if kind == "input" && crate::graph_map::handle_system_input(id, value) {
        return true;
    }
    if kind == "change" && crate::graph_map::handle_system_change(id, value, checked) {
        return true;
    }
    if kind == "input" && id == "system-task" {
        crate::graph_map::focus_system_from_prompt(
            crate::graph_map::SystemEmbeddedKind::Change,
            value,
        );
    }
    if kind == "input" || kind == "change" {
        if action == "system-service" || id == "system-depth" || id == "system-contracts" {
            sync_changes_map_workspace();
            return true;
        }
        if id == "system-member-select" {
            if !value.is_empty() {
                drill_down_service_map("services", value, None);
            }
            return true;
        }
        if id == "system-graphs-agents" {
            let act = if checked { "offer" } else { "withhold" };
            if !m::yes(&m::data("system-graph-prompt-touched")) {
                dom::set_checked("system-graph-prompt", checked);
            }
            post_system_graph_action(act, None).await;
            return true;
        }
        if id == "system-graph-prompt" {
            m::set("system-graph-prompt-touched", json!(true));
            return true;
        }
        if id == "system-editor-name" {
            if !editor_ready() || m::yes(&m::data("editor-pending")) {
                return true;
            }
            let mut data = m::data("editor");
            data["name"] = json!(value);
            m::set("editor", data);
            changed();
            return true;
        }
        if action == "structured-events-search" && m::text(payload, "scope") == "system-events" {
            events_card::set_event_filter_query("system-events", value);
            render_system_events_only();
            return true;
        }
        if id == "system-project" {
            if !sync_editor_target() {
                return true;
            }
            if !m::yes(&m::data("editor-dirty")) {
                m::take("system-run");
                m::take("system-catalog");
                crate::graph_map::load_system_embedded(
                    crate::graph_map::SystemEmbeddedKind::Member,
                    None,
                    false,
                );
                crate::graph_map::load_system_embedded(
                    crate::graph_map::SystemEmbeddedKind::Change,
                    None,
                    false,
                );
                load().await;
            }
            return true;
        }
        if id == "system-runs" {
            read(value).await;
            return true;
        }
        if id == "system-config" {
            if !sync_editor_target() {
                return true;
            }
            m::take("system-catalog");
            dom::hidden("system-catalog", true);
            return true;
        }
        if id == "system-agents-launch" {
            m::set("system-agent-launch", json!(value));
            let v = m::data("system-run");
            agents(&v).await;
            return true;
        }
        if [
            "system-provider",
            "system-access",
            "system-mode",
            "system-permissions",
            "system-recovery-permissions",
            "system-reviewed",
            "system-discovery-provider",
        ]
        .contains(&id)
        {
            if id == "system-provider" {
                sync_system_explanation_context_from_dom();
            }
            controls();
            return true;
        }
    }
    if kind == "submit" {
        match id {
            "system-editor" => {
                save_editor().await;
                return true;
            }
            "system-plan-form" => {
                plan().await;
                return true;
            }
            _ => {}
        }
    }
    if kind != "click" {
        return false;
    }
    if crate::graph_map::handle_system_click(id, action, payload) {
        return true;
    }
    match action {
        "system-load" => catalog_load().await,
        "structured-events-kind" if m::text(payload, "scope") == "system-events" => {
            events_card::set_event_filter_kind("system-events", &m::text(payload, "kind"));
            render_system_events_only();
        }
        "system-localize-failure" => events_card::localize_failure_payload(payload),
        "system-refresh" => load().await,
        "system-edit" => editor_open().await,
        "system-choose-project" => {
            management::folder_open_at(
                "system-project",
                &m::text(&m::project(&dom::value("system-project")), "path"),
                "Choose a system folder",
            )
            .await
        }
        "system-editor-add" => service_folder(None).await,
        "editor-service-folder" => service_folder(Some(payload.as_u64().unwrap_or(0))).await,
        "editor-row-remove" => {
            if m::yes(&m::data("editor-pending")) || !editor_ready() {
                return true;
            }
            let mut data = m::data("editor");
            if let Some(rows) = data
                .pointer_mut(payload["path"].as_str().unwrap_or(""))
                .and_then(V::as_array_mut)
            {
                if let Some(index) = payload["index"].as_u64() {
                    if (index as usize) < rows.len() {
                        rows.remove(index as usize);
                    }
                }
            }
            m::set("editor", data);
            changed();
            render_editor();
        }
        "editor-row-add" => {
            if m::yes(&m::data("editor-pending")) || !editor_ready() {
                return true;
            }
            let empty = match payload["kind"].as_str().unwrap_or("") {
                "capabilities" => {
                    json!({"id":"","description":"","status":"unknown","sources":[],"keywords":[]})
                }
                "provides" => json!({"id":"","kind":"http","version":"1","sources":[]}),
                "consumes" => json!({"service":"","contract":"","version":"1"}),
                _ => json!({"path":"","kind":"code"}),
            };
            let mut data = m::data("editor");
            if let Some(rows) = data
                .pointer_mut(payload["path"].as_str().unwrap_or(""))
                .and_then(V::as_array_mut)
            {
                rows.push(empty);
            }
            m::set("editor", data);
            changed();
            render_editor();
        }
        "system-editor-close" => {
            dom::hidden("system-editor", true);
            layout();
        }
        "system-editor-discard" => discard_editor_draft(false),
        "system-editor-lock-discard" => discard_editor_draft(true),
        "system-plan-change" | "system-new-change" => {
            if m::data("system-catalog").is_null() {
                catalog_load().await;
            }
            session::open_view("system-changes", true).await;
            dom::hidden("system-plan-form", false);
            // The form is the next step; "no changes yet" would only repeat it.
            dom::hidden("system-runs-empty", true);
            sync_changes_map_workspace();
        }
        "system-replan" => {
            let v = m::data("system-run");
            if !v.is_object() || m::yes(&v["active"]) {
                return true;
            }
            if m::data("system-catalog").is_null() {
                catalog_load().await;
            }
            replan(&v);
            sync_changes_map_workspace();
        }
        "system-plan-cancel" => {
            dom::hidden("system-plan-form", true);
            dom::hidden("system-runs-empty", !dom::value("system-runs").is_empty());
            sync_changes_map_workspace();
        }
        "system-graphs-rebuild" => {
            let st = m::data("system-graph-status");
            let act = if matches!(st["state"].as_str(), Some("off" | "empty" | "")) {
                "enable"
            } else {
                "rebuild"
            };
            post_system_graph_action(act, None).await;
        }
        "system-map-level-topology" => switch_services_to_topology(),
        "system-map-level-service" => {
            let svc = m::data("system-member-selected")
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| dom::value("system-member-select"));
            if !svc.is_empty() {
                drill_down_service_map("services", &svc, None);
            }
        }
        "system-change-tab-topology" => switch_changes_to_topology(),
        "system-change-service-tab" => {
            let svc = m::text(payload, "service");
            if !svc.is_empty() {
                drill_down_service_map("changes", &svc, None);
            }
        }
        "system-change-attach-context" => {
            let brief = system_map::current_context_brief();
            if !brief.is_empty() {
                if dom::get("system-plan-form").is_some_and(|f| f.has_attribute("hidden")) {
                    dom::hidden("system-plan-form", false);
                }
                if session::append_draft_block("system-task", &brief, 2000) {
                    dom::text(
                        "system-run-message",
                        "Attached blast-radius map context to the task prompt.",
                    );
                }
            }
        }
        "system-working-pause" | "system-changes-working-pause" => {
            let paused = !m::yes(&m::data("system-working-paused"));
            m::set("system-working-paused", json!(paused));
            let flag = if paused { "true" } else { "false" };
            let label = if paused { "Resume" } else { "Pause" };
            for box_id in [
                "system-working",
                "system-changes-working",
                "system-map-workspace",
                "system-change-map-workspace",
            ] {
                dom::attr(box_id, "data-motion-paused", flag);
            }
            for btn_id in ["system-working-pause", "system-changes-working-pause"] {
                dom::attr(btn_id, "aria-pressed", flag);
                dom::text(btn_id, label);
            }
        }
        "system-service-graph-build" => {
            let svc = m::text(payload, "service");
            let svc_state = m::text(payload, "state");
            let act = if matches!(svc_state.as_str(), "off" | "empty" | "") {
                "enable"
            } else {
                "rebuild"
            };
            post_system_graph_action(act, Some(&svc)).await;
        }
        "system-service-code-map" => {
            let svc = m::text(payload, "service");
            let project = dom::value("system-project");
            let config = dom::value("system-config");
            if !svc.is_empty() && !project.is_empty() {
                let scope = build_system_graph_scope(&project, &config, &svc, false);
                crate::graph_map::open_system_service_map(scope, None);
            }
        }
        "system-service-changes-map" => {
            let svc = m::text(payload, "service");
            let project = dom::value("system-project");
            let config = dom::value("system-config");
            if !svc.is_empty() {
                switch_services_to_topology();
                system_map::focus_service_changes(&svc);
                if !project.is_empty() {
                    let scope = build_system_graph_scope(&project, &config, &svc, false);
                    crate::graph_map::open_system_service_map(scope, Some("work"));
                }
            }
        }
        "inspect-system-file-graph" => {
            // The file (and optional symbol) is shown in the code map of its service
            // that is embedded in this view, and the view is brought to it.
            let svc = m::text(payload, "service");
            let path = m::text(payload, "path");
            let symbol = extract_system_file_symbol(payload);
            let is_changes_view = m::read(|s| s.view == "system-changes");
            if !svc.is_empty() && !path.is_empty() {
                let surf = if is_changes_view {
                    "changes"
                } else {
                    "services"
                };
                drill_down_service_target(surf, &svc, Some(&path), symbol.as_deref());
                focus_system_graph_workspace();
            }
        }
        "system-execute" => {
            if dom::checked("system-reviewed") {
                mutate(
                    "execute",
                    system_execute_payload(
                        &dom::value("system-provider"),
                        &dom::value("system-mode"),
                        &dom::value("system-access"),
                        &dom::value("system-permissions"),
                        dom::num("system-timeout"),
                        dom::checked("system-graph-prompt"),
                    ),
                )
                .await;
            }
        }
        "system-cancel" => mutate("cancel", json!({})).await,
        "system-archive" => {
            mutate("archive", json!({})).await;
            if m::data("system-run")["archived"] == true {
                load().await;
            }
        }
        "system-resume" => {
            let extra = system_resume_payload(
                dom::checked("system-accept-changes"),
                dom::checked("system-graph-prompt"),
                &dom::value("system-recovery-permissions"),
                dom::checked("system-accept-unrun"),
                &dom::value("system-retry"),
            );
            mutate("resume", extra).await;
        }
        "permission-decide" => {
            if run::permission_event_container(payload) != "system-permissions-list" {
                return false;
            }
            let v = m::data("system-run");
            let backing_sid = v["session_id"]
                .as_str()
                .or_else(|| v["session"]["id"].as_str())
                .unwrap_or("")
                .to_owned();
            let run_id = m::text(&v, "id");
            if !backing_sid.is_empty()
                && run::decide_permission_for("system-permissions-list", &backing_sid, payload)
                    .await
                && !run_id.is_empty()
            {
                read(&run_id).await;
            }
        }
        "system-open-session" => {
            let sid = m::text(&m::data("system-run"), "session_id");
            session::select(&sid).await;
        }
        "system-discovery-start" => discovery_start().await,
        "system-discovery-cancel" => {
            let epoch = m::read(|s| s.epochs.get("discovery").copied().unwrap_or(0));
            if !discovery_current(epoch) || !editor_ready() {
                return true;
            }
            let id = m::string(&m::data("discovery-id"));
            if let Err(e) = api::post(
                &format!("/api/sessions/{}/cancel", dom::enc(&id)),
                json!({}),
            )
            .await
            {
                if discovery_current(epoch) {
                    dom::error("system-editor-error", &e.message);
                }
            }
            if discovery_current(epoch) {
                discovery_poll(epoch).await;
            }
        }
        _ => return false,
    }
    true
}

pub(super) fn extract_system_file_symbol(payload: &V) -> Option<String> {
    ["symbol_fqn", "symbol"]
        .into_iter()
        .map(|k| m::text(payload, k))
        .find(|s| !s.is_empty())
        .or_else(|| {
            m::array(&payload["from_symbols"])
                .into_iter()
                .filter_map(|v| {
                    v.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
                .next()
        })
        .or_else(|| {
            let sid = m::text(payload, "symbol_id");
            (!sid.is_empty()).then_some(sid)
        })
}

#[cfg(test)]
#[allow(unused_imports)]
mod test_exports {
    pub(super) use super::catalog_map::*;
    pub(super) use super::editor::*;
    pub(super) use super::runs::*;
    pub(super) use super::*;
}

#[cfg(test)]
mod sync_tests {
    use super::*;

    fn clear_sync_test_keys() {
        for key in [
            "editor-dirty",
            "editor-pending",
            "system-pending",
            "discovery-active",
            "system-pending-project-sync",
            "system-project-sync",
            "system-run",
            "system-catalog",
            "system-graph-status",
            "system-graph-map",
        ] {
            m::take(key);
        }
    }

    #[test]
    fn deferred_project_sync_consumes_when_blocking_states_clear() {
        clear_sync_test_keys();
        m::set("system-project-sync", json!("proj-a"));

        m::set("editor-dirty", json!(true));
        on_global_project_change("proj-b");
        assert_eq!(m::data("system-pending-project-sync"), json!("proj-b"));
        assert_eq!(m::string(&m::data("system-project-sync")), "proj-a");
        assert!(!consume_pending_project_sync());
        assert_eq!(m::data("system-pending-project-sync"), json!("proj-b"));

        m::set("editor-dirty", json!(false));
        m::set("discovery-active", json!(true));
        assert!(!consume_pending_project_sync());
        assert_eq!(m::data("system-pending-project-sync"), json!("proj-b"));

        m::set("discovery-active", json!(false));
        controls();
        assert!(m::data("system-pending-project-sync").is_null());
        assert_eq!(m::string(&m::data("system-project-sync")), "proj-b");
        clear_sync_test_keys();
    }
}
