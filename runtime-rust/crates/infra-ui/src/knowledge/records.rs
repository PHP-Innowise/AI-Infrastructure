use super::usage::{capsule_bar_label, render_capsule_bar_html, render_capsule_legend_html};
use crate::{api, dom, model as m, session};
use serde_json::{json, Value as V};
use std::collections::BTreeSet;

pub(super) fn definitions(action: &str) -> V {
    match action {
        "start" => json!([
            ["task_id", "Task ID", "text", true],
            ["goal", "Goal", "textarea", true],
            ["files", "Files", "lines", false],
            ["sources", "Sources", "lines", false]
        ]),
        "brain-create" => json!([
            [
                "record_type",
                "Record type",
                "select",
                true,
                ["finding", "bug", "incident", "decision", "event"]
            ],
            ["external_id", "External ID", "text", true],
            ["title", "Title", "text", true],
            ["goal", "Goal", "textarea", false],
            ["files", "Files", "lines", false],
            ["sources", "Sources", "lines", false]
        ]),
        "brain-update" => json!([
            ["record_id", "Record ID", "text", true],
            ["revision", "Current revision", "number", true],
            ["progress", "Replace progress", "textarea", false],
            ["next_steps", "Append next steps", "lines", false],
            [
                "phase",
                "Task phase",
                "select",
                false,
                [
                    "",
                    "understanding",
                    "planning",
                    "implementation",
                    "verification",
                    "finalization"
                ]
            ],
            ["transition", "Status transition", "text", false],
            ["reason", "Reason", "textarea", false]
        ]),
        "complete" => json!([
            ["task_id", "Task ID", "text", true],
            ["revision", "Current revision", "number", true],
            ["outcome", "Outcome", "textarea", true],
            ["verification", "Verification", "lines", false],
            ["sources", "Sources", "lines", false]
        ]),
        "export" => json!([
            ["include_archive", "Include archive", "checkbox", false],
            [
                "include_superseded",
                "Include superseded records",
                "checkbox",
                false
            ]
        ]),
        "bank-reverify" => json!([
            ["memory_id", "Memory ID", "text", true],
            ["review_after", "Next review date", "date", false]
        ]),
        "bank-retire" => json!([
            ["memory_id", "Memory ID", "text", true],
            ["valid_to", "Valid through", "date", true],
            ["superseded_by", "Replacement memory ID", "text", false],
            ["reason", "Reason", "textarea", false]
        ]),
        _ => json!([]),
    }
}
pub(super) fn fields(scope: &str) {
    let action = dom::value(&format!("{scope}-knowledge-action"));
    let container = format!("{scope}-knowledge-fields");
    dom::clear(&container);
    for f in m::array(&definitions(&action)) {
        let name = f[0].as_str().unwrap_or("");
        let label = f[1].as_str().unwrap_or("");
        let kind = f[2].as_str().unwrap_or("text");
        let wrap = dom::el("label", "field", label);
        let tag = if kind == "select" {
            "select"
        } else if ["textarea", "lines"].contains(&kind) {
            "textarea"
        } else {
            "input"
        };
        let input = dom::el(tag, "", "");
        let _ = input.set_attribute("id", &format!("{scope}-op-{name}"));
        let _ = input.set_attribute("name", name);
        if tag == "input" {
            let _ = input.set_attribute("type", kind);
        }
        if m::yes(&f[3]) {
            let _ = input.set_attribute("required", "");
        }
        if kind == "number" {
            let _ = input.set_attribute("min", "1");
            let _ = input.set_attribute("max", "2147483647");
            let _ = input.set_attribute("step", "1");
        }
        if tag == "textarea" {
            let _ = input.set_attribute("maxlength", "12000");
        }
        if kind == "select" {
            for v in m::array(&f[4]) {
                let e = dom::el("option", "", v.as_str().unwrap_or(""));
                let _ = e.set_attribute("value", v.as_str().unwrap_or(""));
                dom::add(&input, &e);
            }
        }
        dom::add(&wrap, &input);
        dom::append(&container, &wrap);
    }
    dom::text(
        &format!("{scope}-knowledge-note"),
        match action.as_str() {
            "bank-reverify" => {
                "By running this action you attest that you checked this chunk against its current sources."
            }
            "brain-update" => "Revision must match the current record; refresh if it changed.",
            "complete" => "Record the task outcome and verification evidence.",
            "reindex-bank" => "This regenerates the shared INDEX.md.",
            "compact" => "Archive eligible terminal records.",
            "export" => "Create a downloadable knowledge ZIP.",
            _ => "Run the selected operation against the selected knowledge root.",
        },
    );
}
pub async fn load_context() {
    let p = dom::value("context-project");
    if p.is_empty() {
        return;
    }
    match api::scoped(
        "context",
        &format!("/api/projects/{}/context", dom::enc(&p)),
    )
    .await
    {
        Ok((e, v)) if m::current("context", e) && dom::value("context-project") == p => {
            let files = m::array(&v["files"]);
            dom::text(
                "context-summary",
                &format!(
                    "{} of {} files present",
                    files.iter().filter(|x| m::yes(&x["exists"])).count(),
                    files.len()
                ),
            );
            dom::clear("context-files");
            for file in files {
                let row = dom::el("div", "file-row", "");
                for (class, text) in [
                    ("file-path", m::text(&file, "path")),
                    (
                        "file-status",
                        if m::yes(&file["exists"]) {
                            "Present".into()
                        } else {
                            "Missing".into()
                        },
                    ),
                    ("file-size", bytes(&file["bytes"])),
                ] {
                    dom::add(&row, &dom::el("span", class, &text));
                }
                dom::append("context-files", &row);
            }
        }
        Ok(_) => {}
        Err(e) if !e.aborted => dom::error("context-error", &e.message),
        Err(_) => {}
    }
}
pub fn bytes(v: &V) -> String {
    m::number(v)
        .map(|n| {
            if n < 1024. {
                format!("{n:.0} B")
            } else if n < 1048576. {
                format!("{:.1} KB", n / 1024.)
            } else {
                format!("{:.1} MB", n / 1048576.)
            }
        })
        .unwrap_or_else(|| "—".into())
}
pub(super) fn key(scope: &str) -> String {
    format!("knowledge-{scope}")
}
pub(super) fn knowledge_current(scope: &str, project: &str, bank: &str, generation: u64) -> bool {
    dom::value(&format!("{scope}-project")) == project
        && dom::value(&format!("{scope}-bank")) == bank
        && m::scope_current(
            &format!("{scope}-knowledge-scope"),
            generation,
            project,
            bank,
        )
}
pub(super) fn knowledge_ready(scope: &str) -> bool {
    let state = m::data(&key(scope));
    knowledge_current(
        scope,
        &m::text(&state, "project_id"),
        &m::text(&state, "bank_id"),
        state["view_generation"].as_u64().unwrap_or(0),
    )
}
pub(super) fn clear_knowledge_detail(scope: &str) {
    m::take(&format!("{scope}-selected"));
    for field in [
        "title",
        "path",
        "meta",
        "content",
        "metadata-content",
        "file-error",
    ] {
        dom::text(&format!("{scope}-{field}"), "");
    }
    for field in ["detail-header", "content", "metadata", "content-notice"] {
        dom::hidden(&format!("{scope}-{field}"), true);
    }
    dom::clear(&format!("{scope}-detail-actions"));
    dom::text(&format!("{scope}-message"), "Choose a record to read.");
    dom::hidden(&format!("{scope}-message"), false);
}

