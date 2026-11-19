use super::setup::{build_file_preview_details, file_preview_item};
use crate::{activity::events_card, api, dom, model as m, run, session};
use serde_json::{json, Value as V};
use std::collections::BTreeSet;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CreatorReportView {
    pub(super) key: String,
    pub(super) summary: String,
    pub(super) text: String,
}

pub(super) fn creator_report_view(item: &V, idx: usize) -> CreatorReportView {
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

pub(super) fn normalized_creator_permission(value: Option<&str>) -> &'static str {
    match value {
        Some("manual") => "manual",
        Some("bypass") => "bypass",
        _ => "auto",
    }
}

pub(super) fn creator_restored_permissions(run: &V) -> &'static str {
    session::restored_value(
        "permissions",
        normalized_creator_permission(run["permissions"].as_str()),
    )
}

pub(super) fn creator_permissions_hint_text(provider: &str, permission: &str) -> &'static str {
    session::permissions_hint(provider, "edit", permission)
}

pub(super) fn creator_permissions_hint() {
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

pub(super) fn creator_target_summary(run: &V) -> String {
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

pub(super) fn creator_status_text(run: &V, waiting: &[V]) -> String {
    let base = m::label(run["status"].as_str().unwrap_or(""));
    if creator_busy(run) && !waiting.is_empty() {
        format!("{base} · Waiting for your answer to a permission request…")
    } else {
        base
    }
}

pub(super) fn creator_permissions_panel_state(v: &V) -> (bool, String) {
    let waiting = m::array(&v["waiting_permissions"]);
    (!waiting.is_empty(), run::permission_panel_lead(&waiting))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn creator_start_body(
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

pub(super) fn creator_routing_payload(
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
    m::take("creator-approved-paths");
    m::take("creator-poll-errors");
    m::take("creator-poll-error-shown");
    if cfg!(target_arch = "wasm32") {
        dom::hidden("creator-detail", true);
        dom::hidden("creator-permissions-panel", true);
        dom::text("creator-permissions-lead", "");
        run::sync_waiting_permissions("creator-permissions-list", &[], false);
        events_card::render_structured_events("creator-events", &[]);
        dom::clear("creator-files");
        dom::clear("creator-actions");
        dom::options("creator-runs", &[], m::summary, None);
        creator_permissions_hint();
        crate::session::sync_working_indicator();
    }
}
pub fn reset_creator_project() {
    m::next("creator-list");
    m::next("creator-read");
    m::take("creator-list");
    creator_reset_detail();
    if cfg!(target_arch = "wasm32") {
        dom::cancel_timer("creator-poll");
        dom::cancel_timer("creator-sse-refresh");
        dom::cancel_timer("creator-sse-list");
        dom::set_value("creator-permissions", "auto");
        creator_mutation_controls(false);
    }
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
pub(super) fn creator_should_retry_poll(cached_detail: &V, id: &str, status: u16) -> bool {
    if events_card::is_terminal_poll_status(status) {
        return false;
    }
    let loaded_same = m::text(&cached_detail["run"], "id") == id && cached_detail["run"].is_object();
    !(loaded_same && !creator_busy(&cached_detail["run"]))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CreatorSseOutcome {
    Ignore,
    UpdatedRenderOnly,
    RefreshRunAndList { immediate: bool },
    RefreshListOnly,
}

fn first_non_empty_field(v: &V, keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|k| v[*k].as_str())
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_owned()
}

pub(super) fn apply_sse_creator_delta(
    detail: &mut V,
    current_project: &str,
    current_id: &str,
    data: &V,
) -> CreatorSseOutcome {
    let event_project = first_non_empty_field(data, &["project_id", "project"]);
    if !event_project.is_empty() && !current_project.is_empty() && event_project != current_project {
        return CreatorSseOutcome::Ignore;
    }
    let event_id = first_non_empty_field(data, &["run_id", "id"]);
    let event_id = if event_id.is_empty() {
        m::text(&data["run"], "id")
    } else {
        event_id
    };
    if event_id.is_empty() {
        return CreatorSseOutcome::Ignore;
    }
    if current_id.is_empty() || event_id != current_id {
        return CreatorSseOutcome::RefreshListOnly;
    }
    let has_full_snapshot = data["run"].is_object()
        && (data["reports"].is_object()
            || data["reports"].is_array()
            || data["preview"].is_object()
            || data["events"].is_array());
    if has_full_snapshot {
        let generation = detail["view_generation"].clone();
        *detail = data.clone();
        if !generation.is_null() {
            detail["view_generation"] = generation;
        }
        return CreatorSseOutcome::UpdatedRenderOnly;
    }
    let mut rendered = false;
    let mut need_refresh = false;
    let mut immediate = false;

    if data["event"].is_object() || data["event"].is_string() {
        let mut evs = m::array(&detail["events"]);
        if events_card::merge_sse_event(&mut evs, data["event"].clone()) {
            detail["events"] = V::Array(evs);
            rendered = true;
        }
    }
    if let Some(run_obj) = data["run"].as_object() {
        if !detail["run"].is_object() {
            detail["run"] = json!({});
        }
        for (k, val) in run_obj {
            detail["run"][k] = val.clone();
        }
        rendered = true;
        need_refresh = true;
    }
    let new_status = m::text(data, "status");
    if !new_status.is_empty() && m::text(&detail["run"], "status") != new_status {
        if !detail["run"].is_object() {
            detail["run"] = json!({});
        }
        detail["run"]["status"] = json!(new_status);
        rendered = true;
        need_refresh = true;
        immediate = !creator_busy(&detail["run"]);
    }
    if let Some(rev) = data["revision"].as_u64() {
        if detail["run"]["revision"].as_u64() != Some(rev) {
            need_refresh = true;
        }
    }
    if data["waiting_permissions"].is_array() {
        detail["waiting_permissions"] = data["waiting_permissions"].clone();
        rendered = true;
    } else if data["waiting_permissions"].is_u64() {
        need_refresh = true;
    }
    if need_refresh || !creator_busy(&detail["run"]) {
        CreatorSseOutcome::RefreshRunAndList { immediate }
    } else if rendered {
        CreatorSseOutcome::UpdatedRenderOnly
    } else {
        CreatorSseOutcome::RefreshRunAndList { immediate: false }
    }
}

fn schedule_creator_poll(id: &str, project: &str, generation: u64, delay_ms: i32) {
    let id = id.to_owned();
    let project = project.to_owned();
    dom::later("creator-poll", delay_ms, move || {
        spawn_local(async move {
            creator_read_scoped(&id, &project, generation).await;
        });
    });
}

#[allow(dead_code)]
pub(crate) fn on_sse_creator_resync(_: &V) {
    if !m::read(|s| s.view == "creator") {
        return;
    }
    let project = dom::value("creator-project");
    let detail = m::data("creator-detail");
    let id = m::text(&detail["run"], "id");
    let generation = m::read(|s| s.epochs.get("creator-scope").copied().unwrap_or(0));
    if !id.is_empty() && creator_current(&project, &id, generation) {
        spawn_local(async move {
            creator_read_scoped(&id, &project, generation).await;
        });
    }
    spawn_local(async {
        creator_list().await;
    });
}

#[allow(dead_code)]
pub(crate) fn on_sse_creator_run(data: &V) {
    if !m::read(|s| s.view == "creator") {
        return;
    }
    let project = dom::value("creator-project");
    let mut detail = m::data("creator-detail");
    let current_id = m::text(&detail["run"], "id");
    let current_id = if current_id.is_empty() {
        dom::value("creator-runs")
    } else {
        current_id
    };
    let outcome = apply_sse_creator_delta(&mut detail, &project, &current_id, data);
    match outcome {
        CreatorSseOutcome::Ignore => {}
        CreatorSseOutcome::UpdatedRenderOnly => {
            m::set("creator-poll-errors", json!(0));
            if m::yes(&m::data("creator-poll-error-shown")) {
                dom::error("creator-action-error", "");
                m::set("creator-poll-error-shown", json!(false));
            }
            m::set("creator-detail", detail);
            render_creator();
            if !creator_busy(&m::data("creator-detail")["run"]) {
                dom::cancel_timer("creator-poll");
            }
        }
        CreatorSseOutcome::RefreshRunAndList { immediate } => {
            m::set("creator-poll-errors", json!(0));
            m::set("creator-detail", detail);
            render_creator();
            let generation = m::read(|s| s.epochs.get("creator-scope").copied().unwrap_or(0));
            if creator_current(&project, &current_id, generation) {
                let id_owned = current_id.clone();
                let proj_owned = project.clone();
                let delay = if immediate { 0 } else { 120 };
                dom::later("creator-sse-refresh", delay, move || {
                    spawn_local(async move {
                        creator_read_scoped(&id_owned, &proj_owned, generation).await;
                    });
                });
            }
            dom::later("creator-sse-list", 250, || {
                spawn_local(async {
                    creator_list().await;
                });
            });
        }
        CreatorSseOutcome::RefreshListOnly => {
            dom::later("creator-sse-list", 250, || {
                spawn_local(async {
                    creator_list().await;
                });
            });
        }
    }
}

pub(super) async fn creator_read_scoped(id: &str, project: &str, generation: u64) {
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
            m::set("creator-poll-errors", json!(0));
            if m::yes(&m::data("creator-poll-error-shown")) {
                dom::error("creator-action-error", "");
                m::set("creator-poll-error-shown", json!(false));
            }
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
                let delay = if events_card::sse_is_connected() {
                    15_000
                } else {
                    2000
                };
                schedule_creator_poll(id, project, generation, delay);
            }
        }
        Ok(_) => {}
        Err(error)
            if !error.aborted
                && creator_current(project, id, generation)
                && m::read(|s| s.view == "creator") =>
        {
            let detail = m::data("creator-detail");
            if creator_should_retry_poll(&detail, id, error.status) {
                let attempt = m::data("creator-poll-errors").as_u64().unwrap_or(0) as u32;
                let delay = events_card::retry_backoff_ms(attempt);
                m::set("creator-poll-errors", json!(attempt + 1));
                m::set("creator-poll-error-shown", json!(true));
                dom::error(
                    "creator-action-error",
                    &format!("{} (retrying in {}s)", error.message, delay / 1000),
                );
                schedule_creator_poll(id, project, generation, delay);
            } else {
                m::set("creator-poll-errors", json!(0));
                m::set("creator-poll-error-shown", json!(true));
                dom::error("creator-action-error", &error.message);
            }
        }
        Err(_) => {}
    }
}
pub(super) fn creator_busy(v: &V) -> bool {
    matches!(
        v["status"].as_str(),
        Some("scanning" | "generating" | "applying" | "rolling_back")
    )
}
pub(super) fn render_creator() {
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
    events_card::render_structured_events("creator-events", &m::array(&v["events"]));

    let run_id = m::text(run, "id");
    let preview_id = m::text(&v["preview"], "preview_id");
    let approved = sync_creator_approved_from_dom(&run_id, &preview_id);
    dom::hidden("creator-preview", !v["preview"].is_object());
    let file_items = build_creator_file_render_items(&v["preview"]);
    dom::keyed_render(
        "creator-files",
        &file_items,
        |item| m::text(item, "key"),
        |item| build_creator_file_element(item, &approved),
    );
    if cfg!(target_arch = "wasm32") {
        for input_el in
            dom::all("#creator-files input[type=checkbox][data-action=\"creator-replace\"]")
        {
            let path = dom::value_of(&input_el);
            let want = approved.contains(&path);
            if dom::checked_of(&input_el) != want {
                if let Some(input) = input_el.dyn_ref::<web_sys::HtmlInputElement>() {
                    input.set_checked(want);
                }
            }
        }
    }

    let action_items = build_creator_action_items(&v, &approved);
    dom::keyed_render(
        "creator-actions",
        &action_items,
        |item| m::text(item, "key"),
        build_creator_action_element,
    );
    if v["session"]["id"].is_string() {
        m::upsert(v["session"].clone());
    }
    dom::hidden(
        "creator-corrections",
        creator_busy(run) || run["status"] == "complete" || m::yes(&run["recovery_available"]),
    );
    creator_mutation_controls(m::yes(&m::data("creator-pending")));
}

pub(super) fn creator_approved_paths_for(run_id: &str, preview_id: &str) -> BTreeSet<String> {
    let saved = m::data("creator-approved-paths");
    if saved["run_id"] != run_id || (!preview_id.is_empty() && saved["preview_id"] != preview_id) {
        return BTreeSet::new();
    }
    m::array(&saved["paths"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

pub(super) fn set_creator_approved_path(
    run_id: &str,
    preview_id: &str,
    path: &str,
    approved: bool,
) -> BTreeSet<String> {
    let mut current = creator_approved_paths_for(run_id, preview_id);
    if !path.is_empty() {
        if approved {
            current.insert(path.to_owned());
        } else {
            current.remove(path);
        }
    }
    let list: Vec<String> = current.iter().cloned().collect();
    m::set(
        "creator-approved-paths",
        json!({ "run_id": run_id, "preview_id": preview_id, "paths": list }),
    );
    current
}

fn sync_creator_approved_from_dom(run_id: &str, preview_id: &str) -> BTreeSet<String> {
    let mut set = creator_approved_paths_for(run_id, preview_id);
    if cfg!(target_arch = "wasm32") {
        let checkboxes =
            dom::all("#creator-files input[type=checkbox][data-action=\"creator-replace\"]");
        if !checkboxes.is_empty() {
            for input_el in checkboxes {
                let path = dom::value_of(&input_el);
                if !path.is_empty() {
                    if dom::checked_of(&input_el) {
                        set.insert(path);
                    } else {
                        set.remove(&path);
                    }
                }
            }
            let list: Vec<String> = set.iter().cloned().collect();
            m::set(
                "creator-approved-paths",
                json!({ "run_id": run_id, "preview_id": preview_id, "paths": list }),
            );
        }
    }
    set
}

pub(super) fn creator_apply_disabled(preview: &V, approved_paths: &BTreeSet<String>) -> bool {
    !m::yes(&preview["can_apply"])
        || m::array(&preview["files"]).iter().any(|file| {
            m::yes(&file["requires_decision"]) && !approved_paths.contains(&m::text(file, "path"))
        })
}

pub(super) fn handle_creator_replace_toggle(path: &str, checked: bool) {
    let detail = m::data("creator-detail");
    let run_id = m::text(&detail["run"], "id");
    let preview_id = m::text(&detail["preview"], "preview_id");
    let approved = set_creator_approved_path(&run_id, &preview_id, path, checked);
    dom::disabled(
        "creator-apply",
        creator_apply_disabled(&detail["preview"], &approved),
    );
}

pub(super) fn build_creator_file_render_items(preview: &V) -> Vec<V> {
    let preview_id = m::text(preview, "preview_id");
    let files = m::array(&preview["files"]);
    let mut items = Vec::new();
    let mut seen = std::collections::BTreeMap::<String, usize>::new();
    for file in &files {
        let action = file["action"]
            .as_str()
            .or_else(|| file["status"].as_str())
            .unwrap_or("File")
            .to_owned();
        let path = m::text(file, "path");
        let base_key = format!("file:{action}:{path}");
        let dup = *seen.entry(base_key).and_modify(|n| *n += 1).or_insert(0);
        let mut detail_item = file_preview_item(file, dup);
        detail_item["kind"] = json!("file");
        items.push(detail_item);

        if m::yes(&file["requires_decision"]) {
            // Note: signature deliberately omits transient checked state so polling
            // for the same preview_id preserves native checkbox DOM nodes & focus.
            items.push(json!({
                "key": format!("approve:{path}"),
                "kind": "approve",
                "action": action,
                "path": path,
                "preview_id": preview_id,
            }));
        }
    }
    items
}

pub(super) fn build_creator_action_items(v: &V, approved_paths: &BTreeSet<String>) -> Vec<V> {
    let run = &v["run"];
    let mut buttons = Vec::new();
    let mut actions = Vec::new();
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
        let disabled = action == "apply" && creator_apply_disabled(&v["preview"], approved_paths);
        buttons.push(json!({
            "key": action,
            "kind": "creator-action",
            "id": format!("creator-{action}"),
            "label": label,
            "payload": action,
            "disabled": disabled,
        }));
    }
    if let Some(sid) = v["session"]["id"].as_str() {
        buttons.push(json!({
            "key": "session",
            "kind": "creator-session",
            "id": "creator-session",
            "label": "View results & run usage",
            "payload": sid,
            "disabled": false,
        }));
    }
    buttons
}

fn build_creator_file_element(item: &V, approved_paths: &BTreeSet<String>) -> web_sys::Element {
    if item["kind"] == "approve" {
        let path = m::text(item, "path");
        let action = m::text(item, "action");
        let checked = approved_paths.contains(&path);
        let el = dom::check(
            &format!("Approve {action}: {path}"),
            "creator-replace",
            &path,
            checked,
        );
        if let Ok(Some(input)) = el.query_selector("input") {
            let _ = input.set_attribute("data-focus-key", &format!("creator-approve:{path}"));
        }
        el
    } else {
        build_file_preview_details(item)
    }
}

fn build_creator_action_element(item: &V) -> web_sys::Element {
    let key = m::text(item, "key");
    let kind = m::text(item, "kind");
    let id = m::text(item, "id");
    let label = m::text(item, "label");
    let b = dom::button(&label, &kind, item["payload"].clone());
    if !id.is_empty() {
        let _ = b.set_attribute("id", &id);
    }
    let _ = b.set_attribute("data-focus-key", &format!("creator-action:{key}"));
    if m::yes(&item["disabled"]) {
        let _ = b.set_attribute("disabled", "");
    }
    b
}

pub(super) fn render_creator_events_only() {
    let v = m::data("creator-detail");
    events_card::render_structured_events("creator-events", &m::array(&v["events"]));
}
pub(super) async fn creator_start() {
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
pub(super) async fn creator_action(action: &str) {
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
        let preview_id = m::text(&data["preview"], "preview_id");
        let approved = sync_creator_approved_from_dom(&id, &preview_id);
        let approved_json: Vec<V> = approved.iter().map(|p| json!(p)).collect();
        if creator_apply_disabled(&data["preview"], &approved) {
            return;
        }
        body["preview_id"] = data["preview"]["preview_id"].clone();
        body["replace"] = json!(approved_json);
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
            if action == "apply" {
                m::take("creator-approved-paths");
            }
            value["view_generation"] = json!(generation);
            m::set("creator-detail", value.clone());
            if action != "budgets" {
                dom::set_value("creator-answers", "");
            }
            render_creator();
            if creator_busy(&value["run"]) {
                let delay = if events_card::sse_is_connected() {
                    15_000
                } else {
                    1000
                };
                schedule_creator_poll(&id, &project, generation, delay);
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
