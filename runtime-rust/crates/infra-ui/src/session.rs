use crate::{api, commands, dom, inbox, knowledge, management, model as m, run, systems};
use serde_json::{json, Value as V};
use std::cell::RefCell;
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};
thread_local! {static FILES:RefCell<Vec<web_sys::File>>=const { RefCell::new(Vec::new()) };}
const PREFS: &str = "harness.sessions.preferences.v1";
const PREF_FIELDS: &[&str] = &[
    "provider",
    "workflow",
    "mode",
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
];
pub fn initialize() {
    for prefix in ["session-budgets", "creator-budgets", "creator-run-budgets"] {
        budget_editor(prefix);
    }
    for (role, label) in [("plan", "Planning"), ("edit", "Editing")] {
        for (suffix, title, tag) in [
            ("model-choice", format!("{label} model"), "select"),
            ("model", format!("{label} custom model ID"), "input"),
            (
                "thinking-effort",
                format!("{label} thinking effort"),
                "select",
            ),
        ] {
            let field = dom::el("label", "field", &title);
            let input = dom::el(tag, "", "");
            let _ = input.set_attribute("id", &format!("routing-{role}-{suffix}"));
            let _ = input.set_attribute("form", "session-form");
            if suffix == "model" {
                let _ = field.set_attribute("id", &format!("routing-{role}-custom-field"));
                let _ = input.set_attribute("maxlength", "120");
            }
            dom::add(&field, &input);
            dom::append("model-routing-fields", &field);
        }
    }
    let preference = dom::storage("harness.theme.v1").unwrap_or_else(|| "system".into());
    apply_theme(&preference);
    m::set("run-preferences", dom::json_storage("harness.run-view.v1"));
    let c = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
        apply_theme(&dom::storage("harness.theme.v1").unwrap_or_else(|| "system".into()));
    });
    let _ = dom::window().add_event_listener_with_callback("storage", c.as_ref().unchecked_ref());
    c.forget();
    if let Ok(Some(media)) = dom::window().match_media("(prefers-color-scheme: dark)") {
        let c = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
            apply_theme(&dom::storage("harness.theme.v1").unwrap_or_else(|| "system".into()))
        });
        let _ = media.add_event_listener_with_callback("change", c.as_ref().unchecked_ref());
        c.forget();
    }
}
fn apply_theme(preference: &str) {
    let dark = dom::window()
        .match_media("(prefers-color-scheme: dark)")
        .ok()
        .flatten()
        .map(|m| m.matches())
        .unwrap_or(false);
    let selected = if preference == "dark" || preference == "light" {
        preference
    } else if dark {
        "dark"
    } else {
        "light"
    };
    if let Some(e) = dom::document().document_element() {
        let _ = e.set_attribute("data-theme", selected);
    }
    for e in dom::all("input[name=theme]") {
        if let Some(i) = e.dyn_ref::<web_sys::HtmlInputElement>() {
            i.set_checked(i.value() == preference);
        }
    }
}
pub async fn bootstrap() {
    dom::hidden("connecting", false);
    dom::disabled("reconnect", true);
    match api::get("/api/bootstrap").await {
        Ok(boot)
            if boot["csrf"].is_string()
                && boot["projects"].is_array()
                && boot["providers"].is_array() =>
        {
            m::set("app-update", boot["update"].clone());
            m::write(|s| {
                s.sessions = m::array(&boot["sessions"]);
                s.boot = boot;
                s.auth_failed = false;
            });
            dom::hidden("connection-banner", true);
            render_update();
            spawn_local(async {
                update_status(true).await;
            });
            populate();
            history();
            restore_preferences();
            spawn_local(async {
                sign_in(false).await;
            });
            let wanted = dom::json_storage(PREFS);
            let sid = m::text(&wanted, "session_id");
            if !sid.is_empty() && m::read(|s| s.sessions.iter().any(|x| m::text(x, "id") == sid)) {
                select(&sid).await;
            }
            let view = dom::hash_view();
            open_view(if view.is_empty() { "sessions" } else { &view }, false).await;
        }
        Ok(_) => dom::error(
            "connection-message",
            "The runner returned incomplete workspace configuration.",
        ),
        Err(e) => {
            dom::error("connection-message", &e.message);
            dom::hidden("connection-banner", false);
        }
    }
    dom::hidden("connecting", true);
    dom::disabled("reconnect", false);
    controls();
}
pub fn populate() {
    let boot = m::read(|s| s.boot.clone());
    let projects = m::array(&boot["projects"]);
    for id in [
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
    ] {
        dom::options(id, &projects, m::summary, None);
    }
    dom::options(
        "provider",
        &m::array(&boot["providers"]),
        |p| {
            format!(
                "{}{}",
                m::summary(p),
                if m::yes(&p["available"]) {
                    ""
                } else {
                    " · unavailable"
                }
            )
        },
        None,
    );
    dom::options("workflow", &m::array(&boot["workflows"]), m::summary, None);
    dom::options(
        "sdd-phase",
        &m::array(&boot["sdd_phases"]),
        m::summary,
        None,
    );
    for id in [
        "creator-provider",
        "system-provider",
        "system-discovery-provider",
        "clash-challenger",
    ] {
        dom::options(id, &m::array(&boot["providers"]), m::summary, None);
    }
    let editions = m::array(&boot["accelerators"])
        .into_iter()
        .find(|kit| kit["id"] == "kit2")
        .map(|kit| m::array(&kit["editions"]))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|edition| edition.as_str().map(|id| json!({"id":id,"name":id})))
        .collect::<Vec<_>>();
    for id in ["setup-edition", "setup-attach-edition"] {
        dom::options(id, &editions, m::summary, None);
    }
    dom::clear("fleet-lenses");
    for lens in m::array(&boot["runtime"]["fleet"]["lenses"]) {
        let e = dom::check(
            &m::summary(&lens),
            "fleet-lens",
            &m::text(&lens, "id"),
            true,
        );
        dom::append("fleet-lenses", &e);
    }
    let count = boot["runtime"]["default_agent_count"]
        .as_u64()
        .unwrap_or(3)
        .to_string();
    if dom::value("agent-count").is_empty() {
        dom::set_value("agent-count", &count);
    }
    dom::attr(
        "agent-count",
        "max",
        &boot["runtime"]["max_agents"]
            .as_u64()
            .unwrap_or(40)
            .to_string(),
    );
    dom::text(
        "runtime-label",
        &format!(
            "Local runner · {} active slots",
            boot["runtime"]["max_active"].as_u64().unwrap_or(1)
        ),
    );
    for prefix in ["", "clash-", "routing-plan-", "routing-edit-"] {
        models(prefix, None);
    }
    management::creator_models();
    controls();
    management::accelerators();
}
pub fn models(prefix: &str, requested: Option<String>) {
    let provider = m::provider(&dom::value(if prefix == "clash-" {
        "clash-challenger"
    } else {
        "provider"
    }));
    let metadata = &provider["model_options"];
    let preserve_custom =
        requested.is_none() && dom::value(&format!("{prefix}model-choice")) == "--custom--";
    let old = requested.unwrap_or_else(|| selected_model(prefix).as_str().unwrap_or("").into());
    let mut entries = vec![json!({"id":"","label":"Provider default"})];
    entries.extend(m::array(&metadata["models"]));
    entries.push(json!({"id":"--custom--","label":"Custom model…"}));
    dom::options(
        &format!("{prefix}model-choice"),
        &entries,
        |v| {
            v["label"]
                .as_str()
                .unwrap_or_else(|| v["id"].as_str().unwrap_or(""))
                .into()
        },
        Some(&old),
    );
    let custom = preserve_custom || !old.is_empty() && !entries.iter().any(|x| x["id"] == old);
    if custom {
        dom::set_value(&format!("{prefix}model-choice"), "--custom--");
        dom::set_value(&format!("{prefix}model"), &old);
    }
    let chosen = selected_model(prefix);
    let model = m::array(&metadata["models"])
        .into_iter()
        .find(|x| x["id"] == chosen)
        .unwrap_or(V::Null);
    let efforts = if model["efforts"].is_array() {
        m::array(&model["efforts"])
    } else {
        m::array(&metadata["efforts"])
    };
    let mut options = vec![json!({"id":"","name":"Provider default"})];
    for e in efforts {
        options.push(json!({"id":e,"name":m::label(e.as_str().unwrap_or(""))}));
    }
    dom::options(
        &format!("{prefix}thinking-effort"),
        &options,
        m::summary,
        None,
    );
    dom::hidden(
        &format!("{prefix}custom-field"),
        !custom && dom::value(&format!("{prefix}model-choice")) != "--custom--",
    );
    dom::disabled(
        &format!("{prefix}model"),
        dom::value(&format!("{prefix}model-choice")) != "--custom--",
    );
    if let Some(input) = dom::get(&format!("{prefix}model")) {
        if custom {
            let _ = input.set_attribute("required", "");
        } else {
            let _ = input.remove_attribute("required");
        }
    }
    dom::text("model-hint", metadata["detail"].as_str().unwrap_or(""));
}
fn restore_model_routing(value: &V) {
    dom::set_checked("model-routing-enabled", value.is_object());
    for role in ["plan", "edit"] {
        let prefix = format!("routing-{role}-");
        models(&prefix, Some(m::text(&value[role], "model")));
        dom::set_value(
            &format!("{prefix}thinking-effort"),
            &m::text(&value[role], "thinking_effort"),
        );
    }
}
pub fn selected_model(prefix: &str) -> V {
    let choice = dom::value(&format!("{prefix}model-choice"));
    let v = if choice == "--custom--" {
        dom::value(&format!("{prefix}model")).trim().into()
    } else {
        choice
    };
    if v.is_empty() {
        V::Null
    } else {
        json!(v)
    }
}
pub fn routing() -> V {
    if !dom::checked("model-routing-enabled")
        || dom::value("workflow") == "fleet-review"
        || dom::checked("clash-enabled")
    {
        V::Null
    } else {
        json!({"plan":{"model":selected_model("routing-plan-"),"thinking_effort":nullable(dom::value("routing-plan-thinking-effort"))},"edit":{"model":selected_model("routing-edit-"),"thinking_effort":nullable(dom::value("routing-edit-thinking-effort"))}})
    }
}
pub fn nullable(v: String) -> V {
    if v.is_empty() {
        V::Null
    } else {
        json!(v)
    }
}
pub fn budget_editor(prefix: &str) {
    let panel = dom::el("details", "fleet-report budget-editor", "");
    let _ = panel.set_attribute("open", "");
    dom::add(&panel, &dom::el("summary", "", "Budgets"));
    for agent in [false, true] {
        let box_ = dom::el("div", "page-toolbar", "");
        for (key, label, min, max) in [
            ("usd", "Money (USD)", "0.01", "1000"),
            ("tokens", "Total tokens", "1", "1000000000"),
            ("seconds", "Time (seconds)", "1", "86400"),
        ] {
            let field = dom::el(
                "label",
                "field",
                &format!("{}{label}", if agent { "Per agent · " } else { "" }),
            );
            let input = dom::el("input", "", "");
            let _ = input.set_attribute(
                "id",
                &format!("{prefix}-{}{key}", if agent { "agent-" } else { "" }),
            );
            let _ = input.set_attribute("type", "number");
            let _ = input.set_attribute("min", min);
            let _ = input.set_attribute("max", max);
            let _ = input.set_attribute("step", if key == "usd" { "any" } else { "1" });
            dom::add(&field, &input);
            dom::add(&box_, &field);
        }
        dom::add(&panel, &box_);
    }
    dom::add(
        &panel,
        &dom::button(
            "Fill totals above",
            &format!("{prefix}-agent-apply"),
            V::Null,
        ),
    );
    let err = dom::el("p", "error-text", "");
    let _ = err.set_attribute("id", &format!("{prefix}-error"));
    dom::add(&panel, &err);
    if prefix != "creator-budgets" {
        dom::add(
            &panel,
            &dom::button("Save budgets", &format!("{prefix}-save"), V::Null),
        );
    }
    dom::append(prefix, &panel);
}
pub fn budgets(prefix: &str) -> V {
    let mut v = json!({});
    for k in ["usd", "tokens", "seconds"] {
        v[k] = dom::num(&format!("{prefix}-{k}"));
    }
    if prefix == "session-budgets" && dom::value("workflow") == "fleet-review" {
        v["usd"] = dom::num("fleet-budget");
    }
    v
}
fn valid_budgets(v: &V) -> bool {
    ["usd", "tokens", "seconds"].iter().all(|k| {
        v[*k].is_null()
            || m::number(&v[*k])
                .map(|n| match *k {
                    "usd" => (0.01..=1000.).contains(&n),
                    "tokens" => (1.0..=1000000000.).contains(&n) && n.fract() == 0.,
                    _ => (1.0..=86400.).contains(&n) && n.fract() == 0.,
                })
                .unwrap_or(false)
    })
}
pub fn restore_budgets(prefix: &str, v: &V) {
    for k in ["usd", "tokens", "seconds"] {
        dom::set_value(&format!("{prefix}-{k}"), &m::string(&v[k]));
    }
}
fn split_budgets(prefix: &str) {
    if !["usd", "tokens", "seconds"]
        .iter()
        .all(|key| dom::numeric_valid(&format!("{prefix}-agent-{key}")))
    {
        dom::error(
            &format!("{prefix}-error"),
            "Use valid per-agent money, token and time limits.",
        );
        return;
    }
    let count = dom::num(if prefix == "session-budgets" {
        "agent-count"
    } else {
        "creator-count"
    })
    .as_f64()
    .unwrap_or(1.0);
    let fleet = prefix == "session-budgets" && dom::value("workflow") == "fleet-review";
    let total = if fleet {
        dom::selected_checks("fleet-lenses").len() as f64
    } else if dom::checked(if prefix == "session-budgets" {
        "agents-enabled"
    } else {
        "creator-agents"
    }) {
        1. + count
    } else {
        1.
    };
    let mut v = json!({});
    for k in ["usd", "tokens", "seconds"] {
        let n = dom::num(&format!("{prefix}-agent-{k}"));
        v[k] = m::number(&n)
            .map(|n| {
                json!(if k == "seconds" {
                    n * if fleet {
                        (total / count.max(1.)).ceil()
                    } else {
                        1.
                    }
                } else {
                    n * total
                })
            })
            .unwrap_or(V::Null);
    }
    if valid_budgets(&v) {
        restore_budgets(prefix, &v);
        if fleet {
            dom::set_value("fleet-budget", &m::string(&v["usd"]));
        }
    } else {
        dom::error(
            &format!("{prefix}-error"),
            "The per-agent totals exceed the supported budget limits.",
        );
    }
}
pub fn controls() {
    let inbox_retry = inbox::needs_confirmation();
    let (sid, session, locked, ready) = m::read(|s| {
        (
            s.sid.clone(),
            s.selected.clone(),
            s.pending || m::active(&s.selected) || inbox_retry,
            !s.boot.is_null() && !s.auth_failed,
        )
    });
    let provider = m::provider(&dom::value("provider"));
    let workflow = dom::value("workflow");
    let fleet = workflow == "fleet-review";
    let sdd = workflow == "sdd";
    let feature = dom::value("sdd-feature").trim().to_owned();
    let phase = dom::value("sdd-phase");
    let phases = m::read(|s| m::array(&s.boot["sdd_phases"]));
    let sdd_valid = !sdd || m::sdd_settings_valid(&feature, &phase, &phases);
    if sdd {
        dom::set_value("mode", m::sdd_mode(&phase));
    } else if ["plan", "review", "fleet-review"].contains(&workflow.as_str()) {
        dom::set_value("mode", "plan");
    } else if !sid.is_empty() && session["workflow"] == "native" && routing().is_null() {
        dom::set_value("mode", &m::text(&session, "mode"));
    }
    let dry = fleet && dom::checked("fleet-dry-run");
    let prompt = dom::value("prompt");
    let count = dom::num("agent-count").as_u64().unwrap_or(0);
    let max = m::read(|s| s.boot["runtime"]["max_agents"].as_u64().unwrap_or(40));
    let helper_valid = !dom::checked("agents-enabled") || (1..=max).contains(&count);
    let model = selected_model("");
    let model_valid = model
        .as_str()
        .map(|x| x.len() <= 120 && !x.starts_with('-') && !x.chars().any(char::is_control))
        .unwrap_or(true);
    let inbox_active = inbox::eligible(&session);
    let valid = ready
        && !m::read(|s| s.pending)
        && if inbox_retry {
            true
        } else if m::active(&session) {
            inbox::can_send(&session, &prompt) && FILES.with(|files| files.borrow().is_empty())
        } else {
            ready
                && !locked
                && helper_valid
                && model_valid
                && sdd_valid
                && budget_fields_valid("session-budgets")
                && valid_budgets(&budgets("session-budgets"))
                && !prompt.trim().is_empty()
                && (dry || m::yes(&provider["available"]))
                && session["workflow"] != "fleet-review"
        };
    dom::disabled("send", !valid);
    dom::disabled(
        "cancel-session",
        !m::active(&session) || m::read(|s| s.pending),
    );
    dom::hidden("welcome", !sid.is_empty());
    dom::text(
        "session-status",
        session["status"].as_str().unwrap_or("New session"),
    );
    dom::text(
        "send-label",
        if m::read(|s| s.pending) {
            "Sending…"
        } else if inbox_retry && inbox::unreadable() {
            "Clear saved state"
        } else if inbox_retry {
            "Retry saving"
        } else if inbox_active {
            "Queue next message"
        } else if sid.is_empty() {
            "Send"
        } else {
            "Send follow-up"
        },
    );
    for id in [
        "project",
        "provider",
        "workflow",
        "workspace",
        "worktree-branch",
        "existing-worktree",
    ] {
        dom::disabled(id, !ready || locked || !sid.is_empty());
    }
    for id in [
        "model-choice",
        "thinking-effort",
        "agents-enabled",
        "agent-count",
        "mode",
        "prompt",
        "attach-files",
    ] {
        dom::disabled(id, !ready || locked);
    }
    dom::disabled(
        "prompt",
        !ready || m::read(|s| s.pending) || inbox_retry || (m::active(&session) && !inbox_active),
    );
    dom::attr(
        "prompt",
        "placeholder",
        if inbox_active {
            "Write the next message…"
        } else {
            "Describe a task or follow-up…"
        },
    );
    if inbox_active || inbox_retry {
        commands::close();
    }
    dom::hidden("fleet-settings", !fleet);
    let sdd_locked = !ready || locked || session["status"] == "awaiting_context";
    dom::hidden("sdd-settings", !sdd);
    dom::disabled("sdd-feature", !sdd || sdd_locked || !sid.is_empty());
    dom::disabled("sdd-phase", !sdd || sdd_locked);
    if let Some(input) = dom::get("sdd-feature") {
        if sdd {
            let _ = input.set_attribute("required", "");
        } else {
            let _ = input.remove_attribute("required");
        }
    }
    dom::disabled(
        "mode",
        !ready
            || locked
            || ["plan", "review", "fleet-review", "sdd"].contains(&workflow.as_str())
            || !sid.is_empty() && routing().is_null(),
    );
    let description = phases
        .iter()
        .find(|entry| entry["id"] == phase)
        .and_then(|entry| entry["description"].as_str())
        .unwrap_or("");
    dom::text(
        "sdd-hint",
        &format!(
        "{description} Documents: specs/{}/. Feature stays fixed; phase can change between turns.",
        if feature.is_empty() {"<feature>"} else {&feature}
    ),
    );
    dom::error(
        "sdd-error",
        if sdd && !sdd_valid && (!feature.is_empty() || !prompt.trim().is_empty()) {
            "Enter a feature slug using lowercase letters, digits and single hyphens; memory is reserved."
        } else {
            ""
        },
    );
    if m::data("sdd-document-session") != sid {
        m::next("sdd-documents");
        m::set("sdd-document-session", json!(sid));
        m::set("sdd-documents-pending", json!(false));
        dom::clear("sdd-documents");
    }
    dom::disabled(
        "sdd-refresh",
        !sdd || !session["sdd"].is_object()
            || sdd_locked
            || m::yes(&m::data("sdd-documents-pending")),
    );
    dom::hidden("clash-settings", !dom::checked("clash-enabled"));
    dom::hidden(
        "worktree-branch-field",
        dom::value("workspace") != "worktree",
    );
    dom::hidden(
        "existing-worktree-field",
        dom::value("workspace") != "existing-worktree",
    );
    dom::hidden(
        "custom-model-field",
        dom::value("model-choice") != "--custom--",
    );
    dom::disabled(
        "model",
        locked || dom::value("model-choice") != "--custom--",
    );
    let route_enabled = !routing().is_null();
    dom::hidden(
        "model-routing-config",
        fleet || dom::checked("clash-enabled"),
    );
    dom::hidden("model-routing-fields", !route_enabled);
    dom::disabled("model-routing-enabled", !ready || locked);
    dom::text(
        "model-routing-summary",
        if route_enabled {
            "Planning + editing"
        } else {
            "One model"
        },
    );
    for role in ["plan", "edit"] {
        let prefix = format!("routing-{role}-");
        let custom = dom::value(&format!("{prefix}model-choice")) == "--custom--";
        for suffix in ["model-choice", "thinking-effort"] {
            dom::disabled(
                &format!("{prefix}{suffix}"),
                !ready || locked || !route_enabled,
            );
        }
        dom::disabled(
            &format!("{prefix}model"),
            !ready || locked || !route_enabled || !custom,
        );
    }
    dom::hidden("brain-link-config", !sid.is_empty());
    dom::error(
        "agent-count-error",
        if helper_valid {
            ""
        } else {
            "Choose a supported whole helper count."
        },
    );
    dom::error(
        "model-error",
        if model_valid {
            ""
        } else {
            "Enter a valid model ID of at most 120 characters."
        },
    );
    dom::error(
        "session-budgets-error",
        if budget_fields_valid("session-budgets") && valid_budgets(&budgets("session-budgets")) {
            ""
        } else {
            "Use valid money, token and time limits."
        },
    );
    dom::text(
        "option-project-value",
        &m::summary(&m::project(&dom::value("project"))),
    );
    dom::text("option-workspace-value", &dom::value("workspace"));
    let helper_count = if dom::checked("agents-enabled") {
        dom::value("agent-count")
    } else {
        "Off".to_string()
    };
    dom::text("option-helpers-value", &helper_count);
    dom::text(
        "option-clash-value",
        if dom::checked("clash-enabled") {
            "On"
        } else {
            "Off"
        },
    );
    dom::hidden("waiting", !m::active(&session));
    dom::text(
        "waiting-text",
        if session["status"] == "queued" {
            "Queued — waiting for an available runner."
        } else {
            "Agent is running."
        },
    );
    knowledge::linked_controls();
    context_meter(&session);
    result_controls();
    merge_controls();
    inbox::render();
    run::render();
}
pub async fn open_view(view: &str, record: bool) {
    let previous_view = m::read(|state| state.view.clone());
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
        "creator",
        "setup",
        "context",
        "memory-use",
        "brain",
        "memory",
        "skills",
        "create-skill",
        "accelerators",
        "kit3",
    ];
    let view = if known.contains(&view) {
        view
    } else {
        "sessions"
    };
    if m::read(|s| ["changes", "checks", "usage"].contains(&s.view.as_str()))
        && !["changes", "checks", "usage"].contains(&view)
    {
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
    if record {
        let _ = dom::window()
            .history()
            .and_then(|h| h.push_state_with_url(&JsValue::NULL, "", Some(&format!("#/{view}"))));
    }
    let group = if is_results {
        "sessions"
    } else if view == "system-changes" {
        "systems"
    } else if ["brain", "memory", "memory-use", "context"].contains(&view) {
        "knowledge"
    } else if view == "create-skill" {
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
        "knowledge" => ("Knowledge", "Your local workspace for agent sessions"),
        "skills" => ("Skills", "Your local workspace for agent sessions"),
        "accelerators" => ("Accelerators", "Your local workspace for agent sessions"),
        _ => ("Sessions", "Your local workspace for agent sessions"),
    };
    dom::text("page-title", title);
    dom::text("page-subtitle", subtitle);
    dom::class("sidebar", "open", false);
    dom::focus("view-heading");
    match view {
        "changes" | "checks" | "usage" => results().await,
        "context" => knowledge::load_context().await,
        "memory" | "brain" => knowledge::load(view, false).await,
        "memory-use" => knowledge::load_memory_use().await,
        "setup" => management::load_setup().await,
        "skills" | "create-skill" => management::load_skills().await,
        "creator" => management::creator_list().await,
        "systems" | "system-changes" => {
            systems::adopt_sidebar_project(&previous_view);
            systems::load().await;
        }
        "accelerators" => management::accelerators(),
        "kit3" => dom::attr("kit3-frame", "src", "/kit3/"),
        _ => load_git().await,
    }
}
fn history_projection(sessions: &[V], sid: &str) -> V {
    V::Array(
        sessions
            .iter()
            .map(|session| {
                let id = m::text(session, "id");
                json!({"id":id,"label":format!("{} · {}",m::summary(session),m::label(session["status"].as_str().unwrap_or(""))),"selected":id==sid})
            })
            .collect(),
    )
}
pub fn history() {
    if dom::get("history").is_none() {
        return;
    }
    let projection = m::read(|s| history_projection(&s.sessions, &s.sid));
    if m::data("history-projection") == projection {
        return;
    }
    dom::clear("history");
    for v in projection.as_array().into_iter().flatten() {
        let id = m::text(v, "id");
        let b = dom::button(&m::text(v, "label"), "session-open", json!(id));
        b.set_class_name("history-item");
        if m::yes(&v["selected"]) {
            let _ = b.set_attribute("aria-current", "true");
        }
        dom::append("history", &b);
    }
    m::set("history-projection", projection);
}
pub async fn refresh() {
    match api::get("/api/sessions").await {
        Ok(v) => {
            m::write(|s| s.sessions = m::array(&v["sessions"]));
            history();
        }
        Err(e) => dom::error("events-error", &e.message),
    }
}
fn preferences_selection(mut prefs: V, project: &str, sid: &str) -> V {
    if !prefs.is_object() {
        prefs = json!({});
    }
    prefs["project_id"] = json!(project);
    prefs["session_id"] = nullable(sid.to_owned());
    prefs
}
fn preferences_update(
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
fn merge_draft_owned(draft: &V, sid: &str, project: &str, epoch: u64) -> bool {
    draft.is_object()
        && sid.is_empty()
        && draft["project_id"] == project
        && draft["selection_epoch"].as_u64() == Some(epoch)
}
fn reset_merge_archive() {
    m::next("merge-archive");
    m::next("merge-archive-scope");
    m::take("delete-armed");
    dom::clear("merge-archive-content");
    dom::hidden("merge-archive-content", true);
    dom::text("merge-delete", "Delete saved context");
    dom::hidden("merge-delete-keep", true);
}
fn budget_request(session: &V, limits: V, timeout: V) -> V {
    let mut request = json!({"budgets":limits,"revision":session["budget_revision"]});
    if session["workflow"] == "fleet-review" || session["fleet"].is_object() {
        request["worker_timeout"] = timeout;
    }
    request
}
fn draft_project(selected: &str) -> String {
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
    let project = draft_project(&dom::value("project"));
    let mut fields = json!({});
    for id in PREF_FIELDS {
        fields[*id] = if [
            "agents-enabled",
            "clash-enabled",
            "fleet-dry-run",
            "brain-link-review",
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
    fields["prompt"] = json!(dom::value("prompt"));
    let sid = m::read(|s| s.sid.clone());
    if let Some(updated) =
        preferences_update(prefs, &project, &sid, fields, &m::data("merge-draft"))
    {
        dom::save(PREFS, Some(&updated.to_string()));
    }
}
fn restore_preferences() {
    let prefs = dom::json_storage(PREFS);
    let project = m::text(&prefs, "project_id");
    if !m::project(&project).is_null() {
        sync_project(&project);
    }
    restore_draft(&dom::value("project"));
}
fn restore_draft(project: &str) {
    m::set("composer-project", json!(project));
    let prefs = dom::json_storage(PREFS);
    let fields = &prefs["drafts"][project];
    if !fields.is_object() {
        return;
    }
    m::write(|s| s.restoring = true);
    for id in PREF_FIELDS {
        if let Some(b) = fields[*id].as_bool() {
            dom::set_checked(id, b);
        } else if let Some(v) = fields[*id].as_str() {
            dom::set_value(id, v);
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
pub fn sync_project(id: &str) {
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
    ] {
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
        let changed = scope
            .is_some_and(|scope| m::project_scope_transition(scope, &dom::value(select), id, held));
        if !held {
            dom::set_value(select, id);
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
}
pub fn new_session(project: Option<&str>) {
    if m::read(|s| s.pending) {
        return;
    }
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
    m::take("merge-draft");
    clear_files();
    dom::clear("events");
    dom::set_value("prompt", "");
    if let Some(p) = project {
        sync_project(p);
    }
    restore_draft(&dom::value("project"));
    let selected = preferences_selection(dom::json_storage(PREFS), &dom::value("project"), "");
    dom::save(PREFS, Some(&selected.to_string()));
    history();
    controls();
    dom::focus("prompt");
    spawn_local(async {
        knowledge::load_brain_link().await;
    });
}
pub async fn select(id: &str) {
    if m::read(|s| s.pending) {
        return;
    }
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
    });
    clear_files();
    dom::clear("events");
    dom::set_value("prompt", "");
    inbox::restore(id);
    run::reset();
    spawn_local(async {
        open_view("sessions", true).await;
    });
    let epoch = m::read(|s| s.epoch);
    poll(epoch).await;
    if !m::read(|s| m::selection_matches(&s.sid, s.epoch, id, epoch)) {
        return;
    }
    let project = m::read(|s| m::text(&s.selected, "project_id"));
    if !project.is_empty() {
        sync_project(&project);
    }
    history();
    save_preferences();
    controls();
}
pub fn stop_poll() {
    dom::cancel_timer("session-poll");
    m::next("session-poll");
    inbox::stop();
    dom::error("inbox-error", "");
}
pub async fn poll(epoch: u64) {
    let (sid, cursor) = m::read(|s| (s.sid.clone(), s.cursor));
    if sid.is_empty() || epoch != m::read(|s| s.epoch) {
        return;
    }
    let path = format!("/api/sessions/{}?after={cursor}", dom::enc(&sid));
    match api::scoped("session-poll", &path).await {
        Ok((e, data)) if m::current("session-poll", e) && epoch == m::read(|s| s.epoch) => {
            if data["session"]["id"] != sid {
                dom::error(
                    "events-error",
                    "The runner returned a different session. Refresh before retrying.",
                );
                return;
            }
            m::upsert(data["session"].clone());
            apply_settings(&data["session"]);
            let events = m::array(&data["events"]);
            let length = events.len();
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
                    run::observe(event);
                }
            }
            inbox::refresh(&sid, epoch).await;
            if !m::current("session-poll", e)
                || !m::read(|s| m::selection_matches(&s.sid, s.epoch, &sid, epoch))
            {
                return;
            }
            history();
            controls();
            dom::error("events-error", "");
            if length == 250 || m::read(|s| m::active(&s.selected)) || inbox::watching(&sid) {
                dom::later(
                    "session-poll",
                    if length == 250 { 0 } else { 1000 },
                    move || {
                        spawn_local(async move {
                            poll(epoch).await;
                        })
                    },
                );
            }
        }
        Ok(_) => {}
        Err(e) if !e.aborted && epoch == m::read(|s| s.epoch) => {
            dom::error("events-error", &e.message);
            if (m::read(|s| m::active(&s.selected)) || inbox::watching(&sid))
                && ![401, 403, 404].contains(&e.status)
            {
                dom::later("session-poll", 2500, move || {
                    spawn_local(async move {
                        poll(epoch).await;
                    })
                });
            }
        }
        Err(_) => {}
    }
}
pub fn apply_settings(v: &V) {
    for (id, key) in [
        ("provider", "provider"),
        ("workflow", "workflow"),
        ("mode", "mode"),
        ("workspace", "workspace"),
        ("agent-count", "agent_count"),
    ] {
        if !v[key].is_null() {
            dom::set_value(id, &m::string(&v[key]));
        }
    }
    dom::set_checked("agents-enabled", m::yes(&v["agents_enabled"]));
    models("", Some(m::text(v, "model")));
    dom::set_value("thinking-effort", &m::text(v, "thinking_effort"));
    restore_model_routing(&v["model_routing"]);
    dom::set_value("sdd-feature", &m::text(&v["sdd"], "feature"));
    dom::set_value("sdd-phase", v["sdd"]["phase"].as_str().unwrap_or("specify"));
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
    dom::text("workspace-session-branch", &m::text(v, "branch"));
    dom::text("workspace-session-path", &m::text(v, "project_path"));
    knowledge::render_linked(v);
    run::outcomes(v);
}
fn base64(data: &[u8]) -> String {
    const C: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | (chunk.get(2).copied().unwrap_or(0) as u32);
        s.push(C[((n >> 18) & 63) as usize] as char);
        s.push(C[((n >> 12) & 63) as usize] as char);
        s.push(if chunk.len() > 1 {
            C[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        s.push(if chunk.len() > 2 {
            C[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    s
}
fn render_files() {
    dom::clear("attachment-list");
    FILES.with(|f| {
        for (index, file) in f.borrow().iter().enumerate() {
            let chip = dom::el(
                "div",
                "attachment-chip",
                &format!("{} · {} bytes", file.name(), file.size()),
            );
            dom::add(&chip, &dom::button("×", "attachment-remove", json!(index)));
            dom::append("attachment-list", &chip);
        }
    });
    dom::hidden("attachment-list", FILES.with(|f| f.borrow().is_empty()));
}
fn clear_files() {
    FILES.with(|f| f.borrow_mut().clear());
    dom::set_value("attachment-input", "");
    render_files();
}
fn add_files() {
    if let Some(input) =
        dom::get("attachment-input").and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        if let Some(files) = input.files() {
            let mut all = FILES.with(|f| f.borrow().clone());
            all.extend((0..files.length()).filter_map(|i| files.item(i)));
            input.set_value("");
            if all.len() > 5
                || all.iter().any(|f| f.size() > 4194304.)
                || all.iter().map(|f| f.size()).sum::<f64>() > 8388608.
            {
                dom::error(
                    "attachment-error",
                    "Choose up to 5 files, at most 4 MiB each and 8 MiB total.",
                );
                return;
            }
            FILES.with(|f| *f.borrow_mut() = all);
            dom::error("attachment-error", "");
            render_files();
        }
    }
}
pub async fn send() {
    if inbox::needs_confirmation() || m::read(|s| m::active(&s.selected)) {
        if FILES.with(|files| !files.borrow().is_empty()) {
            dom::error(
                "composer-error",
                "Remove attachments before queueing a plain-text next message.",
            );
        } else {
            inbox::submit().await;
        }
        return;
    }
    if m::read(|s| s.pending || s.auth_failed || s.boot.is_null() || m::active(&s.selected))
        || !dom::validity("session-form")
    {
        return;
    }
    let draft = m::data("merge-draft");
    let (selected, selection_epoch) = m::read(|s| (s.sid.clone(), s.epoch));
    if draft.is_object()
        && !merge_draft_owned(&draft, &selected, &dom::value("project"), selection_epoch)
    {
        m::take("merge-draft");
        reset_merge_archive();
        controls();
        dom::error(
            "composer-error",
            "The selected project or chat changed. Choose the chats to merge again.",
        );
        return;
    }
    let prompt = dom::value("prompt").trim().to_owned();
    if commands::intercept(&prompt).await {
        return;
    }
    if prompt.is_empty() {
        return;
    }
    if !budget_fields_valid("session-budgets") || !valid_budgets(&budgets("session-budgets")) {
        dom::error(
            "session-budgets-error",
            "Use valid money, token and time limits.",
        );
        return;
    }
    let sid = m::read(|s| s.sid.clone());
    let follow = !sid.is_empty();
    if !follow
        && m::data("merge-draft").is_object()
        && (dom::value("workflow") != "native"
            || dom::value("workspace") != "project"
            || dom::checked("clash-enabled")
            || FILES.with(|f| !f.borrow().is_empty()))
    {
        dom::error("composer-error","A merged task uses an ordinary native chat in the project folder, without attachments or Clash.");
        return;
    }
    if !follow {
        let strict = dom::checked("brain-link-review") || dom::value("brain-link-kind") != "auto";
        let meta = m::data("brain-link-meta");
        if strict && (!m::yes(&meta["runtime_available"]) || meta["mode"] != "governed") {
            dom::error(
                "brain-link-error",
                "Choose a governed knowledge root before preparing a linked task.",
            );
            return;
        }
        if m::yes(&m::data("brain-link-loading")) {
            dom::error(
                "brain-link-error",
                "Wait for knowledge roots to finish loading.",
            );
            return;
        }
    }
    let workflow = dom::value("workflow");
    if workflow == "sdd"
        && !m::sdd_settings_valid(
            dom::value("sdd-feature").trim(),
            &dom::value("sdd-phase"),
            &m::read(|s| m::array(&s.boot["sdd_phases"])),
        )
    {
        dom::error(
            "sdd-error",
            "Choose a valid feature slug and SDD phase before sending.",
        );
        return;
    }
    let dry = workflow == "fleet-review" && dom::checked("fleet-dry-run");
    let mut turn = json!({"prompt":prompt,"agents_enabled":dom::checked("agents-enabled"),"agent_count":dom::num("agent-count"),"model_routing":routing(),"model":if dry{V::Null}else{selected_model("")},"thinking_effort":if dry{V::Null}else{nullable(dom::value("thinking-effort"))}});
    if follow && workflow == "native" && !routing().is_null() {
        turn["mode"] = json!(dom::value("mode"));
    }
    if workflow == "sdd" {
        turn["sdd"] =
            json!({"feature":dom::value("sdd-feature").trim(),"phase":dom::value("sdd-phase")});
    }
    if dom::checked("clash-enabled") {
        turn["clash"] = json!({"challenger":dom::value("clash-challenger"),"rounds":dom::num("clash-rounds"),"challenger_model":selected_model("clash-"),"challenger_thinking_effort":nullable(dom::value("clash-thinking-effort"))});
    } else if follow {
        turn["clash"] = V::Null;
    }
    if !follow {
        for (k, v) in [
            ("budgets", budgets("session-budgets")),
            ("project_id", json!(dom::value("project"))),
            ("provider", json!(dom::value("provider"))),
            ("mode", json!(dom::value("mode"))),
            ("workflow", json!(workflow)),
            ("project_context", json!(true)),
            ("workspace", json!(dom::value("workspace"))),
        ] {
            turn[k] = v;
        }
        if dom::value("workspace") == "worktree" && !dom::value("worktree-branch").trim().is_empty()
        {
            turn["worktree_branch"] = json!(dom::value("worktree-branch").trim());
        }
        if dom::value("workspace") == "existing-worktree" {
            turn["worktree_id"] = json!(dom::value("existing-worktree"));
        }
        if workflow == "fleet-review" {
            turn["fleet"] = json!({"lenses":dom::selected_checks("fleet-lenses"),"dry_run":dom::checked("fleet-dry-run"),"budget_usd":dom::num("fleet-budget"),"worker_timeout":dom::num("fleet-worker-timeout")});
        }
        if let Some(brain) = knowledge::brain_config() {
            turn["brain"] = brain;
        }
    }
    m::write(|s| s.pending = true);
    controls();
    let files = FILES.with(|f| f.borrow().clone());
    let mut encoded = vec![];
    for file in files {
        match JsFuture::from(file.array_buffer()).await {
            Ok(a) => encoded.push(
                json!({"name":file.name(),"data":base64(&js_sys::Uint8Array::new(&a).to_vec())}),
            ),
            Err(_) => {
                dom::error(
                    "attachment-error",
                    &format!("Could not read {}. Attach it again.", file.name()),
                );
                m::write(|s| s.pending = false);
                controls();
                return;
            }
        }
    }
    if !encoded.is_empty() {
        turn["attachments"] = json!(encoded);
    }
    let draft = m::data("merge-draft");
    let path;
    if !follow && draft.is_object() {
        let signature = turn.to_string();
        let mut draft = draft;
        if draft["signature"].is_string() && draft["signature"] != signature {
            draft["request_id"] = json!(m::request_identity());
        }
        draft["signature"] = json!(signature);
        let body = json!({"source_ids":draft["sources"],"request_id":draft["request_id"],"destination":turn});
        m::set("merge-draft", draft);
        turn = body;
        path = "/api/sessions/merge".into();
    } else {
        path = if follow {
            format!("/api/sessions/{}/messages", dom::enc(&sid))
        } else {
            "/api/sessions".into()
        };
    }
    m::write(|s| s.pending = true);
    controls();
    dom::error("composer-error", "");
    match api::post(&path, turn).await {
        Ok(v) => {
            let session = &v["session"];
            if !follow && !session["id"].is_string() {
                dom::error(
                    "composer-error",
                    "No session returned. Refresh the list before retrying.",
                );
            } else {
                m::upsert(session.clone());
                if follow {
                    inbox::clear_draft(&sid, &prompt);
                }
                dom::set_value("prompt", "");
                clear_files();
                m::write(|s| s.pending = false);
                if follow {
                    let e = m::read(|s| s.epoch);
                    poll(e).await;
                } else {
                    m::take("merge-draft");
                    select(&m::text(session, "id")).await;
                }
            }
        }
        Err(e) => dom::error("composer-error", &e.message),
    }
    m::write(|s| s.pending = false);
    controls();
}
pub async fn session_action(action: &str, body: V, error: &str) {
    let sid = m::read(|s| s.sid.clone());
    if sid.is_empty() || m::read(|s| s.pending) {
        return;
    }
    stop_poll();
    m::write(|s| s.pending = true);
    controls();
    match api::post(
        &format!("/api/sessions/{}/{}", dom::enc(&sid), action),
        body,
    )
    .await
    {
        Ok(v) => {
            if v["session"].is_object() {
                m::upsert(v["session"].clone());
            }
        }
        Err(e) => dom::error(error, &e.message),
    }
    m::write(|s| s.pending = false);
    let e = m::read(|s| s.epoch);
    poll(e).await;
    controls();
}
pub async fn load_git() {
    let p = dom::value("project");
    if p.is_empty() {
        return;
    }
    for kind in ["git", "worktrees"] {
        let path = format!("/api/projects/{}/{kind}", dom::enc(&p));
        if let Ok((e, v)) = api::scoped(&format!("project-{kind}"), &path).await {
            if !m::current(&format!("project-{kind}"), e) || dom::value("project") != p {
                continue;
            }
            m::set(&format!("project-{kind}"), v.clone());
            if kind == "git" {
                dom::text("project-git-status", &m::summary(&v));
            } else {
                dom::options(
                    "existing-worktree",
                    &m::array(&v["worktrees"]),
                    |v| format!("{} · {}", m::text(v, "branch"), m::text(v, "path")),
                    None,
                );
            }
        }
    }
    knowledge::load_brain_link().await;
    management::load_attachment().await;
}
pub async fn sign_in(refresh: bool) {
    if m::read(|s| s.boot.is_null() || s.auth_failed) || m::yes(&m::data("sign-in-pending")) {
        return;
    }
    let fresh = if refresh { 3000. } else { 30000. };
    if m::data("sign-in-at")
        .as_f64()
        .is_some_and(|at| js_sys::Date::now() - at < fresh)
    {
        return;
    }
    m::set("sign-in-pending", json!(true));
    let path = format!(
        "/api/providers/sign-in{}",
        if refresh { "?refresh=1" } else { "" }
    );
    if let Ok(mut v) = api::get(&path).await {
        let mut states = json!({});
        for provider in m::array(&v["providers"]) {
            if let Some(id) = provider["id"].as_str() {
                states[id] = provider.clone();
            }
        }
        v["states"] = states;
        let chosen = dom::value("provider");
        let state = &v["states"][&chosen];
        let note = if state["state"] == "signed_out" {
            format!(
                "{} is not signed in. Sign in from a terminal with {}, then send again.",
                m::summary(&m::provider(&chosen)),
                state["login"].as_str().unwrap_or("the provider CLI")
            )
        } else {
            String::new()
        };
        dom::text("provider-hint", &note);
        m::set("sign-in", v);
        m::set("sign-in-at", json!(js_sys::Date::now()));
    }
    m::set("sign-in-pending", json!(false));
}
// Updates of this clone, offered as desktop applications offer theirs: an Update button in the
// header while the branch the clone follows has moved on. A click fast-forwards the clone and
// restarts the server, and the page reloads itself on the new version. "app-update" holds the
// server's status (updates.rs), "app-update-page" what this page did: pending, restarting,
// failed, its note and the server instance that answered before the update.
pub async fn update_status(check: bool) {
    let page = m::data("app-update-page");
    if !(m::yes(&page["pending"])
        || m::yes(&page["restarting"])
        || m::read(|s| s.boot.is_null() || s.auth_failed))
    {
        if let Ok(v) = api::get(&format!(
            "/api/app/update{}",
            if check { "?check=1" } else { "" }
        ))
        .await
        {
            m::set("app-update", v);
            render_update();
        }
        if check {
            // A check fetches in the background: its answer is read once it has had time to arrive.
            dom::later("app-update-read", 8000, || {
                spawn_local(async {
                    update_status(false).await;
                })
            });
        }
    }
    dom::later("app-update", 600000, || {
        spawn_local(async {
            update_status(true).await;
        })
    });
}
#[derive(Debug, PartialEq)]
struct UpdateView {
    button: bool,
    busy: bool,
    label: &'static str,
    title: String,
    note: String,
    error: bool,
}
fn update_view(status: &V, page: &V) -> UpdateView {
    let state = status["state"].as_str().unwrap_or("");
    let (pending, restarting) = (m::yes(&page["pending"]), m::yes(&page["restarting"]));
    let mut title = vec![];
    if state == "available" {
        let commits = m::array(&status["commits"]);
        let shown = commits.len().min(10);
        title.push(m::text(status, "detail"));
        title.extend(
            commits[..shown]
                .iter()
                .map(|commit| format!("• {}", m::text(commit, "subject"))),
        );
        let more = status["behind"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(shown as u64);
        if more > 0 {
            title.push(format!("… and {more} more"));
        }
    }
    let note = m::text(page, "note");
    let (note, error) = if !note.is_empty() {
        (note, m::yes(&page["failed"]))
    } else if ["diverged", "failed", "restart-required"].contains(&state) {
        (m::text(status, "detail"), state != "diverged")
    } else {
        (String::new(), false)
    };
    UpdateView {
        // A restart the server refused is pressed again once the clone is rebuilt.
        button: pending || restarting || ["available", "restart-required"].contains(&state),
        busy: pending || restarting,
        label: if restarting {
            "Restarting…"
        } else if pending {
            "Updating…"
        } else {
            "Update"
        },
        title: title
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        note,
        error,
    }
}
/// The pieces of a server note, `true` for a command it quotes in backticks.
fn code_spans(note: &str) -> Vec<(bool, &str)> {
    let pieces = note.split('`').collect::<Vec<_>>();
    if pieces.len() % 2 == 0 {
        return vec![(false, note)];
    }
    pieces
        .into_iter()
        .enumerate()
        .filter(|(_, piece)| !piece.is_empty())
        .map(|(index, piece)| (index % 2 == 1, piece))
        .collect()
}
fn render_update() {
    let view = update_view(&m::data("app-update"), &m::data("app-update-page"));
    dom::hidden("app-update", !view.button);
    dom::disabled("app-update", view.busy);
    dom::text("app-update", view.label);
    dom::attr("app-update", "title", &view.title);
    // Rewritten only when it changes, so the status region is not announced again.
    if let Some(note) = dom::get("app-update-note") {
        if note.get_attribute("data-note").as_deref() != Some(view.note.as_str()) {
            note.set_text_content(None);
            for (code, piece) in code_spans(&view.note) {
                if code {
                    dom::add(&note, &dom::el("code", "", piece));
                } else {
                    let _ = note.append_with_str_1(piece);
                }
            }
            let _ = note.set_attribute("data-note", &view.note);
        }
    }
    dom::hidden("app-update-banner", view.note.is_empty());
    dom::class("app-update-banner", "error", view.error);
}
async fn apply_update() {
    let page = m::data("app-update-page");
    if m::yes(&page["pending"]) || m::yes(&page["restarting"]) {
        return;
    }
    m::set("app-update-page", json!({"pending":true}));
    render_update();
    // The restarted server answers health with another instance.
    let result = match api::get("/api/health").await {
        Ok(health) => api::post("/api/app/update", json!({}))
            .await
            .map(|v| (health["instance"].clone(), v)),
        Err(error) => Err(error),
    };
    let mut status = m::data("app-update");
    let page = match result {
        Ok((_, v)) if m::yes(&v["current"]) => {
            status["state"] = "current".into();
            json!({"note":v["detail"]})
        }
        Ok((instance, v)) => {
            status["state"] = "updated".into();
            let to = m::text(&v, "to");
            let updated = if to.is_empty() {
                "Updated.".to_owned()
            } else {
                format!("Updated to {to}.")
            };
            if m::yes(&v["restarting"]) {
                json!({"restarting":true,"instance":instance,"note":format!("{updated} The Harness is restarting; this page reloads by itself.")})
            } else {
                json!({"note":format!("{updated} Restart the Harness to use it.")})
            }
        }
        Err(error) => json!({"failed":true,"note":error.message}),
    };
    let (restarting, failed) = (m::yes(&page["restarting"]), m::yes(&page["failed"]));
    m::set("app-update", status);
    m::set("app-update-page", page);
    render_update();
    if restarting {
        wait_for_restart(js_sys::Date::now() + 120000.);
    } else if failed {
        // The failure may have changed the server's state, such as a restart it now requires.
        update_status(false).await;
    }
}
fn wait_for_restart(deadline: f64) {
    dom::later("app-restart", 1000, move || {
        spawn_local(async move {
            if let Ok(health) = api::get("/api/health").await {
                if health["instance"].is_string()
                    && health["instance"] != m::data("app-update-page")["instance"]
                {
                    let _ = dom::window().location().reload();
                    return;
                }
            }
            if js_sys::Date::now() < deadline {
                wait_for_restart(deadline);
            } else {
                m::set(
                    "app-update-page",
                    json!({"failed":true,"note":"The Harness did not come back after the update. Start it again from the application icon."}),
                );
                render_update();
            }
        })
    });
}
fn result_view() -> bool {
    m::read(|s| ["changes", "checks", "usage"].contains(&s.view.as_str()))
}
fn reset_results() {
    dom::cancel_timer("results-poll");
    m::next("results");
    m::take("results");
    m::take("delivery");
    invalidate_delivery();
    dom::hidden("delivery-options", true);
    dom::clear("delivery-files");
    dom::set_value("delivery-message", "");
    dom::clear("delivery-source");
    dom::clear("delivery-target");
    dom::text("delivery-status", "");
}
pub async fn results() {
    dom::cancel_timer("results-poll");
    if !result_view() {
        return;
    }
    m::take("delivery");
    invalidate_delivery();
    dom::hidden("delivery-options", true);
    result_controls();
    let sid = m::read(|s| s.sid.clone());
    if sid.is_empty() {
        dom::error("results-error", "Choose a session first.");
        return;
    }
    let key = "results";
    let path = format!("/api/sessions/{}/results", dom::enc(&sid));
    match api::scoped(key, &path).await {
        Ok((e, v)) if m::current(key, e) && m::read(|s| s.sid == sid) && result_view() => {
            m::set("results", v.clone());
            let snapshot = &v["snapshot"];
            dom::text("results-workspace", &m::text(&v, "workspace"));
            dom::hidden(
                "delivery-section",
                m::read(|s| s.selected["workspace"] != "worktree"),
            );
            let checking = m::array(&v["checks"]).iter().any(m::active);
            dom::text(
                "results-status",
                if checking {
                    "Verification is queued or running; agent outcome stays separate."
                } else if m::read(|s| m::active(&s.selected)) {
                    "Agent is active. Refresh the diff after this launch finishes."
                } else {
                    "Verification results are current to the recorded snapshots."
                },
            );
            dom::text(
                "results-base",
                &if snapshot["available"] == true {
                    format!(
                        "Branch: {} · Base: {}{}{}",
                        snapshot["branch"].as_str().unwrap_or("detached HEAD"),
                        m::text(snapshot, "base"),
                        if snapshot["baseline_recorded"] == true {
                            " (first launch)"
                        } else {
                            " (current HEAD; original baseline unavailable)"
                        },
                        if snapshot["preexisting_changes"] == true {
                            " · Workspace already had uncommitted changes at first launch."
                        } else {
                            ""
                        }
                    )
                } else {
                    String::new()
                },
            );
            dom::text(
                "results-diff-note",
                &format!(
                    "{}{}",
                    m::text(snapshot, "message"),
                    if snapshot["available"] == true && snapshot["complete"] != true {
                        " Preview is truncated or omits file contents; verification freshness cannot be certified."
                    } else {
                        ""
                    }
                ),
            );
            let files = m::array(&snapshot["files"])
                .iter()
                .map(|f| format!("{}  {}", m::text(f, "status"), m::text(f, "path")))
                .collect::<Vec<_>>()
                .join("\n");
            dom::text(
                "results-files",
                if files.is_empty() {
                    "No changed files listed."
                } else {
                    &files
                },
            );
            dom::text(
                "results-diff",
                snapshot["diff"]
                    .as_str()
                    .unwrap_or("No text diff available."),
            );
            dom::text("results-usage-note", &m::text(&v, "notice"));
            let totals = &v["totals"];
            let amount = |key: &str, suffix: &str| {
                let t = &totals[key];
                if t["unknown_launches"].as_u64().unwrap_or(0) > 0
                    && t["reported"].as_f64().unwrap_or(0.) == 0.
                {
                    "—".into()
                } else {
                    t["reported"]
                        .as_f64()
                        .map(|n| {
                            if key == "cost_usd" {
                                format!("≈ {}{n:.4}", "$")
                            } else {
                                format!("{n:.0}{suffix}")
                            }
                        })
                        .unwrap_or_else(|| "—".into())
                }
            };
            let unknown = ["tokens", "cost_usd"]
                .iter()
                .filter_map(|key| {
                    totals[*key]["unknown_launches"]
                        .as_u64()
                        .filter(|n| *n > 0)
                        .map(|n| {
                            format!(
                                "{} not reported for {n} launches",
                                if *key == "cost_usd" { "cost" } else { "tokens" }
                            )
                        })
                })
                .collect::<Vec<_>>();
            dom::text(
                "results-totals",
                &format!(
                    "{} · {} · {}{}",
                    amount("tokens", " tokens"),
                    amount("cost_usd", ""),
                    amount("seconds", " s"),
                    if unknown.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", unknown.join(" · "))
                    }
                ),
            );
            let checks = m::array(&v["checks"]);
            dom::keyed_render(
                "results-checks",
                &checks,
                |c| m::text(c, "id"),
                |check| {
                    let d = dom::details(
                        &format!(
                            "{} · {}",
                            m::text(check, "status").to_uppercase(),
                            m::text(check, "command")
                        ),
                        "",
                    );
                    if ["queued", "running", "failed", "timed_out", "output_limit"]
                        .contains(&check["status"].as_str().unwrap_or(""))
                    {
                        let _ = d.set_attribute("open", "");
                    }
                    let current = snapshot["complete"] == true
                        && check["snapshot_complete"] == true
                        && snapshot["id"] == check["snapshot_id"]
                        && check["workspace_changed"] != true;
                    let freshness = if check["kind"] == "target" {
                        format!(
                            "Target {} · tested commit {}{}",
                            m::text(check, "target_branch"),
                            m::text(check, "candidate"),
                            if check["workspace_changed"] == true {
                                " · Check changed the workspace"
                            } else {
                                ""
                            }
                        )
                    } else if current {
                        "Matches the current diff".into()
                    } else if check["workspace_changed"] == true
                        || snapshot["id"].is_string()
                            && check["snapshot_id"].is_string()
                            && snapshot["id"] != check["snapshot_id"]
                    {
                        "Workspace changed since this check".into()
                    } else {
                        "Freshness unavailable".into()
                    };
                    dom::add(
                        &d,
                        &dom::el(
                            "p",
                            "knowledge-note",
                            &format!(
                                "{} · exit code {} · {}s · {freshness}",
                                check["started_at"]
                                    .as_str()
                                    .or_else(|| check["created_at"].as_str())
                                    .unwrap_or("not recorded"),
                                check["exit_code"]
                                    .as_i64()
                                    .map(|n| n.to_string())
                                    .unwrap_or_else(|| "unavailable".into()),
                                check["seconds"]
                                    .as_f64()
                                    .map(|n| n.to_string())
                                    .unwrap_or_else(|| "unknown".into())
                            ),
                        ),
                    );
                    dom::add(
                        &d,
                        &dom::el(
                            "pre",
                            "fleet-report-text",
                            check["output"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .unwrap_or("Waiting for output…"),
                        ),
                    );
                    d
                },
            );
            if checks.is_empty() {
                dom::append(
                    "results-checks",
                    &dom::el("p", "knowledge-note", "No independent checks run yet."),
                );
            }
            dom::rows("results-launches", &m::array(&v["launches"]));
            knowledge::usage(&v);
            delivery_load(&sid).await;
            if !m::current(key, e) || !m::read(|s| s.sid == sid) || !result_view() {
                return;
            }
            result_controls();
            let view = m::read(|s| s.view.clone());
            render_tabs(&view, "sessions");
            if m::read(|s| m::active(&s.selected)) {
                dom::later("results-poll", 1200, || {
                    spawn_local(async {
                        results().await;
                    })
                });
            }
        }
        Ok(_) => {}
        Err(e) if !e.aborted && m::read(|s| s.sid == sid) && result_view() => {
            dom::error("results-error", &e.message)
        }
        Err(_) => {}
    }
}
fn result_controls() {
    let session = m::read(|s| s.selected.clone());
    let locked =
        m::read(|s| s.sid.is_empty() || s.pending || m::active(&s.selected) || s.auth_failed);
    let result = m::data("results");
    let command = dom::value("check-command");
    let timeout = dom::num("check-timeout").as_f64();
    let timeout_valid = timeout.is_some_and(|n| (1.0..=3600.).contains(&n) && n.fract() == 0.);
    dom::disabled(
        "check-start",
        locked
            || !result["snapshot"].is_object()
            || session["creator"].is_object()
            || command.trim().is_empty()
            || command.len() > 4000
            || !timeout_valid,
    );
    dom::disabled("check-command", locked || session["creator"].is_object());
    dom::disabled("check-timeout", locked);
    let delivery = m::data("delivery");
    let preview = m::data("delivery-preview");
    let delivery_locked = locked || delivery["available"] != true;
    let message = dom::value("delivery-message");
    let too_long = message.len() > 4000;
    dom::error(
        "delivery-message-error",
        if too_long {
            "Use a commit message of at most 4,000 UTF-8 bytes."
        } else {
            ""
        },
    );
    for e in dom::all("#delivery-options input,#delivery-options select") {
        if delivery_locked {
            let _ = e.set_attribute("disabled", "");
        } else {
            let _ = e.remove_attribute("disabled");
        }
    }
    dom::disabled(
        "delivery-commit-preview-button",
        delivery_locked
            || dom::selected_checks("delivery-files").is_empty()
            || message.trim().is_empty()
            || too_long,
    );
    let require = dom::checked("delivery-check-required");
    let check = dom::value("delivery-check-command");
    let checktime = dom::num("delivery-check-timeout").as_f64();
    let check_valid = !check.trim().is_empty()
        && check.len() <= 4000
        && checktime.is_some_and(|n| (1.0..=3600.).contains(&n) && n.fract() == 0.);
    dom::disabled("delivery-check-command", delivery_locked || !require);
    dom::disabled("delivery-check-timeout", delivery_locked || !require);
    dom::disabled(
        "delivery-preview-button",
        delivery_locked
            || dom::value("delivery-source").is_empty()
            || dom::value("delivery-target").is_empty()
            || require && !check_valid,
    );
    dom::disabled(
        "delivery-commit",
        delivery_locked || preview["kind"] != "commit" || preview["can_apply"] != true,
    );
    let check = &preview["target_check"];
    dom::hidden("delivery-run-check", preview["check"].is_null());
    dom::disabled(
        "delivery-run-check",
        delivery_locked || preview["check"].is_null() || preview["can_apply"] != true,
    );
    dom::hidden("delivery-cancel-check", !m::active(check));
    dom::disabled("delivery-cancel-check", m::read(|s| s.pending));
    dom::disabled(
        "delivery-apply",
        delivery_locked
            || preview["kind"] != "transfer"
            || preview["can_apply"] != true
            || !preview["check"].is_null()
                && (check["status"] != "passed"
                    || check["workspace_changed"] == true
                    || check["candidate"] != preview["candidate"]),
    );
}
async fn delivery_load(sid: &str) {
    let key = "delivery";
    let response = api::scoped(key, &format!("/api/sessions/{}/delivery", dom::enc(sid))).await;
    if let Ok((epoch, v)) = response {
        if !m::current(key, epoch) || !m::read(|s| s.sid == sid) || !result_view() {
            return;
        }
        m::set("delivery", v.clone());
        dom::hidden(
            "delivery-options",
            !m::yes(&v["available"]) && !v["preview"].is_object(),
        );
        dom::text(
            "delivery-availability",
            v["notice"]
                .as_str()
                .or_else(|| v["reason"].as_str())
                .unwrap_or(""),
        );
        dom::clear("delivery-files");
        for file in m::array(&v["files"]) {
            dom::append(
                "delivery-files",
                &dom::check(
                    &format!("{} · {}", m::text(&file, "path"), m::text(&file, "status")),
                    "delivery-file",
                    &m::text(&file, "path"),
                    false,
                ),
            );
        }
        let mut commits = vec![json!({"id":"","name":"Choose a commit"})];
        commits.extend(m::array(&v["commits"]).into_iter().map(|x|json!({"id":x["sha"],"name":format!("{} · {}",m::text(&x,"sha"),m::text(&x,"subject"))})));
        dom::options("delivery-source", &commits, m::summary, None);
        let mut branches = vec![json!({"id":"","name":"Choose a local branch"})];
        branches.extend(
            m::array(&v["branches"])
                .into_iter()
                .map(|x| json!({"id":x["name"],"name":x["name"]})),
        );
        dom::options("delivery-target", &branches, m::summary, None);
        if v["preview"].is_object() {
            render_delivery(&v["preview"]);
        }
        result_controls();
    }
}
fn render_delivery(v: &V) {
    m::set("delivery-preview", v.clone());
    let prefix = if v["kind"] == "commit" {
        "delivery-commit"
    } else {
        "delivery-preview"
    };
    dom::hidden(
        if prefix == "delivery-commit" {
            "delivery-commit-preview"
        } else {
            "delivery-preview"
        },
        false,
    );
    dom::text(
        &format!("{prefix}-summary"),
        &format!("{} · {}", m::summary(v), m::text(v, "candidate")),
    );
    dom::rows(&format!("{prefix}-files"), &m::array(&v["files"]));
    dom::text(&format!("{prefix}-diff"), &m::text(v, "diff"));
    dom::pretty(&format!("{prefix}-checks"), &v["checks"]);
    dom::text(
        &format!("{prefix}-note"),
        v["notice"]
            .as_str()
            .unwrap_or("Review the files and diff before confirming."),
    );
    dom::disabled(
        "delivery-commit",
        !m::yes(&v["can_apply"]) || v["kind"] != "commit",
    );
    dom::disabled(
        "delivery-apply",
        !m::yes(&v["can_apply"]) || v["kind"] == "commit",
    );
    dom::text("delivery-check-status", &m::summary(&v["target_check"]));
    dom::text(
        "delivery-check-output",
        &m::text(&v["target_check"], "output"),
    );
}
fn invalidate_delivery() {
    m::next("delivery");
    m::take("delivery-preview");
    dom::hidden("delivery-preview", true);
    dom::hidden("delivery-commit-preview", true);
    dom::disabled("delivery-commit", true);
    dom::disabled("delivery-apply", true);
}
async fn delivery(action: &str, body: V) {
    let sid = m::read(|s| s.sid.clone());
    if sid.is_empty() || m::read(|s| s.pending) {
        return;
    }
    m::write(|s| s.pending = true);
    controls();
    invalidate_delivery();
    let mut body = body;
    body["action"] = json!(action);
    match api::post(&format!("/api/sessions/{}/delivery", dom::enc(&sid)), body).await {
        Ok(v) => {
            if action == "check" {
                if v["session"].is_object() {
                    m::upsert(v["session"].clone());
                }
                results().await;
            } else if ["commit", "apply"].contains(&action) {
                dom::text(
                    "delivery-status",
                    &format!(
                        "{} · Commit {}",
                        m::text(&v, "detail"),
                        m::text(&v, "commit")
                    ),
                );
                results().await;
            } else {
                render_delivery(&v);
            }
        }
        Err(e) => dom::error(
            "delivery-error",
            &format!(
                "{} Refresh results and prepare a new preview before retrying.",
                e.message
            ),
        ),
    }
    m::write(|s| s.pending = false);
    controls();
}
fn resize(height: f64) {
    let available = dom::get("sessions-view")
        .map(|e| e.client_height())
        .unwrap_or(500) as f64
        - dom::get("composer-area")
            .map(|e| e.client_height())
            .unwrap_or(0) as f64
        - dom::get("run-view").map(|e| e.client_height()).unwrap_or(0) as f64
        - 100.;
    let h = height.max(80.).min(available.max(80.));
    if let Some(e) =
        dom::get("sessions-view").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = e
            .style()
            .set_property("--configuration-height", &format!("{h}px"));
        let _ = e.class_list().add_1("output-resized");
    }
    m::set("output-height", json!(height));
}
pub fn resize_key(event: &web_sys::KeyboardEvent) {
    if event
        .target()
        .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
        .map(|e| e.id())
        .as_deref()
        != Some("output-resizer")
    {
        return;
    }
    let h = m::data("output-height").as_f64().unwrap_or(200.);
    let next = match event.key().as_str() {
        "ArrowUp" => h - 40.,
        "ArrowDown" => h + 40.,
        "Home" => 80.,
        "End" => dom::get("sessions-view")
            .map(|e| e.client_height())
            .unwrap_or(500) as f64,
        _ => return,
    };
    event.prevent_default();
    resize(next);
}
pub fn resize_event(kind: &str, event: &web_sys::Event) {
    let Some(target) = event
        .target()
        .and_then(|e| e.dyn_into::<web_sys::Element>().ok())
    else {
        return;
    };
    if target.id() != "output-resizer" {
        return;
    }
    if kind == "dblclick" {
        dom::class("sessions-view", "output-resized", false);
        return;
    }
    let Some(p) = event.dyn_ref::<web_sys::PointerEvent>() else {
        return;
    };
    match kind {
        "pointerdown" if p.button() == 0 => {
            let _ = target.set_pointer_capture(p.pointer_id());
            dom::focus("output-resizer");
            event.prevent_default();
        }
        "pointermove" if target.has_pointer_capture(p.pointer_id()) => {
            let top = dom::document()
                .query_selector("#sessions-view .configuration")
                .ok()
                .flatten()
                .map(|e| e.get_bounding_client_rect().top())
                .unwrap_or(0.);
            resize(p.client_y() as f64 - top);
        }
        "pointerup" | "pointercancel" => {
            let _ = target.release_pointer_capture(p.pointer_id());
        }
        _ => {}
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
    if kind == "input" && id == "prompt" {
        save_preferences();
        controls();
        return false;
    }
    if (kind == "input" || kind == "change")
        && (id.starts_with("delivery-") || action == "delivery-file")
    {
        invalidate_delivery();
        result_controls();
        return true;
    }
    if (kind == "input" || kind == "change") && ["check-command", "check-timeout"].contains(&id) {
        result_controls();
        return true;
    }
    if kind == "change" && action == "merge-choice" {
        let n = dom::selected_checks("merge-options").len();
        dom::text("merge-selection-count", &format!("{n} chats selected"));
        dom::disabled("merge-picker-continue", !(2..=8).contains(&n));
        return true;
    }
    if kind == "change" {
        if ["project", "project-switcher"].contains(&id) {
            if value == "__choose-folder__" {
                management::folder_open("session").await;
            } else {
                new_session(Some(value));
                load_git().await;
            }
            return true;
        }
        if [
            "provider",
            "clash-challenger",
            "model-choice",
            "clash-model-choice",
            "routing-plan-model-choice",
            "routing-edit-model-choice",
        ]
        .contains(&id)
        {
            models(
                if id.starts_with("clash-") {
                    "clash-"
                } else if id.starts_with("routing-plan-") {
                    "routing-plan-"
                } else if id.starts_with("routing-edit-") {
                    "routing-edit-"
                } else {
                    ""
                },
                None,
            );
            if id == "provider" {
                for prefix in ["routing-plan-", "routing-edit-"] {
                    models(prefix, Some(String::new()));
                }
            }
            controls();
            save_preferences();
            return true;
        }
        if id == "attachment-input" {
            add_files();
            return true;
        }
        if let Some(e) = dom::get(id) {
            if e.get_attribute("name").as_deref() == Some("theme") {
                dom::save(
                    "harness.theme.v1",
                    if value == "system" { None } else { Some(value) },
                );
                apply_theme(value);
                return true;
            }
        }
        if id == "model-routing-enabled" && checked {
            let current = json!({"model":selected_model(""),"thinking_effort":nullable(dom::value("thinking-effort"))});
            restore_model_routing(&json!({"plan":current,"edit":current}));
        }
        if [
            "workflow",
            "mode",
            "sdd-phase",
            "agents-enabled",
            "agent-count",
            "workspace",
            "clash-enabled",
            "clash-rounds",
            "thinking-effort",
            "model-routing-enabled",
            "brain-link-kind",
            "brain-link-review",
            "fleet-dry-run",
        ]
        .contains(&id)
        {
            controls();
            save_preferences();
            return true;
        }
    }
    if kind == "input"
        && (id.starts_with("session-budgets-")
            || id == "model"
            || id == "agent-count"
            || id == "sdd-feature")
    {
        controls();
        return true;
    }
    if kind == "submit" {
        match id {
            "session-form" => {
                send().await;
                return true;
            }
            "check-form" => {
                let snapshot = m::data("results")["snapshot"].clone();
                session_action("check",json!({"snapshot_id":snapshot["id"],"command":dom::value("check-command").trim(),"timeout":dom::num("check-timeout")}),"results-error").await;
                results().await;
                return true;
            }
            "delivery-commit-form" => {
                delivery("preview_commit",json!({"snapshot_id":m::data("delivery")["snapshot_id"],"paths":dom::selected_checks("delivery-files"),"message":dom::value("delivery-message").trim()})).await;
                return true;
            }
            "delivery-preview-form" => {
                delivery("preview_transfer",json!({"commit":dom::value("delivery-source"),"target_branch":dom::value("delivery-target"),"check":if dom::checked("delivery-check-required"){json!({"command":dom::value("delivery-check-command").trim(),"timeout":dom::num("delivery-check-timeout")})}else{V::Null}})).await;
                return true;
            }
            _ => {}
        }
    }
    if kind != "click" {
        return false;
    }
    if action.ends_with("-agent-apply") {
        split_budgets(repeated_suffix_removed(action, "-agent-apply"));
        controls();
        return true;
    }
    match action {
        "context-meter" => {
            m::take("context-turn-selected");
            m::set("context-page", json!(0));
            open_view("usage", true).await;
        }
        "reconnect" => bootstrap().await,
        "new-session" => {
            new_session(None);
            open_view("sessions", true).await;
        }
        "refresh-sessions" => refresh().await,
        "session-open" => {
            select(payload.as_str().unwrap_or("")).await;
            open_view("sessions", true).await;
        }
        "cancel-session" => session_action("cancel", json!({}), "composer-error").await,
        "inbox-cancel" => inbox::cancel(payload.as_str().unwrap_or("")).await,
        "attach-files" => {
            if let Some(e) =
                dom::get("attachment-input").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                e.click();
            }
        }
        "attachment-remove" => {
            if let Some(index) = payload.as_u64() {
                FILES.with(|f| {
                    if (index as usize) < f.borrow().len() {
                        f.borrow_mut().remove(index as usize);
                    }
                });
                render_files();
            }
        }
        "session-settings-toggle" => {
            let open = dom::get("session-settings")
                .map(|e| e.has_attribute("hidden"))
                .unwrap_or(true);
            dom::hidden("session-settings", !open);
            dom::attr(id, "aria-expanded", if open { "true" } else { "false" });
        }
        "menu-toggle" => {
            let open = dom::get("sidebar")
                .map(|e| !e.class_list().contains("open"))
                .unwrap_or(false);
            dom::class("sidebar", "open", open);
            dom::class("scrim", "open", open);
            dom::attr(
                "menu-toggle",
                "aria-expanded",
                if open { "true" } else { "false" },
            );
        }
        "scrim" => {
            dom::class("sidebar", "open", false);
            dom::class("scrim", "open", false);
            dom::attr("menu-toggle", "aria-expanded", "false");
        }
        "refresh-project-git" => load_git().await,
        "sdd-refresh" => {
            let (sid, selection_epoch) = m::read(|s| (s.sid.clone(), s.epoch));
            let current =
                || m::read(|s| m::selection_matches(&s.sid, s.epoch, &sid, selection_epoch));
            if sid.is_empty() || !m::read(|s| s.selected["sdd"].is_object()) {
                return true;
            }
            m::set("sdd-documents-pending", json!(true));
            controls();
            dom::clear("sdd-documents");
            dom::append(
                "sdd-documents",
                &dom::el("p", "knowledge-note", "Reading documents…"),
            );
            let key = "sdd-documents";
            let response = api::scoped(key, &format!("/api/sessions/{}/sdd", dom::enc(&sid))).await;
            match response {
                Ok((epoch, value)) if m::current(key, epoch) && current() => {
                    dom::clear("sdd-documents");
                    dom::append(
                        "sdd-documents",
                        &dom::el(
                            "p",
                            "knowledge-note",
                            "Document snapshot. Read again after a phase or external edit.",
                        ),
                    );
                    for file in m::array(&value["files"]) {
                        let suffix = if file["available"] != true {
                            " · missing or unreadable"
                        } else if file["truncated"] == true {
                            " · truncated"
                        } else {
                            ""
                        };
                        let title = format!("{}{suffix}", m::text(&file, "path"));
                        dom::append(
                            "sdd-documents",
                            &dom::details(&title, &m::text(&file, "text")),
                        );
                    }
                }
                Err(error) if !error.aborted && current() => {
                    dom::clear("sdd-documents");
                    dom::append("sdd-documents", &dom::el("p", "error-text", &error.message));
                }
                _ => {}
            }
            if current() {
                m::set("sdd-documents-pending", json!(false));
                controls();
            }
        }
        "results-refresh" => results().await,
        "session-budgets-save" => {
            let s = m::read(|s| s.selected.clone());
            if !budget_fields_valid("session-budgets")
                || !valid_budgets(&budgets("session-budgets"))
                || s["fleet"].is_object() && !dom::numeric_valid("fleet-worker-timeout")
            {
                dom::error(
                    "session-budgets-error",
                    "Use valid money, token and time limits.",
                );
                return true;
            }
            session_action(
                "budgets",
                budget_request(
                    &s,
                    budgets("session-budgets"),
                    dom::num("fleet-worker-timeout"),
                ),
                "composer-error",
            )
            .await;
        }
        "fleet-approve" | "fleet-reject" => {
            session_action(
                "decision",
                json!({"approve":action=="fleet-approve"}),
                "fleet-action-error",
            )
            .await
        }
        "fleet-resume" => session_action("resume", json!({}), "fleet-action-error").await,
        "delivery-commit" | "delivery-apply" => {
            let v = m::take("delivery-preview");
            if v["preview_id"].is_string() && m::yes(&v["can_apply"]) {
                delivery(
                    if action == "delivery-commit" {
                        "commit"
                    } else {
                        "apply"
                    },
                    json!({"preview_id":v["preview_id"]}),
                )
                .await;
            }
        }
        "delivery-run-check" => {
            let v = m::data("delivery-preview");
            delivery("check", json!({"preview_id":v["preview_id"]})).await;
        }
        "delivery-cancel-check" => session_action("cancel", json!({}), "delivery-error").await,
        "merge-chats" => merge_picker().await,
        "merge-picker-cancel" => dom::dialog("merge-picker", false),
        "merge-picker-continue" => {
            let ids = dom::selected_checks("merge-options");
            let p = dom::value("project");
            let owner = m::data("merge-picker-owner");
            let epoch = m::read(|s| s.epoch);
            let eligible = m::read(|s| {
                ids.iter().all(|id| {
                    s.sessions
                        .iter()
                        .any(|session| session["id"] == *id && merge_source_eligible(session, &p))
                })
            });
            if !m::read(|s| s.pending)
                && owner["project"] == p
                && owner["epoch"].as_u64() == Some(epoch)
                && (2..=8).contains(&ids.len())
                && eligible
            {
                new_session(Some(&p));
                m::set(
                    "merge-draft",
                    json!({"sources":ids,"request_id":m::request_identity(),"signature":null,"project_id":p,"selection_epoch":m::read(|s|s.epoch)}),
                );
                dom::dialog("merge-picker", false);
                dom::set_value("workflow", "native");
                dom::set_value("workspace", "project");
                dom::set_checked("clash-enabled", false);
                dom::set_value("prompt", crate::model::MERGE_PROMPT);
                merge_controls();
                controls();
            }
        }
        "merge-clear" => {
            m::take("merge-draft");
            reset_merge_archive();
            if dom::value("prompt") == m::MERGE_PROMPT {
                dom::set_value("prompt", "");
            }
            restore_draft(&dom::value("project"));
            controls();
        }
        "merge-restart" => session_action("restart-merge", json!({}), "composer-error").await,
        "merge-delete" => {
            let sid = m::read(|s| s.sid.clone());
            if m::data("delete-armed") == sid {
                m::take("delete-armed");
                session_action("delete-merge-context", json!({}), "composer-error").await;
            } else {
                m::set("delete-armed", json!(sid));
                dom::text("merge-delete", "Delete saved copy");
                dom::hidden("merge-delete-keep", false);
            }
        }
        "merge-delete-keep" => {
            m::take("delete-armed");
            dom::text("merge-delete", "Delete saved context");
            dom::hidden("merge-delete-keep", true);
        }
        "merge-archive" => {
            let (sid, selection_epoch, session) =
                m::read(|s| (s.sid.clone(), s.epoch, s.selected.clone()));
            if sid.is_empty()
                || !session["merge"].is_object()
                || session["merge"]["archive_deleted_at"].is_string()
            {
                return true;
            }
            let generation = m::bind_scope("merge-archive-scope", &sid, "");
            match api::scoped(
                "merge-archive",
                &format!("/api/sessions/{}/merge", dom::enc(&sid)),
            )
            .await
            {
                Ok((request_epoch, value))
                    if m::current("merge-archive", request_epoch)
                        && m::scope_current("merge-archive-scope", generation, &sid, "")
                        && m::read(|s| {
                            m::selection_matches(&s.sid, s.epoch, &sid, selection_epoch)
                        }) =>
                {
                    dom::pretty("merge-archive-content", &value);
                    dom::hidden("merge-archive-content", false);
                }
                Err(error)
                    if !error.aborted
                        && m::scope_current("merge-archive-scope", generation, &sid, "")
                        && m::read(|s| {
                            m::selection_matches(&s.sid, s.epoch, &sid, selection_epoch)
                        }) =>
                {
                    dom::error("composer-error", &error.message)
                }
                _ => {}
            }
        }
        "app-update" => apply_update().await,
        _ if action.starts_with("option-") => {
            let panel = action.trim_start_matches("option-");
            let open = m::data("option-open") != panel;
            m::set("option-open", if open { json!(panel) } else { V::Null });
            for name in [
                "project",
                "helpers",
                "clash",
                "workspace",
                "brain",
                "budgets",
                "models",
            ] {
                dom::hidden(&format!("panel-{name}"), !open || name != panel);
                dom::attr(
                    &format!("option-{name}"),
                    "aria-expanded",
                    if open && name == panel {
                        "true"
                    } else {
                        "false"
                    },
                );
            }
            if open {
                dom::attr(&format!("panel-{panel}"), "tabindex", "-1");
                dom::focus(&format!("panel-{panel}"));
                if panel == "project" {
                    management::load_attachment().await;
                }
            }
            return true;
        }
        _ => return false,
    }
    let _ = checked;
    true
}
fn merge_source_eligible(session: &V, project: &str) -> bool {
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
async fn merge_picker() {
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

fn context_meter(session: &V) {
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
                "skills" => "skills",
                _ => "accelerators",
            }
            .into()
        })
}
fn view_label(view: &str) -> &str {
    match view {
        "sessions" => "Conversation",
        "systems" => "Services",
        "system-changes" | "changes" => "Changes",
        "checks" => "Checks",
        "usage" => "Usage",
        "memory-use" => "Memory use",
        "brain" => "Project Brain",
        "memory" => "Memory bank",
        "context" => "Context files",
        "skills" => "Library",
        "create-skill" => "Create skill",
        "accelerators" => "Overview",
        "creator" => "Infrastructure Creator",
        "kit3" => "Open Source Kit",
        "setup" => "Install into project",
        _ => view,
    }
}
fn render_tabs(view: &str, group: &str) {
    let views: &[&str] = match group {
        "sessions" => &["sessions", "changes", "checks", "usage"],
        "systems" => &["systems", "system-changes"],
        "knowledge" => &["memory-use", "brain", "memory", "context"],
        "skills" => &["skills", "create-skill"],
        _ => &["accelerators", "creator", "kit3", "setup"],
    };
    let sid = m::read(|s| s.sid.clone());
    let views = views
        .iter()
        .copied()
        .filter(|v| !["changes", "checks", "usage"].contains(v) || !sid.is_empty())
        .collect::<Vec<_>>();
    dom::clear("view-tabs");
    dom::hidden("view-tabs", views.len() < 2);
    for item in views {
        let button = dom::button(view_label(item), "view-open", json!(item));
        button.set_class_name("view-tab");
        let _ = button.set_attribute("data-view", item);
        if item == view {
            let _ = button.set_attribute("aria-current", "page");
        }
        dom::append("view-tabs", &button);
    }
    dom::hidden("open-kit3", view != "kit3");
}

fn repeated_suffix_removed<'a>(mut value: &'a str, suffix: &str) -> &'a str {
    if suffix.is_empty() {
        return value;
    }
    while let Some(rest) = value.strip_suffix(suffix) {
        value = rest;
    }
    value
}

fn merge_controls() {
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
        dom::text("merge-context-note","The first launch reads an immutable archive of these chats as untrusted history. Write the new task's instruction below.");
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

#[cfg(test)]
mod history_projection_tests {
    use super::*;

    #[test]
    fn sidebar_projection_preserves_labels_order_and_selection() {
        let sessions = vec![
            json!({"id":"a","name":"Named chat","title":"Unused title","status":"running"}),
            json!({"id":"b","title":"Second chat","status":"awaiting_approval"}),
        ];
        assert_eq!(
            history_projection(&sessions, "b"),
            json!([
                {"id":"a","label":"Named chat · running","selected":false},
                {"id":"b","label":"Second chat · awaiting approval","selected":true}
            ])
        );
        let original = history_projection(&sessions, "b");
        assert_ne!(original, history_projection(&sessions, "a"));
        let mut reversed = sessions.clone();
        reversed.reverse();
        assert_ne!(original, history_projection(&reversed, "b"));
        assert_eq!(history_projection(&[], "b"), json!([]));
    }

    #[test]
    fn sidebar_changes_only_when_rendered_fields_change() {
        let session = json!({"id":"a","title":"Chat","status":"running"});
        let original = history_projection(std::slice::from_ref(&session), "a");
        let mut report_changed = session.clone();
        report_changed["budget_usage"] = json!({"tokens":900});
        report_changed["fleet_result"] = json!({"report":"new report"});
        assert_eq!(original, history_projection(&[report_changed], "a"));
        for (field, value) in [
            ("id", "b"),
            ("title", "Renamed chat"),
            ("name", "Preferred name"),
            ("status", "completed"),
        ] {
            let mut changed = session.clone();
            changed[field] = json!(value);
            assert_ne!(original, history_projection(&[changed], "a"), "{field}");
        }
        assert_eq!(
            history_projection(&[json!({"id":"fallback"})], ""),
            json!([{"id":"fallback","label":"fallback · ","selected":false}])
        );
    }
}

#[cfg(test)]
mod session_contract_tests {
    use super::*;
    #[test]
    fn merge_settings_never_replace_the_saved_ordinary_project_draft() {
        let previous = json!({"drafts":{"a":{"workflow":"sdd","prompt":"ordinary work"}},"project_id":"a","session_id":null});
        let merge = json!({"project_id":"a","selection_epoch":4,"sources":["x","y"]});
        assert!(preferences_update(
            previous.clone(),
            "a",
            "",
            json!({"workflow":"native","prompt":m::MERGE_PROMPT}),
            &merge
        )
        .is_none());
        let next =
            preferences_update(previous, "a", "existing", json!({"prompt":""}), &V::Null).unwrap();
        assert_eq!(next["drafts"]["a"]["workflow"], "sdd");
        assert_eq!(next["drafts"]["a"]["prompt"], "ordinary work");
        assert_eq!(next["session_id"], "existing");
    }
    #[test]
    fn a_merge_draft_ends_on_session_project_or_selection_generation_change() {
        let draft = json!({"project_id":"a","selection_epoch":4,"sources":["x","y"]});
        assert!(merge_draft_owned(&draft, "", "a", 4));
        assert!(!merge_draft_owned(&draft, "chat", "a", 4));
        assert!(!merge_draft_owned(&draft, "", "b", 4));
        assert!(!merge_draft_owned(&draft, "", "a", 6));
    }
    #[test]
    fn budget_payload_contains_worker_timeout_only_for_fleet() {
        let ordinary = budget_request(
            &json!({"workflow":"native","budget_revision":2}),
            json!({"tokens":100}),
            json!(300),
        );
        assert!(ordinary.get("worker_timeout").is_none());
        let fleet = budget_request(
            &json!({"workflow":"fleet-review","fleet":{},"budget_revision":3}),
            json!({"usd":5}),
            json!(17),
        );
        assert_eq!(fleet["worker_timeout"], 17);
        assert_eq!(fleet["revision"], 3);
        assert_eq!(fleet["budgets"]["usd"], 5);
    }
}

#[cfg(test)]
mod draft_project_tests {
    use super::*;
    #[test]
    fn a_direct_project_selection_saves_the_previous_form_under_its_bound_project() {
        m::set("composer-project", json!("a"));
        let owner = draft_project("b");
        let previous = json!({"project_id":"a","session_id":"old-chat","drafts":{
            "a":{"prompt":"old a","bank":"bank-a","task":"task-a"},
            "b":{"prompt":"keep b","bank":"bank-b","task":"task-b","extra":{"retain":true}}
        }});
        let draft_b = previous["drafts"]["b"].clone();
        // The project select already says B while these extracted controls still belong to A.
        let extracted_a = json!({"prompt":"edited a","bank":"bank-a","task":"task-a"});
        let saved =
            preferences_update(previous, &owner, "", extracted_a.clone(), &V::Null).unwrap();
        assert_eq!(saved["drafts"]["a"], extracted_a);
        assert_eq!(saved["drafts"]["b"], draft_b);
        m::set("composer-project", json!("b"));
        let owner = draft_project("b");
        assert_eq!(owner, "b");
        // Selection persistence must not extract the still-unrestored A bank/task controls again.
        let persisted = preferences_selection(saved, &owner, "");
        assert_eq!(persisted["project_id"], "b");
        assert!(persisted["session_id"].is_null());
        assert_eq!(persisted["drafts"]["a"], extracted_a);
        assert_eq!(persisted["drafts"]["b"], draft_b);
        m::take("composer-project");
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    fn status(state: &str, detail: &str) -> V {
        json!({"state":state,"detail":detail,"behind":0,"ahead":0,"commits":[],"upstream":"origin/main","version":"6e5a8cc"})
    }
    #[test]
    fn an_available_update_shows_the_button_and_its_commits_on_hover() {
        let mut available = status("available", "12 new changes on origin/main.");
        available["behind"] = json!(12);
        available["commits"] = (1..=12)
            .map(|n| json!({"hash":format!("abc{n}"),"subject":format!("change {n}")}))
            .collect();
        let view = update_view(&available, &V::Null);
        assert!(view.button && !view.busy);
        assert_eq!(view.label, "Update");
        let lines = view.title.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 12);
        assert_eq!(lines[0], "12 new changes on origin/main.");
        assert_eq!(lines[1], "• change 1");
        assert_eq!(lines[10], "• change 10");
        assert_eq!(lines[11], "… and 2 more");
        assert!(view.note.is_empty());
        let mut one = status("available", "1 new changes on origin/main.");
        one["behind"] = json!(1);
        one["commits"] = json!([{"hash":"abc1","subject":"docs: one"}]);
        assert_eq!(
            update_view(&one, &V::Null).title,
            "1 new changes on origin/main.\n• docs: one"
        );
    }
    #[test]
    fn nothing_is_offered_without_an_update_whatever_else_the_status_says() {
        for state in [
            "unknown",
            "current",
            "unavailable",
            "updating",
            "updated",
            "",
        ] {
            let mut value = status(state, "This clone is not on a branch.");
            // The server never sends this key; the button follows the state alone.
            value["available"] = json!(true);
            let view = update_view(&value, &V::Null);
            assert!(!view.button, "{state}");
            assert!(view.note.is_empty() && view.title.is_empty(), "{state}");
        }
        assert!(!update_view(&V::Null, &V::Null).button);
    }
    #[test]
    fn a_diverged_failed_or_held_update_says_why_in_the_banner() {
        let diverged = update_view(
            &status(
                "diverged",
                "2 new changes on origin/main; this clone has 1 commits of its own. Update it with Git.",
            ),
            &V::Null,
        );
        assert!(!diverged.button && !diverged.error);
        assert!(diverged.note.ends_with("Update it with Git."));
        let failed = update_view(
            &status(
                "failed",
                "The update could not be applied: Git exited with status 128.",
            ),
            &V::Null,
        );
        assert!(!failed.button && failed.error);
        assert_eq!(
            failed.note,
            "The update could not be applied: Git exited with status 128."
        );
        let held = update_view(
            &status("restart-required", "The clone is now at 43a21a0, but its browser interface was built from other sources. In the clone, run `./infra-engine repo build-ui`, then press Update again."),
            &V::Null,
        );
        assert!(held.button && !held.busy && held.error);
        assert_eq!(held.label, "Update");
        assert!(held.note.contains("press Update again"));
    }
    #[test]
    fn what_this_page_did_takes_over_the_button_and_the_banner() {
        let available = status("available", "1 new changes on origin/main.");
        let pending = update_view(&available, &json!({"pending":true}));
        assert!(pending.button && pending.busy);
        assert_eq!(pending.label, "Updating…");
        let restarting = update_view(
            &status("updated", ""),
            &json!({"restarting":true,"note":"Updated to 43a21a0. The Harness is restarting; this page reloads by itself."}),
        );
        assert!(restarting.button && restarting.busy && !restarting.error);
        assert_eq!(restarting.label, "Restarting…");
        assert!(restarting.note.starts_with("Updated to 43a21a0."));
        // A refused update keeps its message, also over the detail the server recorded.
        let refused = update_view(
            &status("restart-required", "recorded detail"),
            &json!({"failed":true,"note":"An update is waiting for the Harness to restart."}),
        );
        assert!(refused.button && !refused.busy && refused.error);
        assert_eq!(
            refused.note,
            "An update is waiting for the Harness to restart."
        );
        let current = update_view(
            &status("current", ""),
            &json!({"note":"This clone is already up to date."}),
        );
        assert!(!current.button && !current.error);
        assert_eq!(current.note, "This clone is already up to date.");
    }
    #[test]
    fn commands_quoted_in_a_note_become_code() {
        assert_eq!(
            code_spans(
                "In the clone, run `cargo build --release`, then `./infra-engine repo build-ui`."
            ),
            vec![
                (false, "In the clone, run "),
                (true, "cargo build --release"),
                (false, ", then "),
                (true, "./infra-engine repo build-ui"),
                (false, "."),
            ]
        );
        assert_eq!(code_spans("`a`"), vec![(true, "a")]);
        assert_eq!(
            code_spans("an unpaired ` stays"),
            vec![(false, "an unpaired ` stays")]
        );
        assert!(code_spans("").is_empty());
    }
}
