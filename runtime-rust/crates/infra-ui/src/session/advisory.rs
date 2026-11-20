use super::*;

pub(super) fn escape_html(s: &str) -> String {
    crate::dom::escape_html(s)
}

pub(crate) fn has_at_file_mention(prompt: &str) -> bool {
    prompt.split_whitespace().any(|raw| {
        let token = raw.trim_matches(|c: char| {
            matches!(
                c,
                '(' | '[' | '{' | '"' | '\'' | ',' | ';' | ')' | ']' | '}'
            )
        });
        let Some(rest) = token.strip_prefix('@') else {
            return false;
        };
        rest.len() >= 2
            && !rest.contains('@')
            && (rest.contains('/') || rest.contains('.'))
            && rest.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '#')
            })
    })
}

pub(crate) fn build_preflight_request(
    prompt: &str,
    sid: &str,
    task_id: &str,
    include_risk_callers: bool,
) -> V {
    let mut body = json!({
        "prompt": prompt,
        "risk_caller_hints": include_risk_callers,
    });
    if !sid.is_empty() {
        body["session_id"] = json!(sid);
    }
    if !task_id.is_empty() {
        body["task_id"] = json!(task_id);
    }
    body
}

pub(super) fn preflight_task_id() -> String {
    let sid = m::read(|s| s.sid.clone());
    if !sid.is_empty() {
        return m::read(|s| {
            s.selected["brain"]["record_id"]
                .as_str()
                .or_else(|| s.selected["brain"]["task_id"].as_str())
                .unwrap_or("")
                .trim()
                .to_owned()
        });
    }
    match dom::value("brain-link-kind").as_str() {
        "existing" => dom::value("brain-link-task").trim().to_owned(),
        "create" => dom::value("brain-link-task-id").trim().to_owned(),
        _ => String::new(),
    }
}

pub(super) fn reset_preflight(clear_opt_in: bool) {
    dom::cancel_timer("preflight-debounce");
    m::next("preflight");
    m::take("preflight");
    dom::text("preflight-status", "");
    dom::error("preflight-error", "");
    dom::clear("preflight-results");
    dom::hidden("preflight-results", true);
    if clear_opt_in {
        dom::set_checked("preflight-risk-callers", false);
    }
}

pub(super) fn inspect_file_button_html(path: &str, label: &str) -> String {
    let payload = json!({"path": path}).to_string();
    format!(
        "<button type=\"button\" class=\"button compact advisory-inspect\" data-action=\"inspect-file-graph\" data-path=\"{}\" data-payload=\"{}\">{}</button>",
        escape_html(path),
        escape_html(&payload),
        escape_html(label)
    )
}

pub(super) fn open_memory_target_button_html(
    scope: &str,
    project_id: &str,
    target_id: &str,
    label: &str,
) -> String {
    let payload = json!({
        "scope": scope,
        "project_id": project_id,
        "target_id": target_id
    })
    .to_string();
    format!(
        "<button type=\"button\" class=\"button compact advisory-inspect\" data-action=\"preflight-open-memory\" data-payload=\"{}\">{}</button>",
        escape_html(&payload),
        escape_html(label)
    )
}

