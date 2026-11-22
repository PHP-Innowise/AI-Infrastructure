use super::*;

pub fn normalize_mobile_pane(pane: &str) -> &'static str {
    if pane == "graph" {
        "graph"
    } else {
        "chat"
    }
}

pub fn set_mobile_pane(pane: &str) {
    let pane = normalize_mobile_pane(pane);
    dom::attr("sessions-view", "data-mobile-pane", pane);
    dom::attr("sessions-workspace", "data-mobile-pane", pane);
    let is_graph = pane == "graph";
    dom::attr(
        "session-mobile-tab-chat",
        "aria-selected",
        if is_graph { "false" } else { "true" },
    );
    dom::attr(
        "session-mobile-tab-chat",
        "tabindex",
        if is_graph { "-1" } else { "0" },
    );
    dom::attr(
        "session-mobile-tab-graph",
        "aria-selected",
        if is_graph { "true" } else { "false" },
    );
    dom::attr(
        "session-mobile-tab-graph",
        "tabindex",
        if is_graph { "0" } else { "-1" },
    );
    if is_graph {
        crate::graph_map::sync_embedded(false);
    }
}

pub(crate) fn current_hash_route() -> dom::HashRoute {
    m::read(|s| dom::HashRoute {
        view: if s.view.is_empty() {
            "sessions".into()
        } else {
            s.view.clone()
        },
        project: m::current_project(),
        session: s.sid.clone(),
        run: m::text(&s.data.get("run").cloned().unwrap_or(V::Null), "launch"),
        path: s.diff_view.focused_file.clone(),
    })
}

pub(crate) fn sync_hash_from_state(push: bool) {
    if m::yes(&m::data("bootstrapping")) || m::read(|s| s.boot.is_null()) {
        return;
    }
    dom::update_hash_route(&current_hash_route(), push);
}

pub(crate) async fn apply_hash_route() {
    let route = dom::hash_route();
    if !route.project.is_empty()
        && !m::project(&route.project).is_null()
        && m::current_project() != route.project
    {
        sync_project(&route.project);
    }
    if !route.session.is_empty() && route.session != m::read(|s| s.sid.clone()) {
        select(&route.session).await;
    }
    if !route.path.is_empty() {
        m::write(|s| s.diff_view.focused_file = route.path.clone());
    }
    let target_view = if route.view.is_empty() {
        "sessions"
    } else {
        route.view.as_str()
    };
    open_view_inner(target_view, false, false).await;
    if !route.path.is_empty() {
        if target_view == "sessions" {
            set_mobile_pane("graph");
            set_workspace_right_tab_inner("diff", false);
            results().await;
            crate::run::jump_to_diff_file("workspace-results", &route.path);
        } else if target_view == "changes" {
            crate::run::jump_to_diff_file("results-diff", &route.path);
        }
    }
}

pub(crate) fn set_sidebar_open(open: bool, restore_focus: bool) {
    let was_open = m::read(|s| s.workspace.sidebar_open)
        || dom::get("sidebar").is_some_and(|e| e.class_list().contains("open"));
    if open && !was_open {
        dom::save_focus_for_overlay("sidebar");
    }
    m::write(|s| s.workspace.sidebar_open = open);
    dom::class("sidebar", "open", open);
    dom::class("scrim", "open", open);
    dom::attr(
        "menu-toggle",
        "aria-expanded",
        if open { "true" } else { "false" },
    );
    if open {
        if dom::get("history-search").is_some() {
            dom::focus("history-search");
        } else if let Some(first) = dom::all("#history button").first() {
            if let Some(html) = first.dyn_ref::<web_sys::HtmlElement>() {
                let _ = html.focus();
            }
        }
    } else if was_open && restore_focus {
        dom::restore_focus_for_overlay("sidebar");
    }
}

pub async fn open_view(view: &str, record: bool) {
    open_view_inner(view, record, record).await;
}

