use serde_json::Value as V;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};
use web_sys::AbortController;

/// Typed workspace navigation, global project, filter, and layout state.
#[derive(Clone, Debug)]
pub struct WorkspaceState {
    pub project_id: String,
    pub scoped_projects: BTreeMap<String, String>,
    pub right_tab: String,
    pub split_percent: Option<f64>,
    pub history_query: String,
    pub history_waiting_only: bool,
    pub command_palette_open: bool,
    pub command_palette_selected: usize,
    pub command_palette_query: String,
    pub sidebar_open: bool,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            project_id: String::new(),
            scoped_projects: BTreeMap::new(),
            right_tab: "graph".into(),
            split_percent: None,
            history_query: String::new(),
            history_waiting_only: false,
            command_palette_open: false,
            command_palette_selected: 0,
            command_palette_query: String::new(),
            sidebar_open: false,
        }
    }
}

/// Typed live-stream, scrollback window, and SSE/fallback transport state.
#[derive(Clone, Debug)]
pub struct SessionStreamState {
    pub sticky_bottom: bool,
    pub unseen_count: usize,
    pub visible_limit: usize,
    pub oldest_event_id: Option<u64>,
    pub latest_event_id: Option<u64>,
    pub has_older: bool,
    pub loading_older: bool,
    pub total_events: usize,
    pub consecutive_poll_errors: u32,
    pub sse_connected: bool,
}

impl Default for SessionStreamState {
    fn default() -> Self {
        Self {
            sticky_bottom: true,
            unseen_count: 0,
            visible_limit: 120,
            oldest_event_id: None,
            latest_event_id: None,
            has_older: false,
            loading_older: false,
            total_events: 0,
            consecutive_poll_errors: 0,
            sse_connected: false,
        }
    }
}

/// Typed file-grouped diff viewer and delivery/checks retry state.
#[derive(Clone, Debug, Default)]
pub struct DiffViewState {
    pub collapsed_files: BTreeSet<String>,
    pub focused_file: String,
    pub results_retry_count: u32,
}

/// Remembered session-level permission rule fetched from the runner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalRule {
    pub id: String,
    pub key: String,
    pub tool: String,
    pub pattern: String,
    pub provider: String,
    pub scope: String,
    pub created_at: String,
}

impl ApprovalRule {
    /// Parses either a raw rule key string or an object returned by `/permission-rules`.
    pub fn from_json(v: &V) -> Option<Self> {
        if let Some(raw) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            let (tool, pattern) = if let Some((head, tail)) = raw.split_once('(') {
                (
                    head.trim().to_owned(),
                    tail.trim_end_matches(')').trim().to_owned(),
                )
            } else if let Some((head, tail)) = raw.split_once(':') {
                (head.trim().to_owned(), tail.trim().to_owned())
            } else {
                (raw.to_owned(), String::new())
            };
            return Some(Self {
                id: raw.to_owned(),
                key: raw.to_owned(),
                tool,
                pattern,
                provider: String::new(),
                scope: "session".into(),
                created_at: String::new(),
            });
        }
        let id = ["id", "rule_key", "key", "rule"]
            .iter()
            .find_map(|field| v[*field].as_str().map(str::trim).filter(|s| !s.is_empty()))?
            .to_owned();
        let key = ["rule_key", "key", "id", "rule", "pattern"]
            .iter()
            .find_map(|field| v[*field].as_str().map(str::trim).filter(|s| !s.is_empty()))
            .unwrap_or(&id)
            .to_owned();
        let tool = ["tool", "tool_name", "kind"]
            .iter()
            .find_map(|field| v[*field].as_str().map(str::trim).filter(|s| !s.is_empty()))
            .unwrap_or("")
            .to_owned();
        let pattern = [
            "rule_summary",
            "pattern",
            "target",
            "command",
            "summary",
            "rule",
        ]
        .iter()
        .find_map(|field| v[*field].as_str().map(str::trim).filter(|s| !s.is_empty()))
        .unwrap_or(&key)
        .to_owned();
        let provider = v["provider"]
            .as_str()
            .map(str::trim)
            .unwrap_or("")
            .to_owned();
        let scope = v["scope"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("session")
            .to_owned();
        let created_at = v["created_at"]
            .as_str()
            .map(str::trim)
            .unwrap_or("")
            .to_owned();
        Some(Self {
            id,
            key,
            tool,
            pattern,
            provider,
            scope,
            created_at,
        })
    }
}

