use super::*;

pub(crate) fn permissions_hint_mode<'a>(
    read_only: bool,
    editing: &str,
    permission: &'a str,
) -> &'a str {
    if read_only || editing != "edit" {
        "plan"
    } else {
        permission
    }
}

pub(crate) fn waiting_banner_text(status: &str, permission_waiting: bool) -> &'static str {
    if status == "queued" {
        "Queued — waiting for an available runner."
    } else if permission_waiting {
        "Waiting for your answer to a permission request…"
    } else {
        "Agent is running."
    }
}

pub(super) fn permission_display_label(permission: &str) -> &'static str {
    match permission {
        "manual" => "Manual",
        "bypass" => "Bypass Permissions",
        _ => "Auto",
    }
}

pub(super) fn mode_display_label(mode: &str) -> &'static str {
    match mode {
        "edit" => "Edit",
        _ => "Plan",
    }
}

pub(crate) fn format_session_summary(
    provider: &str,
    mode: &str,
    permissions: &str,
    model: &str,
    routing_enabled: bool,
) -> String {
    let provider = if provider.trim().is_empty() {
        "Provider"
    } else {
        provider.trim()
    };
    let model_text = if routing_enabled {
        "Routing (Plan + Edit)"
    } else if model.trim().is_empty() {
        "Provider default"
    } else {
        model.trim()
    };
    format!(
        "{provider} · {} · {} · {model_text}",
        mode_display_label(mode),
        permission_display_label(permissions),
    )
}

pub(crate) struct WorkingSignals<'a> {
    pub(crate) selected: &'a V,
    pub(crate) sessions: &'a [V],
    pub(crate) pending: bool,
    pub(crate) booting: bool,
    pub(crate) update_page: &'a V,
    pub(crate) results: &'a V,
    pub(crate) delivery_preview: &'a V,
    pub(crate) creator_pending: bool,
    pub(crate) creator_run: &'a V,
    pub(crate) system_pending: bool,
    pub(crate) editor_pending: bool,
    pub(crate) discovery_active: bool,
    pub(crate) system_run_active: bool,
    pub(crate) graph_building: bool,
    pub(crate) knowledge_pending: bool,
    pub(crate) skills_pending: bool,
    pub(crate) setup_pending: bool,
    pub(crate) create_skill_pending: bool,
    pub(crate) sign_in_pending: bool,
    pub(crate) sdd_pending: bool,
    /// A permission request of the selected session waits for the person.
    pub(crate) permission_waiting: bool,
}

