//! The code graph chip and panel: one switch per project, percent and time
//! left while Graphify builds, what the published graph covers, and a second
//! switch for telling agents about it.
use crate::{api, dom, model as m};
use serde_json::{json, Value as V};
use wasm_bindgen_futures::spawn_local;
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
            view.chip = "Not installed".into();
            view.note = "Graphify is not installed. Install it with “uv tool install graphifyy” or “pipx install graphifyy”, then rebuild.".into();
            view.rebuild = true;
        }
        "empty" => {
            view.chip = "Not built".into();
            view.note = "No graph yet.".into();
            view.rebuild = true;
        }
        "off" => {
            view.chip = "Off".into();
            view.note = "Builds a graph of this project's code on this machine with Graphify and draws it as a dependency map: what uses what, down to classes and methods, for the whole project or for a change.".into();
        }
        // Nothing was read yet, or no project is selected.
        _ => {}
    }
    view
}
fn render() {
    let status = m::data("graph");
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
    if let Some(bar) = dom::get("graph-progress-bar") {
        // A bar without a value is the browser's indeterminate state.
        let _ = match view.percent {
            Some(percent) => bar.set_attribute("value", &percent.to_string()),
            None => bar.remove_attribute("value"),
        };
    }
    dom::text("graph-progress-amount", &view.amount);
    dom::text("graph-progress-left", &view.left);
    dom::text("graph-detail", &view.detail);
    dom::hidden("graph-detail", view.detail.is_empty());
    // The map draws the published graph, also while a newer one is being built.
    dom::hidden("graph-view", !status["graph"].is_object());
    dom::hidden("graph-changes", !status["graph"].is_object());
    dom::hidden("graph-rebuild", !view.rebuild);
    dom::hidden("graph-delete", !view.delete);
}
fn follow() {
    if m::data("graph")["build"].is_object() {
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
    let project = dom::value("project");
    if project.is_empty() {
        m::set("graph", V::Null);
        render();
        follow();
        return;
    }
    let path = format!(
        "/api/projects/{}/graph{}",
        dom::enc(&project),
        if visit { "?check=1" } else { "" }
    );
    if let Ok((epoch, status)) = api::scoped("project-graph", &path).await {
        if !m::current("project-graph", epoch) || dom::value("project") != project {
            return;
        }
        m::set("graph", status);
        render();
        follow();
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
    let project = dom::value("project");
    if project.is_empty() {
        return;
    }
    dom::error("graph-error", "");
    let path = format!("/api/projects/{}/graph", dom::enc(&project));
    match api::post(&path, json!({ "action": action })).await {
        Ok(status) if dom::value("project") == project => {
            m::set("graph", status);
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
        assert_eq!(missing.chip, "Not installed");
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
}