pub(super) fn should_reset_results_on_view_change(
    prev_view: &str,
    next_view: &str,
    sid: &str,
) -> bool {
    let was_results = ["changes", "checks", "usage"].contains(&prev_view);
    if !was_results {
        return false;
    }
    if sid.is_empty() {
        return true;
    }
    !["sessions", "changes", "checks", "usage"].contains(&next_view)
}

pub(super) async fn open_view_inner(view: &str, record: bool, focus_heading: bool) {
    flush_preferences();
    let previous_view = m::read(|state| state.view.clone());
    if ["systems", "system-changes"].contains(&previous_view.as_str())
        && !["systems", "system-changes"].contains(&view)
    {
        systems::leave_systems_view();
    }
    if previous_view == "omnigraph" && view != "omnigraph" {
        omnigraph::leave();
    }
    if m::read(|s| s.view == "memory-use" && s.view != view) {
        knowledge::leave_memory();
    }
    let known = [
        "sessions",
        "changes",
        "checks",
        "usage",
        "systems",
        "system-changes",
        "omnigraph",
        "creator",
        "setup",
        "context",
        "memory-use",
        "brain",
        "memory",
        "skills",
        "skill-capsules",
        "create-skill",
        "accelerators",
        "kit3",
    ];
    let view = if known.contains(&view) {
        view
    } else {
        "sessions"
    };
    let (prev_view, sid) = m::read(|s| (s.view.clone(), s.sid.clone()));
    if should_reset_results_on_view_change(&prev_view, view, &sid) {
        reset_results();
    }
    m::write(|s| s.view = view.into());
    let is_results = ["changes", "checks", "usage"].contains(&view);
    for e in dom::all("#main > section.view") {
        dom::hide(
            &e,
            e.id()
                != if is_results {
                    "results-view".into()
                } else {
                    format!("{view}-view")
                },
        );
    }
    if is_results {
        dom::attr("results-view", "data-tab", view);
    }
    sync_hash_from_state(record);
    let group = if is_results {
        "sessions"
    } else if view == "system-changes" {
        "systems"
    } else if ["brain", "memory", "memory-use", "context"].contains(&view) {
        "knowledge"
    } else if ["create-skill", "skill-capsules"].contains(&view) {
        "skills"
    } else if ["creator", "setup", "kit3"].contains(&view) {
        "accelerators"
    } else {
        view
    };
    m::set(&format!("last-view-{group}"), json!(view));
    render_tabs(view, group);
    for e in dom::all(".navigation [data-nav]") {
        if e.get_attribute("data-nav").as_deref() == Some(group) {
            let _ = e.set_attribute("aria-current", "page");
        } else {
            let _ = e.remove_attribute("aria-current");
        }
    }
    dom::text(
        "view-heading",
        &match view {
            "systems" => "Services".to_owned(),
            "system-changes" => "Changes".to_owned(),
            _ => m::label(view),
        },
    );
    // The page title repeats the navigation entry it belongs to.
    let (title, subtitle) = match group {
        "systems" => (
            "System Orchestration",
            "Services, their contracts and changes that span them",
        ),
        "omnigraph" => (
            "Context Graph",
            "Live cross-session knowledge, branches, and 3-way merges",
        ),
        "knowledge" => (
            "Knowledge",
            "Project Brain, Memory Bank, retrieval analytics, and indexed context",
        ),
        "skills" => (
            "Skills",
            "Skill library, SkillCache capsules, and custom skills",
        ),
        "accelerators" => (
            "Accelerators",
            "Starter packs, Infrastructure Creator, and project setup",
        ),
        _ => ("Sessions", "Your local workspace for agent sessions"),
    };
    dom::text("page-title", title);
    dom::text("page-subtitle", subtitle);
    set_sidebar_open(false, !focus_heading);
    if focus_heading {
        dom::focus("view-heading");
    }
    match view {
        "changes" | "checks" | "usage" => results().await,
        "context" => knowledge::load_context().await,
        "memory" | "brain" => knowledge::load(view, false).await,
        "memory-use" => knowledge::load_memory_use().await,
        "setup" => management::load_setup().await,
        "skills" | "create-skill" => management::load_skills().await,
        "skill-capsules" => management::load_capsules().await,
        "creator" => management::creator_list().await,
        "systems" | "system-changes" => {
            systems::adopt_sidebar_project(&previous_view);
            systems::load().await;
            if view == "systems" {
                systems::catalog_autoload().await;
            }
        }
        "omnigraph" => omnigraph::load().await,
        "accelerators" => management::accelerators(),
        "kit3" => {
            if dom::get("kit3-frame")
                .and_then(|e| e.get_attribute("src"))
                .is_none_or(|src| src.is_empty())
            {
                dom::attr("kit3-frame", "src", "/kit3/");
            }
        }
        _ => load_git().await,
    }
}
pub(crate) fn filter_history_sessions(
    sessions: &[V],
    query: &str,
    waiting_only: bool,
    waiting_counts: &std::collections::BTreeMap<String, usize>,
) -> Vec<V> {
    let q = query.trim().to_ascii_lowercase();
    sessions
        .iter()
        .filter_map(|session| {
            let sid = m::text(session, "id");
            let waiting = waiting_counts
                .get(&sid)
                .copied()
                .map(|c| c as u64)
                .unwrap_or_else(|| session["waiting_permissions"].as_u64().unwrap_or(0));
            if waiting_only && waiting == 0 {
                return None;
            }
            if !q.is_empty() {
                let hay = format!(
                    "{} {} {} {}",
                    m::summary(session),
                    sid,
                    m::text(session, "project_id"),
                    m::text(session, "status")
                )
                .to_ascii_lowercase();
                if !hay.contains(&q) {
                    return None;
                }
            }
            let mut copy = session.clone();
            if copy.is_object() {
                copy["waiting_permissions"] = json!(waiting);
            }
            Some(copy)
        })
        .collect()
}