/// Typed state for pending approvals queue drawer and active session rules.
#[derive(Clone, Debug, Default)]
pub struct ApprovalQueueState {
    pub drawer_open: bool,
    pub rules: Vec<ApprovalRule>,
    pub rules_sid: String,
    pub rules_sid_manual: bool,
    pub rules_loading: bool,
    pub rules_error: String,
    pub waiting_counts: BTreeMap<String, usize>,
}

pub struct State {
    pub boot: V,
    pub sessions: Vec<V>,
    pub selected: V,
    pub sid: String,
    pub events: Vec<V>,
    pub seen: BTreeSet<String>,
    pub epoch: u64,
    pub cursor: u64,
    pub pending: bool,
    pub auth_failed: bool,
    pub view: String,
    pub data: BTreeMap<String, V>,
    pub epochs: BTreeMap<String, u64>,
    pub aborts: BTreeMap<String, AbortController>,
    pub timers: BTreeMap<String, i32>,
    pub restoring: bool,
    pub workspace: WorkspaceState,
    pub stream: SessionStreamState,
    pub diff_view: DiffViewState,
    pub approvals: ApprovalQueueState,
}
impl Default for State {
    fn default() -> Self {
        Self {
            boot: V::Null,
            sessions: vec![],
            selected: V::Null,
            sid: String::new(),
            events: vec![],
            seen: BTreeSet::new(),
            epoch: 0,
            cursor: 0,
            pending: false,
            auth_failed: false,
            view: "sessions".into(),
            data: BTreeMap::new(),
            epochs: BTreeMap::new(),
            aborts: BTreeMap::new(),
            timers: BTreeMap::new(),
            restoring: false,
            workspace: WorkspaceState::default(),
            stream: SessionStreamState::default(),
            diff_view: DiffViewState::default(),
            approvals: ApprovalQueueState::default(),
        }
    }
}

/// Returns true if `id` names the unified workspace project selector or a consolidated legacy `*-project` field.
pub fn is_project_field_id(id: &str) -> bool {
    matches!(
        id,
        "project"
            | "project-switcher"
            | "system-project"
            | "creator-project"
            | "setup-project"
            | "skills-project"
            | "create-skill-project"
            | "accelerator-project"
            | "context-project"
            | "memory-use-project"
            | "memory-project"
            | "brain-project"
            | "omnigraph-project"
    )
}

/// Returns the active global workspace project ID.
pub fn current_project() -> String {
    read(|s| s.workspace.project_id.clone())
}

/// Updates the active global workspace project ID and mirrors it into legacy `data["workspace-project"]`.
pub fn set_current_project(id: &str) {
    write(|s| {
        s.workspace.project_id = id.to_owned();
        s.data
            .insert("workspace-project".into(), serde_json::json!(id));
    });
}

/// Returns the project ID for a view scope, falling back to `current_project()` unless overridden.
pub fn scoped_project(scope_id: &str) -> String {
    if matches!(scope_id, "project" | "project-switcher") {
        return current_project();
    }
    read(|s| {
        s.workspace
            .scoped_projects
            .get(scope_id)
            .cloned()
            .unwrap_or_else(|| s.workspace.project_id.clone())
    })
}

/// Records an explicit project ID for a scoped view (e.g., `"system-project"`, `"creator-project"`).
pub fn set_scoped_project(scope_id: &str, id: &str) {
    if matches!(scope_id, "project" | "project-switcher") {
        set_current_project(id);
        return;
    }
    write(|s| {
        s.workspace
            .scoped_projects
            .insert(scope_id.into(), id.into());
    });
}

