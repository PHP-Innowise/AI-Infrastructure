use super::*;

pub fn pill_presentation(status: &V) -> (&'static str, &'static str, String, String) {
    let connected = m::yes(&status["connected"]);
    let state = m::text(status, "state").to_ascii_lowercase();
    let mode = m::text(status, "mode");
    let err = m::text(status, "error");
    let atoms = status["stats"]["atoms"].as_u64().unwrap_or(0);
    let branches = status["stats"]["branches"].as_u64().unwrap_or(0);
    let mode_label = if mode.is_empty() {
        "live".to_owned()
    } else {
        mode
    };
    if connected {
        let label = format!("Omnigraph: {mode_label} · {atoms} atoms");
        let title = format!(
            "Omnigraph connected ({mode_label}) · {atoms} atoms across {branches} branches"
        );
        return ("ready", "●", label, title);
    }
    if matches!(
        state.as_str(),
        "installing" | "starting" | "applying_schema"
    ) {
        return (
            "building",
            "◐",
            "Omnigraph: starting…".into(),
            "Omnigraph runtime is starting or installing".into(),
        );
    }
    if !err.is_empty() || matches!(state.as_str(), "error" | "failed") {
        return (
            "failed",
            "▲",
            "Omnigraph: error".into(),
            format!("Omnigraph error: {err}"),
        );
    }
    if m::yes(&status["binary_installed"]) || m::yes(&status["docker_available"]) {
        return (
            "stale",
            "◇",
            "Omnigraph: stopped".into(),
            "Omnigraph runtime is installed but stopped — click to open Context Graph".into(),
        );
    }
    (
        "off",
        "○",
        "Omnigraph: off".into(),
        "Omnigraph context graph is not connected — click to set up".into(),
    )
}

pub(super) fn on_sse_omnigraph_status(payload: &V) {
    let ev_project = m::text(payload, "project_id");
    let cur_project = current_project_id();
    if !ev_project.is_empty() && !cur_project.is_empty() && ev_project != cur_project {
        return;
    }
    if payload["connected"].is_boolean() {
        hydrate_status(payload);
    }
    let op = m::text(payload, "operation");
    let in_view = m::read(|s| s.view == "omnigraph");
    let graph_changed = matches!(
        op.as_str(),
        "omnigraph.publish"
            | "omnigraph.branch_act"
            | "omnigraph.import"
            | "omnigraph.sync_knowledge"
            | "omnigraph.promote"
            | "omnigraph.setup"
    );
    spawn_local(async move {
        refresh_pill().await;
        if in_view && graph_changed && m::yes(&m::data("omnigraph-status")["connected"]) {
            refresh_graph_preserving_pages().await;
            poll_changes(false).await;
        }
    });
}

pub(super) fn on_sse_omnigraph_resync() {
    let in_view = m::read(|s| s.view == "omnigraph");
    spawn_local(async move {
        refresh_pill().await;
        if in_view && m::yes(&m::data("omnigraph-status")["connected"]) {
            refresh_graph_preserving_pages().await;
            poll_changes(false).await;
        }
    });
}

pub fn initialize() {
    let _ = ui_state();
    render_tabs();
    sync_pulse_toggle();
    attach_transfer_dropzone_listeners();
    ensure_resize_watch();
    OMNI_SSE_BOUND.with(|bound| {
        if !bound.get() {
            bound.set(true);
            api::on_sse("omnigraph_status", on_sse_omnigraph_status);
            api::on_sse("resync", |_| on_sse_omnigraph_resync());
        }
    });
}

pub fn hydrate_status(status: &V) {
    if !status.is_object() {
        return;
    }
    m::set("omnigraph-status", status.clone());
    apply_status_to_chrome(status);
}

pub fn degraded_status_from_error(previous: &V, error_message: &str) -> V {
    let mut next = if previous.is_object() {
        previous.clone()
    } else {
        json!({})
    };
    next["connected"] = json!(false);
    next["state"] = json!("error");
    next["error"] = json!(error_message);
    next
}

pub async fn refresh_pill() {
    let project = current_project_id();
    let path = format!(
        "/api/omnigraph/status{}",
        dom::query(&[("project_id", project)])
    );
    match api::get(&path).await {
        Ok(status) => hydrate_status(&status),
        Err(e) if !e.aborted => {
            let degraded = degraded_status_from_error(&m::data("omnigraph-status"), &e.message);
            hydrate_status(&degraded);
        }
        Err(_) => {}
    }
}