pub(super) fn history_projection(sessions: &[V], sid: &str) -> V {
    V::Array(
        sessions
            .iter()
            .map(|session| {
                let id = m::text(session, "id");
                let title = m::summary(session);
                let status = session["status"].as_str().unwrap_or("").to_owned();
                let waiting_permissions = session["waiting_permissions"].as_u64().unwrap_or(0);
                let status_text = if waiting_permissions > 0 {
                    if waiting_permissions == 1 {
                        "Waiting for approval".to_owned()
                    } else {
                        format!("{waiting_permissions} approvals waiting")
                    }
                } else {
                    m::label(&status)
                };
                json!({
                    "id": id,
                    "title": title,
                    "status": status,
                    "status_text": status_text,
                    "waiting_permissions": waiting_permissions,
                    "label": format!("{title} · {status_text}"),
                    "selected": id == sid,
                })
            })
            .collect(),
    )
}
pub fn history() {
    if dom::get("history").is_none() {
        return;
    }
    let (sessions, sid, query, waiting_only, counts) = m::read(|s| {
        (
            s.sessions.clone(),
            s.sid.clone(),
            s.workspace.history_query.clone(),
            s.workspace.history_waiting_only,
            s.approvals.waiting_counts.clone(),
        )
    });
    let filtered = filter_history_sessions(&sessions, &query, waiting_only, &counts);
    dom::attr(
        "history-filter-waiting",
        "aria-pressed",
        if waiting_only { "true" } else { "false" },
    );
    if !query.trim().is_empty() || waiting_only {
        dom::text(
            "history-filter-status",
            &format!("Showing {} of {}", filtered.len(), sessions.len()),
        );
        dom::hidden("history-filter-status", false);
    } else {
        dom::text("history-filter-status", "");
        dom::hidden("history-filter-status", true);
    }
    let projection = history_projection(&filtered, &sid);
    if m::data("history-projection") == projection {
        return;
    }
    let items = projection.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let empty_msg = if waiting_only {
        "No sessions waiting for approval."
    } else if !query.trim().is_empty() {
        "No sessions match your search."
    } else {
        "No sessions yet."
    };
    dom::keyed_render_or_empty(
        "history",
        items,
        |v| m::text(v, "id"),
        |v| {
            let id = m::text(v, "id");
            let title = m::text(v, "title");
            let status = m::text(v, "status");
            let status_text = m::text(v, "status_text");
            let waiting = v["waiting_permissions"].as_u64().unwrap_or(0);
            let b = dom::button("", "session-open", json!(id));
            b.set_class_name("history-item");
            let _ = b.set_attribute("data-focus-key", &id);
            if waiting > 0 {
                let _ = b.set_attribute("data-waiting-approval", "true");
            }
            if m::yes(&v["selected"]) {
                let _ = b.set_attribute("aria-current", "true");
            }
            dom::add(&b, &dom::el("span", "history-title", &title));
            let meta = dom::el("span", "history-meta", "");
            let dot_class = if waiting > 0 {
                "status-dot awaiting_approval"
            } else {
                match status.as_str() {
                    "running" => "status-dot running",
                    "queued" => "status-dot queued",
                    "failed" => "status-dot failed",
                    "awaiting_approval" => "status-dot awaiting_approval",
                    "rejected" => "status-dot rejected",
                    _ => "status-dot",
                }
            };
            let dot = dom::el("span", dot_class, "");
            let _ = dot.set_attribute("aria-hidden", "true");
            dom::add(&meta, &dot);
            dom::add(&meta, &dom::el("span", "", &status_text));
            dom::add(&b, &meta);
            b
        },
        "p",
        "muted history-empty",
        empty_msg,
    );
    m::set("history-projection", projection);
    crate::run::sync_approvals_badge();
}
pub async fn refresh() {
    match api::get("/api/sessions").await {
        Ok(v) => {
            dom::error("events-error", "");
            m::write(|s| {
                s.sessions = m::array(&v["sessions"]);
                crate::run::sync_waiting_counts_from_sessions(
                    &s.sessions,
                    &mut s.approvals.waiting_counts,
                );
            });
            history();
            crate::run::sync_approvals_badge();
            if m::read(|s| s.approvals.drawer_open) {
                crate::run::refresh_approvals_queue().await;
            }
        }
        Err(e) => dom::error("events-error", &e.message),
    }
}

