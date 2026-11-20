use super::*;

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

/// What a follow-up changes in its session's Omnigraph settings: the
/// branches the next turn pulls (blank means `main`, as when the session
/// started), its focus query and auto-publish. `None` for a session without
/// Omnigraph context, which a follow-up must not switch on, or when nothing
/// changed.
pub fn followup_payload_from_fields(
    session: &V,
    pull_raw: &str,
    query: &str,
    auto_publish: bool,
) -> Option<V> {
    if !m::omnigraph_enabled(session) {
        return None;
    }
    let omni = &session["omnigraph"];
    let or_main = |branches: Vec<String>| {
        if branches.is_empty() {
            vec!["main".to_owned()]
        } else {
            branches
        }
    };
    let pulls = or_main(parse_csv_list(pull_raw));
    let current = or_main(
        m::array(&omni["pull_branches"])
            .iter()
            .filter_map(|branch| branch.as_str().map(str::to_owned))
            .collect(),
    );
    let mut changes = serde_json::Map::new();
    if pulls != current {
        changes.insert("pull_branches".into(), json!(pulls));
    }
    let query = query.trim();
    if query != m::text(omni, "query").trim() {
        changes.insert(
            "query".into(),
            if query.is_empty() {
                V::Null
            } else {
                json!(query)
            },
        );
    }
    if auto_publish != omni["auto_publish"].as_bool().unwrap_or(true) {
        changes.insert("auto_publish".into(), json!(auto_publish));
    }
    (!changes.is_empty()).then_some(V::Object(changes))
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

pub(super) fn session_atom_count(session: &V) -> usize {
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

pub(super) fn eligible_panel_peers<'a>(
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

pub(super) fn panel_peer_list_key(peers: &[&V]) -> String {
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

pub fn session_bar_error(session: &V) -> String {
    m::text(&session["omnigraph"], "last_error")
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionOmnigraphMetrics {
    pub enabled: bool,
    pub branch: String,
    pub capsule_chars: usize,
    pub capsule_limit: usize,
    pub verified_count: usize,
    pub unverified_count: usize,
    pub atom_count: usize,
    pub atoms: Vec<V>,
}

pub(super) fn normalize_session_atoms(raw: Vec<V>) -> Vec<V> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for item in raw {
        let normalized = if let Some(id_str) = item.as_str().filter(|s| !s.is_empty()) {
            json!({ "id": id_str, "title": id_str, "kind": "finding" })
        } else {
            item
        };
        let aid = atom_id(&normalized);
        if aid.is_empty() || seen.insert(aid) {
            out.push(normalized);
        }
    }
    out
}

pub fn session_bar_metrics(session: &V) -> SessionOmnigraphMetrics {
    if !m::omnigraph_enabled(session) {
        return SessionOmnigraphMetrics {
            enabled: false,
            branch: String::new(),
            capsule_chars: 0,
            capsule_limit: CAPSULE_CHAR_LIMIT,
            verified_count: 0,
            unverified_count: 0,
            atom_count: 0,
            atoms: Vec::new(),
        };
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
    let atoms = normalize_session_atoms(shared_atoms);
    let verified_count = atoms.iter().filter(|a| atom_is_verified(a)).count();
    let unverified_count = atoms.len().saturating_sub(verified_count);
    let atom_count = omni["atom_count"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(atoms.len());
    let capsule_chars = m::text(omni, "capsule_text").chars().count();
    SessionOmnigraphMetrics {
        enabled: true,
        branch,
        capsule_chars,
        capsule_limit: CAPSULE_CHAR_LIMIT,
        verified_count,
        unverified_count,
        atom_count,
        atoms,
    }
}

pub fn session_bar_summary(session: &V) -> (bool, String, String, Vec<V>, bool) {
    let metrics = session_bar_metrics(session);
    if !metrics.enabled {
        return (false, String::new(), String::new(), Vec::new(), false);
    }
    let omni = &session["omnigraph"];
    let branch = metrics.branch;
    let published = m::array(&omni["published_atoms"]);
    let count = metrics.atom_count;
    let pub_count = omni["published_count"]
        .as_u64()
        .unwrap_or(published.len() as u64);
    let mut combined_raw = metrics.atoms;
    combined_raw.extend(published);
    let combined = normalize_session_atoms(combined_raw);
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

pub(super) fn selected_peer_set(key: &str) -> BTreeSet<String> {
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
    // A follow-up may edit these three, so a poll keeps what the person
    // typed until the session's own values or the selection change.
    let signature = json!([
        m::read(|s| s.epoch),
        session["id"],
        omni["pull_branches"],
        omni["query"],
        omni["auto_publish"]
    ]);
    if m::data("omnigraph-followup-signature") != signature {
        m::set("omnigraph-followup-signature", signature);
        let pulls: Vec<String> = m::array(&omni["pull_branches"])
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        dom::set_value("omnigraph-pull-branches", &pulls.join(", "));
        dom::set_value("omnigraph-session-query", &m::text(omni, "query"));
        dom::set_checked(
            "omnigraph-auto-publish",
            omni["auto_publish"].as_bool().unwrap_or(true),
        );
    }
    dom::set_value(
        "omnigraph-shared-space",
        &m::omnigraph_shared_space(session),
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
    // A follow-up may change what the next turn pulls and publishes.
    let follow_edit = !locked && !sid.is_empty() && m::read(|s| m::omnigraph_enabled(&s.selected));
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
        dom::disabled(
            id,
            disable_edit && !(follow_edit && FOLLOWUP_FIELDS.contains(&id)),
        );
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

/// The composer fields a follow-up may change for its session.
pub(super) const FOLLOWUP_FIELDS: [&str; 3] = [
    "omnigraph-pull-branches",
    "omnigraph-session-query",
    "omnigraph-auto-publish",
];

/// The selected session's follow-up change to its Omnigraph settings.
pub fn followup_payload(session: &V) -> Option<V> {
    followup_payload_from_fields(
        session,
        &dom::value("omnigraph-pull-branches"),
        &dom::value("omnigraph-session-query"),
        dom::checked("omnigraph-auto-publish"),
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
    let metrics = session_bar_metrics(session);
    let sid = m::text(session, "id");
    let connected = m::yes(&m::data("omnigraph-status")["connected"]);
    let show_ready_bar = !visible && !sid.is_empty() && connected;
    dom::hidden("session-omnigraph-bar", !visible && !show_ready_bar);
    if !visible && !show_ready_bar {
        dom::error("session-omnigraph-error", "");
        dom::hidden("session-omnigraph-meter", true);
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
        dom::hidden("session-omnigraph-meter", true);
        dom::hidden("session-omnigraph-merge", true);
        dom::hidden("session-omnigraph-peers", true);
        dom::hidden("session-omnigraph-atoms", true);
        return;
    }
    dom::attr("session-omnigraph-bar", "data-active", "true");
    dom::hidden("session-omnigraph-atoms", false);
    dom::hidden("session-omnigraph-meter", false);
    dom::text("session-omnigraph-branch", &branch);
    dom::text("session-omnigraph-meta", &meta);
    dom::error("session-omnigraph-error", &session_bar_error(session));
    dom::hidden("session-omnigraph-merge", !can_merge);
    dom::disabled("session-omnigraph-merge", m::active(session));

    let pct = ((metrics.capsule_chars as f64 / metrics.capsule_limit as f64) * 100.0).min(100.0);
    dom::attr(
        "session-omnigraph-meter-bar",
        "aria-valuenow",
        &metrics.capsule_chars.to_string(),
    );
    dom::attr(
        "session-omnigraph-meter-bar",
        "data-over",
        if metrics.capsule_chars > metrics.capsule_limit {
            "true"
        } else {
            "false"
        },
    );
    if let Some(fill) = dom::get("session-omnigraph-meter-fill")
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = fill.style().set_property("width", &format!("{pct:.1}%"));
    }
    dom::text(
        "session-omnigraph-chars",
        &format!(
            "{} / {} chars",
            format_count(metrics.capsule_chars),
            format_count(metrics.capsule_limit)
        ),
    );
    let total_injected = metrics.verified_count + metrics.unverified_count;
    dom::hidden("session-omnigraph-verified", total_injected == 0);
    dom::text(
        "session-omnigraph-verified",
        &format!("{} verified", metrics.verified_count),
    );
    dom::hidden(
        "session-omnigraph-unverified",
        metrics.unverified_count == 0,
    );
    dom::text(
        "session-omnigraph-unverified",
        &format!("{} unverified reference", metrics.unverified_count),
    );

    let all_sessions = m::read(|s| s.sessions.clone());
    let (show_peers, peers_html) = render_session_peers_html(session, &all_sessions);
    dom::hidden("session-omnigraph-peers", !show_peers);
    if let Some(peers_el) = dom::get("session-omnigraph-peers") {
        peers_el.set_inner_html(&peers_html);
    }

    if let Some(host) = dom::get("session-omnigraph-atoms") {
        let expanded = !sid.is_empty()
            && m::data("omnigraph-session-atoms-expanded").as_str() == Some(sid.as_str());
        host.set_inner_html(&render_session_atoms_html(&atoms, &branch, expanded));
    }
}

/// How many shared atoms the session bar shows before `+N more`.
pub(super) const SESSION_ATOMS_SHOWN: usize = 10;

/// The session bar's shared atoms: the first ten, then a `+N more` toggle
/// that shows the rest and `Show fewer` that folds them again.
pub fn render_session_atoms_html(atoms: &[V], branch: &str, expanded: bool) -> String {
    if atoms.is_empty() {
        return "<span class=\"knowledge-note\">No shared atoms injected yet.</span>".into();
    }
    let shown = if expanded {
        atoms.len()
    } else {
        atoms.len().min(SESSION_ATOMS_SHOWN)
    };
    let mut html = String::new();
    for atom in &atoms[..shown] {
        let id = atom_id(atom);
        let kind = atom["kind"].as_str().unwrap_or("finding");
        let title = atom["title"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(&id);
        html.push_str(&format!(
            "<button type=\"button\" class=\"omni-session-atom-chip\" data-action=\"omnigraph-open-atom\" data-atom-id=\"{}\" data-branch=\"{}\" data-verified=\"{}\"><span aria-hidden=\"true\">{}</span> {}</button>",
            esc(&id),
            esc(branch),
            if atom_is_verified(atom) { "true" } else { "false" },
            kind_glyph(kind),
            esc(title)
        ));
    }
    if atoms.len() > SESSION_ATOMS_SHOWN {
        html.push_str(&format!(
            "<button type=\"button\" class=\"omni-session-atom-more\" data-action=\"omnigraph-session-atoms-toggle\" data-payload=\"{}\" aria-controls=\"session-omnigraph-atoms\" aria-expanded=\"{expanded}\">{}</button>",
            esc(&json!({"expanded": !expanded}).to_string()),
            if expanded {
                "Show fewer".to_owned()
            } else {
                format!("+{} more", atoms.len() - SESSION_ATOMS_SHOWN)
            }
        ));
    }
    html
}
