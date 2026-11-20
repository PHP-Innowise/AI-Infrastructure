use super::*;

pub(super) fn result_view() -> bool {
    m::read(|s| ["changes", "checks", "usage"].contains(&s.view.as_str()))
}

pub(super) fn results_relevant() -> bool {
    result_view()
        || m::read(|s| {
            s.view == "sessions" && matches!(s.workspace.right_tab.as_str(), "diff" | "checks")
        })
}

pub(super) fn reset_results() {
    dom::cancel_timer("results-poll");
    m::next("results");
    m::take("results");
    m::take("delivery");
    m::take("results-memory-sid");
    m::take("results-memory-dirty");
    m::write(|s| {
        s.diff_view.results_retry_count = 0;
        s.diff_view.focused_file.clear();
    });
    invalidate_delivery();
    dom::hidden("delivery-options", true);
    dom::clear("delivery-files");
    dom::set_value("delivery-message", "");
    dom::clear("delivery-source");
    dom::clear("delivery-target");
    dom::text("delivery-status", "");
    dom::set_value("results-memory-progress", "");
    dom::set_value("results-memory-next-steps", "");
    dom::clear("results-memory-learnings");
    dom::set_value("results-memory-used", "");
    dom::set_checked("results-memory-verified", false);
    dom::error("results-memory-error", "");
    dom::clear("results-memory-feedback");
    dom::hidden("results-memory-feedback", true);
    dom::clear("workspace-diff-files");
    dom::clear("workspace-diff-viewer");
    dom::clear("workspace-checks-list");
}

pub(super) fn active_diff_pane_target(
    view: &str,
    right_tab: &str,
) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    match (view, right_tab) {
        ("changes", _) => Some((
            "results-files",
            "results-diff",
            "results-diff",
            "results-diff-note",
        )),
        ("sessions", "diff") => Some((
            "workspace-diff-files",
            "workspace-diff-viewer",
            "workspace-results",
            "workspace-diff-note",
        )),
        _ => None,
    }
}

pub(super) fn active_checks_pane_target(
    view: &str,
    right_tab: &str,
) -> Option<(&'static str, &'static str)> {
    match (view, right_tab) {
        ("checks", _) => Some(("results-checks", "results-status")),
        ("sessions", "checks") => Some(("workspace-checks-list", "workspace-checks-status")),
        _ => None,
    }
}

pub(super) fn files_summary_hash(scope_id: &str, files: &[V]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    feed(scope_id.as_bytes());
    feed(b":");
    for file in files {
        feed(m::text(file, "status").as_bytes());
        feed(b"\x1f");
        feed(m::text(file, "path").as_bytes());
        feed(b"\x1e");
    }
    format!("{hash:016x}:{}", files.len())
}

fn set_workspace_note_error(id: &str, msg: &str, is_error: bool) {
    dom::text(id, msg);
    if let Some(el) = dom::get(id) {
        if is_error {
            let _ = el.set_attribute("data-error", "true");
        } else {
            let _ = el.remove_attribute("data-error");
        }
    }
}

pub(crate) fn results_poll_delay_ms(
    sse_connected: bool,
    session_active: bool,
    checks_active: bool,
) -> Option<i32> {
    if !session_active && !checks_active {
        return None;
    }
    if sse_connected {
        // SSE delivers `results_check` and `session_status`; avoid 1.2s polling loops when connected.
        if checks_active {
            Some(15_000)
        } else {
            None
        }
    } else {
        Some(1200)
    }
}

pub(super) fn render_results_files_into(container_id: &str, scope_id: &str, files: &[V]) {
    let Some(container) = dom::get(container_id) else {
        return;
    };
    let hash = files_summary_hash(scope_id, files);
    let already_rendered = container.get_attribute("data-files-hash").as_deref()
        == Some(hash.as_str())
        && (files.is_empty() || container.first_element_child().is_some());
    if already_rendered {
        return;
    }
    dom::clear(container_id);
    let _ = container.set_attribute("data-files-hash", &hash);
    if files.is_empty() {
        dom::text(container_id, "No changed files listed.");
        return;
    }
    for file in files {
        let status = m::text(file, "status");
        let path = m::text(file, "path");
        let row = dom::el("div", "results-file-row", "");
        let label = dom::el("span", "results-file-name", &format!("{status}  {path}  "));
        dom::add(&row, &label);
        if !path.is_empty() {
            let jump = dom::button(
                "View diff",
                "results-jump-diff",
                json!({"scope": scope_id, "path": path}),
            );
            jump.set_class_name("button compact results-file-jump");
            dom::add(&row, &jump);
            let btn = dom::button(
                "Inspect in graph",
                "inspect-file-graph",
                json!({"path": path}),
            );
            btn.set_class_name("button compact results-file-inspect");
            let _ = btn.set_attribute("data-path", &path);
            dom::add(&row, &btn);
        }
        dom::add(&container, &row);
    }
}