pub(super) fn merge_source_eligible(session: &V, project: &str) -> bool {
    session["project_id"] == project
        && !m::active(session)
        && !["awaiting_context", "awaiting_approval"]
            .contains(&session["status"].as_str().unwrap_or(""))
        && session["workflow"] != "fleet-review"
        && ![
            "fleet",
            "creator",
            "clash",
            "system_run",
            "system_discovery",
        ]
        .iter()
        .any(|key| session[*key].is_object())
}
pub(super) async fn merge_picker() {
    let p = dom::value("project");
    let epoch = m::read(|s| s.epoch);
    refresh().await;
    if dom::value("project") != p || m::read(|s| s.epoch) != epoch {
        return;
    }
    m::set("merge-picker-owner", json!({"project":p,"epoch":epoch}));
    let sessions = m::read(|s| s.sessions.clone());
    dom::clear("merge-options");
    for v in sessions
        .into_iter()
        .filter(|v| merge_source_eligible(v, &p))
    {
        dom::append(
            "merge-options",
            &dom::check(&m::summary(&v), "merge-choice", &m::text(&v, "id"), false),
        );
    }
    dom::disabled("merge-picker-continue", true);
    dom::text("merge-selection-count", "No chats selected");
    dom::dialog("merge-picker", true);
}