pub async fn open_from_omnigraph(scope: &str, project_id: &str, target_id: &str) {
    if !project_id.is_empty() && dom::value(&format!("{scope}-project")) != project_id {
        session::sync_project(project_id);
    }
    if scope == "brain" {
        dom::set_value("brain-category", "");
    }
    // The jump target chooses the record; its id is no filter to leave in
    // the list, and a filter typed earlier could hide it.
    dom::set_value(&format!("{scope}-filter"), "");
    m::set(&format!("{scope}-jump-target"), json!(target_id));
    session::open_view(scope, true).await;
}
/// The listing entry an Omnigraph jump names: by record id, external id or
/// path.
pub(crate) fn jump_entry<'a>(entries: &'a [V], target: &str) -> Option<&'a V> {
    if target.is_empty() {
        return None;
    }
    entries.iter().find(|x| {
        x["id"].as_str() == Some(target)
            || x["external_id"].as_str() == Some(target)
            || m::text(x, "path").contains(target)
    })
}

pub fn knowledge_omnigraph_atom_id(scope: &str, record: &V) -> Option<String> {
    let id = record["id"]
        .as_str()
        .or_else(|| record["metadata"]["id"].as_str())
        .unwrap_or("")
        .trim();
    if id.is_empty() {
        return None;
    }
    match scope {
        "brain" => Some(format!("brain-{id}")),
        "memory" => Some(format!("mem-{id}")),
        _ => None,
    }
}

