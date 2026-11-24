//! The code graph chip and panel: one switch per project, percent and time
//! left while Graphify builds, what the published graph covers, and a second
//! switch for telling agents about it.
use crate::{api, dom, model as m};
use serde_json::{json, Value as V};
use wasm_bindgen_futures::spawn_local;

/// Identifies whether a code-graph request targets a single registered project
/// or a specific member service (`__system__`, standalone repo, or monorepo
/// subfolder) of a multi-service System orchestration workspace.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphTargetScope {
    SingleProject {
        project_id: String,
    },
    SystemService {
        project_id: String,
        config_path: String,
        service_id: String,
        services: Vec<String>,
        run_id: Option<String>,
        run_title: Option<String>,
        launches: Vec<V>,
    },
}

#[cfg(target_arch = "wasm32")]
fn encode_part(value: &str) -> String {
    dom::enc(value)
}

#[cfg(not(target_arch = "wasm32"))]
fn encode_part(value: &str) -> String {
    let mut out = String::new();
    for b in value.as_bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(*b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(target_arch = "wasm32")]
fn encode_query(params: &[(&str, String)]) -> String {
    dom::query(params)
}

#[cfg(not(target_arch = "wasm32"))]
fn encode_query(params: &[(&str, String)]) -> String {
    if params.is_empty() {
        return String::new();
    }
    let parts = params
        .iter()
        .map(|(k, v)| format!("{}={}", encode_part(k), encode_part(v)))
        .collect::<Vec<_>>();
    format!("?{}", parts.join("&"))
}

fn escape_html(text: &str) -> String {
    crate::dom::escape_html(text)
}

impl GraphTargetScope {
    pub fn single(project_id: impl Into<String>) -> Self {
        Self::SingleProject {
            project_id: project_id.into(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::SingleProject { project_id } => project_id.trim().is_empty(),
            Self::SystemService {
                project_id,
                config_path,
                service_id,
                ..
            } => {
                project_id.trim().is_empty()
                    || config_path.trim().is_empty()
                    || service_id.trim().is_empty()
            }
        }
    }

    pub fn is_system(&self) -> bool {
        matches!(self, Self::SystemService { .. })
    }

    pub fn project_id(&self) -> &str {
        match self {
            Self::SingleProject { project_id } | Self::SystemService { project_id, .. } => {
                project_id
            }
        }
    }

    pub fn service_id(&self) -> Option<&str> {
        match self {
            Self::SingleProject { .. } => None,
            Self::SystemService { service_id, .. } => Some(service_id.as_str()),
        }
    }

    pub fn scope_key(&self) -> String {
        match self {
            Self::SingleProject { project_id } => format!("project:{project_id}"),
            Self::SystemService {
                project_id,
                config_path,
                service_id,
                ..
            } => format!("system:{project_id}\n{config_path}\n{service_id}"),
        }
    }

    pub fn with_service(&self, next_service: &str) -> Self {
        match self {
            Self::SingleProject { project_id } => Self::SingleProject {
                project_id: project_id.clone(),
            },
            Self::SystemService {
                project_id,
                config_path,
                services,
                run_id,
                run_title,
                launches,
                ..
            } => {
                let mut all_services = services.clone();
                if !next_service.is_empty()
                    && next_service != "__system__"
                    && !all_services.iter().any(|svc| svc == next_service)
                {
                    all_services.push(next_service.to_owned());
                }
                Self::SystemService {
                    project_id: project_id.clone(),
                    config_path: config_path.clone(),
                    service_id: next_service.to_owned(),
                    services: all_services,
                    run_id: run_id.clone(),
                    run_title: run_title.clone(),
                    launches: launches.clone(),
                }
            }
        }
    }

    pub fn to_json(&self) -> V {
        match self {
            Self::SingleProject { project_id } => json!({
                "kind": "single",
                "project_id": project_id,
            }),
            Self::SystemService {
                project_id,
                config_path,
                service_id,
                services,
                run_id,
                run_title,
                launches,
            } => json!({
                "kind": "system_service",
                "project_id": project_id,
                "config_path": config_path,
                "service_id": service_id,
                "services": services,
                "run_id": run_id,
                "run_title": run_title,
                "launches": launches,
            }),
        }
    }

    pub fn from_json(raw: &V) -> Option<Self> {
        match raw["kind"].as_str()? {
            "single" => Some(Self::SingleProject {
                project_id: m::text(raw, "project_id"),
            }),
            "system_service" => Some(Self::SystemService {
                project_id: m::text(raw, "project_id"),
                config_path: m::text(raw, "config_path"),
                service_id: m::text(raw, "service_id"),
                services: m::array(&raw["services"])
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                run_id: raw["run_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                run_title: raw["run_title"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                launches: m::array(&raw["launches"]),
            }),
            _ => None,
        }
    }

    pub fn status_url(&self, check: bool) -> String {
        match self {
            Self::SingleProject { project_id } => format!(
                "/api/projects/{}/graph{}",
                encode_part(project_id),
                if check { "?check=1" } else { "" }
            ),
            Self::SystemService {
                project_id,
                config_path,
                service_id,
                ..
            } => {
                let mut query = vec![
                    ("project_id", project_id.clone()),
                    ("config_path", config_path.clone()),
                    ("service", service_id.clone()),
                ];
                if check {
                    query.push(("check", "1".to_owned()));
                }
                format!("/api/systems/service-graph{}", encode_query(&query))
            }
        }
    }

    pub fn act_request(&self, action: &str, target: Option<&str>) -> (String, V) {
        match self {
            Self::SingleProject { project_id } => {
                let mut body = json!({ "action": action });
                if let Some(target) = target {
                    body["target"] = json!(target);
                }
                (
                    format!("/api/projects/{}/graph", encode_part(project_id)),
                    body,
                )
            }
            Self::SystemService {
                project_id,
                config_path,
                service_id,
                ..
            } => {
                let mut body = json!({
                    "project_id": project_id,
                    "config_path": config_path,
                    "service": service_id,
                    "action": action,
                });
                if let Some(target) = target {
                    body["target"] = json!(target);
                }
                ("/api/systems/service-graph".to_owned(), body)
            }
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct GraphView {
    pub chip: String,
    pub on: bool,
    pub failed: bool,
    pub note: String,
    pub building: bool,
    /// `None` while building means the build has nothing to estimate from yet.
    pub percent: Option<u64>,
    pub amount: String,
    pub left: String,
    pub detail: String,
    pub rebuild: bool,
    pub delete: bool,
    /// Launches are told about the graph: asked for apart from the graph.
    pub agents: bool,
    pub offer: String,
}
fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
fn span(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds} s")
    } else if seconds.is_multiple_of(60) {
        format!("{} min", seconds / 60)
    } else {
        format!("{} min {} s", seconds / 60, seconds % 60)
    }
}
/// A first build guesses its silent part, so its time left moves in coarse steps.
fn about(seconds: u64, rough: bool) -> u64 {
    if !rough {
        return seconds;
    }
    let step = match seconds {
        0..20 => 5,
        20..90 => 15,
        _ => 30,
    };
    ((seconds + step / 2) / step * step).max(step)
}
fn megabytes(bytes: u64) -> String {
    let megabytes = bytes as f64 / 1_048_576.0;
    if megabytes < 10.0 {
        format!("{megabytes:.1} MB")
    } else {
        format!("{megabytes:.0} MB")
    }
}
fn detail(graph: &V, tool: &V) -> String {
    if !graph.is_object() {
        return String::new();
    }
    let mut parts = vec![];
    if let Some(commit) = graph["commit"].as_str() {
        parts.push(format!(
            "Commit {}",
            commit.chars().take(8).collect::<String>()
        ));
    }
    if let Some(nodes) = graph["nodes"].as_u64() {
        parts.push(format!("{} nodes", count(nodes)));
    }
    if let Some(edges) = graph["edges"].as_u64() {
        parts.push(format!("{} edges", count(edges)));
    }
    if let Some(bytes) = graph["disk"].as_u64().or(graph["bytes"].as_u64()) {
        parts.push(format!("{} on disk", megabytes(bytes)));
    }
    if let Some(seconds) = m::number(&graph["seconds"]) {
        parts.push(format!("built in {}", span(seconds.round() as u64)));
    }
    if let Some(version) = tool["version"].as_str() {
        parts.push(format!("Graphify {version}"));
    }
    parts.join(" · ")
}
/// What the chip and the panel show for one status of the project's graph.
pub fn graph_view(status: &V) -> GraphView {
    let state = status["state"].as_str().unwrap_or("");
    let build = &status["build"];
    let published = status["graph"].is_object();
    let mut view = GraphView {
        on: status["enabled"] == true,
        detail: detail(&status["graph"], &status["tool"]),
        delete: published || build.is_object(),
        agents: status["agents"] == true,
        ..GraphView::default()
    };
    view.offer = match (view.on, view.agents) {
        (true, true) => "Each launch adds a few lines that say how to query the graph, and agents may run Graphify's query, explain and path commands on it without asking. Nothing of the graph itself is put into the prompt.",
        (true, false) => "Agents are not told about the graph: it is built for the map. Telling them adds lines to every launch.",
        (false, _) if status.is_object() => "Agents can be told about the graph once it is on.",
        _ => "",
    }
    .into();
    match state {
        "building" => {
            view.building = true;
            view.percent = build["percent"].as_u64();
            let left = build["remaining_seconds"]
                .as_u64()
                .map(|left| about(left, m::yes(&build["rough"])));
            let overdue = m::yes(&build["overdue"]);
            view.amount = view
                .percent
                .map(|percent| format!("{percent}%"))
                .unwrap_or_default();
            view.left = match (left, overdue) {
                // A first build has no last build to be longer than.
                (_, true) if m::yes(&build["rough"]) => "longer than expected".into(),
                (_, true) => "longer than the last build".into(),
                (Some(left), false) => format!("about {} left", span(left)),
                (None, false) => String::new(),
            };
            view.chip = match (view.percent, left, overdue) {
                (Some(percent), Some(left), false) => format!("{percent}% · {}", span(left)),
                (Some(percent), _, _) => format!("{percent}%"),
                _ => "Building".into(),
            };
            view.note = match build["phase"].as_str() {
                Some("read") => match (build["done"].as_u64(), build["total"].as_u64()) {
                    (Some(done), Some(total)) => {
                        format!("Reading files · {} of {}", count(done), count(total))
                    }
                    _ => "Reading files".into(),
                },
                Some("assemble") => "Assembling and clustering the graph".into(),
                // A commit's graph waits for the project's own build, then
                // has the commit's files written out.
                Some("wait") => "Waiting for the project's graph to finish building".into(),
                Some("export") => "Writing out the files of that commit".into(),
                _ => "Scanning the project files".into(),
            };
            if published {
                view.note
                    .push_str(". The previous graph stays in use until this one is ready.");
            }
        }
        "ready" => {
            view.chip = "Ready".into();
            view.note = "The dependency map draws this graph.".into();
            view.rebuild = true;
        }
        "stale" => {
            view.chip = "Stale".into();
            view.note = "Files changed since this graph was built. It is rebuilt when a run starts or ends.".into();
            view.rebuild = true;
        }
        "failed" => {
            view.chip = "Failed".into();
            view.failed = true;
            let message = m::text(&status["failure"], "message");
            view.note = if message.is_empty() {
                "The last build failed.".to_owned()
            } else {
                format!("The last build failed with “{message}”.")
            };
            if published {
                view.note.push_str(" The previous graph stays in use.");
            }
            view.rebuild = true;
        }
        "missing" => {
            view.chip = "Parser ready".into();
            view.note = "Built-in code graph parser is available; optional Graphify enrichment is not installed (“uv tool install graphifyy” or “pipx install graphifyy”). Click Rebuild to build.".into();
            view.rebuild = true;
        }
        "empty" => {
            view.chip = "Not built".into();
            view.note = "No graph yet.".into();
            view.rebuild = true;
        }
        "off" => {
            view.chip = "Off".into();
            view.note = "Builds a graph of this project's code on this machine (built-in native parser with optional Graphify enrichment) and draws it as a dependency map: what uses what, down to classes and methods, for the whole project or for a change.".into();
        }
        // Nothing was read yet, or no project is selected.
        _ => {}
    }
    view
}
fn set_progress_bar(id: &str, percent: Option<u64>) {
    if let Some(bar) = dom::get(id) {
        // A bar without a value is the browser's indeterminate state.
        let _ = match percent {
            Some(percent) => bar.set_attribute("value", &percent.to_string()),
            None => bar.remove_attribute("value"),
        };
    }
}
/// English build progress line carrying the exact percentage when known.
pub fn build_progress_text(status: &V, view: &GraphView) -> String {
    let custom = m::text(&status["build"], "message");
    let custom = custom.trim();
    if !custom.is_empty() {
        return match view.percent {
            Some(pct) if !custom.contains('%') => format!("{custom} — {pct}%"),
            _ => custom.to_owned(),
        };
    }
    let build = &status["build"];
    let base = match build["phase"].as_str() {
        Some("read") => match (build["done"].as_u64(), build["total"].as_u64()) {
            (Some(done), Some(total)) => {
                format!(
                    "Building code graph: reading {} of {} files",
                    count(done),
                    count(total)
                )
            }
            _ => "Building code graph: reading files".to_owned(),
        },
        Some("assemble") => "Building code graph: assembling and clustering".to_owned(),
        Some("wait") => "Building code graph: waiting for project graph".to_owned(),
        Some("export") => "Building code graph: exporting commit files".to_owned(),
        _ => "Building code graph…".to_owned(),
    };
    match view.percent {
        Some(pct) if base.ends_with('…') => format!("{base} {pct}%"),
        Some(pct) => format!("{base} · {pct}%"),
        None => base,
    }
}
/// Compact `(data-state, label)` tuple for the app-wide sidebar graph pill.
pub fn sidebar_status(project: &str, status: &V, view: &GraphView) -> (&'static str, String) {
    if project.is_empty() {
        return ("idle", "Code graph: select a project".to_owned());
    }
    if !status.is_object() {
        return ("idle", "Code graph: loading…".to_owned());
    }
    if view.building {
        let label = match view.percent {
            Some(pct) => format!("Building code graph… {pct}%"),
            None => "Building code graph…".to_owned(),
        };
        return ("building", label);
    }
    match status["state"].as_str().unwrap_or("") {
        "ready" => ("ready", "Code graph ready".to_owned()),
        "stale" => (
            "stale",
            if status["graph"].is_object() {
                "Code graph stale"
            } else {
                "Code graph needs rebuild"
            }
            .to_owned(),
        ),
        "failed" => ("failed", "Code graph build failed".to_owned()),
        "missing" => ("missing", "Graphify not installed".to_owned()),
        "empty" => ("empty", "No code graph yet".to_owned()),
        "off" => ("off", "Code graph off".to_owned()),
        _ => ("idle", "Code graph: loading…".to_owned()),
    }
}
/// True when the embedded Sessions dependency map should refresh from server.
pub fn sync_needed(prev_project: &str, prev: &V, project: &str, next: &V) -> bool {
    if prev_project != project {
        return true;
    }
    if prev["state"] == "building" && next["state"] != "building" {
        return true;
    }
    if prev["state"] != "ready" && next["state"] == "ready" {
        return true;
    }
    if prev["build"].is_object() && !next["build"].is_object() {
        return true;
    }
    if prev["graph"].is_object() && !next["graph"].is_object() {
        return true;
    }
    next["graph"].is_object() && prev["graph"] != next["graph"]
}
fn apply_status(project: &str, status: V) {
    let prev = m::data("graph");
    let prev_project = m::text(&m::data("graph-sync-stamp"), "project");
    let should_sync = sync_needed(&prev_project, &prev, project, &status);
    m::set("graph", status);
    m::set("graph-sync-stamp", json!({ "project": project }));
    render();
    follow();
    if should_sync {
        crate::graph_map::sync_embedded(true);
    }
}
fn render() {
    let status = m::data("graph");
    let project = dom::value("project");
    let view = graph_view(&status);
    dom::text("option-graph-value", &view.chip);
    dom::attr(
        "option-graph",
        "data-on",
        if view.on { "true" } else { "false" },
    );
    dom::attr(
        "option-graph",
        "data-invalid",
        if view.failed { "true" } else { "false" },
    );
    dom::set_checked("graph-enabled", view.on);
    dom::disabled("graph-enabled", !status.is_object());
    dom::text("graph-note", &view.note);
    dom::set_checked("graph-agents", view.agents);
    dom::disabled("graph-agents", !view.on);
    dom::text("graph-agents-note", &view.offer);
    dom::hidden("graph-progress", !view.building);
    set_progress_bar("graph-progress-bar", view.percent);
    dom::text("graph-progress-amount", &view.amount);
    dom::text("graph-progress-left", &view.left);
    sync_embedded_progress_for_surface("session-graph", &status);
    if !m::read(|s| matches!(s.view.as_str(), "systems" | "system-changes")) {
        let (pill_state, pill_label) = sidebar_status(&project, &status, &view);
        dom::attr("sidebar-graph-pill", "data-state", pill_state);
        dom::text("sidebar-graph-label", &pill_label);
        let pill_title = if view.note.is_empty() {
            pill_label.clone()
        } else {
            view.note.clone()
        };
        dom::attr("sidebar-graph-pill", "title", &pill_title);
        let pill_aria = if view.building && !view.left.is_empty() {
            format!("{pill_label} ({})", view.left)
        } else {
            pill_label
        };
        dom::attr("sidebar-graph-pill", "aria-label", &pill_aria);
    }
    dom::text("graph-detail", &view.detail);
    dom::hidden("graph-detail", view.detail.is_empty());
    // The map draws the published graph, also while a newer one is being built.
    dom::hidden("graph-view", !status["graph"].is_object());
    dom::hidden("graph-changes", !status["graph"].is_object());
    dom::hidden("graph-rebuild", !view.rebuild);
    dom::hidden("graph-delete", !view.delete);
    sync_freshness_pill("graph-map", &status, &V::Null);
    sync_freshness_pill("session-graph", &status, &V::Null);
    crate::session::sync_working_indicator();
}

/// Computes `(data-tone, label)` for graph freshness pills in modal & embedded toolbars.
pub fn freshness_badge_info(status: &V, view: &V) -> (&'static str, String) {
    let state = status["state"].as_str().unwrap_or("").trim();
    if state == "building" || status["build"].is_object() {
        let pct = status["build"]["percent"].as_u64();
        let label = match pct {
            Some(p) => format!("Rebuilding ({p}%)"),
            None => "Rebuilding…".to_owned(),
        };
        return ("building", label);
    }
    if state == "failed" || status["failure"].is_object() {
        return ("error", "Build failed".to_owned());
    }
    let ir_rev = status["ir_revision"]
        .as_u64()
        .or_else(|| status["revision"].as_u64());
    let view_rev = view["ir_revision"].as_u64();
    let stale_rev =
        matches!((ir_rev, view_rev), (Some(sr), Some(vr)) if sr > 0 && vr > 0 && sr > vr);
    let truncated =
        status["ir_summary"]["truncated"] == true || view["ir_summary"]["truncated"] == true;
    let backend = status["backend"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| view["backend"].as_str().filter(|s| !s.is_empty()))
        .or_else(|| {
            status["graph"]["backend"]
                .as_str()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("");
    let is_native_ir = backend.contains("tree") || backend.contains("native");

    if state == "stale" || status["stale"] == true || stale_rev {
        let msg = if truncated {
            "Stale · Truncated IR"
        } else {
            "Stale"
        };
        return ("stale", msg.to_owned());
    }
    if truncated {
        return ("stale", "Native tree-sitter IR (truncated)".to_owned());
    }
    if is_native_ir {
        return ("ir", "Native tree-sitter IR".to_owned());
    }
    if state == "ready" || status["graph"].is_object() || view.is_object() {
        return ("ready", "Ready".to_owned());
    }
    ("off", String::new())
}

pub(crate) fn sync_freshness_pill(prefix: &str, status: &V, view: &V) {
    let pill_id = format!("{prefix}-freshness");
    let (tone, label) = freshness_badge_info(status, view);
    dom::attr(&pill_id, "data-tone", tone);
    dom::text(&pill_id, &label);
    dom::hidden(&pill_id, label.is_empty());
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GraphSseAction {
    Ignored,
    Building,
    RefreshCandidate {
        event_project: String,
        banner_text: String,
        schedule_idle: bool,
    },
}

/// Applies non-DOM model updates for SSE `event: "graph_status"` and invalidates matching
/// surface analysis cache keys without touching browser DOM globals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SseDeltaPayload {
    pub project_id: String,
    pub state: String,
    pub reason: String,
    pub revision: Option<u64>,
    pub nodes: Option<u64>,
    pub edges: Option<u64>,
    pub changed_files: Vec<String>,
    pub removed_files: Vec<String>,
    pub schema_changed: bool,
    pub full_reload: bool,
}

fn push_unique_rel_files(out: &mut Vec<String>, arr: &V) {
    let Some(items) = arr.as_array() else {
        return;
    };
    for item in items {
        let raw = item
            .as_str()
            .or_else(|| item["path"].as_str())
            .unwrap_or("")
            .trim()
            .replace('\\', "/");
        let norm = raw.trim_start_matches("./").trim_start_matches('/');
        if !norm.is_empty() && !out.iter().any(|existing| existing == norm) {
            out.push(norm.to_owned());
        }
    }
}

pub(crate) fn extract_sse_delta_payload(event: &V) -> SseDeltaPayload {
    let reason = m::text(event, "reason");
    let mut changed_files = Vec::new();
    for candidate in [
        &event["changed_files"],
        &event["files"],
        &event["delta"]["changed_files"],
        &event["delta"]["files"],
    ] {
        push_unique_rel_files(&mut changed_files, candidate);
    }
    let mut removed_files = Vec::new();
    for candidate in [
        &event["removed_files"],
        &event["deleted_files"],
        &event["delta"]["removed_files"],
        &event["delta"]["deleted_files"],
    ] {
        push_unique_rel_files(&mut removed_files, candidate);
    }
    let schema_changed = m::yes(&event["schema_changed"])
        || m::yes(&event["schema_updated"])
        || m::yes(&event["structural_change"]);
    let full_reload = m::yes(&event["full_reload"])
        || schema_changed
        || matches!(
            reason.as_str(),
            "rebuild" | "completed" | "architecture_updated" | "schema_changed"
        );
    SseDeltaPayload {
        project_id: m::text(event, "project_id"),
        state: m::text(event, "state"),
        reason,
        revision: event["revision"]
            .as_u64()
            .or_else(|| event["ir_revision"].as_u64()),
        nodes: event["nodes"]
            .as_u64()
            .or_else(|| event["graph"]["nodes"].as_u64()),
        edges: event["edges"]
            .as_u64()
            .or_else(|| event["graph"]["edges"].as_u64()),
        changed_files,
        removed_files,
        schema_changed,
        full_reload,
    }
}

fn sse_event_member_project(event: &V) -> String {
    let src = m::text(event, "source_project_id");
    if !src.is_empty() {
        return src;
    }
    m::text(event, "member_project_id")
}

pub(crate) fn apply_graph_status_sse_event(event: &V, current_project: &str) -> GraphSseAction {
    let active_view = m::read(|s| s.view.clone());
    apply_graph_status_sse_event_for_view(event, current_project, &active_view)
}

pub(crate) fn apply_graph_status_sse_event_for_view(
    event: &V,
    current_project: &str,
    active_view: &str,
) -> GraphSseAction {
    if !event.is_object() {
        return GraphSseAction::Ignored;
    }
    let delta = extract_sse_delta_payload(event);
    let owner_project = delta.project_id.clone();
    let member_project = sse_event_member_project(event);
    let event_project = if !member_project.is_empty() {
        member_project.clone()
    } else {
        owner_project.clone()
    };
    let same_or_global_project = event_project.is_empty()
        || current_project.is_empty()
        || event_project == current_project
        || (!owner_project.is_empty() && owner_project == current_project);
    let external_multi_project_view =
        matches!(active_view, "systems" | "system-changes" | "omnigraph");
    if !same_or_global_project && !external_multi_project_view {
        return GraphSseAction::Ignored;
    }
    let state = delta.state.clone();
    let reason = delta.reason.clone();
    let message = m::text(event, "message");
    if state == "building" {
        if same_or_global_project {
            let mut status = m::data("graph");
            if status.is_object() {
                status["state"] = json!("building");
                m::set("graph", status);
            }
        }
        return GraphSseAction::Building;
    }
    let is_refresh_candidate = matches!(state.as_str(), "ready" | "stale" | "updated" | "rebuilt")
        || matches!(
            reason.as_str(),
            "watcher"
                | "rebuild"
                | "completed"
                | "stale"
                | "ir_updated"
                | "incremental_update"
                | "architecture_updated"
                | "ir_delta"
        );
    if !is_refresh_candidate {
        return GraphSseAction::Ignored;
    }
    let should_invalidate = state == "ready"
        || state == "updated"
        || state == "rebuilt"
        || matches!(
            reason.as_str(),
            "watcher"
                | "ir_updated"
                | "incremental_update"
                | "architecture_updated"
                | "ir_delta"
                | "rebuild"
                | "completed"
        );
    if should_invalidate {
        crate::graph_map::invalidate_surface_analysis_caches_for_live_project(
            &event_project,
            current_project,
        );
        if !owner_project.is_empty() && owner_project != event_project {
            crate::graph_map::invalidate_surface_analysis_caches_for_live_project(
                &owner_project,
                current_project,
            );
        }
    }
    let banner_text = if !message.is_empty() {
        message
    } else if state == "stale" {
        "Code changed on disk — Graph may be stale".to_owned()
    } else {
        "Graph updated — Refresh".to_owned()
    };
    if !event_project.is_empty() {
        m::set(&format!("graph-sse-status:{event_project}"), event.clone());
    }
    if same_or_global_project {
        m::set("graph-sse-status", event.clone());
        let mut synth_status = m::data("graph");
        if !synth_status.is_object() {
            synth_status = json!({});
        }
        let prev_nodes = synth_status["graph"]["nodes"].as_u64();
        let prev_edges = synth_status["graph"]["edges"].as_u64();
        let meta = json!({
            "prev_nodes": prev_nodes,
            "prev_edges": prev_edges,
        });
        if !event_project.is_empty() {
            m::set(
                &format!("graph-sse-delta-meta:{event_project}"),
                meta.clone(),
            );
        }
        m::set("graph-sse-delta-meta", meta);
        synth_status["state"] = json!(if state.is_empty() {
            "ready"
        } else {
            state.as_str()
        });
        if let Some(rev) = delta.revision {
            synth_status["ir_revision"] = json!(rev);
        }
        if delta.nodes.is_some() || delta.edges.is_some() {
            if !synth_status["graph"].is_object() {
                synth_status["graph"] = json!({});
            }
            if let Some(nodes) = delta.nodes {
                synth_status["graph"]["nodes"] = json!(nodes);
            }
            if let Some(edges) = delta.edges {
                synth_status["graph"]["edges"] = json!(edges);
            }
        }
        if !event["backend"].is_null() {
            synth_status["backend"] = event["backend"].clone();
        }
        m::set("graph", synth_status);
    }
    GraphSseAction::RefreshCandidate {
        event_project,
        banner_text,
        schedule_idle: state != "stale",
    }
}

/// Processes SSE `event: "graph_status"` and `event: "system_graph_status"` updates without
/// destroying user camera or selection.
pub(crate) fn on_graph_status_sse(event: &V) {
    let current_project = dom::value("project");
    match apply_graph_status_sse_event(event, &current_project) {
        GraphSseAction::Ignored => {}
        GraphSseAction::Building => {
            let ev_proj = m::text(event, "project_id");
            let member_proj = sse_event_member_project(event);
            if ev_proj.is_empty()
                || current_project.is_empty()
                || ev_proj == current_project
                || (!member_proj.is_empty() && member_proj == current_project)
            {
                render();
            }
        }
        GraphSseAction::RefreshCandidate {
            event_project,
            banner_text,
            schedule_idle,
        } => {
            if event_project.is_empty()
                || current_project.is_empty()
                || event_project == current_project
            {
                render();
            }
            crate::graph_map::show_non_destructive_stale_banner(&event_project, &banner_text);
            if schedule_idle {
                crate::graph_map::maybe_schedule_idle_refresh(&event_project);
            } else {
                crate::graph_map::refresh_external_views_for_stale_sse(&event_project);
            }
        }
    }
}

/// Processes SSE `event: "resync"` when graph status/bridge events lagged behind.
pub(crate) fn on_graph_resync_sse(event: &V) {
    let reason = m::text(event, "reason");
    if !reason.is_empty() && reason != "graph_bridge_lagged" && !reason.starts_with("graph") {
        return;
    }
    let project = dom::value("project");
    if project.is_empty() {
        return;
    }
    crate::graph_map::invalidate_surface_analysis_caches(&project);
    crate::graph_map::show_non_destructive_stale_banner(&project, "Graph updates lagged — Refresh");
    #[cfg(target_arch = "wasm32")]
    spawn_local(async {
        load(false).await;
    });
}

/// Triggers an inline rebuild for the active modal or embedded surface scope.
#[allow(dead_code)]
pub(crate) async fn trigger_rebuild_for_scope(
    scope: GraphTargetScope,
    prefix: &str,
) -> Result<V, api::Error> {
    let busy_pill = format!("{prefix}-freshness");
    dom::attr(&busy_pill, "data-tone", "building");
    dom::text(&busy_pill, "Rebuilding…");
    dom::hidden(&busy_pill, false);
    let res = act_for_scope(&scope, "rebuild").await;
    match &res {
        Ok(status) => {
            if matches!(scope, GraphTargetScope::SingleProject { .. }) {
                apply_status(scope.project_id(), status.clone());
            } else {
                sync_embedded_progress_for_surface(prefix, status);
                sync_freshness_pill(prefix, status, &V::Null);
            }
        }
        Err(err) => {
            dom::attr(&busy_pill, "data-tone", "error");
            dom::text(&busy_pill, "Rebuild failed");
            dom::error(&format!("{prefix}-error"), &err.message);
        }
    }
    res
}

/// Updates the embedded surface build progress strip (`#{prefix}-progress*`) from a service/project status payload.
#[allow(dead_code)]
pub(crate) fn sync_embedded_progress_for_surface(prefix: &str, status: &V) {
    let view = graph_view(status);
    dom::hidden(&format!("{prefix}-progress"), !view.building);
    let progress_text = build_progress_text(status, &view);
    dom::text(&format!("{prefix}-progress-text"), &progress_text);
    set_progress_bar(
        &format!("{prefix}-progress-bar"),
        view.percent.map(|pct| pct.clamp(1, 99)),
    );
    dom::text(&format!("{prefix}-progress-amount"), &view.amount);
    dom::text(&format!("{prefix}-progress-left"), &view.left);
}

/// Renders the per-service Graphify status pill and monotonic `1..=99%` build progress bar
/// for `.system-card` entries in `#system-services`.
#[allow(dead_code)]
pub(crate) fn render_service_graph_pill_html(service_id: &str, status: &V) -> String {
    let esc_service = escape_html(service_id);
    if !status.is_object() {
        return format!(
            "<div class=\"service-graph-status\" data-service=\"{esc_service}\" data-state=\"idle\"><span class=\"sidebar-graph-pill service-graph-pill\" data-state=\"idle\" title=\"Code graph status loading\" aria-label=\"Code graph status loading for {esc_service}\"><span class=\"sidebar-graph-dot\" aria-hidden=\"true\"></span><span class=\"service-graph-chip\">Loading…</span></span></div>"
        );
    }
    let view = graph_view(status);
    let raw_state = status["state"].as_str().unwrap_or("");
    let pill_state = if view.building || raw_state == "building" {
        "building"
    } else if matches!(
        raw_state,
        "ready" | "stale" | "failed" | "missing" | "empty" | "off" | "denied"
    ) {
        raw_state
    } else if status["graph"].is_object() {
        "ready"
    } else {
        "idle"
    };
    let chip = if view.chip.is_empty() {
        match pill_state {
            "denied" => "Denied",
            "ready" => "Ready",
            "building" => "Building",
            "stale" => "Stale",
            "failed" => "Failed",
            "missing" => "Not installed",
            "empty" => "Not built",
            "off" => "Off",
            _ => "Idle",
        }
        .to_owned()
    } else {
        view.chip.clone()
    };
    let title = if !view.note.is_empty() {
        view.note.clone()
    } else if !view.detail.is_empty() {
        view.detail.clone()
    } else {
        format!("{service_id}: {chip}")
    };
    let esc_chip = escape_html(&chip);
    let esc_title = escape_html(&title);
    let mut html = format!(
        "<div class=\"service-graph-status\" data-service=\"{esc_service}\" data-state=\"{pill_state}\"><span class=\"sidebar-graph-pill service-graph-pill\" data-state=\"{pill_state}\" title=\"{esc_title}\" aria-label=\"{esc_service} code graph: {esc_chip}\"><span class=\"sidebar-graph-dot\" aria-hidden=\"true\"></span><span class=\"service-graph-chip\">{esc_chip}</span></span>"
    );
    if view.building {
        let clamped = view.percent.map(|pct| pct.clamp(1, 99));
        let progress_text = build_progress_text(status, &view);
        let value_attr = match clamped {
            Some(pct) => format!(" value=\"{pct}\""),
            None => String::new(),
        };
        let amount = match clamped {
            Some(pct) => format!("{pct}%"),
            None => view.amount.clone(),
        };
        html.push_str(&format!(
            "<div class=\"graph-progress service-graph-progress\"><div class=\"graph-progress-line\"><span class=\"graph-progress-phase\">{}</span><span class=\"graph-progress-amount\">{}</span><span class=\"graph-progress-left\">{}</span></div><progress max=\"100\"{value_attr}></progress></div>",
            escape_html(&progress_text),
            escape_html(&amount),
            escape_html(&view.left),
        ));
    } else if view.failed && !view.note.is_empty() {
        html.push_str(&format!(
            "<span class=\"service-graph-note knowledge-note\">{}</span>",
            escape_html(&view.note),
        ));
    } else if !view.detail.is_empty() {
        html.push_str(&format!(
            "<span class=\"service-graph-detail knowledge-note\">{}</span>",
            escape_html(&view.detail),
        ));
    }
    html.push_str("</div>");
    html
}

/// Computes `(data-state, label, title)` for `#sidebar-graph-pill` from `/api/systems/graph/status`.
pub fn system_sidebar_status(agg_status: &V) -> (&'static str, String, String) {
    if !agg_status.is_object() {
        return (
            "idle",
            "System graphs: select a system".to_owned(),
            "Select a project and system configuration to inspect service code graphs.".to_owned(),
        );
    }
    let services_obj = agg_status["services"].as_object();
    let mut counted_total = 0u64;
    let mut counted_ready = 0u64;
    let mut counted_building = 0u64;
    let mut counted_failed = 0u64;
    let mut counted_stale = 0u64;
    let mut pct_sum = 0u64;
    let mut pct_n = 0u64;
    if let Some(map) = services_obj {
        counted_total = map.len() as u64;
        for svc_status in map.values() {
            let st = svc_status["state"].as_str().unwrap_or("");
            if st == "building" || svc_status["build"].is_object() {
                counted_building += 1;
                if let Some(p) = svc_status["build"]["percent"].as_u64() {
                    pct_sum += p.clamp(1, 99);
                    pct_n += 1;
                }
            }
            if st == "ready" || svc_status["graph"].is_object() {
                counted_ready += 1;
            }
            if st == "failed" {
                counted_failed += 1;
            }
            if st == "stale" {
                counted_stale += 1;
            }
        }
    }
    let total = agg_status["total_count"].as_u64().unwrap_or(counted_total);
    let ready = agg_status["ready_count"].as_u64().unwrap_or(counted_ready);
    let building = agg_status["building_count"]
        .as_u64()
        .unwrap_or(counted_building);
    let top_state = agg_status["state"].as_str().unwrap_or("");

    if building > 0 || top_state == "building" {
        let pct_suffix = pct_sum
            .checked_div(pct_n)
            .map(|avg| format!(" · {}%", avg.clamp(1, 99)))
            .unwrap_or_default();
        let label = if total > 0 {
            format!("Building service graphs… ({ready}/{total}){pct_suffix}")
        } else {
            format!("Building service graphs…{pct_suffix}")
        };
        let title = if total > 0 {
            format!("{building} service graph(s) building · {ready} of {total} ready")
        } else {
            "Building system service code graphs…".to_owned()
        };
        return ("building", label, title);
    }
    if top_state == "missing" {
        return (
            "missing",
            "Graphify not installed".to_owned(),
            "Install Graphify (uv tool install graphifyy) to build service code graphs.".to_owned(),
        );
    }
    if top_state == "failed" || (counted_failed > 0 && ready == 0) {
        let failed = counted_failed.max(1);
        let label = if total > 0 {
            format!("Service graphs failed ({failed}/{total})")
        } else {
            "Service graphs build failed".to_owned()
        };
        return (
            "failed",
            label,
            format!("{failed} service code graph build(s) failed."),
        );
    }
    if total > 0 && ready >= total {
        if counted_stale > 0 || top_state == "stale" {
            return (
                "stale",
                format!("Service graphs stale ({ready}/{total})"),
                format!(
                    "{ready} of {total} service graphs published; rebuild needed for recent changes."
                ),
            );
        }
        return (
            "ready",
            format!("Service graphs ready ({ready}/{total})"),
            format!("All {total} system service code graphs are published and ready."),
        );
    }
    if ready > 0 {
        let state_key = if counted_stale > 0 || top_state == "stale" {
            "stale"
        } else {
            "ready"
        };
        return (
            state_key,
            format!("Service graphs: {ready}/{total} ready"),
            format!("{ready} of {total} system service code graphs published."),
        );
    }
    if top_state == "off" {
        return (
            "off",
            "Service graphs off".to_owned(),
            "Enable code graphs on member services to map dependencies.".to_owned(),
        );
    }
    if total > 0 {
        return (
            "empty",
            format!("Service graphs: 0/{total} built"),
            format!("0 of {total} service code graphs built yet."),
        );
    }
    (
        "idle",
        "System graphs: idle".to_owned(),
        "System code graphs idle.".to_owned(),
    )
}

/// Updates `#sidebar-graph-pill` and `#sidebar-graph-label` for multi-service System views.
#[allow(dead_code)]
pub(crate) fn sync_sidebar_pill_for_system(agg_status: &V) {
    let (pill_state, pill_label, pill_title) = system_sidebar_status(agg_status);
    dom::attr("sidebar-graph-pill", "data-state", pill_state);
    dom::text("sidebar-graph-label", &pill_label);
    dom::attr("sidebar-graph-pill", "title", &pill_title);
    dom::attr("sidebar-graph-pill", "aria-label", &pill_title);
}

/// Executes a graph mutation (`enable|disable|offer|withhold|rebuild|delete`) against any `GraphTargetScope`.
#[allow(dead_code)]
pub(crate) async fn act_for_scope(scope: &GraphTargetScope, action: &str) -> Result<V, api::Error> {
    let (path, body) = scope.act_request(action, None);
    api::post(&path, body).await
}

fn follow() {
    let status = m::data("graph");
    if status["build"].is_object() || status["state"] == "building" {
        dom::later("graph-poll", 1000, || {
            spawn_local(async {
                load(false).await;
            })
        });
    } else {
        dom::cancel_timer("graph-poll");
    }
}
/// Reads the graph of the selected project. `visit` lets the server rebuild a
/// graph whose files changed, as when the project is opened.
pub async fn load(visit: bool) {
    let scope = GraphTargetScope::single(dom::value("project"));
    if scope.is_empty() {
        apply_status("", V::Null);
        return;
    }
    let project = scope.project_id().to_owned();
    let path = scope.status_url(visit);
    if let Ok((epoch, status)) = api::scoped("project-graph", &path).await {
        if !m::current("project-graph", epoch) || dom::value("project") != project {
            return;
        }
        apply_status(&project, status);
    }
}
/// A run starting or ending is when the server brings the graph up to date.
/// The build begins a moment after the run does, so the graph is read again
/// shortly after either change.
pub fn run_state(active: bool) {
    if m::data("graph-run-active") == json!(active) {
        return;
    }
    m::set("graph-run-active", json!(active));
    if !active {
        crate::graph_map::sync_embedded_changes();
    }
    if m::data("graph")["enabled"] != true {
        return;
    }
    for (key, delay) in [
        ("graph-run", 0),
        ("graph-run-soon", 4000),
        ("graph-run-later", 12000),
    ] {
        dom::later(key, delay, || {
            spawn_local(async {
                if !m::data("graph")["build"].is_object() {
                    load(false).await;
                }
            })
        });
    }
}
async fn act(action: &str) {
    let scope = GraphTargetScope::single(dom::value("project"));
    if scope.is_empty() {
        return;
    }
    let project = scope.project_id().to_owned();
    dom::error("graph-error", "");
    match act_for_scope(&scope, action).await {
        Ok(status) if dom::value("project") == project => {
            apply_status(&project, status);
            return;
        }
        Ok(_) => return,
        Err(error) => dom::error("graph-error", &error.message),
    }
    render();
    follow();
}
pub async fn event(kind: &str, id: &str, checked: bool) -> bool {
    let action = match (kind, id) {
        ("change", "graph-enabled") if checked => "enable",
        ("change", "graph-enabled") => "disable",
        ("change", "graph-agents") if checked => "offer",
        ("change", "graph-agents") => "withhold",
        ("click", "graph-rebuild") => "rebuild",
        ("click", "graph-delete") => "delete",
        _ => return false,
    };
    act(action).await;
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    fn status(state: &str, extra: V) -> V {
        let mut status = json!({
            "enabled": state != "off",
            "state": state,
            "tool": {"available": state != "missing", "version": "0.9.105"},
            "graph": null,
            "build": null,
            "failure": null,
        });
        for (key, value) in extra.as_object().into_iter().flatten() {
            status[key] = value.clone();
        }
        status
    }
    fn graph() -> V {
        json!({"commit":"21973e93d248","nodes":50900,"edges":100934,"bytes":55517314,"disk":182452224,"seconds":44.2})
    }
    #[test]
    fn a_running_build_shows_percent_and_time_left() {
        let view = graph_view(&status(
            "building",
            json!({"build":{"phase":"read","done":430,"total":3520,"percent":7,"remaining_seconds":47,"overdue":false}}),
        ));
        assert_eq!(view.chip, "7% · 47 s");
        assert_eq!((view.percent, view.amount.as_str()), (Some(7), "7%"));
        assert_eq!(view.left, "about 47 s left");
        assert_eq!(view.note, "Reading files · 430 of 3,520");
        assert!(view.building && view.on && view.delete && !view.rebuild);
        let later = graph_view(&status(
            "building",
            json!({"graph":graph(),"build":{"phase":"assemble","percent":64,"remaining_seconds":95,"overdue":false}}),
        ));
        assert_eq!(later.chip, "64% · 1 min 35 s");
        assert_eq!(later.left, "about 1 min 35 s left");
        assert_eq!(
            later.note,
            "Assembling and clustering the graph. The previous graph stays in use until this one is ready."
        );
    }
    #[test]
    fn a_build_without_an_estimate_is_indeterminate_and_an_overdue_one_says_so() {
        let first = graph_view(&status(
            "building",
            json!({"build":{"phase":"scan","percent":null,"remaining_seconds":null,"overdue":false}}),
        ));
        assert_eq!((first.percent, first.chip.as_str()), (None, "Building"));
        assert_eq!((first.amount.as_str(), first.left.as_str()), ("", ""));
        assert_eq!(first.note, "Scanning the project files");
        let overdue = graph_view(&status(
            "building",
            json!({"build":{"phase":"assemble","percent":99,"remaining_seconds":0,"overdue":true}}),
        ));
        assert_eq!(overdue.chip, "99%");
        assert_eq!(overdue.left, "longer than the last build");
        let first_overdue = graph_view(&status(
            "building",
            json!({"build":{"phase":"assemble","percent":99,"remaining_seconds":0,"rough":true,"overdue":true}}),
        ));
        assert_eq!(first_overdue.left, "longer than expected");
    }
    #[test]
    fn a_published_graph_is_described_once() {
        let view = graph_view(&status("ready", json!({"graph":graph()})));
        assert_eq!(view.chip, "Ready");
        assert_eq!(
            view.detail,
            "Commit 21973e93 · 50,900 nodes · 100,934 edges · 174 MB on disk · built in 44 s · Graphify 0.9.105"
        );
        assert!(view.rebuild && view.delete && !view.building && !view.failed);
        assert_eq!(
            graph_view(&status("stale", json!({"graph":graph()}))).chip,
            "Stale"
        );
    }
    #[test]
    fn off_missing_and_failed_say_what_to_do() {
        let off = graph_view(&status("off", json!({})));
        assert_eq!((off.chip.as_str(), off.on), ("Off", false));
        assert!(!off.rebuild && !off.delete);
        let missing = graph_view(&status("missing", json!({})));
        assert_eq!(missing.chip, "Parser ready");
        assert!(missing.note.contains("uv tool install graphifyy") && missing.rebuild);
        let failed = graph_view(&status(
            "failed",
            json!({"graph":graph(),"failure":{"message":"boom"}}),
        ));
        assert!(failed.failed);
        assert_eq!(
            failed.note,
            "The last build failed with “boom”. The previous graph stays in use."
        );
        assert_eq!(
            graph_view(&status("failed", json!({"failure":{"message":""}}))).note,
            "The last build failed."
        );
        assert_eq!(graph_view(&V::Null), GraphView::default());
    }
    #[test]
    fn telling_agents_about_the_graph_is_a_switch_of_its_own() {
        // A ready graph is the map's; agents hear of it only when asked.
        let ready = graph_view(&status("ready", json!({"graph":graph()})));
        assert_eq!(ready.note, "The dependency map draws this graph.");
        assert!(ready.on && !ready.agents);
        assert_eq!(
            ready.offer,
            "Agents are not told about the graph: it is built for the map. Telling them adds lines to every launch."
        );
        let told = graph_view(&status("ready", json!({"graph":graph(),"agents":true})));
        assert!(told.agents);
        assert!(
            told.offer.starts_with("Each launch adds a few lines"),
            "{}",
            told.offer
        );
        assert_eq!(told.note, ready.note);
        // The choice is kept while the graph is off, and waits for it.
        let off = graph_view(&status("off", json!({"agents":true})));
        assert!(!off.on && off.agents);
        assert_eq!(
            off.offer,
            "Agents can be told about the graph once it is on."
        );
        assert!(off
            .note
            .starts_with("Builds a graph of this project's code"));
        assert!(!off.note.contains("agents"));
        assert_eq!(graph_view(&V::Null).offer, "");
    }
    #[test]
    fn a_first_build_counts_down_in_coarse_steps() {
        let rough = |left: u64| {
            graph_view(&status(
                "building",
                json!({"build":{"phase":"read","percent":8,"remaining_seconds":left,"rough":true,"overdue":false}}),
            ))
            .left
        };
        // The guesses 54, 72, 68, 75 and 72 seconds read as two steady values.
        assert_eq!(rough(54), "about 1 min left");
        for left in [72, 68, 75, 72] {
            assert_eq!(rough(left), "about 1 min 15 s left");
        }
        assert_eq!(rough(40), "about 45 s left");
        assert_eq!(rough(6), "about 5 s left");
        assert_eq!(rough(0), "about 5 s left");
        assert_eq!(rough(200), "about 3 min 30 s left");
    }
    #[test]
    fn numbers_read_at_a_glance() {
        assert_eq!(count(7), "7");
        assert_eq!(count(1000), "1,000");
        assert_eq!(count(1234567), "1,234,567");
        assert_eq!(span(59), "59 s");
        assert_eq!(span(120), "2 min");
        assert_eq!(megabytes(5_242_880), "5.0 MB");
    }
    #[test]
    fn sidebar_and_progress_use_clear_english_with_percent() {
        let building = status(
            "building",
            json!({
                "build": {
                    "phase": "read",
                    "done": 120,
                    "total": 400,
                    "percent": 30,
                    "remaining_seconds": 18,
                    "rough": false,
                    "overdue": false
                }
            }),
        );
        let view = graph_view(&building);
        assert_eq!(
            sidebar_status("app", &building, &view),
            ("building", "Building code graph… 30%".to_owned())
        );
        assert_eq!(
            build_progress_text(&building, &view),
            "Building code graph: reading 120 of 400 files · 30%"
        );

        let custom = status(
            "building",
            json!({
                "build": {
                    "phase": "assemble",
                    "percent": 65,
                    "message": "Indexing classes and methods"
                }
            }),
        );
        assert_eq!(
            build_progress_text(&custom, &graph_view(&custom)),
            "Indexing classes and methods — 65%"
        );

        let ready = status("ready", json!({ "graph": graph() }));
        assert_eq!(
            sidebar_status("app", &ready, &graph_view(&ready)),
            ("ready", "Code graph ready".to_owned())
        );
        assert_eq!(
            sidebar_status("", &V::Null, &GraphView::default()),
            ("idle", "Code graph: select a project".to_owned())
        );
    }
    #[test]
    fn embedded_map_sync_triggers_on_completion_project_change_or_fingerprint_change() {
        let building = status(
            "building",
            json!({ "build": { "phase": "read", "percent": 40 } }),
        );
        let ready = status("ready", json!({ "graph": graph() }));
        let mut updated_graph = graph();
        updated_graph["commit"] = json!("deadbeef");
        let ready_next = status("ready", json!({ "graph": updated_graph }));

        assert!(sync_needed("", &V::Null, "proj-a", &building));
        assert!(!sync_needed("proj-a", &building, "proj-a", &building));
        assert!(sync_needed("proj-a", &building, "proj-a", &ready));
        assert!(!sync_needed("proj-a", &ready, "proj-a", &ready));
        assert!(sync_needed("proj-a", &ready, "proj-a", &ready_next));
        assert!(sync_needed("proj-a", &ready, "proj-b", &ready));

        let off = status("off", json!({}));
        let stale = status("stale", json!({ "graph": graph() }));
        assert!(sync_needed("proj-a", &off, "proj-a", &ready));
        assert!(sync_needed("proj-a", &stale, "proj-a", &ready));
        assert!(sync_needed("proj-a", &ready, "proj-a", &off));
    }
    #[test]
    fn graph_target_scope_builds_single_and_system_service_urls_and_json_roundtrip() {
        let single = GraphTargetScope::single("my/proj");
        assert!(!single.is_empty());
        assert!(!single.is_system());
        assert_eq!(single.status_url(false), "/api/projects/my%2Fproj/graph");
        assert_eq!(
            single.status_url(true),
            "/api/projects/my%2Fproj/graph?check=1"
        );
        let (act_url, act_body) = single.act_request("rebuild", None);
        assert_eq!(act_url, "/api/projects/my%2Fproj/graph");
        assert_eq!(act_body, json!({"action": "rebuild"}));
        assert_eq!(GraphTargetScope::from_json(&single.to_json()), Some(single));

        let sys = GraphTargetScope::SystemService {
            project_id: "lab/root".to_owned(),
            config_path: "harbor-system/system.json".to_owned(),
            service_id: "orders".to_owned(),
            services: vec!["orders".to_owned(), "inventory".to_owned()],
            run_id: Some("chg-42".to_owned()),
            run_title: Some("Update order contract".to_owned()),
            launches: vec![json!({"id": "l1", "number": 1})],
        };
        assert!(!sys.is_empty());
        assert!(sys.is_system());
        assert_eq!(sys.service_id(), Some("orders"));
        assert_eq!(
            sys.status_url(true),
            "/api/systems/service-graph?project_id=lab%2Froot&config_path=harbor-system%2Fsystem.json&service=orders&check=1"
        );
        let (sys_url, sys_body) = sys.act_request("changes", Some("pr:12"));
        assert_eq!(sys_url, "/api/systems/service-graph");
        assert_eq!(
            sys_body,
            json!({
                "project_id": "lab/root",
                "config_path": "harbor-system/system.json",
                "service": "orders",
                "action": "changes",
                "target": "pr:12",
            })
        );
        let switched = sys.with_service("__system__");
        assert_eq!(switched.service_id(), Some("__system__"));
        assert_eq!(GraphTargetScope::from_json(&sys.to_json()), Some(sys));
    }
    #[test]
    fn service_graph_pill_renders_monotonic_progress_ready_detail_and_escapes_html() {
        let building_zero = status(
            "building",
            json!({ "build": { "phase": "read", "done": 1, "total": 200, "percent": 0, "remaining_seconds": 18 } }),
        );
        let pill_zero = render_service_graph_pill_html("orders<svc>", &building_zero);
        assert!(pill_zero.contains("data-service=\"orders&lt;svc&gt;\""));
        assert!(pill_zero.contains("data-state=\"building\""));
        assert!(pill_zero.contains("<progress max=\"100\" value=\"1\"></progress>"));

        let building_hundred = status(
            "building",
            json!({ "build": { "phase": "assemble", "percent": 100, "remaining_seconds": 1 } }),
        );
        let pill_hundred = render_service_graph_pill_html("inventory", &building_hundred);
        assert!(pill_hundred.contains("<progress max=\"100\" value=\"99\"></progress>"));

        let ready = status("ready", json!({ "graph": graph() }));
        let pill_ready = render_service_graph_pill_html("__system__", &ready);
        assert!(pill_ready.contains("data-state=\"ready\""));
        assert!(pill_ready.contains("Ready"));
        assert!(pill_ready.contains("50,900 nodes"));
    }
    #[test]
    fn system_sidebar_status_summarizes_building_ready_partial_and_failed_states() {
        let building_agg = json!({
            "state": "building",
            "ready_count": 3,
            "total_count": 5,
            "building_count": 2,
            "services": {
                "orders": { "state": "building", "build": { "percent": 60 } },
                "inventory": { "state": "building", "build": { "percent": 20 } },
            }
        });
        let (st, label, _) = system_sidebar_status(&building_agg);
        assert_eq!(st, "building");
        assert_eq!(label, "Building service graphs… (3/5) · 40%");

        let ready_agg = json!({
            "state": "ready",
            "ready_count": 5,
            "total_count": 5,
            "building_count": 0,
        });
        assert_eq!(system_sidebar_status(&ready_agg).0, "ready");
        assert_eq!(
            system_sidebar_status(&ready_agg).1,
            "Service graphs ready (5/5)"
        );

        let failed_agg = json!({
            "state": "failed",
            "ready_count": 0,
            "total_count": 4,
            "building_count": 0,
            "services": {
                "orders": { "state": "failed" },
                "payments": { "state": "failed" },
            }
        });
        assert_eq!(system_sidebar_status(&failed_agg).0, "failed");
        assert_eq!(
            system_sidebar_status(&failed_agg).1,
            "Service graphs failed (2/4)"
        );
    }
    #[test]
    fn freshness_badge_info_reports_ready_stale_building_and_native_ir() {
        let ready = status(
            "ready",
            json!({ "backend": "tree_sitter_ir", "ir_revision": 4 }),
        );
        assert_eq!(
            freshness_badge_info(&ready, &V::Null),
            ("ir", "Native tree-sitter IR".to_owned())
        );

        let stale_rev = freshness_badge_info(&ready, &json!({ "ir_revision": 2 }));
        assert_eq!(stale_rev, ("stale", "Stale".to_owned()));

        let trunc = freshness_badge_info(&ready, &json!({ "ir_summary": { "truncated": true } }));
        assert_eq!(
            trunc,
            ("stale", "Native tree-sitter IR (truncated)".to_owned())
        );

        let building = status("building", json!({ "build": { "percent": 55 } }));
        assert_eq!(
            freshness_badge_info(&building, &V::Null),
            ("building", "Rebuilding (55%)".to_owned())
        );
    }

    #[test]
    fn extract_sse_delta_payload_collects_incremental_files_revision_and_full_reload_flags() {
        let inc = extract_sse_delta_payload(&json!({
            "project_id": "p1",
            "state": "updated",
            "reason": "incremental_update",
            "revision": 77,
            "nodes": 120,
            "edges": 310,
            "changed_files": ["./src\\auth.rs", "src/auth.rs", "src/db.rs"]
        }));
        assert_eq!(inc.revision, Some(77));
        assert_eq!(inc.nodes, Some(120));
        assert_eq!(inc.edges, Some(310));
        assert_eq!(inc.changed_files, vec!["src/auth.rs", "src/db.rs"]);
        assert!(!inc.full_reload);

        let schema = extract_sse_delta_payload(&json!({
            "project_id": "p1",
            "state": "ready",
            "reason": "ir_updated",
            "schema_changed": true,
            "changed_files": ["src/a.rs"]
        }));
        assert!(schema.schema_changed);
        assert!(schema.full_reload);
    }

    #[test]
    fn apply_graph_status_sse_admits_external_projects_in_multi_project_views_only() {
        m::set(
            "graph",
            json!({ "state": "ready", "ir_revision": 10, "graph": { "nodes": 25, "edges": 40 } }),
        );
        let ext_event = json!({
            "project_id": "svc-member",
            "state": "updated",
            "reason": "incremental_update",
            "revision": 77,
            "nodes": 999,
            "edges": 888,
            "changed_files": ["src/api.rs"]
        });

        assert_eq!(
            apply_graph_status_sse_event_for_view(&ext_event, "proj-primary", "sessions"),
            GraphSseAction::Ignored
        );

        for view in ["systems", "system-changes", "omnigraph"] {
            m::write(|s| s.view = view.to_owned());
            let action = apply_graph_status_sse_event(&ext_event, "proj-primary");
            assert_eq!(
                action,
                GraphSseAction::RefreshCandidate {
                    event_project: "svc-member".to_owned(),
                    banner_text: "Graph updated — Refresh".to_owned(),
                    schedule_idle: true,
                },
                "expected external project SSE admit for view={view}"
            );
            // Primary project's sidebar status must NOT be overwritten by external project counts.
            assert_eq!(m::data("graph")["ir_revision"], json!(10));
            assert_eq!(m::data("graph")["graph"]["nodes"], json!(25));
        }

        let sys_ir_delta = json!({
            "project_id": "sys-owner",
            "reason": "ir_delta",
            "ir_revision": 88,
        });
        assert_eq!(
            apply_graph_status_sse_event_for_view(&sys_ir_delta, "proj-primary", "systems"),
            GraphSseAction::RefreshCandidate {
                event_project: "sys-owner".to_owned(),
                banner_text: "Graph updated — Refresh".to_owned(),
                schedule_idle: true,
            }
        );

        // External project and `building` events must NOT clobber primary project's singleton `graph-sse-status`.
        let primary_ready = json!({
            "project_id": "proj-primary",
            "state": "ready",
            "reason": "incremental_update",
            "revision": 12,
            "changed_files": ["src/main.rs"]
        });
        assert!(matches!(
            apply_graph_status_sse_event_for_view(&primary_ready, "proj-primary", "systems"),
            GraphSseAction::RefreshCandidate { .. }
        ));
        assert_eq!(m::data("graph-sse-status")["revision"], json!(12));

        let ext_building = json!({
            "project_id": "svc-member",
            "state": "building",
            "reason": "build_started",
        });
        assert_eq!(
            apply_graph_status_sse_event_for_view(&ext_building, "proj-primary", "systems"),
            GraphSseAction::Building
        );
        assert_eq!(m::data("graph-sse-status")["revision"], json!(12));

        let sys_member_ir_delta = json!({
            "project_id": "sys-owner",
            "source_project_id": "proj-api",
            "service": "api",
            "state": "updated",
            "reason": "ir_delta",
            "ir_revision": 91,
        });
        assert_eq!(
            apply_graph_status_sse_event_for_view(&sys_member_ir_delta, "proj-primary", "systems"),
            GraphSseAction::RefreshCandidate {
                event_project: "proj-api".to_owned(),
                banner_text: "Graph updated — Refresh".to_owned(),
                schedule_idle: true,
            }
        );
        assert_eq!(
            m::data("graph-sse-status:proj-api")["ir_revision"],
            json!(91)
        );
        assert_eq!(m::data("graph-sse-status")["revision"], json!(12));

        m::write(|s| s.view = "sessions".to_owned());
    }
}