pub fn studio_visibility_plan(status: &V, has_loaded_graph: bool) -> (bool, bool, Option<String>) {
    let connected = m::yes(&status["connected"]);
    if connected {
        return (false, true, None);
    }
    let state = m::text(status, "state").to_ascii_lowercase();
    if has_loaded_graph && state != "unconfigured" {
        let err = m::text(status, "error");
        let banner = if m::yes(&status["starting"])
            || matches!(
                state.as_str(),
                "installing" | "starting" | "applying_schema"
            ) {
            "Omnigraph runtime is reconnecting — showing last loaded graph in read-only mode."
                .to_owned()
        } else if !err.is_empty() {
            format!(
                "Omnigraph offline ({err}) — showing last loaded graph. Open Connection… to reconnect."
            )
        } else {
            "Omnigraph is currently offline — showing last loaded graph. Open Connection… to start or reconnect.".to_owned()
        };
        return (false, true, Some(banner));
    }
    (true, false, None)
}

pub fn local_install_copy(status: &V) -> (String, String, bool) {
    let installed = m::yes(&status["binary_installed"]);
    let bin_version = m::text(status, "binary_version");
    let target_version = {
        let tv = m::text(status, "target_version");
        if !tv.is_empty() {
            tv
        } else {
            let sv = m::text(status, "version");
            if !sv.is_empty() {
                sv
            } else {
                bin_version.clone()
            }
        }
    };
    let auto_supported = status["auto_download_supported"]
        .as_bool()
        .or_else(|| status["platform"]["auto_download_supported"].as_bool())
        .unwrap_or(true);
    let os = {
        let p = m::text(&status["platform"], "os");
        if p.is_empty() {
            m::text(status, "os")
        } else {
            p
        }
    };
    let arch = {
        let a = m::text(&status["platform"], "arch");
        if a.is_empty() {
            m::text(status, "arch")
        } else {
            a
        }
    };
    let platform_label = match (os.is_empty(), arch.is_empty()) {
        (false, false) => format!("{os}/{arch}"),
        (false, true) => os,
        (true, false) => arch,
        (true, true) => "this platform".to_owned(),
    };

    let button_label = if !auto_supported && !installed {
        if target_version.is_empty() {
            "Local Auto-Download Unavailable".to_owned()
        } else {
            format!("Local {target_version} Unavailable")
        }
    } else if installed {
        if target_version.is_empty() {
            "Reinstall Local Omnigraph".to_owned()
        } else {
            format!("Reinstall Local Omnigraph {target_version}")
        }
    } else if target_version.is_empty() {
        "Install & Start Local Omnigraph".to_owned()
    } else {
        format!("Install & Start Local Omnigraph {target_version}")
    };

    let detail_text = if !auto_supported && !installed {
        format!(
            "Local sidecar auto-download is unavailable on {platform_label}; use Start via Docker or Connect External Server."
        )
    } else if !bin_version.is_empty() {
        format!(
            "Installed version {bin_version} ({platform_label}). Managed locally with automatic schema apply and bearer auth."
        )
    } else if installed {
        format!(
            "Local sidecar installed ({platform_label}). Managed locally with automatic schema apply and bearer auth."
        )
    } else if !target_version.is_empty() {
        format!(
            "Downloads or launches local Omnigraph {target_version} for {platform_label} with automatic schema setup and bearer auth."
        )
    } else {
        format!(
            "Downloads or launches the local sidecar for {platform_label} with automatic schema setup and bearer auth."
        )
    };
    (button_label, detail_text, auto_supported)
}

pub(super) fn graph_has_loaded_content(graph: &V) -> bool {
    graph.is_object()
        && (!m::array(&graph["atoms"]).is_empty()
            || !m::array(&graph["branches"]).is_empty()
            || !m::array(&graph["sessions"]).is_empty()
            || !m::array(&graph["files"]).is_empty())
}