/// Clears any per-view scoped override so `scope_id` inherits `current_project()`.
#[allow(dead_code)]
pub fn clear_scoped_project(scope_id: &str) {
    write(|s| {
        s.workspace.scoped_projects.remove(scope_id);
    });
}
thread_local! {pub static STATE:RefCell<State>=RefCell::new(State::default());}
pub fn read<T>(f: impl FnOnce(&State) -> T) -> T {
    STATE.with(|s| f(&s.borrow()))
}
pub fn write<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with(|s| f(&mut s.borrow_mut()))
}
pub fn data(key: &str) -> V {
    read(|s| s.data.get(key).cloned().unwrap_or(V::Null))
}
pub fn set(key: &str, v: V) {
    write(|s| {
        s.data.insert(key.into(), v);
    });
}
pub fn take(key: &str) -> V {
    write(|s| s.data.remove(key).unwrap_or(V::Null))
}
pub fn string(v: &V) -> String {
    v.as_str().map(str::to_owned).unwrap_or_else(|| {
        if v.is_null() {
            String::new()
        } else {
            v.to_string()
        }
    })
}
pub fn text(v: &V, key: &str) -> String {
    string(&v[key])
}
pub fn number(v: &V) -> Option<f64> {
    v.as_f64().filter(|v| v.is_finite())
}
pub fn yes(v: &V) -> bool {
    v.as_bool() == Some(true)
}
pub fn array(v: &V) -> Vec<V> {
    v.as_array().cloned().unwrap_or_default()
}
pub fn next(key: &str) -> u64 {
    write(|s| {
        if let Some(a) = s.aborts.remove(key) {
            a.abort();
        }
        let e = s.epochs.entry(key.into()).or_default();
        *e += 1;
        *e
    })
}
pub fn current(key: &str, e: u64) -> bool {
    read(|s| s.epochs.get(key).copied().unwrap_or_default() == e)
}
pub fn active(v: &V) -> bool {
    matches!(v["status"].as_str(), Some("queued" | "running"))
}
pub fn upsert(v: V) {
    if !v.is_object() {
        return;
    }
    write(|s| {
        let id = text(&v, "id");
        if id.is_empty() {
            return;
        }
        if id == s.sid {
            s.selected = v.clone();
        }
        if let Some(old) = s.sessions.iter_mut().find(|x| text(x, "id") == id) {
            *old = v;
        } else {
            s.sessions.insert(0, v);
        }
    });
}
pub fn project(id: &str) -> V {
    read(|s| {
        array(&s.boot["projects"])
            .into_iter()
            .find(|x| text(x, "id") == id)
            .unwrap_or(V::Null)
    })
}
pub fn provider(id: &str) -> V {
    read(|s| {
        array(&s.boot["providers"])
            .into_iter()
            .find(|x| text(x, "id") == id)
            .unwrap_or(V::Null)
    })
}
pub fn summary(v: &V) -> String {
    ["name", "title", "label", "summary", "message", "path", "id"]
        .iter()
        .find_map(|k| v[*k].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("")
        .into()
}
pub fn label(s: &str) -> String {
    match s {
        "omnigraph" => "Omnigraph".into(),
        _ => s.replace(['_', '-'], " "),
    }
}
pub fn request_identity() -> String {
    secure_request_identity().unwrap_or_else(|| format_identity([0; 16]))
}
pub fn secure_request_identity() -> Option<String> {
    let mut bytes = [0u8; 16];
    crate::dom::window()
        .crypto()
        .ok()?
        .get_random_values_with_u8_array(&mut bytes)
        .ok()?;
    Some(format_identity(bytes))
}
fn format_identity(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let h = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}
pub const MERGE_PROMPT:&str="Continue the work from these chats. Keep each chat’s decisions and progress, name any conflicts between them, and propose the next steps.";
pub fn sdd_settings_valid(feature: &str, phase: &str, phases: &[V]) -> bool {
    !feature.is_empty()
        && feature.len() <= 80
        && feature != "memory"
        && feature.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
        && phases.iter().any(|entry| entry["id"] == phase)
}
pub fn sdd_mode(phase: &str) -> &'static str {
    if phase == "review" {
        "plan"
    } else {
        "edit"
    }
}
pub fn selection_matches(
    current_id: &str,
    current_epoch: u64,
    expected_id: &str,
    expected_epoch: u64,
) -> bool {
    current_id == expected_id && current_epoch == expected_epoch
}
pub fn bind_scope(key: &str, owner: &str, item: &str) -> u64 {
    let generation = next(key);
    set(
        &format!("{key}-owner"),
        serde_json::json!({"owner":owner,"item":item}),
    );
    generation
}
pub fn owns_scope(key: &str, owner: &str, item: &str) -> bool {
    let bound = data(&format!("{key}-owner"));
    bound["owner"] == owner && bound["item"] == item
}
pub fn scope_current(key: &str, generation: u64, owner: &str, item: &str) -> bool {
    current(key, generation) && owns_scope(key, owner, item)
}
pub fn bind_pending(key: &str, generation: u64) {
    set(key, serde_json::json!(true));
    set(&format!("{key}-generation"), serde_json::json!(generation));
}
pub fn finish_pending(key: &str, generation: u64) -> bool {
    if data(&format!("{key}-generation")).as_u64() != Some(generation) {
        return false;
    }
    take(&format!("{key}-generation"));
    set(key, serde_json::json!(false));
    true
}
pub fn cache_scope_value(
    cache: &str,
    scope: &str,
    generation: u64,
    owner: &str,
    item: &str,
    mut value: V,
) -> bool {
    if !value.is_object() || !scope_current(scope, generation, owner, item) {
        return false;
    }
    value["view_generation"] = serde_json::json!(generation);
    set(cache, value);
    true
}
pub fn project_scope_transition(scope: &str, previous: &str, selected: &str, held: bool) -> bool {
    if held || previous == selected {
        return false;
    }
    bind_scope(scope, selected, "");
    true
}

