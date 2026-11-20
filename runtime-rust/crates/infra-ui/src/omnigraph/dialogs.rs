use super::*;

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

pub(super) fn is_ndjson_record_obj(obj: &serde_json::Map<String, V>) -> bool {
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

pub(super) fn count_ndjson_record(
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

pub(super) fn share_session_matches_filter(s: &V, needle: &str) -> bool {
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

pub(super) fn filtered_share_sessions<'a>(sessions: &'a [V], filter: &str) -> Vec<&'a V> {
    let needle = filter.trim().to_ascii_lowercase();
    sessions
        .iter()
        .filter(|s| share_session_matches_filter(s, &needle))
        .collect()
}

pub(super) fn share_task_list_key(filtered: &[&V]) -> String {
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

pub(super) fn slugify_space_segment(raw: &str) -> String {
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

pub(super) fn resolved_space_branch(space_name: &str) -> String {
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

pub(super) fn apply_merged_session_state(sid: &str, branch: &str, res: &V) {
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

pub(super) fn attach_transfer_dropzone_listeners() {
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

pub(super) async fn read_import_file(file: web_sys::File) {
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

pub(super) fn set_transfer_tab(tab: &str) {
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

pub(super) fn sync_export_scope_fields() {
    let scope = dom::value("omni-export-scope");
    dom::hidden("omni-export-session-field", scope != "task");
    dom::hidden("omni-export-branch-field", scope != "branch");
}

pub(super) fn current_export_options() -> (String, String, String, String, String, bool) {
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

pub(super) async fn run_export_preview() -> Option<(String, String)> {
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

pub(super) async fn trigger_export_download() {
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

pub(super) async fn copy_export_payload() {
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

pub(super) fn update_import_preview() {
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

pub(super) fn populate_transfer_selectors(
    preferred_session_id: Option<&str>,
    preferred_branch: Option<&str>,
) {
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

pub(super) async fn submit_import_form() {
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

pub(super) fn share_project_sessions() -> Vec<V> {
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

pub(super) fn share_selected_set() -> BTreeSet<String> {
    selected_peer_set("omnigraph-share-selection")
}

pub(super) fn set_share_selected_set(set: &BTreeSet<String>) {
    let arr: Vec<V> = set.iter().cloned().map(V::String).collect();
    m::set("omnigraph-share-selection", V::Array(arr));
}

pub(super) fn share_current_mode() -> String {
    if dom::checked("omni-share-mode-broadcast") {
        "broadcast".into()
    } else {
        "shared_branch".into()
    }
}

pub(super) fn render_share_dialog_state() {
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

pub(super) fn set_share_step(step: usize, state: &str) {
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

pub(super) async fn submit_share_form() {
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
