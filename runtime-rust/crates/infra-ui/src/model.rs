use serde_json::Value as V;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};
use web_sys::AbortController;
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
        }
    }
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
    s.replace(['_', '-'], " ")
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