pub fn render_detail_omnigraph_action_html(scope: &str, project_id: &str, record: &V) -> String {
    let Some(atom_id) = knowledge_omnigraph_atom_id(scope, record) else {
        return String::new();
    };
    let payload = json!({
        "project_id": project_id,
        "atom_id": atom_id,
    })
    .to_string();
    format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"knowledge-open-omnigraph\" data-project-id=\"{}\" data-atom-id=\"{}\" data-payload=\"{}\">Open in Context Graph</button>",
        crate::omnigraph::esc(project_id),
        crate::omnigraph::esc(&atom_id),
        crate::omnigraph::esc(&payload)
    )
}
pub(super) fn clear_knowledge_summary(scope: &str, clear_banks: bool) {
    if scope == "memory" {
        dom::disabled("memory-filter", true);
    }
    dom::text(&format!("{scope}-count"), "");
    dom::text(
        if scope == "brain" {
            "brain-root"
        } else {
            "memory-bank-path"
        },
        "",
    );
    dom::text(&format!("{scope}-knowledge-runtime"), "");
    dom::hidden(&format!("{scope}-list-notice"), true);
    dom::error(&format!("{scope}-error"), "");
    dom::error(&format!("{scope}-knowledge-error"), "");
    if clear_banks {
        dom::options(&format!("{scope}-bank"), &[], m::summary, None);
        dom::disabled(&format!("{scope}-bank"), true);
    }
}
pub fn reset_project(scope: &str) {
    if cfg!(target_arch = "wasm32") {
        clear_knowledge_summary(scope, true);
    }
    for channel in ["list", "file", "meta"] {
        m::next(&format!("{scope}-{channel}"));
    }
    m::take(&key(scope));
    m::take(&format!("{scope}-meta"));
    if cfg!(target_arch = "wasm32") {
        clear_knowledge_detail(scope);
        dom::clear(&format!("{scope}-files"));
        dom::clear(&format!("{scope}-knowledge-output"));
        if scope == "memory" {
            dom::clear("memory-search-results");
        }
        tool_controls(scope);
    }
}
pub async fn load(scope: &str, bank_changed: bool) {
    let p = dom::value(&format!("{scope}-project"));
    let old = m::data(&key(scope));
    clear_knowledge_summary(scope, old["project_id"] != p);
    let bank = if old["project_id"] == p {
        dom::value(&format!("{scope}-bank"))
    } else {
        String::new()
    };
    let scope_key = format!("{scope}-knowledge-scope");
    let generation = m::bind_scope(&scope_key, &p, &bank);
    m::next(&format!("{scope}-file"));
    m::next(&format!("{scope}-meta"));
    m::take(&format!("{scope}-meta"));
    clear_knowledge_detail(scope);
    m::set(
        &key(scope),
        json!({"project_id":p,"bank_id":bank,"entries":[],"selected_path":""}),
    );
    dom::clear(&format!("{scope}-files"));
    for output in [
        format!("{scope}-knowledge-output"),
        format!("{scope}-knowledge-error"),
    ] {
        dom::text(&output, "");
    }
    if scope == "memory" {
        dom::clear("memory-search-results");
    }
    tool_controls(scope);
    if p.is_empty() {
        dom::text(&format!("{scope}-count"), "No project selected.");
        return;
    }
    let path = format!(
        "/api/projects/{}/{}{}",
        dom::enc(&p),
        scope,
        dom::query(&[("bank", bank.clone())])
    );
    let channel = format!("{scope}-list");
    dom::text(&format!("{scope}-count"), "Loading…");
    let result = api::scoped(&channel, &path).await;
    let result = match result {
        Err(e)
            if !bank.is_empty()
                && [400, 404].contains(&e.status)
                && m::scope_current(&scope_key, generation, &p, &bank)
                && dom::value(&format!("{scope}-project")) == p =>
        {
            api::scoped(
                &channel,
                &format!("/api/projects/{}/{}", dom::enc(&p), scope),
            )
            .await
        }
        other => other,
    };
    match result {
        Ok((e, mut v))
            if m::current(&channel, e)
                && dom::value(&format!("{scope}-project")) == p
                && m::scope_current(&scope_key, generation, &p, &bank) =>
        {
            v["project_id"] = json!(p);
            let entries = m::array(&v["entries"]);
            let jump_peek = m::data(&format!("{scope}-jump-target"))
                .as_str()
                .map(str::to_owned)
                .filter(|s| !s.is_empty());
            let initial_saved =
                if !bank_changed && old["project_id"] == p && old["bank_id"] == v["bank_id"] {
                    m::text(&old, "selected_path")
                } else {
                    String::new()
                };
            let selected =
                resolve_initial_selection(scope, &entries, &initial_saved, jump_peek.as_deref())
                    .unwrap_or_default();
            v["selected_path"] = json!(selected);
            dom::options(
                &format!("{scope}-bank"),
                &m::array(&v["banks"]),
                m::summary,
                Some(&m::text(&v, "bank_id")),
            );
            dom::disabled(&format!("{scope}-bank"), m::array(&v["banks"]).is_empty());
            let current_bank = m::text(&v, "bank_id");
            let view_generation = m::bind_scope(&scope_key, &p, &current_bank);
            v["view_generation"] = json!(view_generation);
            m::set(&key(scope), v.clone());
            dom::text(
                if scope == "brain" {
                    "brain-root"
                } else {
                    "memory-bank-path"
                },
                &m::text(&v, "root"),
            );
            dom::hidden(&format!("{scope}-list-notice"), !m::yes(&v["truncated"]));
            if scope == "brain" {
                let mut cats = BTreeSet::new();
                for e in &entries {
                    cats.insert(category(e));
                }
                let mut options = vec![json!({"id":"","name":"All categories"})];
                options.extend(
                    cats.into_iter()
                        .map(|v| json!({"id":v,"name":m::label(&v)})),
                );
                dom::options("brain-category", &options, m::summary, None);
            }
            render_list(scope);
            metadata(scope).await;
            if !knowledge_current(scope, &p, &current_bank, view_generation) {
                return;
            }
            let jump = m::take(&format!("{scope}-jump-target"))
                .as_str()
                .map(str::to_owned)
                .filter(|s| !s.is_empty());
            let selected_path =
                resolve_initial_selection(scope, &entries, &selected, jump.as_deref());
            if let Some(path) = selected_path {
                select(scope, &path).await;
            } else {
                dom::text(&format!("{scope}-message"), "No readable records found.");
            }
        }
        Ok(_) => {}
        Err(e)
            if !e.aborted
                && dom::value(&format!("{scope}-project")) == p
                && m::scope_current(&scope_key, generation, &p, &bank) =>
        {
            dom::error(&format!("{scope}-error"), &e.message)
        }
        Err(_) => {}
    }
}
pub(super) fn resolve_initial_selection(
    scope: &str,
    entries: &[V],
    saved_path: &str,
    jump_target: Option<&str>,
) -> Option<String> {
    jump_target
        .and_then(|target| jump_entry(entries, target))
        .or_else(|| entries.iter().find(|x| x["path"] == saved_path))
        .or_else(|| {
            entries.iter().find(|x| {
                if scope == "memory" {
                    x["kind"] == "chunk"
                } else {
                    x["type"] == "task" && !m::text(x, "path").starts_with("archive/")
                }
            })
        })
        .or_else(|| entries.first())
        .map(|v| m::text(v, "path"))
}

pub(super) fn sync_list_selection(scope: &str, selected_path: &str) {
    let Some(container) = dom::get(&format!("{scope}-files")) else {
        return;
    };
    for child in dom::children(&container) {
        if child.get_attribute("data-path").as_deref() == Some(selected_path) {
            if child.get_attribute("aria-current").as_deref() != Some("true") {
                let _ = child.set_attribute("aria-current", "true");
            }
        } else if child.has_attribute("aria-current") {
            let _ = child.remove_attribute("aria-current");
        }
    }
}