pub(super) fn apply_status_to_chrome(status: &V) {
    let (state, shape, label, title) = pill_presentation(status);
    dom::attr("sidebar-omnigraph-pill", "data-state", state);
    dom::text("sidebar-omnigraph-shape", shape);
    dom::text("sidebar-omnigraph-label", &label);
    dom::attr("sidebar-omnigraph-pill", "title", &title);
    dom::attr("sidebar-omnigraph-pill", "aria-label", &title);

    dom::attr("omnigraph-state-badge", "data-state", state);
    dom::text("omnigraph-state-icon", shape);
    let connected = m::yes(&status["connected"]);
    let state_text = if connected {
        "Connected"
    } else if state == "building" {
        "Starting…"
    } else if state == "failed" {
        "Error"
    } else if state == "stale" {
        "Stopped"
    } else {
        "Disconnected"
    };
    dom::text("omnigraph-state-text", state_text);

    let mode = m::text(status, "mode");
    dom::text(
        "omnigraph-mode-badge",
        &format!("Mode: {}", if mode.is_empty() { "local" } else { &mode }),
    );
    let graph_id = m::text(status, "graph_id");
    dom::text("omnigraph-graph-id", &graph_id);
    dom::hidden("omnigraph-graph-id", graph_id.is_empty());
    let endpoint = m::text(status, "endpoint");
    dom::text("omnigraph-endpoint-badge", &endpoint);
    dom::hidden("omnigraph-endpoint-badge", endpoint.is_empty());

    let stats = &status["stats"];
    dom::text(
        "omnigraph-stat-branches",
        &stats["branches"].as_u64().unwrap_or(0).to_string(),
    );
    dom::text(
        "omnigraph-stat-atoms",
        &stats["atoms"].as_u64().unwrap_or(0).to_string(),
    );
    dom::text(
        "omnigraph-stat-sessions",
        &stats["sessions"].as_u64().unwrap_or(0).to_string(),
    );
    let unmerged = stats["unmerged_branches"].as_u64().unwrap_or(0);
    dom::text("omnigraph-stat-unmerged", &unmerged.to_string());
    dom::attr(
        "omnigraph-stat-unmerged-pill",
        "data-warn",
        if unmerged > 0 { "true" } else { "false" },
    );

    let mcp_url = m::text(status, "mcp_url");
    dom::hidden("omnigraph-mcp-copy", mcp_url.is_empty());
    dom::hidden("omnigraph-apply-schema", !connected);
    dom::hidden("omnigraph-stop", !connected);

    let has_loaded_graph = graph_has_loaded_content(&m::data("omnigraph-graph"));
    let (show_onboarding, show_studio, degraded_banner) =
        studio_visibility_plan(status, has_loaded_graph);
    dom::hidden("omnigraph-onboarding", !show_onboarding);
    dom::hidden("omnigraph-studio", !show_studio);
    if let Some(banner) = degraded_banner {
        dom::text("omnigraph-status-message", &banner);
        dom::attr("omnigraph-status-message", "data-degraded", "true");
        dom::hidden("omnigraph-status-message", false);
    } else if connected
        && dom::get("omnigraph-status-message")
            .and_then(|el| el.get_attribute("data-degraded"))
            .as_deref()
            == Some("true")
    {
        dom::attr("omnigraph-status-message", "data-degraded", "false");
        dom::text("omnigraph-status-message", "");
        dom::hidden("omnigraph-status-message", true);
    }

    let installed = m::yes(&status["binary_installed"]);
    let (btn_label, local_detail, auto_supported) = local_install_copy(status);
    dom::hidden("omnigraph-start-local", !installed || connected);
    dom::text("omnigraph-install-start", &btn_label);
    dom::disabled("omnigraph-install-start", !auto_supported && !installed);
    dom::disabled("omni-dlg-install", !auto_supported && !installed);
    dom::text("omnigraph-local-detail", &local_detail);
    let docker_ok = m::yes(&status["docker_available"]);
    dom::disabled("omnigraph-start-docker", !docker_ok && !connected);
    dom::disabled("omni-dlg-start-docker", !docker_ok);
    dom::disabled("omni-sync-knowledge", !connected);
    dom::disabled("omnigraph-export-open", !connected);
    dom::disabled("omnigraph-import-open", !connected);

    let mut err = m::text(status, "error");
    let stderr_tail = m::array(&status["stderr_tail"])
        .into_iter()
        .filter_map(|v| {
            v.as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    if err.is_empty() && !stderr_tail.is_empty() && state == "error" {
        err = stderr_tail.last().cloned().unwrap_or_default();
    }
    dom::error("omnigraph-error", &err);
    dom::text(
        "omnigraph-session-note",
        if connected {
            "Omnigraph is live. Sessions can read and publish shared learnings across branches."
        } else {
            "Omnigraph is currently offline. Configure or start it in Context Graph to enable live sharing."
        },
    );
    let selected = m::read(|s| s.selected.clone());
    if selected.is_object() {
        render_session_bar(&selected);
    }
}

pub async fn load() {
    let project = current_project_id();
    m::set("omnigraph-pending", json!(true));
    let status_path = format!(
        "/api/omnigraph/status{}",
        dom::query(&[("project_id", project.clone())])
    );
    match api::get(&status_path).await {
        Ok(status) => {
            m::set("omnigraph-status", status.clone());
            apply_status_to_chrome(&status);
            if m::yes(&status["connected"]) {
                sync_controls_from_state();
                refresh_graph().await;
                poll_changes(true).await;
                schedule_pulse_poll();
            } else {
                dom::cancel_timer("omnigraph-pulse");
            }
        }
        Err(e) if !e.aborted => {
            dom::error("omnigraph-error", &e.message);
        }
        _ => {}
    }
    m::set("omnigraph-pending", json!(false));
    crate::session::sync_working_indicator();
}

pub fn leave() {
    dom::cancel_timer("omnigraph-pulse");
}

pub fn visibility() {
    if dom::document().hidden() {
        dom::cancel_timer("omnigraph-pulse");
    } else if m::read(|s| s.view == "omnigraph")
        && m::yes(&m::data("omnigraph-status")["connected"])
    {
        schedule_pulse_poll();
    }
}

pub fn on_project_changed() {
    dom::cancel_timer("omnigraph-pulse");
    OMNI_DENSE_CACHE.with(|c| *c.borrow_mut() = None);
    OMNI_HOVER_HIT.with(|h| *h.borrow_mut() = None);
    OMNI_NAV_HISTORY.with(|h| h.borrow_mut().clear());
    graph_viewport::canvas2d::clear_scene("omnigraph-canvas");
    m::set("omnigraph-graph", V::Null);
    m::set("omnigraph-diff", V::Null);
    m::set("omnigraph-search-results", V::Null);
    m::set("omnigraph-commit-detail", V::Null);
    m::set("omnigraph-head-commit", V::Null);
    m::set("omnigraph-workspace-changes", V::Null);
    m::set("omnigraph-positions", V::Null);
    m::set("omnigraph-raw-positions", V::Null);
    m::set("omnigraph-structure-digest", V::Null);
    m::set("omnigraph-nav-rects", json!([]));
    m::set("omnigraph-nav-edges", json!([]));
    m::set("omnigraph-anchor-filter-key", V::Null);
    m::set("omnigraph-page-count", json!(1));
    m::set("omnigraph-camera", V::Null);
    m::set("omnigraph-pulse-events", json!([]));
    reset_pulse_cursor();
    set_ui_state(|s| {
        s["branch"] = json!("main");
        s["snapshot"] = V::Null;
        s["kind"] = json!("");
        s["query"] = json!("");
        s["show_files"] = json!(true);
        s["show_sessions"] = json!(true);
        s["selected_id"] = V::Null;
        s["focused_id"] = V::Null;
        s["selected_commit_id"] = V::Null;
        s["highlighted_ids"] = json!([]);
        s["path_nodes"] = json!([]);
        s["path_edges"] = json!([]);
        s["isolate_active"] = json!(false);
        s["isolate_seed"] = V::Null;
    });
    if let Some(el) = dom::get("omnigraph-canvas") {
        el.set_inner_html("");
    }
    if let Some(radar) = dom::get("omnigraph-canvas-radar") {
        radar.set_inner_html("");
        dom::hide(&radar, true);
    }
    if let Some(dag) = dom::get("omnigraph-dag-container") {
        dag.set_inner_html("");
    }
    sync_controls_from_state();
    render_inspector();
    if m::read(|s| s.view == "omnigraph") {
        spawn_local(async {
            load().await;
        });
    } else {
        spawn_local(async {
            refresh_pill().await;
        });
    }
}

pub(super) async fn run_setup_action(
    action: &str,
    extra: Option<V>,
    status_target: &str,
    error_target: &str,
) {
    dom::error(error_target, "");
    dom::error("omnigraph-error", "");
    let mut body = json!({ "action": action });
    if let Some(obj) = extra.and_then(|v| v.as_object().cloned()) {
        for (k, v) in obj {
            if !v.is_null() && v.as_str() != Some("") {
                body[k] = v;
            }
        }
    }
    dom::text(status_target, &format!("Running Omnigraph {action}…"));
    dom::hidden(status_target, false);
    m::set("omnigraph-pending", json!(true));
    crate::session::sync_working_indicator();
    let res = api::post("/api/omnigraph/setup", body).await;
    match res {
        Ok(status) => {
            m::set("omnigraph-status", status.clone());
            apply_status_to_chrome(&status);
            if action == "install"
                && !m::yes(&status["connected"])
                && m::yes(&status["binary_installed"])
            {
                let start_res =
                    api::post("/api/omnigraph/setup", json!({ "action": "start_local" })).await;
                if let Ok(started) = start_res {
                    m::set("omnigraph-status", started.clone());
                    apply_status_to_chrome(&started);
                }
            }
            if m::yes(&m::data("omnigraph-status")["connected"]) {
                dom::text(status_target, "Omnigraph is connected and ready.");
                dom::dialog("omni-settings-dialog", false);
                load().await;
            } else {
                dom::text(status_target, &format!("Completed {action}."));
            }
        }
        Err(e) => {
            dom::hidden(status_target, true);
            dom::error(error_target, &e.message);
        }
    }
    m::set("omnigraph-pending", json!(false));
    crate::session::sync_working_indicator();
}
