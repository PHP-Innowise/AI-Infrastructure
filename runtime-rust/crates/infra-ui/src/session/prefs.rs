use super::*;

pub(super) const PREFS: &str = "harness.sessions.preferences.v1";
pub(super) const TAB_DRAFTS: &str = "harness.sessions.tab-drafts.v1";
pub(super) const PREF_FIELDS: &[&str] = &[
    "provider",
    "workflow",
    "mode",
    "permissions",
    "agents-enabled",
    "agent-count",
    "workspace",
    "existing-worktree",
    "worktree-branch",
    "sdd-feature",
    "sdd-phase",
    "clash-enabled",
    "clash-challenger",
    "clash-rounds",
    "fleet-dry-run",
    "fleet-budget",
    "fleet-worker-timeout",
    "brain-link-kind",
    "brain-link-review",
    "brain-link-task-id",
    "brain-link-goal",
    "brain-link-query",
    "model-choice",
    "model",
    "thinking-effort",
    "omnigraph-enabled",
    "omnigraph-branch-mode",
    "omnigraph-existing-branch",
    "omnigraph-pull-branches",
    "omnigraph-session-query",
    "omnigraph-shared-space",
    "omnigraph-auto-publish",
    "omnigraph-auto-merge",
    "skillcache-mode",
    "skillcache-compiler-model",
];

pub(super) fn preferences_selection(mut prefs: V, project: &str, sid: &str) -> V {
    if !prefs.is_object() {
        prefs = json!({});
    }
    prefs["project_id"] = json!(project);
    prefs["session_id"] = nullable(sid.to_owned());
    prefs
}
pub(super) fn preferences_update(
    mut prefs: V,
    project: &str,
    sid: &str,
    fields: V,
    merge_draft: &V,
) -> Option<V> {
    if merge_draft.is_object() {
        return None;
    }
    if !prefs.is_object() {
        prefs = json!({});
    }
    if !prefs["drafts"].is_object() {
        prefs["drafts"] = json!({});
    }
    if sid.is_empty() {
        prefs["drafts"][project] = fields;
    }
    Some(preferences_selection(prefs, project, sid))
}
pub(super) fn merge_draft_owned(draft: &V, sid: &str, project: &str, epoch: u64) -> bool {
    draft.is_object()
        && sid.is_empty()
        && draft["project_id"] == project
        && draft["selection_epoch"].as_u64() == Some(epoch)
}
pub(super) fn reset_merge_archive() {
    m::next("merge-archive");
    m::next("merge-archive-scope");
    m::take("delete-armed");
    dom::clear("merge-archive-content");
    dom::hidden("merge-archive-content", true);
    dom::attr("merge-archive", "aria-expanded", "false");
    dom::text("merge-archive", "View saved context");
    dom::text("merge-delete", "Delete saved context");
    dom::hidden("merge-delete-keep", true);
}
pub(super) fn budget_request(session: &V, limits: V, timeout: V) -> V {
    let mut request = json!({"budgets":limits,"revision":session["budget_revision"]});
    if session["workflow"] == "fleet-review" || session["fleet"].is_object() {
        request["worker_timeout"] = timeout;
    }
    request
}
pub(super) fn draft_project(selected: &str) -> String {
    m::data("composer-project")
        .as_str()
        .filter(|owner| !owner.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| selected.to_owned())
}
pub fn save_preferences() {
    if m::read(|s| s.restoring || s.boot.is_null()) {
        return;
    }
    inbox::save_draft();
    let prefs = dom::json_storage(PREFS);
    let tab_drafts = dom::json_session_storage(TAB_DRAFTS);
    let project = draft_project(&dom::value("project"));
    let mut fields = json!({});
    for id in PREF_FIELDS {
        fields[*id] = if [
            "agents-enabled",
            "clash-enabled",
            "fleet-dry-run",
            "brain-link-review",
            "omnigraph-enabled",
            "omnigraph-auto-publish",
            "omnigraph-auto-merge",
        ]
        .contains(id)
        {
            json!(dom::checked(id))
        } else {
            json!(dom::value(id))
        };
    }
    fields["budgets"] = budgets("session-budgets");
    fields["routing"] = routing();
    fields["bank"] = json!(dom::value("brain-link-bank"));
    fields["task"] = json!(dom::value("brain-link-task"));
    let mut draft_fields = fields.clone();
    draft_fields["prompt"] = json!(dom::value("prompt"));

    let sid = m::read(|s| s.sid.clone());
    // Save per-tab draft state in sessionStorage so multiple tabs never overwrite each other's prompts.
    if let Some(updated_tab) = preferences_update(
        tab_drafts,
        &project,
        &sid,
        draft_fields,
        &m::data("merge-draft"),
    ) {
        dom::save_session(TAB_DRAFTS, Some(&updated_tab.to_string()));
    }
    // Keep cross-tab workspace settings (excluding active draft prompt) in localStorage.
    let mut shared = preferences_selection(prefs, &project, &sid);
    shared["right_tab"] = json!(m::read(|s| s.workspace.right_tab.clone()));
    if let Some(pct) = m::read(|s| s.workspace.split_percent) {
        shared["split_percent"] = json!(pct);
    }
    if !shared["defaults"].is_object() {
        shared["defaults"] = json!({});
    }
    shared["defaults"][&project] = fields;
    dom::save(PREFS, Some(&shared.to_string()));
}
/// Chooses the initial workspace project from URL route, saved preferences, or the first bootstrap project.
pub(super) fn resolve_initial_project(
    route_project: &str,
    saved_project: &str,
    projects: &[V],
) -> String {
    let has = |candidate: &str| {
        !candidate.is_empty() && projects.iter().any(|p| m::text(p, "id") == candidate)
    };
    if has(route_project) {
        return route_project.to_owned();
    }
    if has(saved_project) {
        return saved_project.to_owned();
    }
    projects
        .iter()
        .map(|p| m::text(p, "id"))
        .find(|id| !id.is_empty())
        .unwrap_or_default()
}

