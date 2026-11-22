use super::*;

pub(super) fn base64(data: &[u8]) -> String {
    const C: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | (chunk.get(2).copied().unwrap_or(0) as u32);
        s.push(C[((n >> 18) & 63) as usize] as char);
        s.push(C[((n >> 12) & 63) as usize] as char);
        s.push(if chunk.len() > 1 {
            C[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        s.push(if chunk.len() > 2 {
            C[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    s
}
pub(super) fn render_files() {
    dom::clear("attachment-list");
    FILES.with(|f| {
        for (index, file) in f.borrow().iter().enumerate() {
            let chip = dom::el(
                "div",
                "attachment-chip",
                &format!("{} · {} bytes", file.name(), file.size()),
            );
            dom::add(&chip, &dom::button("×", "attachment-remove", json!(index)));
            dom::append("attachment-list", &chip);
        }
    });
    dom::hidden("attachment-list", FILES.with(|f| f.borrow().is_empty()));
}
pub(super) fn clear_files() {
    FILES.with(|f| f.borrow_mut().clear());
    dom::set_value("attachment-input", "");
    render_files();
}
pub(super) fn add_files() {
    if let Some(input) =
        dom::get("attachment-input").and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        if let Some(files) = input.files() {
            let chosen: Vec<_> = (0..files.length()).filter_map(|i| files.item(i)).collect();
            input.set_value("");
            stage_files(chosen, "attachment-error");
        }
    }
}
pub(super) fn stage_files(files: impl IntoIterator<Item = web_sys::File>, error: &str) -> bool {
    let mut all = FILES.with(|f| f.borrow().clone());
    all.extend(files);
    if all.len() > 5
        || all.iter().any(|f| f.size() > 4194304.)
        || all.iter().map(|f| f.size()).sum::<f64>() > 8388608.
    {
        dom::error(
            error,
            "Choose up to 5 files, at most 4 MiB each and 8 MiB total.",
        );
        return false;
    }
    FILES.with(|f| *f.borrow_mut() = all);
    dom::error(error, "");
    dom::error("attachment-error", "");
    render_files();
    true
}

pub fn attach_map_context(project: &str, text: &str) -> bool {
    let error = "graph-map-error";
    if project.is_empty()
        || project != dom::value("project")
        || m::read(|s| s.pending || s.auth_failed || s.boot.is_null() || m::active(&s.selected))
    {
        dom::error(
            error,
            "Open an available session draft for this project to attach map context.",
        );
        return false;
    }
    let name = "dependency-map.txt";
    if FILES.with(|files| files.borrow().iter().any(|file| file.name() == name)) {
        dom::error(
            error,
            "Remove the existing dependency-map attachment to replace it.",
        );
        return false;
    }
    let content = js_sys::Array::new();
    content.push(&JsValue::from_str(text));
    match web_sys::File::new_with_str_sequence(&content, name) {
        Ok(file) => stage_files([file], error),
        Err(_) => {
            dom::error(error, "Could not create the dependency-map attachment.");
            false
        }
    }
}

pub(crate) fn merge_draft_text(existing: &str, block: &str, max_chars: usize) -> String {
    const MARKER: &str = "[System Architecture & Blast-Radius Context]";
    let clean_block = block.trim();
    if clean_block.is_empty() {
        return existing.to_owned();
    }
    let base = if let Some(idx) = existing.find(MARKER) {
        existing[..idx].trim_end()
    } else {
        existing.trim_end()
    };
    let combined = if base.is_empty() {
        clean_block.to_owned()
    } else {
        format!("{base}\n\n{clean_block}")
    };
    if max_chars > 0 && combined.chars().count() > max_chars {
        combined.chars().take(max_chars).collect()
    } else {
        combined
    }
}

pub(crate) fn append_draft_block(textarea_id: &str, block: &str, max_chars: usize) -> bool {
    if block.trim().is_empty() || dom::get(textarea_id).is_none() {
        return false;
    }
    let next = merge_draft_text(&dom::value(textarea_id), block, max_chars);
    dom::set_value(textarea_id, &next);
    dom::focus(textarea_id);
    true
}

pub async fn send() {
    if inbox::needs_confirmation() || m::read(|s| m::active(&s.selected)) {
        if FILES.with(|files| !files.borrow().is_empty()) {
            dom::error(
                "composer-error",
                "Remove attachments before queueing a plain-text next message.",
            );
        } else {
            inbox::submit().await;
        }
        return;
    }
    if m::read(|s| s.pending || s.auth_failed || s.boot.is_null() || m::active(&s.selected))
        || !dom::validity("session-form")
    {
        return;
    }
    let draft = m::data("merge-draft");
    let (selected, selection_epoch) = m::read(|s| (s.sid.clone(), s.epoch));
    if draft.is_object()
        && !merge_draft_owned(&draft, &selected, &dom::value("project"), selection_epoch)
    {
        m::take("merge-draft");
        reset_merge_archive();
        controls();
        dom::error(
            "composer-error",
            "The selected project or chat changed. Choose the chats to merge again.",
        );
        return;
    }
    let prompt = dom::value("prompt").trim().to_owned();
    if commands::intercept(&prompt).await {
        return;
    }
    if prompt.is_empty() {
        return;
    }
    if !budget_fields_valid("session-budgets") || !valid_budgets(&budgets("session-budgets")) {
        dom::error(
            "session-budgets-error",
            "Use valid money, token and time limits.",
        );
        return;
    }
    let sid = m::read(|s| s.sid.clone());
    let follow = !sid.is_empty();
    if !follow
        && m::data("merge-draft").is_object()
        && (dom::value("workflow") != "native"
            || dom::value("workspace") != "project"
            || dom::checked("clash-enabled")
            || FILES.with(|f| !f.borrow().is_empty()))
    {
        dom::error("composer-error","A merged task uses an ordinary native chat in the project folder, without attachments or Clash.");
        return;
    }
    if !follow {
        let strict = dom::checked("brain-link-review") || dom::value("brain-link-kind") != "auto";
        let meta = m::data("brain-link-meta");
        if strict && (!m::yes(&meta["runtime_available"]) || meta["mode"] != "governed") {
            dom::error(
                "brain-link-error",
                "Choose a governed knowledge root before preparing a linked task.",
            );
            return;
        }
        if m::yes(&m::data("brain-link-loading")) {
            dom::error(
                "brain-link-error",
                "Wait for knowledge roots to finish loading.",
            );
            return;
        }
    }
    let workflow = dom::value("workflow");
    if workflow == "sdd"
        && !m::sdd_settings_valid(
            dom::value("sdd-feature").trim(),
            &dom::value("sdd-phase"),
            &m::read(|s| m::array(&s.boot["sdd_phases"])),
        )
    {
        dom::error(
            "sdd-error",
            "Choose a valid feature slug and SDD phase before sending.",
        );
        return;
    }
    let dry = workflow == "fleet-review" && dom::checked("fleet-dry-run");
    let agent_count = if dom::checked("agents-enabled") {
        dom::num("agent-count")
    } else {
        json!(m::read(|s| {
            s.boot["runtime"]["default_agent_count"]
                .as_u64()
                .unwrap_or(3)
        }))
    };
    let mut turn = json!({"prompt":prompt,"agents_enabled":dom::checked("agents-enabled"),"agent_count":agent_count,"model_routing":routing(),"model":if dry{V::Null}else{selected_model("")},"thinking_effort":if dry{V::Null}else{nullable(dom::value("thinking-effort"))},"permissions":dom::value("permissions")});
    if follow && workflow == "native" && !routing().is_null() {
        turn["mode"] = json!(dom::value("mode"));
    }
    if workflow == "sdd" {
        turn["sdd"] =
            json!({"feature":dom::value("sdd-feature").trim(),"phase":dom::value("sdd-phase")});
    }
    if dom::checked("clash-enabled") {
        turn["clash"] = json!({"challenger":dom::value("clash-challenger"),"rounds":dom::num("clash-rounds"),"challenger_model":selected_model("clash-"),"challenger_thinking_effort":nullable(dom::value("clash-thinking-effort"))});
    } else if follow {
        turn["clash"] = V::Null;
    }
    if !follow {
        for (k, v) in [
            ("budgets", budgets("session-budgets")),
            ("project_id", json!(dom::value("project"))),
            ("provider", json!(dom::value("provider"))),
            ("mode", json!(dom::value("mode"))),
            ("workflow", json!(workflow)),
            ("project_context", json!(true)),
            ("workspace", json!(dom::value("workspace"))),
        ] {
            turn[k] = v;
        }
        if dom::value("workspace") == "worktree" && !dom::value("worktree-branch").trim().is_empty()
        {
            turn["worktree_branch"] = json!(dom::value("worktree-branch").trim());
        }
        if dom::value("workspace") == "existing-worktree" {
            turn["worktree_id"] = json!(dom::value("existing-worktree"));
        }
        if workflow == "fleet-review" {
            turn["fleet"] = json!({"lenses":dom::selected_checks("fleet-lenses"),"dry_run":dom::checked("fleet-dry-run"),"budget_usd":dom::num("fleet-budget"),"worker_timeout":dom::num("fleet-worker-timeout")});
        }
        if let Some(brain) = knowledge::brain_config() {
            turn["brain"] = brain;
        }
        if let Some(omni) = omnigraph::session_payload() {
            turn["omnigraph"] = omni;
        }
        if let Some(option) = crate::skillcache::turn_request(false) {
            turn["skillcache"] = option;
        }
    } else {
        // A follow-up carries only the Omnigraph and task-context settings
        // the person changed for the next turn.
        let selected = m::read(|s| s.selected.clone());
        if let Some(omni) = omnigraph::followup_payload(&selected) {
            turn["omnigraph"] = omni;
        }
        if let Some(option) = crate::skillcache::turn_request(true) {
            turn["skillcache"] = option;
        }
        match knowledge::followup_brain(&selected) {
            Ok(Some(brain)) => turn["brain"] = brain,
            Ok(None) => {}
            Err(message) => {
                dom::error("composer-error", message);
                return;
            }
        }
    }
    m::write(|s| s.pending = true);
    controls();
    let files = FILES.with(|f| f.borrow().clone());
    let mut encoded = vec![];
    for file in files {
        match JsFuture::from(file.array_buffer()).await {
            Ok(a) => encoded.push(
                json!({"name":file.name(),"data":base64(&js_sys::Uint8Array::new(&a).to_vec())}),
            ),
            Err(_) => {
                dom::error(
                    "attachment-error",
                    &format!("Could not read {}. Attach it again.", file.name()),
                );
                m::write(|s| s.pending = false);
                controls();
                return;
            }
        }
    }
    if !encoded.is_empty() {
        turn["attachments"] = json!(encoded);
    }
    let draft = m::data("merge-draft");
    let path;
    if !follow && draft.is_object() {
        let signature = turn.to_string();
        let mut draft = draft;
        if draft["signature"].is_string() && draft["signature"] != signature {
            draft["request_id"] = json!(m::request_identity());
        }
        draft["signature"] = json!(signature);
        let body = json!({"source_ids":draft["sources"],"request_id":draft["request_id"],"destination":turn});
        m::set("merge-draft", draft);
        turn = body;
        path = "/api/sessions/merge".into();
    } else {
        path = if follow {
            format!("/api/sessions/{}/messages", dom::enc(&sid))
        } else {
            "/api/sessions".into()
        };
    }
    m::write(|s| s.pending = true);
    controls();
    dom::error("composer-error", "");
    dom::cancel_timer("preflight-debounce");
    crate::graph_map::focus_from_prompt(&prompt);
    crate::graph::run_state(true);
    match api::post(&path, turn).await {
        Ok(v) => {
            let session = &v["session"];
            if !follow && !session["id"].is_string() {
                dom::error(
                    "composer-error",
                    "No session returned. Refresh the list before retrying.",
                );
            } else {
                m::upsert(session.clone());
                if session["graph_focus"].is_object()
                    && !should_keep_predicting(
                        &m::data("session-graph-focus"),
                        &session["graph_focus"],
                    )
                {
                    crate::graph_map::apply_focus(&session["graph_focus"]);
                }
                if follow {
                    inbox::clear_draft(&sid, &prompt);
                }
                dom::set_value("prompt", "");
                reset_preflight(false);
                clear_files();
                reset_session_settings_state();
                m::write(|s| s.pending = false);
                if follow {
                    let e = m::read(|s| s.epoch);
                    poll(e).await;
                } else {
                    m::take("merge-draft");
                    omnigraph::clear_draft_peers();
                    if session["omnigraph"]["linked_session_ids"].is_array() {
                        refresh().await;
                    }
                    select(&m::text(session, "id")).await;
                }
            }
        }
        Err(e) => dom::error("composer-error", &e.message),
    }
    m::write(|s| s.pending = false);
    controls();
    crate::graph::run_state(m::read(|s| m::active(&s.selected)));
}
pub async fn session_action(action: &str, body: V, error: &str) {
    let sid = m::read(|s| s.sid.clone());
    if sid.is_empty() || m::read(|s| s.pending) {
        return;
    }
    stop_poll();
    m::write(|s| s.pending = true);
    controls();
    match api::post(
        &format!("/api/sessions/{}/{}", dom::enc(&sid), action),
        body,
    )
    .await
    {
        Ok(v) => {
            if v["session"].is_object() {
                m::upsert(v["session"].clone());
            }
        }
        Err(e) => dom::error(error, &e.message),
    }
    m::write(|s| s.pending = false);
    let e = m::read(|s| s.epoch);
    poll(e).await;
    controls();
}
pub(crate) fn format_project_git_status(v: &V) -> String {
    let explicit = m::summary(v);
    if !explicit.is_empty() {
        return explicit;
    }
    if !m::yes(&v["is_git"]) {
        return v["reason"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Select a Git working directory to use a worktree.")
            .to_owned();
    }
    let branch = v["branch"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("HEAD (detached)");
    if m::yes(&v["worktree_available"]) {
        format!("Git branch: {branch} · Worktrees ready")
    } else if let Some(reason) = v["reason"].as_str().filter(|s| !s.trim().is_empty()) {
        format!("Git branch: {branch} · {reason}")
    } else {
        format!("Git branch: {branch} · Worktrees unavailable")
    }
}

pub(crate) fn format_workspace_hint(workspace: &str, git: &V) -> String {
    match workspace {
        "worktree" => {
            "Creates an isolated Git worktree from committed HEAD for this session.".into()
        }
        "existing-worktree" => "Reuses an existing Git worktree listed for this repository.".into(),
        _ if git.is_object() && !m::yes(&git["worktree_available"]) => git["reason"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("The agent runs in the selected project folder.")
            .to_owned(),
        _ => "The agent runs in the selected project folder.".into(),
    }
}

pub async fn load_git() {
    let p = dom::value("project");
    if p.is_empty() {
        return;
    }
    crate::graph_map::sync_embedded(false);
    spawn_local(async {
        crate::graph::load(true).await;
    });
    let mut synced_changes = false;
    for kind in ["git", "worktrees"] {
        let path = format!("/api/projects/{}/{kind}", dom::enc(&p));
        if let Ok((e, v)) = api::scoped(&format!("project-{kind}"), &path).await {
            if !m::current(&format!("project-{kind}"), e) || dom::value("project") != p {
                continue;
            }
            m::set(&format!("project-{kind}"), v.clone());
            if kind == "git" {
                dom::text("project-git-status", &format_project_git_status(&v));
                crate::graph_map::sync_embedded_changes();
                synced_changes = true;
            } else {
                dom::options(
                    "existing-worktree",
                    &m::array(&v["worktrees"]),
                    |v| format!("{} · {}", m::text(v, "branch"), m::text(v, "path")),
                    None,
                );
            }
        }
    }
    if !synced_changes && dom::value("project") == p {
        crate::graph_map::sync_embedded_changes();
    }
    controls();
    knowledge::load_brain_link().await;
    management::load_attachment().await;
}
pub(super) fn provider_sign_in_note(sign_in: &V, provider_id: &str, provider_name: &str) -> String {
    let state = &sign_in["states"][provider_id];
    if state["state"] == "signed_out" {
        let name = if provider_name.is_empty() {
            provider_id
        } else {
            provider_name
        };
        format!(
            "{name} is not signed in. Sign in from a terminal with {}, then send again.",
            state["login"].as_str().unwrap_or("the provider CLI")
        )
    } else {
        String::new()
    }
}
pub(super) fn refresh_provider_hint() {
    let chosen = dom::value("provider");
    let sign_in = m::data("sign-in");
    let note = provider_sign_in_note(&sign_in, &chosen, &m::summary(&m::provider(&chosen)));
    dom::text("provider-hint", &note);
    dom::hidden("provider-hint", note.is_empty());
}
pub async fn sign_in(refresh: bool) {
    if m::read(|s| s.boot.is_null() || s.auth_failed) || m::yes(&m::data("sign-in-pending")) {
        return;
    }
    let fresh = if refresh { 3000. } else { 30000. };
    if m::data("sign-in-at")
        .as_f64()
        .is_some_and(|at| js_sys::Date::now() - at < fresh)
    {
        return;
    }
    m::set("sign-in-pending", json!(true));
    sync_working_indicator();
    let path = format!(
        "/api/providers/sign-in{}",
        if refresh { "?refresh=1" } else { "" }
    );
    if let Ok(mut v) = api::get(&path).await {
        let mut states = json!({});
        for provider in m::array(&v["providers"]) {
            if let Some(id) = provider["id"].as_str() {
                states[id] = provider.clone();
            }
        }
        v["states"] = states;
        m::set("sign-in", v);
        m::set("sign-in-at", json!(js_sys::Date::now()));
        refresh_provider_hint();
    }
    m::set("sign-in-pending", json!(false));
    sync_working_indicator();
}
// Updates of this clone, offered as desktop applications offer theirs: an Update button in the
// header while the branch the clone follows has moved on. A click fast-forwards the clone and
// restarts the server, and the page reloads itself on the new version. "app-update" holds the
// server's status (updates.rs), "app-update-page" what this page did: pending, restarting,
// failed, its note and the server instance that answered before the update.
pub async fn update_status(check: bool) {
    let page = m::data("app-update-page");
    if !(m::yes(&page["pending"])
        || m::yes(&page["restarting"])
        || m::read(|s| s.boot.is_null() || s.auth_failed))
    {
        if let Ok(v) = api::get(&format!(
            "/api/app/update{}",
            if check { "?check=1" } else { "" }
        ))
        .await
        {
            m::set("app-update", v);
            render_update();
        }
        if check {
            // A check fetches in the background: its answer is read once it has had time to arrive.
            dom::later("app-update-read", 8000, || {
                spawn_local(async {
                    update_status(false).await;
                })
            });
        }
    }
    dom::later("app-update", 600000, || {
        spawn_local(async {
            update_status(true).await;
        })
    });
}
#[derive(Debug, PartialEq)]
pub(super) struct UpdateView {
    pub(super) button: bool,
    pub(super) busy: bool,
    pub(super) label: &'static str,
    pub(super) title: String,
    pub(super) note: String,
    pub(super) error: bool,
}
pub(super) fn update_view(status: &V, page: &V) -> UpdateView {
    let state = status["state"].as_str().unwrap_or("");
    let (pending, restarting) = (m::yes(&page["pending"]), m::yes(&page["restarting"]));
    let mut title = vec![];
    if state == "available" {
        let commits = m::array(&status["commits"]);
        let shown = commits.len().min(10);
        title.push(m::text(status, "detail"));
        title.extend(
            commits[..shown]
                .iter()
                .map(|commit| format!("• {}", m::text(commit, "subject"))),
        );
        let more = status["behind"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(shown as u64);
        if more > 0 {
            title.push(format!("… and {more} more"));
        }
    }
    let note = m::text(page, "note");
    let (note, error) = if !note.is_empty() {
        (note, m::yes(&page["failed"]))
    } else if ["diverged", "failed", "restart-required"].contains(&state) {
        (m::text(status, "detail"), state != "diverged")
    } else {
        (String::new(), false)
    };
    UpdateView {
        // A restart the server refused is pressed again once the clone is rebuilt.
        button: pending || restarting || ["available", "restart-required"].contains(&state),
        busy: pending || restarting,
        label: if restarting {
            "Restarting…"
        } else if pending {
            "Updating…"
        } else {
            "Update"
        },
        title: title
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        note,
        error,
    }
}
/// The pieces of a server note, `true` for a command it quotes in backticks.
pub(super) fn code_spans(note: &str) -> Vec<(bool, &str)> {
    let pieces = note.split('`').collect::<Vec<_>>();
    if pieces.len() % 2 == 0 {
        return vec![(false, note)];
    }
    pieces
        .into_iter()
        .enumerate()
        .filter(|(_, piece)| !piece.is_empty())
        .map(|(index, piece)| (index % 2 == 1, piece))
        .collect()
}
pub(super) fn render_update() {
    let view = update_view(&m::data("app-update"), &m::data("app-update-page"));
    dom::hidden("app-update", !view.button);
    dom::disabled("app-update", view.busy);
    dom::text("app-update", view.label);
    dom::attr("app-update", "title", &view.title);
    // Rewritten only when it changes, so the status region is not announced again.
    if let Some(note) = dom::get("app-update-note") {
        if note.get_attribute("data-note").as_deref() != Some(view.note.as_str()) {
            note.set_text_content(None);
            for (code, piece) in code_spans(&view.note) {
                if code {
                    dom::add(&note, &dom::el("code", "", piece));
                } else {
                    let _ = note.append_with_str_1(piece);
                }
            }
            let _ = note.set_attribute("data-note", &view.note);
        }
    }
    dom::hidden("app-update-banner", view.note.is_empty());
    dom::class("app-update-banner", "error", view.error);
    sync_working_indicator();
}
pub(super) async fn apply_update() {
    let page = m::data("app-update-page");
    if m::yes(&page["pending"]) || m::yes(&page["restarting"]) {
        return;
    }
    m::set("app-update-page", json!({"pending":true}));
    render_update();
    // The restarted server answers health with another instance.
    let result = match api::get("/api/health").await {
        Ok(health) => api::post("/api/app/update", json!({}))
            .await
            .map(|v| (health["instance"].clone(), v)),
        Err(error) => Err(error),
    };
    let mut status = m::data("app-update");
    let page = match result {
        Ok((_, v)) if m::yes(&v["current"]) => {
            status["state"] = "current".into();
            json!({"note":v["detail"]})
        }
        Ok((instance, v)) => {
            status["state"] = "updated".into();
            let to = m::text(&v, "to");
            let updated = if to.is_empty() {
                "Updated.".to_owned()
            } else {
                format!("Updated to {to}.")
            };
            if m::yes(&v["restarting"]) {
                json!({"restarting":true,"instance":instance,"note":format!("{updated} The Harness is restarting; this page reloads by itself.")})
            } else {
                json!({"note":format!("{updated} Restart the Harness to use it.")})
            }
        }
        Err(error) => json!({"failed":true,"note":error.message}),
    };
    let (restarting, failed) = (m::yes(&page["restarting"]), m::yes(&page["failed"]));
    m::set("app-update", status);
    m::set("app-update-page", page);
    render_update();
    if restarting {
        wait_for_restart(js_sys::Date::now() + 120000.);
    } else if failed {
        // The failure may have changed the server's state, such as a restart it now requires.
        update_status(false).await;
    }
}
pub(super) fn wait_for_restart(deadline: f64) {
    dom::later("app-restart", 1000, move || {
        spawn_local(async move {
            if let Ok(health) = api::get("/api/health").await {
                if health["instance"].is_string()
                    && health["instance"] != m::data("app-update-page")["instance"]
                {
                    let _ = dom::window().location().reload();
                    return;
                }
            }
            if js_sys::Date::now() < deadline {
                wait_for_restart(deadline);
            } else {
                m::set(
                    "app-update-page",
                    json!({"failed":true,"note":"The Harness did not come back after the update. Start it again from the application icon."}),
                );
                render_update();
            }
        })
    });
}
