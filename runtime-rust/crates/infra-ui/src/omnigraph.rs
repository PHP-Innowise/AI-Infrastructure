use crate::{api, dom, model as m, omnigraph_canvas};
use serde_json::{json, Value as V};
use std::collections::BTreeSet;
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};

const CAMERA_MIN: f64 = 0.25;
const CAMERA_MAX: f64 = 2.6;
const ZOOM_STEP: f64 = 1.22;
const DRAG_THRESHOLD_PX: f64 = 4.0;
const PULSE_INTERVAL_MS: i32 = 3500;
const MAX_PULSE_EVENTS: usize = 80;

#[derive(Clone, Debug, PartialEq)]
pub struct OmnigraphCamera {
    pub scale: f64,
    pub pan_x: f64,
    pub pan_y: f64,
    pub plane_w: f64,
    pub plane_h: f64,
    pub fit_mode: bool,
}

impl Default for OmnigraphCamera {
    fn default() -> Self {
        Self {
            scale: 1.0,
            pan_x: 16.0,
            pan_y: 16.0,
            plane_w: 0.0,
            plane_h: 0.0,
            fit_mode: true,
        }
    }
}

pub fn zoom_at(
    scale: f64,
    pan_x: f64,
    pan_y: f64,
    anchor: (f64, f64),
    target_scale: f64,
) -> (f64, f64, f64) {
    let cur = scale.clamp(CAMERA_MIN, CAMERA_MAX);
    let next = target_scale.clamp(CAMERA_MIN, CAMERA_MAX);
    let world_x = (anchor.0 - pan_x) / cur;
    let world_y = (anchor.1 - pan_y) / cur;
    let next_pan_x = (anchor.0 - world_x * next).round();
    let next_pan_y = (anchor.1 - world_y * next).round();
    (next, next_pan_x, next_pan_y)
}

pub fn camera_for_fit(plane: (f64, f64), viewport: (f64, f64)) -> (f64, f64, f64) {
    const FIT_MARGIN_X: f64 = 36.0;
    const FIT_MARGIN_Y: f64 = 24.0;
    let pw = plane.0.max(1.0);
    let ph = plane.1.max(1.0);
    let vw = (viewport.0 - FIT_MARGIN_X * 2.0).max(160.0);
    let vh = (viewport.1 - FIT_MARGIN_Y * 2.0).max(140.0);
    let scale = (vw / pw).min(vh / ph).clamp(CAMERA_MIN, 1.15);
    let pan_x = ((viewport.0 - pw * scale) / 2.0).max(FIT_MARGIN_X).round();
    let pan_y = ((viewport.1 - ph * scale) / 2.0).max(FIT_MARGIN_Y).round();
    (scale, pan_x, pan_y)
}

pub fn camera_for_fly_to(
    node_pos: (f64, f64),
    viewport: (f64, f64),
    current_scale: f64,
) -> (f64, f64, f64) {
    let scale = current_scale.clamp(0.75, CAMERA_MAX);
    let pan_x = (viewport.0 / 2.0 - node_pos.0 * scale).round();
    let pan_y = (viewport.1 / 2.0 - node_pos.1 * scale).round();
    (scale, pan_x, pan_y)
}

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn parse_csv_list(raw: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for item in raw.split([',', '\n']) {
        let trimmed = item.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_owned()) {
            out.push(trimmed.to_owned());
        }
    }
    out
}

pub fn parse_relates_to(raw: &str) -> Vec<V> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for item in raw.split([',', '\n']) {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (relation, target_id) = if let Some((rel, target)) = trimmed.split_once(':') {
            let rel_lower = rel.trim().to_ascii_lowercase();
            let target_trimmed = target.trim();
            if target_trimmed.is_empty() {
                continue;
            }
            let relation =
                if rel_lower.is_empty() || matches!(rel_lower.as_str(), "relates_to" | "related") {
                    "supports".to_owned()
                } else {
                    rel_lower
                };
            (relation, target_trimmed)
        } else {
            ("supports".to_owned(), trimmed)
        };
        if seen.insert((relation.clone(), target_id.to_owned())) {
            out.push(json!({
                "target_id": target_id,
                "relation": relation,
            }));
        }
    }
    out
}

pub fn session_payload_from_fields(
    enabled: bool,
    branch_mode: &str,
    existing_branch: &str,
    pull_raw: &str,
    query: &str,
    auto_publish: bool,
    auto_merge: bool,
) -> Option<V> {
    if !enabled {
        return None;
    }
    let branch = match branch_mode {
        "main" => "main",
        "existing" => {
            let trimmed = existing_branch.trim();
            if trimmed.is_empty() {
                "main"
            } else {
                trimmed
            }
        }
        _ => "auto",
    };
    let pull_branches = parse_csv_list(pull_raw);
    let mut payload = json!({
        "enabled": true,
        "branch": branch,
        "query": query.trim(),
        "auto_publish": auto_publish,
        "auto_merge": auto_merge,
    });
    if !pull_branches.is_empty() {
        payload["pull_branches"] = json!(pull_branches);
    }
    Some(payload)
}

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

fn kind_glyph(kind: &str) -> &'static str {
    match kind {
        "finding" => "✦",
        "decision" => "◆",
        "warning" => "▲",
        "contract" => "⬡",
        "insight" => "◉",
        "artifact" => "▣",
        "question" => "?",
        "session" => "◎",
        "file" => "▤",
        _ => "◈",
    }
}

fn kind_title(kind: &str) -> &'static str {
    match kind {
        "finding" => "Finding",
        "decision" => "Decision",
        "warning" => "Warning",
        "contract" => "Contract",
        "insight" => "Insight",
        "artifact" => "Artifact",
        "question" => "Question",
        "session" => "Session",
        "file" => "File",
        _ => "Atom",
    }
}

fn atom_id(atom: &V) -> String {
    atom["id"]
        .as_str()
        .or_else(|| atom["atom_id"].as_str())
        .unwrap_or("")
        .to_owned()
}

fn atom_branch<'a>(atom: &'a V, fallback: &'a str) -> &'a str {
    atom["source_branch"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| atom["branch"].as_str().filter(|s| !s.is_empty()))
        .unwrap_or(fallback)
}

fn atom_session_id(atom: &V) -> &str {
    atom["source_session_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| atom["session_id"].as_str().filter(|s| !s.is_empty()))
        .or_else(|| atom["author_session_id"].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("")
}

pub fn format_timestamp_val(v: &V) -> String {
    if let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return s.to_owned();
    }
    let Some(raw) = v
        .as_i64()
        .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
        .or_else(|| v.as_f64().map(|f| f as i64))
    else {
        return String::new();
    };
    if raw <= 0 {
        return String::new();
    }
    let secs = if raw >= 1_000_000_000_000_000_000 {
        raw / 1_000_000_000
    } else if raw >= 1_000_000_000_000_000 {
        raw / 1_000_000
    } else if raw >= 1_000_000_000_000 {
        raw / 1_000
    } else {
        raw
    };
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = rem / 3_600;
    let minute = (rem % 3_600) / 60;
    let second = rem % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }
    format!("{y:04}-{m:02}-{d:02} {hour:02}:{minute:02}:{second:02} UTC")
}

fn first_timestamp_val(vals: &[&V]) -> String {
    for val in vals {
        let formatted = format_timestamp_val(val);
        if !formatted.is_empty() {
            return formatted;
        }
    }
    String::new()
}

fn session_node_id(session: &V) -> String {
    session["id"]
        .as_str()
        .or_else(|| session["session_id"].as_str())
        .unwrap_or("")
        .to_owned()
}

fn file_node_id(file: &V) -> String {
    file["id"]
        .as_str()
        .or_else(|| file["path"].as_str())
        .unwrap_or("")
        .to_owned()
}

fn edge_from(edge: &V) -> String {
    edge["from"]
        .as_str()
        .or_else(|| edge["source"].as_str())
        .or_else(|| edge["src"].as_str())
        .unwrap_or("")
        .to_owned()
}

fn edge_to(edge: &V) -> String {
    edge["to"]
        .as_str()
        .or_else(|| edge["target"].as_str())
        .or_else(|| edge["dst"].as_str())
        .unwrap_or("")
        .to_owned()
}

fn edge_rel(edge: &V) -> String {
    edge["kind"]
        .as_str()
        .or_else(|| edge["relation"].as_str())
        .or_else(|| edge["rel"].as_str())
        .or_else(|| edge["edge_type"].as_str())
        .unwrap_or("relates_to")
        .to_owned()
}

fn branch_names_from_graph(graph: &V) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for item in m::array(&graph["branches"]) {
        let name = item
            .as_str()
            .or_else(|| item["name"].as_str())
            .unwrap_or("")
            .trim();
        if !name.is_empty() && seen.insert(name.to_owned()) {
            out.push(name.to_owned());
        }
    }
    if seen.insert("main".to_owned()) {
        out.insert(0, "main".to_owned());
    }
    out
}