pub(super) fn context_meter(session: &V) {
    let last = &session["context_last"];
    let window = last["window"].as_f64().filter(|n| n.is_finite() && *n > 0.);
    let fill = last["fill"].as_f64().filter(|n| n.is_finite() && *n >= 0.);
    let Some((window, fill)) = window.zip(fill) else {
        dom::hidden("context-meter", true);
        return;
    };
    let ratio = fill / window;
    let compacted = last["compacted"] == true;
    let show = ratio >= 0.5 || compacted;
    dom::hidden("context-meter", !show);
    if !show {
        return;
    }
    for (key, value) in [
        ("max", window),
        ("value", fill.min(window)),
        ("low", window * 0.5),
        ("high", window * 0.8),
        ("optimum", 0.),
    ] {
        dom::attr("context-meter-bar", key, &value.to_string());
    }
    dom::attr(
        "context-meter-bar",
        "aria-valuetext",
        &format!("{fill:.0} of {window:.0} tokens"),
    );
    let full = ratio >= 0.8;
    let percent = format!("{:.0}%", 100. * ratio);
    dom::text(
        "context-meter-text",
        &format!(
            "{}{percent}{}",
            if full { "▲ " } else { "" },
            if compacted {
                " · compacted"
            } else if full {
                " · nearly full"
            } else {
                ""
            }
        ),
    );
    dom::attr(
        "context-meter",
        "data-full",
        if full { "true" } else { "false" },
    );
    dom::attr(
        "context-meter",
        "aria-label",
        &format!(
            "Context {percent} full{}. Open usage.",
            if compacted { ", compacted" } else { "" }
        ),
    );
}

pub fn group_view(group: &str) -> String {
    if group == "sessions" {
        return "sessions".into();
    }
    m::data(&format!("last-view-{group}"))
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            match group {
                "knowledge" => "memory-use",
                "systems" => "systems",
                "omnigraph" => "omnigraph",
                "skills" => "skills",
                _ => "accelerators",
            }
            .into()
        })
}
pub(super) fn view_label(view: &str) -> &str {
    match view {
        "sessions" => "Conversation",
        "systems" => "Services",
        "system-changes" | "changes" => "Changes",
        "omnigraph" => "Context Graph",
        "checks" => "Checks",
        "usage" => "Usage",
        "memory-use" => "Memory use",
        "brain" => "Project Brain",
        "memory" => "Memory bank",
        "context" => "Context files",
        "skills" => "Library",
        "skill-capsules" => "Skill capsules",
        "create-skill" => "Create skill",
        "accelerators" => "Overview",
        "creator" => "Infrastructure Creator",
        "kit3" => "Open Source Kit",
        "setup" => "Install into project",
        _ => view,
    }
}
pub(crate) fn view_tabs_for_group(group: &str, has_session: bool) -> Vec<&'static str> {
    let views: &[&str] = match group {
        "sessions" => &["sessions", "changes", "checks", "usage"],
        "systems" => &["systems", "system-changes"],
        "omnigraph" => &["omnigraph"],
        "knowledge" => &["memory-use", "brain", "memory", "context"],
        "skills" => &["skills", "skill-capsules", "create-skill"],
        _ => &["accelerators", "creator", "kit3", "setup"],
    };
    views
        .iter()
        .copied()
        .filter(|v| !["changes", "checks", "usage"].contains(v) || has_session)
        .collect()
}

