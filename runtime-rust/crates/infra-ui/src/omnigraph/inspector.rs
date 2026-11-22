use super::*;

pub fn atom_is_verified(atom: &V) -> bool {
    if atom["verified"].as_bool() == Some(true) {
        return true;
    }
    if atom["verified"].as_bool() == Some(false) {
        return false;
    }
    matches!(atom["authority"].as_str(), Some("verified"))
}

pub(super) fn render_provenance_pills_html(atom: &V) -> String {
    let mut pills = Vec::new();
    if atom_is_verified(atom) {
        pills.push(
            "<span class=\"omni-badge\" data-kind=\"success\" data-prov=\"verified\">Verified</span>"
                .to_owned(),
        );
    } else {
        pills.push(
            "<span class=\"omni-badge\" data-kind=\"warning\" data-prov=\"unverified\">Unverified reference</span>"
                .to_owned(),
        );
    }
    if let Some(layer) = atom["origin_layer"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        pills.push(format!(
            "<span class=\"omni-badge\" data-prov=\"origin_layer\">{}</span>",
            esc(layer)
        ));
    }
    if let Some(auth) = atom["authority"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        pills.push(format!(
            "<span class=\"omni-meta-pill\" data-prov=\"authority\">authority: {}</span>",
            esc(auth)
        ));
    }
    if let Some(priv_val) = atom["privacy"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        pills.push(format!(
            "<span class=\"omni-meta-pill\" data-prov=\"privacy\">privacy: {}</span>",
            esc(priv_val)
        ));
    }
    pills.join("")
}

pub(super) fn render_provenance_jump_buttons_html(atom: &V) -> String {
    let mut buttons = Vec::new();
    let brain_id = atom["brain_record_id"].as_str().unwrap_or("").trim();
    if !brain_id.is_empty() {
        let short_id: String = brain_id.chars().take(8).collect();
        let payload = json!({ "record_id": brain_id }).to_string();
        buttons.push(format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"omnigraph-open-brain\" data-record-id=\"{}\" data-payload=\"{}\" title=\"Open Project Brain record {}\">Brain: {}</button>",
            esc(brain_id),
            esc(&payload),
            esc(brain_id),
            esc(&short_id)
        ));
    }
    let memory_id = atom["memory_id"].as_str().unwrap_or("").trim();
    if !memory_id.is_empty() {
        let payload = json!({ "memory_id": memory_id }).to_string();
        buttons.push(format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"omnigraph-open-memory\" data-memory-id=\"{}\" data-payload=\"{}\" title=\"Open Memory Bank chunk {}\">Memory: {}</button>",
            esc(memory_id),
            esc(&payload),
            esc(memory_id),
            esc(memory_id)
        ));
    }
    // A Git commit the atom names opens in the Code Map; an Omnigraph
    // snapshot (`commit_id`) keeps `omnigraph-select-commit`.
    for sha in git_commits_named(atom) {
        let payload = json!({ "sha": sha }).to_string();
        buttons.push(format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"omnigraph-open-git-commit\" data-payload=\"{}\" title=\"Open Git commit {} in the Code Map\">Git: {}</button>",
            esc(&payload),
            esc(&sha),
            esc(&sha.chars().take(10).collect::<String>())
        ));
    }
    buttons.join("")
}

