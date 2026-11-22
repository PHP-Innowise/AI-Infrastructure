use super::*;
use crate::activity::events_card;
use crate::{api, dom, model as m, run, session};
use serde_json::{json, Value as V};
use std::collections::{BTreeMap, BTreeSet};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

pub(super) fn normalized_system_permission(value: Option<&str>) -> &'static str {
    match value {
        Some("manual") => "manual",
        Some("bypass") => "bypass",
        _ => "auto",
    }
}

pub(super) fn restored_system_launch_permission(v: &V) -> &'static str {
    session::restored_value(
        "permissions",
        normalized_system_permission(v["permissions"].as_str()),
    )
}

pub(super) fn restored_system_recovery_permission(v: &V) -> &'static str {
    normalized_system_permission(v["permissions"].as_str())
}

pub(crate) fn system_permissions_hint(provider: &str, mode: &str, permission: &str) -> String {
    let editing = if mode == "edit" { "edit" } else { "plan" };
    let base = session::permissions_hint(provider, editing, permission);
    if editing == "edit" && normalized_system_permission(Some(permission)) == "auto" {
        format!(
            "{base} Under Auto, cross-service or contract-boundary writes escalate to Manual confirmation."
        )
    } else {
        base.to_owned()
    }
}

pub(super) fn is_systems_view(view: &str) -> bool {
    matches!(view, "systems" | "system-changes")
}

pub(super) fn system_permissions_panel_state(v: &V) -> (bool, String) {
    let waiting = m::array(&v["waiting_permissions"]);
    if !waiting.is_empty() {
        return (true, run::permission_panel_lead(&waiting));
    }
    if let Some(count) = v["waiting_permissions"].as_u64().filter(|&n| n > 0) {
        let noun = if count == 1 {
            "permission request"
        } else {
            "permission requests"
        };
        return (true, format!("{count} {noun} waiting for your decision."));
    }
    (false, String::new())
}

pub(super) fn system_execute_payload(
    provider: &str,
    mode: &str,
    access: &str,
    permissions: &str,
    timeout: V,
    tell_agents_graph: bool,
) -> V {
    json!({
        "provider": provider,
        "mode": mode,
        "access": access,
        "permissions": normalized_system_permission(Some(permissions)),
        "timeout": timeout,
        "tell_agents_graph": tell_agents_graph,
    })
}

pub(super) fn system_resume_payload(
    accept_source_changes: bool,
    tell_agents_graph: bool,
    permissions: &str,
    accept_unrun_checks: bool,
    retry_step: &str,
) -> V {
    let mut extra = json!({
        "accept_source_changes": accept_source_changes,
        "tell_agents_graph": tell_agents_graph,
        "permissions": normalized_system_permission(Some(permissions)),
    });
    if accept_unrun_checks {
        extra["accept_unrun_checks"] = json!(true);
    } else if !retry_step.is_empty() {
        extra["retry_step"] = json!(retry_step);
    }
    extra
}

/// The run's status and what stopped it: the stopped dispatch's receipt says
/// that best (the agent's own reason, failed checks, the CLI's message); the
/// journal's error is the fallback, a code read as words.
pub(super) fn run_status(v: &V) -> String {
    let has_waiting = !m::array(&v["waiting_permissions"]).is_empty()
        || v["waiting_permissions"].as_u64().unwrap_or(0) > 0;
    if m::yes(&v["active"]) && v["status"] != "queued" && has_waiting {
        return "Waiting for your answer to a permission request…".into();
    }
    let status = m::label(v["status"].as_str().unwrap_or(""));
    // Launches share the Harness runner; one waits while another holds it.
    if v["status"] == "queued" {
        return "queued · waits for a free runner slot".into();
    }
    let stopped = m::array(&v["execution"]["steps"])
        .into_iter()
        .find(|step| matches!(step["status"].as_str(), Some("blocked" | "interrupted")));
    let receipt = stopped.as_ref().and_then(|step| {
        m::array(&v["receipts"])
            .into_iter()
            .find(|receipt| receipt["phase"] == step["id"] && receipt["attempt"] == step["attempt"])
    });
    let reason = match (&stopped, &receipt) {
        (Some(step), Some(receipt)) if receipt["error"] != "interrupted" => Some(format!(
            "{}: {}",
            m::text(step, "id"),
            receipt_outcome(receipt)
        )),
        // An executor that stopped (server restart, crash) left no receipt.
        (Some(step), None) if v["execution"]["error"].is_null() => Some(format!(
            "{} stopped before it saved a receipt; inspect its folder, then retry it",
            m::text(step, "id")
        )),
        _ => v["execution"]["error"].as_str().map(|error| {
            // A journal code reads as words; a sentence is shown as written.
            if error.contains(' ') {
                error.to_owned()
            } else {
                m::label(error)
            }
        }),
    };
    match reason {
        Some(reason) if !m::yes(&v["active"]) && reason != status => {
            format!("{status} · {reason}")
        }
        _ => status,
    }
}
/// Why a scan ended without a proposal. Its status says it at once; the error
/// event of a cancelled scan can arrive after the last poll.
pub(super) fn scan_end(job: &V) -> String {
    match job["status"].as_str() {
        Some("cancelled") => "AI scan cancelled.".into(),
        Some("interrupted") => {
            "AI scan stopped before it finished (Harness restarted or the agent was stopped)."
                .into()
        }
        _ => sentence(
            job["error"]
                .as_str()
                .unwrap_or("AI scan ended without a proposal."),
        ),
    }
}
/// A message that ends as a sentence, so another can follow it: a CLI's own
/// message usually has no full stop.
pub(super) fn sentence(text: &str) -> String {
    let text = text.trim_end();
    if text.ends_with(['.', '!', '?']) {
        text.to_owned()
    } else {
        format!("{text}.")
    }
}
/// The next change ID of a revised plan: `cancel-v3` → `cancel-v4`,
/// `cancel` → `cancel-2`.
pub(super) fn next_change_id(id: &str) -> String {
    let digits = id.chars().rev().take_while(char::is_ascii_digit).count();
    let (stem, number) = id.split_at(id.len() - digits);
    match number.parse::<u64>() {
        Ok(number) if !stem.is_empty() => format!("{stem}{}", number + 1),
        _ => format!("{id}-2"),
    }
}
/// Opens the plan form filled from a change: a blocked agreement asks for a
/// decision, and the next plan states it, so only the task needs editing.
pub(super) fn replan(v: &V) {
    let plan = &v["plan"];
    let impact = m::array(&plan["impact"]);
    let mut starting = impact
        .iter()
        .filter(|entry| {
            matches!(
                entry["reason"].as_str(),
                Some("selected" | "contract_provider")
            )
        })
        .map(|entry| m::text(entry, "service"))
        .collect::<Vec<_>>();
    // A plan that still needs a selection offers what the task matched.
    if starting.is_empty() {
        starting = m::array(&plan["routing"]["candidates"])
            .iter()
            .map(|candidate| m::text(candidate, "service"))
            .collect();
    }
    for e in dom::all("#system-plan-services input[type=checkbox]") {
        if let Some(input) = e.dyn_ref::<web_sys::HtmlInputElement>() {
            if !input.disabled() {
                input.set_checked(starting.contains(&input.value()));
            }
        }
    }
    let downstream = impact
        .iter()
        .any(|entry| entry["reason"] == "potential_consumer");
    let depth = if downstream || impact.is_empty() {
        "32"
    } else {
        "0"
    };
    dom::set_value("system-depth", depth);
    dom::set_value("system-task", &m::text(&plan["context"], "task"));
    dom::set_value("system-contracts", "");
    if let Some(budget) = plan["context_budget_chars"].as_u64() {
        dom::set_value("system-budget", &budget.to_string());
    }
    dom::set_value(
        "system-change-id",
        &next_change_id(&m::text(v, "change_id")),
    );
    dom::hidden("system-plan-form", false);
    dom::hidden("system-runs-empty", true);
    if let Some(task) =
        dom::get("system-task").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = task.focus();
    }
}
/// Which of Plan, Review, Run and Receipts a change has reached, and how that
/// stage stands: a change stops at Review until someone launches it. The
/// stepper was drawn by the browser UI before the Wasm port and stayed empty.
pub(super) fn stages(v: &V) -> Vec<(&'static str, &'static str)> {
    let active = m::yes(&v["active"]);
    let status = m::text(v, "status");
    let launched = v["session_id"].is_string();
    let attention = !active
        && (matches!(
            status.as_str(),
            "blocked" | "interrupted" | "failed" | "cancelled"
        ) || (!launched && v["plan"]["status"] != "needs_review"));
    let current = if !launched {
        1
    } else if status == "completed" && !active {
        4
    } else {
        2
    };
    ["Plan", "Review", "Run", "Receipts"]
        .into_iter()
        .enumerate()
        .map(|(index, label)| {
            let state = if index < current {
                "done"
            } else if index > current {
                "upcoming"
            } else if attention {
                "attention"
            } else if active {
                "running"
            } else {
                "current"
            };
            (label, state)
        })
        .collect()
}
pub(super) fn stepper(v: &V) {
    dom::clear("system-stepper");
    for (index, (label, state)) in stages(v).into_iter().enumerate() {
        let item = dom::el("li", "step", "");
        let _ = item.set_attribute("data-state", state);
        if matches!(state, "current" | "running" | "attention") {
            let _ = item.set_attribute("aria-current", "step");
        }
        let mark = if state == "done" {
            "✓".to_owned()
        } else {
            (index + 1).to_string()
        };
        dom::add(&item, &dom::el("span", "step-mark", &mark));
        dom::add(&item, &dom::el("span", "step-label", label));
        dom::append("system-stepper", &item);
    }
}
/// How a change reads in the change list.
pub(super) fn run_label(run: &V) -> String {
    format!(
        "{} · {}",
        m::text(run, "change_id"),
        m::label(run["status"].as_str().unwrap_or(""))
    )
}
/// Lists the project's changes again with `selected` chosen: a new plan is
/// shown under the change list it joins.
pub(super) async fn relist(project: &str, selected: &str) {
    let path = format!("/api/system-runs?project_id={}", dom::enc(project));
    let Ok((epoch, value)) = api::scoped("system-list", &path).await else {
        return;
    };
    if m::current("system-list", epoch) && dom::value("system-project") == project {
        dom::options(
            "system-runs",
            &m::array(&value["runs"]),
            run_label,
            Some(selected),
        );
        dom::hidden("system-runs-empty", !m::array(&value["runs"]).is_empty());
    }
}
pub(super) fn system_should_retry_poll(cached_run: &V, id: &str, status: u16) -> bool {
    if events_card::is_terminal_poll_status(status) {
        return false;
    }
    let loaded_same = m::text(cached_run, "id") == id && cached_run["active"].is_boolean();
    !(loaded_same && !m::yes(&cached_run["active"]))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SystemSseOutcome {
    Ignore,
    UpdatedRenderOnly,
    RefreshRun { immediate: bool, stopped: bool },
    RefreshListOnly,
}

fn first_non_empty_sys(v: &V, keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|k| v[*k].as_str())
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_owned()
}