pub(super) fn render_linked_record_buttons_html(
    project_id: &str,
    brain_records: &[V],
    memory_chunks: &[V],
) -> String {
    let mut out = String::new();
    let mut seen = std::collections::BTreeSet::new();
    for rec in brain_records.iter().take(4) {
        let target_id = rec["id"]
            .as_str()
            .or_else(|| rec["external_id"].as_str())
            .or_else(|| rec.as_str())
            .unwrap_or("")
            .trim();
        if target_id.is_empty() || !seen.insert(("brain", target_id.to_owned())) {
            continue;
        }
        let title = rec["title"]
            .as_str()
            .or_else(|| rec["external_id"].as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(target_id);
        out.push_str(&open_memory_target_button_html(
            "brain",
            project_id,
            target_id,
            &format!("Open {title}"),
        ));
    }
    for chunk in memory_chunks.iter().take(4) {
        let target_id = chunk["id"]
            .as_str()
            .or_else(|| chunk.as_str())
            .unwrap_or("")
            .trim();
        if target_id.is_empty() || !seen.insert(("memory", target_id.to_owned())) {
            continue;
        }
        let title = chunk["title"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(target_id);
        out.push_str(&open_memory_target_button_html(
            "memory",
            project_id,
            target_id,
            &format!("Open {title}"),
        ));
    }
    out
}

pub(super) fn advisory_path(value: &V) -> String {
    value
        .as_str()
        .or_else(|| value["path"].as_str())
        .or_else(|| value["file"].as_str())
        .unwrap_or("")
        .trim()
        .to_owned()
}

pub(crate) fn render_advisory_panel_html(project_id: &str, v: &V, is_preflight: bool) -> String {
    let warnings_obj = if v["memory_warnings"].is_object() {
        &v["memory_warnings"]
    } else {
        v
    };
    let contradictions = m::array(&warnings_obj["contradiction_warnings"]);
    let hygiene = m::array(&warnings_obj["hygiene_warnings"]);
    let risk_callers = m::array(&v["risk_callers"]);
    let files = m::array(&v["files"]);
    let blast = &v["blast_summary"];
    let hints = m::array(&v["risk_caller_hints"]);

    if !is_preflight && contradictions.is_empty() && hygiene.is_empty() && risk_callers.is_empty() {
        return String::new();
    }
    if is_preflight
        && contradictions.is_empty()
        && hygiene.is_empty()
        && risk_callers.is_empty()
        && files.is_empty()
        && hints.is_empty()
        && !blast.is_object()
    {
        return "<p class=\"knowledge-note advisory-empty\">No referenced files, memory warnings, or graph risk callers flagged.</p>".to_owned();
    }

    let mut links_by_path: std::collections::BTreeMap<String, (Vec<V>, Vec<V>)> =
        std::collections::BTreeMap::new();
    for entry in files.iter().chain(hints.iter()) {
        let p = advisory_path(entry);
        if p.is_empty() {
            continue;
        }
        let entry_links = links_by_path.entry(p).or_default();
        entry_links.0.extend(m::array(&entry["brain_records"]));
        entry_links.1.extend(m::array(&entry["memory_chunks"]));
    }

    let mut out = String::new();
    if is_preflight && (blast.is_object() || !files.is_empty() || !hints.is_empty()) {
        let blast_files = m::array(&blast["files"]);
        let file_count = if !files.is_empty() {
            files.len()
        } else {
            blast_files.len()
        };
        let direct = blast["direct_count"].as_u64().unwrap_or(0);
        let downstream = blast["downstream_count"].as_u64().unwrap_or(0);
        out.push_str("<div class=\"advisory-blast\">");
        out.push_str(&format!(
            "<span class=\"advisory-pill\" data-kind=\"info\">{file_count} file(s)</span>"
        ));
        if blast.is_object() {
            out.push_str(&format!(
                "<span class=\"advisory-pill\" data-kind=\"info\">{direct} direct</span><span class=\"advisory-pill\" data-kind=\"info\">{downstream} downstream</span>"
            ));
        }
        if !hints.is_empty() {
            out.push_str(&format!(
                "<span class=\"advisory-pill\" data-kind=\"arch\">{} hint(s)</span>",
                hints.len()
            ));
        }
        out.push_str("</div>");

        let mut listed_files = Vec::new();
        for entry in files.iter().chain(blast_files.iter()) {
            let p = advisory_path(entry);
            if !p.is_empty() && !listed_files.contains(&p) {
                listed_files.push(p);
            }
        }
        if !listed_files.is_empty() {
            out.push_str("<ul class=\"advisory-file-list\">");
            for path in listed_files.iter().take(12) {
                out.push_str("<li class=\"advisory-file-row\">");
                out.push_str(&format!(
                    "<code class=\"advisory-path\">{}</code><span class=\"advisory-actions\">",
                    escape_html(path)
                ));
                out.push_str(&inspect_file_button_html(path, "Code graph"));
                if let Some((brain_records, memory_chunks)) = links_by_path.get(path) {
                    out.push_str(&render_linked_record_buttons_html(
                        project_id,
                        brain_records,
                        memory_chunks,
                    ));
                }
                out.push_str("</span></li>");
            }
            out.push_str("</ul>");
        }
    }

    if !contradictions.is_empty() {
        out.push_str("<div class=\"advisory-group\" data-kind=\"contradiction\"><p class=\"advisory-group-title\">Memory contradictions</p><ul class=\"advisory-list\">");
        for w in &contradictions {
            let text = w["text"]
                .as_str()
                .unwrap_or("Conflicting memory atoms found.");
            let path = advisory_path(w);
            out.push_str("<li class=\"advisory-item\" data-kind=\"contradiction\">");
            out.push_str("<span class=\"advisory-badge\" data-kind=\"error\">Contradiction</span>");
            out.push_str(&format!(
                "<span class=\"advisory-text\">{}</span>",
                escape_html(text)
            ));
            out.push_str("<span class=\"advisory-actions\">");
            if !path.is_empty() {
                out.push_str(&inspect_file_button_html(&path, "Code graph"));
            }
            let opens = m::array(&w["open"]);
            if opens.is_empty() {
                for target in m::array(&w["linked"]).iter().take(4) {
                    if let Some(id) = target.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                        let scope = if id.starts_with("MEM-") {
                            "memory"
                        } else {
                            "brain"
                        };
                        out.push_str(&open_memory_target_button_html(
                            scope,
                            project_id,
                            id,
                            &format!("Open {id}"),
                        ));
                    }
                }
            } else {
                for open in opens {
                    let scope = open["scope"].as_str().unwrap_or("");
                    let target_id = open["target_id"].as_str().unwrap_or("");
                    if matches!(scope, "brain" | "memory") && !target_id.is_empty() {
                        let title = open["title"].as_str().unwrap_or(target_id);
                        out.push_str(&open_memory_target_button_html(
                            scope,
                            project_id,
                            target_id,
                            &format!("Open {title}"),
                        ));
                    }
                }
            }
            out.push_str("</span></li>");
        }
        out.push_str("</ul></div>");
    }

    if !hygiene.is_empty() {
        out.push_str("<div class=\"advisory-group\" data-kind=\"hygiene\"><p class=\"advisory-group-title\">Memory hygiene warnings</p><ul class=\"advisory-list\">");
        for w in &hygiene {
            let kind = w["kind"].as_str().unwrap_or("hygiene");
            let badge = match kind {
                "cited-file-missing" => "Missing file",
                "cited-file-renamed" => "Renamed file",
                other => other,
            };
            let text = w["text"].as_str().unwrap_or("Review cited memory.");
            let path = advisory_path(w);
            out.push_str("<li class=\"advisory-item\" data-kind=\"hygiene\">");
            out.push_str(&format!(
                "<span class=\"advisory-badge\" data-kind=\"warning\">{}</span>",
                escape_html(badge)
            ));
            out.push_str(&format!(
                "<span class=\"advisory-text\">{}</span>",
                escape_html(text)
            ));
            out.push_str("<span class=\"advisory-actions\">");
            if !path.is_empty() {
                out.push_str(&inspect_file_button_html(&path, "Code graph"));
            }
            for target in m::array(&w["linked"]).iter().take(4) {
                if let Some(id) = target.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    let scope = if id.starts_with("MEM-") {
                        "memory"
                    } else {
                        "brain"
                    };
                    out.push_str(&open_memory_target_button_html(
                        scope,
                        project_id,
                        id,
                        &format!("Open {id}"),
                    ));
                }
            }
            out.push_str("</span></li>");
        }
        out.push_str("</ul></div>");
    }

    if !risk_callers.is_empty() {
        out.push_str("<div class=\"advisory-group\" data-kind=\"risk-callers\"><p class=\"advisory-group-title\">Graph risk callers</p><ul class=\"advisory-list\">");
        for caller in &risk_callers {
            let from = caller["from"]
                .as_str()
                .or_else(|| caller["caller"].as_str())
                .or_else(|| caller["path"].as_str())
                .or_else(|| caller["file"].as_str())
                .or_else(|| caller.as_str())
                .unwrap_or("")
                .trim();
            let to = caller["to"]
                .as_str()
                .or_else(|| caller["target"].as_str())
                .unwrap_or("")
                .trim();
            let impact = caller["impact"]
                .as_str()
                .or_else(|| caller["kind"].as_str())
                .unwrap_or("risk");
            let reason = caller["reason"]
                .as_str()
                .or_else(|| caller["text"].as_str())
                .unwrap_or("")
                .trim();
            if from.is_empty() && reason.is_empty() {
                continue;
            }
            out.push_str("<li class=\"advisory-item\" data-kind=\"risk-caller\">");
            out.push_str(&format!(
                "<span class=\"advisory-badge\" data-kind=\"arch\">{}</span>",
                escape_html(impact)
            ));
            let label = if !from.is_empty() && !to.is_empty() {
                format!("{from} → {to}")
            } else if !from.is_empty() {
                from.to_owned()
            } else {
                reason.to_owned()
            };
            out.push_str(&format!(
                "<code class=\"advisory-path\">{}</code>",
                escape_html(&label)
            ));
            if !reason.is_empty() && !from.is_empty() {
                out.push_str(&format!(
                    "<span class=\"advisory-text\">{}</span>",
                    escape_html(reason)
                ));
            }
            if !from.is_empty() {
                out.push_str("<span class=\"advisory-actions\">");
                out.push_str(&inspect_file_button_html(from, "Code graph"));
                if let Some((brain_records, memory_chunks)) = links_by_path.get(from) {
                    out.push_str(&render_linked_record_buttons_html(
                        project_id,
                        brain_records,
                        memory_chunks,
                    ));
                }
                out.push_str("</span>");
            }
            out.push_str("</li>");
        }
        out.push_str("</ul></div>");
    }

    if is_preflight && !hints.is_empty() {
        out.push_str("<div class=\"advisory-group\" data-kind=\"risk-caller-hints\"><p class=\"advisory-group-title\">Advisory risk-caller hints</p><ul class=\"advisory-list\">");
        for hint in &hints {
            let path = advisory_path(hint);
            let target = hint["target"]
                .as_str()
                .or_else(|| hint["to"].as_str())
                .unwrap_or("")
                .trim();
            let impact = hint["impact"].as_str().unwrap_or("hint");
            let brain_records = m::array(&hint["brain_records"]);
            let memory_chunks = m::array(&hint["memory_chunks"]);
            let hint_hygiene = m::array(&hint["hygiene_warnings"]);
            let source_hints = m::array(&hint["source_hints"]);
            if path.is_empty()
                && brain_records.is_empty()
                && memory_chunks.is_empty()
                && hint_hygiene.is_empty()
                && source_hints.is_empty()
            {
                continue;
            }
            let label = if !path.is_empty() && !target.is_empty() {
                format!("{path} → {target}")
            } else {
                path.clone()
            };
            let mut details = Vec::new();
            for hw in &hint_hygiene {
                if let Some(txt) = hw["text"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    details.push(txt.to_owned());
                }
            }
            let source_paths: Vec<String> = source_hints
                .iter()
                .filter_map(|item| {
                    item["path"]
                        .as_str()
                        .or_else(|| item.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
                .take(4)
                .collect();
            if !source_paths.is_empty() {
                details.push(format!("Sources: {}", source_paths.join(", ")));
            }
            out.push_str("<li class=\"advisory-item\" data-kind=\"risk-caller-hint\">");
            out.push_str(&format!(
                "<span class=\"advisory-badge\" data-kind=\"arch\">{}</span>",
                escape_html(impact)
            ));
            if !label.is_empty() {
                out.push_str(&format!(
                    "<code class=\"advisory-path\">{}</code>",
                    escape_html(&label)
                ));
            }
            if !details.is_empty() {
                out.push_str(&format!(
                    "<span class=\"advisory-text\">{}</span>",
                    escape_html(&details.join(" · "))
                ));
            }
            out.push_str("<span class=\"advisory-actions\">");
            if !path.is_empty() {
                out.push_str(&inspect_file_button_html(&path, "Code graph"));
            }
            out.push_str(&render_linked_record_buttons_html(
                project_id,
                &brain_records,
                &memory_chunks,
            ));
            for hw in &hint_hygiene {
                for target_id in m::array(&hw["linked"]).iter().take(4) {
                    if let Some(id) = target_id.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                        let scope = if id.starts_with("MEM-") {
                            "memory"
                        } else {
                            "brain"
                        };
                        out.push_str(&open_memory_target_button_html(
                            scope,
                            project_id,
                            id,
                            &format!("Open {id}"),
                        ));
                    }
                }
            }
            out.push_str("</span></li>");
        }
        out.push_str("</ul></div>");
    }

    if is_preflight
        && contradictions.is_empty()
        && hygiene.is_empty()
        && risk_callers.is_empty()
        && hints.is_empty()
    {
        out.push_str("<p class=\"knowledge-note advisory-empty\">No contradictions, hygiene warnings, or graph risk callers flagged.</p>");
    }
    out
}

pub(super) fn preflight_status_summary(v: &V) -> String {
    let c = m::array(&v["contradiction_warnings"]).len();
    let h = m::array(&v["hygiene_warnings"]).len();
    let r = m::array(&v["risk_callers"]).len();
    let hints = m::array(&v["risk_caller_hints"]).len();
    let f = m::array(&v["files"]).len();
    if c + h + r + hints == 0 {
        if f == 0 {
            "No risks flagged".to_owned()
        } else {
            format!("Checked {f} file(s) · no warnings")
        }
    } else {
        let mut parts = Vec::new();
        if c > 0 {
            parts.push(format!("{c} contradiction(s)"));
        }
        if h > 0 {
            parts.push(format!("{h} hygiene"));
        }
        if r > 0 {
            parts.push(format!("{r} risk caller(s)"));
        }
        if hints > 0 {
            parts.push(format!("{hints} hint(s)"));
        }
        parts.join(" · ")
    }
}

pub(super) async fn run_preflight(manual: bool) {
    let ready = m::read(|s| !s.boot.is_null() && !s.auth_failed);
    let project = dom::value("project");
    if !ready || project.is_empty() {
        return;
    }
    let prompt = dom::value("prompt");
    if !manual && (prompt.trim().is_empty() || !has_at_file_mention(&prompt)) {
        return;
    }
    let sid = m::read(|s| s.sid.clone());
    let task_id = preflight_task_id();
    let include_callers = dom::checked("preflight-risk-callers");
    let body = build_preflight_request(&prompt, &sid, &task_id, include_callers);
    let key = "preflight";
    let epoch = m::next(key);
    let Ok(controller) = web_sys::AbortController::new() else {
        return;
    };
    m::write(|s| {
        s.aborts.insert(key.into(), controller.clone());
    });
    dom::text("preflight-status", "Checking risks…");
    if manual {
        dom::error("preflight-error", "");
    }
    let url = format!("/api/projects/{}/preflight", dom::enc(&project));
    let response = api::call("POST", &url, Some(body), Some(controller.signal())).await;
    if m::current(key, epoch) {
        m::write(|s| {
            s.aborts.remove(key);
        });
    }
    if !m::current(key, epoch) || dom::value("project") != project {
        return;
    }
    match response {
        Ok(v) => {
            m::set("preflight", v.clone());
            dom::error("preflight-error", "");
            dom::text("preflight-status", &preflight_status_summary(&v));
            let html = render_advisory_panel_html(&project, &v, true);
            if let Some(el) = dom::get("preflight-results") {
                el.set_inner_html(&html);
                dom::hide(&el, html.is_empty());
            }
        }
        Err(e) if e.aborted => {}
        Err(e) => {
            if manual {
                dom::error("preflight-error", &e.message);
                dom::text("preflight-status", "");
            } else {
                dom::text("preflight-status", "Risk check unavailable");
            }
        }
    }
}

pub(crate) fn extract_used_memory_paths(session: &V) -> Vec<String> {
    let mut paths = Vec::new();
    let capsule = &session["brain"]["capsule"];
    for layer in ["selected", "procedural", "semantic", "episodic"] {
        for item in m::array(&capsule[layer]) {
            if let Some(path) = item["path"]
                .as_str()
                .map(str::trim)
                .filter(|p| !p.is_empty())
            {
                let owned = path.to_owned();
                if !paths.contains(&owned) {
                    paths.push(owned);
                    if paths.len() >= 10 {
                        return paths;
                    }
                }
            }
        }
    }
    if paths.is_empty() {
        if let Some(hashes) = session["brain"]["source_hashes"].as_object() {
            for key in hashes.keys() {
                let trimmed = key.trim();
                if !trimmed.is_empty() {
                    let owned = trimmed.to_owned();
                    if !paths.contains(&owned) {
                        paths.push(owned);
                        if paths.len() >= 10 {
                            break;
                        }
                    }
                }
            }
        }
    }
    paths
}

pub(super) fn sanitize_memory_text(value: &str, max_chars: usize) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut trimmed = collapsed.trim();
    for prefix in [
        "user:",
        "assistant:",
        "system:",
        "developer:",
        "tool:",
        "prompt:",
        "response:",
        "reasoning:",
        "stdout:",
        "stderr:",
        "log:",
    ] {
        if trimmed.to_ascii_lowercase().starts_with(prefix) {
            trimmed = trimmed[prefix.len()..].trim();
        }
    }
    trimmed.chars().take(max_chars).collect()
}

pub(super) fn is_canonical_learning_source(value: &str) -> bool {
    let head = value.split('#').next().unwrap_or(value).trim();
    if head.is_empty() || head.starts_with('/') || head.contains('\\') || head.contains(':') {
        return false;
    }
    let parts: Vec<&str> = head.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() || parts.contains(&"..") {
        return false;
    }
    if ["memory-bank/", "project-brain/"]
        .iter()
        .any(|prefix| head.starts_with(prefix))
        || parts.windows(2).any(|pair| {
            matches!(
                (pair[0], pair[1]),
                ("memory-bank", "local" | "chunks" | "indexes")
                    | (
                        "project-brain",
                        "local" | "dynamic" | "archive" | "control" | "indexes"
                    )
            )
        })
    {
        return false;
    }
    let name = parts.last().copied().unwrap_or("").to_ascii_lowercase();
    let suffix = name.rsplit_once('.').map(|(_, s)| s).unwrap_or("");
    if name == ".env"
        || (name.starts_with(".env.") && name.len() > 5)
        || matches!(name.as_str(), "secret" | "secrets" | "credentials.json")
        || (name.starts_with("id_")
            && name.len() > 3
            && name[3..].chars().all(|c| c.is_ascii_alphanumeric()))
        || matches!(
            suffix,
            "pem" | "key" | "p12" | "pfx" | "jks" | "keystore" | "sqlite" | "db"
        )
    {
        return false;
    }
    !parts.iter().any(|part| {
        let folded = part.to_ascii_lowercase();
        matches!(
            folded.as_str(),
            ".git"
                | ".ssh"
                | ".aws"
                | ".kube"
                | "node_modules"
                | "vendor"
                | ".venv"
                | "__pycache__"
                | "secrets"
                | ".secrets"
                | "credentials"
        ) || folded.starts_with(".env")
    })
}

pub(crate) fn build_memory_draft_prefill(session: &V, results: &V, delivery: &V, preview: &V) -> V {
    let snapshot_files = m::array(&results["snapshot"]["files"]);
    let mut active_files = Vec::new();
    let mut deleted_files = Vec::new();
    for entry in &snapshot_files {
        let path = m::text(entry, "path").trim().to_owned();
        if path.is_empty() {
            continue;
        }
        let status = m::text(entry, "status").to_ascii_lowercase();
        if status == "d" || status == "deleted" {
            if !deleted_files.contains(&path) {
                deleted_files.push(path);
            }
        } else if !active_files.contains(&path) {
            active_files.push(path);
        }
    }
    let canonical_active: Vec<String> = active_files
        .iter()
        .filter(|path| is_canonical_learning_source(path))
        .cloned()
        .collect();
    let canonical_deleted: Vec<String> = deleted_files
        .iter()
        .filter(|path| is_canonical_learning_source(path))
        .cloned()
        .collect();

    let mut passed_checks = Vec::new();
    let mut failed_checks = Vec::new();
    for check in m::array(&results["checks"]) {
        let cmd = m::text(&check, "command").trim().to_owned();
        if cmd.is_empty() {
            continue;
        }
        match check["status"].as_str().unwrap_or("") {
            "passed" if !passed_checks.contains(&cmd) => passed_checks.push(cmd),
            "failed" | "timed_out" | "output_limit" if !failed_checks.contains(&cmd) => {
                failed_checks.push(cmd);
            }
            _ => {}
        }
    }
    if preview["target_check"]["status"] == "passed" {
        let cmd = m::text(&preview["target_check"], "command")
            .trim()
            .to_owned();
        let label = if cmd.is_empty() {
            "delivery target check".to_owned()
        } else {
            cmd
        };
        if !passed_checks.contains(&label) {
            passed_checks.push(label);
        }
    }
    for check in m::array(&preview["checks"]) {
        if check["status"] == "passed" {
            let cmd = m::text(&check, "command").trim().to_owned();
            if !cmd.is_empty() && !passed_checks.contains(&cmd) {
                passed_checks.push(cmd);
            }
        }
    }

    let clash_items = m::array(&session["clash_result"]["items"]);
    let mut progress_parts = Vec::new();
    let task_label = session["brain"]["task"]["external_id"]
        .as_str()
        .or_else(|| session["brain"]["task_id"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let progress_active = if canonical_active.is_empty() && canonical_deleted.is_empty() {
        &active_files
    } else {
        &canonical_active
    };
    let progress_deleted = if canonical_active.is_empty() && canonical_deleted.is_empty() {
        &deleted_files
    } else {
        &canonical_deleted
    };
    let total_changed = progress_active.len() + progress_deleted.len();
    if total_changed > 0 {
        let listed: Vec<String> = progress_active
            .iter()
            .chain(progress_deleted.iter())
            .take(5)
            .cloned()
            .collect();
        let suffix = if total_changed > 5 { ", …" } else { "" };
        progress_parts.push(format!(
            "Updated {total_changed} file(s): {}{suffix}.",
            listed.join(", ")
        ));
    } else if let Some(task_prog) = session["brain"]["task"]["progress"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        progress_parts.push(task_prog.to_owned());
    } else if let Some(id) = task_label {
        progress_parts.push(format!("Reviewed session outcome for {id}."));
    }
    if !passed_checks.is_empty() {
        progress_parts.push(format!(
            "Passed checks: {}.",
            passed_checks
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let clash_summary = m::text(&session["clash_result"], "summary");
    if !clash_summary.trim().is_empty() {
        progress_parts.push(clash_summary.trim().to_owned());
    }
    let progress = sanitize_memory_text(&progress_parts.join(" "), 1000);

    let mut next_steps: Vec<String> = Vec::new();
    for cmd in &failed_checks {
        if next_steps.len() >= 3 {
            break;
        }
        next_steps.push(sanitize_memory_text(
            &format!("Fix failing check: {cmd}"),
            300,
        ));
    }
    for item in &clash_items {
        if next_steps.len() >= 3 {
            break;
        }
        let status = m::text(item, "status").to_ascii_lowercase();
        if matches!(status.as_str(), "disputed" | "open" | "unresolved") {
            let claim = m::text(item, "claim");
            if !claim.trim().is_empty() {
                next_steps.push(sanitize_memory_text(
                    &format!("Resolve clash finding: {claim}"),
                    300,
                ));
            }
        }
    }
    if next_steps.len() < 3 && !m::array(&delivery["files"]).is_empty() {
        next_steps.push("Review and commit remaining worktree changes.".to_owned());
    }
    if next_steps.is_empty() {
        for step in m::array(&session["brain"]["task"]["next_steps"]) {
            if next_steps.len() >= 3 {
                break;
            }
            if let Some(text) = step.as_str() {
                let cleaned = sanitize_memory_text(text, 300);
                if !cleaned.is_empty() {
                    next_steps.push(cleaned);
                }
            }
        }
    }

    let mut learnings: Vec<V> = Vec::new();
    for item in &clash_items {
        if learnings.len() >= 3 {
            break;
        }
        let file = m::text(item, "file").trim().to_owned();
        if file.is_empty() || deleted_files.contains(&file) || !is_canonical_learning_source(&file)
        {
            continue;
        }
        let claim = sanitize_memory_text(&m::text(item, "claim"), 200);
        let detail = sanitize_memory_text(
            item["verdict_note"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| item["evidence"].as_str().filter(|s| !s.trim().is_empty()))
                .unwrap_or(&claim),
            1000,
        );
        if claim.is_empty() || detail.is_empty() {
            continue;
        }
        let kind_str = m::text(item, "kind").to_ascii_lowercase();
        let status_str = m::text(item, "status").to_ascii_lowercase();
        let learning_type = if kind_str.contains("decision") || status_str == "accepted" {
            "decision"
        } else {
            "finding"
        };
        learnings.push(json!({
            "type": learning_type,
            "title": claim,
            "consequence": detail,
            "sources": [file],
        }));
    }
    if learnings.is_empty()
        && !canonical_active.is_empty()
        && (!passed_checks.is_empty() || session["brain"].is_object())
    {
        let sources: Vec<String> = canonical_active.iter().take(3).cloned().collect();
        let title =
            sanitize_memory_text(&format!("Verified behavior in {}", sources.join(", ")), 200);
        let consequence = if !passed_checks.is_empty() {
            sanitize_memory_text(
                &format!(
                    "Changes in {} passed {}.",
                    sources.join(", "),
                    passed_checks
                        .first()
                        .map(String::as_str)
                        .unwrap_or("checks")
                ),
                1000,
            )
        } else {
            sanitize_memory_text(
                &format!(
                    "Reviewed implementation in {} against workspace sources.",
                    sources.join(", ")
                ),
                1000,
            )
        };
        if !title.is_empty() && !consequence.is_empty() {
            learnings.push(json!({
                "type": "finding",
                "title": title,
                "consequence": consequence,
                "sources": sources,
            }));
        }
    }

    let used_memory = extract_used_memory_paths(session);
    json!({
        "progress": progress,
        "next_steps": next_steps,
        "learnings": learnings,
        "used_memory": used_memory,
    })
}

pub(crate) fn render_memory_learning_row_html(idx: usize, learning: &V) -> String {
    let ltype = if learning["type"].as_str() == Some("decision") {
        "decision"
    } else {
        "finding"
    };
    let title = learning["title"].as_str().unwrap_or("");
    let consequence = learning["consequence"].as_str().unwrap_or("");
    let sources = m::array(&learning["sources"])
        .into_iter()
        .filter_map(|s| {
            s.as_str()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let remove_payload = json!({"index": idx}).to_string();
    format!(
        "<article class=\"results-memory-learning-row\" data-learning-index=\"{idx}\">\
        <div class=\"results-memory-learning-top\">\
        <label class=\"field\">Type<select class=\"results-memory-learning-type\" data-action=\"results-memory-field\">\
        <option value=\"finding\"{}>finding</option>\
        <option value=\"decision\"{}>decision</option>\
        </select></label>\
        <label class=\"field wide\">Title<input type=\"text\" class=\"results-memory-learning-title\" data-action=\"results-memory-field\" maxlength=\"200\" value=\"{}\" placeholder=\"Concise reusable claim\"></label>\
        <button type=\"button\" class=\"button compact\" data-action=\"results-memory-remove-learning\" data-payload=\"{}\">Remove</button>\
        </div>\
        <div class=\"knowledge-form-grid\">\
        <label class=\"field wide\">Consequence<textarea class=\"results-memory-learning-consequence\" data-action=\"results-memory-field\" maxlength=\"1000\" rows=\"2\" placeholder=\"Reusable rule or consequence verified in project files\">{}</textarea></label>\
        <label class=\"field wide\">Sources (comma-separated relative paths)<input type=\"text\" class=\"results-memory-learning-sources\" data-action=\"results-memory-field\" value=\"{}\" placeholder=\"src/example.rs\"></label>\
        </div></article>",
        if ltype == "finding" { " selected" } else { "" },
        if ltype == "decision" { " selected" } else { "" },
        escape_html(title),
        escape_html(&remove_payload),
        escape_html(consequence),
        escape_html(&sources),
    )
}

pub(crate) fn render_memory_learnings_html(learnings: &[V]) -> String {
    learnings
        .iter()
        .enumerate()
        .map(|(idx, item)| render_memory_learning_row_html(idx, item))
        .collect()
}

pub(crate) fn build_memory_save_payload(
    progress: &str,
    next_steps_text: &str,
    learnings: &[V],
    used_text: &str,
    verified: bool,
) -> Result<V, String> {
    if !verified {
        return Err(
            "Check \"I verified these claims against project files\" before saving.".into(),
        );
    }
    let progress = progress.trim();
    if progress.chars().count() > 1000 {
        return Err("Write progress in at most 1,000 characters.".into());
    }
    let next_steps: Vec<String> = next_steps_text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if next_steps.len() > 3 {
        return Err("Keep at most 3 next steps.".into());
    }
    if next_steps.iter().any(|s| s.chars().count() > 300) {
        return Err("Write each next step in at most 300 characters.".into());
    }
    let mut clean_learnings = Vec::new();
    for item in learnings {
        let ltype = item["type"].as_str().unwrap_or("finding").trim();
        let title = item["title"].as_str().unwrap_or("").trim();
        let consequence = item["consequence"].as_str().unwrap_or("").trim();
        let sources: Vec<String> = item["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| {
                v.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            })
            .collect();
        if title.is_empty() && consequence.is_empty() && sources.is_empty() {
            continue;
        }
        if !matches!(ltype, "finding" | "decision") {
            return Err("Each learning must use type \"finding\" or \"decision\".".into());
        }
        if title.is_empty() || title.chars().count() > 200 {
            return Err("Each learning title must be 1 to 200 characters.".into());
        }
        if consequence.is_empty() || consequence.chars().count() > 1000 {
            return Err("Each learning consequence must be 1 to 1,000 characters.".into());
        }
        if sources.is_empty() || sources.len() > 10 {
            return Err("Each learning must cite 1 to 10 project-relative source paths.".into());
        }
        clean_learnings.push(json!({
            "type": ltype,
            "title": title,
            "consequence": consequence,
            "sources": sources,
        }));
    }
    if clean_learnings.len() > 3 {
        return Err("Keep at most 3 learnings.".into());
    }
    let mut used_memory = Vec::new();
    for raw in used_text.split(['\n', ',']) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let owned = trimmed.to_owned();
            if !used_memory.contains(&owned) {
                used_memory.push(owned);
            }
        }
    }
    if used_memory.len() > 10 {
        return Err("List at most 10 used memory paths.".into());
    }
    if progress.is_empty() && next_steps.is_empty() && clean_learnings.is_empty() {
        return Err("Provide progress, next steps, or at least one learning to save.".into());
    }
    let mut body = json!({
        "next_steps": next_steps,
        "learnings": clean_learnings,
        "verified": true,
    });
    if !progress.is_empty() {
        body["progress"] = json!(progress);
    }
    if !used_memory.is_empty() {
        body["used_memory"] = json!(used_memory);
    }
    Ok(body)
}

pub(crate) fn format_memory_save_feedback_html(resp: &V) -> String {
    let ok = m::yes(&resp["ok"]);
    let error = resp["error"].as_str().unwrap_or("").trim();
    let skipped = m::array(&resp["skipped"]);
    let records = m::array(&resp["saved"]["records"]);
    let task = &resp["saved"]["task"];
    let mut out = String::new();
    if ok {
        let task_ref = task["external_id"]
            .as_str()
            .or_else(|| task["id"].as_str())
            .unwrap_or("");
        let mut msg = format!("Saved verified memory ({} record(s))", records.len());
        if !task_ref.is_empty() {
            msg.push_str(&format!(" for {task_ref}"));
        }
        msg.push('.');
        out.push_str(&format!(
            "<p class=\"results-memory-status\" data-state=\"saved\">{}</p>",
            escape_html(&msg)
        ));
    } else if !error.is_empty() {
        out.push_str(&format!(
            "<p class=\"results-memory-status\" data-state=\"error\">{}</p>",
            escape_html(error)
        ));
    }
    if !skipped.is_empty() {
        out.push_str("<p class=\"knowledge-note\">Skipped items:</p><ul class=\"advisory-list\">");
        for item in &skipped {
            let title = item["title"]
                .as_str()
                .or_else(|| item.as_str())
                .unwrap_or("item");
            let reason = item["reason"].as_str().unwrap_or("skipped");
            out.push_str(&format!(
                "<li class=\"advisory-item\" data-kind=\"skipped\"><span class=\"advisory-badge\" data-kind=\"warning\">{}</span><span class=\"advisory-text\">{}</span></li>",
                escape_html(reason),
                escape_html(title)
            ));
        }
        out.push_str("</ul>");
    }
    out
}

pub(super) fn read_memory_learnings_from_dom() -> Vec<V> {
    dom::all("#results-memory-learnings .results-memory-learning-row")
        .into_iter()
        .map(|row| {
            let ltype = row
                .query_selector(".results-memory-learning-type")
                .ok()
                .flatten()
                .map(|el| dom::value_of(&el))
                .unwrap_or_else(|| "finding".to_owned());
            let title = row
                .query_selector(".results-memory-learning-title")
                .ok()
                .flatten()
                .map(|el| dom::value_of(&el))
                .unwrap_or_default();
            let consequence = row
                .query_selector(".results-memory-learning-consequence")
                .ok()
                .flatten()
                .map(|el| dom::value_of(&el))
                .unwrap_or_default();
            let sources_raw = row
                .query_selector(".results-memory-learning-sources")
                .ok()
                .flatten()
                .map(|el| dom::value_of(&el))
                .unwrap_or_default();
            let sources: Vec<String> = sources_raw
                .split([',', '\n'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            json!({
                "type": ltype,
                "title": title,
                "consequence": consequence,
                "sources": sources,
            })
        })
        .collect()
}

pub(super) fn render_memory_learnings_dom(learnings: &[V]) {
    if let Some(container) = dom::get("results-memory-learnings") {
        container.set_inner_html(&render_memory_learnings_html(learnings));
    }
}

pub(super) fn prefill_results_memory() {
    let session = m::read(|s| s.selected.clone());
    let results = m::data("results");
    let delivery = m::data("delivery");
    let preview = m::data("delivery-preview");
    let draft = build_memory_draft_prefill(&session, &results, &delivery, &preview);
    dom::set_value("results-memory-progress", &m::text(&draft, "progress"));
    let steps = m::array(&draft["next_steps"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect::<Vec<_>>()
        .join("\n");
    dom::set_value("results-memory-next-steps", &steps);
    render_memory_learnings_dom(&m::array(&draft["learnings"]));
    let used = m::array(&draft["used_memory"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect::<Vec<_>>()
        .join("\n");
    dom::set_value("results-memory-used", &used);
}

pub(super) fn sync_results_memory_prefill(sid: &str) {
    if m::data("results-memory-sid") != sid || !m::yes(&m::data("results-memory-dirty")) {
        m::set("results-memory-sid", json!(sid));
        prefill_results_memory();
    }
}

pub(super) async fn save_results_memory() {
    let sid = m::read(|s| s.sid.clone());
    if sid.is_empty() || m::read(|s| s.pending || m::active(&s.selected) || s.auth_failed) {
        return;
    }
    dom::error("results-memory-error", "");
    let learnings = read_memory_learnings_from_dom();
    let payload = match build_memory_save_payload(
        &dom::value("results-memory-progress"),
        &dom::value("results-memory-next-steps"),
        &learnings,
        &dom::value("results-memory-used"),
        dom::checked("results-memory-verified"),
    ) {
        Ok(body) => body,
        Err(msg) => {
            dom::error("results-memory-error", &msg);
            return;
        }
    };
    m::write(|s| s.pending = true);
    controls();
    let response = api::post(&format!("/api/sessions/{}/memory", dom::enc(&sid)), payload).await;
    m::write(|s| s.pending = false);
    match response {
        Ok(v) => {
            let html = format_memory_save_feedback_html(&v);
            if let Some(el) = dom::get("results-memory-feedback") {
                el.set_inner_html(&html);
                dom::hide(&el, html.is_empty());
            }
            if m::yes(&v["ok"]) {
                dom::set_checked("results-memory-verified", false);
                m::take("results-memory-dirty");
            } else if let Some(err) = v["error"].as_str().filter(|s| !s.trim().is_empty()) {
                dom::error("results-memory-error", err);
            }
        }
        Err(e) => {
            dom::error("results-memory-error", &e.message);
        }
    }
    controls();
}