pub(super) fn category(v: &V) -> String {
    let p = m::text(v, "path");
    let c = v["category"].as_str().unwrap_or(&p);
    if c.starts_with("archive/") {
        "archive".into()
    } else if c.starts_with("dynamic/") || c.starts_with("control/") {
        c.split('/').nth(1).unwrap_or("documents").into()
    } else {
        v["type"].as_str().unwrap_or("documents").into()
    }
}
pub(super) fn knowledge_entry_matches(v: &V, query_lower: &str, cat: &str) -> bool {
    if !cat.is_empty() && category(v) != cat {
        return false;
    }
    if query_lower.is_empty() {
        return true;
    }
    let text = [
        "title",
        "path",
        "id",
        "external_id",
        "status",
        "owner",
        "type",
    ]
    .iter()
    .map(|k| m::text(v, k))
    .collect::<Vec<_>>()
    .join(" ");
    text.to_lowercase().contains(query_lower)
}

pub(super) fn knowledge_visible_paths(
    entries: &[V],
    query_lower: &str,
    cat: &str,
) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|v| knowledge_entry_matches(v, query_lower, cat))
        .map(|v| m::text(v, "path"))
        .collect()
}

pub(super) fn build_knowledge_list_items(scope: &str, entries: &[V]) -> Vec<V> {
    entries
        .iter()
        .map(|v| {
            json!({
                "scope": scope,
                "path": m::text(v, "path"),
                "title": v["title"],
                "name": v["name"],
                "label": v["label"],
                "summary": v["summary"],
                "message": v["message"],
                "id": v["id"],
                "status": v["status"],
                "bytes": v["bytes"],
            })
        })
        .collect()
}

fn build_knowledge_entry_button(item: &V) -> web_sys::Element {
    let scope = m::text(item, "scope");
    let path = m::text(item, "path");
    let b = dom::button("", "knowledge-file", json!({"scope": scope, "path": path}));
    b.set_inner_html(&render_list_entry_inner_html(item));
    b.set_class_name("memory-entry");
    let _ = b.set_attribute("data-path", &path);
    let _ = b.set_attribute("data-focus-key", &format!("knowledge:{scope}:{path}"));
    b
}

