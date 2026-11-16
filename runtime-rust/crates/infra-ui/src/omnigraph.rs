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

#[cfg(test)]
pub fn session_payload_from_fields(
    enabled: bool,
    branch_mode: &str,
    existing_branch: &str,
    pull_raw: &str,
    query: &str,
    auto_publish: bool,
    auto_merge: bool,
) -> Option<V> {
    session_payload_with_links(
        enabled,
        branch_mode,
        existing_branch,
        pull_raw,
        query,
        auto_publish,
        auto_merge,
        &[],
        "",
    )
}

#[allow(clippy::too_many_arguments)]
pub fn session_payload_with_links(
    enabled: bool,
    branch_mode: &str,
    existing_branch: &str,
    pull_raw: &str,
    query: &str,
    auto_publish: bool,
    auto_merge: bool,
    linked_session_ids: &[String],
    shared_space: &str,
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
    let mut seen_links = BTreeSet::new();
    let links: Vec<String> = linked_session_ids
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && seen_links.insert((*s).to_owned()))
        .map(str::to_owned)
        .collect();
    if !links.is_empty() {
        payload["linked_session_ids"] = json!(links);
    }
    let space = shared_space.trim();
    if !space.is_empty() {
        payload["shared_space"] = json!(resolved_space_branch(space));
    }
    Some(payload)
}

pub fn percent_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn export_request_body(
    project_id: &str,
    branch: &str,
    session_id: &str,
    format: &str,
    scope: &str,
    include_inherited: bool,
) -> V {
    let norm_format = if format == "ndjson" {
        "ndjson"
    } else {
        "bundle"
    };
    let norm_scope = match scope {
        "task" | "branch" | "all" => scope,
        _ => "branch",
    };
    let mut body = json!({
        "project_id": project_id.trim(),
        "format": norm_format,
        "scope": norm_scope,
        "include_inherited": include_inherited,
    });
    let clean_branch = branch.trim();
    if !clean_branch.is_empty() && clean_branch != "auto" {
        body["branch"] = json!(clean_branch);
    }
    let clean_session = session_id.trim();
    if norm_scope == "task" && !clean_session.is_empty() {
        body["session_id"] = json!(clean_session);
    }
    body
}