pub(super) fn restore_preferences() {
    let prefs = dom::json_storage(PREFS);
    if let Some(rt) = prefs["right_tab"]
        .as_str()
        .filter(|t| matches!(*t, "graph" | "diff" | "checks"))
    {
        set_workspace_right_tab(rt);
    }
    if let Some(pct) = prefs["split_percent"].as_f64() {
        apply_workspace_split_percent(pct, false);
    }
    // Authoritative URL hash project takes precedence over localStorage fallback,
    // and bootstrap defaults to the first registered project so Code Graph activates cleanly.
    let hash_project = dom::hash_route().project;
    let saved_project = m::text(&prefs, "project_id");
    let projects = m::read(|s| m::array(&s.boot["projects"]));
    let project = resolve_initial_project(&hash_project, &saved_project, &projects);
    if !m::project(&project).is_null() {
        sync_project(&project);
    }
    restore_draft(&dom::value("project"));
}
/// A remembered field as a new draft starts with it. Bypass Permissions is
/// chosen for each session on purpose, so a draft starts with Auto instead.
pub(crate) fn restored_value<'a>(id: &str, value: &'a str) -> &'a str {
    if id == "permissions" && value == "bypass" {
        "auto"
    } else {
        value
    }
}
pub(super) fn restore_draft(project: &str) {
    m::set("composer-project", json!(project));
    let tab_drafts = dom::json_session_storage(TAB_DRAFTS);
    let prefs = dom::json_storage(PREFS);
    let fields = if tab_drafts["drafts"][project].is_object() {
        &tab_drafts["drafts"][project]
    } else if prefs["defaults"][project].is_object() {
        &prefs["defaults"][project]
    } else {
        &prefs["drafts"][project]
    };
    if !fields.is_object() {
        return;
    }
    m::write(|s| s.restoring = true);
    for id in PREF_FIELDS {
        if let Some(b) = fields[*id].as_bool() {
            dom::set_checked(id, b);
        } else if let Some(v) = fields[*id].as_str() {
            dom::set_value(id, restored_value(id, v));
        }
    }
    let model = fields["model"].as_str().unwrap_or("").to_owned();
    models(
        "",
        Some(if fields["model-choice"] == "--custom--" {
            model
        } else {
            m::text(fields, "model-choice")
        }),
    );
    dom::set_value("thinking-effort", &m::text(fields, "thinking-effort"));
    restore_budgets("session-budgets", &fields["budgets"]);
    restore_model_routing(&fields["routing"]);
    dom::set_value("prompt", &m::text(fields, "prompt"));
    m::write(|s| s.restoring = false);
}

type ProjectChangeListener = std::rc::Rc<dyn Fn(&str)>;

thread_local! {
    static PROJECT_LISTENERS: RefCell<Vec<ProjectChangeListener>> = const { RefCell::new(Vec::new()) };
}