/// A Git commit id as the Code Map takes it: 7 to 40 lowercase hex digits.
pub(super) fn is_git_sha(text: &str) -> bool {
    (7..=40).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The Git commits an atom names, at most four: its `git_commit` field, or
/// a lowercase hex word of 7 to 40 characters with a letter and a digit
/// standing alone in its tags, title, summary or body (not part of a path,
/// an id like `MEM-…-a1b2c3d4`, or a file name).
pub(super) fn git_commits_named(atom: &V) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut add = |sha: &str| {
        if found.len() < 4 && is_git_sha(sha) && !found.iter().any(|seen| seen == sha) {
            found.push(sha.to_owned());
        }
    };
    if let Some(sha) = atom["git_commit"].as_str() {
        add(sha.trim());
    }
    let texts = m::array(&atom["tags"])
        .iter()
        .filter_map(V::as_str)
        .map(str::to_owned)
        .chain(
            ["title", "summary", "body"]
                .iter()
                .filter_map(|key| atom[*key].as_str().map(str::to_owned)),
        )
        .collect::<Vec<_>>();
    for text in &texts {
        let chars = text.char_indices().collect::<Vec<_>>();
        let mut i = 0;
        while i < chars.len() {
            if !chars[i].1.is_ascii_hexdigit() {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && chars[i].1.is_ascii_hexdigit() {
                i += 1;
            }
            let before = start.checked_sub(1).map(|at| chars[at].1);
            let after = chars.get(i).map(|c| c.1);
            let after_next = chars.get(i + 1).map(|c| c.1);
            let opens = before.is_none_or(|c| c.is_whitespace() || "([{<'\"`:,;".contains(c));
            let closes = match after {
                None => true,
                Some('.') => after_next.is_none_or(char::is_whitespace),
                Some(c) => c.is_whitespace() || ")]}>'\"`,;:!?".contains(c),
            };
            let word = &text[chars[start].0..chars.get(i).map_or(text.len(), |c| c.0)];
            if opens
                && closes
                && word.bytes().any(|b| b.is_ascii_digit())
                && word.bytes().any(|b| (b'a'..=b'f').contains(&b))
            {
                add(word);
            }
        }
    }
    found
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
        let prov_pills = render_provenance_pills_html(atom);
        let jump_buttons = render_provenance_jump_buttons_html(atom);
        html.push_str(&format!(
            "<div class=\"omni-search-hit-card\"><button type=\"button\" class=\"omni-search-hit\" data-action=\"omnigraph-focus-hit\" data-node-id=\"{}\"><div class=\"omni-search-hit-head\"><span class=\"omni-badge\" data-kind=\"{}\"><span aria-hidden=\"true\">{}</span> {}</span><span class=\"omni-meta-pill\">{} · {:.2}{}</span></div><strong>{}</strong>{}<div class=\"omni-search-hit-meta\">{}</div></button>{}</div>",
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
            },
            prov_pills,
            if jump_buttons.is_empty() {
                String::new()
            } else {
                format!("<div class=\"omni-search-hit-links\">{jump_buttons}</div>")
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
    if let Some(symbol_node) = m::array(&graph["symbols"])
        .into_iter()
        .find(|s| symbol_node_id(s) == selected_id || s["fqn"].as_str() == Some(selected_id))
    {
        return render_symbol_node_inspector_html(graph, &symbol_node);
    }
    format!(
        "<div class=\"omni-inspector-card\"><p class=\"knowledge-note\">Node <code>{}</code> is not present in the current filter or snapshot.</p></div>",
        esc(selected_id)
    )
}

pub(super) fn render_atom_inspector_html(
    graph: &V,
    atom: &V,
    sessions: &[V],
    active_branch: &str,
) -> String {
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
        "<div class=\"omni-inspector-group\"><h4>Confidence &amp; Priority</h4><div class=\"omni-node-meta\"><span>{:.0}% confidence</span>{}</div><div class=\"omni-confidence-track\" role=\"progressbar\" aria-label=\"Confidence score\" aria-valuenow=\"{}\" aria-valuemin=\"0\" aria-valuemax=\"100\"><div class=\"omni-confidence-fill\" style=\"--omni-confidence:{:.2}\"></div></div></div>",
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

    let ui_st = ui_state();
    let ctx_filter = ui_st["query"].as_str().unwrap_or("").trim().to_owned();
    let ctx_commit = {
        let from_atom = atom["commit_id"]
            .as_str()
            .or_else(|| atom["commit_sha"].as_str())
            .unwrap_or("")
            .trim();
        if !from_atom.is_empty() {
            from_atom.to_owned()
        } else {
            ui_st["selected_commit_id"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_owned()
        }
    };
    let ctx_sem_kind = {
        let from_ui = ui_st["kind"].as_str().unwrap_or("").trim();
        if !from_ui.is_empty() {
            from_ui.to_owned()
        } else {
            kind.trim().to_owned()
        }
    };

    let files = m::array(&atom["files"]);
    if !files.is_empty() {
        html.push_str("<div class=\"omni-inspector-group\"><h4>Linked files</h4><div class=\"omni-chip-list\">");
        for file in &files {
            if let Some(f) = file.as_str().filter(|s| !s.is_empty()) {
                let mut jump_obj = json!({ "path": f });
                if !ctx_filter.is_empty() {
                    jump_obj["filter"] = json!(ctx_filter);
                }
                if !ctx_commit.is_empty() {
                    jump_obj["target_commit"] = json!(ctx_commit);
                }
                if !ctx_sem_kind.is_empty() {
                    jump_obj["semantic_kind"] = json!(ctx_sem_kind);
                }
                let jump_payload = jump_obj.to_string();
                html.push_str(&format!(
                    "<button type=\"button\" class=\"omni-session-atom-chip omni-mono\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\">▤ {}</button><button type=\"button\" class=\"button compact\" data-action=\"omnigraph-open-code-map\" data-payload=\"{}\" title=\"Open {} in Code Map\">Map ↗</button>",
                    esc(f),
                    esc(f),
                    esc(&jump_payload),
                    esc(f),
                ));
            }
        }
        html.push_str("</div></div>");
    }

    let mut symbol_fqns: Vec<String> = Vec::new();
    for sym in m::array(&atom["symbols"]) {
        if let Some(s) = sym.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            if !symbol_fqns.iter().any(|existing| existing == s) {
                symbol_fqns.push(s.to_owned());
            }
        }
    }
    for edge in m::array(&graph["edges"])
        .into_iter()
        .chain(m::array(&graph["symbol_edges"]))
    {
        let rel = edge_rel(&edge);
        if matches!(
            rel.as_str(),
            "mentions_symbol" | "MentionsSymbol" | "mentions"
        ) && edge_from(&edge) == id
        {
            let target = edge_to(&edge);
            if !target.is_empty() && !symbol_fqns.iter().any(|existing| existing == &target) {
                symbol_fqns.push(target);
            }
        }
    }
    if !symbol_fqns.is_empty() {
        let graph_symbols = m::array(&graph["symbols"]);
        html.push_str("<div class=\"omni-inspector-group\"><h4>Mentioned symbols</h4><div class=\"omni-chip-list\">");
        for fqn in &symbol_fqns {
            let sym_node = graph_symbols
                .iter()
                .find(|s| symbol_node_id(s) == *fqn || s["fqn"].as_str() == Some(fqn.as_str()));
            let label = sym_node
                .map(symbol_display_label)
                .unwrap_or_else(|| short_symbol_from_fqn(fqn));
            let file_path = sym_node
                .map(symbol_file_path)
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| {
                    fqn.split_once("::")
                        .map(|(head, _)| head)
                        .filter(|head| head.contains('/') || head.contains('.'))
                        .unwrap_or("")
                        .to_owned()
                });
            html.push_str(&format!(
                "<button type=\"button\" class=\"omni-symbol-badge\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\" title=\"{}\">&lt;/&gt; {}</button>",
                esc(fqn),
                esc(fqn),
                esc(&label),
            ));
            if !file_path.is_empty() {
                let sym_id = sym_node
                    .and_then(|s| s["symbol_id"].as_str().or_else(|| s["id"].as_str()))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(fqn.as_str());
                let mut jump_obj = json!({
                    "path": file_path,
                    "symbol_id": sym_id,
                    "fqn": fqn,
                    "symbol": label,
                });
                if !ctx_filter.is_empty() {
                    jump_obj["filter"] = json!(ctx_filter);
                }
                if !ctx_commit.is_empty() {
                    jump_obj["target_commit"] = json!(ctx_commit);
                }
                if !ctx_sem_kind.is_empty() {
                    jump_obj["semantic_kind"] = json!(ctx_sem_kind);
                }
                let jump_payload = jump_obj.to_string();
                html.push_str(&format!(
                    "<button type=\"button\" class=\"button compact\" data-action=\"omnigraph-open-code-map\" data-payload=\"{}\" title=\"Open {} in Code Map\">Code Map ↗</button>",
                    esc(&jump_payload),
                    esc(&file_path),
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
    if let Some(rev) = atom["record_revision"].as_i64() {
        provenance_parts.push(format!("Revision: v{rev}"));
    }
    let prov_pills = render_provenance_pills_html(atom);
    let jump_buttons = render_provenance_jump_buttons_html(atom);
    html.push_str("<div class=\"omni-inspector-group\"><h4>Provenance</h4>");
    html.push_str(&format!("<div class=\"omni-chip-list\">{prov_pills}</div>"));
    if !provenance_parts.is_empty() {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">{}</p>",
            provenance_parts.join(" · ")
        ));
    }
    if !session_id.is_empty() || !commit_id.is_empty() || !jump_buttons.is_empty() {
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
        html.push_str(&jump_buttons);
        html.push_str("</div>");
    }
    html.push_str("</div>");

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
    let brain_record_id = atom["brain_record_id"].as_str().unwrap_or("").trim();
    let has_brain_link = !brain_record_id.is_empty();
    let default_promote_kind = match kind {
        "decision" => "decision",
        "warning" => "incident",
        _ => "finding",
    };
    let promote_branch = promote_target_branch(atom, active_branch);
    let promote_payload = json!({
        "atom_id": id,
        "branch": promote_branch,
    })
    .to_string();
    let promote_label = if has_brain_link {
        "Promoted to Brain"
    } else {
        "Promote to Brain…"
    };
    let promote_disabled = if has_brain_link { " disabled" } else { "" };

    html.push_str(&format!(
        "<div class=\"omni-chip-list\"><button type=\"button\" class=\"button compact primary\" data-action=\"omnigraph-share-atom\" data-atom-id=\"{}\" data-branch=\"{}\"{}>Share to Session</button><button type=\"button\" class=\"button compact\" data-action=\"omnigraph-copy-snippet\" data-atom-id=\"{}\">Copy context snippet</button><button type=\"button\" class=\"button compact\" data-action=\"omnigraph-open-branch\" data-branch=\"{}\">Open in Branch</button><button type=\"button\" class=\"button compact\" data-action=\"omnigraph-cite-atom\" data-atom-id=\"{}\">Cite in new atom</button></div><p class=\"knowledge-note\" id=\"omni-share-status\" role=\"status\" hidden></p><p class=\"error-text\" id=\"omni-share-error\" role=\"alert\" hidden></p><div class=\"omni-promote-row\"><div class=\"field\"><label for=\"omni-promote-kind\">Brain record</label><select id=\"omni-promote-kind\"{promote_disabled}>",
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
    for (k, klabel) in [
        ("finding", "Finding"),
        ("decision", "Decision"),
        ("bug", "Bug"),
        ("incident", "Incident"),
    ] {
        let sel = if k == default_promote_kind {
            " selected"
        } else {
            ""
        };
        html.push_str(&format!("<option value=\"{k}\"{sel}>{klabel}</option>"));
    }
    html.push_str(&format!(
        "</select></div><div class=\"field\"><label for=\"omni-promote-privacy\">Privacy</label><select id=\"omni-promote-privacy\"{promote_disabled}><option value=\"team\" selected>Team</option><option value=\"public\">Public</option></select></div><button type=\"button\" class=\"button compact\" id=\"omni-promote-atom\" data-action=\"omnigraph-promote-atom\" data-atom-id=\"{}\" data-branch=\"{}\" data-payload=\"{}\"{promote_disabled}>{promote_label}</button></div><p class=\"knowledge-note\" id=\"omni-promote-status\" role=\"status\" hidden></p><p class=\"error-text\" id=\"omni-promote-error\" role=\"alert\" hidden></p></div></article>",
        esc(&id),
        esc(promote_branch),
        esc(&promote_payload)
    ));
    html
}

pub(super) fn render_session_node_inspector_html(graph: &V, session: &V) -> String {
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

pub(super) fn render_file_node_inspector_html(graph: &V, file: &V) -> String {
    let path = file["path"]
        .as_str()
        .or_else(|| file["id"].as_str())
        .unwrap_or("");
    let mut html = format!(
        "<article class=\"omni-inspector-card\"><div class=\"omni-node-head\"><span class=\"omni-badge\" data-kind=\"file\"><span aria-hidden=\"true\">▤</span> File Anchor</span></div><h3 class=\"omni-inspector-title omni-mono\">{}</h3>",
        esc(path)
    );
    let symbols: Vec<V> = m::array(&graph["symbols"])
        .into_iter()
        .filter(|s| symbol_file_path(s) == path)
        .collect();
    if !symbols.is_empty() {
        html.push_str("<div class=\"omni-inspector-group\"><h4>Mentioned symbols</h4><div class=\"omni-chip-list\">");
        for sym in symbols.iter().take(16) {
            let sid = symbol_node_id(sym);
            let label = symbol_display_label(sym);
            let skind = sym["kind"].as_str().unwrap_or("");
            html.push_str(&format!(
                "<button type=\"button\" class=\"omni-symbol-badge\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\">&lt;/&gt; {}{}</button>",
                esc(&sid),
                esc(&label),
                if skind.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", esc(skind))
                }
            ));
        }
        html.push_str("</div></div>");
    }
    let mut touching: Vec<String> = Vec::new();
    for e in m::array(&graph["edges"])
        .into_iter()
        .chain(m::array(&graph["symbol_edges"]))
    {
        let f = edge_from(&e);
        let t = edge_to(&e);
        if t == path && !f.is_empty() && !touching.contains(&f) {
            touching.push(f);
        } else if f == path && !t.is_empty() && !touching.contains(&t) {
            touching.push(t);
        }
    }
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
        "<div class=\"omni-chip-list\"><button type=\"button\" class=\"button compact primary\" data-action=\"inspect-file-graph\" data-payload=\"{}\">Inspect in Code Graph</button></div></article>",
        esc(&json!({ "path": path }).to_string())
    ));
    html
}

pub(super) fn render_symbol_node_inspector_html(graph: &V, symbol: &V) -> String {
    let sid = symbol_node_id(symbol);
    let sym_id = symbol["symbol_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&sid);
    let fqn = symbol["fqn"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(&sid);
    let title = symbol_display_label(symbol);
    let skind = symbol["kind"].as_str().unwrap_or("symbol");
    let file_path = symbol_file_path(symbol);
    let owner = symbol["owner"].as_str().unwrap_or("").trim();

    let mut html = format!(
        "<article class=\"omni-inspector-card\"><div class=\"omni-node-head\"><span class=\"omni-badge\" data-kind=\"symbol\"><span aria-hidden=\"true\">⟨⟩</span> Symbol</span><span class=\"omni-symbol-kind-pill\">{}</span></div><h3 class=\"omni-inspector-title omni-mono\">{}</h3><p class=\"knowledge-note\">FQN: <code>{}</code></p>",
        esc(skind),
        esc(&title),
        esc(fqn),
    );
    if !file_path.is_empty() || !owner.is_empty() {
        let mut parts = Vec::new();
        if !file_path.is_empty() {
            parts.push(format!("File: <code>{}</code>", esc(&file_path)));
        }
        if !owner.is_empty() {
            parts.push(format!("Owner: <code>{}</code>", esc(owner)));
        }
        html.push_str(&format!(
            "<p class=\"knowledge-note\">{}</p>",
            parts.join(" · ")
        ));
    }

    let mut atom_refs: Vec<String> = m::array(&symbol["atom_ids"])
        .into_iter()
        .filter_map(|v| {
            v.as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .collect();
    for e in m::array(&graph["edges"])
        .into_iter()
        .chain(m::array(&graph["symbol_edges"]))
    {
        let f = edge_from(&e);
        let t = edge_to(&e);
        if (t == sid || t == fqn) && !f.is_empty() && !atom_refs.contains(&f) {
            atom_refs.push(f);
        }
    }
    if !atom_refs.is_empty() {
        let graph_atoms = m::array(&graph["atoms"]);
        html.push_str("<div class=\"omni-inspector-group\"><h4>Mentioning atoms</h4><div class=\"omni-chip-list\">");
        for aid in atom_refs.iter().take(16) {
            let label = graph_atoms
                .iter()
                .find(|a| atom_id(a) == *aid)
                .and_then(|a| a["title"].as_str().filter(|s| !s.is_empty()))
                .unwrap_or(aid.as_str());
            html.push_str(&format!(
                "<button type=\"button\" class=\"omni-session-atom-chip\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\">{}</button>",
                esc(aid),
                esc(label),
            ));
        }
        html.push_str("</div></div>");
    }

    html.push_str("<div class=\"omni-chip-list\">");
    if !file_path.is_empty() {
        let ui_st = ui_state();
        let mut jump_obj = json!({
            "path": file_path,
            "symbol_id": sym_id,
            "fqn": fqn,
            "symbol": title,
        });
        let q = ui_st["query"].as_str().unwrap_or("").trim();
        if !q.is_empty() {
            jump_obj["filter"] = json!(q);
        }
        let commit = symbol["commit_id"]
            .as_str()
            .or_else(|| ui_st["selected_commit_id"].as_str())
            .unwrap_or("")
            .trim();
        if !commit.is_empty() {
            jump_obj["target_commit"] = json!(commit);
        }
        let sem = ui_st["kind"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(skind)
            .trim();
        if !sem.is_empty() {
            jump_obj["semantic_kind"] = json!(sem);
        }
        let jump_payload = jump_obj.to_string();
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact primary\" data-action=\"omnigraph-open-code-map\" data-payload=\"{}\">Open in Code Map ↗</button>",
            esc(&jump_payload),
        ));
        if let Some(file_node) = m::array(&graph["files"]).into_iter().find(|f| {
            f["path"].as_str() == Some(file_path.as_str()) || file_node_id(f) == file_path
        }) {
            html.push_str(&format!(
                "<button type=\"button\" class=\"button compact\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\">Focus File Anchor</button>",
                esc(&file_node_id(&file_node)),
            ));
        }
    }
    html.push_str("</div></article>");
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