pub(super) fn render_list(scope: &str) {
    let state = m::data(&key(scope));
    let query = dom::value(&format!("{scope}-filter")).to_lowercase();
    let cat = if scope == "brain" {
        dom::value("brain-category")
    } else {
        String::new()
    };
    let entries = m::array(&state["entries"]);
    if scope == "memory" {
        dom::disabled("memory-filter", entries.is_empty());
    }
    let items = build_knowledge_list_items(scope, &entries);
    let visible_paths = knowledge_visible_paths(&entries, &query, &cat);
    let selected_path = m::text(&state, "selected_path");
    let container_id = format!("{scope}-files");
    dom::keyed_render(
        &container_id,
        &items,
        |item| m::text(item, "path"),
        build_knowledge_entry_button,
    );
    let mut visible_count = 0usize;
    if let Some(container) = dom::get(&container_id) {
        for child in dom::children(&container) {
            let path = child.get_attribute("data-path").unwrap_or_default();
            let visible = visible_paths.contains(&path);
            if visible {
                visible_count += 1;
            }
            dom::hide(&child, !visible);
            if path == selected_path && !selected_path.is_empty() {
                if child.get_attribute("aria-current").as_deref() != Some("true") {
                    let _ = child.set_attribute("aria-current", "true");
                }
            } else if child.has_attribute("aria-current") {
                let _ = child.remove_attribute("aria-current");
            }
        }
    }
    dom::text(
        &format!("{scope}-count"),
        &format!(
            "{visible_count} of {} records{}",
            entries.len(),
            if m::yes(&state["truncated"]) {
                " · listing limited"
            } else {
                ""
            }
        ),
    );
}
pub(super) async fn metadata(scope: &str) {
    let state = m::data(&key(scope));
    let p = m::text(&state, "project_id");
    let bank = m::text(&state, "bank_id");
    let generation = state["view_generation"].as_u64().unwrap_or(0);
    if bank.is_empty() || !knowledge_current(scope, &p, &bank, generation) {
        return;
    }
    let channel = format!("{scope}-meta");
    if let Ok((e, v)) = api::scoped(
        &channel,
        &format!(
            "/api/projects/{}/knowledge{}",
            dom::enc(&p),
            dom::query(&[("bank", bank.clone())])
        ),
    )
    .await
    {
        if m::current(&channel, e)
            && knowledge_current(scope, &p, &bank, generation)
            && v["project_id"] == p
            && v["bank_id"] == bank
        {
            m::set(&format!("{scope}-meta"), v.clone());
            dom::text(
                &format!("{scope}-knowledge-runtime"),
                &format!("{} mode · {}", m::text(&v, "mode"), m::text(&v, "root")),
            );
            tool_controls(scope);
        }
    }
}
pub(super) fn tool_controls(scope: &str) {
    let meta = m::data(&format!("{scope}-meta"));
    let state = m::data(&key(scope));
    let enabled = knowledge_ready(scope)
        && meta["project_id"] == state["project_id"]
        && meta["bank_id"] == state["bank_id"]
        && m::yes(&meta["runtime_available"])
        && !m::read(|s| s.auth_failed)
        && !m::yes(&m::data("knowledge-pending"));
    let operation = dom::value(&format!("{scope}-knowledge-action"));
    let governed = meta["mode"] == "governed";
    let can = enabled
        && (!["start", "brain-create", "brain-update", "complete"].contains(&operation.as_str())
            || governed);
    dom::disabled(&format!("{scope}-knowledge-submit"), !can);
    dom::disabled(&format!("{scope}-knowledge-action"), !enabled);
    crate::session::sync_working_indicator();
}
pub(super) async fn select(scope: &str, path: &str) {
    if !knowledge_ready(scope) {
        return;
    }
    let mut state = m::data(&key(scope));
    if !m::array(&state["entries"])
        .iter()
        .any(|entry| entry["path"] == path)
    {
        return;
    }
    let generation = state["view_generation"].as_u64().unwrap_or(0);
    clear_knowledge_detail(scope);
    state["selected_path"] = json!(path);
    m::set(&key(scope), state.clone());
    sync_list_selection(scope, path);
    let project = m::text(&state, "project_id");
    let bank = m::text(&state, "bank_id");
    let channel = format!("{scope}-file");
    let url = format!(
        "/api/projects/{}/{}{}",
        dom::enc(&project),
        scope,
        dom::query(&[("bank", bank.clone()), ("path", path.into())])
    );
    match api::scoped(&channel, &url).await {
        Ok((e, v))
            if m::current(&channel, e)
                && knowledge_current(scope, &project, &bank, generation)
                && m::data(&key(scope))["selected_path"] == path =>
        {
            m::set(&format!("{scope}-selected"), v.clone());
            dom::hidden(&format!("{scope}-detail-header"), false);
            dom::text(&format!("{scope}-title"), &m::summary(&v));
            dom::text(&format!("{scope}-path"), &m::text(&v, "path"));
            dom::text(
                &format!("{scope}-meta"),
                &format!(
                    "{} · {} · revision {}",
                    m::text(&v, "type"),
                    m::text(&v, "status"),
                    m::string(&v["revision"])
                ),
            );
            if let Some(el) = dom::get(&format!("{scope}-detail-actions")) {
                el.set_inner_html(&render_detail_omnigraph_action_html(scope, &project, &v));
            }
            let content = m::text(&v, "content");
            let normalized = content.replace("\r\n", "\n");
            let (metadata, body) = if let Some(after) = normalized.strip_prefix("---\n") {
                if let Some((front, body)) = after.split_once("\n---\n") {
                    (Some(front), body.trim_start())
                } else {
                    (None, normalized.as_str())
                }
            } else {
                (None, normalized.as_str())
            };
            dom::hidden(&format!("{scope}-metadata"), metadata.is_none());
            dom::text(&format!("{scope}-metadata-content"), metadata.unwrap_or(""));
            dom::text(&format!("{scope}-content"), body);
            dom::hidden(&format!("{scope}-content"), false);
            dom::hidden(&format!("{scope}-message"), true);
            dom::hidden(&format!("{scope}-content-notice"), !m::yes(&v["truncated"]));
        }
        Ok(_) => {}
        Err(e)
            if !e.aborted
                && knowledge_current(scope, &project, &bank, generation)
                && m::data(&key(scope))["selected_path"] == path =>
        {
            dom::error(&format!("{scope}-file-error"), &e.message)
        }
        Err(_) => {}
    }
}
pub(super) async fn operation(scope: &str, action: &str, mut fields: V) {
    let state = m::data(&key(scope));
    let project = m::text(&state, "project_id");
    if !knowledge_ready(scope) || m::yes(&m::data("knowledge-pending")) {
        return;
    }
    let bank = m::text(&state, "bank_id");
    let generation = state["view_generation"].as_u64().unwrap_or(0);
    let meta = m::data(&format!("{scope}-meta"));
    if !m::yes(&meta["runtime_available"])
        || meta["project_id"] != project
        || meta["bank_id"] != bank
    {
        return;
    }
    fields["bank"] = json!(bank);
    fields["action"] = json!(action);
    let operation_generation = m::next("knowledge-operation");
    m::bind_pending("knowledge-pending", operation_generation);
    for s in ["brain", "memory"] {
        tool_controls(s);
    }
    let output = if action == "search" {
        "memory-search-results".into()
    } else {
        format!("{scope}-knowledge-output")
    };
    dom::hidden(&output, false);
    dom::text(&output, "Running…");
    match api::post(
        &format!("/api/projects/{}/knowledge", dom::enc(&project)),
        fields,
    )
    .await
    {
        Ok(v) if knowledge_current(scope, &project, &bank, generation) => {
            dom::clear(&output);
            dom::append(
                &output,
                &dom::el(
                    "p",
                    "knowledge-note",
                    if m::yes(&v["ok"]) {
                        "Command complete."
                    } else {
                        "Command failed."
                    },
                ),
            );
            if !v["error"].is_null() {
                dom::append(
                    &output,
                    &dom::el("p", "error-text", &m::string(&v["error"])),
                );
            }
            if action == "search" && v["result"].is_object() {
                for doc in m::array(&v["result"]["documents"]) {
                    let a = dom::el("article", "knowledge-search-result", "");
                    dom::add(&a, &dom::el("h4", "", &m::summary(&doc)));
                    dom::add(
                        &a,
                        &dom::el(
                            "p",
                            "memory-detail-path",
                            &format!(
                                "{} · {} · {}",
                                m::text(&doc, "path"),
                                m::text(&doc, "layer"),
                                m::text(&doc, "kind")
                            ),
                        ),
                    );
                    dom::add(&a, &dom::el("p", "", &m::text(&doc, "snippet")));
                    dom::append(&output, &a);
                }
                for episode in m::array(&v["result"]["episodes"]) {
                    dom::append(
                        &output,
                        &dom::details(
                            &m::summary(&episode),
                            &serde_json::to_string_pretty(&episode).unwrap_or_default(),
                        ),
                    );
                }
            } else {
                dom::append(
                    &output,
                    &dom::el(
                        "pre",
                        "knowledge-result",
                        &if v["result"].is_string() {
                            m::string(&v["result"])
                        } else {
                            serde_json::to_string_pretty(&v["result"]).unwrap_or_default()
                        },
                    ),
                );
            }
            if let Some(id) = v["download_id"].as_str() {
                let a = dom::el("a", "button", "Download ZIP");
                let _ =
                    a.set_attribute("href", &format!("/api/knowledge/exports/{}", dom::enc(id)));
                let _ = a.set_attribute("download", "");
                dom::append(&output, &a);
            }
            if m::yes(&v["ok"]) && action != "search" && m::read(|s| s.view == scope) {
                load(scope, false).await;
            }
        }
        Err(e) if knowledge_current(scope, &project, &bank, generation) => dom::error(
            &format!("{scope}-knowledge-error"),
            &format!(
                "{} The command may have run. Refresh before retrying.",
                e.message
            ),
        ),
        _ => {}
    }
    if m::finish_pending("knowledge-pending", operation_generation) {
        for scope in ["brain", "memory"] {
            tool_controls(scope);
        }
    }
}
pub(super) async fn submit(scope: &str) {
    let action = dom::value(&format!("{scope}-knowledge-action"));
    if !dom::validity(&format!("{scope}-knowledge-form")) {
        return;
    }
    let mut data = json!({});
    for f in m::array(&definitions(&action)) {
        let name = f[0].as_str().unwrap_or("");
        let kind = f[2].as_str().unwrap_or("");
        let id = format!("{scope}-op-{name}");
        let value = dom::value(&id).trim().to_owned();
        match kind {
            "checkbox" => data[name] = json!(dom::checked(&id)),
            "number" => {
                let n = value.parse::<u64>().ok();
                if !n.map(|n| (1..=2147483647).contains(&n)).unwrap_or(false) {
                    dom::error(
                        &format!("{scope}-knowledge-error"),
                        "Revision must be a whole number from 1 to 2147483647.",
                    );
                    return;
                }
                data[name] = json!(n);
            }
            "lines" => {
                let lines = value
                    .lines()
                    .map(str::trim)
                    .filter(|x| !x.is_empty())
                    .collect::<Vec<_>>();
                if lines.len() > 50 {
                    dom::error(
                        &format!("{scope}-knowledge-error"),
                        "Use at most 50 lines per field.",
                    );
                    return;
                }
                data[name] = json!(lines);
            }
            _ => {
                if !value.is_empty() {
                    data[name] = json!(value);
                }
            }
        }
    }
    if action == "brain-update"
        && data["progress"].is_null()
        && m::array(&data["next_steps"]).is_empty()
        && data["phase"].is_null()
        && data["transition"].is_null()
    {
        dom::error(
            &format!("{scope}-knowledge-error"),
            "Enter progress, a next step, phase or transition.",
        );
        return;
    }
    operation(scope, &action, data).await;
}
pub(super) fn selected(scope: &str) {
    let v = m::data(&format!("{scope}-selected"));
    let action = dom::value(&format!("{scope}-knowledge-action"));
    if scope == "brain" {
        let name = if action == "complete" {
            "task_id"
        } else {
            "record_id"
        };
        dom::set_value(
            &format!("brain-op-{name}"),
            if action == "complete" {
                v["external_id"]
                    .as_str()
                    .or_else(|| v["id"].as_str())
                    .unwrap_or("")
            } else {
                v["id"].as_str().unwrap_or("")
            },
        );
        dom::set_value("brain-op-revision", &m::string(&v["revision"]));
    } else {
        dom::set_value("memory-op-memory_id", &m::text(&v, "id"));
    }
}
pub async fn load_brain_link() {
    let p = dom::value("project");
    if p.is_empty() || !m::read(|s| s.sid.is_empty()) {
        return;
    }
    let bank = dom::value("brain-link-bank");
    let path = format!(
        "/api/projects/{}/brain{}",
        dom::enc(&p),
        dom::query(&[("bank", bank)])
    );
    m::set("brain-link-loading", json!(true));
    match api::scoped("brain-link", &path).await {
        Ok((e, v)) if m::current("brain-link", e) && dom::value("project") == p => {
            let tasks = m::array(&v["entries"])
                .into_iter()
                .filter(|v| {
                    v["type"] == "task"
                        && !["completed", "cancelled"].contains(&v["status"].as_str().unwrap_or(""))
                        && !m::text(v, "path").starts_with("archive/")
                })
                .collect::<Vec<_>>();
            dom::options(
                "brain-link-bank",
                &m::array(&v["banks"]),
                m::summary,
                Some(&m::text(&v, "bank_id")),
            );
            dom::options(
                "brain-link-task",
                &tasks,
                |v| {
                    format!(
                        "{} · {}",
                        v["external_id"]
                            .as_str()
                            .unwrap_or_else(|| v["id"].as_str().unwrap_or("")),
                        m::summary(v)
                    )
                },
                None,
            );
            m::set("brain-link-meta", v);
            m::set("brain-link-tasks", json!(tasks));
        }
        Ok(_) => {}
        Err(e) if !e.aborted => dom::error("brain-link-error", &e.message),
        Err(_) => {}
    }
    m::set("brain-link-loading", json!(false));
    linked_controls();
}
pub fn brain_config() -> Option<V> {
    let kind = dom::value("brain-link-kind");
    let review = dom::checked("brain-link-review");
    let meta = m::data("brain-link-meta");
    let strict = review || kind != "auto";
    if !strict
        && (!m::yes(&meta["runtime_available"])
            || meta["mode"] != "governed"
            || dom::value("workflow") == "fleet-review" && dom::checked("fleet-dry-run"))
    {
        return None;
    }
    let mut v = json!({"bank":dom::value("brain-link-bank"),"review":review});
    if review {
        v["query"] = json!(dom::value("brain-link-query").trim());
    }
    if kind == "auto" {
        v["auto"] = json!(true);
    } else if kind == "create" {
        v["create"] = json!(true);
        v["task_id"] = json!(dom::value("brain-link-task-id").trim());
        v["goal"] = json!(dom::value("brain-link-goal").trim());
    } else {
        let id = dom::value("brain-link-task");
        let task = m::array(&m::data("brain-link-tasks"))
            .into_iter()
            .find(|v| v["id"] == id)
            .unwrap_or(V::Null);
        v["create"] = json!(false);
        v["task_id"] = task["external_id"]
            .as_str()
            .or_else(|| task["id"].as_str())
            .map(|s| json!(s))
            .unwrap_or(V::Null);
        v["record_id"] = task["id"].clone();
    }
    Some(v)
}
pub fn linked_controls() {
    let kind = dom::value("brain-link-kind");
    dom::hidden("brain-link-existing-field", kind != "existing");
    dom::hidden("brain-link-id-field", kind != "create");
    dom::hidden("brain-link-goal-field", kind != "create");
    dom::hidden("brain-link-query-field", !dom::checked("brain-link-review"));
    let meta = m::data("brain-link-meta");
    dom::text(
        "brain-link-availability",
        if m::yes(&m::data("brain-link-loading")) {
            "Reading knowledge roots and tasks…"
        } else if !m::yes(&meta["runtime_available"]) {
            "No context runtime; sessions use the project’s reference files."
        } else if meta["mode"] != "governed" {
            "Brain tasks require governed mode."
        } else {
            "Each message retrieves project memory and saves checked results."
        },
    );
    dom::text(
        "brain-link-summary",
        if dom::checked("brain-link-review") {
            "Review"
        } else if kind == "auto" {
            "Automatic"
        } else {
            "Named task"
        },
    );
    // A selected session's follow-up may change its task context instead.
    let (selected, session, locked) = m::read(|s| {
        (
            !s.sid.is_empty(),
            s.selected.clone(),
            s.pending || m::active(&s.selected),
        )
    });
    let follow = selected && session["brain"].is_object();
    dom::hidden("brain-followup-config", !follow);
    if selected {
        let review = follow && dom::checked("brain-followup-review");
        dom::disabled("brain-followup-review", locked);
        dom::disabled("brain-followup-query", locked);
        dom::hidden("brain-followup-query-field", !review);
        dom::text("brain-followup-hint", brain_followup_hint(review));
        dom::text(
            "brain-link-summary",
            if !follow {
                "Off"
            } else if review {
                "Review"
            } else {
                "Automatic"
            },
        );
    }
}
pub(super) fn brain_followup_hint(review: bool) -> &'static str {
    if review {
        "The next run waits until you approve the context this query retrieves."
    } else {
        "The next message is the retrieval query, as in every automatic turn."
    }
}
/// The follow-up controls for a session's linked Brain task: whether its
/// next context is reviewed by hand, and the query a reviewed context
/// retrieves with. The task, knowledge root and goal stay as the session
/// started them.
pub(crate) fn render_brain_followup_html(task: &str, review: bool, query: &str) -> String {
    let esc = crate::omnigraph::esc;
    format!(
        "<p class=\"knowledge-note\">Linked task <code>{}</code>. A follow-up may change how its context is retrieved.</p><label class=\"checkbox brain-link-review\"><input id=\"brain-followup-review\" type=\"checkbox\"{}>Review context by hand</label><div class=\"knowledge-form-grid\" id=\"brain-followup-query-field\"{}><label class=\"field wide\">Context query<textarea id=\"brain-followup-query\" maxlength=\"4000\" rows=\"2\" placeholder=\"A concise topic, files, or question for retrieval\">{}</textarea></label></div><p class=\"knowledge-note\" id=\"brain-followup-hint\">{}</p>",
        esc(task),
        if review { " checked" } else { "" },
        if review { "" } else { " hidden" },
        esc(query),
        brain_followup_hint(review)
    )
}
/// What a follow-up changes in its session's task context: `review`, and
/// the query a reviewed context retrieves with. An automatic context takes
/// the next message as its query, so it gets none; `None` when nothing
/// changed.
pub(crate) fn followup_brain_from_fields(
    brain: &V,
    review: bool,
    query: &str,
) -> Result<Option<V>, &'static str> {
    if !brain.is_object() {
        return Ok(None);
    }
    let mut changes = serde_json::Map::new();
    if review != (brain["review"] == true) {
        changes.insert("review".into(), json!(review));
    }
    if review {
        let query = query.trim();
        if query.is_empty() {
            return Err("Enter a context query to review, or turn off Review context by hand.");
        }
        if query != brain["query"].as_str().unwrap_or("").trim() {
            changes.insert("query".into(), json!(query));
        }
    }
    Ok((!changes.is_empty()).then_some(V::Object(changes)))
}
/// The selected session's follow-up change to its task context.
pub fn followup_brain(session: &V) -> Result<Option<V>, &'static str> {
    if dom::get("brain-followup-review").is_none() {
        return Ok(None);
    }
    followup_brain_from_fields(
        &session["brain"],
        dom::checked("brain-followup-review"),
        &dom::value("brain-followup-query"),
    )
}
/// Render the follow-up controls when the selection or the session's own
/// task context changes, and keep what the person typed between polls.
pub(super) fn sync_brain_followup(session: &V) {
    let brain = &session["brain"];
    let signature = json!([
        m::read(|s| s.epoch),
        session["id"],
        brain["task_id"],
        brain["review"],
        brain["query"]
    ]);
    if m::data("brain-followup-signature") == signature {
        return;
    }
    m::set("brain-followup-signature", signature);
    if let Some(host) = dom::get("brain-followup-config") {
        host.set_inner_html(&if brain.is_object() {
            render_brain_followup_html(
                &m::text(brain, "task_id"),
                brain["review"] == true,
                brain["query"].as_str().unwrap_or(""),
            )
        } else {
            String::new()
        });
    }
}
pub fn render_linked_omnigraph_html(session: &V) -> (bool, String) {
    let metrics = crate::omnigraph::session_bar_metrics(session);
    if !metrics.enabled {
        return (false, String::new());
    }
    let waiting = session["status"] == "awaiting_context";
    let capsule_text = m::text(&session["omnigraph"], "capsule_text");
    let mut badges = format!(
        "<span class=\"omni-badge\" data-kind=\"branch\">{}</span><span class=\"omni-meta-pill\">{} atom(s)</span><span class=\"omni-meta-pill\">{} / {} chars</span>",
        crate::omnigraph::esc(&metrics.branch),
        metrics.atom_count,
        crate::omnigraph::format_count(metrics.capsule_chars),
        crate::omnigraph::format_count(metrics.capsule_limit)
    );
    if metrics.verified_count > 0 {
        badges.push_str(&format!(
            "<span class=\"omni-badge\" data-kind=\"success\">{} verified</span>",
            metrics.verified_count
        ));
    }
    if metrics.unverified_count > 0 {
        badges.push_str(&format!(
            "<span class=\"omni-badge\" data-kind=\"warning\">{} unverified reference</span>",
            metrics.unverified_count
        ));
    }
    if waiting {
        badges.push_str("<span class=\"omni-meta-pill\">Included before run</span>");
    }
    let warning_note = if metrics.unverified_count > 0 {
        "<p class=\"knowledge-note linked-omnigraph-warning\">Contains unverified reference atoms; verify claims against repository files, Project Brain, and Memory Bank.</p>"
    } else {
        ""
    };
    let preview = if capsule_text.trim().is_empty() {
        String::new()
    } else {
        format!(
            "<details class=\"linked-omnigraph-preview\"><summary>Omnigraph capsule preview ({} / {} chars)</summary><pre class=\"knowledge-result\" tabindex=\"0\">{}</pre></details>",
            crate::omnigraph::format_count(metrics.capsule_chars),
            crate::omnigraph::format_count(metrics.capsule_limit),
            crate::omnigraph::esc(&capsule_text)
        )
    };
    let html = format!(
        "<div class=\"linked-omnigraph-head\"><div class=\"linked-omnigraph-title\"><strong>Brain + Omnigraph context</strong><div class=\"linked-omnigraph-badges\">{badges}</div></div><button type=\"button\" class=\"button compact\" data-action=\"session-omnigraph-open\">Open in Graph ↗</button></div>{warning_note}{preview}"
    );
    (true, html)
}