pub(super) fn apply_sse_system_delta(
    run_doc: &mut V,
    current_project: &str,
    current_id: &str,
    data: &V,
) -> SystemSseOutcome {
    let event_project = first_non_empty_sys(data, &["project_id", "project"]);
    if !event_project.is_empty() && !current_project.is_empty() && event_project != current_project
    {
        return SystemSseOutcome::Ignore;
    }
    let event_id = first_non_empty_sys(data, &["run_id", "id"]);
    let event_id = if event_id.is_empty() {
        m::text(&data["run"], "id")
    } else {
        event_id
    };
    if event_id.is_empty() {
        return SystemSseOutcome::Ignore;
    }
    if current_id.is_empty() || event_id != current_id {
        return SystemSseOutcome::RefreshListOnly;
    }
    if data["plan"].is_object() || data["execution"].is_object() || data["receipts"].is_array() {
        let generation = run_doc["view_generation"].clone();
        *run_doc = data.clone();
        if !generation.is_null() {
            run_doc["view_generation"] = generation;
        }
        return SystemSseOutcome::UpdatedRenderOnly;
    }
    let was_active = m::yes(&run_doc["active"]);
    if !run_doc.is_object() {
        *run_doc = json!({});
    }
    if let Some(run_obj) = data["run"].as_object() {
        for (k, val) in run_obj {
            run_doc[k] = val.clone();
        }
    }
    if let Some(active) = data["active"].as_bool() {
        run_doc["active"] = json!(active);
    }
    let new_status = m::text(data, "status");
    if !new_status.is_empty() {
        run_doc["status"] = json!(new_status);
    }
    if data["session_id"].is_string() {
        run_doc["session_id"] = data["session_id"].clone();
    }
    let mut permissions_changed = false;
    if data["waiting_permissions"].is_array() {
        run_doc["waiting_permissions"] = data["waiting_permissions"].clone();
    } else if let Some(count) = data["waiting_permissions"].as_u64() {
        if count == 0 {
            run_doc["waiting_permissions"] = json!([]);
        } else {
            let existing_len = run_doc["waiting_permissions"]
                .as_array()
                .map(|a| a.len() as u64);
            if existing_len != Some(count) {
                run_doc["waiting_permissions"] = json!(count);
                permissions_changed = true;
            }
        }
    }
    let mut event_appended = false;
    if data["event"].is_object() || data["event"].is_string() {
        let mut evs = m::array(&run_doc["events"]);
        if events_card::merge_sse_event(&mut evs, data["event"].clone()) {
            run_doc["events"] = V::Array(evs);
            event_appended = true;
        }
    }
    let is_active = m::yes(&run_doc["active"]);
    let stopped = was_active && !is_active;
    if data["event"].is_object()
        && data["status"].is_null()
        && data["active"].is_null()
        && !permissions_changed
        && event_appended
    {
        return SystemSseOutcome::UpdatedRenderOnly;
    }
    SystemSseOutcome::RefreshRun {
        immediate: stopped || !is_active || permissions_changed,
        stopped,
    }
}

fn schedule_system_poll(id: &str, project: &str, generation: u64, delay_ms: i32) {
    let id = id.to_owned();
    let project = project.to_owned();
    dom::later("system-poll", delay_ms, move || {
        spawn_local(async move {
            read_scoped(&id, &project, generation).await;
        })
    });
}

#[allow(dead_code)]
pub(crate) fn on_sse_system_resync(_: &V) {
    if !m::read(|s| is_systems_view(&s.view)) {
        return;
    }
    let project = dom::value("system-project");
    let run_doc = m::data("system-run");
    let id = m::text(&run_doc, "id");
    let generation = m::read(|s| s.epochs.get("system-scope").copied().unwrap_or(0));
    if !id.is_empty() && system_current(&project, &id, generation) {
        let sel = id.clone();
        spawn_local(async move {
            read_scoped(&id, &project, generation).await;
            relist(&dom::value("system-project"), &sel).await;
        });
    } else if !project.is_empty() {
        spawn_local(async move {
            relist(&project, &dom::value("system-runs")).await;
        });
    }
}

#[allow(dead_code)]
pub(crate) fn on_sse_system_run(data: &V) {
    if !m::read(|s| is_systems_view(&s.view)) {
        return;
    }
    let project = dom::value("system-project");
    let mut current = m::data("system-run");
    let current_id = {
        let from_doc = m::text(&current, "id");
        if from_doc.is_empty() {
            dom::value("system-runs")
        } else {
            from_doc
        }
    };
    match apply_sse_system_delta(&mut current, &project, &current_id, data) {
        SystemSseOutcome::Ignore => {}
        SystemSseOutcome::UpdatedRenderOnly => {
            m::set("system-poll-errors", json!(0));
            if m::yes(&m::data("system-poll-error-shown")) {
                dom::error("system-run-error", "");
                m::set("system-poll-error-shown", json!(false));
            }
            m::set("system-run", current);
            render_run();
            if !m::yes(&m::data("system-run")["active"]) {
                dom::cancel_timer("system-poll");
            }
        }
        SystemSseOutcome::RefreshRun { immediate, stopped } => {
            m::set("system-poll-errors", json!(0));
            m::set("system-run", current);
            render_run();
            let generation = m::read(|s| s.epochs.get("system-scope").copied().unwrap_or(0));
            if system_current(&project, &current_id, generation) {
                let id_owned = current_id.clone();
                let proj_owned = project.clone();
                let delay = if immediate { 0 } else { 120 };
                dom::later("system-sse-refresh", delay, move || {
                    spawn_local(async move {
                        read_scoped(&id_owned, &proj_owned, generation).await;
                    });
                });
            }
            if stopped {
                dom::later("system-sse-list", 250, move || {
                    spawn_local(async move {
                        let proj = dom::value("system-project");
                        let sel = dom::value("system-runs");
                        relist(&proj, &sel).await;
                    });
                });
            }
        }
        SystemSseOutcome::RefreshListOnly => {
            dom::later("system-sse-list", 250, || {
                spawn_local(async {
                    let proj = dom::value("system-project");
                    let sel = dom::value("system-runs");
                    relist(&proj, &sel).await;
                });
            });
        }
    }
}