/// Subscribes a workspace module (`systems`, `management`, `knowledge`, `activity`, `graph`) to global project changes.
#[allow(dead_code)]
pub(crate) fn on_project_change(handler: impl Fn(&str) + 'static) {
    PROJECT_LISTENERS.with(|list| list.borrow_mut().push(std::rc::Rc::new(handler)));
}

/// Switches the consolidated global workspace project control and propagates to non-held scoped panels.
pub(crate) fn set_workspace_project(id: &str) {
    sync_project(id);
    if !m::yes(&m::data("bootstrapping")) && !m::read(|s| s.boot.is_null()) {
        sync_hash_from_state(false);
    }
}

/// Shared project switcher used by both `#project-switcher` and the Command Palette.
pub(crate) async fn switch_project_from_shell(id: &str) -> bool {
    if m::read(|s| s.view == "sessions") {
        if m::read(|s| s.pending) {
            let current = m::current_project();
            dom::set_value("project-switcher", &current);
            dom::set_value("project", &current);
            return false;
        }
        new_session(Some(id));
    } else {
        set_workspace_project(id);
    }
    load_git().await;
    true
}

pub fn sync_project(id: &str) {
    let previous_global = m::current_project();
    m::set_current_project(id);
    for select in [
        "project",
        "project-switcher",
        "context-project",
        "memory-use-project",
        "memory-project",
        "brain-project",
        "skills-project",
        "create-skill-project",
        "setup-project",
        "creator-project",
        "accelerator-project",
        "system-project",
        "omnigraph-project",
    ] {
        let prev = if dom::get(select).is_some()
            || m::read(|s| s.workspace.scoped_projects.contains_key(select))
        {
            dom::value(select)
        } else {
            previous_global.clone()
        };
        let held = select == "system-project"
            && (m::yes(&m::data("editor-dirty"))
                || m::yes(&m::data("editor-pending"))
                || m::yes(&m::data("system-pending")))
            || select == "creator-project" && m::yes(&m::data("creator-pending"));
        let scope = match select {
            "creator-project" => Some("creator-scope"),
            "system-project" => Some("editor-scope"),
            "memory-project" => Some("memory-knowledge-scope"),
            "brain-project" => Some("brain-knowledge-scope"),
            _ => None,
        };
        let changed =
            scope.is_some_and(|scope| m::project_scope_transition(scope, &prev, id, held));
        if !held {
            dom::set_value(select, id);
        }
        if select == "omnigraph-project" && prev != id {
            omnigraph::on_project_changed();
        }
        if changed {
            match select {
                "creator-project" => management::reset_creator_project(),
                "system-project" => systems::reset_system_project(),
                "memory-project" => knowledge::reset_project("memory"),
                "brain-project" => knowledge::reset_project("brain"),
                _ => {}
            }
        }
    }
    m::set("composer-project", json!(id));
    if previous_global != id && !m::read(|s| s.boot.is_null()) {
        if !m::yes(&m::data("bootstrapping")) {
            spawn_local(async {
                crate::graph::load(false).await;
            });
        }
        let listeners: Vec<ProjectChangeListener> =
            PROJECT_LISTENERS.with(|list| list.borrow().clone());
        for cb in listeners {
            cb(id);
        }
    }
}

/// What a session's Permissions setting does for the chosen host and mode.
pub(crate) fn permissions_hint(provider: &str, mode: &str, permission: &str) -> &'static str {
    if mode != "edit" {
        return "Planning is read-only, so nothing asks for permission. The setting applies when a turn edits.";
    }
    match (permission, provider) {
        ("manual", "claude") => {
            "Claude Code asks before edits and commands. Each request appears in the chat with Allow and Deny, and the run waits for your answer."
        }
        ("manual", "codex") => {
            "Codex works read-only and asks before each edit and each command that writes. Each request appears in the chat with Allow and Deny, and the run waits for your answer."
        }
        ("manual", "cursor") => {
            "Cursor asks before actions outside your allowlist. Each request appears in the chat with Allow and Deny, and the run waits for your answer."
        }
        ("manual", _) => {
            "Each request the agent asks appears in the chat with Allow and Deny, and the run waits for your answer."
        }
        ("bypass", _) => {
            "Every action runs without asking and without the sandbox. Use it only in a disposable checkout or container."
        }
        (_, "claude") => {
            "Claude Code's auto mode reviews each action and declines risky ones; nothing waits for you. Where a model has no auto mode, it accepts edits and declines the rest."
        }
        (_, "codex") => {
            "Codex edits in its workspace sandbox and its automatic reviewer answers anything that needs approval; nothing waits for you."
        }
        (_, "cursor") => {
            "Cursor edits the workspace and runs commands in its sandbox; anything else that needs approval is declined. Nothing waits for you."
        }
        _ => "The agent works on its own and the host declines what needs approval; nothing waits for you.",
    }
}
pub(crate) fn mark_dirty(group: &str) {
    if m::read(|s| s.restoring || s.sid.is_empty()) {
        return;
    }
    let mut dirty = m::data("session-dirty-fields")
        .as_object()
        .cloned()
        .unwrap_or_default();
    dirty.insert(group.to_owned(), json!(true));
    m::set("session-dirty-fields", V::Object(dirty));
}

