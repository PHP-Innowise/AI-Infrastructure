use crate::{api, dom, management, model as m, session, system_map};
use serde_json::{json, Value as V};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
fn editor_current(project: &str, config: &str, generation: u64) -> bool {
    dom::value("system-project") == project
        && dom::value("system-config") == config
        && m::scope_current("editor-scope", generation, project, config)
}
fn editor_ready() -> bool {
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
fn sync_editor_target() -> bool {
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
fn clear_editor_notices() {
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
fn reset_system_target(project: &str, config: &str) {
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
    m::bind_scope("editor-scope", project, config);
    m::next("system-read");
    m::next("system-list");
    m::next("discovery");
    m::bind_scope("system-scope", project, "");
    dom::options("system-runs", &[], m::summary, None);
    crate::activity::reset("system-agents");
    crate::activity::reset("system-discovery-agent");
    dom::cancel_timer("system-poll");
    dom::cancel_timer("system-graph-poll");
    dom::cancel_timer("discovery-poll");
    for key in [
        "editor",
        "editor-generation",
        "system-run",
        "system-catalog",
        "system-graph-status",
        "system-graph-monotonic",
        "system-live-focus",
        "system-member-selected",
        "system-change-selected-service",
        "discovery-id",
        "discovery-owner",
    ] {
        m::take(key);
    }
    for key in ["editor-dirty", "editor-pending", "discovery-active"] {
        m::set(key, json!(false));
    }
    for id in [
        "system-editor",
        "system-run",
        "system-catalog",
        "system-change-map-workspace",
        "system-discovery-agents",
    ] {
        dom::hidden(id, true);
    }
}
fn system_current(project: &str, id: &str, generation: u64) -> bool {
    dom::value("system-project") == project
        && m::scope_current("system-scope", generation, project, id)
}
fn discovery_current(epoch: u64) -> bool {
    let owner = m::data("discovery-owner");
    m::current("discovery", epoch)
        && editor_current(
            &m::text(&owner, "project"),
            &m::text(&owner, "config"),
            owner["generation"].as_u64().unwrap_or(0),
        )
}

pub fn adopt_sidebar_project(previous_view: &str) {
    let held = [
        "editor-dirty",
        "editor-pending",
        "system-pending",
        "discovery-active",
    ]
    .iter()
    .any(|key| m::yes(&m::data(key)));
    let current = dom::value("system-project");
    let sidebar = dom::value("project-switcher");
    let projects = m::read(|state| m::array(&state.boot["projects"]));
    if let Some(project) = m::project_for_reentry(
        "editor-scope",
        &current,
        &sidebar,
        held,
        &projects,
        previous_view,
    ) {
        dom::set_value("system-project", &project);
        reset_system_project();
    }
}
pub fn reset_system_project() {
    reset_system_target(&dom::value("system-project"), &dom::value("system-config"));
    controls();
}
pub fn leave_systems_view() {
    dom::cancel_timer("system-graph-poll");
    crate::graph_explanation::clear_system_context();
    let status = m::data("graph");
    let project = dom::value("project");
    let view = crate::graph::graph_view(&status);
    let (pill_state, pill_label) = crate::graph::sidebar_status(&project, &status, &view);
    dom::attr("sidebar-graph-pill", "data-state", pill_state);
    dom::text("sidebar-graph-label", &pill_label);
    dom::attr("sidebar-graph-pill", "title", &pill_label);
    dom::attr("sidebar-graph-pill", "aria-label", &pill_label);
    spawn_local(async {
        crate::graph::load(false).await;
    });
}

pub fn focus_system_graph_workspace() {
    let view = m::read(|s| s.view.clone());
    let ws_id = if view == "system-changes" {
        "system-change-map-workspace"
    } else {
        "system-map-workspace"
    };
    spawn_local(async {
        poll_system_graph_status(true).await;
    });
    dom::attr(ws_id, "tabindex", "-1");
    if let Some(el) = dom::get(ws_id) {
        el.scroll_into_view();
    }
    dom::focus(ws_id);
}

/// Switches either `Services` (`surface == "services"`) or `Changes` (`surface == "changes"`)
/// from the cross-service topology canvas into a member service's (or `__system__`'s) code map.
pub fn drill_down_service_map(surface: &str, service_id: &str, focus_path: Option<&str>) {
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    if service_id.is_empty() {
        return;
    }
    if surface == "changes" {
        m::set("system-change-selected-service", json!(service_id));
        dom::hidden("system-change-topology-panel", true);
        dom::hidden("system-change-graph-workspace", false);
        dom::attr("system-change-tab-topology", "aria-pressed", "false");
        dom::text(
            "system-change-graph-title",
            &format!("Service: {service_id}"),
        );
        sync_change_service_tabs();
        let scope = build_system_graph_scope(&project, &config, service_id, true);
        crate::graph_map::load_system_embedded(
            crate::graph_map::SystemEmbeddedKind::Change,
            Some(scope.clone()),
            false,
        );
        let st = &m::data("system-graph-status")["services"][service_id];
        if st.is_object() {
            crate::graph::sync_embedded_progress_for_surface("system-change-graph", st);
        }
        let live_focus = current_service_live_focus(service_id).unwrap_or(V::Null);
        crate::graph_map::apply_system_live_focus(
            crate::graph_map::SystemEmbeddedKind::Change,
            &live_focus,
        );
        // A file is selected in the picture that is open here, not in a
        // window over it.
        if let Some(path) = focus_path.filter(|p| !p.trim().is_empty()) {
            crate::graph_map::focus_system_file(crate::graph_map::SystemEmbeddedKind::Change, path);
        }
    } else {
        m::set("system-member-selected", json!(service_id));
        dom::hidden("system-map", true);
        dom::hidden("system-member-graph-workspace", false);
        dom::hidden("system-member-picker-field", false);
        dom::attr("system-map-level-topology", "aria-pressed", "false");
        dom::attr("system-map-level-service", "aria-pressed", "true");
        dom::set_value("system-member-select", service_id);
        dom::text(
            "system-member-graph-title",
            &format!("Service: {service_id}"),
        );
        let scope = build_system_graph_scope(&project, &config, service_id, false);
        crate::graph_map::load_system_embedded(
            crate::graph_map::SystemEmbeddedKind::Member,
            Some(scope.clone()),
            false,
        );
        let st = &m::data("system-graph-status")["services"][service_id];
        if st.is_object() {
            crate::graph::sync_embedded_progress_for_surface("system-member-graph", st);
        }
        if let Some(path) = focus_path.filter(|p| !p.trim().is_empty()) {
            crate::graph_map::focus_system_file(crate::graph_map::SystemEmbeddedKind::Member, path);
        }
    }
}

pub fn switch_services_to_topology() {
    dom::hidden("system-map", false);
    dom::hidden("system-member-graph-workspace", true);
    dom::hidden("system-member-picker-field", true);
    dom::attr("system-map-level-topology", "aria-pressed", "true");
    dom::attr("system-map-level-service", "aria-pressed", "false");
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let cat = m::data("system-catalog")["catalog"].clone();
    let status = m::data("system-graph-status");
    system_map::refresh_services_map(&project, &config, &cat, &status);
}

pub fn switch_changes_to_topology() {
    m::take("system-change-selected-service");
    dom::hidden("system-change-topology-panel", false);
    dom::hidden("system-change-graph-workspace", true);
    dom::attr("system-change-tab-topology", "aria-pressed", "true");
    sync_change_service_tabs();
    sync_changes_map_workspace();
}

/// Computes `(data-state, label)` for `#system-graphs-pill` and `#sidebar-graph-pill` from `/api/systems/graph/status`.
pub fn aggregate_graphs_pill(status: &V) -> (&'static str, String) {
    if !status.is_object() {
        return ("idle", "Service graphs: loading…".to_owned());
    }
    let ready = status["ready_count"].as_u64().unwrap_or_else(|| {
        status["services"]
            .as_object()
            .map(|m_obj| m_obj.values().filter(|s| s["state"] == "ready").count() as u64)
            .unwrap_or(0)
    });
    let total = status["total_count"].as_u64().unwrap_or_else(|| {
        status["services"]
            .as_object()
            .map(|m_obj| m_obj.len() as u64)
            .unwrap_or(0)
    });
    let building = status["building_count"].as_u64().unwrap_or_else(|| {
        status["services"]
            .as_object()
            .map(|m_obj| {
                m_obj
                    .values()
                    .filter(|s| s["state"] == "building" || s["build"].is_object())
                    .count() as u64
            })
            .unwrap_or(0)
    });
    if building > 0 || status["state"] == "building" {
        return (
            "building",
            format!("Service graphs: {ready}/{total} ready · {building} building"),
        );
    }
    match status["state"].as_str().unwrap_or("") {
        "failed" => (
            "failed",
            format!("Service graphs: {ready}/{total} ready · failed"),
        ),
        "stale" => (
            "stale",
            format!("Service graphs: {ready}/{total} ready · stale"),
        ),
        "missing" => ("missing", "Graphify not installed".to_owned()),
        "off" => (
            "off",
            format!("Service graphs: off ({ready}/{total} ready)"),
        ),
        _ if total > 0 && ready == total => {
            ("ready", format!("Service graphs: {ready}/{total} ready"))
        }
        _ if total > 0 => ("empty", format!("Service graphs: {ready}/{total} ready")),
        _ => ("idle", "Service graphs: 0/0 ready".to_owned()),
    }
}

/// Clamps and enforces monotonic `1..=99%` progress per building member service so bars never jump backward.
pub fn monotonic_service_graph_status(
    service_id: &str,
    raw: &V,
    cache: &mut BTreeMap<String, u64>,
) -> V {
    if !raw.is_object() {
        cache.remove(service_id);
        return raw.clone();
    }
    let is_building = raw["state"] == "building" || raw["build"].is_object();
    if !is_building {
        cache.remove(service_id);
        return raw.clone();
    }
    let mut out = raw.clone();
    let reported = raw["build"]["percent"]
        .as_u64()
        .or_else(|| raw["percent"].as_u64())
        .unwrap_or(1)
        .clamp(1, 99);
    let prev = cache.get(service_id).copied().unwrap_or(1);
    let clamped = reported.max(prev).clamp(1, 99);
    cache.insert(service_id.to_owned(), clamped);
    if !out["build"].is_object() {
        out["build"] = json!({});
    }
    out["build"]["percent"] = json!(clamped);
    out
}

fn apply_monotonic_statuses(mut agg: V) -> V {
    let mut map_cache: BTreeMap<String, u64> = m::data("system-graph-monotonic")
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
                .collect()
        })
        .unwrap_or_default();
    if let Some(services) = agg["services"].as_object_mut() {
        for (sid, s_val) in services.iter_mut() {
            *s_val = monotonic_service_graph_status(sid, s_val, &mut map_cache);
        }
    }
    m::set("system-graph-monotonic", json!(map_cache));
    agg
}

fn catalog_service_ids() -> Vec<String> {
    let cat = &m::data("system-catalog")["catalog"];
    let mut ids: Vec<String> = m::array(&cat["services"])
        .into_iter()
        .map(|s| m::text(&s, "id"))
        .filter(|id| !id.is_empty())
        .collect();
    if !ids.iter().any(|s| s == "__system__") {
        ids.push("__system__".to_owned());
    }
    ids
}

fn build_system_graph_scope(
    project_id: &str,
    config_path: &str,
    service_id: &str,
    include_run: bool,
) -> crate::graph::GraphTargetScope {
    let services = catalog_service_ids();
    let run_doc = m::data("system-run");
    let (run_id, run_title, launches) = if include_run && run_doc.is_object() {
        let rid = m::text(&run_doc, "id");
        let title = {
            let cid = m::text(&run_doc, "change_id");
            let task = m::text(&run_doc["plan"], "task");
            if !cid.is_empty() && !task.is_empty() {
                Some(format!("{cid} · {task}"))
            } else if !cid.is_empty() {
                Some(cid)
            } else if !rid.is_empty() {
                Some(rid.clone())
            } else {
                None
            }
        };
        (
            (!rid.is_empty()).then_some(rid),
            title,
            m::array(&run_doc["launches"]),
        )
    } else {
        (None, None, Vec::new())
    };
    crate::graph::GraphTargetScope::SystemService {
        project_id: project_id.to_owned(),
        config_path: config_path.to_owned(),
        service_id: service_id.to_owned(),
        services,
        run_id,
        run_title,
        launches,
    }
}

fn sync_system_explanation_context_from_dom() {
    let project = dom::value("system-project");
    if project.is_empty() {
        return;
    }
    let config = dom::value("system-config");
    let provider = dom::value("system-provider");
    crate::graph_explanation::set_system_context(
        &project,
        &config,
        &provider,
        catalog_service_ids(),
    );
}

fn sync_system_graph_header(agg: &V) {
    let (state, label) = aggregate_graphs_pill(agg);
    dom::attr("system-graphs-pill", "data-state", state);
    dom::text("system-graphs-pill-text", &label);
    if let Some(tell) = agg["tell_agents"].as_bool().or(agg["agents"].as_bool()) {
        dom::set_checked("system-graphs-agents", tell);
        if !m::yes(&m::data("system-graph-prompt-touched")) {
            dom::set_checked("system-graph-prompt", tell);
        }
    }
    crate::graph::sync_sidebar_pill_for_system(agg);
    session::sync_working_indicator();
}

async fn poll_system_graph_status(check: bool) {
    dom::cancel_timer("system-graph-poll");
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    if project.is_empty() {
        return;
    }
    let epoch = m::next("system-graph-status-fetch");
    let q = dom::query(&[
        ("project_id", project.clone()),
        ("config_path", config.clone()),
        ("check", if check { "1" } else { "0" }.to_owned()),
    ]);
    if let Ok(raw) = api::get(&format!("/api/systems/graph/status{q}")).await {
        if !m::current("system-graph-status-fetch", epoch) {
            return;
        }
        let agg = apply_monotonic_statuses(raw);
        m::set("system-graph-status", agg.clone());
        sync_system_graph_header(&agg);
        update_service_card_pills(&agg);

        if let Some(member_svc) = m::data("system-member-selected")
            .as_str()
            .filter(|s| !s.is_empty())
        {
            let st = &agg["services"][member_svc];
            if st.is_object() {
                crate::graph::sync_embedded_progress_for_surface("system-member-graph", st);
            }
        }
        if let Some(change_svc) = m::data("system-change-selected-service")
            .as_str()
            .filter(|s| !s.is_empty())
        {
            let st = &agg["services"][change_svc];
            if st.is_object() {
                crate::graph::sync_embedded_progress_for_surface("system-change-graph", st);
            }
        }

        let cat = m::data("system-catalog")["catalog"].clone();
        if cat.is_object() {
            system_map::refresh_services_map(&project, &config, &cat, &agg);
            sync_changes_map_workspace();
        }

        let building =
            agg["building_count"].as_u64().unwrap_or(0) > 0 || agg["state"] == "building";
        if building && m::read(|s| matches!(s.view.as_str(), "systems" | "system-changes")) {
            dom::later("system-graph-poll", 900, || {
                spawn_local(async {
                    poll_system_graph_status(false).await;
                });
            });
        }
    }
}

async fn post_system_graph_action(action: &str, service: Option<&str>) {
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    if project.is_empty() {
        return;
    }
    let mut body = json!({
        "project_id": project,
        "config_path": config,
        "action": action,
    });
    if let Some(svc) = service.filter(|s| !s.is_empty()) {
        body["service"] = json!(svc);
    }
    let _ = api::post("/api/systems/graph", body).await;
    poll_system_graph_status(true).await;
}

fn update_service_card_pills(agg: &V) {
    let Some(services_obj) = agg["services"].as_object() else {
        return;
    };
    for (sid, st) in services_obj {
        if let Some(slot) = dom::get(&format!("system-service-graph-slot-{sid}")) {
            slot.set_inner_html(&crate::graph::render_service_graph_pill_html(sid, st));
        }
    }
}

pub fn build_changes_overlay(
    run_doc: &V,
    plan_form_open: bool,
    plan_seeds: &[String],
    depth: u64,
    raw_contracts: &str,
) -> V {
    let parsed_contracts: Vec<String> = raw_contracts
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();

    if !run_doc.is_object() || plan_form_open {
        return json!({
            "stage": "plan",
            "run_id": "",
            "seed_services": plan_seeds,
            "depth": depth,
            "changed_contracts": parsed_contracts,
            "step_states": {},
            "cross_service_unopened": [],
        });
    }

    let run_id = m::text(run_doc, "id");
    let running = m::yes(&run_doc["active"]);
    let steps = if run_doc["execution"]["steps"].is_array() {
        m::array(&run_doc["execution"]["steps"])
    } else {
        m::array(&run_doc["plan"]["steps"])
    };
    let mut step_states = serde_json::Map::new();
    for step in &steps {
        let svc = m::text(step, "service");
        if !svc.is_empty() && svc != "__system__" {
            let st = shown_status(step, &run_doc["receipts"], running);
            step_states.insert(svc, json!(st));
        }
    }

    let seeds: Vec<String> = m::array(&run_doc["plan"]["context"]["services"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let unopened = if run_doc["cross_service_unopened"].is_array() {
        run_doc["cross_service_unopened"].clone()
    } else if run_doc["evidence"]["cross_service_unopened"].is_array() {
        run_doc["evidence"]["cross_service_unopened"].clone()
    } else {
        json!([])
    };

    let stage = if running {
        "run"
    } else if run_doc["session_id"].is_string() {
        "receipts"
    } else {
        "review"
    };

    json!({
        "stage": stage,
        "run_id": run_id,
        "seed_services": if seeds.is_empty() { plan_seeds } else { &seeds },
        "depth": run_doc["plan"]["depth"].as_u64().unwrap_or(depth),
        "changed_contracts": parsed_contracts,
        "step_states": V::Object(step_states),
        "evidence": run_doc["evidence"],
        "cross_service_unopened": unopened,
    })
}

fn extract_service_graph_focus_map(run_doc: &V) -> serde_json::Map<String, V> {
    let mut out = serde_json::Map::new();
    if let Some(gf) = run_doc["graph_focus"].as_object() {
        if let Some(services_obj) = gf.get("services").and_then(V::as_object) {
            for (k, v) in services_obj {
                if v.is_object() {
                    out.insert(k.clone(), v.clone());
                }
            }
        } else if let Some(svc) = gf.get("service").and_then(V::as_str) {
            let f = gf
                .get("focus")
                .cloned()
                .unwrap_or_else(|| run_doc["graph_focus"].clone());
            out.insert(svc.to_owned(), f);
        } else {
            for (k, v) in gf {
                if k != "aggregate" && v.is_object() {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
    }
    for ev in m::array(&run_doc["events"]) {
        if ev["kind"] == "graph_focus" {
            let svc = m::text(&ev, "service");
            if !svc.is_empty() && ev["focus"].is_object() {
                out.insert(svc, ev["focus"].clone());
            }
        }
    }
    out
}

fn current_service_live_focus(service_id: &str) -> Option<V> {
    let cache = m::data("system-live-focus");
    if cache[service_id].is_object() {
        return Some(cache[service_id].clone());
    }
    let run_doc = m::data("system-run");
    extract_service_graph_focus_map(&run_doc)
        .get(service_id)
        .cloned()
}

fn ingest_run_graph_focus(run_doc: &V) {
    let mut by_svc = m::data("system-live-focus")
        .as_object()
        .cloned()
        .unwrap_or_default();
    for (svc, focus) in extract_service_graph_focus_map(run_doc) {
        by_svc.insert(svc, focus);
    }
    m::set("system-live-focus", V::Object(by_svc));
    let selected_svc = m::text(
        &json!({ "s": m::data("system-change-selected-service") }),
        "s",
    );
    if !selected_svc.is_empty() {
        let live_focus = current_service_live_focus(&selected_svc).unwrap_or(V::Null);
        crate::graph_map::apply_system_live_focus(
            crate::graph_map::SystemEmbeddedKind::Change,
            &live_focus,
        );
    }
}

fn sync_change_service_tabs() {
    let Some(wrap) = dom::get("system-change-service-tabs") else {
        return;
    };
    let cat = &m::data("system-catalog")["catalog"];
    let run_doc = m::data("system-run");
    let active_svc = m::data("system-change-selected-service")
        .as_str()
        .unwrap_or("")
        .to_owned();

    let mut service_ids: Vec<String> = m::array(&run_doc["plan"]["context"]["services"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    if service_ids.is_empty() {
        service_ids = m::array(&cat["services"])
            .into_iter()
            .map(|s| m::text(&s, "id"))
            .filter(|s| !s.is_empty())
            .collect();
    }
    if !service_ids.is_empty() && !service_ids.iter().any(|s| s == "__system__") {
        service_ids.push("__system__".to_owned());
    }

    let running = m::yes(&run_doc["active"]);
    let steps = if run_doc["execution"]["steps"].is_array() {
        m::array(&run_doc["execution"]["steps"])
    } else {
        m::array(&run_doc["plan"]["steps"])
    };

    wrap.set_text_content(None);
    for sid in service_ids {
        let step_st = steps
            .iter()
            .find(|s| s["service"] == sid)
            .map(|s| shown_status(s, &run_doc["receipts"], running))
            .unwrap_or_default();
        let label = if sid == "__system__" {
            "Shared (__system__)".to_owned()
        } else if !step_st.is_empty() {
            format!("{sid} · {step_st}")
        } else {
            sid.clone()
        };
        let btn = dom::button(
            &label,
            "system-change-service-tab",
            json!({ "service": sid }),
        );
        btn.set_class_name("button compact session-graph-btn session-graph-toggle");
        let _ = btn.set_attribute(
            "aria-pressed",
            if active_svc == sid { "true" } else { "false" },
        );
        dom::add(&wrap, &btn);
    }
}

fn sync_changes_map_workspace() {
    let cat_wrap = m::data("system-catalog");
    let cat = &cat_wrap["catalog"];
    let run_doc = m::data("system-run");
    let has_content = cat.is_object() || run_doc.is_object();
    dom::hidden("system-change-map-workspace", !has_content);
    if !has_content {
        return;
    }
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let checked_seeds: Vec<String> = dom::selected_checks("system-plan-services")
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let depth = dom::value("system-depth").parse::<u64>().unwrap_or(32);
    let contracts_raw = dom::value("system-contracts");
    let plan_form_open = dom::get("system-plan-form").is_some_and(|f| !f.has_attribute("hidden"));
    let overlay = build_changes_overlay(
        &run_doc,
        plan_form_open,
        &checked_seeds,
        depth,
        &contracts_raw,
    );
    let status = m::data("system-graph-status");
    sync_change_service_tabs();
    system_map::refresh_changes_map(&project, &config, cat, &status, &overlay);
}

pub async fn load() {
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
            }
        }
        Err(error) if !error.aborted && system_current(&project, "", generation) => {
            dom::error("system-run-error", &error.message)
        }
        _ => {}
    }
}
/// The run's status and what stopped it: the stopped dispatch's receipt says
/// that best (the agent's own reason, failed checks, the CLI's message); the
/// journal's error is the fallback, a code read as words.
fn run_status(v: &V) -> String {
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
fn scan_end(job: &V) -> String {
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
fn sentence(text: &str) -> String {
    let text = text.trim_end();
    if text.ends_with(['.', '!', '?']) {
        text.to_owned()
    } else {
        format!("{text}.")
    }
}
/// The next change ID of a revised plan: `cancel-v3` → `cancel-v4`,
/// `cancel` → `cancel-2`.
fn next_change_id(id: &str) -> String {
    let digits = id.chars().rev().take_while(char::is_ascii_digit).count();
    let (stem, number) = id.split_at(id.len() - digits);
    match number.parse::<u64>() {
        Ok(number) if !stem.is_empty() => format!("{stem}{}", number + 1),
        _ => format!("{id}-2"),
    }
}
/// Opens the plan form filled from a change: a blocked agreement asks for a
/// decision, and the next plan states it, so only the task needs editing.
fn replan(v: &V) {
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
fn stages(v: &V) -> Vec<(&'static str, &'static str)> {
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
fn stepper(v: &V) {
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
fn run_label(run: &V) -> String {
    format!(
        "{} · {}",
        m::text(run, "change_id"),
        m::label(run["status"].as_str().unwrap_or(""))
    )
}
/// Lists the project's changes again with `selected` chosen: a new plan is
/// shown under the change list it joins.
async fn relist(project: &str, selected: &str) {
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
async fn read(id: &str) {
    let project = dom::value("system-project");
    let generation = m::bind_scope("system-scope", &project, id);
    read_scoped(id, &project, generation).await;
}
async fn read_scoped(id: &str, project: &str, generation: u64) {
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
                let id = id.to_owned();
                let project = project.to_owned();
                dom::later("system-poll", 1100, move || {
                    spawn_local(async move {
                        read_scoped(&id, &project, generation).await;
                    })
                });
            }
        }
        Err(error) if !error.aborted && system_current(project, id, generation) => {
            dom::error("system-run-error", &error.message)
        }
        _ => {}
    }
}
fn editor_lock_state(scanning: bool, dirty: bool) -> (&'static str, &'static str, bool) {
    if scanning {
        (
            "scanning",
            "An AI scan is filling the system editor on Services. Planning and launching wait until it ends.",
            false,
        )
    } else if dirty {
        (
            "dirty",
            "The system editor on Services has unsaved changes. Save or discard them to plan or launch a change.",
            true,
        )
    } else {
        ("", "", false)
    }
}

fn sync_editor_lock(state: &str, notice: &str, show_discard: bool, locked: bool) {
    let Some(lock) = dom::get("system-editor-lock") else {
        return;
    };
    let current = lock.get_attribute("data-lock-state").unwrap_or_default();
    if current != state {
        let _ = lock.set_attribute("data-lock-state", state);
        lock.set_text_content(None);
        if !notice.is_empty() {
            dom::add(&lock, &dom::el("span", "", notice));
            if show_discard {
                dom::add(&lock, &dom::el("span", "", " "));
                let btn = dom::button("Discard draft", "system-editor-lock-discard", V::Null);
                btn.set_class_name("button compact");
                let _ = btn.set_attribute("id", "system-editor-lock-discard");
                if locked {
                    let _ = btn.set_attribute("disabled", "");
                }
                dom::add(&lock, &btn);
            }
        }
    } else if show_discard {
        dom::disabled("system-editor-lock-discard", locked);
    }
    dom::hide(&lock, notice.is_empty());
}

fn discard_editor_draft(from_lock_banner: bool) {
    if m::yes(&m::data("editor-pending")) {
        return;
    }
    let lock_had_focus = from_lock_banner
        && dom::document()
            .active_element()
            .zip(dom::get("system-editor-lock"))
            .is_some_and(|(active, lock)| lock.contains(Some(&active)));
    clear_editor_notices();
    remember_discovery(None);
    m::next("discovery");
    dom::cancel_timer("discovery-poll");
    crate::activity::reset("system-discovery-agent");
    m::take("discovery-owner");
    m::next("editor-scope");
    m::take("editor");
    m::take("editor-generation");
    m::set("editor-dirty", json!(false));
    dom::set_value("system-editor-name", "");
    dom::text("system-editor-location", "");
    dom::clear("system-editor-services");
    dom::clear("system-editor-shared");
    dom::hidden("system-editor", true);
    controls();
    if from_lock_banner {
        dom::text(
            "system-run-message",
            "Discarded unsaved system editor draft.",
        );
        if lock_had_focus {
            for candidate in [
                "system-prepare",
                "system-execute",
                "system-new-change",
                "system-runs",
            ] {
                let can_focus = dom::get(candidate).is_some_and(|el| {
                    !el.has_attribute("hidden")
                        && !dom::is_disabled(&el)
                        && el.closest("[hidden]").ok().flatten().is_none()
                });
                if can_focus {
                    dom::focus(candidate);
                    break;
                }
            }
        }
    }
}

fn controls() {
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
fn layout() {
    let shown = |id: &str| dom::get(id).is_some_and(|e| !e.has_attribute("hidden"));
    dom::hidden(
        "system-empty",
        shown("system-editor") || shown("system-catalog"),
    );
}
/// Opening Services shows the project's system when its file exists: the
/// form already names it, so a Load system click on every visit added
/// nothing. A project without the file keeps the empty state, quietly.
pub async fn catalog_autoload() {
    if !m::data("system-catalog").is_null()
        || m::yes(&m::data("editor-dirty"))
        || dom::value("system-project").is_empty()
    {
        return;
    }
    catalog_fetch(true).await;
}
async fn catalog_load() {
    if !sync_editor_target() {
        return;
    }
    if m::yes(&m::data("editor-dirty")) {
        dom::error(
            "system-error",
            "Save or discard the current system draft before reloading.",
        );
        return;
    }
    catalog_fetch(false).await;
}
async fn catalog_fetch(quiet_when_missing: bool) {
    if !sync_editor_target() {
        return;
    }
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let generation = m::read(|s| s.epochs.get("editor-scope").copied().unwrap_or(0));
    let epoch = m::next("system-catalog-load");
    match api::post(
        "/api/systems/catalog",
        json!({"project_id":project,"config_path":config}),
    )
    .await
    {
        Ok(value)
            if editor_current(&project, &config, generation)
                && m::current("system-catalog-load", epoch) =>
        {
            m::set("system-catalog", value.clone());
            render_catalog(&value);
            poll_system_graph_status(true).await;
        }
        Err(error)
            if editor_current(&project, &config, generation)
                && m::current("system-catalog-load", epoch)
                && !(quiet_when_missing && error.status == 404) =>
        {
            dom::error("system-error", &error.message)
        }
        _ => {}
    }
}
/// The line from a provider node to a consumer node (190×76 boxes on a 236×124
/// grid). It leaves and enters on the facing sides, so it never crosses its own
/// two boxes; a pair linked both ways gets two parallel lines; a link that skips
/// boxes in the same row arcs over them (forward) or under them (backward).
#[cfg(test)]
fn edge_path(a: (usize, usize), b: (usize, usize)) -> String {
    let (ax, ay) = (a.0 as i64, a.1 as i64);
    let (bx, by) = (b.0 as i64, b.1 as i64);
    if a == b {
        // A service that consumes its own contract: a small loop on its corner.
        return format!(
            "M{},{ay} C{},{} {},{} {},{}",
            ax + 160,
            ax + 160,
            ay - 24,
            ax + 214,
            ay + 8,
            ax + 190,
            ay + 22
        );
    }
    if ay == by {
        let span = (bx - ax).abs() / 236;
        let forward = bx > ax;
        if span <= 1 {
            let y = ay + if forward { 30 } else { 46 };
            let (x1, x2) = if forward {
                (ax + 190, bx)
            } else {
                (ax, bx + 190)
            };
            return format!("M{x1},{y} L{x2},{y}");
        }
        let (x1, x2) = (ax + 95, bx + 95);
        let (y, bend) = if forward {
            (ay, ay - 22)
        } else {
            (ay + 76, ay + 76 + 22)
        };
        return format!("M{x1},{y} Q{},{bend} {x2},{y}", (x1 + x2) / 2);
    }
    if by > ay {
        format!("M{},{} L{},{by}", ax + 80, ay + 76, bx + 80)
    } else {
        format!("M{},{ay} L{},{}", ax + 110, bx + 110, by + 76)
    }
}
/// What a run step shows: a dispatch the journal left running is not running
/// once its run stopped, and one whose current attempt saved a successful
/// receipt has reported complete; resuming records that receipt.
fn shown_status(step: &V, receipts: &V, active: bool) -> String {
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
struct SystemRelationshipView {
    provider: String,
    consumer: String,
    contract: String,
    version: String,
    provenance: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemStepView {
    key: String,
    id: String,
    stage_class: String,
    row_state: String,
    status_label: String,
    service: String,
    mode: String,
    attempt: Option<String>,
    goal: String,
    depends_on: Option<String>,
    task_reference: Option<String>,
}

fn build_relationship_view(edge: &V) -> SystemRelationshipView {
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

fn step_stage_class(status: &str) -> &'static str {
    match status {
        "running" | "in_progress" | "active" => "system-stage running",
        "blocked" | "failed" | "error" => "system-stage blocked",
        "interrupted" | "reported" | "stopped" => "system-stage interrupted",
        _ => "system-stage",
    }
}

fn step_row_state(status: &str) -> &'static str {
    match status {
        "running" | "in_progress" | "active" => "running",
        "failed" | "blocked" | "error" => "failed",
        _ => "idle",
    }
}

fn build_system_step_view(step: &V, receipts: &V, running: bool, idx: usize) -> SystemStepView {
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
fn unrun_checks(step: &V, receipts: &V) -> Option<Vec<String>> {
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
fn receipt_outcome(receipt: &V) -> String {
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
fn selection_hint(candidates: &[V]) -> String {
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
fn joined(values: &V) -> String {
    let items = m::array(values).iter().map(m::string).collect::<Vec<_>>();
    if items.is_empty() {
        "none".into()
    } else {
        items.join(", ")
    }
}
/// A catalog or plan warning as a sentence: who it concerns, what is wrong, where.
fn warning_text(warning: &V) -> String {
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
fn metric_list(rows: &[(&str, String, bool)]) -> web_sys::Element {
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

fn receipt_evidence_summary(evidence: &V) -> Option<(String, bool)> {
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

fn run_has_agent_activity(v: &V) -> bool {
    !m::array(&v["launches"]).is_empty()
        || !m::array(&v["evidence"]["dispatches"]).is_empty()
        || !m::array(&v["receipts"]).is_empty()
}

fn render_catalog(v: &V) {
    let catalog = &v["catalog"];
    let services = m::array(&catalog["services"]);
    let edges = m::array(&catalog["relationships"]);
    dom::hidden("system-catalog", false);
    dom::hidden("system-empty", true);
    dom::text("system-name", &m::text(catalog, "system"));
    dom::text(
        "system-catalog-summary",
        &format!(
            "{} services · {} declared contract links",
            services.len(),
            edges.len()
        ),
    );
    let mut indegree: BTreeMap<String, usize> =
        services.iter().map(|s| (m::text(s, "id"), 0)).collect();
    for edge in &edges {
        *indegree.entry(m::text(edge, "consumer")).or_default() += 1;
    }
    let mut queue: VecDeque<String> = services
        .iter()
        .filter(|s| indegree.get(&m::text(s, "id")) == Some(&0))
        .map(|s| m::text(s, "id"))
        .collect();
    let mut ordered = vec![];
    while let Some(id) = queue.pop_front() {
        if ordered.contains(&id) {
            continue;
        }
        ordered.push(id.clone());
        for edge in edges.iter().filter(|e| e["provider"] == id) {
            let next = m::text(edge, "consumer");
            if let Some(n) = indegree.get_mut(&next) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    queue.push_back(next);
                }
            }
        }
    }
    for s in &services {
        let id = m::text(s, "id");
        if !ordered.contains(&id) {
            ordered.push(id);
        }
    }

    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let provider = {
        let p = dom::value("system-provider");
        if !p.is_empty() {
            p
        } else {
            dom::value("system-discovery-provider")
        }
    };

    let mut member_options: Vec<V> = ordered
        .iter()
        .map(|sid| json!({ "id": sid, "label": sid }))
        .collect();
    member_options.push(json!({ "id": "__system__", "label": "Shared (__system__)" }));
    let chosen_member = m::data("system-member-selected")
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| ordered.first().cloned());
    dom::options(
        "system-member-select",
        &member_options,
        m::summary,
        chosen_member.as_deref(),
    );

    let mut explain_services = ordered.clone();
    if !explain_services.iter().any(|s| s == "__system__") {
        explain_services.push("__system__".to_owned());
    }
    crate::graph_explanation::set_system_context(&project, &config, &provider, explain_services);

    let graph_status = m::data("system-graph-status");
    dom::clear("system-services");
    let chosen = dom::selected_checks("system-plan-services");
    dom::clear("system-plan-services");
    for id in &ordered {
        let Some(service) = services.iter().find(|s| s["id"] == *id) else {
            continue;
        };
        let card = dom::el("article", "system-card", "");
        dom::add(&card, &dom::el("h4", "", id));
        dom::add(
            &card,
            &dom::el("p", "", &m::text(&service["declared"], "description")),
        );
        dom::add(
            &card,
            &dom::el(
                "p",
                "system-card-meta",
                &format!(
                    "{} · {}",
                    m::text(&service["declared"], "owner"),
                    m::text(service, "root")
                ),
            ),
        );
        for cap in m::array(&service["declared"]["capabilities"]) {
            dom::add(
                &card,
                &dom::el(
                    "p",
                    "system-card-meta",
                    &format!(
                        "{} · {} — {} · sources {}",
                        m::text(&cap, "id"),
                        m::text(&cap, "status"),
                        m::text(&cap, "description"),
                        joined(&cap["sources"])
                    ),
                ),
            );
        }
        if service["declared"]["relationships_complete"] == false {
            dom::add(
                &card,
                &dom::el(
                    "p",
                    "system-card-warning",
                    "Relationships are declared incomplete. Confirm missing dependencies.",
                ),
            );
        }
        let svc_graph = &graph_status["services"][id.as_str()];
        let slot = dom::el("div", "system-service-graph-slot", "");
        let _ = slot.set_attribute("id", &format!("system-service-graph-slot-{id}"));
        slot.set_inner_html(&crate::graph::render_service_graph_pill_html(id, svc_graph));
        dom::add(&card, &slot);

        let svc_state = svc_graph["state"].as_str().unwrap_or("off");
        let build_label = if matches!(svc_state, "off" | "empty" | "") {
            "Enable code graph"
        } else {
            "Rebuild graph"
        };
        let actions = dom::el("div", "skills-actions system-card-actions", "");
        let build_btn = dom::button(
            build_label,
            "system-service-graph-build",
            json!({ "service": id, "state": svc_state }),
        );
        build_btn.set_class_name("button compact");
        if matches!(svc_state, "building" | "missing") || service["access"] != "available" {
            let _ = build_btn.set_attribute("disabled", "");
        }
        let map_btn = dom::button(
            "Code map",
            "system-service-code-map",
            json!({ "service": id }),
        );
        map_btn.set_class_name("button compact");
        let changes_btn = dom::button(
            "Changes on map",
            "system-service-changes-map",
            json!({ "service": id }),
        );
        changes_btn.set_class_name("button compact");
        dom::add(&actions, &build_btn);
        dom::add(&actions, &map_btn);
        dom::add(&actions, &changes_btn);
        dom::add(&card, &actions);

        dom::append("system-services", &card);
        let choice = dom::check(id, "system-service", id, chosen.iter().any(|v| v == id));
        if service["access"] != "available" {
            if let Ok(Some(input)) = choice.query_selector("input") {
                let _ = input.set_attribute("disabled", "");
            }
        }
        dom::append("system-plan-services", &choice);
    }

    system_map::refresh_services_map(&project, &config, catalog, &graph_status);
    sync_changes_map_workspace();

    dom::clear("system-relationships");
    let warnings = m::array(&catalog["warnings"]);
    if edges.is_empty() && warnings.is_empty() {
        dom::append(
            "system-relationships",
            &dom::el("p", "system-note", "No cross-service contracts declared."),
        );
    } else {
        for edge in &edges {
            let view = build_relationship_view(edge);
            let wrap = dom::el("div", "system-contract-row", "");
            let row = dom::el("div", "step-row", "");
            dom::add(&row, &dom::el("span", "system-chip", &view.provider));
            dom::add(&row, &dom::el("span", "step-target", "→"));
            dom::add(&row, &dom::el("span", "system-chip", &view.consumer));
            dom::add(&row, &dom::el("strong", "step-name", &view.contract));
            if !view.version.is_empty() {
                dom::add(&row, &dom::el("span", "system-badge", &view.version));
            }
            if !view.provenance.is_empty() {
                dom::add(&row, &dom::el("span", "system-chip", &view.provenance));
            }
            dom::add(&wrap, &row);
            dom::append("system-relationships", &wrap);
        }
        for warning in warnings {
            dom::append(
                "system-relationships",
                &dom::el("p", "system-warning", &warning_text(&warning)),
            );
        }
    }
    dom::clear("system-banks");
    for service in services {
        let sid = m::text(&service, "id");
        let root = m::text(&service, "root");
        let access = m::text(&service, "access");
        let gstate = graph_status["services"][&sid]["state"]
            .as_str()
            .unwrap_or("off");
        let bank_card = dom::el("article", "system-card", "");
        dom::add(&bank_card, &dom::el("h5", "", &sid));
        let dl = metric_list(&[
            ("Project Brain", format!("{root}/project-brain"), false),
            ("Memory Bank", format!("{root}/memory-bank"), false),
            (
                "Access",
                if access.is_empty() {
                    "unknown".to_owned()
                } else {
                    m::label(&access)
                },
                access != "available",
            ),
            (
                "Code graph",
                m::label(gstate),
                matches!(gstate, "off" | "empty" | "missing"),
            ),
        ]);
        dom::add(&bank_card, &dl);
        dom::append("system-banks", &bank_card);
    }
    controls();
}
fn build_receipt_render_items(receipts: &[V], steps: &[V], overlaps: &[OverlapRow]) -> Vec<V> {
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
        items.push(json!({
            "kind": "receipt",
            "key": key,
            "receipt": receipt,
            "accepted": accepted,
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

fn build_receipt_card_element(item: &V) -> web_sys::Element {
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

    let raw = serde_json::to_string_pretty(receipt).unwrap_or_default();
    let details = dom::details("Receipt JSON", &raw);
    let _ = details.set_attribute("data-restore-key", "receipt-json");
    if let Some(summary) = details.first_element_child() {
        let _ = summary.set_attribute("data-focus-key", "receipt-json-summary");
    }
    dom::add(&card, &details);
    card
}

fn render_run() {
    let v = m::data("system-run");
    if !v.is_object() {
        dom::hidden("system-run", true);
        sync_changes_map_workspace();
        return;
    }
    dom::hidden("system-run", false);
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
    dom::pretty("system-plan-json", &v["plan"]);
    dom::clear("system-plan-summary");
    let participants = joined(&v["plan"]["context"]["services"]);
    let shared_mem = shared_memory_summary(&v["plan"]);
    for (term, value, is_zero) in [
        ("Participants", participants.clone(), participants == "none"),
        ("Context", context_summary(&v["plan"]), false),
        ("Shared memory", shared_mem.clone(), shared_mem == "none"),
        ("System file", m::text(&v, "config_path"), false),
    ] {
        dom::append("system-plan-summary", &dom::el("dt", "", term));
        let dd = dom::el("dd", "", &value);
        if is_zero {
            let _ = dd.set_attribute("data-zero", "true");
        }
        dom::append("system-plan-summary", &dd);
    }
    let steps = if v["execution"]["steps"].is_array() {
        m::array(&v["execution"]["steps"])
    } else {
        m::array(&v["plan"]["steps"])
    };
    let running = m::yes(&v["active"]);
    let step_entries = steps
        .iter()
        .enumerate()
        .map(|(idx, step)| json!({"idx": idx, "step": step}))
        .collect::<Vec<_>>();
    dom::keyed_render(
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
            dom::add(&item, &body);
            item
        },
    );
    if steps.is_empty() {
        dom::append(
            "system-flow",
            &dom::el("li", "system-stage", "No planned steps yet."),
        );
    }
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
    let receipt_items = build_receipt_render_items(&receipts, &steps, &overlaps);
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
    let events_text = m::array(&v["events"])
        .iter()
        .filter(|v| v["kind"] != "user")
        .map(|v| m::text(v, "text"))
        .collect::<Vec<_>>()
        .join("\n");
    dom::text("system-events", &events_text);
    dom::hidden("system-events-log", events_text.trim().is_empty());
    dom::hidden("system-open-session", !v["session_id"].is_string());
    sync_changes_map_workspace();
    controls();
}
async fn mutate(action: &str, extra: V) {
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
                dom::later("system-poll", 1100, move || {
                    spawn_local(async move {
                        read_scoped(&id, &project, generation).await;
                    })
                });
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
async fn plan() {
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
pub async fn editor_open() {
    if m::yes(&m::data("editor-pending")) {
        return;
    }
    if !sync_editor_target() {
        return;
    }
    if m::yes(&m::data("editor-dirty")) && editor_ready() {
        dom::hidden("system-editor", false);
        layout();
        return;
    }
    clear_editor_notices();
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let generation = m::bind_scope("editor-scope", &project, &config);
    m::bind_pending("editor-pending", generation);
    controls();
    match api::post(
        "/api/systems/editor",
        json!({"project_id":project,"config_path":config}),
    )
    .await
    {
        Ok(value)
            if editor_current(&project, &config, generation)
                && value["project_id"] == project
                && value["config_path"] == config =>
        {
            m::set("editor-generation", json!(generation));
            m::set("editor", value);
            m::set("editor-dirty", json!(false));
            render_editor();
            restore_discovery().await;
        }
        Err(error) if editor_current(&project, &config, generation) => {
            dom::hidden("system-editor", false);
            dom::error("system-editor-error", &error.message);
        }
        _ => {}
    }
    if m::finish_pending("editor-pending", generation) {
        m::set("editor-pending", m::data("discovery-active"));
        controls();
    }
}
fn field(
    parent: &web_sys::Element,
    label: &str,
    value: &V,
    path: &str,
    kind: &str,
    choices: &[&str],
    constraints: (bool, usize),
) {
    let (required, limit) = constraints;
    let wrap = dom::el(
        "label",
        if kind == "checkbox" {
            "checkbox"
        } else if kind == "lines" || kind == "textarea" {
            "field wide"
        } else {
            "field"
        },
        if kind == "checkbox" { "" } else { label },
    );
    let input = dom::el(
        if !choices.is_empty() {
            "select"
        } else if kind == "lines" || kind == "textarea" {
            "textarea"
        } else {
            "input"
        },
        "",
        "",
    );
    let _ = input.set_attribute("data-field", &format!("{path}|{kind}"));
    let focus_key = if let Some((_, tail)) = path.split_once("/passport/") {
        format!("field:passport/{tail}")
    } else {
        format!("field:{path}")
    };
    let _ = input.set_attribute("data-focus-key", &focus_key);
    if required {
        let _ = input.set_attribute("required", "");
    }
    if choices.is_empty() {
        let _ = input.set_attribute("maxlength", &limit.to_string());
    }
    if kind == "checkbox" {
        let _ = input.set_attribute("type", "checkbox");
        if let Some(i) = input.dyn_ref::<web_sys::HtmlInputElement>() {
            i.set_checked(m::yes(value));
        }
    } else {
        for choice in choices {
            let e = dom::el("option", "", choice);
            let _ = e.set_attribute("value", choice);
            dom::add(&input, &e);
        }
        dom::set_element_value(
            &input,
            &if kind == "lines" {
                m::array(value)
                    .iter()
                    .map(m::string)
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                m::string(value)
            },
        );
    }
    dom::add(&wrap, &input);
    if kind == "checkbox" {
        // A checkbox reads as one line: the box, then its statement.
        dom::add(&wrap, &dom::el("span", "", label));
    }
    dom::add(parent, &wrap);
}
/// A service card is named by its ID; before one is entered, by its folder.
fn service_title(passport: &V, path: &str) -> String {
    let id = m::text(passport, "id");
    if !id.trim().is_empty() {
        return id;
    }
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .map(|name| format!("New service · {name}"))
        .unwrap_or_else(|| "New service".into())
}
/// One-line singular name of a row in an editor section, used in headings and
/// button names so that "Remove" and "Add" say what they act on.
fn row_name(kind: &str) -> &'static str {
    match kind {
        "capabilities" => "Capability",
        "provides" => "Provided contract",
        "consumes" => "Consumed contract",
        _ => "Source",
    }
}
fn build_rows_details(
    title: &str,
    values: &V,
    path: &str,
    kind: &str,
    restore_key: &str,
) -> web_sys::Element {
    let d = dom::el("details", "system-editor-section", "");
    let _ = d.set_attribute("data-restore-key", restore_key);
    let summary = dom::el(
        "summary",
        "",
        &format!("{title} ({})", m::array(values).len()),
    );
    let _ = summary.set_attribute("data-focus-key", &format!("summary:{restore_key}"));
    dom::add(&d, &summary);
    let name = row_name(kind);
    for (index, value) in m::array(values).into_iter().enumerate() {
        let item = dom::el("div", "system-editor-row", "");
        let head = dom::el("div", "system-editor-row-head", "");
        let base = format!("{path}/{index}");
        dom::add(&head, &dom::el("h6", "", &format!("{name} {}", index + 1)));
        let remove = dom::button(
            "Remove",
            "editor-row-remove",
            json!({"path":path,"index":index}),
        );
        let _ = remove.set_attribute("class", "button quiet");
        let _ = remove.set_attribute(
            "aria-label",
            &format!("Remove {} {}", name.to_lowercase(), index + 1),
        );
        let _ = remove.set_attribute("data-focus-key", &format!("remove:{kind}:{index}"));
        dom::add(&head, &remove);
        dom::add(&item, &head);
        // Field helpers below write into the row's two-column grid.
        let row = dom::el("div", "system-editor-grid", "");
        dom::add(&item, &row);
        match kind {
            "capabilities" => {
                field(
                    &row,
                    "Capability ID",
                    &value["id"],
                    &format!("{base}/id"),
                    "text",
                    &[],
                    (true, 80),
                );
                field(
                    &row,
                    "Status",
                    &value["status"],
                    &format!("{base}/status"),
                    "text",
                    &["unknown", "planned", "partial", "implemented"],
                    (false, 200),
                );
                field(
                    &row,
                    "Description",
                    &value["description"],
                    &format!("{base}/description"),
                    "textarea",
                    &[],
                    (true, 2000),
                );
                field(
                    &row,
                    "Source paths (one per line)",
                    &value["sources"],
                    &format!("{base}/sources"),
                    "lines",
                    &[],
                    (false, 102400),
                );
                field(
                    &row,
                    "Routing keywords (one per line)",
                    &value["keywords"],
                    &format!("{base}/keywords"),
                    "lines",
                    &[],
                    (false, 102400),
                );
            }
            "provides" => {
                field(
                    &row,
                    "Contract ID",
                    &value["id"],
                    &format!("{base}/id"),
                    "text",
                    &[],
                    (true, 80),
                );
                field(
                    &row,
                    "Contract type",
                    &value["kind"],
                    &format!("{base}/kind"),
                    "text",
                    &["http", "event", "rpc", "graphql", "other"],
                    (false, 200),
                );
                field(
                    &row,
                    "Contract version",
                    &value["version"],
                    &format!("{base}/version"),
                    "text",
                    &[],
                    (true, 100),
                );
                field(
                    &row,
                    "Source paths (one per line)",
                    &value["sources"],
                    &format!("{base}/sources"),
                    "lines",
                    &[],
                    (false, 102400),
                );
            }
            "consumes" => {
                for (key, label, limit) in [
                    ("service", "Provider service ID", 80),
                    ("contract", "Consumed contract ID", 80),
                    ("version", "Contract version", 100),
                ] {
                    field(
                        &row,
                        label,
                        &value[key],
                        &format!("{base}/{key}"),
                        "text",
                        &[],
                        (true, limit),
                    );
                }
            }
            _ => {
                field(
                    &row,
                    "Source path",
                    &value["path"],
                    &format!("{base}/path"),
                    "text",
                    &[],
                    (true, 1024),
                );
                field(
                    &row,
                    "Source kind",
                    &value["kind"],
                    &format!("{base}/kind"),
                    "text",
                    &["policy", "spec", "contract", "code", "test", "memory"],
                    (false, 200),
                );
            }
        }
        dom::add(&d, &item);
    }
    let add_btn = dom::button(
        &format!("Add {}", name.to_lowercase()),
        "editor-row-add",
        json!({"path":path,"kind":kind}),
    );
    let _ = add_btn.set_attribute("data-focus-key", &format!("add:{kind}"));
    dom::add(&d, &add_btn);
    d
}

fn rows(parent: &web_sys::Element, title: &str, values: &V, path: &str, kind: &str) {
    dom::add(parent, &build_rows_details(title, values, path, kind, kind));
}

fn build_editor_service_items(services: &[V]) -> Vec<V> {
    let mut seen_paths = BTreeMap::<String, usize>::new();
    services
        .iter()
        .enumerate()
        .map(|(index, service)| {
            let path = m::text(service, "path");
            let base = if path.is_empty() {
                format!("idx:{index}")
            } else {
                format!("path:{path}")
            };
            let count = seen_paths.entry(base.clone()).or_insert(0);
            let key = if *count == 0 {
                base
            } else {
                format!("{base}#{}", *count)
            };
            *count += 1;
            json!({
                "key": key,
                "index": index,
                "service": service,
            })
        })
        .collect()
}

fn build_editor_service_card(item: &V) -> web_sys::Element {
    let index = item["index"].as_u64().unwrap_or(0) as usize;
    let service = &item["service"];
    let base = format!("/services/{index}/passport");
    let p = &service["passport"];
    let path = m::text(service, "path");
    let card = dom::el("section", "system-editor-service", "");
    let head = dom::el("div", "system-editor-service-head", "");
    let title = dom::el("div", "", "");
    dom::add(&title, &dom::el("h5", "", &service_title(p, &path)));
    dom::add(&title, &dom::el("p", "system-note", &path));
    dom::add(&head, &title);
    let actions = dom::el("div", "system-editor-service-actions", "");
    for (label, action, payload, focus_key) in [
        (
            "Choose another folder",
            "editor-service-folder",
            json!(index),
            "service-folder",
        ),
        (
            "Remove service",
            "editor-row-remove",
            json!({"path":"/services","index":index}),
            "service-remove",
        ),
    ] {
        let button = dom::button(label, action, payload);
        let _ = button.set_attribute("class", "button quiet");
        let _ = button.set_attribute("data-focus-key", focus_key);
        dom::add(&actions, &button);
    }
    dom::add(&head, &actions);
    dom::add(&card, &head);
    let grid = dom::el("div", "system-editor-grid", "");
    for (key, label, kind, required, limit) in [
        ("id", "Service ID", "text", true, 80),
        ("owner", "Owning team", "text", true, 200),
        ("description", "Service description", "textarea", true, 2000),
    ] {
        field(
            &grid,
            label,
            &p[key],
            &format!("{base}/{key}"),
            kind,
            &[],
            (required, limit),
        );
    }
    dom::add(&card, &grid);
    field(
        &card,
        "I have described all known dependencies",
        &p["relationships_complete"],
        &format!("{base}/relationships_complete"),
        "checkbox",
        &[],
        (false, 200),
    );
    for (key, title) in [
        ("capabilities", "Capabilities"),
        ("provides", "Provided contracts"),
        ("consumes", "Consumed contracts"),
        ("sources", "Sources of context"),
    ] {
        rows(&card, title, &p[key], &format!("{base}/{key}"), key);
    }
    card
}

fn render_editor() {
    let data = m::data("editor");
    if !data.is_object() {
        return;
    }
    dom::hidden("system-editor", false);
    let name = m::text(&data, "name");
    if dom::value("system-editor-name") != name {
        dom::set_value("system-editor-name", &name);
    }
    let location = format!(
        "{} / {}",
        m::text(&m::project(&m::text(&data, "project_id")), "path"),
        m::text(&data, "config_path")
    );
    if dom::get("system-editor-location")
        .and_then(|e| e.text_content())
        .as_deref()
        != Some(location.as_str())
    {
        dom::text("system-editor-location", &location);
    }
    let service_items = build_editor_service_items(&m::array(&data["services"]));
    dom::keyed_render(
        "system-editor-services",
        &service_items,
        |item| m::text(item, "key"),
        build_editor_service_card,
    );
    let shared_items = [json!({
        "key": "shared_sources",
        "values": data["shared_sources"],
    })];
    dom::keyed_render(
        "system-editor-shared",
        &shared_items,
        |item| m::text(item, "key"),
        |item| {
            build_rows_details(
                "Shared sources",
                &item["values"],
                "/shared_sources",
                "sources",
                "shared_sources",
            )
        },
    );
    controls();
}
fn pointer_token(token: &str) -> Option<String> {
    let mut decoded = String::new();
    let mut chars = token.chars();
    while let Some(ch) = chars.next() {
        if ch == '~' {
            decoded.push(match chars.next()? {
                '0' => '~',
                '1' => '/',
                _ => return None,
            });
        } else {
            decoded.push(ch);
        }
    }
    Some(decoded)
}
fn set_editor_value(data: &mut V, path: &str, value: V) -> bool {
    let Some(tokens) = path.strip_prefix('/') else {
        return false;
    };
    if tokens
        .split('/')
        .any(|token| pointer_token(token).is_none())
    {
        return false;
    }
    let Some((parent, leaf)) = path.rsplit_once('/') else {
        return false;
    };
    let Some(leaf) = pointer_token(leaf) else {
        return false;
    };
    if let Some(slot) = data.pointer_mut(path) {
        *slot = value;
        return true;
    }
    let Some(object) = data.pointer_mut(parent).and_then(V::as_object_mut) else {
        return false;
    };
    object.insert(leaf, value);
    true
}
pub fn edit_field(field: &str, value: &str, checked: bool) {
    if !editor_ready() || m::yes(&m::data("editor-pending")) {
        return;
    }
    if m::yes(&m::data("editor-pending")) {
        return;
    }
    let (path, kind) = field.rsplit_once('|').unwrap_or((field, "text"));
    let mut data = m::data("editor");
    let v = if kind == "checkbox" {
        json!(checked)
    } else if kind == "lines" {
        json!(value
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>())
    } else {
        json!(value)
    };
    if set_editor_value(&mut data, path, v) {
        m::set("editor", data);
        changed();
    }
}
fn changed() {
    m::set("editor-dirty", json!(true));
    dom::text("system-editor-status", "Unsaved changes");
    controls();
}
async fn service_folder(index: Option<u64>) {
    if !editor_ready() || m::yes(&m::data("editor-pending")) {
        return;
    }
    let generation = m::read(|s| s.epochs.get("editor-scope").copied().unwrap_or(0));
    m::set(
        "service-folder-owner",
        json!({"project":dom::value("system-project"),"config":dom::value("system-config"),"generation":generation}),
    );
    let purpose = index
        .map(|i| format!("service:{i}"))
        .unwrap_or_else(|| "service:".into());
    management::folder_open_at(
        &purpose,
        &service_folder_start(index),
        "Choose a service folder",
    )
    .await;
}
/// Where the service folder browser opens: the replaced service's own folder; for a
/// new service, the folder that holds the last added one (services are usually
/// siblings); for the first, the system folder.
fn service_folder_start(index: Option<u64>) -> String {
    let services = m::array(&m::data("editor")["services"]);
    if let Some(service) = index.and_then(|index| services.get(index as usize)) {
        return m::text(service, "path");
    }
    if let Some(last) = services.last() {
        let path = m::text(last, "path");
        if let Some((parent, _)) = path.trim_end_matches(['/', '\\']).rsplit_once(['/', '\\']) {
            if !parent.is_empty() {
                return parent.to_owned();
            }
        }
    }
    m::text(&m::project(&dom::value("system-project")), "path")
}
pub async fn add_service(project: &str, index: Option<usize>) {
    let selected = m::take("service-folder-owner");
    if !editor_current(
        &m::text(&selected, "project"),
        &m::text(&selected, "config"),
        selected["generation"].as_u64().unwrap_or(0),
    ) {
        dom::error(
            "system-editor-error",
            "The selected system changed. Choose the service folder again.",
        );
        return;
    }
    if m::yes(&m::data("editor-pending")) || !editor_ready() {
        return;
    }
    let owner = dom::value("system-project");
    let config = dom::value("system-config");
    let generation = m::bind_scope("editor-scope", &owner, &config);
    m::set("editor-generation", json!(generation));
    m::bind_pending("editor-pending", generation);
    controls();
    match api::post("/api/systems/service", json!({"project_id":project})).await {
        Ok(mut service) if editor_current(&owner, &config, generation) && editor_ready() => {
            let mut data = m::data("editor");
            if let Some(services) = data["services"].as_array_mut() {
                if let Some(index) = index {
                    if index < services.len() {
                        if service["fingerprint"].is_null() {
                            service["passport"] = services[index]["passport"].clone();
                        }
                        services[index] = service;
                    }
                } else {
                    services.push(service);
                }
            }
            m::set("editor", data);
            changed();
            render_editor();
        }
        Err(error) if editor_current(&owner, &config, generation) => {
            dom::error("system-editor-error", &error.message)
        }
        _ => {}
    }
    if m::finish_pending("editor-pending", generation) {
        controls();
    }
}
fn editor_payload() -> V {
    let mut data = m::data("editor");
    if let Some(items) = data["services"].as_array_mut() {
        for item in items {
            if let Some(map) = item.as_object_mut() {
                map.remove("path");
            }
        }
    }
    data
}
async fn save_editor() {
    if m::yes(&m::data("editor-pending")) || !dom::validity("system-editor") {
        return;
    }
    if !editor_ready() {
        dom::error(
            "system-editor-error",
            "Load the current project's system before saving.",
        );
        return;
    }
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let payload = editor_payload();
    let generation = m::bind_scope("editor-scope", &project, &config);
    m::set("editor-generation", json!(generation));
    m::next("discovery");
    m::bind_pending("editor-pending", generation);
    controls();
    let mut saved = false;
    match api::post("/api/systems/preview", payload).await {
        Ok(preview) if editor_current(&project, &config, generation) && editor_ready() => {
            if let Some(id) = preview["preview_id"].as_str() {
                // The file preview is reviewed for this same bound target.
                if editor_current(&project, &config, generation) && editor_ready() {
                    match api::post("/api/systems/apply", json!({"preview_id":id})).await {
                        Ok(value)
                            if editor_current(&project, &config, generation)
                                && value["editor"]["project_id"] == project
                                && value["editor"]["config_path"] == config =>
                        {
                            m::set("editor", value["editor"].clone());
                            m::set("editor-dirty", json!(false));
                            m::set("system-catalog", value.clone());
                            render_catalog(&value);
                            remember_discovery(None);
                            saved = true;
                        }
                        Err(error) if editor_current(&project, &config, generation) => {
                            dom::error("system-editor-error", &error.message)
                        }
                        _ => {}
                    }
                }
            } else {
                dom::error(
                    "system-editor-error",
                    "The runner returned no system preview.",
                );
            }
        }
        Err(error) if editor_current(&project, &config, generation) => {
            dom::error("system-editor-error", &error.message)
        }
        _ => {}
    }
    if m::finish_pending("editor-pending", generation) {
        if editor_current(&project, &config, generation) {
            if saved {
                dom::hidden("system-editor", true);
                dom::text(
                    "system-message",
                    "System saved. The map shows the saved services.",
                );
            } else {
                dom::text(
                    "system-editor-status",
                    "Save did not complete. The form is retained; reload metadata before retrying if files changed.",
                );
            }
        }
        controls();
    }
}
fn discovery_key() -> String {
    let d = m::data("editor");
    format!(
        "harness-discovery:{}:{}",
        m::text(&d, "project_id"),
        m::text(&d, "config_path")
    )
}
fn remember_discovery(id: Option<&str>) {
    dom::save(&discovery_key(), id);
}
async fn discovery_start() {
    if m::yes(&m::data("editor-pending")) || !editor_ready() {
        return;
    }
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let generation = m::bind_scope("editor-scope", &project, &config);
    m::set("editor-generation", json!(generation));
    let epoch = m::next("discovery");
    m::set(
        "discovery-owner",
        json!({"project":project,"config":config,"generation":generation}),
    );
    m::set("editor-pending", json!(true));
    m::set("discovery-active", json!(true));
    controls();
    dom::text(
        "system-discovery-status",
        "Capturing service evidence and queueing AI scan…",
    );
    match api::post("/api/system-discoveries",json!({"editor":editor_payload(),"provider":dom::value("system-discovery-provider"),"timeout":dom::num("system-discovery-timeout")})).await {
        Ok(value) if discovery_current(epoch) => {
            let id=m::text(&value,"id");
            m::set("discovery-id",json!(id));
            remember_discovery(Some(&id));
            session::refresh().await;
            if discovery_current(epoch) { discovery_poll(epoch).await; }
        }
        Err(error) if discovery_current(epoch) => {
            dom::error("system-editor-error",&error.message);
            m::set("editor-pending",json!(false));
            m::set("discovery-active",json!(false));
            controls();
        }
        _ => {}
    }
}
/// What a proposed passport changes from the one the form held before the
/// scan: the proposal replaces the whole form, so a scan of a system whose
/// passports were edited by hand says what Save would overwrite.
fn passport_changes(before: &V, after: &V) -> String {
    if !before.is_object() {
        return "new passport".into();
    }
    let changed = [
        "description",
        "owner",
        "relationships_complete",
        "capabilities",
        "provides",
        "consumes",
        "sources",
    ]
    .iter()
    .filter(|key| before[**key] != after[**key])
    .map(
        |key| match (before[*key].as_array(), after[*key].as_array()) {
            (Some(old), Some(new)) if old.len() != new.len() => {
                format!("{} {} → {}", m::label(key), old.len(), new.len())
            }
            _ => m::label(key),
        },
    )
    .collect::<Vec<_>>();
    if changed.is_empty() {
        "same as saved".into()
    } else {
        format!("changes {}", changed.join(", "))
    }
}
fn proposal(v: &V, draft: &V) {
    if !editor_ready()
        || v["editor"]["project_id"] != dom::value("system-project")
        || v["editor"]["config_path"] != dom::value("system-config")
    {
        return;
    }
    if v["editor"].is_object() {
        m::set("editor", v["editor"].clone());
        changed();
        render_editor();
    }
    dom::clear("system-discovery-report");
    dom::hidden("system-discovery-report", false);
    dom::append(
        "system-discovery-report",
        &dom::el(
            "p",
            "system-note",
            "AI filled the forms. Verify sources, ownership and dependencies before saving. Complete dependency coverage remains unconfirmed.",
        ),
    );
    for warning in m::array(&v["warnings"]) {
        dom::append(
            "system-discovery-report",
            &dom::el("p", "system-warning", &m::string(&warning)),
        );
    }
    for report in m::array(&v["reports"]) {
        let saved = m::array(&draft["services"])
            .into_iter()
            .find(|service| service["passport"]["id"] == report["id"])
            .map(|service| service["passport"].clone())
            .unwrap_or(V::Null);
        let d = dom::details(
            &format!(
                "{} · {} capabilities · {} provided · {} consumed contracts · {}",
                m::text(&report, "id"),
                m::array(&report["passport"]["capabilities"]).len(),
                m::array(&report["passport"]["provides"]).len(),
                m::array(&report["passport"]["consumes"]).len(),
                passport_changes(&saved, &report["passport"])
            ),
            &serde_json::to_string_pretty(&report).unwrap_or_default(),
        );
        dom::add(
            &d,
            &dom::details(
                "Captured evidence inventory",
                &serde_json::to_string_pretty(&v["inventory"][m::text(&report, "id")])
                    .unwrap_or_default(),
            ),
        );
        dom::append("system-discovery-report", &d);
    }
}
async fn discovery_poll(epoch: u64) {
    if !discovery_current(epoch) {
        return;
    }
    dom::cancel_timer("discovery-poll");
    let id = m::string(&m::data("discovery-id"));
    if id.is_empty() {
        return;
    }
    match api::get(&format!("/api/system-discoveries/{}", dom::enc(&id))).await {
        Ok(job) if discovery_current(epoch) && job["id"] == id => {
            let active = m::yes(&job["active"]);
            m::set("discovery-active", json!(active));
            m::set("editor-pending", json!(active));
            dom::text(
                "system-discovery-status",
                &format!(
                    "AI scan · {} · {}",
                    m::text(&job, "provider"),
                    m::label(job["status"].as_str().unwrap_or(""))
                ),
            );
            dom::hidden("system-discovery-agents", false);
            dom::hidden("system-discovery-live", !active);
            discovery_agents(&job).await;
            if !discovery_current(epoch) {
                return;
            }
            if active {
                dom::later("discovery-poll", 1000, move || {
                    spawn_local(async move {
                        discovery_poll(epoch).await;
                    })
                });
            } else {
                if job["proposal"].is_object() {
                    proposal(&job["proposal"], &job["draft"]);
                    dom::text(
                        "system-editor-status",
                        "AI filled service details. Review sources and uncertainties, then Save system.",
                    );
                } else {
                    dom::text(
                        "system-discovery-status",
                        &format!("{} Your original form is retained.", scan_end(&job)),
                    );
                }
                session::refresh().await;
            }
            if discovery_current(epoch) {
                controls();
            }
        }
        Err(error) if discovery_current(epoch) => {
            if [400, 404].contains(&error.status) {
                let unavailable = match api::get(&format!("/api/sessions/{}", dom::enc(&id))).await
                {
                    Ok(value) => !m::active(&value["session"]),
                    Err(missing) => [400, 404].contains(&missing.status),
                };
                if !discovery_current(epoch) {
                    return;
                }
                if unavailable {
                    m::set("discovery-active", json!(false));
                    m::set("editor-pending", json!(false));
                    changed();
                    remember_discovery(None);
                    dom::text(
                        "system-discovery-status",
                        "AI scan is no longer available. The original form is retained.",
                    );
                    controls();
                    return;
                }
            }
            if !discovery_current(epoch) {
                return;
            }
            dom::text(
                "system-discovery-status",
                &format!("Reconnecting to AI scan: {}", error.message),
            );
            dom::later("discovery-poll", 3000, move || {
                spawn_local(async move {
                    discovery_poll(epoch).await;
                })
            });
        }
        _ => {}
    }
}
async fn restore_discovery() {
    if !editor_ready() {
        return;
    }
    let key = discovery_key();
    let Some(id) = dom::storage(&key) else {
        return;
    };
    let owner = m::data("editor-scope-owner");
    let project = m::text(&owner, "owner");
    let config = m::text(&owner, "item");
    let generation = m::read(|s| s.epochs.get("editor-scope").copied().unwrap_or(0));
    let epoch = m::next("discovery");
    m::set(
        "discovery-owner",
        json!({"project":project,"config":config,"generation":generation}),
    );
    match api::get(&format!("/api/system-discoveries/{}", dom::enc(&id))).await {
        Ok(job) if discovery_current(epoch) && key == discovery_key() && job["id"] == id => {
            m::set("discovery-id", json!(id));
            discovery_agents(&job).await;
            if !discovery_current(epoch) {
                return;
            }
            let active = m::yes(&job["active"]);
            m::set("discovery-active", json!(active));
            m::set("editor-pending", json!(active));
            let draft = &job["draft"];
            let same_target = draft["project_id"] == project && draft["config_path"] == config;
            if active && same_target {
                m::set("editor", draft.clone());
                changed();
                render_editor();
                discovery_poll(epoch).await;
            } else if job["proposal"].is_object() {
                proposal(&job["proposal"], &job["draft"]);
            } else if same_target && draft["revision"] == m::data("editor")["revision"] {
                m::set("editor", draft.clone());
                changed();
                render_editor();
            }
        }
        Err(error) if discovery_current(epoch) => dom::text(
            "system-discovery-status",
            &format!("Previous AI scan unavailable: {}", error.message),
        ),
        _ => {}
    }
}
fn resolve_agent_launch_selection(
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

fn build_agent_launch_options(launches: &[V]) -> Vec<V> {
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

fn sync_agent_launch_options(options: &[V], selected: Option<&str>) {
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

async fn agents(v: &V) {
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
fn context_summary(plan: &V) -> String {
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
fn shared_memory_summary(plan: &V) -> String {
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
fn handoff_memory(handoff: &V) -> String {
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
fn service_agent_goal(service: &str, access: &V, mode: &V) -> String {
    let reads = "reads the system folder and every service of this change";
    if mode != "edit" {
        format!("Investigates {service}; {reads}.")
    } else if access == "all" {
        format!("Works on {service}; {reads} and edits any selected service.")
    } else {
        format!("Works on {service}; {reads} and edits only its own folder.")
    }
}
async fn discovery_agents(job: &V) {
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
            // The file of a receipt is shown in the code map of its service
            // that is embedded in this view, and the view is brought to it.
            let svc = m::text(payload, "service");
            let path = m::text(payload, "path");
            let is_changes_view = m::read(|s| s.view == "system-changes");
            if !svc.is_empty() && !path.is_empty() {
                let surf = if is_changes_view {
                    "changes"
                } else {
                    "services"
                };
                drill_down_service_map(surf, &svc, Some(&path));
                focus_system_graph_workspace();
            }
        }
        "system-execute" => {
            if dom::checked("system-reviewed") {
                mutate(
                    "execute",
                    json!({
                        "provider": dom::value("system-provider"),
                        "mode": dom::value("system-mode"),
                        "access": dom::value("system-access"),
                        "timeout": dom::num("system-timeout"),
                        "tell_agents_graph": dom::checked("system-graph-prompt"),
                    }),
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
            let mut extra = json!({
                "accept_source_changes": dom::checked("system-accept-changes"),
                "tell_agents_graph": dom::checked("system-graph-prompt"),
            });
            // Accepting the unrun checks completes verification; it is not retried.
            if dom::checked("system-accept-unrun") {
                extra["accept_unrun_checks"] = json!(true);
            } else if !dom::value("system-retry").is_empty() {
                extra["retry_step"] = json!(dom::value("system-retry"));
            }
            mutate("resume", extra).await;
        }
        "system-open-session" => {
            let sid = m::text(&m::data("system-run"), "session_id");
            session::select(&sid).await;
            session::open_view("sessions", true).await;
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

#[cfg(test)]
mod editor_value_tests {
    use super::*;
    fn partial_passport() -> V {
        json!({"project_id":"a","config_path":"system.json","services":[{"passport":{
            "schema_version":1,"id":"service-a","description":"Existing valid service","owner":"Team A",
            "capabilities":[{"id":"read","description":"Read records","status":"implemented","sources":[]}],
            "provides":[],"consumes":[],"sources":[]
        }}]})
    }
    #[test]
    fn edits_insert_optional_fields_of_an_existing_native_passport() {
        let mut data = partial_passport();
        assert!(data
            .pointer("/services/0/passport/relationships_complete")
            .is_none());
        assert!(data
            .pointer("/services/0/passport/capabilities/0/keywords")
            .is_none());
        assert!(set_editor_value(
            &mut data,
            "/services/0/passport/relationships_complete",
            json!(true)
        ));
        assert!(set_editor_value(
            &mut data,
            "/services/0/passport/capabilities/0/keywords",
            json!(["routing", "lookup"])
        ));
        assert_eq!(
            data["services"][0]["passport"]["relationships_complete"],
            true
        );
        assert_eq!(
            data["services"][0]["passport"]["capabilities"][0]["keywords"],
            json!(["routing", "lookup"])
        );
        assert_eq!(data["services"][0]["passport"]["owner"], "Team A");
        assert_eq!(data["project_id"], "a");
    }
    #[test]
    fn setter_decodes_final_object_keys_and_preserves_existing_array_indices() {
        let mut data = json!({"object":{},"array":[{"name":"before"}]});
        assert!(set_editor_value(
            &mut data,
            "/object/a~1b~0c",
            json!("created")
        ));
        assert_eq!(data["object"]["a/b~c"], "created");
        assert!(set_editor_value(&mut data, "/array/0/name", json!("after")));
        assert!(set_editor_value(&mut data, "/array/0/new", json!(7)));
        assert_eq!(data["array"][0], json!({"name":"after","new":7}));
        assert_eq!(data["array"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn missing_parents_invalid_pointers_and_array_growth_leave_the_document_unchanged() {
        let mut data = partial_passport();
        let before = data.clone();
        for path in [
            "",
            "services/0/passport/owner",
            "/services/1/passport/owner",
            "/services/-",
            "/services/0/missing/leaf",
            "/services/0/passport/invalid~2escape",
            "/services/0/passport/capabilities/01/keywords",
        ] {
            assert!(
                !set_editor_value(&mut data, path, json!("discarded")),
                "{path}"
            );
            assert_eq!(data, before, "{path}");
        }
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;
    fn segment(path: &str) -> Vec<i64> {
        path.split(|c: char| !c.is_ascii_digit() && c != '-')
            .filter(|part| !part.is_empty())
            .map(|part| part.parse().unwrap())
            .collect()
    }
    #[test]
    fn an_upward_edge_leaves_the_top_of_its_box_and_enters_the_bottom_of_the_target() {
        // provider in the second row, consumer above it in the first
        let points = segment(&edge_path((24, 148), (260, 24)));
        assert_eq!(points[1], 148, "starts at the provider's top edge");
        assert_eq!(points[3], 24 + 76, "ends at the consumer's bottom edge");
    }
    #[test]
    fn a_downward_edge_leaves_the_bottom_and_enters_the_top() {
        let points = segment(&edge_path((24, 24), (260, 148)));
        assert_eq!(points[1], 24 + 76);
        assert_eq!(points[3], 148);
    }
    #[test]
    fn a_pair_linked_both_ways_gets_two_separate_lines() {
        let forward = edge_path((24, 24), (260, 24));
        let backward = edge_path((260, 24), (24, 24));
        assert_ne!(segment(&forward)[1], segment(&backward)[1]);
        // each line runs between the facing sides, never across its own boxes
        assert_eq!(segment(&forward)[0], 24 + 190);
        assert_eq!(segment(&forward)[2], 260);
        assert_eq!(segment(&backward)[0], 260);
        assert_eq!(segment(&backward)[2], 24 + 190);
    }
    #[test]
    fn a_link_that_skips_boxes_in_its_row_arcs_around_them() {
        assert!(edge_path((24, 24), (496, 24)).contains('Q'));
        assert!(edge_path((496, 24), (24, 24)).contains('Q'));
        assert!(!edge_path((24, 24), (260, 24)).contains('Q'));
    }
    #[test]
    fn catalog_warnings_read_as_sentences() {
        assert_eq!(
            warning_text(&json!({"service":"orders","reason":"relationships_incomplete"})),
            "orders: dependencies are not confirmed complete"
        );
        assert_eq!(
            warning_text(&json!({"service":"__system__","reason":"dependency_cycle"})),
            "System: declared contract dependencies form a cycle"
        );
        assert_eq!(
            warning_text(
                &json!({"service":"billing","reason":"source_unavailable_or_ineligible","path":"src/a.php"})
            ),
            "billing: a registered source is missing or not eligible: src/a.php"
        );
        assert_eq!(
            warning_text(&json!({"service":"x","reason":"new_reason"})),
            "x: new reason"
        );
        assert_eq!(warning_text(&json!("plain text")), "plain text");
    }
    #[test]
    fn a_service_card_is_named_by_its_id_or_else_its_folder() {
        assert_eq!(
            service_title(&json!({"id":"orders"}), "/x/orders"),
            "orders"
        );
        assert_eq!(
            service_title(&json!({"id":""}), "/lab/harbor-payments/"),
            "New service · harbor-payments"
        );
        assert_eq!(service_title(&json!({"id":" "}), ""), "New service");
    }
    #[test]
    fn a_plan_without_services_explains_what_the_task_matched() {
        let hint = selection_hint(&[
            json!({"service":"orders","matches":[{"terms":["order","cancel"]},{"terms":["order"]}]}),
            json!({"service":"payments","matches":[{"terms":["refund"]}]}),
        ]);
        assert!(hint.contains("orders (cancel, order)"), "{hint}");
        assert!(hint.contains("payments (refund)"), "{hint}");
        assert!(selection_hint(&[]).contains("matches no declared capability"));
    }
    #[test]
    fn a_receipt_names_what_stopped_its_dispatch() {
        assert_eq!(receipt_outcome(&json!({"ok":true})), "completed");
        assert_eq!(
            receipt_outcome(&json!({"ok":true,"report":{"status":"completed","checks":[
                {"name":"reproduction","status":"failed"},{"name":"unit","status":"passed"}]}})),
            "completed; failed checks are findings: reproduction"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"error":"invalid_or_sensitive_worker_result",
                "rejection":"Read-only worker reported writes","report":null})
            ),
            "rejected: Read-only worker reported writes"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"error":"worker_blocked_or_verification_incomplete",
                "report":{"status":"blocked","summary":"Refund rule undecided","checks":[]}})
            ),
            "blocked by the agent: Refund rule undecided"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"phase":"service-inventory","error":"worker_blocked_or_verification_incomplete",
                "report":{"status":"completed","checks":[{"name":"tests","status":"passed"},{"name":"composer_metadata","status":"failed"}]}})
            ),
            "failed checks: composer_metadata"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"phase":"verify","error":"worker_blocked_or_verification_incomplete",
                "report":{"status":"completed","checks":[{"name":"go","status":"not_run"},{"name":"php","status":"passed"}]}})
            ),
            "checks not run: go"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"phase":"contracts","error":"read_only_worker_reported_writes",
                "report":{"status":"completed","checks":[],"changed_files":["billing/.tool/marker"]}})
            ),
            "read-only worker reported writes: billing/.tool/marker"
        );
        assert_eq!(
            receipt_outcome(&json!({"ok":false,"error":"worker_exit_failure","report":null})),
            "blocked: worker exit failure"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"error":"worker_exit_failure","report":null,
                "failure":"Failed to authenticate: OAuth session expired"})
            ),
            "the agent CLI failed: Failed to authenticate: OAuth session expired"
        );
    }
    #[test]
    fn a_stopped_step_with_a_successful_receipt_reads_as_reported() {
        let receipts = json!([
            {"phase":"service-orders","attempt":1,"ok":true},
            {"phase":"service-billing","attempt":1,"ok":false}
        ]);
        let orders = json!({"id":"service-orders","attempt":1,"status":"running"});
        assert_eq!(shown_status(&orders, &receipts, false), "reported");
        assert_eq!(shown_status(&orders, &receipts, true), "running");
        // A receipt of an earlier attempt says nothing of this one.
        let retried = json!({"id":"service-orders","attempt":2,"status":"interrupted"});
        assert_eq!(shown_status(&retried, &receipts, false), "interrupted");
        let billing = json!({"id":"service-billing","attempt":1,"status":"running"});
        assert_eq!(shown_status(&billing, &receipts, false), "interrupted");
        let blocked = json!({"id":"service-billing","attempt":1,"status":"blocked"});
        assert_eq!(shown_status(&blocked, &receipts, false), "blocked");
        let done = json!({"id":"contracts","attempt":1,"status":"completed"});
        assert_eq!(shown_status(&done, &receipts, false), "completed");
    }
    #[test]
    fn only_a_verification_whose_gaps_are_unrun_checks_offers_acceptance() {
        let step = json!({"id":"verify","attempt":2,"status":"blocked"});
        let receipt = |checks: V, status: &str| {
            json!([{"phase":"verify","attempt":2,"ok":false,"error":"worker_blocked_or_verification_incomplete",
                "rejection":null,"report":{"status":status,"checks":checks,"changed_files":[]}}])
        };
        let honest = receipt(
            json!([{"name":"unit","status":"passed"},{"name":"runtime_integration","status":"not_run"}]),
            "completed",
        );
        assert_eq!(
            unrun_checks(&step, &honest),
            Some(vec!["runtime_integration".to_string()])
        );
        let failed = receipt(
            json!([{"name":"unit","status":"failed"},{"name":"runtime_integration","status":"not_run"}]),
            "completed",
        );
        assert_eq!(unrun_checks(&step, &failed), None);
        let blocked = receipt(json!([{"name":"unit","status":"not_run"}]), "blocked");
        assert_eq!(unrun_checks(&step, &blocked), None);
        let nothing_ran = receipt(json!([{"name":"unit","status":"not_run"}]), "completed");
        assert_eq!(unrun_checks(&step, &nothing_ran), None);
        let earlier = json!({"id":"verify","attempt":3,"status":"blocked"});
        assert_eq!(unrun_checks(&earlier, &honest), None);
        let service = json!({"id":"service-orders","attempt":2,"status":"blocked"});
        assert_eq!(unrun_checks(&service, &honest), None);
    }
    #[test]
    fn a_queued_run_says_what_it_waits_for() {
        assert_eq!(
            run_status(&json!({"status":"queued","active":true})),
            "queued · waits for a free runner slot"
        );
    }
    #[test]
    fn a_stopped_run_says_what_stopped_it() {
        let run = |error: V, receipts: V| {
            json!({"status":"blocked","active":false,
                "execution":{"error":error,"steps":[
                    {"id":"contracts","status":"blocked","attempt":2}]},
                "receipts":receipts})
        };
        let agent = run(
            json!("worker_blocked_or_verification_incomplete"),
            json!([{"phase":"contracts","attempt":2,"ok":false,
                "error":"worker_blocked_or_verification_incomplete",
                "report":{"status":"blocked","summary":"The refund owner is undecided","checks":[]}}]),
        );
        assert_eq!(
            run_status(&agent),
            "blocked · contracts: blocked by the agent: The refund owner is undecided"
        );
        // Without a receipt of the stopped attempt the journal's error is shown.
        let stale = run(
            json!("worker_blocked_or_verification_incomplete"),
            json!([{"phase":"contracts","attempt":1,"ok":false,"error":"interrupted","report":null}]),
        );
        assert_eq!(
            run_status(&stale),
            "blocked · worker blocked or verification incomplete"
        );
        let restarted = run(V::Null, json!([]));
        assert_eq!(
            run_status(&restarted),
            "blocked · contracts stopped before it saved a receipt; inspect its folder, then retry it"
        );
        let sentence = run(
            json!("Sources changed between dispatches; prepare new plan"),
            json!([]),
        );
        assert_eq!(
            run_status(&sentence),
            "blocked · Sources changed between dispatches; prepare new plan"
        );
    }
    #[test]
    fn a_proposal_says_what_it_changes_from_the_saved_passport() {
        let saved = json!({"id":"orders","description":"Orders","owner":"@team",
            "capabilities":[{"id":"a"}],"provides":[],"consumes":[],"sources":[]});
        let mut proposed = saved.clone();
        assert_eq!(passport_changes(&saved, &proposed), "same as saved");
        proposed["owner"] = json!("unknown");
        proposed["capabilities"] = json!([{"id":"a"},{"id":"b"}]);
        assert_eq!(
            passport_changes(&saved, &proposed),
            "changes owner, capabilities 1 → 2"
        );
        assert_eq!(passport_changes(&V::Null, &proposed), "new passport");
    }
    #[test]
    fn a_scan_that_ended_without_a_proposal_says_why() {
        assert_eq!(
            scan_end(&json!({"status":"cancelled","error":null})),
            "AI scan cancelled."
        );
        assert_eq!(
            scan_end(
                &json!({"status":"failed","error":"The codex CLI exited with code 1: Not signed in"})
            ),
            "The codex CLI exited with code 1: Not signed in."
        );
        assert_eq!(
            scan_end(&json!({"status":"failed"})),
            "AI scan ended without a proposal."
        );
    }
    #[test]
    fn a_message_followed_by_another_ends_as_a_sentence() {
        assert_eq!(
            sentence("Failed to authenticate"),
            "Failed to authenticate."
        );
        assert_eq!(sentence("Scan ended. "), "Scan ended.");
    }
    #[test]
    fn a_revised_plan_gets_the_next_change_id() {
        assert_eq!(
            next_change_id("cancel-after-capture-v3"),
            "cancel-after-capture-v4"
        );
        assert_eq!(
            next_change_id("payments-missing-customer"),
            "payments-missing-customer-2"
        );
        assert_eq!(next_change_id("change-001"), "change-2");
        assert_eq!(next_change_id("7"), "7-2");
    }
    #[test]
    fn the_stepper_shows_where_a_change_stands() {
        let states = |v: V| {
            stages(&v)
                .into_iter()
                .map(|(_, state)| state)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            states(json!({"status":"needs_review","plan":{"status":"needs_review"}})),
            ["done", "current", "upcoming", "upcoming"]
        );
        assert_eq!(
            states(json!({"status":"needs_selection","plan":{"status":"needs_selection"}})),
            ["done", "attention", "upcoming", "upcoming"]
        );
        assert_eq!(
            states(json!({"status":"running","active":true,"session_id":"s"})),
            ["done", "done", "running", "upcoming"]
        );
        assert_eq!(
            states(json!({"status":"blocked","session_id":"s"})),
            ["done", "done", "attention", "upcoming"]
        );
        assert_eq!(
            states(json!({"status":"completed","session_id":"s"})),
            ["done", "done", "done", "done"]
        );
    }
    #[test]
    fn lists_read_as_text() {
        assert_eq!(joined(&json!(["a", "b"])), "a, b");
        assert_eq!(joined(&json!([])), "none");
    }
    #[test]
    fn a_plan_says_how_much_memory_every_agent_shares() {
        assert_eq!(
            shared_memory_summary(&json!({"context":{"sources":[]}})),
            "none"
        );
        let plan = json!({"context":{"sources":[
            {"kind":"memory","trust":"reviewed_memory"},
            {"kind":"memory","trust":"automatic_memory"},
            {"kind":"code","trust":"source_evidence"}]},
            "warnings":[{"reason":"memory_not_shared","count":3}]});
        assert_eq!(
            shared_memory_summary(&plan),
            "2 entries · 1 not reviewed by a person · 3 not shared"
        );
    }
    #[test]
    fn a_handoff_says_what_memory_the_outcome_became() {
        let title = "System change refunds: check receipts";
        assert_eq!(
            handoff_memory(
                &json!({"memory":{"status":"applied","review_mode":"automatic","memory_id":"MEM-1","title":title}})
            ),
            "\nShared as system memory: System change refunds: check receipts (MEM-1). Later changes give it to every agent."
        );
        assert_eq!(
            handoff_memory(
                &json!({"memory":{"status":"proposed","review_mode":"human","proposal_id":"p1","title":title}})
            ),
            "\nProposed as system memory: System change refunds: check receipts (proposal p1). The system folder turns automatic promotion off: approve it with memory promote-review, then promote-apply."
        );
        assert_eq!(
            handoff_memory(
                &json!({"memory":{"status":"proposed","review_mode":"automatic","note":"Promotion source is not eligible"}})
            ),
            "\nThe outcome is not system memory yet: Promotion source is not eligible"
        );
        assert_eq!(
            handoff_memory(&json!({"memory":{"error":"Record content contains a secret"}})),
            "\nThe outcome did not become system memory: Record content contains a secret"
        );
        assert_eq!(handoff_memory(&json!({})), "");
    }
    #[test]
    fn a_waiting_service_agent_reads_the_whole_change() {
        assert_eq!(
            service_agent_goal("payments", &json!("service"), &json!("read-only")),
            "Investigates payments; reads the system folder and every service of this change."
        );
        assert_eq!(
            service_agent_goal("payments", &json!("service"), &json!("edit")),
            "Works on payments; reads the system folder and every service of this change and edits only its own folder."
        );
        assert_eq!(
            service_agent_goal("payments", &json!("all"), &json!("edit")),
            "Works on payments; reads the system folder and every service of this change and edits any selected service."
        );
    }
    #[test]
    fn aggregate_graphs_pill_summarizes_ready_building_and_failed_states() {
        assert_eq!(
            aggregate_graphs_pill(&V::Null),
            ("idle", "Service graphs: loading…".to_owned())
        );
        let building = json!({
            "state": "building",
            "services": {
                "orders": { "state": "ready" },
                "payments": { "state": "building", "build": { "percent": 40 } },
                "catalog": { "state": "empty" }
            }
        });
        assert_eq!(
            aggregate_graphs_pill(&building),
            (
                "building",
                "Service graphs: 1/3 ready · 1 building".to_owned()
            )
        );
        let ready = json!({
            "state": "ready",
            "ready_count": 3,
            "total_count": 3,
            "building_count": 0
        });
        assert_eq!(
            aggregate_graphs_pill(&ready),
            ("ready", "Service graphs: 3/3 ready".to_owned())
        );
    }
    #[test]
    fn monotonic_service_graph_status_clamps_and_never_regresses() {
        let mut cache = BTreeMap::new();
        let s1 = monotonic_service_graph_status(
            "orders",
            &json!({ "state": "building", "build": { "percent": 0 } }),
            &mut cache,
        );
        assert_eq!(s1["build"]["percent"], 1);
        let s2 = monotonic_service_graph_status(
            "orders",
            &json!({ "state": "building", "build": { "percent": 55 } }),
            &mut cache,
        );
        assert_eq!(s2["build"]["percent"], 55);
        let s3 = monotonic_service_graph_status(
            "orders",
            &json!({ "state": "building", "build": { "percent": 22 } }),
            &mut cache,
        );
        assert_eq!(s3["build"]["percent"], 55);
        let done =
            monotonic_service_graph_status("orders", &json!({ "state": "ready" }), &mut cache);
        assert_eq!(done["state"], "ready");
        assert!(!cache.contains_key("orders"));
    }
    #[test]
    fn build_changes_overlay_tracks_plan_blast_radius_and_live_run_waves() {
        let plan_overlay = build_changes_overlay(
            &V::Null,
            true,
            &["orders".to_owned()],
            1,
            "orders.http, order.placed",
        );
        assert_eq!(plan_overlay["stage"], "plan");
        assert_eq!(plan_overlay["depth"], 1);
        assert_eq!(
            plan_overlay["changed_contracts"],
            json!(["orders.http", "order.placed"])
        );

        let run_doc = json!({
            "id": "run-42",
            "active": true,
            "session_id": "sess-1",
            "plan": {
                "depth": 32,
                "context": { "services": ["orders", "inventory"] }
            },
            "execution": {
                "steps": [
                    { "id": "service-orders", "service": "orders", "status": "completed", "attempt": 1 },
                    { "id": "service-inventory", "service": "inventory", "status": "running", "attempt": 1 }
                ]
            },
            "receipts": [],
            "evidence": {
                "cross_service_unopened": ["gateway"]
            }
        });
        let live_overlay = build_changes_overlay(&run_doc, false, &[], 32, "");
        assert_eq!(live_overlay["stage"], "run");
        assert_eq!(live_overlay["step_states"]["orders"], "completed");
        assert_eq!(live_overlay["step_states"]["inventory"], "running");
        assert_eq!(live_overlay["cross_service_unopened"], json!(["gateway"]));
    }
    #[test]
    fn receipt_changed_files_deduplicates_report_and_evidence_paths() {
        let receipt = json!({
            "phase": "service-orders",
            "report": { "changed_files": ["src/OrderService.php", "src/Dto.php"] },
            "evidence": {
                "changed_files": [
                    { "path": "src/OrderService.php", "status": "M" },
                    { "path": "tests/OrderTest.php", "status": "A" }
                ]
            }
        });
        let (svc, files) = receipt_changed_files(&receipt);
        assert_eq!(svc, "orders");
        assert_eq!(
            files,
            vec!["src/OrderService.php", "src/Dto.php", "tests/OrderTest.php"]
        );
    }
    #[test]
    fn integrated_service_graph_pill_and_scope_switcher_roundtrip() {
        let pill = crate::graph::render_service_graph_pill_html(
            "orders",
            &json!({ "state": "building", "build": { "percent": 42, "phase": "indexing" } }),
        );
        assert!(pill.contains("data-service=\"orders\""));
        assert!(pill.contains("value=\"42\""));

        let scope = crate::graph::GraphTargetScope::SystemService {
            project_id: "proj-1".to_owned(),
            config_path: "system.toml".to_owned(),
            service_id: "orders".to_owned(),
            services: vec![
                "orders".to_owned(),
                "payments".to_owned(),
                "__system__".to_owned(),
            ],
            run_id: Some("run-9".to_owned()),
            run_title: Some("change-001".to_owned()),
            launches: Vec::new(),
        };
        let opts = crate::graph_map::service_switcher_options_html(&scope);
        assert!(opts.contains("__topology__"));
        assert!(opts.contains("__system__"));
        assert!(opts.contains("orders"));
        assert!(opts.contains("payments"));
    }
    #[test]
    fn dispatch_evidence_keys_graph_focus_wrapper_and_agent_visibility_align_with_backend() {
        let receipt = json!({
            "phase": "service-orders",
            "report": { "changed_files": ["src/OrderService.php"] },
            "evidence": {
                "git_changed": ["src/OrderService.php", "routes/api.php"],
                "ledger_edited": [{ "path": "app/Dto/OrderDto.php" }],
                "opened": ["src/OrderService.php", "routes/api.php"],
                "shell_reads": ["config/orders.php"],
                "graph_asked": true
            }
        });
        let (svc, files) = receipt_changed_files(&receipt);
        assert_eq!(svc, "orders");
        assert_eq!(
            files,
            vec![
                "src/OrderService.php",
                "routes/api.php",
                "app/Dto/OrderDto.php"
            ]
        );
        assert_eq!(
            receipt_evidence_summary(&receipt["evidence"]),
            Some(("2 files · 1 shell reads · graph queried".to_owned(), false))
        );
        assert_eq!(
            receipt_evidence_summary(
                &json!({ "opened": [], "shell_reads": [], "graph_asked": false })
            ),
            Some(("0 files · graph not queried".to_owned(), true))
        );

        let wrapped_run = json!({
            "graph_focus": {
                "services": {
                    "orders": { "zone": ["src/OrderService.php"] },
                    "gateway": { "zone": ["src/OrdersClient.ts"] }
                },
                "aggregate": { "services_count": 2 }
            }
        });
        let focus_map = extract_service_graph_focus_map(&wrapped_run);
        assert_eq!(
            focus_map.get("orders"),
            Some(&json!({ "zone": ["src/OrderService.php"] }))
        );
        assert!(!focus_map.contains_key("aggregate"));

        let unrun_plan = json!({
            "status": "needs_review",
            "launches": [],
            "receipts": [],
            "evidence": { "dispatches": [], "files": [] }
        });
        assert!(!run_has_agent_activity(&unrun_plan));

        let cli_run = json!({
            "status": "completed",
            "launches": [],
            "receipts": [],
            "evidence": { "dispatches": [{ "phase": "service-orders" }] }
        });
        assert!(run_has_agent_activity(&cli_run));

        let session_run = json!({
            "status": "running",
            "launches": [{ "session_id": "sess-1" }],
            "receipts": [],
            "evidence": { "dispatches": [] }
        });
        assert!(run_has_agent_activity(&session_run));
    }

    #[test]
    fn the_files_several_steps_edited_are_listed_in_the_order_the_steps_ran() {
        let evidence = json!({"overlaps": [
            {"path": "orders/src/Order.php", "service": "orders", "file": "src/Order.php",
             "steps": ["service-orders", "verify"], "services": ["orders", "__system__"]},
            {"path": "packages/money/Money.php", "service": null, "file": null,
             "steps": ["service-orders", "service-billing"], "services": ["orders", "billing"]},
            {"path": "__system__/docs/a.md", "service": "__system__", "file": "docs/a.md",
             "steps": ["contracts", "verify"], "services": ["__system__", "__system__"]}
        ]});
        assert_eq!(
            overlap_rows(&evidence),
            [
                (
                    "orders/src/Order.php".to_owned(),
                    "service-orders → verify".to_owned(),
                    Some(("orders".to_owned(), "src/Order.php".to_owned()))
                ),
                (
                    "packages/money/Money.php".to_owned(),
                    "service-orders → service-billing".to_owned(),
                    None
                ),
                // The folder of the system has no code map of its own.
                (
                    "__system__/docs/a.md".to_owned(),
                    "contracts → verify".to_owned(),
                    None
                ),
            ]
        );
        assert!(overlap_rows(&json!({})).is_empty());
    }

    #[test]
    fn receipt_render_items_key_by_phase_attempt_and_preserve_accepted_and_overlaps() {
        let receipts = vec![
            json!({ "phase": "contracts", "attempt": 1, "ok": true }),
            json!({ "phase": "verify", "attempt": 1, "ok": false }),
            json!({ "phase": "verify", "attempt": 1, "ok": true }),
        ];
        let steps = vec![json!({
            "id": "verify",
            "attempt": 1,
            "accepted_unrun_checks": ["phpstan"]
        })];
        let overlaps = vec![(
            "orders/src/Order.php".to_owned(),
            "service-orders → verify".to_owned(),
            Some(("orders".to_owned(), "src/Order.php".to_owned())),
        )];
        let items = build_receipt_render_items(&receipts, &steps, &overlaps);
        let keys: Vec<String> = items.iter().map(|i| m::text(i, "key")).collect();
        assert_eq!(
            keys,
            vec![
                "receipt:contracts:1",
                "receipt:verify:1",
                "receipt:verify:1#1",
                "overlaps",
            ]
        );
        assert_eq!(items[0]["accepted"], false);
        assert_eq!(items[1]["accepted"], true);
        assert_eq!(items[3]["kind"], "overlaps");
    }

    #[test]
    fn editor_service_items_key_by_path_with_index_updates_and_collision_fallback() {
        let services = vec![
            json!({ "path": "services/orders", "passport": { "id": "orders" } }),
            json!({ "path": "", "passport": { "id": "draft" } }),
            json!({ "path": "services/orders", "passport": { "id": "orders-dup" } }),
        ];
        let items = build_editor_service_items(&services);
        assert_eq!(items[0]["key"], "path:services/orders");
        assert_eq!(items[0]["index"], 0);
        assert_eq!(items[1]["key"], "idx:1");
        assert_eq!(items[1]["index"], 1);
        assert_eq!(items[2]["key"], "path:services/orders#1");
        assert_eq!(items[2]["index"], 2);
    }

    #[test]
    fn editor_lock_state_offers_discard_only_when_dirty_and_not_scanning() {
        assert_eq!(editor_lock_state(false, false), ("", "", false));
        let (state, msg, show_discard) = editor_lock_state(false, true);
        assert_eq!(state, "dirty");
        assert!(msg.contains("unsaved changes"));
        assert!(show_discard);
        let (scan_state, scan_msg, scan_discard) = editor_lock_state(true, true);
        assert_eq!(scan_state, "scanning");
        assert!(scan_msg.contains("AI scan"));
        assert!(!scan_discard);
    }

    #[test]
    fn agent_launch_options_and_selection_humanize_labels_and_track_latest() {
        let launches = vec![
            json!({ "session_id": "s1", "status": "completed" }),
            json!({ "session_id": "s2", "status": "needs_review" }),
        ];
        let options = build_agent_launch_options(&launches);
        assert_eq!(
            options,
            vec![
                json!({ "id": "s1", "label": "Launch 1 · completed" }),
                json!({ "id": "s2", "label": "Launch 2 · needs review" }),
            ]
        );
        // Preserves user's chosen earlier launch when launch count didn't change
        assert_eq!(
            resolve_agent_launch_selection(&launches, 2, "s1"),
            ("s1".to_owned(), false)
        );
        // Advances to newest launch when a new launch was added
        assert_eq!(
            resolve_agent_launch_selection(&launches, 1, "s1"),
            ("s2".to_owned(), true)
        );
        // Falls back to latest when preferred no longer exists
        assert_eq!(
            resolve_agent_launch_selection(&launches, 2, "missing"),
            ("s2".to_owned(), true)
        );
    }

    #[test]
    fn system_relationship_and_flow_cards_preserve_badges_and_stage_state() {
        let edge = json!({
            "provider": "orders",
            "consumer": "billing",
            "contract": "OrderPaid",
            "version": "2",
            "provenance": "composer_json"
        });
        let rel = super::build_relationship_view(&edge);
        assert_eq!(rel.provider, "orders");
        assert_eq!(rel.consumer, "billing");
        assert_eq!(rel.contract, "OrderPaid");
        assert_eq!(rel.version, "v2");
        assert_eq!(rel.provenance, "composer json");

        let step = json!({
            "id": "verify",
            "service": "billing",
            "status": "running",
            "mode": "write_patch",
            "attempt": 2,
            "goal": "Validate invoice webhook",
            "depends_on": ["implement"],
            "task_reference": "TASK-042"
        });
        let active_step = super::build_system_step_view(&step, &json!([]), true, 0);
        assert_eq!(active_step.stage_class, "system-stage running");
        assert_eq!(active_step.row_state, "running");
        assert_eq!(active_step.status_label, "running");
        assert_eq!(active_step.service, "billing");
        assert_eq!(active_step.mode, "write patch");
        assert_eq!(active_step.attempt.as_deref(), Some("attempt 2"));
        assert_eq!(
            active_step.depends_on.as_deref(),
            Some("Depends on: implement")
        );
        assert_eq!(active_step.task_reference.as_deref(), Some("TASK-042"));

        let reported = super::build_system_step_view(
            &step,
            &json!([{"phase": "verify", "attempt": 2, "ok": true}]),
            false,
            0,
        );
        assert_eq!(reported.stage_class, "system-stage interrupted");
        assert_eq!(
            reported.status_label,
            "reported complete; resume to record it"
        );
    }
}