pub fn project_for_reentry(
    scope: &str,
    current: &str,
    sidebar: &str,
    held: bool,
    projects: &[V],
    previous_view: &str,
) -> Option<String> {
    if ["systems", "system-changes"].contains(&previous_view) {
        return None;
    }
    if !projects.iter().any(|project| project["id"] == sidebar) {
        return None;
    }
    project_scope_transition(scope, current, sidebar, held).then(|| sidebar.to_owned())
}

pub fn omnigraph_enabled(session: &V) -> bool {
    let omni = &session["omnigraph"];
    omni.is_object()
        && omni["enabled"].as_bool() != Some(false)
        && (yes(&omni["enabled"])
            || omni["branch"].is_string()
            || !text(omni, "shared_space").is_empty()
            || !array(&omni["linked_session_ids"]).is_empty())
}

pub fn omnigraph_branch(session: &V) -> String {
    session["omnigraph"]["branch"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("main")
        .to_owned()
}

pub fn omnigraph_shared_space(session: &V) -> String {
    text(&session["omnigraph"], "shared_space")
}

pub fn omnigraph_linked_session_ids(session: &V) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for item in array(&session["omnigraph"]["linked_session_ids"]) {
        if let Some(id) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            if seen.insert(id.to_owned()) {
                out.push(id.to_owned());
            }
        }
    }
    out
}

pub fn upsert_sessions(sessions: &[V]) {
    for session in sessions {
        upsert(session.clone());
    }
}

/// Parses a numeric stream event ID from either an integer or string `"id"` field.
pub(crate) fn numeric_event_id(event: &V) -> Option<u64> {
    event["id"]
        .as_u64()
        .or_else(|| event["id"].as_str()?.trim().parse().ok())
}

/// Removes up to `remove_count` oldest events from `events` and clears their IDs from `seen`
/// so evicted history can still be reloaded cleanly via `?before=` pagination.
pub(crate) fn prune_events_prefix(
    events: &mut Vec<V>,
    seen: &mut BTreeSet<String>,
    remove_count: usize,
) -> (Vec<V>, Option<u64>, bool) {
    let count = remove_count.min(events.len());
    if count == 0 {
        return (Vec::new(), events.first().and_then(numeric_event_id), false);
    }
    let drained: Vec<V> = events.drain(..count).collect();
    let mut drained_any_numeric = false;
    for ev in &drained {
        if numeric_event_id(ev).is_some() {
            drained_any_numeric = true;
        }
        let id = string(&ev["id"]);
        if !id.is_empty() {
            seen.remove(&id);
        }
    }
    let oldest = events.first().and_then(numeric_event_id);
    (drained, oldest, drained_any_numeric)
}
#[cfg(test)]
mod sdd_contract_tests {
    use super::*;
    fn phases() -> Vec<V> {
        [
            "initialize",
            "specify",
            "plan",
            "tasks",
            "implement",
            "review",
        ]
        .into_iter()
        .map(|phase| serde_json::json!({"id":phase}))
        .collect()
    }
    #[test]
    fn starting_a_new_sdd_feature_needs_no_existing_documents_or_accelerator() {
        assert!(sdd_settings_valid("billing-v2", "initialize", &phases()));
        assert!(sdd_settings_valid("billing-v2", "specify", &phases()));
    }
    #[test]
    fn reserved_and_ambiguous_feature_paths_and_unknown_phases_are_rejected() {
        for feature in ["", "memory", "Billing", "a/b", "a_b", "a--b", "-a", "a-"] {
            assert!(
                !sdd_settings_valid(feature, "specify", &phases()),
                "{feature}"
            );
        }
        assert!(sdd_settings_valid(&"a".repeat(80), "specify", &phases()));
        assert!(!sdd_settings_valid(&"a".repeat(81), "specify", &phases()));
        assert!(!sdd_settings_valid("billing", "deploy", &phases()));
        assert!(!sdd_settings_valid("billing", "specify", &[]));
    }
    #[test]
    fn returning_to_a_session_does_not_rebind_an_old_document_request() {
        assert!(selection_matches("x", 7, "x", 7));
        assert!(!selection_matches("y", 8, "x", 7));
        assert!(!selection_matches("x", 9, "x", 7));
        assert!(selection_matches("x", 9, "x", 9));
    }
    #[test]
    fn visible_modes_match_the_native_sdd_permission_contract() {
        for phase in ["initialize", "specify", "plan", "tasks", "implement"] {
            assert_eq!(sdd_mode(phase), "edit", "{phase}");
        }
        assert_eq!(sdd_mode("review"), "plan");
    }
}