pub async fn results() {
    dom::cancel_timer("results-poll");
    if !results_relevant() {
        return;
    }
    if result_view() {
        m::take("delivery");
        invalidate_delivery();
        dom::hidden("delivery-options", true);
        result_controls();
    }
    let sid = m::read(|s| s.sid.clone());
    if sid.is_empty() {
        dom::error("results-error", "Choose a session first.");
        set_workspace_note_error(
            "workspace-diff-note",
            "Select or start a session to view changes.",
            true,
        );
        set_workspace_note_error(
            "workspace-checks-status",
            "Select or start a session to view checks.",
            true,
        );
        return;
    }
    let key = "results";
    let path = format!("/api/sessions/{}/results", dom::enc(&sid));
    match api::scoped(key, &path).await {
        Ok((e, v)) if m::current(key, e) && m::read(|s| s.sid == sid) && results_relevant() => {
            m::write(|s| s.diff_view.results_retry_count = 0);
            dom::error("results-error", "");
            m::set("results", v.clone());
            let (view, right_tab) = m::read(|s| (s.view.clone(), s.workspace.right_tab.clone()));
            let snapshot = &v["snapshot"];
            if result_view() {
                dom::text("results-workspace", &m::text(&v, "workspace"));
                dom::hidden(
                    "delivery-section",
                    m::read(|s| s.selected["workspace"] != "worktree"),
                );
            }
            let checks = m::array(&v["checks"]);
            let checking = checks.iter().any(m::active);
            let status_msg = if checking {
                "Verification is queued or running; agent outcome stays separate."
            } else if m::read(|s| m::active(&s.selected)) {
                "Agent is active. Refresh the diff after this launch finishes."
            } else {
                "Verification results are current to the recorded snapshots."
            };
            if result_view() {
                dom::text("results-status", status_msg);
            }
            if let Some((files_id, diff_id, scope_id, note_id)) =
                active_diff_pane_target(&view, &right_tab)
            {
                if view == "changes" {
                    dom::text(
                        "results-base",
                        &if snapshot["available"] == true {
                            format!(
                                "Branch: {} · Base: {}{}{}",
                                snapshot["branch"].as_str().unwrap_or("detached HEAD"),
                                m::text(snapshot, "base"),
                                if snapshot["baseline_recorded"] == true {
                                    " (first launch)"
                                } else {
                                    " (current HEAD; original baseline unavailable)"
                                },
                                if snapshot["preexisting_changes"] == true {
                                    " · Workspace already had uncommitted changes at first launch."
                                } else {
                                    ""
                                }
                            )
                        } else {
                            String::new()
                        },
                    );
                }
                let diff_note = format!(
                    "{}{}",
                    m::text(snapshot, "message"),
                    if snapshot["available"] == true && snapshot["complete"] != true {
                        " Preview is truncated or omits file contents; verification freshness cannot be certified."
                    } else {
                        ""
                    }
                );
                set_workspace_note_error(note_id, &diff_note, false);
                render_results_files_into(files_id, scope_id, &m::array(&snapshot["files"]));
                let diff_text = snapshot["diff"].as_str().unwrap_or("");
                if diff_text.trim().is_empty() {
                    crate::run::render_empty_diff_into(diff_id, "No text diff available.");
                } else {
                    crate::run::render_grouped_diff_into(diff_id, diff_text, scope_id);
                }
            }
            if view == "usage" {
                dom::text("results-usage-note", &m::text(&v, "notice"));
                let totals = &v["totals"];
                let amount = |key: &str, suffix: &str| {
                    let t = &totals[key];
                    if t["unknown_launches"].as_u64().unwrap_or(0) > 0
                        && t["reported"].as_f64().unwrap_or(0.) == 0.
                    {
                        "—".into()
                    } else {
                        t["reported"]
                            .as_f64()
                            .map(|n| {
                                if key == "cost_usd" {
                                    format!("≈ {}{n:.4}", "$")
                                } else {
                                    format!("{n:.0}{suffix}")
                                }
                            })
                            .unwrap_or_else(|| "—".into())
                    }
                };
                let unknown = ["tokens", "cost_usd"]
                    .iter()
                    .filter_map(|key| {
                        totals[*key]["unknown_launches"]
                            .as_u64()
                            .filter(|n| *n > 0)
                            .map(|n| {
                                format!(
                                    "{} not reported for {n} launches",
                                    if *key == "cost_usd" { "cost" } else { "tokens" }
                                )
                            })
                    })
                    .collect::<Vec<_>>();
                dom::text(
                    "results-totals",
                    &format!(
                        "{} · {} · {}{}",
                        amount("tokens", " tokens"),
                        amount("cost_usd", ""),
                        amount("seconds", " s"),
                        if unknown.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", unknown.join(" · "))
                        }
                    ),
                );
            }
            if let Some((checks_list_id, checks_status_id)) =
                active_checks_pane_target(&view, &right_tab)
            {
                set_workspace_note_error(checks_status_id, status_msg, false);
                if checks.is_empty() {
                    let already_empty = dom::get(checks_list_id).is_some_and(|c| {
                        c.child_element_count() == 1
                            && c.first_element_child().is_some_and(|f| f.tag_name() == "P")
                    });
                    if !already_empty {
                        dom::clear(checks_list_id);
                        dom::append(
                            checks_list_id,
                            &dom::el("p", "knowledge-note", "No independent checks run yet."),
                        );
                    }
                } else {
                    if let Some(c) = dom::get(checks_list_id) {
                        if let Some(first) = c.first_element_child() {
                            if first.tag_name() == "P" && first.get_attribute("data-key").is_none()
                            {
                                dom::clear(checks_list_id);
                            }
                        }
                    }
                    dom::keyed_render(
                        checks_list_id,
                        &checks,
                        |c| m::text(c, "id"),
                        |check| {
                            let status_str = m::text(check, "status");
                            let cmd_str = m::text(check, "command");
                            let d = dom::details(
                                &format!("{} · {cmd_str}", status_str.to_uppercase()),
                                "",
                            );
                            if ["queued", "running", "failed", "timed_out", "output_limit"]
                                .contains(&status_str.as_str())
                            {
                                let _ = d.set_attribute("open", "");
                            }
                            let current = snapshot["complete"] == true
                                && check["snapshot_complete"] == true
                                && snapshot["id"] == check["snapshot_id"]
                                && check["workspace_changed"] != true;
                            let freshness = if check["kind"] == "target" {
                                format!(
                                    "Target {} · tested commit {}{}",
                                    m::text(check, "target_branch"),
                                    m::text(check, "candidate"),
                                    if check["workspace_changed"] == true {
                                        " · Check changed the workspace"
                                    } else {
                                        ""
                                    }
                                )
                            } else if current {
                                "Matches the current diff".into()
                            } else if check["workspace_changed"] == true
                                || snapshot["id"].is_string()
                                    && check["snapshot_id"].is_string()
                                    && snapshot["id"] != check["snapshot_id"]
                            {
                                "Workspace changed since this check".into()
                            } else {
                                "Freshness unavailable".into()
                            };
                            dom::add(
                                &d,
                                &dom::el(
                                    "p",
                                    "knowledge-note",
                                    &format!(
                                        "{} · exit code {} · {}s · {freshness}",
                                        check["started_at"]
                                            .as_str()
                                            .or_else(|| check["created_at"].as_str())
                                            .unwrap_or("not recorded"),
                                        check["exit_code"]
                                            .as_i64()
                                            .map(|n| n.to_string())
                                            .unwrap_or_else(|| "unavailable".into()),
                                        check["seconds"]
                                            .as_f64()
                                            .map(|n| n.to_string())
                                            .unwrap_or_else(|| "unknown".into())
                                    ),
                                ),
                            );
                            let output_text = check["output"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .unwrap_or("Waiting for output…");
                            dom::add(&d, &dom::el("pre", "fleet-report-text", output_text));
                            if matches!(
                                status_str.as_str(),
                                "failed" | "timed_out" | "output_limit"
                            ) {
                                let req = crate::run::failure_localize_payload(check, output_text);
                                let btn = dom::button(
                                    "Localize failure on map",
                                    "localize-run-failure",
                                    req,
                                );
                                let _ = btn.class_list().add_1("compact");
                                dom::add(&d, &btn);
                            }
                            d
                        },
                    );
                }
            }
            if view == "usage" {
                dom::rows("results-launches", &m::array(&v["launches"]));
                knowledge::usage(&v);
            }
            if view == "changes" {
                delivery_load(&sid).await;
                if !m::current(key, e) || !m::read(|s| s.sid == sid) || !results_relevant() {
                    return;
                }
                sync_results_memory_prefill(&sid);
            }
            if result_view() {
                result_controls();
                let view = m::read(|s| s.view.clone());
                render_tabs(&view, "sessions");
            }
            if let Some(delay) = results_poll_delay_ms(
                api::sse_connected(),
                m::read(|s| m::active(&s.selected)),
                checks.iter().any(m::active),
            ) {
                dom::later("results-poll", delay, || {
                    spawn_local(async {
                        results().await;
                    })
                });
            }
        }
        Ok(_) => {}
        Err(e) if !e.aborted && m::read(|s| s.sid == sid) && results_relevant() => {
            dom::error("results-error", &e.message);
            set_workspace_note_error("workspace-diff-note", &e.message, true);
            set_workspace_note_error("workspace-checks-status", &e.message, true);
            if ![401, 403, 404].contains(&e.status) {
                let attempt = m::write(|s| {
                    s.diff_view.results_retry_count =
                        s.diff_view.results_retry_count.saturating_add(1);
                    s.diff_view.results_retry_count
                });
                if attempt <= 4 {
                    let delay = match attempt {
                        1 => 1500,
                        2 => 3000,
                        _ => 6000,
                    };
                    dom::later("results-poll", delay, || {
                        spawn_local(async {
                            results().await;
                        })
                    });
                }
            }
        }
        Err(_) => {}
    }
}
pub(super) fn result_controls() {
    let session = m::read(|s| s.selected.clone());
    let locked =
        m::read(|s| s.sid.is_empty() || s.pending || m::active(&s.selected) || s.auth_failed);
    let result = m::data("results");
    let command = dom::value("check-command");
    let timeout = dom::num("check-timeout").as_f64();
    let timeout_valid = timeout.is_some_and(|n| (1.0..=3600.).contains(&n) && n.fract() == 0.);
    dom::disabled(
        "check-start",
        locked
            || !result["snapshot"].is_object()
            || session["creator"].is_object()
            || command.trim().is_empty()
            || command.len() > 4000
            || !timeout_valid,
    );
    dom::disabled("check-command", locked || session["creator"].is_object());
    dom::disabled("check-timeout", locked);
    let delivery = m::data("delivery");
    let preview = m::data("delivery-preview");
    let delivery_locked = locked || delivery["available"] != true;
    let message = dom::value("delivery-message");
    let too_long = message.len() > 4000;
    dom::error(
        "delivery-message-error",
        if too_long {
            "Use a commit message of at most 4,000 UTF-8 bytes."
        } else {
            ""
        },
    );
    for e in dom::all("#delivery-options input,#delivery-options select") {
        if delivery_locked {
            let _ = e.set_attribute("disabled", "");
        } else {
            let _ = e.remove_attribute("disabled");
        }
    }
    dom::disabled(
        "delivery-commit-preview-button",
        delivery_locked
            || dom::selected_checks("delivery-files").is_empty()
            || message.trim().is_empty()
            || too_long,
    );
    let require = dom::checked("delivery-check-required");
    let check = dom::value("delivery-check-command");
    let checktime = dom::num("delivery-check-timeout").as_f64();
    let check_valid = !check.trim().is_empty()
        && check.len() <= 4000
        && checktime.is_some_and(|n| (1.0..=3600.).contains(&n) && n.fract() == 0.);
    dom::disabled("delivery-check-command", delivery_locked || !require);
    dom::disabled("delivery-check-timeout", delivery_locked || !require);
    dom::disabled(
        "delivery-preview-button",
        delivery_locked
            || dom::value("delivery-source").is_empty()
            || dom::value("delivery-target").is_empty()
            || require && !check_valid,
    );
    dom::disabled(
        "delivery-commit",
        delivery_locked || preview["kind"] != "commit" || preview["can_apply"] != true,
    );
    let check = &preview["target_check"];
    dom::hidden("delivery-run-check", preview["check"].is_null());
    dom::disabled(
        "delivery-run-check",
        delivery_locked || preview["check"].is_null() || preview["can_apply"] != true,
    );
    dom::hidden("delivery-cancel-check", !m::active(check));
    dom::disabled("delivery-cancel-check", m::read(|s| s.pending));
    dom::disabled(
        "delivery-apply",
        delivery_locked
            || preview["kind"] != "transfer"
            || preview["can_apply"] != true
            || !preview["check"].is_null()
                && (check["status"] != "passed"
                    || check["workspace_changed"] == true
                    || check["candidate"] != preview["candidate"]),
    );
    let brain_id = session["brain"]["record_id"]
        .as_str()
        .or_else(|| session["brain"]["task_id"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let has_brain_task = brain_id.is_some();
    dom::text(
        "results-memory-note",
        &if let Some(id) = brain_id {
            if m::active(&session) {
                format!(
                    "Linked Brain task {id}. Wait for this session to finish before saving memory."
                )
            } else {
                format!("Linked Brain task {id}. Edit candidate notes and verify claims against project files before saving.")
            }
        } else {
            "Link a Project Brain task to this session before saving verified memory.".to_owned()
        },
    );
    let memory_locked = locked || !has_brain_task;
    for el in dom::all(
        "#results-memory-form input,#results-memory-form select,#results-memory-form textarea",
    ) {
        if memory_locked {
            let _ = el.set_attribute("disabled", "");
        } else {
            let _ = el.remove_attribute("disabled");
        }
    }
    let learnings_count = dom::all("#results-memory-learnings .results-memory-learning-row").len();
    dom::disabled(
        "results-memory-add-learning",
        memory_locked || learnings_count >= 3,
    );
    dom::disabled("results-memory-prefill", memory_locked);
    for btn in dom::all("#results-memory-learnings button") {
        if memory_locked {
            let _ = btn.set_attribute("disabled", "");
        } else {
            let _ = btn.remove_attribute("disabled");
        }
    }
    dom::disabled(
        "results-memory-save",
        memory_locked || !dom::checked("results-memory-verified"),
    );
    sync_working_indicator();
}
pub(super) async fn delivery_load(sid: &str) {
    let key = "delivery";
    let response = api::scoped(key, &format!("/api/sessions/{}/delivery", dom::enc(sid))).await;
    if let Ok((epoch, v)) = response {
        if !m::current(key, epoch) || !m::read(|s| s.sid == sid) || !result_view() {
            return;
        }
        m::set("delivery", v.clone());
        dom::hidden(
            "delivery-options",
            !m::yes(&v["available"]) && !v["preview"].is_object(),
        );
        dom::text(
            "delivery-availability",
            v["notice"]
                .as_str()
                .or_else(|| v["reason"].as_str())
                .unwrap_or(""),
        );
        dom::clear("delivery-files");
        for file in m::array(&v["files"]) {
            dom::append(
                "delivery-files",
                &dom::check(
                    &format!("{} · {}", m::text(&file, "path"), m::text(&file, "status")),
                    "delivery-file",
                    &m::text(&file, "path"),
                    false,
                ),
            );
        }
        let mut commits = vec![json!({"id":"","name":"Choose a commit"})];
        commits.extend(m::array(&v["commits"]).into_iter().map(|x|json!({"id":x["sha"],"name":format!("{} · {}",m::text(&x,"sha"),m::text(&x,"subject"))})));
        dom::options("delivery-source", &commits, m::summary, None);
        let mut branches = vec![json!({"id":"","name":"Choose a local branch"})];
        branches.extend(
            m::array(&v["branches"])
                .into_iter()
                .map(|x| json!({"id":x["name"],"name":x["name"]})),
        );
        dom::options("delivery-target", &branches, m::summary, None);
        if v["preview"].is_object() {
            render_delivery(&v["preview"]);
        }
        result_controls();
    }
}
pub(super) fn render_delivery(v: &V) {
    m::set("delivery-preview", v.clone());
    let prefix = if v["kind"] == "commit" {
        "delivery-commit"
    } else {
        "delivery-preview"
    };
    dom::hidden(
        if prefix == "delivery-commit" {
            "delivery-commit-preview"
        } else {
            "delivery-preview"
        },
        false,
    );
    dom::text(
        &format!("{prefix}-summary"),
        &format!("{} · {}", m::summary(v), m::text(v, "candidate")),
    );
    dom::rows(&format!("{prefix}-files"), &m::array(&v["files"]));
    crate::run::render_grouped_diff_into(&format!("{prefix}-diff"), &m::text(v, "diff"), prefix);
    dom::pretty(&format!("{prefix}-checks"), &v["checks"]);
    dom::text(
        &format!("{prefix}-note"),
        v["notice"]
            .as_str()
            .unwrap_or("Review the files and diff before confirming."),
    );
    let project_id = m::read(|s| {
        let p = m::text(&s.selected, "project_id");
        if p.is_empty() {
            dom::value("project")
        } else {
            p
        }
    });
    let risks_id = format!("{prefix}-risks");
    let risks_html = render_advisory_panel_html(&project_id, v, false);
    if let Some(el) = dom::get(&risks_id) {
        el.set_inner_html(&risks_html);
        dom::hide(&el, risks_html.is_empty());
    }
    dom::disabled(
        "delivery-commit",
        !m::yes(&v["can_apply"]) || v["kind"] != "commit",
    );
    dom::disabled(
        "delivery-apply",
        !m::yes(&v["can_apply"]) || v["kind"] == "commit",
    );
    dom::text("delivery-check-status", &m::summary(&v["target_check"]));
    dom::text(
        "delivery-check-output",
        &m::text(&v["target_check"], "output"),
    );
}
pub(super) fn invalidate_delivery() {
    m::next("delivery");
    m::take("delivery-preview");
    dom::hidden("delivery-preview", true);
    dom::hidden("delivery-commit-preview", true);
    for id in ["delivery-commit-risks", "delivery-preview-risks"] {
        dom::clear(id);
        dom::hidden(id, true);
    }
    dom::disabled("delivery-commit", true);
    dom::disabled("delivery-apply", true);
}
pub(super) async fn delivery(action: &str, body: V) {
    let sid = m::read(|s| s.sid.clone());
    if sid.is_empty() || m::read(|s| s.pending) {
        return;
    }
    m::write(|s| s.pending = true);
    controls();
    invalidate_delivery();
    let mut body = body;
    body["action"] = json!(action);
    match api::post(&format!("/api/sessions/{}/delivery", dom::enc(&sid)), body).await {
        Ok(v) => {
            if action == "check" {
                if v["session"].is_object() {
                    m::upsert(v["session"].clone());
                }
                results().await;
            } else if ["commit", "apply"].contains(&action) {
                dom::text(
                    "delivery-status",
                    &format!(
                        "{} · Commit {}",
                        m::text(&v, "detail"),
                        m::text(&v, "commit")
                    ),
                );
                results().await;
            } else {
                render_delivery(&v);
            }
        }
        Err(e) => dom::error(
            "delivery-error",
            &format!(
                "{} Refresh results and prepare a new preview before retrying.",
                e.message
            ),
        ),
    }
    m::write(|s| s.pending = false);
    controls();
}
pub(super) fn resize_column_height() -> f64 {
    dom::get("sessions-chat-column")
        .map(|e| e.client_height())
        .filter(|&h| h > 0)
        .or_else(|| dom::get("sessions-view").map(|e| e.client_height()))
        .unwrap_or(500) as f64
}
pub(super) fn output_resize_percent(height: f64, max_height: f64) -> u64 {
    let min_h = 80.;
    let max_h = max_height.max(min_h);
    if max_h <= min_h {
        return 100;
    }
    let clamped = height.clamp(min_h, max_h);
    let ratio = (max_h - clamped) / (max_h - min_h);
    (ratio * 100.).round().clamp(0., 100.) as u64
}
pub(super) fn resize(height: f64) {
    let available = resize_column_height()
        - dom::get("composer-area")
            .map(|e| e.client_height())
            .unwrap_or(0) as f64
        - dom::get("run-view").map(|e| e.client_height()).unwrap_or(0) as f64
        - 100.;
    let max_h = available.max(80.);
    let h = height.max(80.).min(max_h);
    if let Some(e) =
        dom::get("sessions-view").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = e
            .style()
            .set_property("--configuration-height", &format!("{h}px"));
        let _ = e.class_list().add_1("output-resized");
    }
    m::set("output-height", json!(h));
    dom::attr(
        "output-resizer",
        "aria-valuenow",
        &output_resize_percent(h, max_h).to_string(),
    );
}
pub fn resize_key(event: &web_sys::KeyboardEvent) {
    if event
        .target()
        .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
        .map(|e| e.id())
        .as_deref()
        != Some("output-resizer")
    {
        return;
    }
    let h = m::data("output-height").as_f64().unwrap_or(200.);
    let next = match event.key().as_str() {
        "ArrowUp" => h - 40.,
        "ArrowDown" => h + 40.,
        "Home" => 80.,
        "End" => resize_column_height(),
        _ => return,
    };
    event.prevent_default();
    resize(next);
}
pub fn resize_event(kind: &str, event: &web_sys::Event) {
    let Some(target) = event
        .target()
        .and_then(|e| e.dyn_into::<web_sys::Element>().ok())
    else {
        return;
    };
    if target.id() != "output-resizer" {
        return;
    }
    if kind == "dblclick" {
        dom::class("sessions-view", "output-resized", false);
        m::take("output-height");
        dom::attr("output-resizer", "aria-valuenow", "50");
        return;
    }
    let Some(p) = event.dyn_ref::<web_sys::PointerEvent>() else {
        return;
    };
    match kind {
        "pointerdown" if p.button() == 0 => {
            let _ = target.set_pointer_capture(p.pointer_id());
            dom::focus("output-resizer");
            event.prevent_default();
        }
        "pointermove" if target.has_pointer_capture(p.pointer_id()) => {
            let top = dom::document()
                .query_selector("#sessions-view .configuration")
                .ok()
                .flatten()
                .map(|e| e.get_bounding_client_rect().top())
                .unwrap_or(0.);
            resize(p.client_y() as f64 - top);
        }
        "pointerup" | "pointercancel" => {
            let _ = target.release_pointer_capture(p.pointer_id());
        }
        _ => {}
    }
}

pub(crate) fn normalize_right_tab(tab: &str) -> &'static str {
    match tab {
        "diff" => "diff",
        "checks" => "checks",
        _ => "graph",
    }
}