pub(super) fn mark_dirty_for_id(id: &str) {
    match id {
        "permissions" => mark_dirty("permissions"),
        "mode" => mark_dirty("mode"),
        "agents-enabled" | "agent-count" => mark_dirty("agents"),
        "model-choice" | "model" | "thinking-effort" => mark_dirty("model"),
        "model-routing-enabled"
        | "routing-plan-model-choice"
        | "routing-plan-model"
        | "routing-plan-thinking-effort"
        | "routing-edit-model-choice"
        | "routing-edit-model"
        | "routing-edit-thinking-effort" => mark_dirty("model_routing"),
        "sdd-phase" | "sdd-feature" => mark_dirty("sdd"),
        "clash-enabled"
        | "clash-challenger"
        | "clash-rounds"
        | "clash-model-choice"
        | "clash-model"
        | "clash-thinking-effort" => mark_dirty("clash"),
        "skillcache-mode" | "skillcache-compiler-model" => mark_dirty("skillcache"),
        _ => {}
    }
}

pub(super) fn reset_session_settings_state() {
    m::take("session-dirty-fields");
    m::take("applied-session-snapshot");
}

pub(super) fn session_settings_snapshot_fields(v: &V) -> V {
    let clash = if v["clash"].is_object() {
        json!({
            "enabled": true,
            "challenger": m::text(&v["clash"], "challenger"),
            "rounds": m::string(&v["clash"]["rounds"]),
            "model": m::text(&v["clash"], "challenger_model"),
            "effort": m::text(&v["clash"], "challenger_thinking_effort"),
        })
    } else {
        json!({"enabled": false})
    };
    json!({
        "base": {
            "provider": v["provider"],
            "workflow": v["workflow"],
            "workspace": v["workspace"],
        },
        "mode": v["mode"],
        "permissions": v["permissions"].as_str().unwrap_or("auto"),
        "agents": {
            "enabled": m::yes(&v["agents_enabled"]),
            "count": v["agent_count"],
        },
        "model": {
            "model": m::text(v, "model"),
            "effort": m::text(v, "thinking_effort"),
        },
        "model_routing": v["model_routing"],
        "sdd": {
            "feature": m::text(&v["sdd"], "feature"),
            "phase": v["sdd"]["phase"].as_str().unwrap_or("specify"),
        },
        "clash": clash,
        "skillcache": crate::skillcache::snapshot(v),
    })
}

pub(super) fn should_apply_setting_group(
    snapshot: &V,
    dirty: &V,
    epoch: u64,
    sid: &str,
    group: &str,
    incoming: &V,
) -> bool {
    let same_selection =
        snapshot["epoch"].as_u64() == Some(epoch) && snapshot["sid"].as_str() == Some(sid);
    if !same_selection {
        return true;
    }
    if dirty[group] == true {
        return false;
    }
    snapshot["fields"][group] != *incoming
}