pub fn export_download_url(
    project_id: &str,
    branch: &str,
    session_id: &str,
    format: &str,
    scope: &str,
    include_inherited: bool,
) -> String {
    let norm_format = if format == "ndjson" {
        "ndjson"
    } else {
        "bundle"
    };
    let norm_scope = match scope {
        "task" | "branch" | "all" => scope,
        _ => "branch",
    };
    let mut pairs = vec![
        ("project_id", project_id.trim().to_owned()),
        ("format", norm_format.to_owned()),
        ("scope", norm_scope.to_owned()),
        (
            "include_inherited",
            if include_inherited { "true" } else { "false" }.to_owned(),
        ),
        ("download", "1".to_owned()),
    ];
    let clean_branch = branch.trim();
    if !clean_branch.is_empty() && clean_branch != "auto" {
        pairs.push(("branch", clean_branch.to_owned()));
    }
    let clean_session = session_id.trim();
    if norm_scope == "task" && !clean_session.is_empty() {
        pairs.push(("session_id", clean_session.to_owned()));
    }
    let query = pairs
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(&v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("/api/omnigraph/export?{query}")
}

pub fn export_preview_summary(res: &V, requested_format: &str) -> (String, String, String, String) {
    let counts = &res["counts"];
    let atoms = counts["atoms"]
        .as_u64()
        .unwrap_or_else(|| m::array(&res["bundle"]["atoms"]).len() as u64);
    let files = counts["files"]
        .as_u64()
        .unwrap_or_else(|| m::array(&res["bundle"]["files"]).len() as u64);
    let edges = counts["edges"]
        .as_u64()
        .unwrap_or_else(|| m::array(&res["bundle"]["edges"]).len() as u64);
    let sessions = counts["sessions"]
        .as_u64()
        .unwrap_or_else(|| m::array(&res["bundle"]["sessions"]).len() as u64);
    let mut summary = format!("{atoms} atoms · {files} files · {edges} edges");
    if sessions > 0 {
        summary.push_str(&format!(" · {sessions} sessions"));
    }
    let default_name = if requested_format == "ndjson" {
        "omnigraph-context.ndjson"
    } else {
        "omnigraph-context.json"
    };
    let filename = m::text(res, "filename");
    let filename = if filename.is_empty() {
        default_name.to_owned()
    } else {
        filename
    };
    let branch = m::text(res, "source_branch");
    let branch = if branch.is_empty() {
        m::text(&res["bundle"], "source_branch")
    } else {
        branch
    };
    let branch = if branch.is_empty() {
        "main".to_owned()
    } else {
        branch
    };
    let preview = if requested_format == "ndjson" {
        res["ndjson"]
            .as_str()
            .or_else(|| res["bundle"]["ndjson"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| serde_json::to_string_pretty(res).unwrap_or_default())
    } else if res["bundle"].is_object() {
        serde_json::to_string_pretty(&res["bundle"]).unwrap_or_default()
    } else if let Some(s) = res["bundle"].as_str() {
        s.to_owned()
    } else {
        serde_json::to_string_pretty(res).unwrap_or_default()
    };
    (summary, filename, branch, preview)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportFormat {
    Empty,
    Bundle,
    Ndjson,
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedImportPreview {
    pub format: ImportFormat,
    pub atoms: usize,
    pub files: usize,
    pub edges: usize,
    pub sessions: usize,
    pub source_branch: String,
    pub bundle: Option<V>,
    pub raw_text: Option<String>,
    pub error: String,
}

fn is_ndjson_record_obj(obj: &serde_json::Map<String, V>) -> bool {
    let has_bundle_collections = obj.contains_key("atoms")
        || obj.contains_key("edges")
        || obj.contains_key("files")
        || obj.contains_key("bundle");
    !has_bundle_collections
        && (obj.contains_key("type")
            || obj.contains_key("entity_kind")
            || obj.contains_key("op")
            || obj.contains_key("edge")
            || (obj.contains_key("from") && obj.contains_key("to"))
            || (obj.contains_key("source") && obj.contains_key("target")))
}

fn count_ndjson_record(
    val: &V,
    atoms: &mut usize,
    files: &mut usize,
    edges: &mut usize,
    sessions: &mut usize,
    source_branch: &mut String,
) {
    let raw_type = val["type"].as_str().unwrap_or("");
    let kind = if raw_type.is_empty() || raw_type == "node" {
        val["entity_kind"]
            .as_str()
            .or_else(|| val["type_name"].as_str())
            .or_else(|| val["edge"].as_str())
            .unwrap_or(raw_type)
    } else {
        raw_type
    };
    match kind {
        "ContextAtom" | "context_atom" | "atom" => *atoms += 1,
        "FileRef" | "file_ref" | "file" => *files += 1,
        "Session" | "session" => *sessions += 1,
        "SymbolRef" | "symbol_ref" | "symbol" | "TurnSnapshot" | "turn_snapshot" | "turn"
        | "Project" | "project" | "Branch" | "branch" | "Commit" | "commit" => {}
        "PublishedBy" | "published_by" | "ConsumedBy" | "consumed_by" | "TouchesFile"
        | "touches_file" | "MentionsSymbol" | "mentions_symbol" | "RelatesTo" | "relates_to"
        | "DerivedFrom" | "derived_from" | "SessionFollows" | "session_follows" | "edge" => {
            *edges += 1
        }
        _ => {
            if val["edge"].is_string()
                || val["from"].is_string()
                || (val["source"].is_string() && val["target"].is_string())
                || val["src"].is_string()
                || val["endpoints"].is_object()
            {
                *edges += 1;
            } else if val["path"].is_string() && !val["title"].is_string() {
                *files += 1;
            } else if val["title"].is_string() || val["summary"].is_string() {
                *atoms += 1;
            }
        }
    }
    if source_branch.is_empty() {
        for container in [val, &val["data"], &val["properties"]] {
            if !container.is_object() {
                continue;
            }
            for key in ["source_branch", "branch", "graph_branch"] {
                if let Some(b) = container[key]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    *source_branch = b.to_owned();
                    return;
                }
            }
        }
    }
}

pub fn inspect_import_payload(raw_text: &str) -> ParsedImportPreview {
    let trimmed = raw_text.trim();
    if trimmed.is_empty() {
        return ParsedImportPreview {
            format: ImportFormat::Empty,
            atoms: 0,
            files: 0,
            edges: 0,
            sessions: 0,
            source_branch: String::new(),
            bundle: None,
            raw_text: None,
            error: String::new(),
        };
    }

    if let Ok(parsed) = serde_json::from_str::<V>(trimmed) {
        if let Some(obj) = parsed.as_object() {
            if is_ndjson_record_obj(obj) {
                let mut atoms = 0;
                let mut files = 0;
                let mut edges = 0;
                let mut sessions = 0;
                let mut source_branch = String::new();
                count_ndjson_record(
                    &parsed,
                    &mut atoms,
                    &mut files,
                    &mut edges,
                    &mut sessions,
                    &mut source_branch,
                );
                return ParsedImportPreview {
                    format: ImportFormat::Ndjson,
                    atoms,
                    files,
                    edges,
                    sessions,
                    source_branch,
                    bundle: None,
                    raw_text: Some(trimmed.to_owned()),
                    error: String::new(),
                };
            }
            let bundle_val = if obj.get("bundle").is_some_and(V::is_object) {
                obj["bundle"].clone()
            } else {
                parsed.clone()
            };
            let is_bundle = bundle_val["format"]
                .as_str()
                .is_some_and(|s| s.starts_with("harness.omnigraph"))
                || bundle_val["atoms"].is_array()
                || bundle_val["files"].is_array()
                || bundle_val["edges"].is_array()
                || bundle_val["sessions"].is_array();
            if is_bundle {
                let source_branch = bundle_val["source_branch"]
                    .as_str()
                    .or_else(|| bundle_val["branch"].as_str())
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                return ParsedImportPreview {
                    format: ImportFormat::Bundle,
                    atoms: m::array(&bundle_val["atoms"]).len(),
                    files: m::array(&bundle_val["files"]).len(),
                    edges: m::array(&bundle_val["edges"]).len(),
                    sessions: m::array(&bundle_val["sessions"]).len(),
                    source_branch,
                    bundle: Some(bundle_val),
                    raw_text: None,
                    error: String::new(),
                };
            }
            return ParsedImportPreview {
                format: ImportFormat::Invalid,
                atoms: 0,
                files: 0,
                edges: 0,
                sessions: 0,
                source_branch: String::new(),
                bundle: None,
                raw_text: None,
                error:
                    "JSON object lacks Omnigraph bundle fields (atoms, files, edges, or format)."
                        .into(),
            };
        }
    }

    let mut atoms = 0;
    let mut files = 0;
    let mut edges = 0;
    let mut sessions = 0;
    let mut source_branch = String::new();
    let mut records = 0;
    for (idx, line) in trimmed.lines().enumerate() {
        let line_trimmed = line.trim();
        if line_trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<V>(line_trimmed) {
            Ok(val) if val.is_object() => {
                records += 1;
                count_ndjson_record(
                    &val,
                    &mut atoms,
                    &mut files,
                    &mut edges,
                    &mut sessions,
                    &mut source_branch,
                );
            }
            _ => {
                return ParsedImportPreview {
                    format: ImportFormat::Invalid,
                    atoms: 0,
                    files: 0,
                    edges: 0,
                    sessions: 0,
                    source_branch: String::new(),
                    bundle: None,
                    raw_text: None,
                    error: format!("Line {}: expected a JSON object.", idx + 1),
                };
            }
        }
    }
    if records == 0 {
        return ParsedImportPreview {
            format: ImportFormat::Empty,
            atoms: 0,
            files: 0,
            edges: 0,
            sessions: 0,
            source_branch: String::new(),
            bundle: None,
            raw_text: None,
            error: String::new(),
        };
    }
    ParsedImportPreview {
        format: ImportFormat::Ndjson,
        atoms,
        files,
        edges,
        sessions,
        source_branch,
        bundle: None,
        raw_text: Some(trimmed.to_owned()),
        error: String::new(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn import_request_body(
    project_id: &str,
    raw_text: &str,
    target_branch: &str,
    from_branch: &str,
    create_branch: bool,
    mode: &str,
    inject_into_sessions: bool,
    target_session_id: &str,
) -> Result<V, String> {
    let preview = inspect_import_payload(raw_text);
    match preview.format {
        ImportFormat::Empty => {
            return Err("Paste or choose an Omnigraph bundle or NDJSON file first.".into())
        }
        ImportFormat::Invalid => return Err(preview.error),
        ImportFormat::Bundle | ImportFormat::Ndjson => {}
    }
    let clean_mode = if mode == "replace" {
        "replace"
    } else {
        "merge"
    };
    let clean_branch = target_branch.trim();
    let resolved_branch = if clean_branch.is_empty() {
        "main"
    } else {
        clean_branch
    };
    let mut body = json!({
        "project_id": project_id.trim(),
        "target_branch": resolved_branch,
        "create_branch": create_branch,
        "mode": clean_mode,
        "inject_into_sessions": inject_into_sessions,
    });
    let clean_from = from_branch.trim();
    if !clean_from.is_empty() {
        body["from_branch"] = json!(clean_from);
    }
    if let Some(bundle) = preview.bundle {
        body["bundle"] = bundle;
    } else if let Some(raw) = preview.raw_text {
        body["raw_text"] = json!(raw);
    }
    let sid = target_session_id.trim();
    if inject_into_sessions && !sid.is_empty() {
        body["session_id"] = json!(sid);
        body["target_session_ids"] = json!([sid]);
    }
    Ok(body)
}

pub fn link_sessions_request_body(
    project_id: &str,
    session_ids: &[String],
    mode: &str,
    source_session_id: &str,
    space_name: &str,
    target_branch: &str,
) -> Result<V, String> {
    let clean_mode = if mode == "broadcast" {
        "broadcast"
    } else {
        "shared_branch"
    };
    let mut seen = BTreeSet::new();
    let ids: Vec<String> = session_ids
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && seen.insert((*s).to_owned()))
        .map(str::to_owned)
        .collect();
    let src = source_session_id.trim();
    if clean_mode == "broadcast" {
        if src.is_empty() {
            return Err("Choose a source task to broadcast context from.".into());
        }
        let target_count = ids.iter().filter(|id| id.as_str() != src).count();
        if target_count == 0 {
            return Err("Select at least one target task to receive the broadcast.".into());
        }
    } else if ids.len() < 2 {
        return Err("Select at least 2 tasks to create a shared Omnigraph context space.".into());
    }
    let mut body = json!({
        "project_id": project_id.trim(),
        "session_ids": ids,
        "mode": clean_mode,
        "inject_live": true,
    });
    if clean_mode == "broadcast" && !src.is_empty() {
        body["source_session_id"] = json!(src);
    }
    let space = space_name.trim();
    if !space.is_empty() {
        body["space_name"] = json!(space);
    }
    let branch = target_branch.trim();
    if !branch.is_empty() && branch != "auto" {
        body["target_branch"] = json!(branch);
    }
    Ok(body)
}

pub fn sync_sessions_from_response(res: &V) {
    let list = m::array(&res["sessions"]);
    if !list.is_empty() {
        m::upsert_sessions(&list);
    }
    if res["session"].is_object() {
        m::upsert(res["session"].clone());
    }
}

fn session_atom_count(session: &V) -> usize {
    let omni = &session["omnigraph"];
    if let Some(n) = omni["atom_count"].as_u64() {
        return n as usize;
    }
    let mut seen = BTreeSet::new();
    for item in m::array(&omni["injected_atoms"])
        .into_iter()
        .chain(m::array(&omni["published_atoms"]))
        .chain(m::array(&omni["shared_atoms"]))
        .chain(m::array(&omni["atoms"]))
    {
        if let Some(id) = item
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| item["id"].as_str().filter(|s| !s.is_empty()))
            .or_else(|| item["atom_id"].as_str().filter(|s| !s.is_empty()))
        {
            seen.insert(id.to_owned());
        }
    }
    seen.len()
}

pub fn render_session_peers_html(session: &V, all_sessions: &[V]) -> (bool, String) {
    let sid = m::text(session, "id");
    let space = m::omnigraph_shared_space(session);
    let peers: Vec<String> = m::omnigraph_linked_session_ids(session)
        .into_iter()
        .filter(|id| id != &sid)
        .collect();
    if space.is_empty() && peers.is_empty() {
        return (false, String::new());
    }
    let mut html = String::new();
    if !space.is_empty() {
        html.push_str(&format!(
            "<span class=\"omni-badge\" data-kind=\"space\" title=\"Shared Omnigraph context space\"><span aria-hidden=\"true\">⇄</span> Space: {}</span>",
            esc(&space)
        ));
    }
    if !peers.is_empty() {
        html.push_str("<span class=\"omni-peers-label\">Linked tasks:</span>");
        for peer_id in peers {
            let peer = all_sessions.iter().find(|s| m::text(s, "id") == peer_id);
            let title = peer
                .map(m::summary)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| peer_id.chars().take(12).collect());
            let status = peer
                .map(|s| m::text(s, "status"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "idle".to_owned());
            let branch = peer
                .map(m::omnigraph_branch)
                .unwrap_or_else(|| "main".to_owned());
            html.push_str(&format!(
                "<button type=\"button\" class=\"omni-peer-chip\" data-action=\"omnigraph-open-session\" data-session-id=\"{}\" title=\"Open linked task {}\"><span class=\"omni-peer-dot\" data-status=\"{}\" aria-hidden=\"true\"></span><span class=\"omni-peer-title\">{}</span><span class=\"omni-peer-branch\">{}</span></button>",
                esc(&peer_id),
                esc(&title),
                esc(&status),
                esc(&title),
                esc(&branch),
            ));
        }
    }
    (true, html)
}

fn eligible_panel_peers<'a>(
    sessions: &'a [V],
    current_sid: &str,
    current_project: &str,
) -> Vec<&'a V> {
    sessions
        .iter()
        .filter(|s| {
            let id = m::text(s, "id");
            let proj = m::text(s, "project_id");
            !id.is_empty()
                && id != current_sid
                && (current_project.is_empty() || proj.is_empty() || proj == current_project)
        })
        .take(24)
        .collect()
}

fn panel_peer_list_key(peers: &[&V]) -> String {
    peers
        .iter()
        .map(|s| {
            format!(
                "{}|{}|{}|{}|{}",
                m::text(s, "id"),
                m::summary(s),
                s["status"].as_str().unwrap_or("idle"),
                m::omnigraph_branch(s),
                session_atom_count(s),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_panel_peer_checklist_html(
    sessions: &[V],
    current_sid: &str,
    current_project: &str,
    selected_ids: &BTreeSet<String>,
    disabled: bool,
) -> (bool, usize, String) {
    let eligible = eligible_panel_peers(sessions, current_sid, current_project);
    if eligible.is_empty() {
        return (false, 0, String::new());
    }
    let mut checked_count = 0;
    let mut html = String::new();
    for s in eligible {
        let id = m::text(s, "id");
        let checked = selected_ids.contains(&id);
        if checked {
            checked_count += 1;
        }
        let title = {
            let t = m::summary(s);
            if t.is_empty() {
                id.clone()
            } else {
                t
            }
        };
        let status = m::label(s["status"].as_str().unwrap_or("idle"));
        let branch = m::omnigraph_branch(s);
        let atoms = session_atom_count(s);
        html.push_str(&format!(
            "<label class=\"omni-peer-option\"><span class=\"omni-peer-option-top\"><input type=\"checkbox\" data-action=\"omnigraph-peer-choice\" value=\"{}\"{}{}><strong>{}</strong></span><span class=\"omni-peer-option-meta\"><span class=\"omni-branch-pill\">{}</span><span>{} · {} atoms</span></span></label>",
            esc(&id),
            if checked { " checked" } else { "" },
            if disabled { " disabled" } else { "" },
            esc(&title),
            esc(&branch),
            esc(&status),
            atoms,
        ));
    }
    (true, checked_count, html)
}

fn share_session_matches_filter(s: &V, needle: &str) -> bool {
    let id = m::text(s, "id");
    if id.is_empty() {
        return false;
    }
    if needle.is_empty() {
        return true;
    }
    let hay = format!(
        "{id} {} {} {} {}",
        m::summary(s),
        m::text(s, "provider"),
        m::omnigraph_branch(s),
        m::omnigraph_shared_space(s)
    )
    .to_ascii_lowercase();
    hay.contains(needle)
}

fn filtered_share_sessions<'a>(sessions: &'a [V], filter: &str) -> Vec<&'a V> {
    let needle = filter.trim().to_ascii_lowercase();
    sessions
        .iter()
        .filter(|s| share_session_matches_filter(s, &needle))
        .collect()
}

fn share_task_list_key(filtered: &[&V]) -> String {
    filtered
        .iter()
        .map(|s| {
            format!(
                "{}|{}|{}|{}|{}|{}|{}",
                m::text(s, "id"),
                m::summary(s),
                m::text(s, "provider"),
                s["status"].as_str().unwrap_or("idle"),
                m::omnigraph_branch(s),
                session_atom_count(s),
                m::omnigraph_shared_space(s),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_share_task_list_html(
    sessions: &[V],
    selected_ids: &BTreeSet<String>,
    mode: &str,
    source_id: &str,
    filter: &str,
) -> String {
    let filtered = filtered_share_sessions(sessions, filter);
    if filtered.is_empty() {
        return "<p class=\"empty-note\">No matching tasks found in this project.</p>".into();
    }
    let mut html = String::new();
    for s in filtered {
        let id = m::text(s, "id");
        let is_selected = selected_ids.contains(&id);
        let is_source = mode == "broadcast" && !source_id.is_empty() && id == source_id;
        let title = {
            let t = m::summary(s);
            if t.is_empty() {
                id.clone()
            } else {
                t
            }
        };
        let provider = m::text(s, "provider");
        let status_raw = s["status"].as_str().unwrap_or("idle");
        let status_label = m::label(status_raw);
        let branch = m::omnigraph_branch(s);
        let atoms = session_atom_count(s);
        let space = m::omnigraph_shared_space(s);
        html.push_str(&format!(
            "<label class=\"omni-share-task-card\" data-selected=\"{}\" data-source=\"{}\"><span class=\"omni-share-task-top\"><input type=\"checkbox\" data-action=\"omni-share-task-check\" value=\"{}\"{}><span class=\"omni-share-task-title\">{}</span>{}</span><span class=\"omni-share-task-meta\"><span class=\"omni-peer-dot\" data-status=\"{}\" aria-hidden=\"true\"></span><span>{}</span><span class=\"omni-branch-pill\">{}</span><span class=\"omni-meta-pill\">{atoms} atoms</span>{}</span></label>",
            if is_selected { "true" } else { "false" },
            if is_source { "true" } else { "false" },
            esc(&id),
            if is_selected { " checked" } else { "" },
            esc(&title),
            if provider.is_empty() {
                String::new()
            } else {
                format!("<span class=\"omni-provider-badge\">{}</span>", esc(&provider))
            },
            esc(status_raw),
            esc(&status_label),
            esc(&branch),
            if space.is_empty() {
                String::new()
            } else {
                format!("<span class=\"omni-space-pill\">⇄ {}</span>", esc(&space))
            },
        ));
    }
    html
}

fn slugify_space_segment(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if ch == '/' {
            if !out.is_empty() && !out.ends_with('/') {
                if out.ends_with('-') {
                    out.pop();
                }
                out.push('/');
            }
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() && !out.ends_with('/') {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') || out.ends_with('/') {
        out.pop();
    }
    out
}

fn resolved_space_branch(space_name: &str) -> String {
    let trimmed = space_name.trim().trim_matches('/');
    let rest = trimmed
        .strip_prefix("shared/")
        .or_else(|| trimmed.strip_prefix("SHARED/"))
        .unwrap_or(trimmed);
    let slug = slugify_space_segment(rest);
    if slug.is_empty() {
        "shared/context".to_owned()
    } else {
        format!("shared/{slug}")
    }
}

pub fn share_summary_text(
    sessions: &[V],
    selected_ids: &BTreeSet<String>,
    mode: &str,
    source_id: &str,
    space_name: &str,
) -> (bool, String) {
    let chosen: Vec<&V> = sessions
        .iter()
        .filter(|s| selected_ids.contains(&m::text(s, "id")))
        .collect();
    if mode == "broadcast" {
        if source_id.is_empty() {
            return (
                false,
                "Choose a source task to broadcast context from.".into(),
            );
        }
        let target_count = chosen
            .iter()
            .filter(|s| m::text(s, "id") != source_id)
            .count();
        if target_count == 0 {
            return (
                false,
                "Check at least 1 target task to receive the broadcast.".into(),
            );
        }
        let src_title = sessions
            .iter()
            .find(|s| m::text(s, "id") == source_id)
            .map(m::summary)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| source_id.to_owned());
        return (
            true,
            format!("Broadcast context from \"{src_title}\" → {target_count} target task(s)."),
        );
    }
    if chosen.len() < 2 {
        return (
            false,
            format!(
                "{} task selected — check at least 2 tasks to link in a shared Omnigraph space.",
                chosen.len()
            ),
        );
    }
    let space_branch = resolved_space_branch(space_name);
    let branches: Vec<String> = chosen
        .iter()
        .map(|s| m::omnigraph_branch(s))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    (
        true,
        format!(
            "{} tasks · branches {} ⇄ {space_branch}",
            chosen.len(),
            branches.join(", ")
        ),
    )
}

pub fn render_share_topology_html(
    sessions: &[V],
    selected_ids: &BTreeSet<String>,
    mode: &str,
    source_id: &str,
    space_name: &str,
) -> String {
    let selected: Vec<&V> = sessions
        .iter()
        .filter(|s| selected_ids.contains(&m::text(s, "id")))
        .collect();

    if mode == "broadcast" {
        let src_sess = sessions.iter().find(|s| m::text(s, "id") == source_id);
        let targets: Vec<&V> = selected
            .into_iter()
            .filter(|s| m::text(s, "id") != source_id)
            .collect();
        if src_sess.is_none() || targets.is_empty() {
            return "<p class=\"omni-topo-empty\">Select a source task and at least 1 target task below to preview the broadcast context flow.</p>".into();
        }
        let src = src_sess.expect("source checked");
        let src_title = m::summary(src);
        let src_branch = m::omnigraph_branch(src);
        let src_atoms = session_atom_count(src);
        let visible_targets: Vec<&V> = targets.iter().copied().take(5).collect();
        let extra = targets.len().saturating_sub(visible_targets.len());
        let mut svg_lines = String::new();
        let n = visible_targets.len().max(1);
        for idx in 0..n {
            let y_pct = ((idx as f64 + 0.5) / (n as f64)) * 140.0 + 10.0;
            svg_lines.push_str(&format!(
                "<path class=\"omni-topo-line\" data-dir=\"broadcast\" d=\"M 176 80 C 260 80, 360 {y_pct:.1}, 456 {y_pct:.1}\" marker-end=\"url(#omni-topo-arrow-broadcast)\"/>"
            ));
        }
        let mut target_cards = String::new();
        for t in visible_targets {
            let title = m::summary(t);
            let branch = m::omnigraph_branch(t);
            let atoms = session_atom_count(t);
            target_cards.push_str(&format!(
                "<div class=\"omni-topo-card\"><span class=\"omni-topo-card-title\">{}</span><span class=\"omni-topo-card-meta\">{} · {atoms} atoms</span></div>",
                esc(&title),
                esc(&branch),
            ));
        }
        if extra > 0 {
            target_cards.push_str(&format!(
                "<span class=\"omni-meta-pill\">+{extra} more tasks</span>"
            ));
        }
        return format!(
            "<div class=\"omni-topo-wrap\" role=\"img\" aria-label=\"Broadcast topology from {} to {} tasks\"><div class=\"omni-topo-bar\"><span><strong>→ Broadcast mode</strong> · pushes source findings live</span><span class=\"omni-meta-pill\">{} target(s)</span></div><div class=\"omni-topo-stage\"><svg class=\"omni-topo-svg\" viewBox=\"0 0 640 160\" preserveAspectRatio=\"none\" aria-hidden=\"true\"><defs><marker id=\"omni-topo-arrow-broadcast\" class=\"omni-topo-arrow\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto\"><path d=\"M0 1 L8 4 L0 7 Z\"/></marker></defs>{svg_lines}</svg><div class=\"omni-topo-grid\"><div class=\"omni-topo-col\"><div class=\"omni-topo-card\" data-role=\"source\"><span class=\"omni-badge\" data-kind=\"contract\">Source</span><span class=\"omni-topo-card-title\">{}</span><span class=\"omni-topo-card-meta\">{} · {src_atoms} atoms</span></div></div><div class=\"omni-topo-hub\"><span class=\"omni-topo-hub-title\">→ Fan-out</span><span class=\"omni-topo-hub-branch\">SessionFollows + ConsumedBy</span></div><div class=\"omni-topo-col\">{target_cards}</div></div></div></div>",
            esc(&src_title),
            targets.len(),
            targets.len(),
            esc(&src_title),
            esc(&src_branch),
        );
    }

    if selected.len() < 2 {
        return "<p class=\"omni-topo-empty\">Check 2 or more tasks below to visualize their live mutual Omnigraph context hub.</p>".into();
    }

    let space_branch = resolved_space_branch(space_name);
    let total_atoms: usize = selected.iter().map(|s| session_atom_count(s)).sum();
    let visible: Vec<&V> = selected.iter().copied().take(6).collect();
    let extra = selected.len().saturating_sub(visible.len());
    let mut left = Vec::new();
    let mut right = Vec::new();
    for (idx, sess) in visible.into_iter().enumerate() {
        if idx % 2 == 0 {
            left.push(sess);
        } else {
            right.push(sess);
        }
    }
    let mut svg_lines = String::new();
    let ln = left.len().max(1);
    for idx in 0..left.len() {
        let y = ((idx as f64 + 0.5) / (ln as f64)) * 140.0 + 10.0;
        svg_lines.push_str(&format!(
            "<path class=\"omni-topo-line\" data-dir=\"mutual\" d=\"M 176 {y:.1} C 236 {y:.1}, 256 80, 290 80\" marker-end=\"url(#omni-topo-arrow-mutual)\"/>"
        ));
    }
    let rn = right.len().max(1);
    for idx in 0..right.len() {
        let y = ((idx as f64 + 0.5) / (rn as f64)) * 140.0 + 10.0;
        svg_lines.push_str(&format!(
            "<path class=\"omni-topo-line\" data-dir=\"mutual\" d=\"M 350 80 C 384 80, 404 {y:.1}, 464 {y:.1}\" marker-end=\"url(#omni-topo-arrow-mutual)\"/>"
        ));
    }
    let render_col = |list: Vec<&V>| -> String {
        list.into_iter()
            .map(|s| {
                let title = m::summary(s);
                let branch = m::omnigraph_branch(s);
                let atoms = session_atom_count(s);
                format!(
                    "<div class=\"omni-topo-card\"><span class=\"omni-topo-card-title\">{}</span><span class=\"omni-topo-card-meta\">{} · {atoms} atoms</span></div>",
                    esc(&title),
                    esc(&branch),
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };
    let mut right_html = render_col(right);
    if extra > 0 {
        right_html.push_str(&format!(
            "<span class=\"omni-meta-pill\">+{extra} more tasks</span>"
        ));
    }
    format!(
        "<div class=\"omni-topo-wrap\" role=\"img\" aria-label=\"Mutual shared space topology across {} tasks\"><div class=\"omni-topo-bar\"><span><strong>⇄ Mutual Shared Space</strong> · bidirectional branch sync</span><span class=\"omni-meta-pill\">{} tasks · {total_atoms} atoms</span></div><div class=\"omni-topo-stage\"><svg class=\"omni-topo-svg\" viewBox=\"0 0 640 160\" preserveAspectRatio=\"none\" aria-hidden=\"true\"><defs><marker id=\"omni-topo-arrow-mutual\" class=\"omni-topo-arrow-mutual\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto\"><path d=\"M0 1 L8 4 L0 7 Z\"/></marker></defs>{svg_lines}</svg><div class=\"omni-topo-grid\"><div class=\"omni-topo-col\">{}</div><div class=\"omni-topo-hub\"><span class=\"omni-topo-hub-title\">⇄ Shared Hub</span><span class=\"omni-topo-hub-branch\">{}</span></div><div class=\"omni-topo-col\">{right_html}</div></div></div></div>",
        selected.len(),
        selected.len(),
        render_col(left),
        esc(&space_branch),
    )
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
    let space = session["shared_space"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            session["omnigraph"]["shared_space"]
                .as_str()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("");
    let mut html = format!(
        "<article class=\"omni-inspector-card\"><div class=\"omni-node-head\"><span class=\"omni-badge\" data-kind=\"session\"><span aria-hidden=\"true\">◎</span> Session</span><span class=\"omni-badge\" data-kind=\"branch\">{}</span>{}</div><h3 class=\"omni-inspector-title\">{}</h3><p class=\"knowledge-note\">ID: <code>{}</code>{}</p>",
        esc(branch),
        if space.is_empty() {
            String::new()
        } else {
            format!(
                "<span class=\"omni-badge\" data-kind=\"space\"><span aria-hidden=\"true\">⇄</span> {}</span>",
                esc(space)
            )
        },
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
    attach_transfer_dropzone_listeners();
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
    dom::disabled("omnigraph-export-open", !connected);
    dom::disabled("omnigraph-import-open", !connected);

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

fn selected_peer_set(key: &str) -> BTreeSet<String> {
    m::array(&m::data(key))
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

pub fn clear_draft_peers() {
    m::set("omnigraph-draft-peers", json!([]));
    m::set("omnigraph-session-peers", json!([]));
}

pub fn sync_peer_draft_from_dom() {
    let checked = dom::selected_checks("omnigraph-peer-sessions");
    m::set("omnigraph-draft-peers", V::Array(checked));
}

pub fn apply_session_settings(session: &V) {
    let omni = &session["omnigraph"];
    if !omni.is_object() {
        dom::set_checked("omnigraph-enabled", false);
        dom::set_value("omnigraph-branch-mode", "auto");
        dom::set_value("omnigraph-existing-branch", "main");
        dom::set_value("omnigraph-pull-branches", "");
        dom::set_value("omnigraph-session-query", "");
        dom::set_value("omnigraph-shared-space", "");
        dom::set_checked("omnigraph-auto-publish", true);
        dom::set_checked("omnigraph-auto-merge", false);
        m::set("omnigraph-session-peers", json!([]));
        return;
    }
    dom::set_checked("omnigraph-enabled", m::omnigraph_enabled(session));
    let branch = m::omnigraph_branch(session);
    if branch == "main" {
        dom::set_value("omnigraph-branch-mode", "main");
    } else if branch.starts_with("session/") || branch == "auto" {
        dom::set_value("omnigraph-branch-mode", "auto");
    } else {
        dom::set_value("omnigraph-branch-mode", "existing");
        dom::set_value("omnigraph-existing-branch", &branch);
    }
    let pulls: Vec<String> = m::array(&omni["pull_branches"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    dom::set_value("omnigraph-pull-branches", &pulls.join(", "));
    dom::set_value("omnigraph-session-query", &m::text(omni, "query"));
    dom::set_value(
        "omnigraph-shared-space",
        &m::omnigraph_shared_space(session),
    );
    dom::set_checked(
        "omnigraph-auto-publish",
        omni["auto_publish"].as_bool().unwrap_or(true),
    );
    dom::set_checked("omnigraph-auto-merge", m::yes(&omni["auto_merge"]));
    let linked: Vec<V> = m::omnigraph_linked_session_ids(session)
        .into_iter()
        .map(V::String)
        .collect();
    m::set("omnigraph-session-peers", V::Array(linked));
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
        "omnigraph-shared-space",
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

    let project = dom::value("project");
    let sessions = m::read(|s| s.sessions.clone());
    let selected_peers = if sid.is_empty() {
        selected_peer_set("omnigraph-draft-peers")
    } else {
        selected_peer_set("omnigraph-session-peers")
    };
    let (show_peers, peer_count, peer_html) =
        render_panel_peer_checklist_html(&sessions, &sid, &project, &selected_peers, disable_edit);
    dom::hidden("omnigraph-peer-sessions-wrap", !enabled || !show_peers);
    dom::text("omnigraph-peer-count", &format!("{peer_count} linked"));
    if let Some(host) = dom::get("omnigraph-peer-sessions") {
        let peers = eligible_panel_peers(&sessions, &sid, &project);
        let list_key = panel_peer_list_key(&peers);
        let inputs =
            dom::all("#omnigraph-peer-sessions input[data-action=\"omnigraph-peer-choice\"]");
        let same_shape = !list_key.is_empty()
            && host.get_attribute("data-list-key").as_deref() == Some(list_key.as_str())
            && inputs.len() == peers.len();
        if same_shape {
            for el in inputs {
                if let Ok(inp) = el.dyn_into::<web_sys::HtmlInputElement>() {
                    let id = inp.value();
                    inp.set_checked(selected_peers.contains(&id));
                    inp.set_disabled(disable_edit);
                }
            }
        } else {
            let _ = host.set_attribute("data-list-key", &list_key);
            host.set_inner_html(&peer_html);
        }
    }

    let summary = if !enabled {
        "Off".to_owned()
    } else if peer_count > 0 {
        format!("{peer_count} linked")
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
    let linked: Vec<String> = if m::read(|s| s.sid.is_empty()) {
        selected_peer_set("omnigraph-draft-peers")
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    session_payload_with_links(
        dom::checked("omnigraph-enabled"),
        &dom::value("omnigraph-branch-mode"),
        &dom::value("omnigraph-existing-branch"),
        &dom::value("omnigraph-pull-branches"),
        &dom::value("omnigraph-session-query"),
        dom::checked("omnigraph-auto-publish"),
        dom::checked("omnigraph-auto-merge"),
        &linked,
        &dom::value("omnigraph-shared-space"),
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
    let sid = m::text(session, "id");
    let connected = m::yes(&m::data("omnigraph-status")["connected"]);
    let show_ready_bar = !visible && !sid.is_empty() && connected;
    dom::hidden("session-omnigraph-bar", !visible && !show_ready_bar);
    if !visible && !show_ready_bar {
        dom::error("session-omnigraph-error", "");
        dom::hidden("session-omnigraph-peers", true);
        return;
    }
    if show_ready_bar {
        dom::attr("session-omnigraph-bar", "data-active", "false");
        dom::text("session-omnigraph-branch", "not linked");
        dom::text(
            "session-omnigraph-meta",
            "Omnigraph ready · share with tasks, export, or import context",
        );
        dom::error("session-omnigraph-error", "");
        dom::hidden("session-omnigraph-merge", true);
        dom::hidden("session-omnigraph-peers", true);
        dom::hidden("session-omnigraph-atoms", true);
        return;
    }
    dom::attr("session-omnigraph-bar", "data-active", "true");
    dom::hidden("session-omnigraph-atoms", false);
    dom::text("session-omnigraph-branch", &branch);
    dom::text("session-omnigraph-meta", &meta);
    dom::error("session-omnigraph-error", &session_bar_error(session));
    dom::hidden("session-omnigraph-merge", !can_merge);
    dom::disabled("session-omnigraph-merge", m::active(session));

    let all_sessions = m::read(|s| s.sessions.clone());
    let (show_peers, peers_html) = render_session_peers_html(session, &all_sessions);
    dom::hidden("session-omnigraph-peers", !show_peers);
    if let Some(peers_el) = dom::get("session-omnigraph-peers") {
        peers_el.set_inner_html(&peers_html);
    }

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
    let target = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    if let Some(el) = target.as_ref() {
        if el.get_attribute("role").as_deref() == Some("tab")
            && el
                .closest("#omnigraph-transfer-dialog")
                .ok()
                .flatten()
                .is_some()
        {
            let key = event.key();
            if ["ArrowLeft", "ArrowRight", "Home", "End"].contains(&key.as_str()) {
                let next = match (el.id().as_str(), key.as_str()) {
                    (_, "Home") => "export",
                    (_, "End") => "import",
                    ("omni-transfer-tab-export", "ArrowRight" | "ArrowLeft") => "import",
                    ("omni-transfer-tab-import", "ArrowRight" | "ArrowLeft") => "export",
                    _ => "export",
                };
                set_transfer_tab(next);
                dom::focus(&format!("omni-transfer-tab-{next}"));
                return true;
            }
        }
    }

    if m::read(|s| s.view != "omnigraph") {
        return false;
    }
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

fn attach_transfer_dropzone_listeners() {
    let Some(dropzone) = dom::get("omni-import-dropzone") else {
        return;
    };
    if dropzone.get_attribute("data-drop-bound").as_deref() == Some("true") {
        return;
    }
    let _ = dropzone.set_attribute("data-drop-bound", "true");
    for ev_name in ["dragenter", "dragover", "dragleave", "drop"] {
        let name = ev_name.to_owned();
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            event.prevent_default();
            event.stop_propagation();
            match name.as_str() {
                "dragenter" | "dragover" => {
                    dom::attr("omni-import-dropzone", "data-dragover", "true");
                }
                "dragleave" => {
                    dom::attr("omni-import-dropzone", "data-dragover", "false");
                }
                "drop" => {
                    dom::attr("omni-import-dropzone", "data-dragover", "false");
                    if let Ok(dt) = js_sys::Reflect::get(&event, &JsValue::from_str("dataTransfer"))
                    {
                        if let Ok(files_val) =
                            js_sys::Reflect::get(&dt, &JsValue::from_str("files"))
                        {
                            if let Ok(files) = files_val.dyn_into::<web_sys::FileList>() {
                                if let Some(file) = files.item(0) {
                                    spawn_local(async move {
                                        read_import_file(file).await;
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        });
        if dropzone
            .add_event_listener_with_callback(ev_name, cb.as_ref().unchecked_ref())
            .is_ok()
        {
            cb.forget();
        }
    }
}

async fn read_import_file(file: web_sys::File) {
    let name = file.name();
    let size = file.size() as u64;
    match JsFuture::from(file.array_buffer()).await {
        Ok(buf) => {
            let bytes = js_sys::Uint8Array::new(&buf).to_vec();
            match String::from_utf8(bytes) {
                Ok(text) => {
                    dom::text("omni-import-file-name", &format!("{name} ({size} bytes)"));
                    dom::set_value("omni-import-text", &text);
                    dom::set_value("omni-import-file", "");
                    update_import_preview();
                }
                Err(_) => {
                    dom::error(
                        "omni-import-error",
                        "Selected file is not valid UTF-8 JSON or NDJSON.",
                    );
                }
            }
        }
        Err(_) => {
            dom::error("omni-import-error", &format!("Could not read {name}."));
        }
    }
}

fn set_transfer_tab(tab: &str) {
    let active_tab = if tab == "import" { "import" } else { "export" };
    m::set("omnigraph-transfer-tab", json!(active_tab));
    for name in ["export", "import"] {
        let is_active = name == active_tab;
        dom::attr(
            &format!("omni-transfer-tab-{name}"),
            "aria-selected",
            if is_active { "true" } else { "false" },
        );
        dom::hidden(&format!("omni-transfer-pane-{name}"), !is_active);
    }
}

fn sync_export_scope_fields() {
    let scope = dom::value("omni-export-scope");
    dom::hidden("omni-export-session-field", scope != "task");
    dom::hidden("omni-export-branch-field", scope != "branch");
}

fn current_export_options() -> (String, String, String, String, String, bool) {
    let project = current_project_id();
    let scope = dom::value("omni-export-scope");
    let format = dom::value("omni-export-format");
    let session_id = dom::value("omni-export-session");
    let branch = if scope == "all" {
        "main".to_owned()
    } else if scope == "task" {
        m::read(|s| {
            s.sessions
                .iter()
                .chain(std::iter::once(&s.selected))
                .find(|sess| m::text(sess, "id") == session_id)
                .and_then(|sess| {
                    sess["omnigraph"]["branch"]
                        .as_str()
                        .map(str::trim)
                        .filter(|b| !b.is_empty() && *b != "auto")
                        .map(str::to_owned)
                })
                .unwrap_or_default()
        })
    } else {
        dom::value("omni-export-branch")
    };
    let include_inherited = dom::checked("omni-export-include-inherited");
    (
        project,
        branch,
        session_id,
        format,
        scope,
        include_inherited,
    )
}

async fn run_export_preview() -> Option<(String, String)> {
    dom::error("omni-export-error", "");
    dom::hidden("omni-export-status", true);
    let (project, branch, session_id, format, scope, include_inherited) = current_export_options();
    if scope == "task" && session_id.is_empty() {
        dom::error("omni-export-error", "Select a task / session to export.");
        return None;
    }
    let body = export_request_body(
        &project,
        &branch,
        &session_id,
        &format,
        &scope,
        include_inherited,
    );
    let key = body.to_string();
    dom::text("omni-export-preview-meta", "generating…");
    match api::post("/api/omnigraph/export", body).await {
        Ok(res) => {
            if res["ok"].as_bool() == Some(false) {
                let msg = res["error"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Export failed.");
                dom::text("omni-export-preview-meta", "export failed");
                dom::error("omni-export-error", msg);
                return None;
            }
            let (summary, filename, src_branch, preview) = export_preview_summary(&res, &format);
            dom::text("omni-export-summary", &summary);
            dom::text("omni-export-filename", &filename);
            dom::hidden("omni-export-filename", filename.is_empty());
            dom::text("omni-export-source-branch", &src_branch);
            dom::hidden("omni-export-source-branch", src_branch.is_empty());
            dom::text(
                "omni-export-preview-meta",
                &format!("{} · {} bytes", filename, preview.len()),
            );
            dom::text("omni-export-preview-code", &preview);
            m::set(
                "omnigraph-export-cache",
                json!({ "key": key, "filename": filename, "text": preview }),
            );
            Some((filename, preview))
        }
        Err(e) => {
            dom::text("omni-export-preview-meta", "export failed");
            dom::error("omni-export-error", &e.message);
            None
        }
    }
}

async fn trigger_export_download() {
    let (project, branch, session_id, format, scope, include_inherited) = current_export_options();
    if scope == "task" && session_id.is_empty() {
        dom::error("omni-export-error", "Select a task / session to export.");
        return;
    }
    let req_key = export_request_body(
        &project,
        &branch,
        &session_id,
        &format,
        &scope,
        include_inherited,
    )
    .to_string();
    let cached = m::data("omnigraph-export-cache");
    let filename = if cached["key"].as_str() == Some(req_key.as_str()) {
        m::text(&cached, "filename")
    } else if let Some((fname, _)) = run_export_preview().await {
        fname
    } else {
        return;
    };
    let url = export_download_url(
        &project,
        &branch,
        &session_id,
        &format,
        &scope,
        include_inherited,
    );
    let anchor = dom::el("a", "", "");
    let _ = anchor.set_attribute("href", &url);
    let _ = anchor.set_attribute("download", &filename);
    if let Some(body) = dom::document().body() {
        let _ = body.append_child(&anchor);
    }
    if let Some(html_el) = anchor.dyn_ref::<web_sys::HtmlElement>() {
        html_el.click();
    }
    anchor.remove();
    dom::text("omni-export-status", &format!("Downloading {filename}…"));
    dom::hidden("omni-export-status", false);
}

async fn copy_export_payload() {
    let (project, branch, session_id, format, scope, include_inherited) = current_export_options();
    if scope == "task" && session_id.is_empty() {
        dom::error("omni-export-error", "Select a task / session to export.");
        return;
    }
    let req_key = export_request_body(
        &project,
        &branch,
        &session_id,
        &format,
        &scope,
        include_inherited,
    )
    .to_string();
    let cached = m::data("omnigraph-export-cache");
    let text = if cached["key"].as_str() == Some(req_key.as_str()) {
        m::text(&cached, "text")
    } else if let Some((_, txt)) = run_export_preview().await {
        txt
    } else {
        return;
    };
    let _ = JsFuture::from(dom::window().navigator().clipboard().write_text(&text)).await;
    dom::text("omni-export-status", "Copied export payload to clipboard.");
    dom::hidden("omni-export-status", false);
}

fn update_import_preview() {
    dom::error("omni-import-error", "");
    let raw = dom::value("omni-import-text");
    let parsed = inspect_import_payload(&raw);
    match parsed.format {
        ImportFormat::Empty => {
            dom::attr("omni-import-preview", "data-state", "empty");
            dom::text("omni-import-format-pill", "Waiting for payload");
            dom::text(
                "omni-import-preview-text",
                "Drop a file or paste Omnigraph JSON / NDJSON to inspect its contents before importing.",
            );
            dom::disabled("omni-import-submit", true);
        }
        ImportFormat::Invalid => {
            dom::attr("omni-import-preview", "data-state", "error");
            dom::text("omni-import-format-pill", "Invalid payload");
            dom::text("omni-import-preview-text", &parsed.error);
            dom::disabled("omni-import-submit", true);
        }
        ImportFormat::Bundle | ImportFormat::Ndjson => {
            dom::attr("omni-import-preview", "data-state", "ready");
            dom::text(
                "omni-import-format-pill",
                if parsed.format == ImportFormat::Bundle {
                    "Bundle JSON"
                } else {
                    "Raw NDJSON"
                },
            );
            let mut details = format!(
                "{} atoms · {} files · {} edges",
                parsed.atoms, parsed.files, parsed.edges
            );
            if parsed.sessions > 0 {
                details.push_str(&format!(" · {} sessions", parsed.sessions));
            }
            if !parsed.source_branch.is_empty() {
                details.push_str(&format!(" · from {}", parsed.source_branch));
            }
            dom::text("omni-import-preview-text", &details);
            dom::disabled("omni-import-submit", false);
        }
    }
    let inject = dom::checked("omni-import-inject");
    dom::hidden("omni-import-session-field", !inject);
}

fn populate_transfer_selectors(preferred_session_id: Option<&str>, preferred_branch: Option<&str>) {
    let project = current_project_id();
    let sessions = m::read(|s| {
        s.sessions
            .iter()
            .filter(|sess| {
                let p = m::text(sess, "project_id");
                project.is_empty() || p.is_empty() || p == project
            })
            .cloned()
            .collect::<Vec<_>>()
    });
    let mut sess_items = vec![json!({ "id": "", "name": "No session selected" })];
    let mut seen_ids = BTreeSet::new();
    for s in &sessions {
        let id = m::text(s, "id");
        if !id.is_empty() && seen_ids.insert(id.clone()) {
            let label = m::summary(s);
            sess_items.push(json!({
                "id": id,
                "name": if label.is_empty() { id } else { label }
            }));
        }
    }
    let active_sid = preferred_session_id
        .map(str::to_owned)
        .unwrap_or_else(|| m::read(|s| s.sid.clone()));
    if !active_sid.is_empty() && !seen_ids.contains(&active_sid) {
        let selected_label = m::read(|s| {
            if m::text(&s.selected, "id") == active_sid {
                m::summary(&s.selected)
            } else {
                String::new()
            }
        });
        sess_items.push(json!({
            "id": active_sid.clone(),
            "name": if selected_label.is_empty() {
                active_sid.clone()
            } else {
                selected_label
            }
        }));
    }
    dom::options(
        "omni-export-session",
        &sess_items,
        m::summary,
        Some(&active_sid),
    );
    dom::options(
        "omni-import-session",
        &sess_items,
        m::summary,
        Some(&active_sid),
    );

    let graph = m::data("omnigraph-graph");
    let mut branches = branch_names_from_graph(&graph);
    if let Some(b) = preferred_branch
        .map(str::trim)
        .filter(|b| !b.is_empty() && *b != "auto")
    {
        if !branches.iter().any(|existing| existing == b) {
            branches.push(b.to_owned());
        }
    }
    let branch_items: Vec<V> = branches
        .iter()
        .map(|name| json!({ "id": name, "name": name }))
        .collect();
    let active_branch = preferred_branch
        .map(str::trim)
        .filter(|b| !b.is_empty() && *b != "auto")
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let b = m::text(&ui_state(), "branch");
            if b.is_empty() {
                "main".to_owned()
            } else {
                b
            }
        });
    dom::options(
        "omni-export-branch",
        &branch_items,
        m::summary,
        Some(&active_branch),
    );
    if let Some(datalist) = dom::get("omni-import-branch-list") {
        datalist.set_inner_html(
            &branches
                .iter()
                .map(|b| format!("<option value=\"{}\"></option>", esc(b)))
                .collect::<Vec<_>>()
                .join(""),
        );
    }
    if dom::value("omni-import-target-branch").trim().is_empty() || preferred_branch.is_some() {
        dom::set_value("omni-import-target-branch", &active_branch);
    }
}

pub async fn open_transfer_dialog(
    tab: &str,
    preferred_scope: Option<&str>,
    preferred_session_id: Option<&str>,
    preferred_branch: Option<&str>,
) {
    attach_transfer_dropzone_listeners();
    if m::read(|s| s.sessions.is_empty()) {
        crate::session::refresh().await;
    }
    let sid = preferred_session_id
        .map(str::to_owned)
        .unwrap_or_else(|| m::read(|s| s.sid.clone()));
    let branch = preferred_branch.map(str::to_owned).or_else(|| {
        let sel = m::read(|s| s.selected.clone());
        if sel.is_object() && m::omnigraph_enabled(&sel) {
            Some(m::omnigraph_branch(&sel))
        } else {
            None
        }
    });
    populate_transfer_selectors(Some(&sid), branch.as_deref());

    let default_scope = preferred_scope.unwrap_or(if !sid.is_empty() { "task" } else { "branch" });
    dom::set_value("omni-export-scope", default_scope);
    sync_export_scope_fields();

    dom::set_checked("omni-import-inject", !sid.is_empty());
    dom::hidden("omni-import-status", true);
    dom::error("omni-import-error", "");
    dom::hidden("omni-export-status", true);
    dom::error("omni-export-error", "");
    update_import_preview();
    set_transfer_tab(tab);
    dom::dialog("omnigraph-transfer-dialog", true);

    if tab == "export" {
        run_export_preview().await;
    }
}

async fn submit_import_form() {
    dom::error("omni-import-error", "");
    dom::hidden("omni-import-status", true);
    let project = current_project_id();
    let raw_text = dom::value("omni-import-text");
    let target_branch = dom::value("omni-import-target-branch").trim().to_owned();
    let target_branch = if target_branch.is_empty() {
        "main".to_owned()
    } else {
        target_branch
    };
    let graph = m::data("omnigraph-graph");
    let existing_branches = branch_names_from_graph(&graph);
    let create_branch = !existing_branches.iter().any(|b| b == &target_branch);
    let from_branch = {
        let b = m::text(&ui_state(), "branch");
        if b.is_empty() {
            "main".to_owned()
        } else {
            b
        }
    };
    let mode = dom::value("omni-import-mode");
    let inject = dom::checked("omni-import-inject");
    let target_session_id = if inject {
        dom::value("omni-import-session")
    } else {
        String::new()
    };
    if inject && target_session_id.is_empty() {
        dom::error(
            "omni-import-error",
            "Choose a target session to inject into, or uncheck injection.",
        );
        return;
    }

    let body = match import_request_body(
        &project,
        &raw_text,
        &target_branch,
        &from_branch,
        create_branch,
        &mode,
        inject,
        &target_session_id,
    ) {
        Ok(body) => body,
        Err(msg) => {
            dom::error("omni-import-error", &msg);
            return;
        }
    };

    dom::disabled("omni-import-submit", true);
    m::set("omnigraph-pending", json!(true));
    crate::session::sync_working_indicator();
    match api::post("/api/omnigraph/import", body).await {
        Ok(res) => {
            if res["ok"].as_bool() == Some(false) {
                let msg = res["error"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Import failed.");
                dom::error("omni-import-error", msg);
            } else {
                sync_sessions_from_response(&res);
                let cur = m::read(|s| s.selected.clone());
                if cur.is_object() {
                    render_session_bar(&cur);
                }
                crate::session::history();
                session_controls();
                let resolved_branch = res["target_branch"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&target_branch);
                let atoms = res["imported_atoms"].as_u64().unwrap_or(0);
                let files = res["imported_files"].as_u64().unwrap_or(0);
                let edges = res["imported_edges"].as_u64().unwrap_or(0);
                dom::text(
                    "omni-import-status",
                    &format!("Imported {atoms} atoms, {files} files, {edges} edges into {resolved_branch}."),
                );
                dom::hidden("omni-import-status", false);
                let _ = switch_branch_state(resolved_branch, true);
                refresh_pill().await;
                if m::read(|s| s.view == "omnigraph") || m::data("omnigraph-graph").is_object() {
                    refresh_graph().await;
                    poll_changes(true).await;
                }
            }
        }
        Err(e) => dom::error("omni-import-error", &e.message),
    }
    dom::disabled("omni-import-submit", false);
    m::set("omnigraph-pending", json!(false));
    crate::session::sync_working_indicator();
}

fn share_project_sessions() -> Vec<V> {
    let project = current_project_id();
    m::read(|s| {
        s.sessions
            .iter()
            .filter(|sess| {
                let id = m::text(sess, "id");
                let p = m::text(sess, "project_id");
                !id.is_empty() && (project.is_empty() || p.is_empty() || p == project)
            })
            .cloned()
            .collect()
    })
}

fn share_selected_set() -> BTreeSet<String> {
    selected_peer_set("omnigraph-share-selection")
}

fn set_share_selected_set(set: &BTreeSet<String>) {
    let arr: Vec<V> = set.iter().cloned().map(V::String).collect();
    m::set("omnigraph-share-selection", V::Array(arr));
}

fn share_current_mode() -> String {
    if dom::checked("omni-share-mode-broadcast") {
        "broadcast".into()
    } else {
        "shared_branch".into()
    }
}

fn render_share_dialog_state() {
    let sessions = share_project_sessions();
    let selected = share_selected_set();
    let mode = share_current_mode();
    let source_id = dom::value("omni-share-source");
    let space_name = dom::value("omni-share-space");
    let filter = dom::value("omni-share-filter");

    dom::hidden("omni-share-source-field", mode != "broadcast");
    dom::text("omni-share-selected-count", &selected.len().to_string());

    if let Some(list_el) = dom::get("omnigraph-share-task-list") {
        let filtered = filtered_share_sessions(&sessions, &filter);
        let list_key = share_task_list_key(&filtered);
        let cards = dom::all("#omnigraph-share-task-list .omni-share-task-card");
        let checks =
            dom::all("#omnigraph-share-task-list input[data-action=\"omni-share-task-check\"]");
        let same_shape = !list_key.is_empty()
            && list_el.get_attribute("data-list-key").as_deref() == Some(list_key.as_str())
            && cards.len() == filtered.len()
            && checks.len() == filtered.len();
        if same_shape {
            for (card, check_el) in cards.into_iter().zip(checks) {
                if let Ok(inp) = check_el.dyn_into::<web_sys::HtmlInputElement>() {
                    let id = inp.value();
                    let is_selected = selected.contains(&id);
                    let is_source = mode == "broadcast" && !source_id.is_empty() && id == source_id;
                    inp.set_checked(is_selected);
                    let _ = card
                        .set_attribute("data-selected", if is_selected { "true" } else { "false" });
                    let _ =
                        card.set_attribute("data-source", if is_source { "true" } else { "false" });
                }
            }
        } else {
            let _ = list_el.set_attribute("data-list-key", &list_key);
            list_el.set_inner_html(&render_share_task_list_html(
                &sessions, &selected, &mode, &source_id, &filter,
            ));
        }
    }
    if let Some(topo_el) = dom::get("omnigraph-share-topology") {
        topo_el.set_inner_html(&render_share_topology_html(
            &sessions,
            &selected,
            &mode,
            &source_id,
            &space_name,
        ));
    }
    let (can_submit, summary) =
        share_summary_text(&sessions, &selected, &mode, &source_id, &space_name);
    dom::text("omni-share-summary", &summary);
    dom::disabled("omni-share-submit", !can_submit);
}

fn set_share_step(step: usize, state: &str) {
    dom::attr(&format!("omni-share-step-{step}"), "data-state", state);
}

pub async fn open_share_dialog(preselect_sid: Option<&str>) {
    if m::data("omnigraph-status").is_null() {
        refresh_pill().await;
    }
    if m::read(|s| s.sessions.is_empty()) {
        crate::session::refresh().await;
    }
    let connected = m::yes(&m::data("omnigraph-status")["connected"]);
    dom::hidden("omni-share-offline-note", connected);

    let sessions = share_project_sessions();
    let active_sid = preselect_sid
        .map(str::to_owned)
        .unwrap_or_else(|| m::read(|s| s.sid.clone()));

    let mut initial = BTreeSet::new();
    let mut initial_space = String::new();
    if !active_sid.is_empty() {
        initial.insert(active_sid.clone());
        if let Some(sess) = sessions.iter().find(|s| m::text(s, "id") == active_sid) {
            initial_space = m::omnigraph_shared_space(sess);
            for peer_id in m::omnigraph_linked_session_ids(sess) {
                initial.insert(peer_id);
            }
        }
    }
    set_share_selected_set(&initial);

    let src_items: Vec<V> = sessions
        .iter()
        .map(|s| {
            let id = m::text(s, "id");
            let label = m::summary(s);
            json!({
                "id": id,
                "name": if label.is_empty() { id } else { label }
            })
        })
        .collect();
    let default_src = if !active_sid.is_empty() {
        active_sid
    } else {
        src_items
            .first()
            .map(|v| m::text(v, "id"))
            .unwrap_or_default()
    };
    dom::options(
        "omni-share-source",
        &src_items,
        m::summary,
        Some(&default_src),
    );

    dom::set_checked("omni-share-mode-shared", true);
    dom::set_checked("omni-share-mode-broadcast", false);
    dom::set_value("omni-share-space", &initial_space);
    dom::set_value("omni-share-filter", "");
    dom::hidden("omni-share-progress", true);
    dom::hidden("omni-share-result", true);
    dom::error("omni-share-dlg-error", "");
    set_share_step(1, "pending");
    set_share_step(2, "pending");
    set_share_step(3, "pending");

    render_share_dialog_state();
    dom::dialog("omnigraph-share-dialog", true);
}

async fn submit_share_form() {
    dom::error("omni-share-dlg-error", "");
    dom::hidden("omni-share-result", true);

    let project = current_project_id();
    let mode = share_current_mode();
    let source_id = dom::value("omni-share-source");
    let space_name = dom::value("omni-share-space");
    let selected: Vec<String> = share_project_sessions()
        .into_iter()
        .map(|s| m::text(&s, "id"))
        .filter(|id| share_selected_set().contains(id))
        .collect();
    let eff_space = if mode == "shared_branch" {
        space_name.trim().to_owned()
    } else {
        String::new()
    };

    let body =
        match link_sessions_request_body(&project, &selected, &mode, &source_id, &eff_space, "") {
            Ok(body) => body,
            Err(msg) => {
                dom::error("omni-share-dlg-error", &msg);
                return;
            }
        };

    dom::hidden("omni-share-progress", false);
    set_share_step(1, "done");
    set_share_step(2, "running");
    set_share_step(3, "pending");
    dom::disabled("omni-share-submit", true);
    m::set("omnigraph-pending", json!(true));
    crate::session::sync_working_indicator();

    match api::post("/api/omnigraph/sessions/link", body).await {
        Ok(res) => {
            if res["ok"].as_bool() == Some(false) {
                set_share_step(2, "attention");
                let msg = res["error"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Failed to link tasks via Omnigraph.");
                dom::error("omni-share-dlg-error", msg);
                render_share_dialog_state();
            } else {
                set_share_step(2, "done");
                set_share_step(3, "running");
                sync_sessions_from_response(&res);
                let cur = m::read(|s| s.selected.clone());
                if cur.is_object() {
                    render_session_bar(&cur);
                }
                crate::session::history();
                session_controls();
                refresh_pill().await;
                if m::read(|s| s.view == "omnigraph") || m::data("omnigraph-graph").is_object() {
                    refresh_graph().await;
                    poll_changes(true).await;
                }
                set_share_step(3, "done");

                let shared_branch = res["shared_branch"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("main");
                let synced_atoms = res["synced_atoms"].as_u64().unwrap_or(0);
                let linked_count = m::array(&res["linked_session_ids"])
                    .len()
                    .max(selected.len());
                let conflicts = m::array(&res["conflicts"]).len();
                m::set("omnigraph-share-result-branch-val", json!(shared_branch));
                dom::text("omni-share-result-branch", shared_branch);
                dom::text(
                    "omni-share-result-atoms",
                    &format!("{synced_atoms} synced atoms"),
                );
                let mut copy = format!(
                    "Linked {linked_count} task(s) via {shared_branch}. Peer sessions now receive live context updates."
                );
                if conflicts > 0 {
                    copy.push_str(&format!(
                        " ({conflicts} merge conflict(s) flagged — inspect 3-Way Merge in Context Graph.)"
                    ));
                }
                dom::text("omni-share-result-copy", &copy);
                dom::hidden("omni-share-result", false);
                render_share_dialog_state();
            }
        }
        Err(e) => {
            set_share_step(2, "attention");
            dom::error("omni-share-dlg-error", &e.message);
            render_share_dialog_state();
        }
    }

    render_share_dialog_state();
    m::set("omnigraph-pending", json!(false));
    crate::session::sync_working_indicator();
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
    if kind == "input" && id == "omni-import-text" {
        update_import_preview();
        return true;
    }
    if kind == "input" && (id == "omni-share-space" || id == "omni-share-filter") {
        render_share_dialog_state();
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
            "omni-export-scope"
            | "omni-export-format"
            | "omni-export-session"
            | "omni-export-branch"
            | "omni-export-include-inherited" => {
                sync_export_scope_fields();
                run_export_preview().await;
                return true;
            }
            "omni-import-file" => {
                if let Some(input) = dom::get("omni-import-file")
                    .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
                {
                    if let Some(files) = input.files() {
                        if let Some(file) = files.item(0) {
                            read_import_file(file).await;
                        }
                    }
                }
                return true;
            }
            "omni-import-text" | "omni-import-inject" => {
                update_import_preview();
                return true;
            }
            "omni-share-mode-shared"
            | "omni-share-mode-broadcast"
            | "omni-share-source"
            | "omni-share-space" => {
                render_share_dialog_state();
                return true;
            }
            _ if action == "omni-share-task-check" => {
                let mut set = share_selected_set();
                if checked {
                    set.insert(value.to_owned());
                } else {
                    set.remove(value);
                }
                set_share_selected_set(&set);
                render_share_dialog_state();
                return true;
            }
            _ => {}
        }
    }

    if kind == "submit" {
        match id {
            "omni-import-form" => {
                submit_import_form().await;
                return true;
            }
            "omni-share-form" => {
                submit_share_form().await;
                return true;
            }
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
        "session-omnigraph-export" => {
            let sid = m::read(|s| m::text(&s.selected, "id"));
            let branch = m::read(|s| m::omnigraph_branch(&s.selected));
            open_transfer_dialog(
                "export",
                Some(if sid.is_empty() { "branch" } else { "task" }),
                (!sid.is_empty()).then_some(sid.as_str()),
                Some(&branch),
            )
            .await;
            true
        }
        "session-omnigraph-import" => {
            let sid = m::read(|s| m::text(&s.selected, "id"));
            let branch = m::read(|s| m::omnigraph_branch(&s.selected));
            open_transfer_dialog(
                "import",
                None,
                (!sid.is_empty()).then_some(sid.as_str()),
                Some(&branch),
            )
            .await;
            true
        }
        "session-omnigraph-share-tasks" => {
            let sid = m::read(|s| m::text(&s.selected, "id"));
            open_share_dialog((!sid.is_empty()).then_some(sid.as_str())).await;
            true
        }
        "omnigraph-export-open" => {
            let branch = m::text(&ui_state(), "branch");
            open_transfer_dialog("export", Some("branch"), None, Some(&branch)).await;
            true
        }
        "omnigraph-import-open" => {
            let branch = m::text(&ui_state(), "branch");
            open_transfer_dialog("import", None, None, Some(&branch)).await;
            true
        }
        "omni-transfer-close" => {
            dom::dialog("omnigraph-transfer-dialog", false);
            true
        }
        "omnigraph-transfer-tab" => {
            let tab = payload["tab"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or_else(|| id.strip_prefix("omni-transfer-tab-").unwrap_or("export"));
            set_transfer_tab(tab);
            if tab == "export" {
                run_export_preview().await;
            }
            true
        }
        "omni-export-preview-btn" => {
            run_export_preview().await;
            true
        }
        "omni-export-download" => {
            trigger_export_download().await;
            true
        }
        "omni-export-copy" => {
            copy_export_payload().await;
            true
        }
        "omni-import-browse" | "omni-import-dropzone" => {
            if let Some(el) =
                dom::get("omni-import-file").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                el.click();
            }
            true
        }
        "omni-import-clear" => {
            dom::set_value("omni-import-text", "");
            dom::set_value("omni-import-file", "");
            dom::text("omni-import-file-name", "No file selected");
            dom::hidden("omni-import-status", true);
            dom::error("omni-import-error", "");
            update_import_preview();
            true
        }
        "omni-share-dialog-close" | "omni-share-cancel" | "omni-share-done" => {
            dom::dialog("omnigraph-share-dialog", false);
            true
        }
        "omni-share-open-setup" => {
            dom::dialog("omnigraph-share-dialog", false);
            crate::session::open_view("omnigraph", true).await;
            true
        }
        "omni-share-open-graph" => {
            let branch = m::string(&m::data("omnigraph-share-result-branch-val"));
            dom::dialog("omnigraph-share-dialog", false);
            if !branch.is_empty() {
                focus_branch(&branch);
            }
            crate::session::open_view("omnigraph", true).await;
            true
        }
        "omni-share-select-all" => {
            let needle = dom::value("omni-share-filter").trim().to_ascii_lowercase();
            let mut set = share_selected_set();
            for s in share_project_sessions() {
                if share_session_matches_filter(&s, &needle) {
                    set.insert(m::text(&s, "id"));
                }
            }
            set_share_selected_set(&set);
            render_share_dialog_state();
            true
        }
        "omni-share-clear" => {
            set_share_selected_set(&BTreeSet::new());
            render_share_dialog_state();
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

    #[test]
    fn import_parser_detects_bundle_ndjson_single_line_and_errors() {
        let empty = inspect_import_payload("   \n ");
        assert_eq!(empty.format, ImportFormat::Empty);

        let bundle_raw = r#"{
            "format": "harness.omnigraph.bundle.v1",
            "source_branch": "session/auth-v2",
            "atoms": [{"id": "a1", "title": "Token check"}, {"id": "a2", "title": "Cookie rule"}],
            "files": [{"path": "src/auth.rs"}],
            "edges": [{"from": "a1", "to": "a2", "relation": "supports"}],
            "sessions": [{"id": "s1"}]
        }"#;
        let bundle = inspect_import_payload(bundle_raw);
        assert_eq!(bundle.format, ImportFormat::Bundle);
        assert_eq!(bundle.atoms, 2);
        assert_eq!(bundle.files, 1);
        assert_eq!(bundle.edges, 1);
        assert_eq!(bundle.sessions, 1);
        assert_eq!(bundle.source_branch, "session/auth-v2");
        assert!(bundle.bundle.is_some());
        assert!(bundle.raw_text.is_none());

        // Wrapped export API response `{ ok, bundle: {...} }` unwraps cleanly.
        let wrapped_raw = json!({
            "ok": true,
            "bundle": {
                "format": "harness.omnigraph.bundle.v1",
                "source_branch": "main",
                "atoms": [{"id": "a3"}],
                "files": [],
                "edges": []
            }
        })
        .to_string();
        let wrapped = inspect_import_payload(&wrapped_raw);
        assert_eq!(wrapped.format, ImportFormat::Bundle);
        assert_eq!(wrapped.atoms, 1);
        assert_eq!(wrapped.source_branch, "main");

        // Multi-line NDJSON stream
        let ndjson_raw = concat!(
            r#"{"type":"Session","id":"s-1","branch":"session/s1"}"#,
            "\n",
            r#"{"type":"ContextAtom","id":"a-1","title":"Finding","source_branch":"session/s1"}"#,
            "\n",
            r#"{"type":"FileRef","path":"src/lib.rs"}"#,
            "\n",
            r#"{"type":"TouchesFile","from":"a-1","to":"src/lib.rs"}"#
        );
        let ndjson = inspect_import_payload(ndjson_raw);
        assert_eq!(ndjson.format, ImportFormat::Ndjson);
        assert_eq!(ndjson.sessions, 1);
        assert_eq!(ndjson.atoms, 1);
        assert_eq!(ndjson.files, 1);
        assert_eq!(ndjson.edges, 1);
        assert_eq!(ndjson.source_branch, "session/s1");
        assert!(ndjson.raw_text.is_some());

        // Single-line NDJSON object should NOT be misclassified as Bundle.
        let single_ndjson = inspect_import_payload(r#"{"type":"ContextAtom","id":"a-single"}"#);
        assert_eq!(single_ndjson.format, ImportFormat::Ndjson);
        assert_eq!(single_ndjson.atoms, 1);

        // Malformed line reports exact line number.
        let bad_ndjson = inspect_import_payload("{\"type\":\"ContextAtom\"}\n{bad json");
        assert_eq!(bad_ndjson.format, ImportFormat::Invalid);
        assert!(bad_ndjson.error.contains("Line 2"));

        // SymbolRef / TurnSnapshot / nested data.source_branch / {edge, source, target} lines
        let backend_ndjson_raw = concat!(
            r#"{"type":"SymbolRef","id":"sym-1","name":"validateJwt","data":{"source_branch":"session/nested"}}"#,
            "\n",
            r#"{"type":"TurnSnapshot","id":"turn-1","data":{"source_branch":"session/nested"}}"#,
            "\n",
            r#"{"type":"ContextAtom","id":"a-real","data":{"title":"Verified JWT flow","source_branch":"session/nested"}}"#,
            "\n",
            r#"{"type":"FileRef","path":"src/jwt.rs"}"#,
            "\n",
            r#"{"edge":"relates_to","source":"a-real","target":"sym-1"}"#
        );
        let backend_ndjson = inspect_import_payload(backend_ndjson_raw);
        assert_eq!(backend_ndjson.format, ImportFormat::Ndjson);
        assert_eq!(backend_ndjson.atoms, 1);
        assert_eq!(backend_ndjson.files, 1);
        assert_eq!(backend_ndjson.edges, 1);
        assert_eq!(backend_ndjson.sessions, 0);
        assert_eq!(backend_ndjson.source_branch, "session/nested");

        let single_edge =
            inspect_import_payload(r#"{"edge":"relates_to","source":"a","target":"b"}"#);
        assert_eq!(single_edge.format, ImportFormat::Ndjson);
        assert_eq!(single_edge.atoms, 0);
        assert_eq!(single_edge.edges, 1);
    }

    #[test]
    fn export_and_import_payloads_and_download_urls_format_cleanly() {
        let req = export_request_body("proj-1", "session/s1", "sess-1", "bundle", "task", true);
        assert_eq!(
            req,
            json!({
                "project_id": "proj-1",
                "branch": "session/s1",
                "session_id": "sess-1",
                "format": "bundle",
                "scope": "task",
                "include_inherited": true
            })
        );

        // Task scope with no explicit branch omits branch so the backend resolves it by session_id.
        let task_req_no_branch =
            export_request_body("proj-1", "", "sess-1", "bundle", "task", false);
        assert!(task_req_no_branch.get("branch").is_none());
        assert_eq!(task_req_no_branch["session_id"], "sess-1");
        let task_url_no_branch =
            export_download_url("proj-1", "", "sess-1", "bundle", "task", false);
        assert!(!task_url_no_branch.contains("branch="));

        let url = export_download_url("proj 1", "session/a&b", "sess-1", "ndjson", "task", false);
        assert!(url.starts_with("/api/omnigraph/export?"));
        assert!(url.contains("project_id=proj%201"));
        assert!(url.contains("branch=session%2Fa%26b"));
        assert!(url.contains("session_id=sess-1"));
        assert!(url.contains("format=ndjson"));
        assert!(url.contains("scope=task"));
        assert!(url.contains("include_inherited=false"));
        assert!(url.contains("download=1"));

        // Branch scope omits session_id even if passed.
        let branch_url = export_download_url("p1", "main", "ignored", "bundle", "branch", true);
        assert!(!branch_url.contains("session_id="));

        let summary = export_preview_summary(
            &json!({
                "filename": "task-sess1.json",
                "source_branch": "session/s1",
                "counts": { "atoms": 4, "files": 2, "edges": 6, "sessions": 1 },
                "bundle": { "format": "harness.omnigraph.bundle.v1", "atoms": [] }
            }),
            "bundle",
        );
        assert_eq!(summary.0, "4 atoms · 2 files · 6 edges · 1 sessions");
        assert_eq!(summary.1, "task-sess1.json");
        assert_eq!(summary.2, "session/s1");
        assert!(summary.3.contains("harness.omnigraph.bundle.v1"));

        let import_body = import_request_body(
            "proj-1",
            r#"{"format":"harness.omnigraph.bundle.v1","atoms":[{"id":"a1"}]}"#,
            "session/target",
            "main",
            true,
            "replace",
            true,
            "sess-9",
        )
        .unwrap();
        assert_eq!(import_body["target_branch"], "session/target");
        assert_eq!(import_body["from_branch"], "main");
        assert_eq!(import_body["create_branch"], true);
        assert_eq!(import_body["mode"], "replace");
        assert_eq!(import_body["inject_into_sessions"], true);
        assert_eq!(import_body["session_id"], "sess-9");
        assert_eq!(import_body["target_session_ids"], json!(["sess-9"]));
        assert!(import_body["bundle"].is_object());
        assert!(import_body.get("raw_text").is_none());

        let ndjson_import = import_request_body(
            "proj-1",
            "{\"type\":\"ContextAtom\",\"id\":\"a1\"}",
            "",
            "",
            false,
            "merge",
            false,
            "",
        )
        .unwrap();
        assert_eq!(ndjson_import["target_branch"], "main");
        assert!(ndjson_import["raw_text"].is_string());
        assert!(ndjson_import.get("bundle").is_none());
    }

    #[test]
    fn multi_task_link_payload_state_sync_topology_and_peer_strips_work_cleanly() {
        let sess_payload = session_payload_with_links(
            true,
            "auto",
            "",
            "main",
            "auth",
            true,
            false,
            &["s1".into(), " s2 ".into(), "s1".into(), "".into()],
            " shared/sprint-auth ",
        )
        .unwrap();
        assert_eq!(sess_payload["linked_session_ids"], json!(["s1", "s2"]));
        assert_eq!(sess_payload["shared_space"], "shared/sprint-auth");

        assert!(link_sessions_request_body(
            "p1",
            &["s1".into()],
            "shared_branch",
            "",
            "sprint",
            ""
        )
        .is_err());

        let mutual = link_sessions_request_body(
            "p1",
            &["s1".into(), "s2".into(), "s1".into()],
            "shared_branch",
            "",
            "sprint-auth",
            "shared/sprint-auth",
        )
        .unwrap();
        assert_eq!(mutual["mode"], "shared_branch");
        assert_eq!(mutual["session_ids"], json!(["s1", "s2"]));
        assert_eq!(mutual["space_name"], "sprint-auth");
        assert_eq!(mutual["target_branch"], "shared/sprint-auth");
        assert_eq!(mutual["inject_live"], true);

        assert!(
            link_sessions_request_body("p1", &["s1".into()], "broadcast", "s1", "", "").is_err()
        );
        let broadcast = link_sessions_request_body(
            "p1",
            &["s1".into(), "s2".into(), "s3".into()],
            "broadcast",
            "s1",
            "",
            "",
        )
        .unwrap();
        assert_eq!(broadcast["mode"], "broadcast");
        assert_eq!(broadcast["source_session_id"], "s1");

        // Session state synchronization updates s.sessions and s.selected.
        m::write(|s| {
            s.sid = "s1".into();
            s.selected = json!({ "id": "s1", "title": "Auth API" });
            s.sessions = vec![
                json!({ "id": "s1", "title": "Auth API", "project_id": "p1", "status": "running" }),
                json!({ "id": "s2", "title": "Auth UI", "project_id": "p1", "status": "completed" }),
            ];
        });
        sync_sessions_from_response(&json!({
            "ok": true,
            "sessions": [
                {
                    "id": "s1",
                    "title": "Auth API",
                    "project_id": "p1",
                    "status": "running",
                    "omnigraph": {
                        "enabled": true,
                        "branch": "shared/sprint-auth",
                        "shared_space": "shared/sprint-auth",
                        "linked_session_ids": ["s2"],
                        "injected_atoms": [{"id": "a1", "title": "JWT contract"}]
                    }
                },
                {
                    "id": "s2",
                    "title": "Auth UI",
                    "project_id": "p1",
                    "status": "completed",
                    "omnigraph": {
                        "enabled": true,
                        "branch": "shared/sprint-auth",
                        "shared_space": "shared/sprint-auth",
                        "linked_session_ids": ["s1"]
                    }
                }
            ]
        }));

        let (selected, all) = m::read(|s| (s.selected.clone(), s.sessions.clone()));
        assert_eq!(selected["omnigraph"]["shared_space"], "shared/sprint-auth");
        let (show_peers, peers_html) = render_session_peers_html(&selected, &all);
        assert!(show_peers);
        assert!(peers_html.contains("Space: shared/sprint-auth"));
        assert!(peers_html.contains("data-action=\"omnigraph-open-session\""));
        assert!(peers_html.contains("data-session-id=\"s2\""));
        assert!(peers_html.contains("Auth UI"));

        let chosen = ["s1".to_owned(), "s2".to_owned()]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let topo_mutual =
            render_share_topology_html(&all, &chosen, "shared_branch", "", "sprint-auth");
        assert!(topo_mutual.contains("Mutual Shared Space"));
        assert!(topo_mutual.contains("shared/sprint-auth"));
        assert!(topo_mutual.contains("Auth API"));
        assert!(topo_mutual.contains("Auth UI"));

        let topo_broadcast = render_share_topology_html(&all, &chosen, "broadcast", "s1", "");
        assert!(topo_broadcast.contains("Broadcast mode"));
        assert!(topo_broadcast.contains("data-role=\"source\""));

        let cards = render_share_task_list_html(&all, &chosen, "shared_branch", "", "");
        assert!(cards.contains("omni-share-task-card"));
        assert!(cards.contains("data-selected=\"true\""));

        let (show_panel_peers, linked_count, panel_html) =
            render_panel_peer_checklist_html(&all, "", "p1", &chosen, false);
        assert!(show_panel_peers);
        assert_eq!(linked_count, 2);
        assert!(panel_html.contains("data-action=\"omnigraph-peer-choice\""));

        // Slugification normalizes free-text shared space names cleanly.
        assert_eq!(resolved_space_branch(""), "shared/context");
        assert_eq!(resolved_space_branch("Team Auth"), "shared/team-auth");
        assert_eq!(
            resolved_space_branch("shared/Team Auth"),
            "shared/team-auth"
        );
        assert_eq!(
            resolved_space_branch("shared/sprint-auth"),
            "shared/sprint-auth"
        );
        let slugged_session = session_payload_with_links(
            true,
            "auto",
            "",
            "main",
            "",
            true,
            false,
            &["s1".into()],
            "Team Auth",
        )
        .unwrap();
        assert_eq!(slugged_session["shared_space"], "shared/team-auth");

        // Omitting target_branch allows the backend to slugify space_name or keep broadcast source_branch.
        let space_only = link_sessions_request_body(
            "p1",
            &["s1".into(), "s2".into()],
            "shared_branch",
            "",
            "Team Auth",
            "",
        )
        .unwrap();
        assert_eq!(space_only["space_name"], "Team Auth");
        assert!(space_only.get("target_branch").is_none());
        let broadcast_clean = link_sessions_request_body(
            "p1",
            &["s1".into(), "s2".into()],
            "broadcast",
            "s1",
            "",
            "",
        )
        .unwrap();
        assert!(broadcast_clean.get("space_name").is_none());
        assert!(broadcast_clean.get("target_branch").is_none());

        // Filter parity between rendered share task cards and shared helper used by Select All.
        let filtered_cards =
            render_share_task_list_html(&all, &chosen, "shared_branch", "", "sprint-auth");
        assert!(filtered_cards.contains("Auth API"));
        assert!(filtered_cards.contains("Auth UI"));
        assert!(share_session_matches_filter(&all[0], "sprint-auth"));
        assert!(!share_session_matches_filter(&all[0], "nonexistent"));
    }
}