/// Switches between the side-by-side right-pane tabs (`Code graph` | `Changes & Diff` | `Checks`).
pub(crate) fn set_workspace_right_tab(tab: &str) {
    set_workspace_right_tab_inner(tab, true);
}

pub(crate) fn set_workspace_right_tab_inner(tab: &str, fetch_results: bool) {
    let tab = normalize_right_tab(tab);
    m::write(|s| s.workspace.right_tab = tab.to_owned());
    dom::attr("sessions-right-pane", "data-right-tab", tab);
    for (name, pane_id) in [
        ("graph", "session-graph-workspace"),
        ("diff", "workspace-diff-pane"),
        ("checks", "workspace-checks-pane"),
    ] {
        let selected = name == tab;
        let tab_id = format!("right-tab-{name}");
        dom::attr(
            &tab_id,
            "aria-selected",
            if selected { "true" } else { "false" },
        );
        dom::attr(&tab_id, "tabindex", if selected { "0" } else { "-1" });
        dom::hidden(pane_id, !selected);
    }
    if let Some(tabs) = dom::get("sessions-right-tabs") {
        dom::sync_roving_widget(&tabs);
    }
    if tab == "graph" {
        crate::graph_map::sync_embedded(false);
    } else if fetch_results {
        spawn_local(async {
            results().await;
        });
    }
    if !m::yes(&m::data("bootstrapping")) && !m::read(|s| s.boot.is_null()) {
        save_preferences();
    }
}