pub(crate) fn working_message(signals: &WorkingSignals<'_>) -> Option<&'static str> {
    if m::active(signals.selected) {
        return Some(if signals.selected["status"] == "queued" {
            "Queued — waiting for an available runner…"
        } else if signals.permission_waiting {
            "Waiting for your answer to a permission request…"
        } else {
            "Agent is working on your task…"
        });
    }
    if signals.pending {
        return Some("Working on your request…");
    }
    if m::yes(&signals.update_page["restarting"]) {
        return Some("Restarting Harness…");
    }
    if m::yes(&signals.update_page["pending"]) {
        return Some("Updating Harness…");
    }
    if m::array(&signals.results["checks"]).iter().any(m::active)
        || m::active(&signals.delivery_preview["check"])
    {
        return Some("Running verification checks…");
    }
    if signals.creator_pending
        || matches!(
            signals.creator_run["status"].as_str(),
            Some("scanning" | "generating" | "applying" | "rolling_back")
        )
    {
        return Some("Generator is running…");
    }
    if signals.discovery_active {
        return Some("Scanning project services…");
    }
    if signals.system_pending || signals.editor_pending || signals.system_run_active {
        return Some("System orchestration is running…");
    }
    if signals.graph_building {
        return Some("Building dependency map…");
    }
    if signals.knowledge_pending
        || signals.skills_pending
        || signals.setup_pending
        || signals.create_skill_pending
        || signals.sign_in_pending
        || signals.sdd_pending
    {
        return Some("Working in background…");
    }
    if signals.booting {
        return Some("Loading workspace…");
    }
    if signals
        .sessions
        .iter()
        .any(|s| s["waiting_permissions"].as_u64().unwrap_or(0) > 0)
    {
        return Some("Another session is waiting for approval…");
    }
    if signals.sessions.iter().any(m::active) {
        return Some("Another session is running…");
    }
    None
}

/// Synchronize the animated prompt-area working companion with active app work.
pub fn sync_working_indicator() {
    let update_page = m::data("app-update-page");
    let results = m::data("results");
    let delivery_preview = m::data("delivery-preview");
    let creator_detail = m::data("creator-detail");
    let system_run = m::data("system-run");
    let graph_data = m::data("graph");
    let system_graph_status = m::data("system-graph-status");
    let sys_graph_building = system_graph_status["building_count"].as_u64().unwrap_or(0) > 0
        || system_graph_status["state"] == "building";
    let creator_pending = m::yes(&m::data("creator-pending"));
    let system_pending = m::yes(&m::data("system-pending"));
    let editor_pending = m::yes(&m::data("editor-pending"));
    let discovery_active = m::yes(&m::data("discovery-active"));
    let knowledge_pending =
        m::yes(&m::data("knowledge-pending")) || m::yes(&m::data("omnigraph-pending"));
    let skills_pending = m::yes(&m::data("skills-pending"));
    let setup_pending = m::yes(&m::data("setup-pending"));
    let create_skill_pending = m::yes(&m::data("create-skill-pending"));
    let sign_in_pending = m::yes(&m::data("sign-in-pending"));
    let sdd_pending = m::yes(&m::data("sdd-documents-pending"));
    let status = m::read(|s| {
        let signals = WorkingSignals {
            selected: &s.selected,
            sessions: &s.sessions,
            pending: s.pending,
            booting: s.boot.is_null() && !s.auth_failed,
            update_page: &update_page,
            results: &results,
            delivery_preview: &delivery_preview,
            creator_pending,
            creator_run: &creator_detail["run"],
            system_pending,
            editor_pending,
            discovery_active,
            system_run_active: m::yes(&system_run["active"]),
            graph_building: graph_data["build"].is_object() || sys_graph_building,
            knowledge_pending,
            skills_pending,
            setup_pending,
            create_skill_pending,
            sign_in_pending,
            sdd_pending,
            permission_waiting: crate::run::permission_waiting(),
        };
        working_message(&signals)
    });
    let active = if status.is_some() { "true" } else { "false" };
    let hidden_attr = if status.is_some() { "false" } else { "true" };
    for (box_id, text_id) in [
        ("composer-working", "composer-working-text"),
        ("system-working", "system-working-text"),
        ("system-changes-working", "system-changes-working-text"),
    ] {
        dom::attr(box_id, "data-working", active);
        dom::attr(box_id, "aria-hidden", hidden_attr);
        if let Some(message) = status {
            dom::text(text_id, message);
        }
    }
    dom::attr("session-form", "data-working", active);
}

pub fn new_session(project: Option<&str>) {
    if m::read(|s| s.pending) {
        return;
    }
    reset_preflight(true);
    reset_results();
    save_preferences();
    reset_merge_archive();
    m::take("merge-draft");
    stop_poll();
    m::write(|s| {
        s.epoch += 1;
        s.sid.clear();
        s.selected = V::Null;
        s.events.clear();
        s.seen.clear();
        s.cursor = 0;
    });
    reset_session_settings_state();
    m::take("session-settings-owner");
    m::take("merge-draft");
    crate::graph_map::apply_focus(&V::Null);
    crate::graph::run_state(false);
    omnigraph::clear_draft_peers();
    omnigraph::render_session_bar(&V::Null);
    clear_files();
    dom::clear("events");
    dom::set_value("prompt", "");
    if let Some(p) = project {
        sync_project(p);
    }
    crate::graph_map::sync_embedded_changes();
    restore_draft(&dom::value("project"));
    let selected = preferences_selection(dom::json_storage(PREFS), &dom::value("project"), "");
    dom::save(PREFS, Some(&selected.to_string()));
    history();
    controls();
    dom::focus("prompt");
    run::sync_approvals_badge();
    run::render_permission_rules_panel();
    sync_hash_from_state(false);
    spawn_local(async {
        knowledge::load_brain_link().await;
    });
}
pub async fn select(id: &str) {
    if m::read(|s| s.pending) {
        return;
    }
    let before_project = dom::value("project");
    reset_preflight(false);
    reset_results();
    save_preferences();
    stop_poll();
    m::write(|s| {
        s.epoch += 1;
        s.sid = id.into();
        s.selected = V::Null;
        s.events.clear();
        s.seen.clear();
        s.cursor = 0;
        s.stream.has_older = false;
        s.stream.oldest_event_id = None;
        s.stream.latest_event_id = None;
        s.stream.visible_limit = 120;
    });
    reset_session_settings_state();
    m::take("session-settings-owner");
    dom::hidden("session-settings", true);
    dom::attr("session-settings-toggle", "aria-expanded", "false");
    clear_files();
    dom::clear("events");
    dom::set_value("prompt", "");
    inbox::restore(id);
    run::reset();
    if !m::yes(&m::data("bootstrapping")) {
        spawn_local(async {
            open_view("sessions", true).await;
        });
    }
    let epoch = m::read(|s| s.epoch);
    poll(epoch).await;
    if !m::read(|s| m::selection_matches(&s.sid, s.epoch, id, epoch)) {
        return;
    }
    let project = m::read(|s| m::text(&s.selected, "project_id"));
    if !project.is_empty() {
        sync_project(&project);
        if project != before_project {
            spawn_local(async {
                load_git().await;
            });
        } else {
            crate::graph_map::sync_embedded_changes();
        }
    }
    history();
    save_preferences();
    controls();
    sync_hash_from_state(false);
    let sid_owned = id.to_owned();
    spawn_local(async move {
        run::load_permission_rules(&sid_owned).await;
    });
    let right_tab = m::read(|s| s.workspace.right_tab.clone());
    if matches!(right_tab.as_str(), "diff" | "checks") {
        spawn_local(async {
            results().await;
        });
    }
}
pub fn stop_poll() {
    dom::cancel_timer("session-poll");
    m::next("session-poll");
    inbox::stop();
    dom::error("inbox-error", "");
}
#[derive(Debug, Clone, PartialEq)]
pub(super) enum FocusUpdate {
    Apply(V),
    Clear,
    Keep,
}

pub(super) fn event_focus_payload(event: &V) -> Option<V> {
    if event["kind"] != "graph_focus" {
        return None;
    }
    for key in ["graph_focus", "focus", "data"] {
        if event[key].is_object() {
            return Some(event[key].clone());
        }
    }
    if event.is_object()
        && (event["phase"].is_string()
            || event["summary"].is_string()
            || event["target_folders"].is_array()
            || event["target_paths"].is_array()
            || event["target_nodes"].is_array()
            || event["created_entities"].is_array())
    {
        return Some(event.clone());
    }
    None
}

pub(super) fn should_keep_predicting(current_focus: &V, incoming: &V) -> bool {
    m::text(current_focus, "phase") == "predicting"
        && m::text(incoming, "phase") == "idle"
        && m::array(&incoming["created_entities"]).is_empty()
}

pub(super) fn select_poll_focus(
    session: &V,
    fresh_events: &[V],
    initial_poll: bool,
    current_focus: &V,
) -> FocusUpdate {
    let session_focus = &session["graph_focus"];
    if let Some(event_focus) = fresh_events.iter().rev().find_map(event_focus_payload) {
        if session_focus.is_object() && m::text(session_focus, "phase") != "idle" {
            return FocusUpdate::Apply(session_focus.clone());
        }
        if should_keep_predicting(current_focus, &event_focus) {
            return FocusUpdate::Keep;
        }
        return FocusUpdate::Apply(event_focus);
    }
    if session_focus.is_object() {
        if should_keep_predicting(current_focus, session_focus) {
            return FocusUpdate::Keep;
        }
        return FocusUpdate::Apply(session_focus.clone());
    }
    if initial_poll && !m::active(session) {
        return FocusUpdate::Clear;
    }
    FocusUpdate::Keep
}

pub async fn poll(epoch: u64) {
    let (sid, cursor) = m::read(|s| (s.sid.clone(), s.cursor));
    if sid.is_empty() || epoch != m::read(|s| s.epoch) {
        return;
    }
    let was_active = m::read(|s| s.selected["id"] == sid && m::active(&s.selected));
    let initial_poll = cursor == 0;
    let path = if initial_poll {
        format!("/api/sessions/{}?tail=1&limit=250", dom::enc(&sid))
    } else {
        format!("/api/sessions/{}?after={cursor}&limit=250", dom::enc(&sid))
    };
    match api::scoped("session-poll", &path).await {
        Ok((e, data)) if m::current("session-poll", e) && epoch == m::read(|s| s.epoch) => {
            if data["session"]["id"] != sid {
                dom::error(
                    "events-error",
                    "The runner returned a different session. Refresh before retrying.",
                );
                return;
            }
            m::write(|s| s.stream.consecutive_poll_errors = 0);
            let waiting_list = data["waiting_permissions"].as_array().cloned();
            let mut session = data["session"].clone();
            if let Some(waiting) = &waiting_list {
                if session.is_object() {
                    session["waiting_permissions"] = json!(waiting.len());
                }
            }
            m::upsert(session.clone());
            apply_settings(&session);
            let events = m::array(&data["events"]);
            let length = events.len();
            let mut fresh_events = Vec::new();
            m::write(|s| {
                if let Some(has_older) = data["has_older"].as_bool() {
                    if initial_poll {
                        s.stream.has_older = has_older;
                    }
                }
                if initial_poll {
                    if let Some(oldest) = data["oldest_event_id"].as_u64() {
                        s.stream.oldest_event_id = Some(oldest);
                    } else if let Some(first_id) = events.first().and_then(|ev| {
                        ev["id"]
                            .as_u64()
                            .or_else(|| ev["id"].as_str()?.parse().ok())
                    }) {
                        s.stream.oldest_event_id = Some(first_id);
                    }
                }
                if let Some(latest) = data["latest_event_id"].as_u64() {
                    s.stream.latest_event_id = Some(latest);
                }
                if let Some(total) = data["total_events"].as_u64() {
                    s.stream.total_events = total as usize;
                }
            });
            for event in events {
                let key = m::string(&event["id"]);
                let fresh = m::write(|s| {
                    if !s.seen.insert(key) {
                        false
                    } else {
                        s.cursor = s.cursor.max(
                            event["id"]
                                .as_u64()
                                .or_else(|| event["id"].as_str()?.parse().ok())
                                .unwrap_or(0),
                        );
                        s.events.push(event.clone());
                        true
                    }
                });
                if fresh {
                    fresh_events.push(event);
                }
            }
            if initial_poll && m::read(|s| s.events.len() > s.stream.visible_limit) {
                run::replay_visible_stream();
            } else {
                for event in &fresh_events {
                    run::observe(event.clone());
                }
            }
            run::sync_load_earlier_control();
            if let Some(waiting) = &waiting_list {
                run::reconcile_waiting_permissions(waiting);
            }
            let current_focus = m::data("session-graph-focus");
            let focus_update =
                select_poll_focus(&data["session"], &fresh_events, cursor == 0, &current_focus);
            let focus_changed = match &focus_update {
                FocusUpdate::Apply(focus) => focus != &current_focus,
                FocusUpdate::Clear => !current_focus.is_null(),
                FocusUpdate::Keep => false,
            };
            match focus_update {
                FocusUpdate::Apply(focus) => crate::graph_map::apply_focus(&focus),
                FocusUpdate::Clear => crate::graph_map::apply_focus(&V::Null),
                FocusUpdate::Keep => {}
            }
            inbox::refresh(&sid, epoch).await;
            if !m::current("session-poll", e)
                || !m::read(|s| m::selection_matches(&s.sid, s.epoch, &sid, epoch))
            {
                return;
            }
            history();
            controls();
            run::render();
            run::sync_approvals_badge();
            let is_active = m::read(|s| m::active(&s.selected));
            crate::graph::run_state(is_active);
            if was_active && !is_active {
                spawn_local(async {
                    crate::graph::load(true).await;
                });
                crate::graph_map::sync_embedded(true);
            }
            if should_sync_embedded_changes(&fresh_events, focus_changed, was_active, is_active) {
                crate::graph_map::sync_embedded_changes();
            }
            dom::error("events-error", "");
            if (!initial_poll && length == 250)
                || m::read(|s| m::active(&s.selected))
                || inbox::watching(&sid)
            {
                let delay = if !initial_poll && length == 250 {
                    0
                } else if api::sse_connected() {
                    15000
                } else {
                    1000
                };
                dom::later("session-poll", delay, move || {
                    spawn_local(async move {
                        poll(epoch).await;
                    })
                });
            }
        }
        Ok(_) => {}
        Err(e) if !e.aborted && epoch == m::read(|s| s.epoch) => {
            dom::error("events-error", &e.message);
            if (m::read(|s| m::active(&s.selected)) || inbox::watching(&sid))
                && ![401, 403, 404].contains(&e.status)
            {
                let attempt = m::write(|s| {
                    s.stream.consecutive_poll_errors =
                        s.stream.consecutive_poll_errors.saturating_add(1);
                    s.stream.consecutive_poll_errors
                });
                let delay = match attempt {
                    1 => 2500,
                    2 => 5000,
                    _ => 10000,
                };
                dom::later("session-poll", delay, move || {
                    spawn_local(async move {
                        poll(epoch).await;
                    })
                });
            }
        }
        Err(_) => {}
    }
}

/// Wires `/api/events/stream` SSE event handlers for real-time session events, statuses, approvals, and fallback recovery.
pub(crate) fn initialize_sse_listeners() {
    if m::yes(&m::data("session-sse-wired")) {
        return;
    }
    m::set("session-sse-wired", json!(true));

    api::on_sse("session_event", |payload| {
        let target_sid = m::text(payload, "session_id");
        let (current_sid, was_active) =
            m::read(|s| (s.sid.clone(), !s.sid.is_empty() && m::active(&s.selected)));
        let event = if payload["event"].is_object() {
            payload["event"].clone()
        } else {
            payload.clone()
        };
        if !target_sid.is_empty() && target_sid == current_sid {
            let key = m::string(&event["id"]);
            let fresh = m::write(|s| {
                if !key.is_empty() && !s.seen.insert(key) {
                    false
                } else {
                    let ev_id = payload["id"]
                        .as_u64()
                        .or_else(|| event["id"].as_u64())
                        .or_else(|| event["id"].as_str()?.parse().ok())
                        .unwrap_or(0);
                    s.cursor = s.cursor.max(ev_id);
                    s.events.push(event.clone());
                    true
                }
            });
            if fresh {
                run::observe(event.clone());
                run::render();
                controls();
                if is_mutation_tool_event(&event) {
                    crate::graph_map::sync_embedded_changes();
                }
                if let Some(focus) = event_focus_payload(&event) {
                    crate::graph_map::apply_focus(&focus);
                }
                run::sync_approvals_badge();
            }
        } else if event["kind"] == "permission" {
            spawn_local(async {
                refresh().await;
            });
        }
        let is_active = m::read(|s| !s.sid.is_empty() && m::active(&s.selected));
        if was_active && !is_active {
            crate::graph::run_state(false);
        }
    });

    api::on_sse("session_status", |payload| {
        let session = if payload["session"].is_object() {
            payload["session"].clone()
        } else {
            payload.clone()
        };
        let sid = m::text(&session, "id");
        if sid.is_empty() {
            return;
        }
        let was_active = m::read(|s| s.sid == sid && m::active(&s.selected));
        if let Some(waiting) = session["waiting_permissions"].as_u64() {
            m::write(|s| {
                s.approvals
                    .waiting_counts
                    .insert(sid.clone(), waiting as usize);
            });
        }
        m::upsert(session.clone());
        history();
        run::sync_approvals_badge();
        if m::read(|s| s.sid == sid) {
            apply_settings(&session);
            controls();
            run::render();
            let is_active = m::active(&session);
            crate::graph::run_state(is_active);
            if was_active && !is_active {
                let current_sid = sid.clone();
                spawn_local(async move {
                    let epoch = m::read(|s| s.epoch);
                    if m::read(|s| s.sid == current_sid) {
                        poll(epoch).await;
                    }
                    crate::graph::load(true).await;
                });
                crate::graph_map::sync_embedded(true);
            }
        }
    });

    api::on_sse("waiting_counts", |payload| {
        if let Some(map) = payload["counts"].as_object() {
            let mut next_counts = std::collections::BTreeMap::new();
            for (k, v) in map {
                next_counts.insert(k.clone(), v.as_u64().unwrap_or(0) as usize);
            }
            m::write(|s| {
                for s_item in &mut s.sessions {
                    let id = m::text(s_item, "id");
                    if let Some(&count) = next_counts.get(&id) {
                        s_item["waiting_permissions"] = json!(count);
                    }
                }
                s.approvals.waiting_counts = next_counts;
            });
            history();
            run::sync_approvals_badge();
            if m::read(|s| s.approvals.drawer_open) {
                spawn_local(async {
                    run::refresh_approvals_queue().await;
                });
            }
        }
    });

    api::on_sse("results_check", |payload| {
        let sid = m::text(payload, "session_id");
        let current = m::read(|s| s.sid.clone());
        let relevant_view = result_view()
            || m::read(|s| matches!(s.workspace.right_tab.as_str(), "diff" | "checks"));
        if !current.is_empty() && (sid.is_empty() || sid == current) && relevant_view {
            spawn_local(async {
                results().await;
            });
        }
    });

    api::on_sse("resync", |_| {
        spawn_local(async {
            refresh().await;
            let (sid, epoch) = m::read(|s| (s.sid.clone(), s.epoch));
            if !sid.is_empty() {
                poll(epoch).await;
            }
        });
    });

    api::on_sse("sse_error", |_| {
        let (sid, epoch, active) = m::read(|s| {
            (
                s.sid.clone(),
                s.epoch,
                !s.sid.is_empty() && (m::active(&s.selected) || inbox::watching(&s.sid)),
            )
        });
        if active && !sid.is_empty() {
            dom::later("session-poll", 250, move || {
                spawn_local(async move {
                    poll(epoch).await;
                });
            });
        }
    });
}
