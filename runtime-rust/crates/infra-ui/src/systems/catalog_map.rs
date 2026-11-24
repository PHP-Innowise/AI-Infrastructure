use super::*;
use crate::{api, dom, model as m, session, system_map};
use serde_json::{json, Value as V};
use std::collections::{BTreeMap, VecDeque};
use wasm_bindgen_futures::spawn_local;

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
    drill_down_service_target(surface, service_id, focus_path, None);
}

/// Switches `Services` or `Changes` into a member service's embedded code map and focuses
/// an optional file path + symbol target (`symbol_fqn`, short symbol, or `symbol_id`).
pub fn drill_down_service_target(
    surface: &str,
    service_id: &str,
    focus_path: Option<&str>,
    focus_symbol: Option<&str>,
) {
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
            crate::graph_map::focus_system_target(
                crate::graph_map::SystemEmbeddedKind::Change,
                path,
                focus_symbol,
            );
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
            crate::graph_map::focus_system_target(
                crate::graph_map::SystemEmbeddedKind::Member,
                path,
                focus_symbol,
            );
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

pub(super) fn apply_monotonic_statuses(mut agg: V) -> V {
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

pub(super) fn catalog_service_ids() -> Vec<String> {
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

pub(super) fn build_system_graph_scope(
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

pub(super) fn sync_system_explanation_context_from_dom() {
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

pub(super) fn sync_system_graph_header(agg: &V) {
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

pub(crate) async fn poll_system_graph_status(check: bool) {
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

pub(super) async fn post_system_graph_action(action: &str, service: Option<&str>) {
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

pub(super) fn update_service_card_pills(agg: &V) {
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

    // Prefer wave-0 root seeds (`reason` selected/contract_provider or path len 1)
    // from `plan.impact`, then explicit `plan.selected` / checked UI `plan_seeds`;
    // only fall back to `plan.context.services` when no root seeds were specified.
    let mut seeds: Vec<String> = m::array(&run_doc["plan"]["impact"])
        .into_iter()
        .filter(|entry| {
            matches!(
                entry["reason"].as_str(),
                Some("selected" | "contract_provider")
            ) || m::array(&entry["path"]).len() == 1
        })
        .map(|entry| m::text(&entry, "service"))
        .filter(|s| !s.is_empty())
        .collect();
    if seeds.is_empty() {
        seeds = m::array(&run_doc["plan"]["selected"])
            .into_iter()
            .filter_map(|v| {
                v.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            })
            .collect();
    }
    if seeds.is_empty() && !plan_seeds.is_empty() {
        seeds = plan_seeds.to_vec();
    }
    if seeds.is_empty() {
        seeds = m::array(&run_doc["plan"]["context"]["services"])
            .into_iter()
            .filter_map(|v| {
                v.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            })
            .collect();
    }
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
        "seed_services": seeds,
        "depth": run_doc["plan"]["depth"].as_u64().unwrap_or(depth),
        "changed_contracts": parsed_contracts,
        "step_states": V::Object(step_states),
        "evidence": run_doc["evidence"],
        "cross_service_unopened": unopened,
    })
}

pub(super) fn extract_service_graph_focus_map(run_doc: &V) -> serde_json::Map<String, V> {
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

pub(super) fn current_service_live_focus(service_id: &str) -> Option<V> {
    let cache = m::data("system-live-focus");
    if cache[service_id].is_object() {
        return Some(cache[service_id].clone());
    }
    let run_doc = m::data("system-run");
    extract_service_graph_focus_map(&run_doc)
        .get(service_id)
        .cloned()
}

pub(super) fn ingest_run_graph_focus(run_doc: &V) {
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

pub(super) fn sync_change_service_tabs() {
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

pub(super) fn sync_changes_map_workspace() {
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

/// Opening Services shows the project's system when its file exists: the
/// form already names it, so a Load system click on every visit added
/// nothing. A project without the file keeps the empty state, quietly.
/// Returns `true` when `/api/systems/catalog` reported a cleanly missing manifest on an optional probe.
pub(super) fn catalog_response_is_missing(value: &V) -> bool {
    m::yes(&value["missing"]) || value["catalog"].is_null()
}

pub async fn catalog_autoload() {
    if !m::data("system-catalog").is_null()
        || m::yes(&m::data("editor-dirty"))
        || dom::value("system-project").is_empty()
    {
        return;
    }
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let generation = m::read(|s| s.epochs.get("editor-scope").copied().unwrap_or(0));
    if m::data("system-catalog-probed") == json!([project, config, generation]) {
        return;
    }
    catalog_fetch(true).await;
}
pub(super) async fn catalog_load() {
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
    m::take("system-catalog-probed");
    catalog_fetch(false).await;
}
pub(super) async fn catalog_fetch(quiet_when_missing: bool) {
    if !sync_editor_target() {
        return;
    }
    let project = dom::value("system-project");
    let config = dom::value("system-config");
    let generation = m::read(|s| s.epochs.get("editor-scope").copied().unwrap_or(0));
    let epoch = m::next("system-catalog-load");
    match api::post(
        "/api/systems/catalog",
        json!({"project_id":project,"config_path":config,"optional":quiet_when_missing}),
    )
    .await
    {
        Ok(value)
            if editor_current(&project, &config, generation)
                && m::current("system-catalog-load", epoch) =>
        {
            if catalog_response_is_missing(&value) {
                m::take("system-catalog");
                m::set(
                    "system-catalog-probed",
                    json!([project, config, generation]),
                );
                dom::hidden("system-catalog", true);
                dom::error("system-error", "");
                layout();
                return;
            }
            m::take("system-catalog-probed");
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
pub(super) fn edge_path(a: (usize, usize), b: (usize, usize)) -> String {
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

pub(super) fn render_catalog(v: &V) {
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