pub fn format_context_snippet(atom: &V) -> String {
    let kind = kind_title(atom["kind"].as_str().unwrap_or("finding"));
    let title = atom["title"].as_str().unwrap_or("Context Atom");
    let branch = atom_branch(atom, "main");
    let summary = atom["summary"].as_str().unwrap_or("");
    let body = atom["body"].as_str().unwrap_or("");
    let files = m::array(&atom["files"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let mut lines = vec![format!("[{kind}] {title} (branch: {branch})")];
    if !summary.is_empty() {
        lines.push(summary.to_owned());
    }
    if !body.is_empty() && body != summary {
        lines.push(body.to_owned());
    }
    if !files.is_empty() {
        lines.push(format!("Files: {}", files.join(", ")));
    }
    lines.join("\n")
}

pub fn render_search_hits_html(search_res: &V) -> String {
    let hits = m::array(&search_res["hits"]);
    if hits.is_empty() {
        let q = search_res["query"].as_str().unwrap_or("");
        if q.is_empty() {
            return String::new();
        }
        return format!(
            "<p class=\"knowledge-note\">No ranked Omnigraph hits for <code>{}</code>.</p>",
            esc(q)
        );
    }
    let mut html = format!(
        "<div class=\"omni-inspector-group\"><h4>Search hits ({})</h4>",
        hits.len()
    );
    for hit in hits.iter().take(12) {
        let atom = &hit["atom"];
        let id = atom_id(atom);
        let kind = atom["kind"].as_str().unwrap_or("finding");
        let title = atom["title"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(&id);
        let summary = atom["summary"].as_str().unwrap_or("");
        let score = hit["score"].as_f64().unwrap_or(0.0);
        let via = hit["matched_via"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("bm25");
        let neighbors = m::array(&hit["neighbors"]).len();
        html.push_str(&format!(
            "<button type=\"button\" class=\"omni-search-hit\" data-action=\"omnigraph-focus-hit\" data-node-id=\"{}\"><div class=\"omni-search-hit-head\"><span class=\"omni-badge\" data-kind=\"{}\"><span aria-hidden=\"true\">{}</span> {}</span><span class=\"omni-meta-pill\">{} · {:.2}{}</span></div><strong>{}</strong>{}</button>",
            esc(&id),
            esc(kind),
            kind_glyph(kind),
            kind_title(kind),
            esc(via),
            score,
            if neighbors > 0 {
                format!(" · {neighbors} neighbors")
            } else {
                String::new()
            },
            esc(title),
            if summary.is_empty() {
                String::new()
            } else {
                format!("<span class=\"omni-node-summary\">{}</span>", esc(summary))
            }
        ));
    }
    html.push_str("</div>");
    html
}

pub fn render_node_inspector_html(
    graph: &V,
    selected_id: &str,
    sessions: &[V],
    active_branch: &str,
) -> String {
    if selected_id.is_empty() {
        return "<p class=\"empty-note\">Select a node, commit, or search hit to inspect its provenance, files, relations, and share controls.</p>".into();
    }
    if let Some(atom) = m::array(&graph["atoms"])
        .into_iter()
        .find(|a| atom_id(a) == selected_id)
    {
        return render_atom_inspector_html(graph, &atom, sessions, active_branch);
    }
    if let Some(session_node) = m::array(&graph["sessions"])
        .into_iter()
        .find(|s| session_node_id(s) == selected_id)
    {
        return render_session_node_inspector_html(graph, &session_node);
    }
    if let Some(file_node) = m::array(&graph["files"])
        .into_iter()
        .find(|f| file_node_id(f) == selected_id)
    {
        return render_file_node_inspector_html(graph, &file_node);
    }
    format!(
        "<div class=\"omni-inspector-card\"><p class=\"knowledge-note\">Node <code>{}</code> is not present in the current filter or snapshot.</p></div>",
        esc(selected_id)
    )
}

fn render_atom_inspector_html(graph: &V, atom: &V, sessions: &[V], active_branch: &str) -> String {
    let id = atom_id(atom);
    let kind = atom["kind"].as_str().unwrap_or("finding");
    let title = atom["title"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(&id);
    let summary = atom["summary"].as_str().unwrap_or("");
    let body = atom["body"].as_str().unwrap_or("");
    let branch = atom_branch(atom, active_branch);
    let confidence = atom["confidence"].as_f64().unwrap_or(0.85).clamp(0.0, 1.0);
    let pin = atom["pin_priority"].as_i64().unwrap_or(0);
    let actor = atom["actor_id"]
        .as_str()
        .or_else(|| atom["author"].as_str())
        .unwrap_or("");
    let session_id = atom_session_id(atom);
    let commit_id = atom["commit_id"].as_str().unwrap_or("");
    let created_at = first_timestamp_val(&[
        &atom["created_at"],
        &atom["authored_at"],
        &atom["timestamp"],
    ]);

    let mut html = format!(
        "<article class=\"omni-inspector-card\"><div class=\"omni-node-head\"><span class=\"omni-badge\" data-kind=\"{}\"><span aria-hidden=\"true\">{}</span> {}</span><span class=\"omni-badge\" data-kind=\"branch\">{}</span></div><h3 class=\"omni-inspector-title\">{}</h3>",
        esc(kind),
        kind_glyph(kind),
        kind_title(kind),
        esc(branch),
        esc(title)
    );
    if !summary.is_empty() {
        html.push_str(&format!(
            "<p class=\"omni-inspector-text\">{}</p>",
            esc(summary)
        ));
    }
    if !body.is_empty() && body != summary {
        html.push_str(&format!(
            "<p class=\"omni-inspector-text\">{}</p>",
            esc(body)
        ));
    }
    html.push_str(&format!(
        "<div class=\"omni-inspector-group\"><h4>Confidence &amp; Priority</h4><div class=\"omni-node-meta\"><span>{:.0}% confidence</span>{}</div><div class=\"omni-confidence-track\" role=\"progressbar\" aria-valuenow=\"{}\" aria-valuemin=\"0\" aria-valuemax=\"100\"><div class=\"omni-confidence-fill\" style=\"--omni-confidence:{:.2}\"></div></div></div>",
        confidence * 100.0,
        if pin > 0 {
            format!("<span class=\"omni-meta-pill\">Pin {pin}</span>")
        } else {
            String::new()
        },
        (confidence * 100.0).round() as i64,
        confidence
    ));

    let tags = m::array(&atom["tags"]);
    if !tags.is_empty() {
        html.push_str(
            "<div class=\"omni-inspector-group\"><h4>Tags</h4><div class=\"omni-chip-list\">",
        );
        for tag in tags {
            if let Some(t) = tag.as_str().filter(|s| !s.is_empty()) {
                html.push_str(&format!(
                    "<span class=\"omni-meta-pill\">#{}</span>",
                    esc(t)
                ));
            }
        }
        html.push_str("</div></div>");
    }

    let files = m::array(&atom["files"]);
    if !files.is_empty() {
        html.push_str("<div class=\"omni-inspector-group\"><h4>Linked files</h4><div class=\"omni-chip-list\">");
        for file in files {
            if let Some(f) = file.as_str().filter(|s| !s.is_empty()) {
                html.push_str(&format!(
                    "<button type=\"button\" class=\"omni-session-atom-chip omni-mono\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\">▤ {}</button>",
                    esc(f),
                    esc(f)
                ));
            }
        }
        html.push_str("</div></div>");
    }

    let mut provenance_parts = Vec::new();
    if !actor.is_empty() {
        provenance_parts.push(format!("Actor: <code>{}</code>", esc(actor)));
    }
    if !created_at.is_empty() {
        provenance_parts.push(format!("Created: {}", esc(&created_at)));
    }
    if !provenance_parts.is_empty() || !session_id.is_empty() || !commit_id.is_empty() {
        html.push_str("<div class=\"omni-inspector-group\"><h4>Provenance</h4>");
        if !provenance_parts.is_empty() {
            html.push_str(&format!(
                "<p class=\"knowledge-note\">{}</p>",
                provenance_parts.join(" · ")
            ));
        }
        html.push_str("<div class=\"omni-chip-list\">");
        if !session_id.is_empty() {
            html.push_str(&format!(
                "<button type=\"button\" class=\"button compact\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\">Session: {}</button>",
                esc(session_id),
                esc(session_id)
            ));
        }
        if !commit_id.is_empty() {
            html.push_str(&format!(
                "<button type=\"button\" class=\"button compact\" data-action=\"omnigraph-select-commit\" data-commit-id=\"{}\" data-branch=\"{}\">Commit: {}</button>",
                esc(commit_id),
                esc(branch),
                esc(&commit_id.chars().take(10).collect::<String>())
            ));
        }
        html.push_str("</div></div>");
    }

    let related: Vec<(String, String, String)> = m::array(&graph["edges"])
        .into_iter()
        .filter_map(|e| {
            let f = edge_from(&e);
            let t = edge_to(&e);
            let rel = edge_rel(&e);
            if f == id && !t.is_empty() {
                Some(("→".into(), rel, t))
            } else if t == id && !f.is_empty() {
                Some(("←".into(), rel, f))
            } else {
                None
            }
        })
        .collect();
    if !related.is_empty() {
        html.push_str(
            "<div class=\"omni-inspector-group\"><h4>Relations</h4><div class=\"omni-chip-list\">",
        );
        for (dir, rel, other) in related.iter().take(16) {
            html.push_str(&format!(
                "<button type=\"button\" class=\"omni-session-atom-chip\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\">{dir} {} · {}</button>",
                esc(other),
                esc(rel),
                esc(other)
            ));
        }
        html.push_str("</div></div>");
    }

    let mut target_sessions: Vec<(String, String)> = Vec::new();
    let mut seen_sessions = BTreeSet::new();
    for s in sessions {
        let sid = m::text(s, "id");
        if !sid.is_empty() && seen_sessions.insert(sid.clone()) {
            let label = m::summary(s);
            target_sessions.push((sid.clone(), if label.is_empty() { sid } else { label }));
        }
    }
    for s in m::array(&graph["sessions"]) {
        let sid = session_node_id(&s);
        if !sid.is_empty() && seen_sessions.insert(sid.clone()) {
            let label = m::summary(&s);
            target_sessions.push((sid.clone(), if label.is_empty() { sid } else { label }));
        }
    }

    html.push_str("<div class=\"omni-action-box\"><div class=\"omni-share-row\"><div class=\"field\"><label for=\"omni-share-session\">Share to session</label><select id=\"omni-share-session\">");
    if target_sessions.is_empty() {
        html.push_str("<option value=\"\">No target sessions available</option>");
    } else {
        for (sid, label) in &target_sessions {
            html.push_str(&format!(
                "<option value=\"{}\">{}</option>",
                esc(sid),
                esc(label)
            ));
        }
    }
    html.push_str("</select></div><div class=\"field\"><label for=\"omni-share-mode\">Mode</label><select id=\"omni-share-mode\"><option value=\"inject\">Inject</option><option value=\"reference\">Reference</option><option value=\"inject_and_link\">Inject &amp; Link</option><option value=\"copy\">Copy to Branch</option></select></div></div>");
    html.push_str(&format!(
        "<div class=\"omni-chip-list\"><button type=\"button\" class=\"button compact primary\" data-action=\"omnigraph-share-atom\" data-atom-id=\"{}\" data-branch=\"{}\"{}>Share to Session</button><button type=\"button\" class=\"button compact\" data-action=\"omnigraph-copy-snippet\" data-atom-id=\"{}\">Copy context snippet</button><button type=\"button\" class=\"button compact\" data-action=\"omnigraph-open-branch\" data-branch=\"{}\">Open in Branch</button><button type=\"button\" class=\"button compact\" data-action=\"omnigraph-cite-atom\" data-atom-id=\"{}\">Cite in new atom</button></div><p class=\"knowledge-note\" id=\"omni-share-status\" role=\"status\" hidden></p><p class=\"error-text\" id=\"omni-share-error\" role=\"alert\" hidden></p></div></article>",
        esc(&id),
        esc(branch),
        if target_sessions.is_empty() {
            " disabled"
        } else {
            ""
        },
        esc(&id),
        esc(branch),
        esc(&id)
    ));
    html
}

fn render_session_node_inspector_html(graph: &V, session: &V) -> String {
    let sid = session_node_id(session);
    let title = m::summary(session);
    let status = m::text(session, "status");
    let branch = session["branch"].as_str().unwrap_or("main");
    let mut html = format!(
        "<article class=\"omni-inspector-card\"><div class=\"omni-node-head\"><span class=\"omni-badge\" data-kind=\"session\"><span aria-hidden=\"true\">◎</span> Session</span><span class=\"omni-badge\" data-kind=\"branch\">{}</span></div><h3 class=\"omni-inspector-title\">{}</h3><p class=\"knowledge-note\">ID: <code>{}</code>{}</p>",
        esc(branch),
        esc(if title.is_empty() { &sid } else { &title }),
        esc(&sid),
        if status.is_empty() {
            String::new()
        } else {
            format!(" · Status: {}", esc(&status))
        }
    );
    let linked: Vec<(String, String)> = m::array(&graph["edges"])
        .into_iter()
        .filter_map(|e| {
            let f = edge_from(&e);
            let t = edge_to(&e);
            let rel = edge_rel(&e);
            if f == sid {
                Some((rel, t))
            } else if t == sid {
                Some((rel, f))
            } else {
                None
            }
        })
        .collect();
    if !linked.is_empty() {
        html.push_str("<div class=\"omni-inspector-group\"><h4>Connected context</h4><div class=\"omni-chip-list\">");
        for (rel, other) in linked.iter().take(16) {
            html.push_str(&format!(
                "<button type=\"button\" class=\"omni-session-atom-chip\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\">{} · {}</button>",
                esc(other),
                esc(rel),
                esc(other)
            ));
        }
        html.push_str("</div></div>");
    }
    html.push_str(&format!(
        "<div class=\"omni-chip-list\"><button type=\"button\" class=\"button compact primary\" data-action=\"session-open\" data-payload=\"{}\">Open Session</button><button type=\"button\" class=\"button compact\" data-action=\"omnigraph-open-branch\" data-branch=\"{}\">Focus Branch</button></div></article>",
        esc(&json!(sid).to_string()),
        esc(branch)
    ));
    html
}

fn render_file_node_inspector_html(graph: &V, file: &V) -> String {
    let path = file["path"]
        .as_str()
        .or_else(|| file["id"].as_str())
        .unwrap_or("");
    let mut html = format!(
        "<article class=\"omni-inspector-card\"><div class=\"omni-node-head\"><span class=\"omni-badge\" data-kind=\"file\"><span aria-hidden=\"true\">▤</span> File Anchor</span></div><h3 class=\"omni-inspector-title omni-mono\">{}</h3>",
        esc(path)
    );
    let touching: Vec<String> = m::array(&graph["edges"])
        .into_iter()
        .filter_map(|e| {
            let f = edge_from(&e);
            let t = edge_to(&e);
            if t == path {
                Some(f)
            } else if f == path {
                Some(t)
            } else {
                None
            }
        })
        .collect();
    if !touching.is_empty() {
        html.push_str("<div class=\"omni-inspector-group\"><h4>Referencing atoms</h4><div class=\"omni-chip-list\">");
        for node_id in touching.iter().take(16) {
            html.push_str(&format!(
                "<button type=\"button\" class=\"omni-session-atom-chip\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\">{}</button>",
                esc(node_id),
                esc(node_id)
            ));
        }
        html.push_str("</div></div>");
    }
    html.push_str(&format!(
        "<div class=\"omni-chip-list\"><button type=\"button\" class=\"button compact\" data-action=\"inspect-file-graph\" data-payload=\"{}\">Inspect in Code Graph</button></div></article>",
        esc(&json!({ "path": path }).to_string())
    ));
    html
}

pub fn render_commit_inspector_html(detail: &V) -> String {
    let commit = &detail["commit"];
    let cid = commit["id"]
        .as_str()
        .or_else(|| commit["graph_commit_id"].as_str())
        .or_else(|| commit["commit_id"].as_str())
        .unwrap_or("commit");
    let branch = commit["branch"]
        .as_str()
        .or_else(|| commit["graph_branch"].as_str())
        .or_else(|| commit["source_branch"].as_str())
        .unwrap_or("main");
    let actor = commit["actor_id"]
        .as_str()
        .or_else(|| commit["actor"].as_str())
        .or_else(|| commit["author"].as_str())
        .unwrap_or("harness");
    let created = first_timestamp_val(&[
        &commit["created_at"],
        &commit["authored_at"],
        &commit["timestamp"],
    ]);
    let changes = m::array(&detail["changes"]);
    let atoms = m::array(&detail["atoms"]);
    let mut html = format!(
        "<article class=\"omni-inspector-card\"><div class=\"omni-node-head\"><span class=\"omni-badge\" data-kind=\"decision\"><span aria-hidden=\"true\">⏱</span> Snapshot Commit</span><span class=\"omni-badge\" data-kind=\"branch\">{}</span></div><h3 class=\"omni-inspector-title omni-mono\">{}</h3><p class=\"knowledge-note\">Actor: <code>{}</code>{}</p>",
        esc(branch),
        esc(cid),
        esc(actor),
        if created.is_empty() {
            String::new()
        } else {
            format!(" · {}", esc(&created))
        }
    );
    if !changes.is_empty() {
        html.push_str(&format!(
            "<div class=\"omni-inspector-group\"><h4>Changes ({})</h4><div class=\"omni-chip-list\">",
            changes.len()
        ));
        for ch in changes.iter().take(12) {
            let op = ch["op"]
                .as_str()
                .or_else(|| ch["kind"].as_str())
                .unwrap_or("change");
            let type_name = ch["type"]["name"]
                .as_str()
                .or_else(|| ch["type_name"].as_str())
                .or_else(|| ch["entity_kind"].as_str())
                .unwrap_or("");
            let label = ch["title"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    ch["after"]["properties"]["title"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                })
                .or_else(|| {
                    ch["after"]["properties"]["summary"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                })
                .or_else(|| ch["after"]["title"].as_str().filter(|s| !s.is_empty()))
                .or_else(|| ch["after"]["summary"].as_str().filter(|s| !s.is_empty()))
                .or_else(|| {
                    ch["before"]["properties"]["title"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                })
                .or_else(|| ch["entity_id"].as_str().filter(|s| !s.is_empty()))
                .or_else(|| ch["id"].as_str().filter(|s| !s.is_empty()))
                .unwrap_or("item");
            let tag = if !type_name.is_empty() && type_name != op {
                format!("{op} {type_name}")
            } else {
                op.to_owned()
            };
            html.push_str(&format!(
                "<span class=\"omni-meta-pill\">{} · {}</span>",
                esc(&tag),
                esc(label)
            ));
        }
        html.push_str("</div></div>");
    }
    if !atoms.is_empty() {
        html.push_str(&format!(
            "<div class=\"omni-inspector-group\"><h4>Snapshot atoms ({})</h4><div class=\"omni-chip-list\">",
            atoms.len()
        ));
        for atom in atoms.iter().take(16) {
            let id = atom_id(atom);
            let title = atom["title"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(&id);
            html.push_str(&format!(
                "<button type=\"button\" class=\"omni-session-atom-chip\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\">{}</button>",
                esc(&id),
                esc(title)
            ));
        }
        html.push_str("</div></div>");
    }
    html.push_str("<div class=\"omni-chip-list\"><button type=\"button\" class=\"button compact primary\" data-action=\"omnigraph-return-live\">Return to live head</button></div></article>");
    html
}

pub fn render_pulse_list_html(events: &[V]) -> String {
    if events.is_empty() {
        return "<p class=\"empty-note\">No cross-session context events recorded yet.</p>".into();
    }
    let mut html = String::new();
    for ev in events.iter().take(MAX_PULSE_EVENTS) {
        let kind = ev["kind"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                ev["after"]["properties"]["kind"]
                    .as_str()
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| ev["type_name"].as_str().filter(|s| !s.is_empty()))
            .or_else(|| ev["entity_kind"].as_str().filter(|s| !s.is_empty()))
            .or_else(|| ev["op"].as_str().filter(|s| !s.is_empty()))
            .or_else(|| ev["event"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("change");
        let branch = ev["branch"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| ev["source_branch"].as_str().filter(|s| !s.is_empty()))
            .or_else(|| ev["graph_branch"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("main");
        let node_id = ev["atom_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| ev["node_id"].as_str().filter(|s| !s.is_empty()))
            .or_else(|| ev["entity_id"].as_str().filter(|s| !s.is_empty()))
            .or_else(|| ev["session_id"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("");
        let op = ev["op"].as_str().unwrap_or("");
        let fallback_title = if !op.is_empty() && !node_id.is_empty() {
            format!("{op} · {node_id}")
        } else if !node_id.is_empty() {
            node_id.to_owned()
        } else {
            kind.to_owned()
        };
        let title = ev["title"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| ev["summary"].as_str().filter(|s| !s.is_empty()))
            .or_else(|| {
                ev["after"]["properties"]["title"]
                    .as_str()
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| {
                ev["after"]["properties"]["summary"]
                    .as_str()
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or(&fallback_title);
        let actor = ev["actor_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| ev["source_session_id"].as_str().filter(|s| !s.is_empty()))
            .or_else(|| ev["session_id"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("system");
        let when = first_timestamp_val(&[&ev["created_at"], &ev["authored_at"], &ev["timestamp"]]);
        html.push_str(&format!(
            "<button type=\"button\" class=\"omni-pulse-item\" data-action=\"omnigraph-pulse-focus\" data-node-id=\"{}\" data-branch=\"{}\"><div class=\"omni-pulse-item-head\"><span class=\"omni-badge\" data-kind=\"{}\">{}</span><span>{}</span></div><strong>{}</strong><span class=\"knowledge-note\">{} · {}</span></button>",
            esc(node_id),
            esc(branch),
            esc(kind),
            esc(kind),
            esc(&when),
            esc(title),
            esc(branch),
            esc(actor)
        ));
    }
    html
}

pub fn session_bar_error(session: &V) -> String {
    m::text(&session["omnigraph"], "last_error")
}

pub fn session_bar_summary(session: &V) -> (bool, String, String, Vec<V>, bool) {
    if !m::omnigraph_enabled(session) {
        return (false, String::new(), String::new(), Vec::new(), false);
    }
    let omni = &session["omnigraph"];
    let branch = m::omnigraph_branch(session);
    let shared_atoms = {
        let injected = m::array(&omni["injected_atoms"]);
        if !injected.is_empty() {
            injected
        } else {
            let shared = m::array(&omni["shared_atoms"]);
            if !shared.is_empty() {
                shared
            } else {
                m::array(&omni["atoms"])
            }
        }
    };
    let published = m::array(&omni["published_atoms"]);
    let count = omni["atom_count"]
        .as_u64()
        .unwrap_or(shared_atoms.len() as u64);
    let pub_count = omni["published_count"]
        .as_u64()
        .unwrap_or(published.len() as u64);
    let mut combined = Vec::new();
    let mut seen = BTreeSet::new();
    for item in shared_atoms.into_iter().chain(published) {
        let normalized = if let Some(id_str) = item.as_str().filter(|s| !s.is_empty()) {
            json!({ "id": id_str, "title": id_str, "kind": "finding" })
        } else {
            item
        };
        let aid = atom_id(&normalized);
        if aid.is_empty() || seen.insert(aid) {
            combined.push(normalized);
        }
    }
    let mut parts = vec![format!("{count} shared atoms")];
    if pub_count > 0 {
        parts.push(format!("{pub_count} published"));
    }
    if m::yes(&omni["auto_publish"]) {
        parts.push("auto-publish".into());
    }
    if m::yes(&omni["auto_merge"]) {
        parts.push("auto-merge".into());
    }
    let merge_status = m::text(omni, "merge_status");
    if !merge_status.is_empty() {
        parts.push(format!("merge: {merge_status}"));
    }
    let last_error = m::text(omni, "last_error");
    if !last_error.is_empty() {
        parts.push(format!("error: {last_error}"));
    }
    let is_merged = matches!(
        merge_status.to_ascii_lowercase().as_str(),
        "merged" | "fast_forward"
    );
    let can_merge = !is_merged && branch != "main" && branch != "auto";
    (true, branch, parts.join(" · "), combined, can_merge)
}

fn current_project_id() -> String {
    let p = dom::value("omnigraph-project");
    if !p.is_empty() {
        return p;
    }
    dom::value("project")
}

fn ui_state() -> V {
    let v = m::data("omnigraph-ui");
    if v.is_object() {
        return v;
    }
    let init = json!({
        "stage_tab": "constellation",
        "drawer_tab": "inspector",
        "branch": "main",
        "snapshot": null,
        "kind": "",
        "query": "",
        "show_files": true,
        "show_sessions": true,
        "selected_id": null,
        "selected_commit_id": null,
        "pulse_paused": false,
        "highlighted_ids": [],
    });
    m::set("omnigraph-ui", init.clone());
    init
}

fn set_ui_state(patch: impl FnOnce(&mut V)) {
    let mut state = ui_state();
    patch(&mut state);
    m::set("omnigraph-ui", state);
}

fn load_camera() -> OmnigraphCamera {
    let raw = m::data("omnigraph-camera");
    if !raw.is_object() {
        return OmnigraphCamera::default();
    }
    OmnigraphCamera {
        scale: raw["scale"]
            .as_f64()
            .unwrap_or(1.0)
            .clamp(CAMERA_MIN, CAMERA_MAX),
        pan_x: raw["pan_x"].as_f64().unwrap_or(16.0),
        pan_y: raw["pan_y"].as_f64().unwrap_or(16.0),
        plane_w: raw["plane_w"].as_f64().unwrap_or(0.0),
        plane_h: raw["plane_h"].as_f64().unwrap_or(0.0),
        fit_mode: raw["fit_mode"].as_bool().unwrap_or(true),
    }
}

fn save_camera(cam: &OmnigraphCamera) {
    m::set(
        "omnigraph-camera",
        json!({
            "scale": cam.scale,
            "pan_x": cam.pan_x,
            "pan_y": cam.pan_y,
            "plane_w": cam.plane_w,
            "plane_h": cam.plane_h,
            "fit_mode": cam.fit_mode,
        }),
    );
}

fn viewport_size() -> (f64, f64) {
    let Some(el) =
        dom::get("omnigraph-canvas").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return (760.0, 420.0);
    };
    let w = el.client_width() as f64;
    let h = el.client_height() as f64;
    if w > 40.0 && h > 40.0 {
        (w, h)
    } else {
        (760.0, 420.0)
    }
}

fn apply_camera(cam: &OmnigraphCamera) {
    let pct = format!("{}%", (cam.scale * 100.0).round() as i64);
    dom::text("omni-zoom-level", &pct);
    let Some(canvas) = dom::get("omnigraph-canvas") else {
        return;
    };
    let Ok(Some(plane)) = canvas.query_selector(".graph-map-plane") else {
        return;
    };
    let Some(plane_el) = plane.dyn_into::<web_sys::HtmlElement>().ok() else {
        return;
    };
    let style = format!(
        "width:{:.0}px;height:{:.0}px;--graph-scale:{:.4};--graph-pan-x:{:.1}px;--graph-pan-y:{:.1}px;transform:translate3d({:.1}px,{:.1}px,0) scale({:.4});transform-origin:0 0",
        cam.plane_w.max(320.0),
        cam.plane_h.max(240.0),
        cam.scale,
        cam.pan_x,
        cam.pan_y,
        cam.pan_x,
        cam.pan_y,
        cam.scale,
    );
    let _ = plane_el.set_attribute("style", &style);
    if cam.plane_w > 0.0 && cam.plane_h > 0.0 {
        if let Some(radar_view) = dom::get("omnigraph-canvas-radar")
            .and_then(|r| r.query_selector(".omni-radar-view").ok().flatten())
        {
            let (vp_w, vp_h) = viewport_size();
            let vx = (-cam.pan_x / cam.scale).clamp(0.0, cam.plane_w);
            let vy = (-cam.pan_y / cam.scale).clamp(0.0, cam.plane_h);
            let vw = (vp_w / cam.scale).clamp(16.0, (cam.plane_w - vx).max(16.0));
            let vh = (vp_h / cam.scale).clamp(16.0, (cam.plane_h - vy).max(16.0));
            let _ = radar_view.set_attribute("x", &format!("{vx:.1}"));
            let _ = radar_view.set_attribute("y", &format!("{vy:.1}"));
            let _ = radar_view.set_attribute("width", &format!("{vw:.1}"));
            let _ = radar_view.set_attribute("height", &format!("{vh:.1}"));
        }
    }
}

fn fit_camera() {
    let mut cam = load_camera();
    let vw_vh = viewport_size();
    cam.fit_mode = true;
    if cam.plane_w > 0.0 && cam.plane_h > 0.0 {
        let (s, px, py) = camera_for_fit((cam.plane_w, cam.plane_h), vw_vh);
        cam.scale = s;
        cam.pan_x = px;
        cam.pan_y = py;
    }
    save_camera(&cam);
    apply_camera(&cam);
}

fn reset_camera_100() {
    let mut cam = load_camera();
    let (vw, vh) = viewport_size();
    cam.scale = 1.0;
    cam.pan_x = if cam.plane_w > 0.0 && cam.plane_w < vw {
        ((vw - cam.plane_w) / 2.0).round()
    } else {
        16.0
    };
    cam.pan_y = if cam.plane_h > 0.0 && cam.plane_h < vh {
        ((vh - cam.plane_h) / 2.0).round()
    } else {
        16.0
    };
    cam.fit_mode = false;
    save_camera(&cam);
    apply_camera(&cam);
}

fn zoom_canvas_at(factor: f64, client_anchor: Option<(f64, f64)>) {
    let mut cam = load_camera();
    let (vw, vh) = viewport_size();
    let anchor = if let Some((cx, cy)) = client_anchor {
        if let Some(canvas) = dom::get("omnigraph-canvas") {
            let rect = canvas.get_bounding_client_rect();
            (
                (cx - rect.left()).clamp(0.0, vw),
                (cy - rect.top()).clamp(0.0, vh),
            )
        } else {
            (vw / 2.0, vh / 2.0)
        }
    } else {
        (vw / 2.0, vh / 2.0)
    };
    let (s, px, py) = zoom_at(cam.scale, cam.pan_x, cam.pan_y, anchor, cam.scale * factor);
    cam.scale = s;
    cam.pan_x = px;
    cam.pan_y = py;
    cam.fit_mode = false;
    save_camera(&cam);
    apply_camera(&cam);
}

fn fly_to_node(node_id: &str) {
    let positions = m::data("omnigraph-positions");
    let Some(arr) = positions[node_id].as_array() else {
        return;
    };
    let (Some(nx), Some(ny)) = (
        arr.first().and_then(|v| v.as_f64()),
        arr.get(1).and_then(|v| v.as_f64()),
    ) else {
        return;
    };
    let mut cam = load_camera();
    let (s, px, py) = camera_for_fly_to((nx, ny), viewport_size(), cam.scale);
    cam.scale = s;
    cam.pan_x = px;
    cam.pan_y = py;
    cam.fit_mode = false;
    save_camera(&cam);
    apply_camera(&cam);
}

pub fn initialize() {
    let _ = ui_state();
    render_tabs();
    sync_pulse_toggle();
}

fn reset_pulse_cursor() {
    m::set("omnigraph-pulse-cursor", json!({ "value": "" }));
}

fn switch_branch_state(branch: &str, clear_snapshot: bool) -> bool {
    let target = branch.trim();
    if target.is_empty() || target == "auto" {
        return false;
    }
    let changed = m::text(&ui_state(), "branch") != target;
    set_ui_state(|s| {
        s["branch"] = json!(target);
        if clear_snapshot {
            s["snapshot"] = V::Null;
            s["selected_commit_id"] = V::Null;
        }
    });
    if changed {
        reset_pulse_cursor();
    }
    changed
}

pub fn focus_branch(branch: &str) {
    let _ = switch_branch_state(branch, true);
}

pub fn hydrate_status(status: &V) {
    if !status.is_object() {
        return;
    }
    m::set("omnigraph-status", status.clone());
    apply_status_to_chrome(status);
}

pub async fn refresh_pill() {
    let project = current_project_id();
    let path = format!(
        "/api/omnigraph/status{}",
        dom::query(&[("project_id", project)])
    );
    if let Ok(status) = api::get(&path).await {
        hydrate_status(&status);
    }
}

fn apply_status_to_chrome(status: &V) {
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
    dom::hidden("omnigraph-onboarding", connected);
    dom::hidden("omnigraph-studio", !connected);

    let installed = m::yes(&status["binary_installed"]);
    let version = m::text(status, "binary_version");
    dom::hidden("omnigraph-start-local", !installed || connected);
    dom::text(
        "omnigraph-install-start",
        if installed {
            "Reinstall Local Omnigraph v0.15.0"
        } else {
            "Install & Start Local Omnigraph v0.15.0"
        },
    );
    if !version.is_empty() {
        dom::text(
            "omnigraph-local-detail",
            &format!("Installed version {version}. Managed locally with automatic schema apply and bearer auth."),
        );
    }
    let docker_ok = m::yes(&status["docker_available"]);
    dom::disabled("omnigraph-start-docker", !docker_ok && !connected);
    dom::disabled("omni-dlg-start-docker", !docker_ok);

    let err = m::text(status, "error");
    dom::error("omnigraph-error", &err);
    dom::text(
        "omnigraph-session-note",
        if connected {
            "Omnigraph is live. Sessions can read and publish shared learnings across branches."
        } else {
            "Omnigraph is currently offline. Configure or start it in Context Graph to enable live sharing."
        },
    );
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
    m::set("omnigraph-graph", V::Null);
    m::set("omnigraph-diff", V::Null);
    m::set("omnigraph-search-results", V::Null);
    m::set("omnigraph-commit-detail", V::Null);
    m::set("omnigraph-pulse-events", json!([]));
    reset_pulse_cursor();
    set_ui_state(|s| {
        s["branch"] = json!("main");
        s["snapshot"] = V::Null;
        s["selected_id"] = V::Null;
        s["selected_commit_id"] = V::Null;
        s["highlighted_ids"] = json!([]);
    });
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

fn schedule_pulse_poll() {
    dom::cancel_timer("omnigraph-pulse");
    if dom::document().hidden()
        || m::read(|s| s.view != "omnigraph")
        || !m::yes(&m::data("omnigraph-status")["connected"])
    {
        return;
    }
    dom::later("omnigraph-pulse", PULSE_INTERVAL_MS, || {
        spawn_local(async {
            poll_changes(false).await;
            schedule_pulse_poll();
        });
    });
}

async fn refresh_graph() {
    let project = current_project_id();
    let st = ui_state();
    let branch = m::text(&st, "branch");
    let snapshot = m::text(&st, "snapshot");
    let kind = m::text(&st, "kind");
    let query = m::text(&st, "query");
    let path = format!(
        "/api/omnigraph/graph{}",
        dom::query(&[
            ("project_id", project),
            ("branch", branch),
            ("snapshot", snapshot),
            ("kind", kind),
            ("q", query),
        ])
    );
    match api::scoped("omnigraph-graph", &path).await {
        Ok((epoch, graph)) if m::current("omnigraph-graph", epoch) => {
            m::set("omnigraph-graph", graph.clone());
            sync_branch_selectors(&graph);
            sync_session_selectors(&graph);
            render_studio();
        }
        Err(e) if !e.aborted => dom::error("omnigraph-error", &e.message),
        _ => {}
    }
}

async fn run_search(query: String) {
    let project = current_project_id();
    set_ui_state(|s| {
        s["query"] = json!(query);
    });
    let trimmed = query.trim().to_owned();
    dom::hidden("omnigraph-search-clear", trimmed.is_empty());
    if trimmed.is_empty() {
        m::set("omnigraph-search-results", V::Null);
        set_ui_state(|s| {
            s["highlighted_ids"] = json!([]);
        });
        render_studio();
        return;
    }
    let st = ui_state();
    let path = format!(
        "/api/omnigraph/search{}",
        dom::query(&[
            ("project_id", project),
            ("q", trimmed),
            ("branch", m::text(&st, "branch")),
            ("kind", m::text(&st, "kind")),
            ("limit", "20".into()),
        ])
    );
    match api::scoped("omnigraph-search", &path).await {
        Ok((epoch, res)) if m::current("omnigraph-search", epoch) => {
            m::set("omnigraph-search-results", res.clone());
            let mut ids = Vec::new();
            for hit in m::array(&res["hits"]) {
                let aid = atom_id(&hit["atom"]);
                if !aid.is_empty() {
                    ids.push(aid);
                }
                for neighbor in m::array(&hit["neighbors"]) {
                    if let Some(nid) = neighbor
                        .as_str()
                        .or_else(|| neighbor["id"].as_str())
                        .filter(|s| !s.is_empty())
                    {
                        ids.push(nid.to_owned());
                    }
                }
            }
            set_ui_state(|s| {
                s["highlighted_ids"] = json!(ids);
            });
            render_studio();
        }
        Err(e) if !e.aborted => dom::error("omnigraph-error", &e.message),
        _ => {}
    }
}

async fn poll_changes(force: bool) {
    if !force && m::read(|s| s.view != "omnigraph") {
        return;
    }
    let project = current_project_id();
    let st = ui_state();
    let cursor = m::text(&m::data("omnigraph-pulse-cursor"), "value");
    let path = format!(
        "/api/omnigraph/changes{}",
        dom::query(&[
            ("project_id", project),
            ("cursor", cursor),
            ("branch", m::text(&st, "branch")),
        ])
    );
    match api::scoped("omnigraph-changes", &path).await {
        Ok((epoch, data)) if m::current("omnigraph-changes", epoch) => {
            let next_cursor = m::text(&data, "cursor");
            if !next_cursor.is_empty() {
                m::set("omnigraph-pulse-cursor", json!({ "value": next_cursor }));
            }
            let incoming = m::array(&data["events"]);
            if !incoming.is_empty() {
                let mut existing = m::array(&m::data("omnigraph-pulse-events"));
                let mut merged = incoming.clone();
                merged.append(&mut existing);
                merged.truncate(MAX_PULSE_EVENTS);
                m::set("omnigraph-pulse-events", json!(merged));
                render_pulse();
                if !force {
                    refresh_graph().await;
                    refresh_pill().await;
                }
            } else {
                render_pulse();
            }
        }
        _ => {}
    }
}

async fn load_commit_detail(commit_id: &str, branch: &str) {
    let project = current_project_id();
    let _ = switch_branch_state(branch, false);
    set_ui_state(|s| {
        s["snapshot"] = json!(commit_id);
        s["selected_commit_id"] = json!(commit_id);
        s["drawer_tab"] = json!("inspector");
    });
    render_tabs();
    sync_controls_from_state();
    refresh_graph().await;
    let path = format!(
        "/api/omnigraph/commits/{}{}",
        dom::enc(commit_id),
        dom::query(&[("project_id", project)])
    );
    match api::scoped("omnigraph-commit", &path).await {
        Ok((epoch, detail)) if m::current("omnigraph-commit", epoch) => {
            m::set("omnigraph-commit-detail", detail);
            render_inspector();
        }
        Err(e) if !e.aborted => dom::error("omnigraph-error", &e.message),
        _ => {}
    }
}

async fn preview_merge(source: &str, target: &str) {
    if source.is_empty() || target.is_empty() {
        dom::error("omnigraph-error", "Select both source and target branches.");
        return;
    }
    dom::error("omnigraph-error", "");
    let project = current_project_id();
    let path = format!(
        "/api/omnigraph/branches/diff{}",
        dom::query(&[
            ("project_id", project),
            ("source", source.into()),
            ("target", target.into()),
        ])
    );
    match api::scoped("omnigraph-diff", &path).await {
        Ok((epoch, diff)) if m::current("omnigraph-diff", epoch) => {
            m::set("omnigraph-diff", diff.clone());
            render_merge();
        }
        Err(e) if !e.aborted => dom::error("omnigraph-error", &e.message),
        _ => {}
    }
}

pub fn branch_merge_outcome(res: &V) -> Result<(), String> {
    let outcome = res["outcome"].as_str().unwrap_or("");
    let ok = res["ok"].as_bool();
    if outcome == "conflict" || ok == Some(false) {
        let conflicts = m::array(&res["conflicts"]);
        let msg = res["error"]
            .as_str()
            .or_else(|| res["message"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if !conflicts.is_empty() {
                    format!(
                        "Merge conflict across {} item(s); review the diff below.",
                        conflicts.len()
                    )
                } else if outcome == "conflict" {
                    "Merge conflict detected; resolve conflicts before merging.".to_owned()
                } else {
                    "Branch operation failed.".to_owned()
                }
            });
        Err(msg)
    } else {
        Ok(())
    }
}

async fn execute_branch_action(body: V, status_msg: &str) -> Result<V, api::Error> {
    dom::error("omnigraph-error", "");
    let res = api::post("/api/omnigraph/branches", body).await?;
    if branch_merge_outcome(&res).is_ok() {
        dom::text("omnigraph-status-message", status_msg);
        dom::hidden("omnigraph-status-message", false);
    } else {
        dom::hidden("omnigraph-status-message", true);
    }
    refresh_pill().await;
    refresh_graph().await;
    Ok(res)
}

async fn run_setup_action(action: &str, extra: Option<V>, status_target: &str, error_target: &str) {
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

fn render_tabs() {
    let st = ui_state();
    let stage = st["stage_tab"].as_str().unwrap_or("constellation");
    for tab in ["constellation", "dag", "merge"] {
        let active = stage == tab;
        dom::attr(
            &format!("omni-tab-{tab}"),
            "aria-selected",
            if active { "true" } else { "false" },
        );
        dom::hidden(&format!("omni-pane-{tab}"), !active);
    }
    dom::hidden("omni-canvas-tools", stage != "constellation");

    let drawer = st["drawer_tab"].as_str().unwrap_or("inspector");
    for tab in ["inspector", "pulse", "publish"] {
        let active = drawer == tab;
        dom::attr(
            &format!("omni-drawer-tab-{tab}"),
            "aria-selected",
            if active { "true" } else { "false" },
        );
        dom::hidden(&format!("omni-drawer-{tab}"), !active);
    }
}

fn sync_pulse_toggle() {
    let paused = m::yes(&ui_state()["pulse_paused"]);
    dom::attr(
        "omnigraph-view",
        "data-pulse-paused",
        if paused { "true" } else { "false" },
    );
    dom::text(
        "omni-toggle-pulse",
        if paused { "Play pulse" } else { "Pause pulse" },
    );
}

fn sync_controls_from_state() {
    let st = ui_state();
    let kind = m::text(&st, "kind");
    for el in dom::all("#omnigraph-kind-chips .omni-kind-chip") {
        let k = el.get_attribute("data-kind").unwrap_or_default();
        let _ = el.set_attribute("aria-pressed", if k == kind { "true" } else { "false" });
    }
    let branch = m::text(&st, "branch");
    if !branch.is_empty() {
        dom::set_value("omnigraph-branch", &branch);
        dom::hidden("omni-delete-branch", branch == "main");
    }
    dom::set_checked(
        "omnigraph-show-sessions",
        st["show_sessions"].as_bool().unwrap_or(true),
    );
    dom::set_checked(
        "omnigraph-show-files",
        st["show_files"].as_bool().unwrap_or(true),
    );
    let snapshot = m::text(&st, "snapshot");
    dom::hidden("omnigraph-timetravel-banner", snapshot.is_empty());
    if !snapshot.is_empty() {
        dom::text(
            "omnigraph-timetravel-text",
            &format!("Viewing commit {snapshot} on branch {branch}"),
        );
    }
}

fn sync_branch_selectors(graph: &V) {
    let branches = branch_names_from_graph(graph);
    let st = ui_state();
    let active_branch = m::text(&st, "branch");
    let items: Vec<V> = branches
        .iter()
        .map(|name| json!({ "id": name, "name": name }))
        .collect();
    dom::options("omnigraph-branch", &items, m::summary, Some(&active_branch));
    dom::options(
        "omni-new-branch-from",
        &items,
        m::summary,
        Some(&active_branch),
    );
    dom::options("omni-pub-branch", &items, m::summary, Some(&active_branch));
    dom::options("omnigraph-existing-branch", &items, m::summary, None);

    let non_main: Vec<V> = branches
        .iter()
        .filter(|name| name.as_str() != "main")
        .map(|name| json!({ "id": name, "name": name }))
        .collect();
    let merge_src = if non_main.is_empty() {
        &items
    } else {
        &non_main
    };
    let prev_src = dom::value("omni-merge-source");
    dom::options(
        "omni-merge-source",
        merge_src,
        m::summary,
        if prev_src.is_empty() && active_branch != "main" {
            Some(&active_branch)
        } else {
            None
        },
    );
    dom::options("omni-merge-target", &items, m::summary, Some("main"));
    dom::hidden("omni-delete-branch", active_branch == "main");
}

fn sync_session_selectors(graph: &V) {
    let mut items = vec![json!({ "id": "", "name": "No specific session" })];
    let mut seen = BTreeSet::new();
    for s in m::read(|s| s.sessions.clone()) {
        let id = m::text(&s, "id");
        if !id.is_empty() && seen.insert(id.clone()) {
            let label = m::summary(&s);
            items.push(json!({ "id": id, "name": label }));
        }
    }
    for s in m::array(&graph["sessions"]) {
        let id = session_node_id(&s);
        if !id.is_empty() && seen.insert(id.clone()) {
            let label = m::summary(&s);
            items.push(json!({ "id": id, "name": label }));
        }
    }
    dom::options("omni-pub-session", &items, m::summary, None);
}

fn render_studio() {
    render_tabs();
    sync_controls_from_state();
    render_constellation();
    render_dag();
    render_merge();
    render_inspector();
    render_pulse();
}

fn render_constellation() {
    let graph = m::data("omnigraph-graph");
    if !graph.is_object() {
        return;
    }
    let st = ui_state();
    let highlighted: BTreeSet<String> = m::array(&st["highlighted_ids"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let filter = omnigraph_canvas::ConstellationFilter {
        query: m::text(&st, "query"),
        kind: m::text(&st, "kind"),
        branch: m::text(&st, "branch"),
        show_files: st["show_files"].as_bool().unwrap_or(true),
        show_sessions: st["show_sessions"].as_bool().unwrap_or(true),
        selected_id: st["selected_id"].as_str().map(str::to_owned),
        highlighted_ids: highlighted,
    };
    let mut cam = load_camera();
    let viewport = omnigraph_canvas::CanvasViewport {
        pan_x: cam.pan_x,
        pan_y: cam.pan_y,
        scale: cam.scale,
    };
    let rendered = omnigraph_canvas::render_constellation(&graph, &filter, &viewport);
    if let Some(el) = dom::get("omnigraph-canvas") {
        el.set_inner_html(&rendered.html);
    }
    if let Some(radar) = dom::get("omnigraph-canvas-radar") {
        radar.set_inner_html(&rendered.radar_html);
        dom::hide(&radar, rendered.radar_html.trim().is_empty());
    }
    let pos_map: V = rendered
        .positions
        .into_iter()
        .map(|(k, (x, y))| (k, json!([x, y])))
        .collect::<serde_json::Map<String, V>>()
        .into();
    m::set("omnigraph-positions", pos_map);
    cam.plane_w = rendered.width;
    cam.plane_h = rendered.height;
    save_camera(&cam);
    if cam.fit_mode {
        fit_camera();
    } else {
        apply_camera(&cam);
    }
}

fn render_dag() {
    let graph = m::data("omnigraph-graph");
    if !graph.is_object() {
        return;
    }
    let st = ui_state();
    let branch = m::text(&st, "branch");
    let snapshot = st["snapshot"].as_str();
    let html = omnigraph_canvas::render_branch_dag(&graph, &branch, snapshot);
    if let Some(el) = dom::get("omnigraph-dag-container") {
        el.set_inner_html(&html);
    }
}

fn render_merge() {
    let diff = m::data("omnigraph-diff");
    let Some(el) = dom::get("omnigraph-merge-container") else {
        return;
    };
    if !diff.is_object() {
        el.set_inner_html("<p class=\"empty-note\">Choose a source and target branch above, then click <strong>Preview Merge</strong> to inspect additions, updates, deletions, and 3-way merge conflicts.</p>");
        return;
    }
    el.set_inner_html(&omnigraph_canvas::render_merge_diff(&diff));
}

fn render_inspector() {
    let search_res = m::data("omnigraph-search-results");
    let st = ui_state();
    let query = m::text(&st, "query");
    if let Some(hits_el) = dom::get("omnigraph-search-hits") {
        if query.trim().is_empty() || !search_res.is_object() {
            dom::hide(&hits_el, true);
            hits_el.set_inner_html("");
        } else {
            hits_el.set_inner_html(&render_search_hits_html(&search_res));
            dom::hide(&hits_el, false);
        }
    }
    let Some(body) = dom::get("omnigraph-inspector-body") else {
        return;
    };
    let snapshot = m::text(&st, "snapshot");
    let selected_id = m::text(&st, "selected_id");
    let commit_detail = m::data("omnigraph-commit-detail");
    if selected_id.is_empty() && !snapshot.is_empty() && commit_detail.is_object() {
        body.set_inner_html(&render_commit_inspector_html(&commit_detail));
        return;
    }
    let graph = m::data("omnigraph-graph");
    let sessions = m::read(|s| s.sessions.clone());
    let branch = m::text(&st, "branch");
    body.set_inner_html(&render_node_inspector_html(
        &graph,
        &selected_id,
        &sessions,
        &branch,
    ));
}

fn render_pulse() {
    let events = m::array(&m::data("omnigraph-pulse-events"));
    dom::text("omni-pulse-count", &events.len().to_string());
    if let Some(el) = dom::get("omnigraph-pulse-list") {
        el.set_inner_html(&render_pulse_list_html(&events));
    }
}

pub fn session_controls() {
    let sid = m::read(|s| s.sid.clone());
    let locked = m::read(|s| s.pending || m::active(&s.selected));
    let enabled = dom::checked("omnigraph-enabled");
    let mode = dom::value("omnigraph-branch-mode");
    dom::hidden("omnigraph-session-fields", !enabled);
    dom::hidden("omnigraph-session-checks", !enabled);
    dom::hidden(
        "omnigraph-existing-branch-field",
        !enabled || mode != "existing",
    );

    let disable_edit = locked || !sid.is_empty();
    for id in [
        "omnigraph-enabled",
        "omnigraph-branch-mode",
        "omnigraph-existing-branch",
        "omnigraph-pull-branches",
        "omnigraph-session-query",
        "omnigraph-auto-publish",
        "omnigraph-auto-merge",
    ] {
        dom::disabled(id, disable_edit);
    }
    dom::disabled(
        "omnigraph-existing-branch",
        disable_edit || mode != "existing",
    );

    let summary = if !enabled {
        "Off".to_owned()
    } else if mode == "main" {
        "main".to_owned()
    } else if mode == "existing" {
        let b = dom::value("omnigraph-existing-branch");
        if b.is_empty() {
            "Existing".to_owned()
        } else {
            b
        }
    } else {
        "Auto branch".to_owned()
    };
    dom::text("option-omnigraph-value", &summary);
    dom::attr(
        "option-omnigraph",
        "data-on",
        if enabled { "true" } else { "false" },
    );
}

pub fn session_payload() -> Option<V> {
    session_payload_from_fields(
        dom::checked("omnigraph-enabled"),
        &dom::value("omnigraph-branch-mode"),
        &dom::value("omnigraph-existing-branch"),
        &dom::value("omnigraph-pull-branches"),
        &dom::value("omnigraph-session-query"),
        dom::checked("omnigraph-auto-publish"),
        dom::checked("omnigraph-auto-merge"),
    )
}

pub async fn load_session_branches() {
    if !m::yes(&m::data("omnigraph-status")["connected"]) {
        refresh_pill().await;
    }
    if !m::yes(&m::data("omnigraph-status")["connected"]) {
        return;
    }
    let project = current_project_id();
    let path = format!(
        "/api/omnigraph/graph{}",
        dom::query(&[("project_id", project)])
    );
    if let Ok(graph) = api::get(&path).await {
        let items: Vec<V> = branch_names_from_graph(&graph)
            .into_iter()
            .map(|name| json!({ "id": name, "name": name }))
            .collect();
        dom::options("omnigraph-existing-branch", &items, m::summary, None);
        session_controls();
    }
}

pub fn render_session_bar(session: &V) {
    let (visible, branch, meta, atoms, can_merge) = session_bar_summary(session);
    dom::hidden("session-omnigraph-bar", !visible);
    if !visible {
        dom::error("session-omnigraph-error", "");
        return;
    }
    dom::text("session-omnigraph-branch", &branch);
    dom::text("session-omnigraph-meta", &meta);
    dom::error("session-omnigraph-error", &session_bar_error(session));
    dom::hidden("session-omnigraph-merge", !can_merge);
    dom::disabled("session-omnigraph-merge", m::active(session));
    if let Some(host) = dom::get("session-omnigraph-atoms") {
        if atoms.is_empty() {
            host.set_inner_html(
                "<span class=\"knowledge-note\">No shared atoms injected yet.</span>",
            );
        } else {
            let mut html = String::new();
            for atom in atoms.iter().take(10) {
                let id = atom_id(atom);
                let kind = atom["kind"].as_str().unwrap_or("finding");
                let title = atom["title"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&id);
                html.push_str(&format!(
                    "<button type=\"button\" class=\"omni-session-atom-chip\" data-action=\"omnigraph-open-atom\" data-atom-id=\"{}\" data-branch=\"{}\"><span aria-hidden=\"true\">{}</span> {}</button>",
                    esc(&id),
                    esc(&branch),
                    kind_glyph(kind),
                    esc(title)
                ));
            }
            host.set_inner_html(&html);
        }
    }
}

pub fn attach_wheel_listeners() {
    let Some(canvas) = dom::get("omnigraph-canvas") else {
        return;
    };
    if canvas.get_attribute("data-wheel-bound").as_deref() == Some("true") {
        return;
    }
    let _ = canvas.set_attribute("data-wheel-bound", "true");
    let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Ok(wheel) = event.dyn_into::<web_sys::WheelEvent>() else {
            return;
        };
        wheel.prevent_default();
        wheel.stop_propagation();
        let dy = wheel.delta_y();
        if dy.abs() < f64::EPSILON {
            return;
        }
        let factor = if dy < 0.0 { 1.14 } else { 1.0 / 1.14 };
        dom::attr("omnigraph-canvas", "data-wheeling", "true");
        zoom_canvas_at(
            factor,
            Some((wheel.client_x() as f64, wheel.client_y() as f64)),
        );
        dom::later("omnigraph-canvas-wheel-end", 140, || {
            dom::attr("omnigraph-canvas", "data-wheeling", "false");
        });
    });
    let options = web_sys::AddEventListenerOptions::new();
    options.set_passive(false);
    if canvas
        .add_event_listener_with_callback_and_add_event_listener_options(
            "wheel",
            callback.as_ref().unchecked_ref(),
            &options,
        )
        .is_ok()
    {
        callback.forget();
    }
}

fn jump_from_radar(client_x: f64, client_y: f64) {
    let Some(radar) = dom::get("omnigraph-canvas-radar").and_then(|el| {
        el.query_selector(".omni-radar-svg")
            .ok()
            .flatten()
            .or(Some(el))
    }) else {
        return;
    };
    let rect = radar.get_bounding_client_rect();
    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        return;
    }
    let rx = ((client_x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    let ry = ((client_y - rect.top()) / rect.height()).clamp(0.0, 1.0);
    let mut cam = load_camera();
    let (vw, vh) = viewport_size();
    let world_x = rx * cam.plane_w.max(320.0);
    let world_y = ry * cam.plane_h.max(240.0);
    cam.pan_x = (vw / 2.0 - world_x * cam.scale).round();
    cam.pan_y = (vh / 2.0 - world_y * cam.scale).round();
    cam.fit_mode = false;
    save_camera(&cam);
    apply_camera(&cam);
}

pub fn pointer_event(kind: &str, event: &web_sys::Event) -> bool {
    if m::read(|s| s.view != "omnigraph") {
        return false;
    }
    let target_el = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    let in_radar = target_el
        .as_ref()
        .and_then(|el| el.closest("#omnigraph-canvas-radar").ok().flatten())
        .is_some();
    if in_radar {
        if let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() {
            if kind == "pointerdown" || (kind == "pointermove" && pe.buttons() == 1) {
                jump_from_radar(pe.client_x() as f64, pe.client_y() as f64);
                return true;
            }
        }
        return false;
    }
    let in_canvas = target_el
        .as_ref()
        .and_then(|el| el.closest("#omnigraph-canvas").ok().flatten())
        .is_some();
    let drag = m::data("omnigraph-drag");
    let drag_active = m::yes(&drag["active"]);
    match kind {
        "pointerdown" if in_canvas => {
            let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() else {
                return false;
            };
            if pe.button() != 0 {
                return false;
            }
            let cam = load_camera();
            m::set(
                "omnigraph-drag",
                json!({
                    "active": true,
                    "pointer_id": pe.pointer_id(),
                    "start_x": pe.client_x() as f64,
                    "start_y": pe.client_y() as f64,
                    "orig_pan_x": cam.pan_x,
                    "orig_pan_y": cam.pan_y,
                    "moved": false,
                    "suppress_click": false,
                }),
            );
            true
        }
        "pointermove" if drag_active => {
            let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() else {
                return true;
            };
            if pe.pointer_id() as i64 != drag["pointer_id"].as_i64().unwrap_or(-1) {
                return true;
            }
            let dx = pe.client_x() as f64 - drag["start_x"].as_f64().unwrap_or(0.0);
            let dy = pe.client_y() as f64 - drag["start_y"].as_f64().unwrap_or(0.0);
            let mut moved = m::yes(&drag["moved"]);
            if !moved {
                if (dx * dx + dy * dy).sqrt() <= DRAG_THRESHOLD_PX {
                    return true;
                }
                moved = true;
                dom::attr("omnigraph-canvas", "data-panning", "true");
                if let Some(c) = dom::get("omnigraph-canvas") {
                    let _ = c.set_pointer_capture(pe.pointer_id());
                }
            }
            let orig_x = drag["orig_pan_x"].as_f64().unwrap_or(0.0);
            let orig_y = drag["orig_pan_y"].as_f64().unwrap_or(0.0);
            let mut next_drag = drag;
            next_drag["moved"] = json!(moved);
            m::set("omnigraph-drag", next_drag);
            let mut cam = load_camera();
            cam.pan_x = (orig_x + dx).round();
            cam.pan_y = (orig_y + dy).round();
            cam.fit_mode = false;
            save_camera(&cam);
            apply_camera(&cam);
            true
        }
        "pointerup" | "pointercancel" if drag_active => {
            dom::attr("omnigraph-canvas", "data-panning", "false");
            if let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() {
                if m::yes(&drag["moved"]) {
                    if let Some(c) = dom::get("omnigraph-canvas") {
                        let _ = c.release_pointer_capture(pe.pointer_id());
                    }
                }
            }
            let suppress = kind == "pointerup" && m::yes(&drag["moved"]);
            m::set(
                "omnigraph-drag",
                json!({
                    "active": false,
                    "moved": false,
                    "suppress_click": suppress,
                }),
            );
            if suppress {
                dom::later("omnigraph-drag-clear", 70, || {
                    let mut d = m::data("omnigraph-drag");
                    if d.is_object() {
                        d["suppress_click"] = json!(false);
                        m::set("omnigraph-drag", d);
                    }
                });
            }
            true
        }
        "dblclick" if in_canvas => {
            if let Ok(me) = event.clone().dyn_into::<web_sys::MouseEvent>() {
                zoom_canvas_at(
                    ZOOM_STEP,
                    Some((me.client_x() as f64, me.client_y() as f64)),
                );
            }
            true
        }
        _ => false,
    }
}

pub fn key(event: &web_sys::KeyboardEvent) -> bool {
    if m::read(|s| s.view != "omnigraph") {
        return false;
    }
    let target = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    let in_input = target
        .as_ref()
        .is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT"));

    if event.key() == "/"
        && !event.ctrl_key()
        && !event.meta_key()
        && !event.alt_key()
        && !in_input
        && dom::get("omnigraph-search")
            .is_some_and(|el| el.closest("[hidden]").ok().flatten().is_none())
    {
        dom::focus("omnigraph-search");
        return true;
    }

    if let Some(el) = target.as_ref() {
        if el.get_attribute("role").as_deref() == Some("tab")
            && el.closest("#omnigraph-view").ok().flatten().is_some()
        {
            let key = event.key();
            if ["ArrowLeft", "ArrowRight", "Home", "End"].contains(&key.as_str()) {
                let stage_tabs = ["constellation", "dag", "merge"];
                let drawer_tabs = ["inspector", "pulse", "publish"];
                let id = el.id();
                if let Some(pos) = stage_tabs
                    .iter()
                    .position(|t| id == format!("omni-tab-{t}"))
                {
                    let next = match key.as_str() {
                        "ArrowRight" => (pos + 1) % stage_tabs.len(),
                        "ArrowLeft" => (pos + stage_tabs.len() - 1) % stage_tabs.len(),
                        "Home" => 0,
                        _ => stage_tabs.len() - 1,
                    };
                    let tab = stage_tabs[next];
                    set_ui_state(|s| s["stage_tab"] = json!(tab));
                    render_tabs();
                    dom::focus(&format!("omni-tab-{tab}"));
                    return true;
                }
                if let Some(pos) = drawer_tabs
                    .iter()
                    .position(|t| id == format!("omni-drawer-tab-{t}"))
                {
                    let next = match key.as_str() {
                        "ArrowRight" => (pos + 1) % drawer_tabs.len(),
                        "ArrowLeft" => (pos + drawer_tabs.len() - 1) % drawer_tabs.len(),
                        "Home" => 0,
                        _ => drawer_tabs.len() - 1,
                    };
                    let tab = drawer_tabs[next];
                    set_ui_state(|s| s["drawer_tab"] = json!(tab));
                    render_tabs();
                    dom::focus(&format!("omni-drawer-tab-{tab}"));
                    return true;
                }
            }
        }
    }

    if !in_input && !event.ctrl_key() && !event.meta_key() && !event.alt_key() {
        match event.key().as_str() {
            "+" | "=" => {
                zoom_canvas_at(ZOOM_STEP, None);
                return true;
            }
            "-" => {
                zoom_canvas_at(1.0 / ZOOM_STEP, None);
                return true;
            }
            "0" => {
                fit_camera();
                return true;
            }
            _ => {}
        }
    }
    false
}

fn apply_merged_session_state(sid: &str, branch: &str, res: &V) {
    let merge_status = res["outcome"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("merged");
    if res["session"].is_object() {
        let mut updated = res["session"].clone();
        if updated["omnigraph"].is_object()
            && updated["omnigraph"]["merge_status"]
                .as_str()
                .is_none_or(str::is_empty)
        {
            updated["omnigraph"]["merge_status"] = json!(merge_status);
        }
        m::upsert(updated);
    }
    m::write(|s| {
        let patch = |entry: &mut V| {
            let eid = entry["id"].as_str().unwrap_or("");
            let ebranch = entry["omnigraph"]["branch"].as_str().unwrap_or("");
            let matches_target =
                (!sid.is_empty() && eid == sid) || (!branch.is_empty() && ebranch == branch);
            if matches_target && entry["omnigraph"].is_object() {
                entry["omnigraph"]["merge_status"] = json!(merge_status);
                entry["omnigraph"]["last_error"] = V::Null;
            }
        };
        patch(&mut s.selected);
        for item in &mut s.sessions {
            patch(item);
        }
    });
}

pub async fn event(
    kind: &str,
    id: &str,
    action: &str,
    payload: &V,
    value: &str,
    checked: bool,
) -> bool {
    if kind == "input" && id == "omnigraph-search" {
        let q = value.to_owned();
        dom::later("omnigraph-search-debounce", 220, move || {
            spawn_local(async move {
                run_search(q).await;
            });
        });
        return true;
    }

    if kind == "change" {
        match id {
            "omnigraph-project" => {
                crate::session::sync_project(value);
                on_project_changed();
                return true;
            }
            "omnigraph-branch" => {
                let _ = switch_branch_state(value, true);
                reset_pulse_cursor();
                m::set("omnigraph-commit-detail", V::Null);
                refresh_graph().await;
                return true;
            }
            "omnigraph-show-sessions" => {
                set_ui_state(|s| s["show_sessions"] = json!(checked));
                render_constellation();
                return true;
            }
            "omnigraph-show-files" => {
                set_ui_state(|s| s["show_files"] = json!(checked));
                render_constellation();
                return true;
            }
            "omnigraph-search" => {
                run_search(value.to_owned()).await;
                return true;
            }
            _ => {}
        }
    }

    if kind == "submit" {
        match id {
            "omnigraph-connect-form" => {
                run_setup_action(
                    "connect",
                    Some(json!({
                        "endpoint": dom::value("omnigraph-ext-endpoint").trim(),
                        "token": dom::value("omnigraph-ext-token").trim(),
                        "graph_id": dom::value("omnigraph-ext-graph").trim(),
                    })),
                    "omnigraph-status-message",
                    "omnigraph-error",
                )
                .await;
                return true;
            }
            "omni-dlg-connect-form" => {
                run_setup_action(
                    "connect",
                    Some(json!({
                        "endpoint": dom::value("omni-dlg-endpoint").trim(),
                        "token": dom::value("omni-dlg-token").trim(),
                        "graph_id": dom::value("omni-dlg-graph").trim(),
                    })),
                    "omni-dlg-status",
                    "omni-dlg-error",
                )
                .await;
                return true;
            }
            "omni-create-branch-form" => {
                let name = dom::value("omni-new-branch-name").trim().to_owned();
                if name.is_empty() {
                    return true;
                }
                let from = dom::value("omni-new-branch-from");
                let project = current_project_id();
                match execute_branch_action(
                    json!({
                        "action": "create",
                        "project_id": project,
                        "name": name,
                        "from": if from.is_empty() { "main" } else { &from },
                    }),
                    &format!("Created branch {name}."),
                )
                .await
                {
                    Ok(_) => {
                        dom::set_value("omni-new-branch-name", "");
                        let _ = switch_branch_state(&name, true);
                        reset_pulse_cursor();
                        refresh_graph().await;
                    }
                    Err(e) => dom::error("omnigraph-error", &e.message),
                }
                return true;
            }
            "omni-publish-form" => {
                dom::error("omni-pub-error", "");
                dom::hidden("omni-pub-status", true);
                let title = dom::value("omni-pub-title").trim().to_owned();
                let summary = dom::value("omni-pub-summary").trim().to_owned();
                if title.is_empty() || summary.is_empty() {
                    dom::error("omni-pub-error", "Title and summary are required.");
                    return true;
                }
                let project = current_project_id();
                let branch = dom::value("omni-pub-branch");
                let session_id = dom::value("omni-pub-session");
                let confidence = dom::value("omni-pub-confidence")
                    .trim()
                    .parse::<f64>()
                    .unwrap_or(0.9)
                    .clamp(0.0, 1.0);
                let pin_priority = dom::value("omni-pub-pin")
                    .trim()
                    .parse::<i64>()
                    .unwrap_or(50)
                    .clamp(0, 100);
                let mut body = json!({
                    "project_id": project,
                    "branch": if branch.is_empty() { "main" } else { &branch },
                    "kind": dom::value("omni-pub-kind"),
                    "title": title,
                    "summary": summary,
                    "body": dom::value("omni-pub-body").trim(),
                    "tags": parse_csv_list(&dom::value("omni-pub-tags")),
                    "files": parse_csv_list(&dom::value("omni-pub-files")),
                    "relates_to": parse_relates_to(&dom::value("omni-pub-relates")),
                    "confidence": confidence,
                    "pin_priority": pin_priority,
                });
                if !session_id.is_empty() {
                    body["session_id"] = json!(session_id);
                }
                match api::post("/api/omnigraph/atoms", body).await {
                    Ok(res) => {
                        if res["ok"].as_bool() == Some(false) {
                            let msg = res["error"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .unwrap_or("Failed to publish context atom.");
                            dom::error("omni-pub-error", msg);
                            return true;
                        }
                        let new_id = res["atom"]["atom_id"]
                            .as_str()
                            .or_else(|| res["atom"]["id"].as_str())
                            .or_else(|| res["atom_id"].as_str())
                            .or_else(|| res["id"].as_str())
                            .unwrap_or("")
                            .to_owned();
                        dom::set_value("omni-pub-title", "");
                        dom::set_value("omni-pub-summary", "");
                        dom::set_value("omni-pub-body", "");
                        dom::set_value("omni-pub-tags", "");
                        dom::set_value("omni-pub-files", "");
                        dom::set_value("omni-pub-relates", "");
                        dom::text("omni-pub-status", "Published context atom to Omnigraph.");
                        dom::hidden("omni-pub-status", false);
                        if !new_id.is_empty() {
                            set_ui_state(|s| {
                                s["selected_id"] = json!(new_id);
                                s["drawer_tab"] = json!("inspector");
                            });
                        }
                        refresh_pill().await;
                        refresh_graph().await;
                        if !new_id.is_empty() {
                            fly_to_node(&new_id);
                        }
                    }
                    Err(e) => dom::error("omni-pub-error", &e.message),
                }
                return true;
            }
            _ => {}
        }
    }

    if kind != "click" {
        return false;
    }

    if action == "omnigraph-select-node" && m::yes(&m::data("omnigraph-drag")["suppress_click"]) {
        return true;
    }

    match action {
        "omnigraph-install-start" | "omni-dlg-install" => {
            let status_el = if action == "omni-dlg-install" {
                "omni-dlg-status"
            } else {
                "omnigraph-status-message"
            };
            let err_el = if action == "omni-dlg-install" {
                "omni-dlg-error"
            } else {
                "omnigraph-error"
            };
            run_setup_action("install", None, status_el, err_el).await;
            true
        }
        "omnigraph-start-local" | "omni-dlg-start-local" => {
            let status_el = if action == "omni-dlg-start-local" {
                "omni-dlg-status"
            } else {
                "omnigraph-status-message"
            };
            let err_el = if action == "omni-dlg-start-local" {
                "omni-dlg-error"
            } else {
                "omnigraph-error"
            };
            run_setup_action("start_local", None, status_el, err_el).await;
            true
        }
        "omnigraph-start-docker" | "omni-dlg-start-docker" => {
            let status_el = if action == "omni-dlg-start-docker" {
                "omni-dlg-status"
            } else {
                "omnigraph-status-message"
            };
            let err_el = if action == "omni-dlg-start-docker" {
                "omni-dlg-error"
            } else {
                "omnigraph-error"
            };
            run_setup_action("start_docker", None, status_el, err_el).await;
            true
        }
        "omnigraph-apply-schema" | "omni-dlg-apply-schema" => {
            let status_el = if action == "omni-dlg-apply-schema" {
                "omni-dlg-status"
            } else {
                "omnigraph-status-message"
            };
            let err_el = if action == "omni-dlg-apply-schema" {
                "omni-dlg-error"
            } else {
                "omnigraph-error"
            };
            run_setup_action("apply_schema", None, status_el, err_el).await;
            true
        }
        "omnigraph-stop" | "omni-dlg-stop" => {
            let status_el = if action == "omni-dlg-stop" {
                "omni-dlg-status"
            } else {
                "omnigraph-status-message"
            };
            let err_el = if action == "omni-dlg-stop" {
                "omni-dlg-error"
            } else {
                "omnigraph-error"
            };
            run_setup_action("stop", None, status_el, err_el).await;
            true
        }
        "omnigraph-open-settings" => {
            let status = m::data("omnigraph-status");
            dom::set_value("omni-dlg-endpoint", &m::text(&status, "endpoint"));
            dom::set_value("omni-dlg-graph", &m::text(&status, "graph_id"));
            dom::hidden("omni-dlg-status", true);
            dom::error("omni-dlg-error", "");
            dom::dialog("omni-settings-dialog", true);
            true
        }
        "omni-settings-close" => {
            dom::dialog("omni-settings-dialog", false);
            true
        }
        "omnigraph-mcp-copy" => {
            let mcp = m::text(&m::data("omnigraph-status"), "mcp_url");
            if !mcp.is_empty() {
                let _ =
                    JsFuture::from(dom::window().navigator().clipboard().write_text(&mcp)).await;
                dom::text(
                    "omnigraph-status-message",
                    &format!("Copied MCP URL: {mcp}"),
                );
                dom::hidden("omnigraph-status-message", false);
            }
            true
        }
        "omnigraph-refresh-header" | "omni-refresh" => {
            load().await;
            true
        }
        "omnigraph-search-clear" => {
            dom::set_value("omnigraph-search", "");
            run_search(String::new()).await;
            dom::focus("omnigraph-search");
            true
        }
        "omnigraph-clear-filters" => {
            dom::set_value("omnigraph-search", "");
            m::set("omnigraph-search-results", V::Null);
            set_ui_state(|s| {
                s["query"] = json!("");
                s["kind"] = json!("");
                s["show_files"] = json!(true);
                s["show_sessions"] = json!(true);
                s["highlighted_ids"] = json!([]);
            });
            dom::hidden("omnigraph-search-clear", true);
            sync_controls_from_state();
            refresh_graph().await;
            true
        }
        "omnigraph-open-publish" => {
            set_ui_state(|s| s["drawer_tab"] = json!("publish"));
            render_tabs();
            dom::focus("omni-pub-title");
            true
        }
        "omnigraph-filter-kind" => {
            let next_kind = payload["kind"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or("");
            set_ui_state(|s| s["kind"] = json!(next_kind));
            sync_controls_from_state();
            refresh_graph().await;
            true
        }
        "omnigraph-stage-tab" => {
            let tab = payload["tab"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or_else(|| id.strip_prefix("omni-tab-").unwrap_or("constellation"));
            set_ui_state(|s| s["stage_tab"] = json!(tab));
            render_tabs();
            if tab == "constellation" {
                render_constellation();
            } else if tab == "dag" {
                render_dag();
            } else if tab == "merge" {
                let src = dom::value("omni-merge-source");
                let dst = dom::value("omni-merge-target");
                if !src.is_empty() && !dst.is_empty() && !m::data("omnigraph-diff").is_object() {
                    preview_merge(&src, &dst).await;
                } else {
                    render_merge();
                }
            }
            true
        }
        "omnigraph-drawer-tab" => {
            let tab = payload["tab"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or_else(|| id.strip_prefix("omni-drawer-tab-").unwrap_or("inspector"));
            set_ui_state(|s| s["drawer_tab"] = json!(tab));
            render_tabs();
            true
        }
        "omni-zoom-in" => {
            zoom_canvas_at(ZOOM_STEP, None);
            true
        }
        "omni-zoom-out" => {
            zoom_canvas_at(1.0 / ZOOM_STEP, None);
            true
        }
        "omni-zoom-level" => {
            reset_camera_100();
            true
        }
        "omni-fit" => {
            fit_camera();
            true
        }
        "omni-toggle-pulse" => {
            let paused = !m::yes(&ui_state()["pulse_paused"]);
            set_ui_state(|s| s["pulse_paused"] = json!(paused));
            sync_pulse_toggle();
            true
        }
        "omnigraph-return-live" | "omni-return-live" => {
            set_ui_state(|s| {
                s["snapshot"] = V::Null;
                s["selected_commit_id"] = V::Null;
            });
            reset_pulse_cursor();
            m::set("omnigraph-commit-detail", V::Null);
            sync_controls_from_state();
            refresh_graph().await;
            true
        }
        "omnigraph-select-node" | "omnigraph-focus-hit" => {
            let node_id = payload["node_id"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or("")
                .to_owned();
            if !node_id.is_empty() {
                set_ui_state(|s| {
                    s["selected_id"] = json!(node_id);
                    s["drawer_tab"] = json!("inspector");
                    if action == "omnigraph-focus-hit" {
                        s["stage_tab"] = json!("constellation");
                    }
                });
                render_tabs();
                render_constellation();
                render_inspector();
                fly_to_node(&node_id);
            }
            true
        }
        "omnigraph-pulse-focus" => {
            let node_id = payload["node_id"].as_str().unwrap_or("").to_owned();
            let branch = payload["branch"].as_str().unwrap_or("").to_owned();
            if switch_branch_state(&branch, true) {
                reset_pulse_cursor();
            }
            set_ui_state(|s| {
                if !node_id.is_empty() {
                    s["selected_id"] = json!(node_id);
                }
                s["stage_tab"] = json!("constellation");
                s["drawer_tab"] = json!("inspector");
            });
            render_tabs();
            refresh_graph().await;
            if !node_id.is_empty() {
                fly_to_node(&node_id);
            }
            true
        }
        "omnigraph-select-commit" => {
            let commit_id = payload["commit_id"].as_str().unwrap_or("").to_owned();
            let branch = payload["branch"].as_str().unwrap_or("").to_owned();
            if !commit_id.is_empty() {
                set_ui_state(|s| s["selected_id"] = V::Null);
                load_commit_detail(&commit_id, &branch).await;
            }
            true
        }
        "omnigraph-preview-diff" => {
            let branch = payload["branch"].as_str().unwrap_or("").to_owned();
            if !branch.is_empty() {
                dom::set_value("omni-merge-source", &branch);
                dom::set_value("omni-merge-target", "main");
            }
            set_ui_state(|s| s["stage_tab"] = json!("merge"));
            render_tabs();
            let src = dom::value("omni-merge-source");
            let dst = dom::value("omni-merge-target");
            preview_merge(&src, &dst).await;
            true
        }
        "omnigraph-switch-branch" | "omnigraph-open-branch" => {
            let branch = payload["branch"].as_str().unwrap_or("").to_owned();
            if !branch.is_empty() {
                let _ = switch_branch_state(&branch, true);
                reset_pulse_cursor();
                m::set("omnigraph-commit-detail", V::Null);
                sync_controls_from_state();
                refresh_graph().await;
            }
            true
        }
        "omni-delete-branch" => {
            let branch = m::text(&ui_state(), "branch");
            if !branch.is_empty() && branch != "main" {
                let project = current_project_id();
                match execute_branch_action(
                    json!({
                        "action": "delete",
                        "project_id": project,
                        "name": branch,
                    }),
                    &format!("Deleted branch {branch}."),
                )
                .await
                {
                    Ok(_) => {
                        let _ = switch_branch_state("main", true);
                        reset_pulse_cursor();
                        refresh_graph().await;
                    }
                    Err(e) => dom::error("omnigraph-error", &e.message),
                }
            }
            true
        }
        "omni-merge-preview" => {
            preview_merge(
                &dom::value("omni-merge-source"),
                &dom::value("omni-merge-target"),
            )
            .await;
            true
        }
        "omni-merge-exec" | "omni-merge-delete" => {
            let src = dom::value("omni-merge-source");
            let dst = dom::value("omni-merge-target");
            let delete_branch = action == "omni-merge-delete";
            if src.is_empty() || dst.is_empty() {
                dom::error(
                    "omnigraph-error",
                    "Choose source and target branches to merge.",
                );
                return true;
            }
            let project = current_project_id();
            match execute_branch_action(
                json!({
                    "action": "merge",
                    "project_id": project,
                    "source": src,
                    "target": dst,
                    "dry_run": false,
                    "delete_branch": delete_branch,
                }),
                &format!("Merged {src} into {dst}."),
            )
            .await
            {
                Ok(mut res) => {
                    if let Err(msg) = branch_merge_outcome(&res) {
                        dom::error("omnigraph-error", &msg);
                        if !m::array(&res["conflicts"]).is_empty() || res["outcome"] == "conflict" {
                            if res["source"].is_null() {
                                res["source"] = json!(src);
                            }
                            if res["target"].is_null() {
                                res["target"] = json!(dst);
                            }
                            m::set("omnigraph-diff", res);
                            render_merge();
                        } else {
                            preview_merge(&src, &dst).await;
                        }
                    } else {
                        m::set("omnigraph-diff", res);
                        if delete_branch && m::text(&ui_state(), "branch") == src {
                            let _ = switch_branch_state(&dst, true);
                        }
                        render_merge();
                        refresh_graph().await;
                    }
                }
                Err(e) => {
                    dom::error("omnigraph-error", &e.message);
                    if e.status == 409 {
                        preview_merge(&src, &dst).await;
                    }
                }
            }
            true
        }
        "omnigraph-share-atom" => {
            dom::error("omni-share-error", "");
            dom::hidden("omni-share-status", true);
            let atom_id_str = payload["atom_id"].as_str().unwrap_or("").to_owned();
            let branch = payload["branch"].as_str().unwrap_or("main").to_owned();
            let target_session_id = dom::value("omni-share-session");
            let mode = dom::value("omni-share-mode");
            if atom_id_str.is_empty() || target_session_id.is_empty() {
                dom::error("omni-share-error", "Select a target session first.");
                return true;
            }
            let project = current_project_id();
            match api::post(
                "/api/omnigraph/share",
                json!({
                    "project_id": project,
                    "target_session_id": target_session_id,
                    "atom_ids": [atom_id_str],
                    "source_branch": branch,
                    "mode": if mode.is_empty() { "inject" } else { &mode },
                }),
            )
            .await
            {
                Ok(res) => {
                    if res["session"].is_object() {
                        m::upsert(res["session"].clone());
                        let updated_id = m::text(&res["session"], "id");
                        if !updated_id.is_empty()
                            && m::read(|s| m::text(&s.selected, "id")) == updated_id
                        {
                            render_session_bar(&res["session"]);
                        }
                    }
                    dom::text("omni-share-status", "Shared atom into session.");
                    dom::hidden("omni-share-status", false);
                    poll_changes(true).await;
                }
                Err(e) => dom::error("omni-share-error", &e.message),
            }
            true
        }
        "omnigraph-copy-snippet" => {
            let aid = payload["atom_id"].as_str().unwrap_or("");
            if let Some(atom) = m::array(&m::data("omnigraph-graph")["atoms"])
                .into_iter()
                .find(|a| atom_id(a) == aid)
            {
                let snippet = format_context_snippet(&atom);
                let _ = JsFuture::from(dom::window().navigator().clipboard().write_text(&snippet))
                    .await;
                dom::text("omni-share-status", "Copied context snippet to clipboard.");
                dom::hidden("omni-share-status", false);
            }
            true
        }
        "omnigraph-cite-atom" => {
            let aid = payload["atom_id"].as_str().unwrap_or("");
            if !aid.is_empty() {
                let cur = dom::value("omni-pub-relates");
                let mut list = parse_csv_list(&cur);
                if !list.iter().any(|x| {
                    x == aid
                        || x.split_once(':')
                            .is_some_and(|(_, target)| target.trim() == aid)
                }) {
                    list.push(aid.to_owned());
                }
                dom::set_value("omni-pub-relates", &list.join(", "));
                set_ui_state(|s| s["drawer_tab"] = json!("publish"));
                render_tabs();
                dom::focus("omni-pub-title");
            }
            true
        }
        "omni-pulse-poll" => {
            poll_changes(true).await;
            true
        }
        "omnigraph-open-atom" => {
            let aid = payload["atom_id"].as_str().unwrap_or("").to_owned();
            let branch = payload["branch"].as_str().unwrap_or("main").to_owned();
            let _ = switch_branch_state(&branch, true);
            set_ui_state(|s| {
                s["selected_id"] = json!(aid);
                s["stage_tab"] = json!("constellation");
                s["drawer_tab"] = json!("inspector");
            });
            crate::session::open_view("omnigraph", true).await;
            if !aid.is_empty() {
                fly_to_node(&aid);
            }
            true
        }
        "session-omnigraph-merge" => {
            let session = m::read(|s| s.selected.clone());
            let sid = m::text(&session, "id");
            let branch = session["omnigraph"]["branch"].as_str().unwrap_or("");
            let project = m::text(&session, "project_id");
            if branch.is_empty() || branch == "main" {
                return true;
            }
            dom::error("session-omnigraph-error", "");
            let mut body = json!({
                "action": "merge",
                "project_id": if project.is_empty() { current_project_id() } else { project },
                "source": branch,
                "target": "main",
                "dry_run": false,
                "delete_branch": false,
            });
            if !sid.is_empty() {
                body["session_id"] = json!(sid);
            }
            match api::post("/api/omnigraph/branches", body).await {
                Ok(res) => {
                    if let Err(msg) = branch_merge_outcome(&res) {
                        dom::error("session-omnigraph-error", &msg);
                    } else {
                        apply_merged_session_state(&sid, branch, &res);
                        let cur = m::read(|s| s.selected.clone());
                        if !sid.is_empty() && m::text(&cur, "id") == sid {
                            render_session_bar(&cur);
                        }
                        dom::text("session-omnigraph-meta", &format!("Merged {branch} → main"));
                        dom::hidden("session-omnigraph-merge", true);
                        refresh_pill().await;
                    }
                }
                Err(e) => dom::error("session-omnigraph-error", &e.message),
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pill_presentation_reflects_connected_building_stopped_failed_and_off_states() {
        let live = json!({
            "connected": true,
            "mode": "local",
            "stats": { "atoms": 14, "branches": 3 }
        });
        let (state, shape, label, title) = pill_presentation(&live);
        assert_eq!(state, "ready");
        assert_eq!(shape, "●");
        assert_eq!(label, "Omnigraph: local · 14 atoms");
        assert!(title.contains("3 branches"));

        let starting = json!({ "connected": false, "state": "starting" });
        assert_eq!(pill_presentation(&starting).0, "building");

        let failed = json!({ "connected": false, "error": "port 8080 in use" });
        assert_eq!(pill_presentation(&failed).0, "failed");
        assert!(pill_presentation(&failed).3.contains("port 8080"));

        let stopped = json!({ "connected": false, "binary_installed": true });
        assert_eq!(pill_presentation(&stopped).0, "stale");
        assert_eq!(pill_presentation(&stopped).2, "Omnigraph: stopped");

        let off = json!({ "connected": false });
        assert_eq!(pill_presentation(&off).0, "off");
        assert_eq!(pill_presentation(&off).2, "Omnigraph: off");
    }

    #[test]
    fn session_payload_parses_branch_modes_pull_branches_and_query() {
        assert!(
            session_payload_from_fields(false, "auto", "", "main", "auth", true, false).is_none()
        );

        let auto = session_payload_from_fields(
            true,
            "auto",
            "ignored",
            "main, session/fix-1, main , ",
            "  auth tokens  ",
            true,
            true,
        )
        .unwrap();
        assert_eq!(
            auto,
            json!({
                "enabled": true,
                "branch": "auto",
                "pull_branches": ["main", "session/fix-1"],
                "query": "auth tokens",
                "auto_publish": true,
                "auto_merge": true,
            })
        );

        let existing =
            session_payload_from_fields(true, "existing", "session/wave-2", "", "", false, false)
                .unwrap();
        assert_eq!(existing["branch"], "session/wave-2");
        assert!(existing.get("pull_branches").is_none());
    }

    #[test]
    fn camera_zoom_at_preserves_cursor_world_point_and_fly_to_centers_node() {
        let (s, px, py) = zoom_at(1.0, 20.0, 40.0, (220.0, 140.0), 1.5);
        assert!((s - 1.5).abs() < 1e-6);
        let world_x_before = (220.0 - 20.0) / 1.0;
        let world_x_after = (220.0 - px) / s;
        assert!((world_x_before - world_x_after).abs() < 1.0);
        let world_y_before = (140.0 - 40.0) / 1.0;
        let world_y_after = (140.0 - py) / s;
        assert!((world_y_before - world_y_after).abs() < 1.0);

        let (fs, fpx, fpy) = camera_for_fly_to((300.0, 200.0), (800.0, 600.0), 1.0);
        assert_eq!(fs, 1.0);
        assert_eq!(fpx, 400.0 - 300.0);
        assert_eq!(fpy, 300.0 - 200.0);
    }

    #[test]
    fn inspector_renders_atom_provenance_relations_files_and_escapes_html() {
        let graph = json!({
            "atoms": [{
                "id": "atom-1",
                "kind": "warning",
                "title": "Avoid <unsafe> SQL",
                "summary": "Found unescaped interpolation & risk",
                "body": "Use bound parameters everywhere.",
                "branch": "session/sec-audit",
                "confidence": 0.92,
                "pin_priority": 80,
                "actor_id": "codex-sec",
                "session_id": "sess-9",
                "commit_id": "commit-abcdef1234",
                "tags": ["security", "sql"],
                "files": ["src/Repo.php"]
            }],
            "sessions": [{ "id": "sess-9", "title": "Security Audit" }],
            "edges": [
                { "from": "atom-1", "to": "atom-2", "kind": "contradicts" },
                { "from": "atom-1", "to": "src/Repo.php", "kind": "touches_file" }
            ]
        });
        let sessions = vec![json!({ "id": "sess-10", "title": "Fix Worker" })];
        let html = render_node_inspector_html(&graph, "atom-1", &sessions, "main");
        assert!(html.contains("Avoid &lt;unsafe&gt; SQL"));
        assert!(html.contains("interpolation &amp; risk"));
        assert!(html.contains("92% confidence"));
        assert!(html.contains("Pin 80"));
        assert!(html.contains("session/sec-audit"));
        assert!(html.contains("src/Repo.php"));
        assert!(html.contains("contradicts"));
        assert!(html.contains("Fix Worker"));
        assert!(html.contains("data-action=\"omnigraph-share-atom\""));
        assert!(html.contains("data-action=\"omnigraph-copy-snippet\""));
        assert!(html.contains("data-action=\"omnigraph-open-branch\""));
    }

    #[test]
    fn search_hits_and_pulse_events_render_and_escape_safely() {
        let hits = json!({
            "query": "<script>",
            "hits": [{
                "atom": {
                    "id": "a1",
                    "kind": "decision",
                    "title": "Use <Outbox> table",
                    "summary": "Atomic writes"
                },
                "score": 4.25,
                "matched_via": "bm25",
                "neighbors": ["a2"]
            }]
        });
        let hits_html = render_search_hits_html(&hits);
        assert!(!hits_html.contains("<script>"));
        assert!(hits_html.contains("Use &lt;Outbox&gt; table"));
        assert!(hits_html.contains("4.25"));
        assert!(hits_html.contains("1 neighbors"));

        let pulse = vec![json!({
            "kind": "atom_published",
            "branch": "session/s1",
            "atom_id": "a1",
            "title": "New finding & fix",
            "actor_id": "claude",
            "created_at": "2026-11-16T18:00:00Z"
        })];
        let pulse_html = render_pulse_list_html(&pulse);
        assert!(pulse_html.contains("New finding &amp; fix"));
        assert!(pulse_html.contains("data-action=\"omnigraph-pulse-focus\""));
        assert!(pulse_html.contains("data-node-id=\"a1\""));
    }

    #[test]
    fn session_bar_extraction_handles_branches_and_atoms() {
        let off = json!({ "id": "s1" });
        assert!(!session_bar_summary(&off).0);

        let active = json!({
            "id": "s2",
            "omnigraph": {
                "enabled": true,
                "branch": "session/s2",
                "auto_publish": true,
                "auto_merge": false,
                "injected_atoms": [
                    { "id": "a1", "kind": "contract", "title": "API schema" }
                ]
            }
        });
        let (visible, branch, meta, atoms, can_merge) = session_bar_summary(&active);
        assert!(visible);
        assert_eq!(branch, "session/s2");
        assert!(meta.contains("1 shared atoms"));
        assert!(meta.contains("auto-publish"));
        assert_eq!(atoms.len(), 1);
        assert!(can_merge);
    }

    #[test]
    fn csv_parser_trims_and_deduplicates_entries() {
        assert_eq!(
            parse_csv_list(" auth , migration, auth\napi , "),
            vec!["auth", "migration", "api"]
        );
        assert!(parse_csv_list(" , \n ").is_empty());
    }

    #[test]
    fn commit_inspector_session_and_file_nodes_render_cleanly() {
        let graph = json!({
            "sessions": [{ "id": "sess-1", "title": "Auth Refactor", "branch": "session/auth", "status": "completed" }],
            "files": [{ "id": "src/auth.rs", "path": "src/auth.rs" }],
            "atoms": [{ "id": "atom-1", "kind": "finding", "title": "Token check", "branch": "session/auth" }],
            "edges": [
                { "from": "atom-1", "to": "sess-1", "kind": "published_by" },
                { "from": "atom-1", "to": "src/auth.rs", "kind": "touches_file" }
            ]
        });
        let sess_html = render_node_inspector_html(&graph, "sess-1", &[], "main");
        assert!(sess_html.contains("Auth Refactor"));
        assert!(sess_html.contains("data-action=\"session-open\""));
        assert!(sess_html.contains("published_by"));

        let file_html = render_node_inspector_html(&graph, "src/auth.rs", &[], "main");
        assert!(file_html.contains("src/auth.rs"));
        assert!(file_html.contains("data-action=\"inspect-file-graph\""));

        let detail = json!({
            "commit": { "id": "c-1234", "branch": "session/auth", "actor_id": "claude", "created_at": "2026-11-16T18:05:00Z" },
            "changes": [{ "op": "insert", "title": "Token <check>" }],
            "atoms": [{ "id": "atom-1", "title": "Token check" }]
        });
        let commit_html = render_commit_inspector_html(&detail);
        assert!(commit_html.contains("c-1234"));
        assert!(commit_html.contains("Token &lt;check&gt;"));
        assert!(commit_html.contains("data-action=\"omnigraph-return-live\""));
    }

    #[test]
    fn context_snippet_and_canvas_positions_integrate_cleanly() {
        let atom = json!({
            "kind": "contract",
            "title": "Bearer Auth Header",
            "branch": "main",
            "summary": "All requests require bearer auth.",
            "files": ["src/http.rs", "src/auth.rs"]
        });
        let snippet = format_context_snippet(&atom);
        assert!(snippet.contains("[Contract] Bearer Auth Header (branch: main)"));
        assert!(snippet.contains("Files: src/http.rs, src/auth.rs"));

        let graph = json!({
            "branches": ["main"],
            "sessions": [{ "id": "s1", "title": "Session 1", "branch": "main" }],
            "atoms": [{ "id": "a1", "kind": "finding", "title": "Auth finding", "branch": "main" }],
            "files": [{ "id": "src/auth.rs", "path": "src/auth.rs" }],
            "edges": [
                { "from": "a1", "to": "s1", "kind": "published_by" },
                { "from": "a1", "to": "src/auth.rs", "kind": "touches_file" }
            ]
        });
        let rendered = omnigraph_canvas::render_constellation(
            &graph,
            &omnigraph_canvas::ConstellationFilter {
                show_files: true,
                show_sessions: true,
                ..Default::default()
            },
            &omnigraph_canvas::CanvasViewport {
                pan_x: 0.0,
                pan_y: 0.0,
                scale: 1.0,
            },
        );
        assert!(rendered.node_count >= 3);
        let pos = rendered
            .positions
            .get("a1")
            .copied()
            .expect("a1 center position");
        let (scale, px, py) = camera_for_fly_to(pos, (800.0, 600.0), 1.0);
        assert_eq!(scale, 1.0);
        assert_eq!(px, (400.0 - pos.0).round());
        assert_eq!(py, (300.0 - pos.1).round());
    }

    #[test]
    fn backend_contract_alignment_relates_to_conflicts_raw_changes_and_session_bar() {
        assert_eq!(
            parse_relates_to(
                "atom-1, supports: atom-2, atom-1, contradicts:atom-3, :atom-4, relates_to: atom-5, RELATED:atom-1"
            ),
            vec![
                json!({ "target_id": "atom-1", "relation": "supports" }),
                json!({ "target_id": "atom-2", "relation": "supports" }),
                json!({ "target_id": "atom-3", "relation": "contradicts" }),
                json!({ "target_id": "atom-4", "relation": "supports" }),
                json!({ "target_id": "atom-5", "relation": "supports" }),
            ]
        );

        let conflict_res = json!({
            "ok": false,
            "outcome": "conflict",
            "conflicts": [{ "entity_id": "atom-1" }],
            "error": "Conflict on atom-1 title"
        });
        assert_eq!(
            branch_merge_outcome(&conflict_res).unwrap_err(),
            "Conflict on atom-1 title"
        );
        assert!(branch_merge_outcome(&json!({ "ok": true, "outcome": "merged" })).is_ok());

        let graph = json!({
            "atoms": [{
                "atom_id": "atom-9",
                "kind": "contract",
                "title": "Source fields contract",
                "summary": "Backend emits source_branch and source_session_id",
                "source_branch": "session/s9",
                "source_session_id": "sess-9",
                "authored_at": 1_763_316_000_i64
            }],
            "sessions": [{ "id": "sess-9", "title": "Session 9" }],
            "edges": []
        });
        let inspector = render_node_inspector_html(&graph, "atom-9", &[], "main");
        assert!(inspector.contains("session/s9"));
        assert!(inspector.contains("Session: sess-9"));
        assert!(inspector.contains("UTC"));
        assert!(format_context_snippet(&graph["atoms"][0]).contains("(branch: session/s9)"));

        let pulse = vec![json!({
            "entity_id": "atom-42",
            "entity_kind": "atom",
            "type_name": "ContextAtom",
            "op": "insert",
            "authored_at": 1_763_316_000_i64,
            "source_branch": "session/s4"
        })];
        let pulse_html = render_pulse_list_html(&pulse);
        assert!(pulse_html.contains("data-node-id=\"atom-42\""));
        assert!(pulse_html.contains("ContextAtom"));
        assert!(pulse_html.contains("insert · atom-42"));
        assert!(pulse_html.contains("UTC"));

        let commit_detail = json!({
            "commit": { "id": "c-raw", "branch": "session/s4", "actor_id": "harness", "authored_at": 1_763_316_000_i64 },
            "changes": [{
                "op": "upsert",
                "entity_id": "atom-42",
                "type": { "name": "ContextAtom" },
                "after": { "properties": { "title": "Auth boundary" } }
            }],
            "atoms": []
        });
        let commit_html = render_commit_inspector_html(&commit_detail);
        assert!(commit_html.contains("upsert ContextAtom · Auth boundary"));
        assert!(commit_html.contains("UTC"));

        let merged_session = json!({
            "id": "s-merged",
            "omnigraph": {
                "enabled": true,
                "branch": "session/s-merged",
                "merge_status": "merged",
                "last_error": "post-merge warning",
                "injected_atoms": [{ "id": "a1", "kind": "finding", "title": "Injected" }],
                "published_atoms": ["atom-pub-1"]
            }
        });
        let (visible, branch, meta, atoms, can_merge) = session_bar_summary(&merged_session);
        assert!(visible);
        assert_eq!(branch, "session/s-merged");
        assert!(meta.contains("1 published"));
        assert!(meta.contains("merge: merged"));
        assert!(meta.contains("error: post-merge warning"));
        assert_eq!(atoms.len(), 2);
        assert!(!can_merge);
        assert_eq!(session_bar_error(&merged_session), "post-merge warning");

        m::write(|s| {
            let sess = json!({
                "id": "s-merge-sync",
                "omnigraph": {
                    "enabled": true,
                    "branch": "session/s-merge-sync",
                    "last_error": "stale error"
                }
            });
            s.selected = sess.clone();
            s.sessions = vec![sess];
        });
        apply_merged_session_state(
            "s-merge-sync",
            "session/s-merge-sync",
            &json!({ "ok": true, "outcome": "fast_forward" }),
        );
        m::read(|s| {
            assert_eq!(s.selected["omnigraph"]["merge_status"], "fast_forward");
            assert!(s.selected["omnigraph"]["last_error"].is_null());
            assert_eq!(s.sessions[0]["omnigraph"]["merge_status"], "fast_forward");
            assert!(!session_bar_summary(&s.sessions[0]).4);
        });
    }

    #[test]
    fn branch_switch_resets_pulse_cursor_and_fit_keeps_right_margin() {
        let _ = switch_branch_state("main", true);
        m::set("omnigraph-pulse-cursor", json!({ "value": "cur-main-42" }));
        assert_eq!(
            m::text(&m::data("omnigraph-pulse-cursor"), "value"),
            "cur-main-42"
        );

        assert!(switch_branch_state("session/feature-1", true));
        assert_eq!(m::text(&ui_state(), "branch"), "session/feature-1");
        assert_eq!(m::text(&m::data("omnigraph-pulse-cursor"), "value"), "");

        // Same branch switch keeps existing cursor; distinct branch resets it again.
        m::set("omnigraph-pulse-cursor", json!({ "value": "cur-feat-7" }));
        assert!(!switch_branch_state("session/feature-1", false));
        assert_eq!(
            m::text(&m::data("omnigraph-pulse-cursor"), "value"),
            "cur-feat-7"
        );
        focus_branch("main");
        assert_eq!(m::text(&ui_state(), "branch"), "main");
        assert_eq!(m::text(&m::data("omnigraph-pulse-cursor"), "value"), "");

        let (scale, px, py) = camera_for_fit((1040.0, 540.0), (800.0, 500.0));
        assert!(px >= 36.0);
        assert!(py >= 24.0);
        assert!(px + 1040.0 * scale <= 800.0 - 35.0);
    }
}