pub(crate) async fn read(id: &str) {
    let project = dom::value("system-project");
    let generation = m::bind_scope("system-scope", &project, id);
    read_scoped(id, &project, generation).await;
}
pub(super) async fn read_scoped(id: &str, project: &str, generation: u64) {
    if !system_current(project, id, generation) {
        return;
    }
    dom::cancel_timer("system-poll");
    if id.is_empty() {
        return;
    }
    match api::scoped("system-read", &format!("/api/system-runs/{}", dom::enc(id))).await {
        Ok((epoch, mut value))
            if m::current("system-read", epoch)
                && system_current(project, id, generation)
                && value["id"] == id
                && value["project_id"] == project =>
        {
            m::set("system-poll-errors", json!(0));
            if m::yes(&m::data("system-poll-error-shown")) {
                dom::error("system-run-error", "");
                m::set("system-poll-error-shown", json!(false));
            }
            let previous = m::data("system-run");
            let stopped = previous["id"] == value["id"]
                && m::yes(&previous["active"])
                && !m::yes(&value["active"]);
            if previous["id"] != value["id"] {
                dom::set_checked("system-reviewed", false);
                dom::set_checked("system-accept-changes", false);
                m::take("agent-session");
                m::take("agents");
                crate::activity::reset("system-agents");
            }
            value["view_generation"] = json!(generation);
            m::set("system-run", value.clone());
            ingest_run_graph_focus(&value);
            crate::activity::set_run_evidence(id, &value);
            render_run();
            agents(&value).await;
            // Sessions shows each launch; its status changes when the run stops.
            if stopped {
                session::refresh().await;
                poll_system_graph_status(false).await;
            }
            if system_current(project, id, generation) && m::yes(&value["active"]) {
                let delay = if events_card::sse_is_connected() {
                    15_000
                } else {
                    1100
                };
                schedule_system_poll(id, project, generation, delay);
            }
        }
        Ok(_) => {}
        Err(error) if !error.aborted && system_current(project, id, generation) => {
            let cached = m::data("system-run");
            if system_should_retry_poll(&cached, id, error.status) {
                let attempt = m::data("system-poll-errors").as_u64().unwrap_or(0) as u32;
                let delay = events_card::retry_backoff_ms(attempt);
                m::set("system-poll-errors", json!(attempt + 1));
                m::set("system-poll-error-shown", json!(true));
                dom::error(
                    "system-run-error",
                    &format!("{} (retrying in {}s)", error.message, delay / 1000),
                );
                schedule_system_poll(id, project, generation, delay);
            } else {
                m::set("system-poll-errors", json!(0));
                m::set("system-poll-error-shown", json!(true));
                dom::error("system-run-error", &error.message);
            }
        }
        _ => {}
    }
}