#[cfg(test)]
mod ownership_contract_tests {
    use super::*;
    #[test]
    fn old_creator_reply_cannot_reopen_actions_after_empty_project_or_return() {
        let a = bind_scope("creator-contract", "project-a", "run-a");
        let empty_b = bind_scope("creator-contract", "project-b", "");
        assert!(!scope_current("creator-contract", a, "project-a", "run-a"));
        assert!(!owns_scope("creator-contract", "project-b", "run-a"));
        assert!(scope_current("creator-contract", empty_b, "project-b", ""));
        let returned_a = bind_scope("creator-contract", "project-a", "run-a");
        assert!(!scope_current("creator-contract", a, "project-a", "run-a"));
        assert!(scope_current(
            "creator-contract",
            returned_a,
            "project-a",
            "run-a"
        ));
    }
    #[test]
    fn mutation_requires_the_current_project_and_selected_object() {
        let generation = bind_scope("editor-contract", "project-a", "system.json");
        assert!(!scope_current(
            "editor-contract",
            generation,
            "project-b",
            "system.json"
        ));
        assert!(!owns_scope(
            "editor-contract",
            "project-a",
            "another-system.json"
        ));
        assert!(scope_current(
            "editor-contract",
            generation,
            "project-a",
            "system.json"
        ));
        let reloaded = bind_scope("editor-contract", "project-a", "system.json");
        assert!(!scope_current(
            "editor-contract",
            generation,
            "project-a",
            "system.json"
        ));
        assert!(scope_current(
            "editor-contract",
            reloaded,
            "project-a",
            "system.json"
        ));
    }
    #[test]
    fn an_old_completion_cannot_unlock_a_new_operation() {
        bind_pending("pending-contract", 11);
        bind_pending("pending-contract", 12);
        assert!(!finish_pending("pending-contract", 11));
        assert!(yes(&data("pending-contract")));
        assert!(finish_pending("pending-contract", 12));
        assert!(!yes(&data("pending-contract")));
    }
    #[test]
    fn same_path_in_another_bank_does_not_own_a_knowledge_response() {
        let first = bind_scope("knowledge-contract", "project-a", "bank-a");
        let second = bind_scope("knowledge-contract", "project-a", "bank-b");
        assert!(!scope_current(
            "knowledge-contract",
            first,
            "project-a",
            "bank-a"
        ));
        assert!(!scope_current(
            "knowledge-contract",
            second,
            "project-a",
            "bank-a"
        ));
        assert!(scope_current(
            "knowledge-contract",
            second,
            "project-a",
            "bank-b"
        ));
    }
}