pub fn render_linked(session: &V) {
    sync_brain_followup(session);
    let brain = &session["brain"];
    dom::hidden("linked-context", !brain.is_object());
    if !brain.is_object() {
        dom::class("linked-context", "linked-context-review", false);
        dom::hidden("linked-omnigraph-summary", true);
        dom::clear("linked-omnigraph-summary");
        return;
    }
    let waiting = session["status"] == "awaiting_context";
    dom::class("linked-context", "linked-context-review", waiting);
    dom::text(
        "linked-context-title",
        if waiting {
            "Review context before running"
        } else {
            "Project memory"
        },
    );
    dom::text(
        "linked-context-state",
        if waiting {
            "Awaiting your review"
        } else {
            "Retrieved for each message"
        },
    );
    dom::text(
        "linked-context-target",
        &format!(
            "{} · {}",
            m::text(session, "project_path"),
            m::text(brain, "task_id")
        ),
    );
    dom::text(
        "linked-capsule",
        &if brain["capsule"].is_string() {
            m::text(brain, "capsule")
        } else {
            serde_json::to_string_pretty(&brain["capsule"]).unwrap_or_default()
        },
    );
    let (show_omni, omni_html) = render_linked_omnigraph_html(session);
    dom::hidden("linked-omnigraph-summary", !show_omni);
    if let Some(el) = dom::get("linked-omnigraph-summary") {
        el.set_inner_html(&omni_html);
    }
    dom::hidden("linked-context-form", brain["review"] == false);
    dom::hidden("linked-context-run", !waiting);
    dom::disabled(
        "linked-context-run",
        !waiting
            || !brain["context_id"].is_string()
            || m::read(|s| s.pending)
            || dom::value("linked-context-query").trim()
                != brain["query"].as_str().unwrap_or("").trim(),
    );
    if dom::value("linked-context-query").is_empty() {
        dom::set_value("linked-context-query", &m::text(brain, "query"));
    }
    // The server measures the capsule as the runtime caps it.
    let meter = [
        &brain["capsule_meter"],
        &session["capsule_delivery_meter"],
        &session["capsule_meter"],
    ]
    .into_iter()
    .find(|meter| meter.is_object())
    .cloned()
    .unwrap_or(V::Null);
    dom::hidden("capsule-meter", !meter.is_object());
    if !meter.is_object() {
        return;
    }
    if let Some(bar) = dom::get("capsule-bar") {
        bar.set_inner_html(&render_capsule_bar_html(&meter));
        let _ = bar.set_attribute("aria-label", &capsule_bar_label(&meter));
    }
    dom::text(
        "capsule-facts",
        &format!(
            "{} of {} characters",
            m::string(&meter["characters"]),
            m::string(&meter["limit"])
        ),
    );
    if let Some(legend) = dom::get("capsule-legend") {
        legend.set_inner_html(&render_capsule_legend_html(&meter));
    }
    dom::text(
        "capsule-repeats",
        &format!("{} characters repeat", m::string(&meter["repeats"])),
    );
}

/// A Brain or Memory Bank listing row: its title, its path and its state.
pub(crate) fn render_list_entry_inner_html(v: &V) -> String {
    let meta = [m::text(v, "status"), bytes(&v["bytes"])]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    format!(
        "<span class=\"memory-entry-title\">{}</span><span class=\"memory-entry-path\">{}</span><span class=\"memory-entry-meta\">{}</span>",
        crate::omnigraph::esc(&m::summary(v)),
        crate::omnigraph::esc(&m::text(v, "path")),
        crate::omnigraph::esc(&meta)
    )
}