/// What a run step shows: a dispatch the journal left running is not running
/// once its run stopped, and one whose current attempt saved a successful
/// receipt has reported complete; resuming records that receipt.
pub(super) fn shown_status(step: &V, receipts: &V, active: bool) -> String {
    let status = m::text(step, "status");
    if active || !matches!(status.as_str(), "running" | "interrupted" | "blocked") {
        return status;
    }
    let reported = m::array(receipts).iter().any(|receipt| {
        receipt["phase"] == step["id"]
            && receipt["attempt"] == step["attempt"]
            && m::yes(&receipt["ok"])
    });
    if reported {
        "reported".into()
    } else if status == "running" {
        "interrupted".into()
    } else {
        status
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SystemRelationshipView {
    pub(super) provider: String,
    pub(super) consumer: String,
    pub(super) contract: String,
    pub(super) version: String,
    pub(super) provenance: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SystemStepView {
    pub(super) key: String,
    pub(super) id: String,
    pub(super) stage_class: String,
    pub(super) row_state: String,
    pub(super) status_label: String,
    pub(super) service: String,
    pub(super) mode: String,
    pub(super) attempt: Option<String>,
    pub(super) goal: String,
    pub(super) depends_on: Option<String>,
    pub(super) task_reference: Option<String>,
}

pub(super) fn build_relationship_view(edge: &V) -> SystemRelationshipView {
    let raw_version = m::text(edge, "version");
    let trimmed_ver = raw_version.trim();
    let version = if trimmed_ver.is_empty() {
        String::new()
    } else if trimmed_ver.starts_with('v') {
        trimmed_ver.to_owned()
    } else {
        format!("v{trimmed_ver}")
    };
    let raw_prov = m::text(edge, "provenance");
    let provenance = if raw_prov.trim().is_empty() {
        String::new()
    } else {
        m::label(raw_prov.trim())
    };
    let provider = m::text(edge, "provider");
    let consumer = m::text(edge, "consumer");
    let contract = m::text(edge, "contract");
    SystemRelationshipView {
        provider: if provider.trim().is_empty() {
            "provider".to_owned()
        } else {
            provider.trim().to_owned()
        },
        consumer: if consumer.trim().is_empty() {
            "consumer".to_owned()
        } else {
            consumer.trim().to_owned()
        },
        contract: if contract.trim().is_empty() {
            "contract".to_owned()
        } else {
            contract.trim().to_owned()
        },
        version,
        provenance,
    }
}

pub(super) fn step_stage_class(status: &str) -> &'static str {
    match status {
        "running" | "in_progress" | "active" => "system-stage running",
        "blocked" | "failed" | "error" => "system-stage blocked",
        "interrupted" | "reported" | "stopped" => "system-stage interrupted",
        _ => "system-stage",
    }
}

pub(super) fn step_row_state(status: &str) -> &'static str {
    match status {
        "running" | "in_progress" | "active" => "running",
        "failed" | "blocked" | "error" => "failed",
        _ => "idle",
    }
}

pub(super) fn build_system_step_view(
    step: &V,
    receipts: &V,
    running: bool,
    idx: usize,
) -> SystemStepView {
    let raw_status = shown_status(step, receipts, running);
    let stage_class = step_stage_class(raw_status.trim()).to_owned();
    let row_state = step_row_state(raw_status.trim()).to_owned();
    let status_label = if raw_status == "reported" {
        "reported complete; resume to record it".to_owned()
    } else if raw_status.trim().is_empty() {
        "planned".to_owned()
    } else {
        m::label(raw_status.trim())
    };
    let raw_id = m::text(step, "id");
    let id = if raw_id.trim().is_empty() {
        format!("step-{}", idx + 1)
    } else {
        raw_id.trim().to_owned()
    };
    let service = m::text(step, "service").trim().to_owned();
    let mode = m::label(m::text(step, "mode").trim());
    let attempt = step["attempt"]
        .as_u64()
        .filter(|&n| n > 0)
        .map(|n| format!("attempt {n}"));
    let goal = m::text(step, "goal").trim().to_owned();
    let deps = m::array(&step["depends_on"])
        .into_iter()
        .filter_map(|dep| {
            let s = dep
                .as_str()
                .map(str::trim)
                .map(str::to_owned)
                .unwrap_or_else(|| m::summary(&dep));
            (!s.is_empty()).then_some(s)
        })
        .collect::<Vec<_>>();
    let depends_on = (!deps.is_empty()).then(|| format!("Depends on: {}", deps.join(", ")));
    let task_ref = m::text(step, "task_reference");
    let task_reference = (!task_ref.trim().is_empty()).then(|| task_ref.trim().to_owned());
    SystemStepView {
        key: format!("{idx}:{id}"),
        id,
        stage_class,
        row_state,
        status_label,
        service,
        mode,
        attempt,
        goal,
        depends_on,
        task_reference,
    }
}
/// Why a dispatch receipt did or did not complete, naming what to inspect: the
/// validator's rejection, the agent's own block, the checks that failed (or,
/// for verification, did not run), or the files a read-only worker wrote.
/// The checks a stopped verification's current attempt could not run when
/// nothing else stopped it, as the runner decides them: only those can be
/// accepted on resume.
pub(super) fn unrun_checks(step: &V, receipts: &V) -> Option<Vec<String>> {
    let receipt = m::array(receipts)
        .into_iter()
        .find(|receipt| receipt["phase"] == step["id"] && receipt["attempt"] == step["attempt"])?;
    let report = &receipt["report"];
    let checks = m::array(&report["checks"]);
    let unrun = checks
        .iter()
        .filter(|check| check["status"] == "not_run")
        .map(|check| m::text(check, "name"))
        .collect::<Vec<_>>();
    (step["id"] == "verify"
        && !m::yes(&receipt["ok"])
        && receipt["error"] == "worker_blocked_or_verification_incomplete"
        && receipt["rejection"].is_null()
        && report["status"] == "completed"
        && m::array(&report["changed_files"]).is_empty()
        && checks.iter().any(|check| check["status"] == "passed")
        && checks
            .iter()
            .all(|check| matches!(check["status"].as_str(), Some("passed" | "not_run")))
        && !unrun.is_empty())
    .then_some(unrun)
}
pub(super) fn receipt_outcome(receipt: &V) -> String {
    if m::yes(&receipt["ok"]) {
        // In an investigation a failed check is a finding of a completed report.
        let findings = m::array(&receipt["report"]["checks"])
            .iter()
            .filter(|check| check["status"] == "failed")
            .map(|check| m::text(check, "name"))
            .collect::<Vec<_>>();
        return if findings.is_empty() {
            "completed".into()
        } else {
            format!(
                "completed; failed checks are findings: {}",
                findings.join(", ")
            )
        };
    }
    if let Some(rejection) = receipt["rejection"].as_str() {
        return format!("rejected: {rejection}");
    }
    let report = &receipt["report"];
    if report.is_object() {
        if report["status"] == "blocked" {
            return format!("blocked by the agent: {}", m::text(report, "summary"));
        }
        let checks = m::array(&report["checks"]);
        let named = |status: &str| {
            checks
                .iter()
                .filter(|check| check["status"] == status)
                .map(|check| m::text(check, "name"))
                .collect::<Vec<_>>()
        };
        let failed = named("failed");
        if !failed.is_empty() {
            return format!("failed checks: {}", failed.join(", "));
        }
        if receipt["error"] == "read_only_worker_reported_writes" {
            return format!(
                "read-only worker reported writes: {}",
                joined(&report["changed_files"])
            );
        }
        if receipt["phase"] == "verify" {
            if checks.is_empty() {
                return "verification reported no checks".into();
            }
            let unrun = named("not_run");
            if !unrun.is_empty() {
                return format!("checks not run: {}", unrun.join(", "));
            }
        }
    }
    if let Some(failure) = receipt["failure"].as_str() {
        return format!("the agent CLI failed: {failure}");
    }
    match receipt["error"].as_str() {
        Some(error) => format!("blocked: {}", m::label(error)),
        None => "blocked".into(),
    }
}
/// What to do when a plan could not choose its starting services itself.
pub(super) fn selection_hint(candidates: &[V]) -> String {
    if candidates.is_empty() {
        return "The task matches no declared capability. Choose starting services or changed contracts, then prepare the plan again.".into();
    }
    let listed = candidates
        .iter()
        .map(|candidate| {
            let mut terms = m::array(&candidate["matches"])
                .iter()
                .flat_map(|matched| m::array(&matched["terms"]))
                .map(|term| m::string(&term))
                .collect::<Vec<_>>();
            terms.sort();
            terms.dedup();
            terms.truncate(4);
            format!("{} ({})", m::text(candidate, "service"), terms.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "The task matches {} services, so none was chosen for you: {listed}. They are now ticked as starting services; adjust them and prepare the plan again.",
        candidates.len()
    )
}
/// A list of names or paths as running text.
pub(super) fn joined(values: &V) -> String {
    let items = m::array(values).iter().map(m::string).collect::<Vec<_>>();
    if items.is_empty() {
        "none".into()
    } else {
        items.join(", ")
    }
}
/// A catalog or plan warning as a sentence: who it concerns, what is wrong, where.
pub(super) fn warning_text(warning: &V) -> String {
    if let Some(text) = warning.as_str() {
        return text.into();
    }
    let reason = m::text(warning, "reason");
    let meaning = match reason.as_str() {
        "relationships_incomplete" => "dependencies are not confirmed complete",
        "root_not_allowed" => "the service folder is not registered in Harness",
        "manifest_unavailable" => "its ai-service.json passport is missing or unreadable",
        "provider_contract_unavailable" => {
            "a consumed contract's provider has no readable passport"
        }
        "contract_version_mismatch" => "a consumed contract version differs from the provider's",
        "dependency_cycle" => "declared contract dependencies form a cycle",
        "impact_depth_exceeded" => "impact search stopped at the depth limit",
        "source_unavailable_or_ineligible" => "a registered source is missing or not eligible",
        "source_read_limit" => "source reading stopped at its limit; later sources were left out",
        _ => "",
    };
    let service = m::text(warning, "service");
    [
        if service == "__system__" {
            "System".to_owned()
        } else {
            service
        },
        if meaning.is_empty() {
            m::label(&reason)
        } else {
            meaning.to_owned()
        },
        m::text(warning, "path"),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(": ")
}
pub(super) fn metric_list(rows: &[(&str, String, bool)]) -> web_sys::Element {
    let dl = dom::el("dl", "memory-metrics", "");
    for (term, val, is_zero) in rows {
        dom::add(&dl, &dom::el("dt", "", term));
        let dd = dom::el("dd", "", val);
        if *is_zero {
            let _ = dd.set_attribute("data-zero", "true");
        }
        dom::add(&dl, &dd);
    }
    dl
}

pub fn receipt_changed_files(receipt: &V) -> (String, Vec<String>) {
    let phase = m::text(receipt, "phase");
    let mut service = m::text(receipt, "service");
    if service.is_empty() {
        service = m::text(&receipt["evidence"], "service");
    }
    if service.is_empty() {
        service = if let Some(rest) = phase.strip_prefix("service-") {
            rest.to_owned()
        } else {
            "__system__".to_owned()
        };
    }
    let mut seen = BTreeSet::new();
    let mut files = Vec::new();
    for src in [
        &receipt["report"]["changed_files"],
        &receipt["evidence"]["changed_files"],
        &receipt["evidence"]["git_changed"],
        &receipt["evidence"]["ledger_edited"],
    ] {
        for item in m::array(src) {
            let path = item
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| m::text(&item, "path"));
            let trimmed = path.trim();
            if !trimmed.is_empty() && seen.insert(trimmed.to_owned()) {
                files.push(trimmed.to_owned());
            }
        }
    }
    (service, files)
}

pub(super) fn receipt_evidence_summary(evidence: &V) -> Option<(String, bool)> {
    if !evidence.is_object() {
        return None;
    }
    let opened_count = if evidence["opened"].is_array() {
        m::array(&evidence["opened"]).len()
    } else {
        m::array(&evidence["opened_files"]).len()
    };
    let shell_reads_count = m::array(&evidence["shell_reads"]).len();
    let (graph_part, has_graph) = if let Some(asked) = evidence["graph_asked"].as_bool() {
        (
            if asked {
                "graph queried".to_owned()
            } else {
                "graph not queried".to_owned()
            },
            asked,
        )
    } else {
        let queries = evidence["graph_query_count"]
            .as_u64()
            .unwrap_or_else(|| m::array(&evidence["graph_queries"]).len() as u64);
        (format!("{queries} graph queries"), queries > 0)
    };
    let summary = if shell_reads_count > 0 {
        format!("{opened_count} files · {shell_reads_count} shell reads · {graph_part}")
    } else {
        format!("{opened_count} files · {graph_part}")
    };
    Some((
        summary,
        opened_count == 0 && shell_reads_count == 0 && !has_graph,
    ))
}

pub(super) fn run_has_agent_activity(v: &V) -> bool {
    !m::array(&v["launches"]).is_empty()
        || !m::array(&v["evidence"]["dispatches"]).is_empty()
        || !m::array(&v["receipts"]).is_empty()
}

pub(super) fn build_system_localize_payload(
    run_doc: &V,
    service_id: &str,
    texts: &[&str],
    extra_paths: &[String],
    fallback_query: &str,
) -> V {
    let failing_queries = events_card::extract_failure_queries(texts, fallback_query);
    let hint_paths = events_card::extract_hint_paths(texts, extra_paths);
    let mut req = json!({
        "project_id": m::text(run_doc, "project_id"),
        "failing_queries": failing_queries,
        "hint_paths": hint_paths,
        "prefer_full_modal": true,
    });
    let run_id = m::text(run_doc, "id");
    if !run_id.is_empty() {
        req["run_id"] = json!(run_id);
    }
    let config_path = m::text(run_doc, "config_path");
    if !config_path.is_empty() {
        req["config_path"] = json!(config_path);
    }
    let sid = m::text(run_doc, "session_id");
    if !sid.is_empty() {
        req["session_id"] = json!(sid);
    }
    if !service_id.is_empty() && service_id != "__system__" {
        req["service_id"] = json!(service_id);
    }
    req
}

pub(super) fn receipt_localize_payload(run_doc: &V, receipt: &V) -> Option<V> {
    let checks = m::array(&receipt["report"]["checks"]);
    let failed_checks: Vec<&V> = checks.iter().filter(|c| c["status"] == "failed").collect();
    if m::yes(&receipt["ok"]) && failed_checks.is_empty() {
        return None;
    }
    let (svc_id, changed_files) = receipt_changed_files(receipt);
    let mut strings = Vec::new();
    for val in [
        m::text(receipt, "message"),
        m::text(receipt, "error"),
        m::text(&receipt["report"], "summary"),
    ] {
        if !val.is_empty() {
            strings.push(val);
        }
    }
    for chk in &failed_checks {
        for k in ["command", "name", "error", "output", "summary"] {
            let val = m::text(chk, k);
            if !val.is_empty() {
                strings.push(val);
            }
        }
    }
    let text_refs: Vec<&str> = strings.iter().map(String::as_str).collect();
    let fallback = m::text(receipt, "phase");
    Some(build_system_localize_payload(
        run_doc,
        &svc_id,
        &text_refs,
        &changed_files,
        &fallback,
    ))
}

pub(super) fn step_localize_payload(
    run_doc: &V,
    step: &V,
    receipts: &[V],
    row_state: &str,
) -> Option<V> {
    let step_id = m::text(step, "id");
    let matching_failed_receipt = receipts.iter().find(|r| {
        m::text(r, "phase") == step_id
            && (!m::yes(&r["ok"])
                || m::array(&r["report"]["checks"])
                    .iter()
                    .any(|c| c["status"] == "failed"))
    });
    if row_state != "failed" && matching_failed_receipt.is_none() {
        return None;
    }
    if let Some(receipt) = matching_failed_receipt {
        return receipt_localize_payload(run_doc, receipt);
    }
    let svc = step["service"]
        .as_str()
        .filter(|s| !s.is_empty() && *s != "__system__")
        .or_else(|| {
            step["services"]
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(V::as_str)
                .filter(|s| !s.is_empty() && *s != "__system__")
        })
        .unwrap_or("");
    let goal = m::text(step, "goal");
    let err = m::text(step, "error");
    let texts = [err.as_str(), goal.as_str(), step_id.as_str()];
    Some(build_system_localize_payload(
        run_doc,
        svc,
        &texts,
        &[],
        if !goal.is_empty() { &goal } else { &step_id },
    ))
}

pub(super) fn failed_services_in_run(run_doc: &V) -> Vec<String> {
    let mut failed = BTreeSet::new();
    let receipts = m::array(&run_doc["receipts"]);
    let steps = if run_doc["execution"]["steps"].is_array() {
        m::array(&run_doc["execution"]["steps"])
    } else {
        m::array(&run_doc["plan"]["steps"])
    };
    for r in &receipts {
        if receipt_localize_payload(run_doc, r).is_some() {
            let (svc_id, _) = receipt_changed_files(r);
            if !svc_id.is_empty() && svc_id != "__system__" {
                failed.insert(svc_id);
            }
        }
    }
    for step in &steps {
        if step["status"] == "failed" {
            if let Some(svc) = step["service"]
                .as_str()
                .filter(|s| !s.is_empty() && *s != "__system__")
            {
                failed.insert(svc.to_owned());
            }
        }
    }
    failed.into_iter().collect()
}

fn mark_failed_services_on_map(services: &[String]) {
    crate::system_map::mark_failed_services(services);
}

#[cfg(test)]
pub(super) fn render_system_step_actions_html_for_test(localize: &V) -> String {
    let payload = events_card::escape_html(&localize.to_string());
    format!(
        "<div class=\"system-step-actions\"><button type=\"button\" class=\"button compact secondary\" data-action=\"system-localize-failure\" data-payload=\"{payload}\">Localize failure on map</button></div>"
    )
}

#[cfg(test)]
pub(super) fn build_receipt_render_items(
    receipts: &[V],
    steps: &[V],
    overlaps: &[OverlapRow],
) -> Vec<V> {
    build_receipt_render_items_with_run(&V::Null, receipts, steps, overlaps)
}

pub(super) fn build_receipt_render_items_with_run(
    run_doc: &V,
    receipts: &[V],
    steps: &[V],
    overlaps: &[OverlapRow],
) -> Vec<V> {
    let mut seen_keys = BTreeMap::<String, usize>::new();
    let mut items = Vec::with_capacity(receipts.len() + usize::from(!overlaps.is_empty()));
    for receipt in receipts {
        let base = format!(
            "receipt:{}:{}",
            m::text(receipt, "phase"),
            m::string(&receipt["attempt"])
        );
        let count = seen_keys.entry(base.clone()).or_insert(0);
        let key = if *count == 0 {
            base
        } else {
            format!("{base}#{}", *count)
        };
        *count += 1;
        let accepted = steps.iter().any(|step| {
            step["id"] == receipt["phase"]
                && step["attempt"] == receipt["attempt"]
                && step["accepted_unrun_checks"].is_array()
        });
        let localize = receipt_localize_payload(run_doc, receipt);
        items.push(json!({
            "kind": "receipt",
            "key": key,
            "receipt": receipt,
            "accepted": accepted,
            "localize": localize,
        }));
    }
    if !overlaps.is_empty() {
        let overlap_rows_json = overlaps
            .iter()
            .map(|(path, steps_text, open)| {
                json!({
                    "path": path,
                    "steps": steps_text,
                    "open": open.as_ref().map(|(service, file)| json!([service, file])),
                })
            })
            .collect::<Vec<_>>();
        items.push(json!({
            "kind": "overlaps",
            "key": "overlaps",
            "overlaps": overlap_rows_json,
        }));
    }
    items
}

pub(super) fn build_receipt_card_element(item: &V) -> web_sys::Element {
    if item["kind"] == "overlaps" {
        let overlaps = m::array(&item["overlaps"]);
        let card = dom::el(
            "article",
            "system-card system-receipt-card system-overlaps",
            "",
        );
        dom::add(
            &card,
            &dom::el(
                "h5",
                "",
                &format!("Edited by more than one step: {}", overlaps.len()),
            ),
        );
        dom::add(
            &card,
            &dom::el(
                "p",
                "system-note",
                "The steps of a run go one after another, so no two wrote a file at once; a later step may still have changed what an earlier one made.",
            ),
        );
        let list = dom::el("div", "system-receipt-files", "");
        for entry in &overlaps {
            let path = m::text(entry, "path");
            let steps_text = m::text(entry, "steps");
            let row = dom::el("div", "system-receipt-file-row", "");
            dom::add(&row, &dom::el("code", "system-receipt-file-path", &path));
            dom::add(&row, &dom::el("span", "system-note", &steps_text));
            if let Some(pair) = entry["open"].as_array() {
                if let (Some(service), Some(file)) = (
                    pair.first().and_then(V::as_str),
                    pair.get(1).and_then(V::as_str),
                ) {
                    let map_btn = dom::button(
                        "Map",
                        "inspect-system-file-graph",
                        json!({ "service": service, "path": file }),
                    );
                    map_btn.set_class_name("button compact");
                    let _ = map_btn.set_attribute(
                        "aria-label",
                        &format!("Show {path} on {service} dependency map"),
                    );
                    dom::add(&row, &map_btn);
                }
            }
            dom::add(&list, &row);
        }
        dom::add(&card, &list);
        return card;
    }

    let receipt = &item["receipt"];
    let accepted = m::yes(&item["accepted"]);
    let outcome = if accepted {
        format!("{}; a person accepted them", receipt_outcome(receipt))
    } else {
        receipt_outcome(receipt)
    };
    let (svc_id, changed_files) = receipt_changed_files(receipt);
    let checks = m::array(&receipt["report"]["checks"]);
    let passed = checks.iter().filter(|c| c["status"] == "passed").count();
    let failed = checks.iter().filter(|c| c["status"] == "failed").count();
    let not_run = checks.iter().filter(|c| c["status"] == "not_run").count();
    let checks_summary = if checks.is_empty() {
        "none".to_owned()
    } else {
        format!("{passed} passed · {failed} failed · {not_run} not run")
    };
    let card = dom::el("article", "system-card system-receipt-card", "");
    dom::add(
        &card,
        &dom::el(
            "h5",
            "",
            &format!(
                "{} · attempt {} · {}",
                m::text(receipt, "phase"),
                m::string(&receipt["attempt"]),
                outcome
            ),
        ),
    );
    let mut metrics = vec![
        ("Service", svc_id.clone(), false),
        ("Outcome", outcome.clone(), !m::yes(&receipt["ok"])),
        ("Checks", checks_summary, checks.is_empty()),
        (
            "Changed files",
            changed_files.len().to_string(),
            changed_files.is_empty(),
        ),
    ];
    if let Some((ev_summary, ev_zero)) = receipt_evidence_summary(&receipt["evidence"]) {
        metrics.push(("Opened / Graph", ev_summary, ev_zero));
    }
    dom::add(&card, &metric_list(&metrics));

    if !changed_files.is_empty() {
        let file_list = dom::el("div", "system-receipt-files", "");
        for path in &changed_files {
            let row = dom::el("div", "system-receipt-file-row", "");
            dom::add(&row, &dom::el("code", "system-receipt-file-path", path));
            let map_btn = dom::button(
                "Map",
                "inspect-system-file-graph",
                json!({ "service": svc_id, "path": path }),
            );
            map_btn.set_class_name("button compact");
            let _ = map_btn.set_attribute(
                "aria-label",
                &format!("Show {path} on {svc_id} dependency map"),
            );
            dom::add(&row, &map_btn);
            dom::add(&file_list, &row);
        }
        dom::add(&card, &file_list);
    }

    if item["localize"].is_object() {
        let actions = dom::el("div", "system-step-actions", "");
        let loc_btn = dom::button(
            "Localize failure on map",
            "system-localize-failure",
            item["localize"].clone(),
        );
        loc_btn.set_class_name("button compact secondary");
        dom::add(&actions, &loc_btn);
        dom::add(&card, &actions);
    }

    let raw = serde_json::to_string_pretty(receipt).unwrap_or_default();
    let details = dom::details("Receipt JSON", &raw);
    let _ = details.set_attribute("data-restore-key", "receipt-json");
    if let Some(summary) = details.first_element_child() {
        let _ = summary.set_attribute("data-focus-key", "receipt-json-summary");
    }
    dom::add(&card, &details);
    card
}

pub(super) fn render_run() {
    let v = m::data("system-run");
    if !v.is_object() {
        run::sync_waiting_permissions("system-permissions-list", &[], false);
        dom::hidden("system-permissions-panel", true);
        dom::text("system-permissions-lead", "");
        m::take("system-permissions-owner");
        dom::hidden("system-run", true);
        sync_changes_map_workspace();
        return;
    }
    dom::hidden("system-run", false);
    let perm_owner = json!([v["id"], v["session_id"], v["permissions"]]);
    if m::data("system-permissions-owner") != perm_owner {
        m::set("system-permissions-owner", perm_owner);
        dom::set_value("system-permissions", restored_system_launch_permission(&v));
        dom::set_value(
            "system-recovery-permissions",
            restored_system_recovery_permission(&v),
        );
    }
    ingest_run_graph_focus(&v);
    crate::activity::set_run_evidence(&m::text(&v, "id"), &v);
    stepper(&v);
    // The list entry follows the run's status as it changes.
    if let Some(option) = dom::get("system-runs")
        .and_then(|list| list.dyn_into::<web_sys::HtmlSelectElement>().ok())
        .and_then(|list| {
            (0..list.length())
                .filter_map(|index| list.item(index))
                .find(|option| option.get_attribute("value").as_deref() == Some(&m::text(&v, "id")))
        })
    {
        option.set_text_content(Some(&run_label(&v)));
    }
    dom::text(
        "system-run-title",
        &format!("{} · {}", m::text(&v, "change_id"), m::text(&v, "system")),
    );
    dom::text("system-run-status", &run_status(&v));
    dom::hidden(
        "system-selection-note",
        v["plan"]["status"] != "needs_selection",
    );
    let plan_sig = dom::json_signature(&v["plan"]);
    if let Some(plan_el) = dom::get("system-plan-json") {
        if plan_el.get_attribute("data-signature").as_deref() != Some(plan_sig.as_str()) {
            let _ = plan_el.set_attribute("data-signature", &plan_sig);
            dom::pretty("system-plan-json", &v["plan"]);
        }
    }
    let participants = joined(&v["plan"]["context"]["services"]);
    let shared_mem = shared_memory_summary(&v["plan"]);
    let mut summary_html = String::new();
    for (term, value, is_zero) in [
        ("Participants", participants.clone(), participants == "none"),
        ("Context", context_summary(&v["plan"]), false),
        ("Shared memory", shared_mem.clone(), shared_mem == "none"),
        ("System file", m::text(&v, "config_path"), false),
    ] {
        let zero_attr = if is_zero { " data-zero=\"true\"" } else { "" };
        summary_html.push_str(&format!(
            "<dt>{}</dt><dd{zero_attr}>{}</dd>",
            dom::escape_html(term),
            dom::escape_html(&value),
        ));
    }
    dom::set_markup_id("system-plan-summary", &summary_html);
    let steps = if v["execution"]["steps"].is_array() {
        m::array(&v["execution"]["steps"])
    } else {
        m::array(&v["plan"]["steps"])
    };
    let running = m::yes(&v["active"]);
    let receipts_for_steps = m::array(&v["receipts"]);
    mark_failed_services_on_map(&failed_services_in_run(&v));
    let step_entries = steps
        .iter()
        .enumerate()
        .map(|(idx, step)| {
            let sv = build_system_step_view(step, &v["receipts"], running, idx);
            let localize = step_localize_payload(&v, step, &receipts_for_steps, &sv.row_state);
            json!({"idx": idx, "step": step, "localize": localize})
        })
        .collect::<Vec<_>>();
    dom::keyed_render_or_empty(
        "system-flow",
        &step_entries,
        |entry| {
            let idx = entry["idx"].as_u64().unwrap_or(0) as usize;
            build_system_step_view(&entry["step"], &v["receipts"], running, idx).key
        },
        |entry| {
            let idx = entry["idx"].as_u64().unwrap_or(0) as usize;
            let view = build_system_step_view(&entry["step"], &v["receipts"], running, idx);
            let item = dom::el("li", &view.stage_class, "");
            dom::add(&item, &dom::el("strong", "", &view.id));
            let body = dom::el("div", "system-stage-body", "");
            let row = dom::el("div", "step-row", "");
            let _ = row.set_attribute("data-state", &view.row_state);
            if !view.service.is_empty() {
                dom::add(&row, &dom::el("span", "system-chip", &view.service));
            }
            dom::add(&row, &dom::el("span", "step-state", &view.status_label));
            if !view.mode.is_empty() {
                dom::add(&row, &dom::el("span", "system-badge", &view.mode));
            }
            if let Some(attempt) = &view.attempt {
                dom::add(&row, &dom::el("span", "system-badge", attempt));
            }
            if let Some(task_ref) = &view.task_reference {
                dom::add(&row, &dom::el("span", "system-chip", task_ref));
            }
            dom::add(&body, &row);
            if !view.goal.is_empty() {
                dom::add(&body, &dom::el("p", "system-stage-goal", &view.goal));
            }
            if let Some(deps) = &view.depends_on {
                dom::add(&body, &dom::el("p", "system-note", deps));
            }
            if entry["localize"].is_object() {
                let actions = dom::el("div", "system-step-actions", "");
                let btn = dom::button(
                    "Localize failure on map",
                    "system-localize-failure",
                    entry["localize"].clone(),
                );
                btn.set_class_name("button compact secondary");
                dom::add(&actions, &btn);
                dom::add(&body, &actions);
            }
            dom::add(&item, &body);
            item
        },
        "li",
        "system-stage",
        "No planned steps yet.",
    );
    dom::options(
        "system-provider",
        &m::array(&v["providers"]),
        m::summary,
        Some(
            v["provider"]
                .as_str()
                .unwrap_or(&dom::value("system-provider")),
        ),
    );
    if !m::yes(&m::data("system-graph-prompt-touched")) {
        if let Some(tell) = v["tell_agents_graph"].as_bool() {
            dom::set_checked("system-graph-prompt", tell);
        }
    }
    let mut retry =
        vec![json!({"id":"","label":"Reconcile saved receipt / continue pending dispatch"})];
    let unfinished = steps
        .iter()
        .filter(|s| {
            matches!(
                s["status"].as_str(),
                Some("running" | "interrupted" | "blocked")
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    retry.extend(unfinished.iter().map(|s| {
        let status = shown_status(s, &v["receipts"], running);
        json!({"id":s["id"],"label":format!("Retry {} · {}",m::text(s,"id"),if status=="reported"{"reported complete".to_owned()}else{m::label(&status)})})
    }));
    // Only a successful receipt can be reconciled; without one the dispatch
    // must be retried, so that is what recovery offers first.
    let reconcilable = unfinished.first().is_some_and(|step| {
        m::array(&v["receipts"]).iter().any(|receipt| {
            receipt["phase"] == step["id"]
                && receipt["attempt"] == step["attempt"]
                && m::yes(&receipt["ok"])
        })
    });
    let preselected = unfinished
        .first()
        .filter(|_| !reconcilable)
        .map(|step| m::text(step, "id"));
    dom::options("system-retry", &retry, m::summary, preselected.as_deref());
    let unrun = unfinished
        .first()
        .and_then(|step| unrun_checks(step, &v["receipts"]));
    dom::hidden("system-accept-unrun-field", unrun.is_none());
    if let Some(names) = &unrun {
        dom::text(
            "system-accept-unrun-text",
            &format!(
                "I accept the checks verification could not run ({}); resuming completes it without running it again",
                names.join(", ")
            ),
        );
    } else if let Some(input) = dom::get("system-accept-unrun")
        .and_then(|input| input.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        input.set_checked(false);
    }

    let receipts = m::array(&v["receipts"]);
    let overlaps = overlap_rows(&v["evidence"]);
    let receipt_items = build_receipt_render_items_with_run(&v, &receipts, &steps, &overlaps);
    dom::keyed_render(
        "system-receipts",
        &receipt_items,
        |item| m::text(item, "key"),
        build_receipt_card_element,
    );

    dom::pretty("system-native-tasks", &v["execution"]["tasks"]);
    dom::hidden(
        "system-native-details",
        !v["execution"]["tasks"].is_object(),
    );
    let handoff_text = if v["handoff"].is_null() {
        String::new()
    } else {
        format!(
            "Handoff saved: {}/handoff.json\n{}{}",
            m::text(&v["execution"], "run_dir"),
            m::text(&v["handoff"], "knowledge"),
            handoff_memory(&v["handoff"])
        )
    };
    dom::text("system-handoff", &handoff_text);
    dom::hidden(
        "system-results",
        receipts.is_empty() && handoff_text.is_empty() && !v["execution"]["tasks"].is_object(),
    );
    let run_events: Vec<V> = m::array(&v["events"])
        .into_iter()
        .filter(|ev| ev["kind"] != "user")
        .collect();
    events_card::render_structured_events("system-events", &run_events);
    dom::hidden("system-events-log", run_events.is_empty());
    dom::hidden("system-open-session", !v["session_id"].is_string());
    let waiting = m::array(&v["waiting_permissions"]);
    let (show_waiting, lead) = system_permissions_panel_state(&v);
    run::sync_waiting_permissions("system-permissions-list", &waiting, running);
    dom::hidden("system-permissions-panel", !show_waiting);
    dom::text("system-permissions-lead", &lead);
    sync_changes_map_workspace();
    controls();
}
pub(super) async fn mutate(action: &str, extra: V) {
    if m::yes(&m::data("system-pending")) {
        return;
    }
    let value = m::data("system-run");
    let project = dom::value("system-project");
    let id = m::text(&value, "id");
    if id.is_empty()
        || value["project_id"] != project
        || !system_current(
            &project,
            &id,
            value["view_generation"].as_u64().unwrap_or(0),
        )
    {
        dom::error(
            "system-run-error",
            "Load the current project's change before continuing.",
        );
        return;
    }
    let mut body = json!({"action":action,"revision":value["revision"]});
    if let Some(map) = extra.as_object() {
        for (key, value) in map {
            body[key] = value.clone();
        }
    }
    let generation = m::bind_scope("system-scope", &project, &id);
    m::next("system-read");
    dom::cancel_timer("system-poll");
    m::bind_pending("system-pending", generation);
    controls();
    match api::post(&format!("/api/system-runs/{}", dom::enc(&id)), body).await {
        Ok(value)
            if system_current(&project, &id, generation)
                && value["id"] == id
                && value["project_id"] == project =>
        {
            m::cache_scope_value(
                "system-run",
                "system-scope",
                generation,
                &project,
                &id,
                value.clone(),
            );
            render_run();
            agents(&value).await;
            // A launch is a new session in Sessions.
            if matches!(action, "execute" | "resume") {
                session::refresh().await;
            }
            if m::yes(&value["active"]) {
                let delay = if events_card::sse_is_connected() {
                    15_000
                } else {
                    1100
                };
                schedule_system_poll(&id, &project, generation, delay);
            }
        }
        Err(error) if system_current(&project, &id, generation) => {
            dom::error("system-run-error", &error.message);
            read_scoped(&id, &project, generation).await;
        }
        _ => {}
    }
    if m::finish_pending("system-pending", generation) {
        controls();
    }
}
pub(super) async fn plan() {
    if !sync_editor_target()
        || m::yes(&m::data("editor-dirty"))
        || m::yes(&m::data("discovery-active"))
        || m::yes(&m::data("system-pending"))
        || m::data("system-catalog").is_null()
        || !dom::validity("system-plan-form")
    {
        return;
    }
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    if !m::owns_scope("editor-scope", &project, &config) {
        return;
    }
    let generation = m::read(|s| s.epochs.get("editor-scope").copied().unwrap_or(0));
    let epoch = m::next("system-plan");
    let body = json!({"project_id":project,"config_path":config,"task":dom::value("system-task"),"change_id":dom::value("system-change-id"),"services":dom::selected_checks("system-plan-services"),"contracts":dom::value("system-contracts").split(',').map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>(),"budget":dom::num("system-budget"),"depth":dom::num("system-depth")});
    m::bind_pending("system-pending", epoch);
    controls();
    match api::post("/api/system-runs", body).await {
        Ok(value)
            if editor_current(&project, &config, generation)
                && m::current("system-plan", epoch)
                && value["project_id"] == project =>
        {
            let id = m::text(&value, "id");
            let run_generation = m::bind_scope("system-scope", &project, &id);
            let mut value = value;
            value["view_generation"] = json!(run_generation);
            let needs_selection = value["plan"]["status"] == "needs_selection";
            let candidates = m::array(&value["plan"]["routing"]["candidates"]);
            m::set("system-run", value);
            if needs_selection {
                // The form stays open with the routed candidates ticked, so the
                // next plan starts from what the task matched.
                let wanted = candidates
                    .iter()
                    .map(|candidate| m::text(candidate, "service"))
                    .collect::<Vec<_>>();
                for e in dom::all("#system-plan-services input[type=checkbox]") {
                    if let Some(input) = e.dyn_ref::<web_sys::HtmlInputElement>() {
                        if !input.disabled() {
                            input.set_checked(wanted.contains(&input.value()));
                        }
                    }
                }
                dom::text("system-run-message", &selection_hint(&candidates));
            } else {
                dom::hidden("system-plan-form", true);
            }
            dom::set_checked("system-reviewed", false);
            render_run();
            agents(&m::data("system-run")).await;
            relist(&project, &id).await;
        }
        Err(error)
            if editor_current(&project, &config, generation)
                && m::current("system-plan", epoch) =>
        {
            dom::error("system-run-error", &error.message)
        }
        _ => {}
    }
    if m::finish_pending("system-pending", epoch) {
        controls();
    }
}

pub(super) fn resolve_agent_launch_selection(
    launches: &[V],
    prev_count: u64,
    preferred: &str,
) -> (String, bool) {
    let latest = launches
        .last()
        .map(|l| m::text(l, "session_id"))
        .unwrap_or_default();
    let sid = if prev_count != launches.len() as u64
        || !launches.iter().any(|l| l["session_id"] == preferred)
    {
        latest.clone()
    } else {
        preferred.to_owned()
    };
    let is_current = sid == latest;
    (sid, is_current)
}

pub(super) fn build_agent_launch_options(launches: &[V]) -> Vec<V> {
    launches
        .iter()
        .enumerate()
        .map(|(i, l)| {
            json!({
                "id": l["session_id"],
                "label": format!(
                    "Launch {} · {}",
                    i + 1,
                    m::label(l["status"].as_str().unwrap_or(""))
                )
            })
        })
        .collect()
}

pub(super) fn sync_agent_launch_options(options: &[V], selected: Option<&str>) {
    let Some(select) = dom::get("system-agents-launch") else {
        return;
    };
    let current = dom::children(&select)
        .into_iter()
        .map(|opt| {
            (
                opt.get_attribute("value").unwrap_or_default(),
                opt.text_content().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    let expected = if options.is_empty() {
        vec![(String::new(), "None available".to_owned())]
    } else {
        options
            .iter()
            .map(|o| (m::text(o, "id"), m::summary(o)))
            .collect::<Vec<_>>()
    };
    if current == expected {
        if let Some(sid) = selected {
            if dom::value_of(&select) != sid {
                dom::set_element_value(&select, sid);
            }
        }
        return;
    }
    dom::options("system-agents-launch", options, m::summary, selected);
}

pub(super) async fn agents(v: &V) {
    let launches = m::array(&v["launches"]);
    let show_agents = run_has_agent_activity(v);
    dom::hidden("system-agents-section", !show_agents);
    dom::hidden("system-agents-live", !m::yes(&v["active"]));
    if !show_agents {
        crate::activity::reset("system-agents");
        return;
    }
    let (sid, current) = if launches.is_empty() {
        m::set("agent-launch-count", json!(0));
        m::set("system-agent-launch", json!(""));
        sync_agent_launch_options(&[], None);
        dom::hidden("system-agents-launch-field", true);
        (String::new(), true)
    } else {
        let count = m::data("agent-launch-count").as_u64().unwrap_or(0);
        let preferred = m::data("system-agent-launch")
            .as_str()
            .map(str::to_owned)
            .unwrap_or_default();
        let (sid, is_current) = resolve_agent_launch_selection(&launches, count, &preferred);
        m::set("agent-launch-count", json!(launches.len()));
        m::set("system-agent-launch", json!(sid.clone()));
        let options = build_agent_launch_options(&launches);
        sync_agent_launch_options(&options, Some(&sid));
        dom::hidden("system-agents-launch-field", launches.len() < 2);
        (sid, is_current)
    };
    let mut steps = m::array(&v["execution"]["steps"]);
    if steps.is_empty() {
        steps.push(json!({"id":"contracts","service":"__system__","mode":"read-only"}));
        for service in m::array(&v["plan"]["context"]["services"]) {
            steps.push(json!({"id":format!("service-{}",m::string(&service)),"service":service,"mode":v["mode"]}));
        }
        steps.push(json!({"id":"verify","service":"__system__","mode":"read-only"}));
    }
    let active = current && m::yes(&v["active"]);
    for step in &mut steps {
        let id = m::text(step, "id");
        let service = m::text(step, "service");
        let (label, goal) = match id.as_str() {
            "contracts" => (
                "Contracts agent".into(),
                "Reads contracts and shared context before dispatch.".into(),
            ),
            "verify" => (
                "Verification agent".into(),
                "Checks service receipts and cross-service contracts.".into(),
            ),
            _ => (
                format!("{service} agent"),
                service_agent_goal(&service, &v["access"], &step["mode"]),
            ),
        };
        step["label"] = json!(label);
        step["goal"] = json!(goal);
        step["journal"] = if current && step["status"].is_string() {
            json!(shown_status(step, &v["receipts"], active))
        } else {
            V::Null
        };
    }
    crate::activity::set_plan("system-agents", steps, active);
    crate::activity::set_run_evidence(&m::text(v, "id"), v);
    crate::activity::load("system-agents", &sid).await;
}
pub(super) fn context_summary(plan: &V) -> String {
    format!(
        "{} of {} characters · {} sources · {} omitted",
        m::string(&plan["context_chars"]),
        m::string(&plan["context_budget_chars"]),
        m::array(&plan["context"]["sources"]).len(),
        m::array(&plan["omitted_sources"]).len()
    )
}
/// How much memory the plan gives every agent: entries of the system's and
/// the services' Memory Banks, those no person reviewed, and those it found
/// but could not share.
pub(super) fn shared_memory_summary(plan: &V) -> String {
    let memory = m::array(&plan["context"]["sources"])
        .into_iter()
        .filter(|source| source["kind"] == "memory")
        .collect::<Vec<_>>();
    let automatic = memory
        .iter()
        .filter(|source| source["trust"] == "automatic_memory")
        .count();
    let unshared = m::array(&plan["warnings"])
        .iter()
        .filter(|warning| warning["reason"] == "memory_not_shared")
        .map(|warning| warning["count"].as_u64().unwrap_or(0))
        .sum::<u64>();
    let mut text = match memory.len() {
        0 => "none".to_string(),
        1 => "1 entry".to_string(),
        count => format!("{count} entries"),
    };
    if automatic > 0 {
        text.push_str(&format!(" · {automatic} not reviewed by a person"));
    }
    if unshared > 0 {
        text.push_str(&format!(" · {unshared} not shared"));
    }
    text
}
/// The line a run's handoff adds about the system memory its outcome became:
/// shared by itself, held for a person where the system folder turns
/// automatic promotion off, or why it is not memory yet.
pub(super) fn handoff_memory(handoff: &V) -> String {
    let memory = &handoff["memory"];
    let title = m::text(memory, "title");
    if memory["status"] == "applied" {
        format!(
            "\nShared as system memory: {title} ({}). Later changes give it to every agent.",
            m::text(memory, "memory_id")
        )
    } else if memory["review_mode"] == "human" {
        format!(
            "\nProposed as system memory: {title} (proposal {}). The system folder turns automatic promotion off: approve it with memory promote-review, then promote-apply.",
            m::text(memory, "proposal_id")
        )
    } else if let Some(note) = memory["note"].as_str() {
        format!("\nThe outcome is not system memory yet: {note}")
    } else if let Some(error) = memory["error"].as_str() {
        format!("\nThe outcome did not become system memory: {error}")
    } else {
        String::new()
    }
}
/// What a service agent's card says before its dispatch starts. Every agent
/// reads the system folder and every service of the change (they share one
/// context); the access scope decides only where a service agent edits. The
/// card used to say "own service folder only" of an agent that reads them all.
pub(super) fn service_agent_goal(service: &str, access: &V, mode: &V) -> String {
    let reads = "reads the system folder and every service of this change";
    if mode != "edit" {
        format!("Investigates {service}; {reads}.")
    } else if access == "all" {
        format!("Works on {service}; {reads} and edits any selected service.")
    } else {
        format!("Works on {service}; {reads} and edits only its own folder.")
    }
}
pub(super) async fn discovery_agents(job: &V) {
    dom::hidden("system-discovery-agents", false);
    dom::hidden("system-discovery-live", !m::yes(&job["active"]));
    let journal = if m::yes(&job["active"]) {
        V::Null
    } else {
        json!(match job["status"].as_str().unwrap_or("") {
            "completed" | "stale" => "completed",
            "failed" => "blocked",
            "cancelled" | "interrupted" => "interrupted",
            _ => "pending",
        })
    };
    crate::activity::set_plan(
        "system-discovery-agent",
        vec![
            json!({"id":"discovery","service":"__system__","mode":"read-only","label":"Discovery agent","goal":"Reads captured evidence and fills the service forms.","journal":journal}),
        ],
        m::yes(&job["active"]),
    );
    crate::activity::load("system-discovery-agent", &m::text(job, "id")).await;
}

/// A file more than one step of a run edited: the file, the steps in the
/// order they ran, and, for a file of a member service, that service and
/// the path it opens at on its code map.
pub type OverlapRow = (String, String, Option<(String, String)>);
/// The files more than one step of a run edited.
pub fn overlap_rows(evidence: &V) -> Vec<OverlapRow> {
    m::array(&evidence["overlaps"])
        .iter()
        .map(|row| {
            let steps = m::array(&row["steps"])
                .iter()
                .filter_map(|step| step.as_str())
                .collect::<Vec<_>>()
                .join(" → ");
            let open = match (row["service"].as_str(), row["file"].as_str()) {
                (Some(service), Some(file)) if service != "__system__" => {
                    Some((service.to_owned(), file.to_owned()))
                }
                _ => None,
            };
            (m::text(row, "path"), steps, open)
        })
        .collect()
}

pub(super) fn render_system_events_only() {
    let v = m::data("system-run");
    let run_events: Vec<V> = m::array(&v["events"])
        .into_iter()
        .filter(|ev| ev["kind"] != "user")
        .collect();
    events_card::render_structured_events("system-events", &run_events);
    dom::hidden("system-events-log", run_events.is_empty());
}