pub(crate) fn clamp_workspace_split(percent: f64) -> f64 {
    percent.clamp(25.0, 75.0)
}

pub(crate) fn apply_workspace_split_percent(percent: f64, persist: bool) {
    let clamped = clamp_workspace_split(percent);
    m::write(|s| s.workspace.split_percent = Some(clamped));
    if let Some(ws) =
        dom::get("sessions-workspace").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = ws
            .style()
            .set_property("--workspace-split", &format!("{clamped:.2}%"));
    }
    dom::attr(
        "workspace-resizer",
        "aria-valuenow",
        &(clamped.round() as u64).to_string(),
    );
    if persist && !m::yes(&m::data("bootstrapping")) && !m::read(|s| s.boot.is_null()) {
        save_preferences();
    }
}

pub fn workspace_split_key(event: &web_sys::KeyboardEvent) {
    if event
        .target()
        .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
        .map(|e| e.id())
        .as_deref()
        != Some("workspace-resizer")
    {
        return;
    }
    let cur = m::read(|s| s.workspace.split_percent).unwrap_or(42.0);
    let next = match event.key().as_str() {
        "ArrowLeft" => cur - 4.0,
        "ArrowRight" => cur + 4.0,
        "Home" => 25.0,
        "End" => 75.0,
        _ => return,
    };
    event.prevent_default();
    apply_workspace_split_percent(next, true);
}