#[cfg(test)]
mod scope_transition_tests {
    use super::*;
    #[test]
    fn accepted_cancel_reply_keeps_the_owned_run_ready_for_resume() {
        let first = bind_scope("system-response-contract", "project-a", "run-a");
        assert!(cache_scope_value(
            "system-response-cache",
            "system-response-contract",
            first,
            "project-a",
            "run-a",
            serde_json::json!({"id":"run-a","project_id":"project-a","active":true})
        ));
        let cancelled = bind_scope("system-response-contract", "project-a", "run-a");
        assert!(cache_scope_value(
            "system-response-cache",
            "system-response-contract",
            cancelled,
            "project-a",
            "run-a",
            serde_json::json!({"id":"run-a","project_id":"project-a","active":false,"status":"cancelled"})
        ));
        let saved = data("system-response-cache");
        assert_eq!(saved["status"], "cancelled");
        assert!(scope_current(
            "system-response-contract",
            saved["view_generation"].as_u64().unwrap(),
            "project-a",
            "run-a"
        ));
        let resumed = bind_scope("system-response-contract", "project-a", "run-a");
        assert!(!cache_scope_value(
            "system-response-cache",
            "system-response-contract",
            cancelled,
            "project-a",
            "run-a",
            serde_json::json!({"status":"cancelled"})
        ));
        assert!(cache_scope_value(
            "system-response-cache",
            "system-response-contract",
            resumed,
            "project-a",
            "run-a",
            serde_json::json!({"id":"run-a","project_id":"project-a","active":true,"status":"running"})
        ));
        assert_eq!(data("system-response-cache")["view_generation"], resumed);
    }
    #[test]
    fn global_project_return_invalidates_old_reads_but_held_targets_keep_their_owner() {
        let old = bind_scope("global-project-contract", "a", "run");
        assert!(project_scope_transition(
            "global-project-contract",
            "a",
            "b",
            false
        ));
        assert!(project_scope_transition(
            "global-project-contract",
            "b",
            "a",
            false
        ));
        assert!(!scope_current("global-project-contract", old, "a", "run"));
        let held = bind_scope("global-project-contract", "a", "run");
        assert!(!project_scope_transition(
            "global-project-contract",
            "a",
            "b",
            true
        ));
        assert!(scope_current("global-project-contract", held, "a", "run"));
        assert!(!project_scope_transition(
            "global-project-contract",
            "a",
            "a",
            false
        ));
        assert!(scope_current("global-project-contract", held, "a", "run"));
    }
}

#[cfg(test)]
mod project_reentry_tests {
    use super::*;
    #[test]
    fn released_reentry_follows_registered_sidebar_and_invalidates_the_old_target() {
        let projects = vec![serde_json::json!({"id":"a"}), serde_json::json!({"id":"b"})];
        let held = bind_scope("reentry-contract", "a", "system.json");
        // Save can release a held target while the sidebar still names another project.
        // Plan/New change and internal tabs must retain that explicit target.
        for previous in ["systems", "system-changes"] {
            assert!(
                project_for_reentry("reentry-contract", "a", "b", false, &projects, previous)
                    .is_none()
            );
            assert!(scope_current("reentry-contract", held, "a", "system.json"));
        }
        assert!(
            project_for_reentry("reentry-contract", "a", "b", true, &projects, "memory-use")
                .is_none()
        );
        assert!(scope_current("reentry-contract", held, "a", "system.json"));
        assert_eq!(
            project_for_reentry("reentry-contract", "a", "b", false, &projects, "memory-use")
                .as_deref(),
            Some("b")
        );
        assert!(!scope_current("reentry-contract", held, "a", "system.json"));
        let adopted = read(|state| state.epochs["reentry-contract"]);
        assert!(scope_current("reentry-contract", adopted, "b", ""));
        assert!(project_for_reentry(
            "reentry-contract",
            "b",
            "missing",
            false,
            &projects,
            "memory-use"
        )
        .is_none());
        assert!(
            project_for_reentry("reentry-contract", "b", "b", false, &projects, "memory-use")
                .is_none()
        );
        assert!(scope_current("reentry-contract", adopted, "b", ""));
    }
}

