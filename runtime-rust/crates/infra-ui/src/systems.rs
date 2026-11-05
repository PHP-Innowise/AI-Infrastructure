use crate::{api, dom, management, model as m, session};
use serde_json::{json, Value as V};
use std::collections::{BTreeMap, VecDeque};
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
    dom::cancel_timer("discovery-poll");
    for key in [
        "editor",
        "editor-generation",
        "system-run",
        "system-catalog",
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
            render_run();
            agents(&value).await;
            // Sessions shows each launch; its status changes when the run stops.
            if stopped {
                session::refresh().await;
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
        if e.closest("#system-discovery-agents")
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
    let notice = if scanning {
        "An AI scan is filling the system editor on Services. Planning and launching wait until it ends."
    } else if dirty {
        "The system editor on Services has unsaved changes. Save or discard them to plan or launch a change."
    } else {
        ""
    };
    dom::text("system-editor-lock", notice);
    dom::hidden("system-editor-lock", notice.is_empty());
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
    layout();
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
fn svg(tag: &str, attrs: &[(&str, String)], text: &str) -> web_sys::Element {
    let e = dom::document()
        .create_element_ns(Some("http://www.w3.org/2000/svg"), tag)
        .expect("SVG element");
    for (k, v) in attrs {
        let _ = e.set_attribute(k, v);
    }
    e.set_text_content(Some(text));
    e
}
/// The line from a provider node to a consumer node (190×76 boxes on a 236×124
/// grid). It leaves and enters on the facing sides, so it never crosses its own
/// two boxes; a pair linked both ways gets two parallel lines; a link that skips
/// boxes in the same row arcs over them (forward) or under them (backward).
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
    let columns = ordered.len().clamp(1, 4);
    let width = columns * 236 + 2;
    let height = ordered.len().div_ceil(columns) * 124;
    let map = svg(
        "svg",
        &[
            ("viewBox", format!("0 0 {width} {height}")),
            ("width", width.to_string()),
            ("height", height.to_string()),
            ("role", "img".into()),
            ("aria-label", "Declared service contract graph".into()),
        ],
        "",
    );
    let defs = svg("defs", &[], "");
    let marker = svg(
        "marker",
        &[
            ("id", "system-arrow".into()),
            ("viewBox", "0 0 10 10".into()),
            ("refX", "9".into()),
            ("refY", "5".into()),
            ("markerWidth", "6".into()),
            ("markerHeight", "6".into()),
            ("orient", "auto-start-reverse".into()),
        ],
        "",
    );
    dom::add(
        &marker,
        &svg(
            "path",
            &[
                ("d", "M 0 0 L 10 5 L 0 10 z".into()),
                ("class", "system-arrow".into()),
            ],
            "",
        ),
    );
    dom::add(&defs, &marker);
    dom::add(&map, &defs);
    let positions: BTreeMap<_, _> = ordered
        .iter()
        .enumerate()
        .map(|(i, id)| {
            (
                id.clone(),
                (24 + (i % columns) * 236, 24 + (i / columns) * 124),
            )
        })
        .collect();
    for edge in &edges {
        if let (Some(a), Some(b)) = (
            positions.get(&m::text(edge, "provider")),
            positions.get(&m::text(edge, "consumer")),
        ) {
            let path = svg(
                "path",
                &[
                    ("d", edge_path(*a, *b)),
                    ("class", "system-edge".into()),
                    ("marker-end", "url(#system-arrow)".into()),
                ],
                "",
            );
            dom::add(
                &path,
                &svg(
                    "title",
                    &[],
                    &format!(
                        "{} → {}: {}",
                        m::text(edge, "provider"),
                        m::text(edge, "consumer"),
                        m::text(edge, "contract")
                    ),
                ),
            );
            dom::add(&map, &path);
        }
    }
    dom::clear("system-services");
    let chosen = dom::selected_checks("system-plan-services");
    dom::clear("system-plan-services");
    for id in ordered {
        let Some(service) = services.iter().find(|s| s["id"] == id) else {
            continue;
        };
        let (x, y) = positions[&id];
        let group = svg(
            "g",
            &[(
                "class",
                format!(
                    "system-node{}",
                    if service["access"] == "available" {
                        ""
                    } else {
                        " unavailable"
                    }
                ),
            )],
            "",
        );
        dom::add(
            &group,
            &svg(
                "rect",
                &[
                    ("x", x.to_string()),
                    ("y", y.to_string()),
                    ("width", "190".into()),
                    ("height", "76".into()),
                    ("rx", "8".into()),
                ],
                "",
            ),
        );
        dom::add(
            &group,
            &svg(
                "text",
                &[
                    ("x", (x + 16).to_string()),
                    ("y", (y + 28).to_string()),
                    ("class", "system-node-title".into()),
                ],
                &id,
            ),
        );
        dom::add(
            &group,
            &svg(
                "text",
                &[
                    ("x", (x + 16).to_string()),
                    ("y", (y + 48).to_string()),
                    ("class", "system-node-meta".into()),
                ],
                service["declared"]["owner"]
                    .as_str()
                    .unwrap_or("No passport"),
            ),
        );
        dom::add(&group, &svg("title", &[], &m::text(service, "root")));
        dom::add(&map, &group);
        let card = dom::el("article", "system-card", "");
        dom::add(&card, &dom::el("h4", "", &id));
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
        dom::append("system-services", &card);
        let choice = dom::check(&id, "system-service", &id, chosen.iter().any(|v| v == &id));
        if service["access"] != "available" {
            if let Ok(Some(input)) = choice.query_selector("input") {
                let _ = input.set_attribute("disabled", "");
            }
        }
        dom::append("system-plan-services", &choice);
    }
    dom::clear("system-map");
    dom::append("system-map", &map);
    dom::rows(
        "system-relationships",
        &edges
            .iter()
            .map(|edge| {
                let mut edge = edge.clone();
                edge["title"] = json!(format!(
                    "{} → {} · {} v{}",
                    m::text(&edge, "provider"),
                    m::text(&edge, "consumer"),
                    m::text(&edge, "contract"),
                    m::text(&edge, "version")
                ));
                edge
            })
            .collect::<Vec<_>>(),
    );
    for warning in m::array(&catalog["warnings"]) {
        dom::append(
            "system-relationships",
            &dom::el("p", "system-warning", &warning_text(&warning)),
        );
    }
    dom::clear("system-banks");
    for service in services {
        dom::append(
            "system-banks",
            &dom::el(
                "p",
                "system-card-meta",
                &format!(
                    "{} · {}/project-brain · {}/memory-bank",
                    m::text(&service, "id"),
                    m::text(&service, "root"),
                    m::text(&service, "root")
                ),
            ),
        );
    }
    controls();
}
fn render_run() {
    let v = m::data("system-run");
    if !v.is_object() {
        dom::hidden("system-run", true);
        return;
    }
    dom::hidden("system-run", false);
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
    dom::pretty("system-plan-json", &v["plan"]);
    dom::clear("system-plan-summary");
    for (term, value) in [
        ("Participants", joined(&v["plan"]["context"]["services"])),
        (
            "Context",
            format!(
                "{} of {} characters · {} sources · {} omitted",
                m::string(&v["plan"]["context_chars"]),
                m::string(&v["plan"]["context_budget_chars"]),
                m::array(&v["plan"]["context"]["sources"]).len(),
                m::array(&v["plan"]["omitted_sources"]).len()
            ),
        ),
        ("System file", m::text(&v, "config_path")),
    ] {
        dom::append("system-plan-summary", &dom::el("dt", "", term));
        dom::append("system-plan-summary", &dom::el("dd", "", &value));
    }
    let steps = if v["execution"]["steps"].is_array() {
        m::array(&v["execution"]["steps"])
    } else {
        m::array(&v["plan"]["steps"])
    };
    let running = m::yes(&v["active"]);
    dom::rows(
        "system-flow",
        &steps
            .iter()
            .map(|step| {
                let mut step = step.clone();
                let status = shown_status(&step, &v["receipts"], running);
                let status = if status == "reported" {
                    "reported complete; resume to record it".to_owned()
                } else {
                    status
                };
                step["title"] = json!([m::text(&step, "id"), m::label(&status)]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · "));
                step
            })
            .collect::<Vec<_>>(),
    );
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
    dom::rows(
        "system-receipts",
        &m::array(&v["receipts"])
            .iter()
            .map(|receipt| {
                let mut receipt = receipt.clone();
                let accepted = steps.iter().any(|step| {
                    step["id"] == receipt["phase"]
                        && step["attempt"] == receipt["attempt"]
                        && step["accepted_unrun_checks"].is_array()
                });
                let outcome = if accepted {
                    format!("{}; a person accepted them", receipt_outcome(&receipt))
                } else {
                    receipt_outcome(&receipt)
                };
                receipt["title"] = json!(format!(
                    "{} · attempt {} · {}",
                    m::text(&receipt, "phase"),
                    m::string(&receipt["attempt"]),
                    outcome
                ));
                receipt
            })
            .collect::<Vec<_>>(),
    );
    dom::pretty("system-native-tasks", &v["execution"]["tasks"]);
    dom::hidden(
        "system-native-details",
        !v["execution"]["tasks"].is_object(),
    );
    dom::text(
        "system-handoff",
        &if v["handoff"].is_null() {
            String::new()
        } else {
            format!(
                "Handoff saved: {}/handoff.json\n{}",
                m::text(&v["execution"], "run_dir"),
                m::text(&v["handoff"], "knowledge")
            )
        },
    );
    dom::text(
        "system-events",
        &m::array(&v["events"])
            .iter()
            .filter(|v| v["kind"] != "user")
            .map(|v| m::text(v, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let launches = m::array(&v["launches"]);
    let options=launches.iter().enumerate().map(|(i,l)|json!({"id":l["session_id"],"label":format!("Launch {} · {}",i+1,m::text(l,"status"))})).collect::<Vec<_>>();
    let chosen = m::data("system-agent-launch")
        .as_str()
        .map(str::to_owned)
        .or_else(|| launches.last().map(|v| m::text(v, "session_id")));
    dom::options(
        "system-agents-launch",
        &options,
        m::summary,
        chosen.as_deref(),
    );
    dom::hidden("system-agents-section", launches.is_empty());
    dom::hidden("system-agents-live", !m::yes(&v["active"]));
    dom::hidden("system-open-session", !v["session_id"].is_string());
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
fn rows(parent: &web_sys::Element, title: &str, values: &V, path: &str, kind: &str) {
    let d = dom::el("details", "system-editor-section", "");
    dom::add(
        &d,
        &dom::el(
            "summary",
            "",
            &format!("{title} ({})", m::array(values).len()),
        ),
    );
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
    dom::add(
        &d,
        &dom::button(
            &format!("Add {}", name.to_lowercase()),
            "editor-row-add",
            json!({"path":path,"kind":kind}),
        ),
    );
    dom::add(parent, &d);
}
fn render_editor() {
    let data = m::data("editor");
    if !data.is_object() {
        return;
    }
    dom::hidden("system-editor", false);
    dom::set_value("system-editor-name", &m::text(&data, "name"));
    dom::text(
        "system-editor-location",
        &format!(
            "{} / {}",
            m::text(&m::project(&m::text(&data, "project_id")), "path"),
            m::text(&data, "config_path")
        ),
    );
    dom::clear("system-editor-services");
    dom::clear("system-editor-shared");
    for (index, service) in m::array(&data["services"]).into_iter().enumerate() {
        let base = format!("/services/{index}/passport");
        let p = &service["passport"];
        let path = m::text(&service, "path");
        let card = dom::el("section", "system-editor-service", "");
        let head = dom::el("div", "system-editor-service-head", "");
        let title = dom::el("div", "", "");
        dom::add(&title, &dom::el("h5", "", &service_title(p, &path)));
        dom::add(&title, &dom::el("p", "system-note", &path));
        dom::add(&head, &title);
        let actions = dom::el("div", "system-editor-service-actions", "");
        for (label, action, payload) in [
            (
                "Choose another folder",
                "editor-service-folder",
                json!(index),
            ),
            (
                "Remove service",
                "editor-row-remove",
                json!({"path":"/services","index":index}),
            ),
        ] {
            let button = dom::button(label, action, payload);
            let _ = button.set_attribute("class", "button quiet");
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
        dom::append("system-editor-services", &card);
    }
    if let Some(e) = dom::get("system-editor-shared") {
        rows(
            &e,
            "Shared sources",
            &data["shared_sources"],
            "/shared_sources",
            "sources",
        );
    }
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
                dom::text("system-editor-status", "Save did not complete. The form is retained; reload metadata before retrying if files changed.");
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
    dom::append("system-discovery-report",&dom::el("p","system-note","AI filled the forms. Verify sources, ownership and dependencies before saving. Complete dependency coverage remains unconfirmed."));
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
                    dom::text("system-editor-status","AI filled service details. Review sources and uncertainties, then Save system.");
                } else {
                    dom::text(
                        "system-discovery-status",
                        &format!(
                            "{} Your original form is retained.",
                            sentence(
                                job["error"]
                                    .as_str()
                                    .unwrap_or("AI scan ended without a proposal.")
                            )
                        ),
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
async fn agents(v: &V) {
    let launches = m::array(&v["launches"]);
    dom::hidden("system-agents-section", launches.is_empty());
    dom::hidden("system-agents-live", !m::yes(&v["active"]));
    if launches.is_empty() {
        crate::activity::reset("system-agents");
        return;
    }
    let latest = launches
        .last()
        .map(|l| m::text(l, "session_id"))
        .unwrap_or_default();
    let count = m::data("agent-launch-count").as_u64().unwrap_or(0);
    let preferred = m::data("system-agent-launch")
        .as_str()
        .map(str::to_owned)
        .unwrap_or_default();
    let sid = if count != launches.len() as u64
        || !launches.iter().any(|l| l["session_id"] == preferred)
    {
        latest.clone()
    } else {
        preferred
    };
    m::set("agent-launch-count", json!(launches.len()));
    m::set("system-agent-launch", json!(sid));
    dom::options("system-agents-launch",&launches.iter().enumerate().map(|(i,l)|json!({"id":l["session_id"],"label":format!("Launch {} · {}",i+1,m::label(l["status"].as_str().unwrap_or("")))})).collect::<Vec<_>>(),m::summary,Some(&sid));
    dom::hidden("system-agents-launch-field", launches.len() < 2);
    let current = sid == latest;
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
                format!(
                    "Works on {service}; folder access: {}.",
                    if v["access"] == "all" {
                        "every selected service folder (read; write in edit mode)"
                    } else {
                        "own service folder only"
                    }
                ),
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
    crate::activity::load("system-agents", &sid).await;
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
    _checked: bool,
) -> bool {
    if kind == "input" || kind == "change" {
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
        "system-editor-discard" => {
            if !m::yes(&m::data("editor-pending")) {
                clear_editor_notices();
                remember_discovery(None);
                m::next("discovery");
                dom::cancel_timer("discovery-poll");
                crate::activity::reset("system-discovery-agent");
                m::take("discovery-owner");
                m::next("editor-scope");
                m::take("editor");
                m::set("editor-dirty", json!(false));
                dom::hidden("system-editor", true);
                controls();
            }
        }
        "system-plan-change" | "system-new-change" => {
            if m::data("system-catalog").is_null() {
                catalog_load().await;
            }
            session::open_view("system-changes", true).await;
            dom::hidden("system-plan-form", false);
            // The form is the next step; "no changes yet" would only repeat it.
            dom::hidden("system-runs-empty", true);
        }
        "system-plan-cancel" => {
            dom::hidden("system-plan-form", true);
            dom::hidden("system-runs-empty", !dom::value("system-runs").is_empty());
        }
        "system-execute" => {
            if dom::checked("system-reviewed") {
                mutate("execute",json!({"provider":dom::value("system-provider"),"mode":dom::value("system-mode"),"access":dom::value("system-access"),"timeout":dom::num("system-timeout")})).await;
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
            let mut extra = json!({"accept_source_changes":dom::checked("system-accept-changes")});
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
    fn a_message_followed_by_another_ends_as_a_sentence() {
        assert_eq!(
            sentence("Failed to authenticate"),
            "Failed to authenticate."
        );
        assert_eq!(sentence("Scan ended. "), "Scan ended.");
    }
    #[test]
    fn lists_read_as_text() {
        assert_eq!(joined(&json!(["a", "b"])), "a, b");
        assert_eq!(joined(&json!([])), "none");
    }
}