pub fn workspace_split_event(kind: &str, event: &web_sys::Event) {
    let Some(target) = event
        .target()
        .and_then(|e| e.dyn_into::<web_sys::Element>().ok())
    else {
        return;
    };
    if target.id() != "workspace-resizer" {
        return;
    }
    if kind == "dblclick" {
        if let Some(ws) =
            dom::get("sessions-workspace").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = ws.style().remove_property("--workspace-split");
        }
        m::write(|s| s.workspace.split_percent = None);
        dom::attr("workspace-resizer", "aria-valuenow", "42");
        save_preferences();
        return;
    }
    let Some(p) = event.dyn_ref::<web_sys::PointerEvent>() else {
        return;
    };
    match kind {
        "pointerdown" if p.button() == 0 => {
            let _ = target.set_pointer_capture(p.pointer_id());
            dom::focus("workspace-resizer");
            event.prevent_default();
        }
        "pointermove" if target.has_pointer_capture(p.pointer_id()) => {
            if let Some(ws) = dom::get("sessions-workspace") {
                let rect = ws.get_bounding_client_rect();
                if rect.width() > 0.0 {
                    let pct = ((p.client_x() as f64 - rect.left()) / rect.width()) * 100.0;
                    apply_workspace_split_percent(pct, false);
                }
            }
        }
        "pointerup" | "pointercancel" => {
            let _ = target.release_pointer_capture(p.pointer_id());
            save_preferences();
        }
        _ => {}
    }
}