pub(super) fn render_tabs(view: &str, group: &str) {
    let sid = m::read(|s| s.sid.clone());
    let views = view_tabs_for_group(group, !sid.is_empty());
    let sig = views.join(",");
    dom::attr("view-tabs", "role", "tablist");
    dom::attr("view-tabs", "aria-label", "Section tabs");
    dom::hidden("view-tabs", views.len() < 2);
    let reuse_existing = dom::get("view-tabs").is_some_and(|tabs| {
        tabs.get_attribute("data-tab-views").as_deref() == Some(sig.as_str())
            && dom::children(&tabs).len() == views.len()
    });
    if reuse_existing {
        if let Some(tabs) = dom::get("view-tabs") {
            for button in dom::children(&tabs) {
                let item = button.get_attribute("data-view").unwrap_or_default();
                let selected = item == view;
                let _ =
                    button.set_attribute("aria-selected", if selected { "true" } else { "false" });
                let _ = button.set_attribute("tabindex", if selected { "0" } else { "-1" });
                if selected {
                    let _ = button.set_attribute("aria-current", "page");
                } else {
                    let _ = button.remove_attribute("aria-current");
                }
            }
            dom::sync_roving_widget(&tabs);
        }
    } else {
        dom::clear("view-tabs");
        dom::attr("view-tabs", "data-tab-views", &sig);
        for item in views {
            let selected = item == view;
            let button = dom::button(view_label(item), "view-open", json!(item));
            button.set_class_name("view-tab");
            let _ = button.set_attribute("role", "tab");
            let _ = button.set_attribute("data-view", item);
            let _ = button.set_attribute("aria-selected", if selected { "true" } else { "false" });
            let _ = button.set_attribute("tabindex", if selected { "0" } else { "-1" });
            if selected {
                let _ = button.set_attribute("aria-current", "page");
            }
            dom::append("view-tabs", &button);
        }
        if let Some(tabs) = dom::get("view-tabs") {
            dom::sync_roving_widget(&tabs);
        }
    }
    dom::hidden("open-kit3", view != "kit3");
}

pub(super) fn repeated_suffix_removed<'a>(mut value: &'a str, suffix: &str) -> &'a str {
    if suffix.is_empty() {
        return value;
    }
    while let Some(rest) = value.strip_suffix(suffix) {
        value = rest;
    }
    value
}

pub(super) fn merge_controls() {
    let mut draft = m::data("merge-draft");
    let (sid, epoch) = m::read(|s| (s.sid.clone(), s.epoch));
    if draft.is_object() && !merge_draft_owned(&draft, &sid, &dom::value("project"), epoch) {
        m::take("merge-draft");
        draft = V::Null;
        reset_merge_archive();
    }
    let session = m::read(|s| s.selected.clone());
    let merging = draft.is_object();
    if merging {
        dom::hidden("merge-context", false);
        dom::text("merge-context-title", "Merge chats into a new task");
        dom::text(
            "merge-context-note",
            "The first launch reads an immutable archive of these chats as untrusted history. Write the new task's instruction below.",
        );
        dom::clear("merge-context-sources");
        for id in m::array(&draft["sources"]) {
            let source = m::read(|s| {
                s.sessions
                    .iter()
                    .find(|v| v["id"] == id)
                    .cloned()
                    .unwrap_or(V::Null)
            });
            dom::append(
                "merge-context-sources",
                &dom::el("p", "merge-source", &m::summary(&source)),
            );
        }
        dom::hidden("merge-clear", false);
        for id in ["workflow", "workspace", "clash-enabled", "attach-files"] {
            dom::disabled(id, true);
        }
    } else if session["merge"].is_object() {
        dom::hidden("merge-context", false);
        dom::hidden("merge-clear", true);
        dom::hidden(
            "merge-archive",
            session["merge"]["archive_deleted_at"].is_string(),
        );
        dom::hidden(
            "merge-delete",
            session["merge"]["archive_deleted_at"].is_string(),
        );
        dom::disabled("merge-delete", m::active(&session));
        dom::hidden(
            "merge-restart",
            !["interrupted", "failed"].contains(&session["status"].as_str().unwrap_or(""))
                || session["native_session_id"].is_string()
                || session["merge"]["archive_deleted_at"].is_string(),
        );
    } else {
        dom::hidden("merge-context", true);
    }
}

pub fn budget_fields_valid(prefix: &str) -> bool {
    ["usd", "tokens", "seconds"]
        .iter()
        .all(|key| dom::numeric_valid(&format!("{prefix}-{key}")))
}