#[cfg(test)]
mod omnigraph_model_tests {
    use super::*;
    #[test]
    fn omnigraph_session_helpers_detect_enabled_and_branch() {
        assert!(!omnigraph_enabled(&serde_json::json!({"id": "s1"})));
        assert!(!omnigraph_enabled(
            &serde_json::json!({"omnigraph": {"enabled": false}})
        ));
        assert!(!omnigraph_enabled(
            &serde_json::json!({"omnigraph": {"enabled": false, "branch": "main"}})
        ));
        assert_eq!(label("omnigraph"), "Omnigraph");
        assert_eq!(label("cut_total"), "cut total");
        let active = serde_json::json!({"omnigraph": {"enabled": true, "branch": "session/s2"}});
        assert!(omnigraph_enabled(&active));
        assert_eq!(omnigraph_branch(&active), "session/s2");
        assert_eq!(
            omnigraph_branch(&serde_json::json!({"omnigraph": {"enabled": true}})),
            "main"
        );

        let shared = serde_json::json!({
            "id": "s3",
            "omnigraph": {
                "shared_space": "shared/sprint-auth",
                "linked_session_ids": ["s1", " s2 ", "s1", ""]
            }
        });
        assert!(omnigraph_enabled(&shared));
        assert_eq!(omnigraph_shared_space(&shared), "shared/sprint-auth");
        assert_eq!(omnigraph_linked_session_ids(&shared), vec!["s1", "s2"]);
    }

    #[test]
    fn state_default_and_upsert_sessions_keep_selected_in_sync() {
        let state = State::default();
        assert_eq!(state.view, "sessions");
        assert!(state.sessions.is_empty());
        assert!(state.selected.is_null());

        write(|s| {
            s.sid = "sess-2".into();
            s.selected = serde_json::json!({"id": "sess-2", "title": "Old"});
            s.sessions = vec![
                serde_json::json!({"id": "sess-1", "title": "One"}),
                serde_json::json!({"id": "sess-2", "title": "Old"}),
            ];
        });
        upsert_sessions(&[
            serde_json::json!({"id": "sess-2", "title": "Updated", "omnigraph": {"shared_space": "shared/alpha"}}),
            serde_json::json!({"id": "sess-3", "title": "Three"}),
        ]);
        read(|s| {
            assert_eq!(s.selected["title"], "Updated");
            assert_eq!(s.selected["omnigraph"]["shared_space"], "shared/alpha");
            assert!(s.sessions.iter().any(|item| item["id"] == "sess-3"));
            assert_eq!(
                s.sessions
                    .iter()
                    .find(|item| item["id"] == "sess-2")
                    .unwrap()["title"],
                "Updated"
            );
        });
    }

    #[test]
    fn workspace_project_helpers_and_approval_rules_parse_cleanly() {
        set_current_project("proj-alpha");
        assert_eq!(current_project(), "proj-alpha");
        assert_eq!(scoped_project("project"), "proj-alpha");
        assert_eq!(scoped_project("system-project"), "proj-alpha");

        set_scoped_project("system-project", "proj-beta");
        assert_eq!(scoped_project("system-project"), "proj-beta");
        assert_eq!(scoped_project("skills-project"), "proj-alpha");

        clear_scoped_project("system-project");
        assert_eq!(scoped_project("system-project"), "proj-alpha");

        let from_str = ApprovalRule::from_json(&serde_json::json!("Bash(php -l)")).unwrap();
        assert_eq!(from_str.tool, "Bash");
        assert_eq!(from_str.pattern, "php -l");
        assert_eq!(from_str.key, "Bash(php -l)");

        let from_obj = ApprovalRule::from_json(&serde_json::json!({
            "key": "write:src/*",
            "tool": "Write",
            "pattern": "src/*",
            "scope": "session"
        }))
        .unwrap();
        assert_eq!(from_obj.key, "write:src/*");
        assert_eq!(from_obj.tool, "Write");
        assert_eq!(from_obj.pattern, "src/*");
    }

    #[test]
    fn prune_events_prefix_evicts_oldest_and_frees_seen_ids() {
        let mut events = (1..=5)
            .map(|id| serde_json::json!({"id": id, "kind": "text", "text": format!("msg {id}")}))
            .collect::<Vec<_>>();
        let mut seen: BTreeSet<String> = (1..=5).map(|id| id.to_string()).collect();

        let (drained, oldest, any_num) = prune_events_prefix(&mut events, &mut seen, 3);
        assert_eq!(drained.len(), 3);
        assert_eq!(oldest, Some(4));
        assert!(any_num);
        assert_eq!(events.len(), 2);
        assert!(!seen.contains("1") && !seen.contains("3"));
        assert!(seen.contains("4") && seen.contains("5"));
    }
}