type FailureLocalizerFn = std::rc::Rc<dyn Fn(&V)>;

thread_local! {
    static FAILURE_LOCALIZERS: RefCell<Vec<FailureLocalizerFn>> = const { RefCell::new(Vec::new()) };
}

/// Registers `@ui-dep`'s `graph_map::localize_failure_request` handler without hard compile-time coupling across branches.
#[allow(dead_code)]
pub(crate) fn register_failure_localizer(handler: impl Fn(&V) + 'static) {
    let rc: FailureLocalizerFn = std::rc::Rc::new(handler);
    FAILURE_LOCALIZERS.with(|list| list.borrow_mut().push(rc.clone()));
    if let Some(pending) = m::take("pending-failure-localize")
        .as_object()
        .map(|o| V::Object(o.clone()))
    {
        rc(&pending);
    }
}

/// Forwards a failed command/test/check localization request to registered `@ui-dep` map handlers.
pub(crate) fn localize_failure_on_map(request: &V) {
    let handlers: Vec<FailureLocalizerFn> = FAILURE_LOCALIZERS.with(|list| list.borrow().clone());
    if handlers.is_empty() {
        m::set("pending-failure-localize", request.clone());
        #[cfg(target_arch = "wasm32")]
        {
            set_workspace_right_tab("graph");
            dom::announce(
                "run-announcer",
                "Switched to Code graph; awaiting map localizer binding.",
            );
        }
        return;
    }
    #[cfg(target_arch = "wasm32")]
    if m::read(|s| s.view == "sessions") {
        set_workspace_right_tab("graph");
    }
    for handler in handlers {
        handler(request);
    }
}