pub fn apply_settings(v: &V) {
    let epoch = m::read(|s| s.epoch);
    let sid = m::text(v, "id");
    let snapshot = m::data("applied-session-snapshot");
    let dirty = m::data("session-dirty-fields");
    let incoming = session_settings_snapshot_fields(v);
    let mut next_fields = if snapshot["epoch"].as_u64() == Some(epoch)
        && snapshot["sid"].as_str() == Some(sid.as_str())
        && snapshot["fields"].is_object()
    {
        snapshot["fields"].clone()
    } else {
        json!({})
    };
    if should_apply_setting_group(&snapshot, &dirty, epoch, &sid, "base", &incoming["base"]) {
        for (id, key) in [
            ("provider", "provider"),
            ("workflow", "workflow"),
            ("workspace", "workspace"),
        ] {
            if !v[key].is_null() {
                dom::set_value(id, &m::string(&v[key]));
            }
        }
        next_fields["base"] = incoming["base"].clone();
    }
    if should_apply_setting_group(&snapshot, &dirty, epoch, &sid, "mode", &incoming["mode"]) {
        if !v["mode"].is_null() {
            dom::set_value("mode", &m::string(&v["mode"]));
        }
        next_fields["mode"] = incoming["mode"].clone();
    }
    if should_apply_setting_group(
        &snapshot,
        &dirty,
        epoch,
        &sid,
        "permissions",
        &incoming["permissions"],
    ) {
        dom::set_value("permissions", v["permissions"].as_str().unwrap_or("auto"));
        next_fields["permissions"] = incoming["permissions"].clone();
    }
    if should_apply_setting_group(
        &snapshot,
        &dirty,
        epoch,
        &sid,
        "agents",
        &incoming["agents"],
    ) {
        if !v["agent_count"].is_null() {
            dom::set_value("agent-count", &m::string(&v["agent_count"]));
        }
        dom::set_checked("agents-enabled", m::yes(&v["agents_enabled"]));
        next_fields["agents"] = incoming["agents"].clone();
    }
    if should_apply_setting_group(&snapshot, &dirty, epoch, &sid, "model", &incoming["model"]) {
        models("", Some(m::text(v, "model")));
        dom::set_value("thinking-effort", &m::text(v, "thinking_effort"));
        next_fields["model"] = incoming["model"].clone();
    }
    if should_apply_setting_group(
        &snapshot,
        &dirty,
        epoch,
        &sid,
        "model_routing",
        &incoming["model_routing"],
    ) {
        restore_model_routing(&v["model_routing"]);
        next_fields["model_routing"] = incoming["model_routing"].clone();
    }
    if should_apply_setting_group(&snapshot, &dirty, epoch, &sid, "sdd", &incoming["sdd"]) {
        dom::set_value("sdd-feature", &m::text(&v["sdd"], "feature"));
        dom::set_value("sdd-phase", v["sdd"]["phase"].as_str().unwrap_or("specify"));
        next_fields["sdd"] = incoming["sdd"].clone();
    }
    let revision = format!("{}:{}", m::text(v, "id"), m::string(&v["budget_revision"]));
    if m::data("budget-revision") != revision {
        restore_budgets("session-budgets", &v["budgets"]);
        if v["fleet"].is_object() {
            dom::set_checked("fleet-dry-run", m::yes(&v["fleet"]["dry_run"]));
            dom::set_value("fleet-budget", &m::string(&v["fleet"]["budget_usd"]));
            dom::set_value(
                "fleet-worker-timeout",
                &m::string(&v["fleet"]["worker_timeout"]),
            );
            for input in dom::all("#fleet-lenses input") {
                let lens = dom::value_of(&input);
                if let Some(input) = input.dyn_ref::<web_sys::HtmlInputElement>() {
                    input.set_checked(
                        m::array(&v["fleet"]["lenses"])
                            .iter()
                            .any(|value| value == &lens),
                    );
                }
            }
        }
        m::set("budget-revision", json!(revision));
    }
    if should_apply_setting_group(&snapshot, &dirty, epoch, &sid, "clash", &incoming["clash"]) {
        dom::set_checked("clash-enabled", v["clash"].is_object());
        if v["clash"].is_object() {
            dom::set_value("clash-challenger", &m::text(&v["clash"], "challenger"));
            dom::set_value("clash-rounds", &m::string(&v["clash"]["rounds"]));
            models("clash-", Some(m::text(&v["clash"], "challenger_model")));
            dom::set_value(
                "clash-thinking-effort",
                &m::text(&v["clash"], "challenger_thinking_effort"),
            );
        }
        next_fields["clash"] = incoming["clash"].clone();
    }
    if should_apply_setting_group(
        &snapshot,
        &dirty,
        epoch,
        &sid,
        "skillcache",
        &incoming["skillcache"],
    ) {
        crate::skillcache::apply_session(v);
        next_fields["skillcache"] = incoming["skillcache"].clone();
    }
    m::set(
        "applied-session-snapshot",
        json!({"epoch": epoch, "sid": sid, "fields": next_fields}),
    );
    dom::text("workspace-session-branch", &m::text(v, "branch"));
    dom::text("workspace-session-path", &m::text(v, "project_path"));
    knowledge::render_linked(v);
    omnigraph::apply_session_settings(v);
    omnigraph::render_session_bar(v);
    run::outcomes(v);
}
