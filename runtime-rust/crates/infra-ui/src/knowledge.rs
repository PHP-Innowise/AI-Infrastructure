use crate::{api, dom, model as m, session};
use serde_json::{json, Value as V};
use std::collections::{BTreeMap, BTreeSet};
use wasm_bindgen::JsCast;
pub fn initialize() {
    m::set("memory-window", json!(30));
    for (days, label) in [(7, "7 d"), (30, "30 d"), (90, "90 d"), (0, "All")] {
        let b = dom::button(label, "memory-window", json!(days));
        let _ = b.set_attribute("role", "radio");
        let _ = b.set_attribute("aria-checked", if days == 30 { "true" } else { "false" });
        let _ = b.set_attribute("tabindex", if days == 30 { "0" } else { "-1" });
        dom::append("memory-use-window", &b);
    }

    for scope in ["memory", "brain"] {
        let panel = dom::el("details", "knowledge-tools", "");
        let _ = panel.set_attribute("id", &format!("{scope}-knowledge-panel"));
        dom::add(
            &panel,
            &dom::el(
                "summary",
                "",
                if scope == "brain" {
                    "Record tools & maintenance"
                } else {
                    "Search & maintenance"
                },
            ),
        );
        let runtime = dom::el("p", "knowledge-note", "Checking context runtime…");
        let _ = runtime.set_attribute("id", &format!("{scope}-knowledge-runtime"));
        dom::add(&panel, &runtime);
        if scope == "memory" {
            let f = dom::el("form", "knowledge-search", "");
            let _ = f.set_attribute("id", "memory-search-form");
            let q = dom::el("input", "", "");
            let _ = q.set_attribute("id", "memory-search-query");
            let _ = q.set_attribute("required", "");
            let _ = q.set_attribute("maxlength", "2000");
            let _ = q.set_attribute("placeholder", "Search indexed content");
            dom::add(&f, &q);
            let layer = dom::el("select", "", "");
            let _ = layer.set_attribute("id", "memory-search-layer");
            for value in ["", "procedural", "semantic", "episodic"] {
                let e = dom::el(
                    "option",
                    "",
                    if value.is_empty() {
                        "All layers"
                    } else {
                        value
                    },
                );
                let _ = e.set_attribute("value", value);
                dom::add(&layer, &e);
            }
            dom::add(&f, &layer);
            let b = dom::el("button", "button", "Search index");
            let _ = b.set_attribute("type", "submit");
            dom::add(&f, &b);
            let out = dom::el("section", "knowledge-output", "");
            let _ = out.set_attribute("id", "memory-search-results");
            dom::add(&f, &out);
            dom::add(&panel, &f);
        }
        let form = dom::el("form", "", "");
        let _ = form.set_attribute("id", &format!("{scope}-knowledge-form"));
        let select = dom::el("select", "", "");
        let _ = select.set_attribute("id", &format!("{scope}-knowledge-action"));
        for name in if scope == "brain" {
            vec![
                "start",
                "brain-create",
                "brain-update",
                "complete",
                "compact",
                "validate",
                "export",
            ]
        } else {
            vec![
                "status",
                "index",
                "validate",
                "bank-audit",
                "reindex-bank",
                "export",
                "bank-reverify",
                "bank-retire",
            ]
        } {
            let option = dom::el("option", "", &m::label(name));
            let _ = option.set_attribute("value", name);
            dom::add(&select, &option);
        }
        dom::add(&form, &select);
        for (suffix, class) in [
            ("note", "knowledge-note"),
            ("fields", "knowledge-form-grid"),
            ("error", "error-text"),
        ] {
            let e = dom::el(if suffix == "fields" { "div" } else { "p" }, class, "");
            let _ = e.set_attribute("id", &format!("{scope}-knowledge-{suffix}"));
            dom::add(&form, &e);
        }
        dom::add(
            &form,
            &dom::button("Use selected", &format!("{scope}-use-selected"), V::Null),
        );
        let b = dom::el("button", "button", "Run operation");
        let _ = b.set_attribute("type", "submit");
        let _ = b.set_attribute("id", &format!("{scope}-knowledge-submit"));
        dom::add(&form, &b);
        dom::add(&panel, &form);
        let out = dom::el("section", "knowledge-output", "");
        let _ = out.set_attribute("id", &format!("{scope}-knowledge-output"));
        dom::add(&panel, &out);
        dom::append(&format!("{scope}-tools"), &panel);
        fields(scope);
    }
}
fn definitions(action: &str) -> V {
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
fn fields(scope: &str) {
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
fn key(scope: &str) -> String {
    format!("knowledge-{scope}")
}
fn knowledge_current(scope: &str, project: &str, bank: &str, generation: u64) -> bool {
    dom::value(&format!("{scope}-project")) == project
        && dom::value(&format!("{scope}-bank")) == bank
        && m::scope_current(
            &format!("{scope}-knowledge-scope"),
            generation,
            project,
            bank,
        )
}
fn knowledge_ready(scope: &str) -> bool {
    let state = m::data(&key(scope));
    knowledge_current(
        scope,
        &m::text(&state, "project_id"),
        &m::text(&state, "bank_id"),
        state["view_generation"].as_u64().unwrap_or(0),
    )
}
fn clear_knowledge_detail(scope: &str) {
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
fn clear_knowledge_summary(scope: &str, clear_banks: bool) {
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
    clear_knowledge_summary(scope, true);
    for channel in ["list", "file", "meta"] {
        m::next(&format!("{scope}-{channel}"));
    }
    m::take(&key(scope));
    m::take(&format!("{scope}-meta"));
    clear_knowledge_detail(scope);
    dom::clear(&format!("{scope}-files"));
    dom::clear(&format!("{scope}-knowledge-output"));
    if scope == "memory" {
        dom::clear("memory-search-results");
    }
    tool_controls(scope);
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
fn resolve_initial_selection(
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

fn sync_list_selection(scope: &str, selected_path: &str) {
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

fn category(v: &V) -> String {
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
fn render_list(scope: &str) {
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
    dom::clear(&format!("{scope}-files"));
    let mut count = 0;
    for v in &entries {
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
        if !text.to_lowercase().contains(&query) || !cat.is_empty() && category(v) != cat {
            continue;
        }
        count += 1;
        let p = m::text(v, "path");
        let b = dom::button("", "knowledge-file", json!({"scope":scope,"path":p}));
        b.set_inner_html(&render_list_entry_inner_html(v));
        b.set_class_name("memory-entry");
        let _ = b.set_attribute("data-path", &p);
        if state["selected_path"] == p {
            let _ = b.set_attribute("aria-current", "true");
        }
        dom::append(&format!("{scope}-files"), &b);
    }
    dom::text(
        &format!("{scope}-count"),
        &format!(
            "{count} of {} records{}",
            entries.len(),
            if m::yes(&state["truncated"]) {
                " · listing limited"
            } else {
                ""
            }
        ),
    );
}
async fn metadata(scope: &str) {
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
fn tool_controls(scope: &str) {
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
async fn select(scope: &str, path: &str) {
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
async fn operation(scope: &str, action: &str, mut fields: V) {
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
async fn submit(scope: &str) {
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
fn selected(scope: &str) {
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
fn brain_followup_hint(review: bool) -> &'static str {
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
fn sync_brain_followup(session: &V) {
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
const CAPSULE_KINDS: [(&str, &str); 3] = [
    ("brain", "Project Brain"),
    ("bank", "Memory bank"),
    ("rules", "Rules & docs"),
];
/// Each memory kind's characters and share of the capsule bar, which stands
/// for the runtime's cap.
pub(crate) fn capsule_segments(meter: &V) -> Vec<(&'static str, &'static str, u64, f64)> {
    let limit = meter["limit"]
        .as_u64()
        .filter(|limit| *limit > 0)
        .unwrap_or(8000) as f64;
    CAPSULE_KINDS
        .iter()
        .map(|(kind, label)| {
            let characters = meter["kinds"][*kind].as_u64().unwrap_or(0);
            (
                *kind,
                *label,
                characters,
                (characters as f64 / limit * 100.).min(100.),
            )
        })
        .collect()
}
pub(crate) fn render_capsule_bar_html(meter: &V) -> String {
    capsule_segments(meter)
        .into_iter()
        .map(|(kind, label, characters, share)| {
            if characters == 0 {
                format!("<span class=\"capsule-part\" data-kind=\"{kind}\" hidden></span>")
            } else {
                format!(
                    "<span class=\"capsule-part\" data-kind=\"{kind}\" style=\"width:{share:.2}%\" title=\"{label}: {characters} characters\"></span>"
                )
            }
        })
        .collect()
}
fn capsule_bar_label(meter: &V) -> String {
    let parts = capsule_segments(meter)
        .into_iter()
        .filter(|(_, _, characters, _)| *characters > 0)
        .map(|(_, label, characters, _)| format!("{label} {characters}"))
        .collect::<Vec<_>>();
    format!(
        "{} of {} characters",
        if parts.is_empty() {
            "Nothing".to_string()
        } else {
            parts.join(", ")
        },
        m::string(&meter["limit"])
    )
}
pub(crate) fn render_capsule_legend_html(meter: &V) -> String {
    let mut html = String::new();
    for (kind, label, characters, _) in capsule_segments(meter) {
        if characters == 0 {
            continue;
        }
        let items = meter["items"][kind].as_u64().unwrap_or(0);
        html.push_str(&format!(
            "<span><i class=\"capsule-swatch\" data-kind=\"{kind}\" aria-hidden=\"true\"></i>{label} · {characters} characters · {items} item{}</span>",
            if items == 1 { "" } else { "s" }
        ));
    }
    let dropped = meter["dropped"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(layer, count)| {
            count
                .as_u64()
                .filter(|count| *count > 0)
                .map(|count| format!("{count} {}", crate::omnigraph::esc(layer)))
        })
        .collect::<Vec<_>>();
    if !dropped.is_empty() {
        html.push_str(&format!(
            "<span>Dropped to fit: {}</span>",
            dropped.join(", ")
        ));
    }
    html
}
/// A day's bar of retrievals: those that delivered a Memory Bank chunk at the
/// foot, the rest above them, as shares of the busiest day.
pub(crate) fn history_day_heights(day: &V, most: u64) -> (f64, f64) {
    let retrievals = day["retrievals"].as_u64().unwrap_or(0);
    let with_chunk = day["with_chunk"].as_u64().unwrap_or(0).min(retrievals);
    let most = most.max(1) as f64;
    (
        (retrievals - with_chunk) as f64 / most * 100.,
        with_chunk as f64 / most * 100.,
    )
}
const USAGE_KINDS: [(&str, &str); 7] = [
    ("brain", "Project Brain"),
    ("bank", "Memory bank"),
    ("rules", "Rules & docs"),
    ("omnigraph", "Omnigraph"),
    ("rest", "Conversation and tools"),
    ("added", "Added this turn"),
    ("free", "Free"),
];
/// The tokens a turn's bar shows for one kind: a compacted turn shows its
/// whole fill as added, and a turn without a measured rest no memory split.
fn turn_part(turn: &V, kind: &str) -> f64 {
    if !m::array(&turn["compactions"]).is_empty() {
        return if kind == "added" {
            m::number(&turn["end"]).unwrap_or(0.)
        } else {
            0.
        };
    }
    if turn["parts"]["rest"].is_null() && ["brain", "bank", "rules", "omnigraph"].contains(&kind) {
        return 0.;
    }
    m::number(&turn["parts"][kind]).unwrap_or(0.)
}
/// A turn's button in the strip: a small track of what filled its window and
/// its number.
pub(crate) fn render_usage_turn_button_html(turn: &V) -> String {
    let window = m::number(&turn["window"]).unwrap_or(0.);
    let mut parts = String::new();
    if window > 0. {
        for kind in ["brain", "bank", "rules", "omnigraph", "rest", "added"] {
            let value = turn_part(turn, kind);
            if value > 0. {
                parts.push_str(&format!(
                    "<span class=\"context-turn-part\" data-kind=\"{kind}\" style=\"height:{:.2}%\"></span>",
                    (value / window * 100.).clamp(0., 100.)
                ));
            }
        }
    }
    format!(
        "<span class=\"context-turn-track\" aria-hidden=\"true\">{parts}</span><span class=\"context-turn-label\">{}</span>",
        crate::omnigraph::esc(&m::string(&turn["ordinal"]))
    )
}
/// The legend rows of the selected turn, each with its swatch.
pub(crate) fn render_usage_legend_html(turn: &V) -> String {
    let mut rows = String::new();
    for (kind, label) in USAGE_KINDS {
        let value = m::number(&turn["parts"][kind])
            .map(|n| {
                format!(
                    "{}{n:.0}",
                    if ["brain", "bank", "rules", "omnigraph", "rest"].contains(&kind) {
                        "≈ "
                    } else {
                        ""
                    }
                )
            })
            .unwrap_or_else(|| "—".into());
        rows.push_str(&format!(
            "<tr><th scope=\"row\"><i class=\"context-swatch\" data-kind=\"{kind}\" aria-hidden=\"true\"></i>{label}</th><td class=\"context-number\">{value}</td></tr>"
        ));
    }
    format!("<tbody>{rows}</tbody>")
}
const FEEDBACK_VOTES: [(&str, &str); 3] = [
    ("helped", "Helped"),
    ("stale", "Stale"),
    ("harmful", "Harmful"),
];
/// One retrieval's delivered chunks and cuts, with the verdict a person
/// recorded on each delivered chunk and buttons to change it. A verdict is
/// telemetry: nothing ranks, admits or packs memory by it.
pub(crate) fn render_retrieval_trace_html(row: &V, feedback: &V, supported: bool) -> String {
    let esc = crate::omnigraph::esc;
    let manifest = row["manifest_id"].as_str().unwrap_or("");
    let mut html = format!(
        "<p class=\"knowledge-note\">{} Project Brain · {} Memory Bank · {} rules and docs</p><ul class=\"retrieval-trace-items\">",
        m::string(&row["brain"]),
        m::string(&row["bank"]),
        m::string(&row["rules"])
    );
    for chunk in m::array(&row["chunks"]) {
        let chunk = chunk.as_str().unwrap_or("");
        if chunk.is_empty() {
            continue;
        }
        let vote = feedback[manifest][chunk]["vote"].as_str().unwrap_or("");
        html.push_str(&format!("<li><code>{}</code>", esc(chunk)));
        if supported && !manifest.is_empty() {
            html.push_str("<span class=\"retrieval-votes\" role=\"group\" aria-label=\"Was this chunk right?\">");
            for (choice, label) in FEEDBACK_VOTES {
                let pressed = vote == choice;
                let payload = json!({
                    "manifest_id": manifest,
                    "chunk_id": chunk,
                    // A second press takes the verdict back.
                    "vote": if pressed { "clear" } else { choice },
                });
                html.push_str(&format!(
                    "<button type=\"button\" class=\"button compact\" data-action=\"memory-feedback\" data-payload=\"{}\" aria-pressed=\"{pressed}\">{label}</button>",
                    esc(&payload.to_string())
                ));
            }
            html.push_str("</span>");
        }
        html.push_str("</li>");
    }
    for cut in m::array(&row["cuts"]) {
        let (id, reason) = (cut[0].as_str().unwrap_or(""), cut[1].as_str().unwrap_or(""));
        if !id.is_empty() {
            html.push_str(&format!(
                "<li class=\"retrieval-cut\">{} — cut: {}</li>",
                esc(id),
                esc(reason)
            ));
        }
    }
    html.push_str("</ul>");
    html
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
/// Where a proposal stands in the Memory use queue: `waiting` for a person,
/// an automatic one that `stalled`, or `auto` once applied without review.
fn promotion_group(item: &V) -> Option<&'static str> {
    let automatic = item["mode"] == "automatic";
    match item["status"].as_str() {
        Some("proposed" | "reviewed") => Some(if automatic { "stalled" } else { "waiting" }),
        Some("applied") if automatic => Some("auto"),
        _ => None,
    }
}
fn promotion_id(item: &V) -> Option<&str> {
    item["id"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
}
/// How many automatic promotions the queue lists, newest first.
const PROMOTION_AUTO_SHOWN: usize = 20;
/// The promotion queue on Memory use. Proposals, their source records and
/// the chunks they made appear by ID only, so no Project Brain title or path
/// reaches the page; a person reads a source in Project Brain before
/// approving. Approve and Reject sign a human proposal with the reviewer's
/// name, and Apply publishes an approved one. An automatic proposal takes no
/// signature, so one that stalled before its review waits for the next
/// automatic run. `since` keeps the automatic promotions applied at or after
/// that UTC moment, and an empty `since` keeps them all.
pub(crate) fn render_promotion_queue_html(
    items: &[V],
    since: &str,
    reviewer: &str,
    busy: bool,
    status: &str,
) -> String {
    let esc = crate::omnigraph::esc;
    let moment = |item: &V, key: &str| item[key].as_str().unwrap_or("").to_owned();
    // RFC 3339 moments compare as text to the second.
    let second = |at: &str| at.get(..19).unwrap_or(at).to_owned();
    let (mut waiting, mut stalled, mut auto) = (vec![], vec![], vec![]);
    for item in items {
        match promotion_group(item) {
            Some("waiting") => waiting.push(item),
            Some("stalled") => stalled.push(item),
            Some("auto")
                if since.is_empty() || second(&moment(item, "applied_at")) >= second(since) =>
            {
                auto.push(item)
            }
            _ => {}
        }
    }
    waiting.sort_by_key(|item| moment(item, "created_at"));
    stalled.sort_by_key(|item| moment(item, "created_at"));
    auto.sort_by_key(|item| std::cmp::Reverse(moment(item, "applied_at")));
    let mut html = format!(
        "<section class=\"memory-section promotion-queue\" aria-labelledby=\"promotion-queue-title\"><div class=\"memory-section-head\"><h3 id=\"promotion-queue-title\">Promotion queue</h3><span class=\"memory-range\">{} waiting · {} stalled · {} auto-promoted</span></div><p class=\"knowledge-note\">Proposals appear by ID only. Read a source record in Project Brain before you approve; Apply writes the approved chunk to the Memory Bank.</p>",
        waiting.len(),
        stalled.len(),
        auto.len()
    );
    if waiting
        .iter()
        .any(|item| item["status"] == "proposed" && promotion_id(item).is_some())
    {
        html.push_str(&format!(
            "<label class=\"field promotion-reviewer\">Reviewer<input id=\"memory-promotion-reviewer\" type=\"text\" maxlength=\"200\" autocomplete=\"name\" spellcheck=\"false\" value=\"{}\"></label>",
            esc(reviewer)
        ));
    }
    if !status.is_empty() {
        html.push_str(&format!(
            "<p class=\"knowledge-note promotion-status\" role=\"status\">{}</p>",
            esc(status)
        ));
    }
    for (group, title, list) in [
        ("waiting", "Waiting for review", &waiting),
        ("stalled", "Stalled automatic promotions", &stalled),
        ("auto", "Auto-promoted in this window", &auto),
    ] {
        if list.is_empty() {
            continue;
        }
        html.push_str(&format!(
            "<h4 class=\"promotion-group-title\">{title} · {}</h4><ul class=\"promotion-list\">",
            list.len()
        ));
        let shown = if group == "auto" {
            PROMOTION_AUTO_SHOWN
        } else {
            list.len()
        };
        for item in list.iter().take(shown) {
            html.push_str(&render_promotion_item_html(item, group, busy));
        }
        html.push_str("</ul>");
        if list.len() > shown {
            html.push_str(&format!(
                "<p class=\"knowledge-note\">Showing the {shown} newest of {}.</p>",
                list.len()
            ));
        }
    }
    if waiting.is_empty() && stalled.is_empty() && auto.is_empty() {
        html.push_str("<p class=\"knowledge-note\">Nothing waits for review, no automatic promotion stalled, and none was applied automatically in this window.</p>");
    }
    html.push_str("</section>");
    html
}
fn render_promotion_item_html(item: &V, group: &str, busy: bool) -> String {
    let esc = crate::omnigraph::esc;
    let id = promotion_id(item);
    let status = item["status"].as_str().unwrap_or("");
    let sources = item["source_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(V::as_str)
        .map(str::trim)
        .filter(|source| !source.is_empty())
        .collect::<Vec<_>>();
    let mut html = format!(
        "<li class=\"promotion-item\" data-group=\"{group}\" data-status=\"{}\"><div class=\"promotion-head\">",
        esc(status)
    );
    match id {
        Some(id) => html.push_str(&format!("<code class=\"promotion-id\">{}</code>", esc(id))),
        None => html.push_str("<span class=\"promotion-id\">Proposal without a readable ID</span>"),
    }
    html.push_str(&format!(
        "<span class=\"promotion-state\">{}</span></div>",
        match status {
            "proposed" => "Proposed",
            "reviewed" => "Approved, not applied",
            _ => "Applied",
        }
    ));
    let mut facts = vec![if sources.is_empty() {
        "No source record ID".to_owned()
    } else {
        format!(
            "{} {}",
            if sources.len() == 1 {
                "Source"
            } else {
                "Sources"
            },
            sources
                .iter()
                .map(|source| format!("<code>{}</code>", esc(source)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }];
    for (key, label) in [
        ("created_at", "proposed"),
        ("reviewed_at", "reviewed"),
        ("applied_at", "applied"),
    ] {
        if let Some(at) = item[key].as_str().filter(|at| !at.is_empty()) {
            facts.push(format!("{label} {}", esc(at.get(..10).unwrap_or(at))));
        }
    }
    if let Some(reason) = item["reason"].as_str().filter(|reason| !reason.is_empty()) {
        facts.push(format!("outcome <code>{}</code>", esc(reason)));
    }
    if let Some(memory) = item["memory_id"]
        .as_str()
        .filter(|memory| !memory.is_empty())
    {
        facts.push(format!("chunk <code>{}</code>", esc(memory)));
    }
    html.push_str(&format!(
        "<p class=\"promotion-facts\">{}</p><div class=\"promotion-actions\">",
        facts.join(" · ")
    ));
    let button = |label: &str, action: &str, payload: V| {
        format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"{action}\" data-payload=\"{}\"{}>{label}</button>",
            esc(&payload.to_string()),
            if id.is_none() || busy {
                " disabled"
            } else {
                ""
            }
        )
    };
    let promotion = id.unwrap_or("");
    match (group, status) {
        ("waiting", "proposed") => {
            for (label, reject) in [("Approve", false), ("Reject", true)] {
                html.push_str(&button(
                    label,
                    "memory-promotion-review",
                    json!({"promotion_id": promotion, "reject": reject}),
                ));
            }
        }
        ("waiting" | "stalled", "reviewed") => html.push_str(&button(
            "Apply",
            "memory-promotion-apply",
            json!({"promotion_id": promotion}),
        )),
        ("stalled", "proposed") => html.push_str("<span class=\"promotion-note\">An automatic proposal takes no reviewer; the next automatic promotion retries it.</span>"),
        _ => {}
    }
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"memory-promotion-brain\" data-payload=\"{}\">Review in Project Brain</button>",
        esc(&json!({"record_id": sources.first().copied().unwrap_or("")}).to_string())
    ));
    if id.is_none() && group != "auto" {
        html.push_str("<span class=\"promotion-note\">Without a readable ID this proposal can only be reviewed in Project Brain.</span>");
    }
    html.push_str("</div></li>");
    html
}
fn memory_model(v: &V, window: u64) -> V {
    let now = js_sys::Date::now();
    let within = |date: &V| {
        date.as_str()
            .filter(|s| !s.is_empty())
            .map(|s| {
                let timestamp = if s.len() == 10 {
                    js_sys::Date::parse(&format!("{s}T00:00:00"))
                } else {
                    js_sys::Date::parse(s)
                };
                timestamp.is_finite()
                    && (window == 0 || timestamp >= now - window as f64 * 86400000.)
            })
            .unwrap_or(false)
    };
    let chunks = m::array(&v["chunks"]["items"]);
    let records = m::array(&v["brain"]["items"]);
    let promotions = m::array(&v["promotions"]["items"]);
    let rows = m::array(&v["retrievals"]["items"]);
    let active = chunks
        .iter()
        .filter(|c| c["status"] == "active")
        .collect::<Vec<_>>();
    let retired = chunks
        .iter()
        .filter(|c| matches!(c["status"].as_str(), Some("superseded" | "archived")))
        .collect::<Vec<_>>();
    let review = |c: &&V| {
        day_distance(
            v["today"].as_str().unwrap_or(""),
            c["review_after"].as_str().unwrap_or(""),
        )
    };
    let mut selected = BTreeMap::<String, u64>::new();
    let mut tasksets = BTreeMap::<String, BTreeSet<String>>::new();
    let mut cutin = BTreeMap::<String, u64>::new();
    let mut cuts = BTreeMap::<String, u64>::new();
    let mut routes = BTreeMap::<String, u64>::new();
    let mut kinds = BTreeMap::from([("brain", 0u64), ("bank", 0), ("rules", 0)]);
    for row in &rows {
        let ids = m::array(&row["chunks"])
            .iter()
            .map(m::string)
            .collect::<BTreeSet<_>>();
        for id in ids {
            *selected.entry(id.clone()).or_default() += 1;
            tasksets.entry(id).or_default().insert(m::text(row, "task"));
        }
        for cut in m::array(&row["cuts"]) {
            *cutin.entry(m::string(&cut[0])).or_default() += 1;
            *cuts.entry(m::string(&cut[1])).or_default() += 1;
        }
        *routes.entry(m::text(row, "route")).or_default() += 1;
        for key in ["brain", "bank", "rules"] {
            *kinds.entry(key).or_default() += row[key].as_u64().unwrap_or(0);
        }
    }
    let pending = |mode: &str| {
        promotions
            .iter()
            .filter(|p| {
                p["mode"] == mode && matches!(p["status"].as_str(), Some("proposed" | "reviewed"))
            })
            .count()
    };
    let applied = promotions
        .iter()
        .filter(|p| p["status"] == "applied" && within(&p["applied_at"]))
        .collect::<Vec<_>>();
    let past = active
        .iter()
        .filter(|c| review(c).is_some_and(|d| d < 0))
        .count();
    let changed = active
        .iter()
        .filter(|c| c["sources_changed"] == true)
        .count();
    let stalled = if v["promotions"].is_null() {
        0
    } else {
        pending("automatic")
    };
    let kept = m::array(&v["history"]["days"]);
    let days = kept
        .iter()
        .filter(|d| within(&d["day"]))
        .cloned()
        .collect::<Vec<_>>();
    let mut history_chunks = BTreeMap::<String, u64>::new();
    for day in &kept {
        for (id, count) in day["chunks"].as_object().into_iter().flatten() {
            *history_chunks.entry(id.clone()).or_default() += count.as_u64().unwrap_or(0);
        }
    }
    let since = rows.first().map(|r| m::text(r, "at")).unwrap_or_default();
    let health = m::array(&v["health"]["items"])
        .into_iter()
        .filter(|h| !since.is_empty() && h["at"].as_str().unwrap_or("") >= since.as_str())
        .collect::<Vec<_>>();
    json!({"window_days":window,"count":past+changed+stalled,"selected_in":selected,"cut_in":cutin,
 "history":{"days":days,"kept":kept,"since":kept.first().map(|d|d["day"].clone()),"retrievals":days.iter().filter_map(|d|d["retrievals"].as_u64()).sum::<u64>(),"with_chunk":days.iter().filter_map(|d|d["with_chunk"].as_u64()).sum::<u64>(),"chunks":history_chunks},
 "brain":if v["brain"].is_null(){V::Null}else{json!({"total":records.len(),"open":records.iter().filter(|r|r["open"]==true).count(),"archived":records.iter().filter(|r|r["archived"]==true).count(),"private":records.iter().filter(|r|r["private"]==true).count(),"resolved":records.iter().filter(|r|within(&r["resolved_at"])).count(),"not_promoted":records.iter().filter(|r|within(&r["resolved_at"])&&r["promotable"]==true&&r["promoted"]!=true).count(),"truncated":v["brain"]["truncated"]})},
 "promotion":if v["promotions"].is_null(){V::Null}else{json!({"proposed":promotions.iter().filter(|p|within(&p["created_at"])).count(),"applied":applied.len(),"auto":applied.iter().filter(|p|p["mode"]=="automatic").count(),"human":applied.iter().filter(|p|p["mode"]=="human").count(),"waiting":pending("human"),"stalled":stalled,"rejected":promotions.iter().filter(|p|p["status"]=="rejected"&&within(&p["created_at"])).count()})},
 "bank":if v["chunks"].is_null(){V::Null}else{json!({"files":chunks.len(),"active":active.len(),"person":active.iter().filter(|c|c["auto"]!=true).count(),"auto":active.iter().filter(|c|c["auto"]==true).count(),"reattested":active.iter().filter(|c|c["auto"]==true&&c["last_verified"].as_str().zip(c["created"].as_str()).map(|(a,b)|a>b).unwrap_or(false)).count(),"drafts":chunks.iter().filter(|c|c["status"]=="needs-review").count(),"superseded":chunks.iter().filter(|c|c["status"]=="superseded").count(),"archived":chunks.iter().filter(|c|c["status"]=="archived").count(),"retired":retired.len(),"bytes":v["chunks"]["bytes"],"created":chunks.iter().filter(|c|within(&c["created"])).count(),"retired_in_window":retired.iter().filter(|c|within(&c["valid_to"])).count(),"past_review":past,"changed":changed,"due":active.iter().filter(|c|review(c).is_some_and(|d|(0..=30).contains(&d))).count(),"truncated":v["chunks"]["truncated"]})},
 "selected":{"retrievals":rows.len(),"with_chunk":rows.iter().filter(|r|!m::array(&r["chunks"]).is_empty()).count(),"selections":rows.iter().map(|r|m::array(&r["chunks"]).len()).sum::<usize>(),"distinct":selected.len(),"reused":tasksets.values().filter(|s|s.len()>1).count(),"cuts":cuts,"cut_total":cuts.values().sum::<u64>(),"routes":routes,"kinds":kinds,"since":since,"dropped":health.iter().filter(|h|h["dropped"]==true).count(),"measured":health.iter().filter(|h|!h["dropped"].is_null()).count(),"merged":v["retrievals"]["merged"],"found":v["retrievals"]["found"],"limit":v["retrievals"]["limit"]}})
}
pub async fn load_memory_use() {
    let p = dom::value("memory-use-project");
    let bank = dom::value("memory-use-bank");
    let previous = m::data("memory-use");
    if previous["project_id"] != p || previous["bank_id"] != bank {
        leave_memory();
        for key in [
            "memory-use-check",
            "memory-pinned",
            "memory-selected-chunk",
            "memory-filters",
            "memory-active",
            "memory-promotion-status",
        ] {
            m::take(key);
        }
    }
    if p.is_empty() {
        return;
    }
    let path = format!(
        "/api/projects/{}/memory-use{}",
        dom::enc(&p),
        dom::query(&[("bank", bank)])
    );
    match api::scoped("memory-use", &path).await {
        Ok((e, v)) if m::current("memory-use", e) && dom::value("memory-use-project") == p => {
            dom::options(
                "memory-use-bank",
                &m::array(&v["banks"]),
                m::summary,
                Some(&m::text(&v, "bank_id")),
            );
            m::set("memory-use", v);
            render_memory();
        }
        Ok(_) => {}
        Err(e) if !e.aborted => dom::error("memory-use-error", &e.message),
        Err(_) => {}
    }
}
fn metric_label(key: &str) -> String {
    let label = m::label(key);
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => label,
    }
}

fn memory_stage_metrics(stage: &str, stage_model: &V) -> Option<Vec<(String, String, bool)>> {
    let map = stage_model.as_object()?;
    let order: &[&str] = match stage {
        "brain" => &[
            "total",
            "open",
            "resolved",
            "not_promoted",
            "archived",
            "private",
            "truncated",
        ],
        "promotion" => &[
            "proposed", "applied", "auto", "human", "waiting", "stalled", "rejected",
        ],
        "bank" => &[
            "active",
            "files",
            "person",
            "auto",
            "reattested",
            "created",
            "drafts",
            "due",
            "past_review",
            "changed",
            "retired",
            "retired_in_window",
            "superseded",
            "archived",
            "bytes",
            "truncated",
        ],
        "selected" => &[
            "retrievals",
            "with_chunk",
            "selections",
            "distinct",
            "reused",
            "cut_total",
            "dropped",
            "measured",
        ],
        _ => &[],
    };
    let mut rows = Vec::new();
    let mut push = |k: &str, value: &V| {
        if !value.is_number() && !value.is_boolean() {
            return;
        }
        let is_zero = value.as_u64() == Some(0)
            || value.as_i64() == Some(0)
            || value.as_f64() == Some(0.0)
            || value.as_bool() == Some(false);
        let formatted = match value.as_bool() {
            Some(true) => "Yes".to_string(),
            Some(false) => "No".to_string(),
            None => m::string(value),
        };
        rows.push((metric_label(k), formatted, is_zero));
    };
    for &k in order {
        if let Some(value) = map.get(k) {
            push(k, value);
        }
    }
    for (k, value) in map {
        if !order.contains(&k.as_str()) {
            push(k, value);
        }
    }
    Some(rows)
}

fn set_text_if_changed(id: &str, value: &str) {
    if dom::get(id).and_then(|e| e.text_content()).as_deref() != Some(value) {
        dom::text(id, value);
    }
}

fn memory_chunk_matches_ui_filter(chunk: &V, today: &str, mode: &str) -> bool {
    !(mode == "past"
        && !chunk["review_after"]
            .as_str()
            .map(|s| s < today)
            .unwrap_or(false)
        || mode == "changed" && chunk["sources_changed"] != true
        || mode == "active" && chunk["status"] != "active"
        || mode == "retired"
            && !["superseded", "archived"].contains(&chunk["status"].as_str().unwrap_or("")))
}

fn build_memory_chunk_items(chunks: &[V], today: &str, mode: &str) -> Vec<V> {
    let mut seen = BTreeMap::<String, usize>::new();
    let mut items = Vec::new();
    for (index, c) in chunks.iter().enumerate() {
        if !memory_chunk_matches_ui_filter(c, today, mode) {
            continue;
        }
        let id = m::text(c, "id");
        let path = m::text(c, "path");
        let base = if !id.is_empty() {
            format!("chunk:{id}")
        } else if !path.is_empty() {
            format!("chunk:{path}")
        } else {
            format!("chunk:idx:{index}")
        };
        let count = seen.entry(base.clone()).or_insert(0);
        let key = if *count == 0 {
            base
        } else {
            format!("{base}#{}", *count)
        };
        *count += 1;
        items.push(json!({
            "key": key,
            "chunk": c,
        }));
    }
    items
}

fn build_memory_trace_items(rows: &[V], feedback: &V, supported: bool) -> Vec<V> {
    let mut seen = BTreeMap::<String, usize>::new();
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let manifest = m::text(row, "manifest_id");
            let at = m::text(row, "at");
            let base = if !manifest.is_empty() {
                format!("trace:{manifest}")
            } else if !at.is_empty() {
                format!("trace:{at}")
            } else {
                format!("trace:idx:{index}")
            };
            let count = seen.entry(base.clone()).or_insert(0);
            let key = if *count == 0 {
                base
            } else {
                format!("{base}#{}", *count)
            };
            *count += 1;
            let row_feedback = if manifest.is_empty() {
                V::Null
            } else {
                feedback[&manifest].clone()
            };
            json!({
                "key": key,
                "row": row,
                "feedback": row_feedback,
                "supported": supported,
            })
        })
        .collect()
}

fn ensure_memory_layout() {
    if dom::get("memory-use-flow").is_some()
        && dom::get("memory-use-promotions").is_some()
        && dom::get("memory-use-check-slot").is_some()
        && dom::get("memory-ui-filter").is_some()
        && dom::get("memory-use-chunks").is_some()
        && dom::get("memory-use-strip").is_some()
        && dom::get("memory-use-horizon").is_some()
        && dom::get("memory-use-history").is_some()
        && dom::get("memory-table-chunk").is_some()
        && dom::get("memory-table-retrieval").is_some()
        && dom::get("memory-use-daily-history").is_some()
        && dom::get("memory-use-traces").is_some()
    {
        return;
    }
    dom::clear("memory-use-body");

    let flow = dom::el("ol", "memory-flow", "");
    let _ = flow.set_attribute("id", "memory-use-flow");
    dom::append("memory-use-body", &flow);

    let promotions = dom::el("div", "", "");
    let _ = promotions.set_attribute("id", "memory-use-promotions");
    dom::append("memory-use-body", &promotions);

    let check_slot = dom::el("div", "", "");
    let _ = check_slot.set_attribute("id", "memory-use-check-slot");
    dom::append("memory-use-body", &check_slot);

    let filter = dom::el("select", "", "");
    let _ = filter.set_attribute("id", "memory-ui-filter");
    for (id, label) in [
        ("all", "All chunks"),
        ("past", "Past review"),
        ("changed", "Changed sources"),
        ("active", "Active"),
        ("retired", "Retired"),
    ] {
        let o = dom::el("option", "", label);
        let _ = o.set_attribute("value", id);
        dom::add(&filter, &o);
    }
    dom::append("memory-use-body", &filter);

    let chunks = dom::el("div", "", "");
    let _ = chunks.set_attribute("id", "memory-use-chunks");
    dom::append("memory-use-body", &chunks);

    let strip = dom::el("section", "memory-section memory-strip-section", "");
    let _ = strip.set_attribute("id", "memory-use-strip");
    dom::append("memory-use-body", &strip);

    let horizon = dom::el("section", "memory-section memory-horizon-section", "");
    let _ = horizon.set_attribute("id", "memory-use-horizon");
    dom::append("memory-use-body", &horizon);

    let history = dom::el("section", "memory-history", "");
    let _ = history.set_attribute("id", "memory-use-history");
    dom::append("memory-use-body", &history);

    for kind in ["chunk", "retrieval"] {
        let details = dom::el("details", "fleet-report", "");
        let _ = details.set_attribute("id", &format!("memory-table-{kind}"));
        let summary = dom::el("summary", "", &format!("{} table (0)", m::label(kind)));
        let _ = summary.set_attribute("id", &format!("memory-table-{kind}-summary"));
        dom::add(&details, &summary);
        let host = dom::el("div", "", "");
        let _ = host.set_attribute("id", &format!("memory-table-{kind}-host"));
        dom::add(&details, &host);
        dom::append("memory-use-body", &details);
    }

    let daily = dom::el("details", "setup-file", "");
    let _ = daily.set_attribute("id", "memory-use-daily-history");
    dom::add(&daily, &dom::el("summary", "", "Daily retrieval history"));
    let pre = dom::el("pre", "knowledge-result", "");
    let _ = pre.set_attribute("id", "memory-use-daily-history-pre");
    dom::add(&daily, &pre);
    dom::append("memory-use-body", &daily);

    let traces = dom::el("div", "", "");
    let _ = traces.set_attribute("id", "memory-use-traces");
    dom::append("memory-use-body", &traces);
}

fn sync_memory_promotions(v: &V, window: u64) {
    let Some(host) = dom::get("memory-use-promotions") else {
        return;
    };
    if !v["promotions"].is_object() {
        dom::hide(&host, true);
        if host.first_element_child().is_some() {
            host.set_text_content(None);
        }
        let _ = host.remove_attribute("data-base-signature");
        let _ = host.remove_attribute("data-full-signature");
        return;
    }
    dom::hide(&host, false);
    let since = if window == 0 {
        String::new()
    } else {
        js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
            js_sys::Date::now() - window as f64 * 86_400_000.,
        ))
        .to_iso_string()
        .as_string()
        .unwrap_or_default()
    };
    let items = m::array(&v["promotions"]["items"]);
    let reviewer = promotion_reviewer();
    let busy = m::yes(&m::data("memory-promotion-busy"));
    let status = m::data("memory-promotion-status")
        .as_str()
        .unwrap_or("")
        .to_owned();
    let base_sig = json!([items, since]).to_string();
    let full_sig = json!([items, since, reviewer, busy, status]).to_string();
    if host.get_attribute("data-full-signature").as_deref() == Some(full_sig.as_str()) {
        return;
    }
    if host.get_attribute("data-base-signature").as_deref() == Some(base_sig.as_str())
        && host
            .query_selector(".promotion-queue")
            .ok()
            .flatten()
            .is_some()
    {
        let _ = host.set_attribute("data-full-signature", &full_sig);
        if let Ok(buttons) = host.query_selector_all(
            "button[data-action=\"memory-promotion-review\"],button[data-action=\"memory-promotion-apply\"]",
        ) {
            for i in 0..buttons.length() {
                if let Some(btn) = buttons.item(i).and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                {
                    let has_id = btn
                        .get_attribute("data-payload")
                        .and_then(|s| serde_json::from_str::<V>(&s).ok())
                        .and_then(|p| p["promotion_id"].as_str().map(|s| !s.is_empty()))
                        .unwrap_or(false);
                    if busy || !has_id {
                        let _ = btn.set_attribute("disabled", "");
                    } else {
                        let _ = btn.remove_attribute("disabled");
                    }
                }
            }
        }
        if let Ok(Some(queue)) = host.query_selector(".promotion-queue") {
            let existing = queue.query_selector(".promotion-status").ok().flatten();
            if status.is_empty() {
                if let Some(p) = existing {
                    p.remove();
                }
            } else if let Some(p) = existing {
                if p.text_content().as_deref() != Some(status.as_str()) {
                    p.set_text_content(Some(&status));
                }
            } else {
                let p = dom::el("p", "knowledge-note promotion-status", &status);
                let _ = p.set_attribute("role", "status");
                let anchor = queue
                    .query_selector(".promotion-group-title")
                    .ok()
                    .flatten();
                let _ = queue.insert_before(&p, anchor.as_ref().map(|e| e.as_ref()));
            }
        }
        if let Some(input) = dom::get("memory-promotion-reviewer") {
            let active = dom::document().active_element().as_ref() == Some(&input);
            if !active && dom::value_of(&input) != reviewer {
                dom::set_element_value(&input, &reviewer);
            }
        }
        return;
    }

    let focused_reviewer = dom::get("memory-promotion-reviewer")
        .filter(|input| dom::document().active_element().as_ref() == Some(input))
        .and_then(|input| input.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| {
            (
                input.value(),
                input.selection_start().ok().flatten(),
                input.selection_end().ok().flatten(),
            )
        });
    let _ = host.set_attribute("data-base-signature", &base_sig);
    let _ = host.set_attribute("data-full-signature", &full_sig);
    let html = render_promotion_queue_html(&items, &since, &reviewer, busy, &status);
    host.set_inner_html(&html);
    if let Some((val, start, end)) = focused_reviewer {
        if let Some(input) = dom::get("memory-promotion-reviewer")
            .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
        {
            input.set_value(&val);
            let _ = input.focus();
            if let (Some(s), Some(e)) = (start, end) {
                let _ = input.set_selection_range(s, e);
            }
        }
    }
}

fn render_memory() {
    let v = m::data("memory-use");
    let window = m::data("memory-window").as_u64().unwrap_or(30);
    let model = memory_model(&v, window);
    m::set("memory-use-model", model.clone());
    ensure_memory_layout();

    let stage_items = [
        ("brain", "Project Brain"),
        ("promotion", "Promotion"),
        ("bank", "Memory bank"),
        ("selected", "Selected by retrieval"),
    ]
    .into_iter()
    .map(|(stage, title)| {
        let metrics = memory_stage_metrics(stage, &model[stage])
            .filter(|rows| !rows.is_empty())
            .map(|rows| {
                rows.into_iter()
                    .map(|(label, value, is_zero)| json!([label, value, is_zero]))
                    .collect::<Vec<_>>()
            });
        json!({
            "stage": stage,
            "title": title,
            "metrics": metrics,
        })
    })
    .collect::<Vec<_>>();
    dom::keyed_render(
        "memory-use-flow",
        &stage_items,
        |item| m::text(item, "stage"),
        |item| {
            let stage = m::text(item, "stage");
            let title = m::text(item, "title");
            let card = dom::el("li", "memory-stage", "");
            let _ = card.set_attribute("data-stage", &stage);
            dom::add(&card, &dom::el("h3", "", &title));
            if let Some(rows) = item["metrics"].as_array() {
                let list = dom::el("dl", "memory-metrics", "");
                for row in rows {
                    let label = m::string(&row[0]);
                    let value = m::string(&row[1]);
                    let is_zero = m::yes(&row[2]);
                    dom::add(&list, &dom::el("dt", "", &label));
                    let dd = dom::el("dd", "", &value);
                    if is_zero {
                        let _ = dd.set_attribute("data-zero", "true");
                    }
                    dom::add(&list, &dd);
                }
                dom::add(&card, &list);
            } else {
                dom::add(
                    &card,
                    &dom::el("p", "knowledge-note", "Not recorded for this memory root."),
                );
            }
            card
        },
    );

    sync_memory_promotions(&v, window);

    let mut check_items = Vec::new();
    if m::yes(&v["check_available"]) {
        check_items.push(json!({ "key": "check-btn" }));
    }
    if !m::data("memory-use-check").is_null() {
        check_items.push(json!({
            "key": "check-details",
            "value": m::data("memory-use-check"),
        }));
    }
    dom::keyed_render(
        "memory-use-check-slot",
        &check_items,
        |item| m::text(item, "key"),
        |item| {
            if item["key"] == "check-btn" {
                let btn = dom::button("Check current memory state", "memory-use-check", V::Null);
                let _ = btn.set_attribute("data-focus-key", "memory-use-check-btn");
                btn
            } else {
                let details = dom::details(
                    "Current check",
                    &serde_json::to_string_pretty(&item["value"]).unwrap_or_default(),
                );
                let _ = details.set_attribute("data-restore-key", "memory-use-check");
                details
            }
        },
    );

    let anchor_key = format!(
        "harness.memory-use.v1:{}:{}",
        dom::value("memory-use-project"),
        m::text(&v, "bank_id")
    );
    let before = dom::json_storage(&anchor_key);
    let chunks = m::array(&v["chunks"]["items"]);
    if !before["at"].is_string() {
        set_text_if_changed("memory-use-since", "First visit to this knowledge root");
    } else {
        memory_changes(&v, &before);
    }

    let filter_val = m::data("memory-filter")
        .as_str()
        .unwrap_or("all")
        .to_owned();
    if dom::value("memory-ui-filter") != filter_val {
        dom::set_value("memory-ui-filter", &filter_val);
    }
    let chunk_items =
        build_memory_chunk_items(&chunks, v["today"].as_str().unwrap_or(""), &filter_val);
    dom::keyed_render(
        "memory-use-chunks",
        &chunk_items,
        |item| m::text(item, "key"),
        |item| {
            let key = m::text(item, "key");
            let c = &item["chunk"];
            let row = dom::details(
                &format!(
                    "{} · {} · review {}",
                    m::summary(c),
                    m::text(c, "status"),
                    m::text(c, "review_after")
                ),
                &serde_json::to_string_pretty(c).unwrap_or_default(),
            );
            let _ = row.set_attribute("data-restore-key", &key);
            if let Some(summary) = row.first_element_child() {
                let _ = summary.set_attribute("data-focus-key", &format!("chunk-summary:{key}"));
            }
            dom::add(
                &row,
                &dom::button("Open chunk", "memory-chunk-open", c.clone()),
            );
            row
        },
    );

    analytics(&v, &model);

    if let Some(pre) = dom::get("memory-use-daily-history-pre") {
        let hist_sig = v["history"].to_string();
        if pre.get_attribute("data-signature").as_deref() != Some(hist_sig.as_str()) {
            let _ = pre.set_attribute("data-signature", &hist_sig);
            pre.set_text_content(Some(
                &serde_json::to_string_pretty(&v["history"]).unwrap_or_default(),
            ));
        }
    }

    let supported = v["feedback_supported"] == true;
    let trace_items = build_memory_trace_items(
        &m::array(&v["retrievals"]["items"]),
        &v["feedback"],
        supported,
    );
    dom::keyed_render(
        "memory-use-traces",
        &trace_items,
        |item| m::text(item, "key"),
        |item| {
            let key = m::text(item, "key");
            let row = &item["row"];
            let mut feedback_map = serde_json::Map::new();
            let manifest = m::text(row, "manifest_id");
            if !manifest.is_empty() && !item["feedback"].is_null() {
                feedback_map.insert(manifest, item["feedback"].clone());
            }
            let feedback_obj = V::Object(feedback_map);
            let block = dom::el("details", "setup-file retrieval-trace", "");
            let _ = block.set_attribute("data-restore-key", &key);
            let summary = dom::el(
                "summary",
                "",
                &format!(
                    "{} · {} · {} chunks",
                    m::text(row, "at"),
                    m::text(row, "route"),
                    m::array(&row["chunks"]).len()
                ),
            );
            let _ = summary.set_attribute("data-focus-key", &format!("trace-summary:{key}"));
            dom::add(&block, &summary);
            let body = dom::el("div", "", "");
            body.set_inner_html(&render_retrieval_trace_html(
                row,
                &feedback_obj,
                m::yes(&item["supported"]),
            ));
            dom::add(&block, &body);
            block
        },
    );

    let mut snapshot = json!({"at":js_sys::Date::new_0().to_iso_string().as_string(),"today":v["today"],"chunks":{},"promotions":{},"newest":m::array(&v["retrievals"]["items"]).last().map(|r|r["at"].clone())});
    for c in m::array(&v["chunks"]["items"]) {
        snapshot["chunks"][m::text(&c, "id")] =
            json!([c["status"], c["last_verified"], c["review_after"]]);
    }
    for promotion in m::array(&v["promotions"]["items"]) {
        snapshot["promotions"][format!(
            "{}|{}",
            m::text(&promotion, "created_at"),
            m::text(&promotion, "mode")
        )] = promotion["status"].clone();
    }
    m::set("memory-anchor-key", json!(anchor_key));
    m::set("memory-anchor-snapshot", snapshot);
    if m::data("memory-visible-since").is_null() {
        m::set("memory-visible-since", json!(js_sys::Date::now()));
    }
}
pub fn context_turns_model(v: &V) -> Vec<V> {
    let launches = m::array(&v["launches"]);
    let mut turns = vec![];
    let mut earlier = 0.0;
    let mut prior_unknown = false;
    for launch in launches.into_iter().filter(|x| x["kind"] == "native") {
        let context = &launch["context"];
        let fill = &context["fill"];
        let ledger = &context["ledger"];
        let capsule = &ledger["capsule"];
        let excerpts = m::array(&ledger["excerpts"]);
        let chars = excerpts
            .iter()
            .filter_map(|v| m::number(&v["characters"]))
            .sum::<f64>();
        let brain = m::number(&capsule["kinds"]["brain"]).unwrap_or(0.) / 3.6;
        let bank = m::number(&capsule["kinds"]["bank"]).unwrap_or(0.) / 3.6;
        let rules = m::number(&capsule["kinds"]["rules"]).unwrap_or(0.) / 3.6 + chars / 4.7;
        let omnigraph = m::number(&ledger["omnigraph"]).unwrap_or(0.) / 3.6;
        let memory = brain + bank + rules + omnigraph;
        let start = m::number(&fill["start"]);
        let end = m::number(&fill["end"]);
        let window = m::number(&fill["window"]);
        let compacted = !m::array(&fill["compactions"]).is_empty();
        let rest = start.filter(|n| *n >= memory).map(|n| n - memory);
        let added = if compacted {
            None
        } else {
            start.zip(end).map(|(a, b)| b - a)
        };
        let free = window.zip(end).map(|(w, e)| w - e);
        let t = json!({"id":launch["id"],"ordinal":turns.len()+1,"at":launch["started_at"],"start":fill["start"],"end":fill["end"],"window":fill["window"],"peak":fill["peak"],"calls":fill["calls"],"cache":fill["cache_share"],"recorded":ledger.is_object(),"memory":memory,"parts":{"brain":brain,"bank":bank,"rules":rules,"omnigraph":omnigraph,"rest":rest,"added":added,"free":free},"earlier_memory_upper_bound":if prior_unknown{V::Null}else{json!(earlier)},"compactions":fill["compactions"],"ledger":ledger,"hooks":context["hooks"],"cli_files":context["cli_files"]});
        if compacted {
            earlier = 0.;
            prior_unknown = false;
        }
        if ledger.is_object() {
            earlier += memory;
        } else {
            prior_unknown = true;
        }
        turns.push(t);
    }
    turns
}
pub fn usage(v: &V) {
    let turns = context_turns_model(v);
    m::set("context-turns", json!(turns));
    render_usage();
}
fn context_turn_roving_states(shown: &[V], selected_id: &V) -> Vec<(String, bool, &'static str)> {
    let checked_idx = shown.iter().position(|t| &t["id"] == selected_id);
    let tab_idx = checked_idx.unwrap_or_else(|| shown.len().saturating_sub(1));
    shown
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let checked = checked_idx == Some(i);
            let tabindex = if i == tab_idx { "0" } else { "-1" };
            (m::string(&t["id"]), checked, tabindex)
        })
        .collect()
}

fn build_usage_detail_items(turn: &V, turns: &[V]) -> Vec<V> {
    let mut items = Vec::new();
    for (field, title) in [
        ("ledger", "Exact delivered context ledger"),
        ("hooks", "Hook context"),
        ("cli_files", "CLI files"),
        ("compactions", "Compactions"),
    ] {
        if !turn[field].is_null() {
            items.push(json!({
                "key": field,
                "title": title,
                "kind": "json",
                "value": turn[field],
            }));
        }
    }
    items.push(json!({
        "key": "turn-table",
        "title": "Turn table",
        "kind": "table",
        "turns": turns,
    }));
    items
}

fn ensure_usage_layout() {
    if dom::get("context-usage-summary").is_some()
        && dom::get("context-usage-pager").is_some()
        && dom::get("context-turns").is_some()
        && dom::get("context-usage-details").is_some()
    {
        return;
    }
    dom::clear("context-usage");
    let summary = dom::el("div", "", "");
    let _ = summary.set_attribute("id", "context-usage-summary");
    dom::append("context-usage", &summary);

    let pager = dom::el("div", "", "");
    let _ = pager.set_attribute("id", "context-usage-pager");
    dom::append("context-usage", &pager);

    let strip = dom::el("div", "context-turns", "");
    let _ = strip.set_attribute("id", "context-turns");
    let _ = strip.set_attribute("role", "radiogroup");
    let _ = strip.set_attribute("aria-label", "Context turns");
    dom::append("context-usage", &strip);

    let details = dom::el("div", "", "");
    let _ = details.set_attribute("id", "context-usage-details");
    dom::append("context-usage", &details);
}

fn sync_context_turns_selection(shown: &[V], selected_id: &V) {
    let Some(strip) = dom::get("context-turns") else {
        return;
    };
    let states = context_turn_roving_states(shown, selected_id);
    for (child, (_, checked, tabindex)) in dom::children(&strip).into_iter().zip(states) {
        let checked_str = if checked { "true" } else { "false" };
        if child.get_attribute("aria-checked").as_deref() != Some(checked_str) {
            let _ = child.set_attribute("aria-checked", checked_str);
        }
        if child.get_attribute("tabindex").as_deref() != Some(tabindex) {
            let _ = child.set_attribute("tabindex", tabindex);
        }
    }
}

fn render_usage() {
    let turns = m::array(&m::data("context-turns"));
    dom::hidden("context-usage", turns.is_empty());
    if turns.is_empty() {
        return;
    }
    ensure_usage_layout();
    let selected = m::data("context-turn-selected");
    let turn = turns
        .iter()
        .find(|v| v["id"] == selected)
        .or_else(|| turns.last())
        .cloned()
        .unwrap_or(V::Null);

    if let Some(summary) = dom::get("context-usage-summary") {
        let sig = turn.to_string();
        if summary.get_attribute("data-signature").as_deref() != Some(sig.as_str()) {
            let _ = summary.set_attribute("data-signature", &sig);
            summary.set_text_content(None);
            dom::add(&summary, &dom::el("h3", "", "Context"));
            let headline = if let (Some(end), Some(window)) =
                (m::number(&turn["end"]), m::number(&turn["window"]))
            {
                format!(
                    "{end:.0} of {window:.0} tokens · {:.0}%",
                    end / window * 100.
                )
            } else {
                "Context size not reported by this provider".into()
            };
            dom::add(&summary, &dom::el("p", "context-headline", &headline));
            dom::add(
                &summary,
                &dom::el(
                    "p",
                    "knowledge-note",
                    &format!(
                        "Memory sent this turn ≈ {:.0} tokens. Provider fill is exact; memory shares are character-based estimates.",
                        turn["memory"].as_f64().unwrap_or(0.)
                    ),
                ),
            );
            let bar = dom::el("div", "context-bar", "");
            let _ = bar.set_attribute("role", "img");
            for kind in ["brain", "bank", "rules", "omnigraph", "rest", "added"] {
                let value = turn_part(&turn, kind);
                let window = m::number(&turn["window"]).unwrap_or(0.);
                if value <= 0. || window <= 0. {
                    continue;
                }
                let part = dom::el("span", "context-part", "");
                let _ = part.set_attribute("data-kind", kind);
                let _ =
                    part.set_attribute("title", &format!("{} ≈ {value:.0} tokens", m::label(kind)));
                if let Some(e) = part.dyn_ref::<web_sys::HtmlElement>() {
                    let _ = e
                        .style()
                        .set_property("width", &format!("{}%", (value / window * 100.).max(0.33)));
                }
                dom::add(&bar, &part);
            }
            dom::add(&summary, &bar);
            let legend = dom::el("table", "context-legend", "");
            legend.set_inner_html(&render_usage_legend_html(&turn));
            dom::add(&summary, &legend);
        }
    }

    let page = m::data("context-page").as_u64().unwrap_or(0) as usize;
    let size = if dom::window()
        .match_media("(max-width:760px)")
        .ok()
        .flatten()
        .map(|m| m.matches())
        .unwrap_or(false)
    {
        6
    } else {
        12
    };
    let page = page.min(turns.len().saturating_sub(1) / size);
    let end = turns.len() - page * size;
    let shown = &turns[end.saturating_sub(size)..end];

    if let Some(pager) = dom::get("context-usage-pager") {
        let pager_sig = json!([end > size, page > 0, page]).to_string();
        if pager.get_attribute("data-signature").as_deref() != Some(pager_sig.as_str()) {
            let focus_key = dom::document()
                .active_element()
                .filter(|f| pager.contains(Some(f)))
                .and_then(|f| f.get_attribute("data-focus-key"));
            let _ = pager.set_attribute("data-signature", &pager_sig);
            pager.set_text_content(None);
            if end > size {
                let earlier = dom::button("‹ Earlier", "context-page", json!(page + 1));
                let _ = earlier.set_attribute("data-focus-key", "context-earlier");
                dom::add(&pager, &earlier);
            }
            if page > 0 {
                let newer = dom::button("Newer ›", "context-page", json!(page - 1));
                let _ = newer.set_attribute("data-focus-key", "context-newer");
                dom::add(&pager, &newer);
            }
            let latest = dom::button("Latest", "context-latest", V::Null);
            let _ = latest.set_attribute("data-focus-key", "context-latest");
            dom::add(&pager, &latest);
            if let Some(fk) = focus_key {
                if let Ok(nodes) = pager.query_selector_all("[data-focus-key]") {
                    for i in 0..nodes.length() {
                        if let Some(e) = nodes
                            .item(i)
                            .and_then(|v| v.dyn_into::<web_sys::HtmlElement>().ok())
                        {
                            if e.get_attribute("data-focus-key").as_deref() == Some(fk.as_str()) {
                                let _ = e.focus();
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    dom::keyed_render(
        "context-turns",
        shown,
        |t| m::string(&t["id"]),
        |t| {
            let b = dom::button("", "context-turn", t["id"].clone());
            b.set_class_name("context-turn");
            b.set_inner_html(&render_usage_turn_button_html(t));
            let _ = b.set_attribute("aria-label", &format!("Turn {}", m::string(&t["ordinal"])));
            let _ = b.set_attribute("role", "radio");
            let _ = b.set_attribute("aria-checked", "false");
            let _ = b.set_attribute("tabindex", "-1");
            b
        },
    );
    sync_context_turns_selection(shown, &turn["id"]);

    let detail_items = build_usage_detail_items(&turn, &turns);
    dom::keyed_render(
        "context-usage-details",
        &detail_items,
        |item| m::text(item, "key"),
        |item| {
            let key = m::text(item, "key");
            let title = m::text(item, "title");
            let details = if item["kind"] == "table" {
                let all = dom::details(&title, "");
                for t in m::array(&item["turns"]) {
                    dom::add(
                        &all,
                        &dom::el(
                            "p",
                            "",
                            &format!(
                                "Turn {} · end {} · window {} · memory ≈ {:.0}",
                                m::string(&t["ordinal"]),
                                m::string(&t["end"]),
                                m::string(&t["window"]),
                                t["memory"].as_f64().unwrap_or(0.)
                            ),
                        ),
                    );
                }
                all
            } else {
                dom::details(
                    &title,
                    &serde_json::to_string_pretty(&item["value"]).unwrap_or_default(),
                )
            };
            let _ = details.set_attribute("data-restore-key", &key);
            details
        },
    );
}
/// Where this browser keeps the reviewer name a person last typed on Memory
/// use.
const PROMOTION_REVIEWER_KEY: &str = "harness.memory-reviewer.v1";
fn promotion_reviewer() -> String {
    match m::data("memory-promotion-reviewer").as_str() {
        Some(name) => name.to_owned(),
        None => dom::storage(PROMOTION_REVIEWER_KEY).unwrap_or_default(),
    }
}
/// Approve, reject or apply one proposal through the knowledge operation,
/// then read Memory use again.
async fn promotion_action(action: &str, payload: &V) {
    let Some(id) = payload["promotion_id"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let project = dom::value("memory-use-project");
    let data = m::data("memory-use");
    if project.is_empty()
        || data["project_id"] != project
        || m::yes(&m::data("memory-promotion-busy"))
    {
        return;
    }
    let review = action == "memory-promotion-review";
    let reject = payload["reject"] == true;
    let mut body = if review {
        let reviewer = dom::value("memory-promotion-reviewer").trim().to_owned();
        if reviewer.is_empty() {
            dom::error(
                "memory-use-error",
                "Enter your name as the reviewer before you approve or reject a proposal.",
            );
            dom::focus("memory-promotion-reviewer");
            return;
        }
        json!({"action": "promote-review", "promotion_id": id, "reviewer": reviewer, "reject": reject})
    } else {
        json!({"action": "promote-apply", "promotion_id": id})
    };
    if data["bank_id"].is_string() {
        body["bank"] = data["bank_id"].clone();
    }
    dom::error("memory-use-error", "");
    m::take("memory-promotion-status");
    m::set("memory-promotion-busy", json!(true));
    render_memory();
    let result = api::post(
        &format!("/api/projects/{}/knowledge", dom::enc(&project)),
        body,
    )
    .await;
    m::take("memory-promotion-busy");
    match result {
        Ok(v) if m::yes(&v["ok"]) => m::set(
            "memory-promotion-status",
            json!(format!(
                "{} promotion {id}.",
                if !review {
                    "Applied"
                } else if reject {
                    "Rejected"
                } else {
                    "Approved"
                }
            )),
        ),
        Ok(v) => {
            let error = v["error"]
                .as_str()
                .filter(|error| !error.is_empty())
                .unwrap_or("The promotion operation failed.");
            dom::error(
                "memory-use-error",
                &match v["error_code"].as_str().filter(|code| !code.is_empty()) {
                    Some(code) => format!("{error} ({code})"),
                    None => error.to_owned(),
                },
            );
        }
        Err(e) => dom::error("memory-use-error", &e.message),
    }
    if dom::value("memory-use-project") == project {
        // Drawn again at once, so a failed reload leaves no button waiting.
        render_memory();
        load_memory_use().await;
    }
}
pub async fn event(kind: &str, id: &str, action: &str, payload: &V, value: &str) -> bool {
    if kind == "input" && id == "memory-promotion-reviewer" {
        m::set("memory-promotion-reviewer", json!(value));
        dom::save(
            PROMOTION_REVIEWER_KEY,
            Some(value.trim()).filter(|name| !name.is_empty()),
        );
        return true;
    }
    if kind == "input" && (id == "memory-filter" || id == "brain-filter") {
        render_list(if id.starts_with("brain") {
            "brain"
        } else {
            "memory"
        });
        return true;
    }
    if kind == "change" {
        if id == "brain-category" {
            render_list("brain");
            return true;
        }
        if id == "brain-followup-review" {
            linked_controls();
            return true;
        }
        if id == "context-project" {
            load_context().await;
            return true;
        }
        if id == "memory-use-project" || id == "memory-use-bank" {
            load_memory_use().await;
            return true;
        }
        if id == "memory-use-window" {
            render_memory();
            return true;
        }
        if id == "memory-ui-filter" {
            m::set("memory-filter", json!(value));
            render_memory();
            return true;
        }
        for scope in ["brain", "memory"] {
            if id == format!("{scope}-project") || id == format!("{scope}-bank") {
                load(scope, true).await;
                return true;
            }
            if id == format!("{scope}-knowledge-action") {
                fields(scope);
                tool_controls(scope);
                return true;
            }
        }
        if id == "brain-link-bank" {
            load_brain_link().await;
            return true;
        }
    }
    if kind == "submit" {
        if id == "memory-search-form" {
            let mut fields = json!({"query":dom::value("memory-search-query").trim()});
            let layer = dom::value("memory-search-layer");
            if !layer.is_empty() {
                fields["layer"] = json!(layer);
            }
            operation("memory", "search", fields).await;
            return true;
        }
        if id == "linked-context-form" {
            session::session_action(
                "context",
                json!({"query":dom::value("linked-context-query").trim()}),
                "linked-context-error",
            )
            .await;
            return true;
        }
        for scope in ["memory", "brain"] {
            if id == format!("{scope}-knowledge-form") {
                submit(scope).await;
                return true;
            }
        }
    }
    if (kind == "pointerover" || kind == "focusin") && action == "memory-retrieval" {
        retrieval_detail(payload.as_u64().unwrap_or(0) as usize);
        return true;
    }
    if (kind == "pointerover" || kind == "focusin") && action == "memory-horizon-chunk" {
        chunk_detail(payload);
        return true;
    }
    if kind != "click" {
        return false;
    }
    match action {
        "memory-window" => {
            m::set("memory-window", payload.clone());
            for e in dom::all("#memory-use-window button") {
                let selected = e
                    .get_attribute("data-payload")
                    .and_then(|s| serde_json::from_str::<V>(&s).ok())
                    .as_ref()
                    == Some(payload);
                let _ = e.set_attribute("aria-checked", if selected { "true" } else { "false" });
                let _ = e.set_attribute("tabindex", if selected { "0" } else { "-1" });
            }
            render_memory();
        }
        "memory-horizon-chunk" => {
            m::set("memory-selected-chunk", payload["id"].clone());
            chunk_detail(payload);
        }
        "memory-retrieval" => {
            m::set("memory-pinned-retrieval", payload.clone());
            retrieval_detail(payload.as_u64().unwrap_or(0) as usize);
        }
        "memory-filter-chip" => {
            let name = payload.as_str().unwrap_or("all");
            let mut selected = m::array(&m::data("memory-filters"));
            if name == "all" {
                selected.clear();
            } else if selected.iter().any(|v| v == name) {
                selected.retain(|v| v != name);
            } else {
                selected.push(json!(name));
            }
            m::set("memory-filters", json!(selected));
            render_memory();
        }
        "memory-sort" => {
            let kind = payload["kind"].as_str().unwrap_or("chunk");
            let field = payload["field"].as_str().unwrap_or("id");
            let old = m::data(&format!("memory-sort-{kind}"));
            m::set(
                &format!("memory-sort-{kind}"),
                json!({"field":field,"desc":old["field"]==field&&old["desc"]!=true}),
            );
            render_memory();
        }
        "memory-chunk-action" => {
            let p = dom::value("memory-use-project");
            dom::set_value("memory-project", &p);
            let bank = m::data("memory-use")["bank_id"].clone();
            dom::set_value("memory-bank", &m::string(&bank));
            session::open_view("memory", true).await;
            let chunk = &payload["chunk"];
            if let Some(path) = chunk["path"].as_str() {
                select("memory", path).await;
            }
            if let Some(action) = payload["action"].as_str() {
                dom::set_value("memory-knowledge-action", action);
                fields("memory");
                dom::set_value("memory-op-memory_id", &m::text(chunk, "id"));
                if let Some(e) = dom::get("memory-knowledge-panel") {
                    let _ = e.set_attribute("open", "");
                }
                dom::focus("memory-op-memory_id");
            }
        }

        "refresh-context" => load_context().await,
        "refresh-memory" => load("memory", false).await,
        "refresh-brain" => load("brain", false).await,
        "knowledge-file" => {
            select(
                payload["scope"].as_str().unwrap_or("memory"),
                payload["path"].as_str().unwrap_or(""),
            )
            .await
        }
        "memory-use-refresh" => load_memory_use().await,
        "memory-use-check" => {
            let data = m::data("memory-use");
            match api::post(
                &format!(
                    "/api/projects/{}/memory-use/check",
                    dom::enc(&dom::value("memory-use-project"))
                ),
                json!({"bank":data["bank_id"]}),
            )
            .await
            {
                Ok(v) => {
                    m::set("memory-use-check", v);
                    render_memory();
                }
                Err(e) => dom::error("memory-use-error", &e.message),
            }
        }
        "memory-feedback" => {
            let project = dom::value("memory-use-project");
            let bank = m::data("memory-use")["bank_id"].clone();
            let mut body = json!({
                "action": "retrieval-feedback",
                "manifest_id": payload["manifest_id"],
                "chunk_id": payload["chunk_id"],
                "vote": payload["vote"],
            });
            if bank.is_string() {
                body["bank"] = bank;
            }
            match api::post(
                &format!("/api/projects/{}/knowledge", dom::enc(&project)),
                body,
            )
            .await
            {
                Ok(_) => load_memory_use().await,
                Err(e) => dom::error("memory-use-error", &e.message),
            }
        }
        "memory-promotion-review" | "memory-promotion-apply" => {
            promotion_action(action, payload).await
        }
        "memory-promotion-brain" => {
            // Project Brain shows the source record Memory use names only by ID.
            let project = dom::value("memory-use-project");
            if let Some(bank) = m::data("memory-use")["bank_id"].as_str() {
                if dom::value("brain-project") == project {
                    dom::set_value("brain-bank", bank);
                }
            }
            open_from_omnigraph(
                "brain",
                &project,
                payload["record_id"].as_str().unwrap_or(""),
            )
            .await;
        }
        "memory-chunk-open" => {
            let p = dom::value("memory-use-project");
            dom::set_value("memory-project", &p);
            let bank = m::data("memory-use")["bank_id"].clone();
            dom::set_value("memory-bank", &m::string(&bank));
            session::open_view("memory", true).await;
            if let Some(path) = payload["path"].as_str() {
                select("memory", path).await;
            }
        }
        "brain-use-selected" => selected("brain"),
        "memory-use-selected" => selected("memory"),
        "brain-link-refresh" => load_brain_link().await,
        "linked-context-run" => {
            let context = m::read(|s| s.selected["brain"]["context_id"].clone());
            if context.is_string() {
                session::session_action(
                    "run",
                    json!({"context_id":context}),
                    "linked-context-error",
                )
                .await;
            }
        }
        "context-page" => {
            m::set("context-page", payload.clone());
            render_usage();
        }
        "context-turn" => {
            m::set("context-turn-selected", payload.clone());
            render_usage();
        }
        "context-latest" => {
            m::take("context-turn-selected");
            m::set("context-page", json!(0));
            render_usage();
        }
        "context-meter" => session::open_view("usage", true).await,
        "knowledge-open-omnigraph" => {
            let project = payload["project_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    let p = dom::value("brain-project");
                    if p.is_empty() {
                        dom::value("memory-project")
                    } else {
                        p
                    }
                });
            let atom_id = payload["atom_id"].as_str().unwrap_or("").trim().to_owned();
            crate::omnigraph::open_knowledge_atom(&project, &atom_id).await;
        }
        _ => return false,
    }
    true
}

pub fn leave_memory() {
    let Some(since) = m::data("memory-visible-since").as_f64() else {
        return;
    };
    if js_sys::Date::now() - since < 2000. {
        return;
    }
    let key = m::string(&m::data("memory-anchor-key"));
    let snapshot = m::data("memory-anchor-snapshot");
    if !key.is_empty() && snapshot.is_object() {
        dom::save(&key, Some(&snapshot.to_string()));
    }
    m::take("memory-visible-since");
}
pub fn visibility() {
    if m::read(|s| s.view != "memory-use") {
        return;
    }
    if dom::document().hidden() {
        leave_memory();
    } else {
        m::set("memory-visible-since", json!(js_sys::Date::now()));
    }
}
fn day_distance(from: &str, to: &str) -> Option<i64> {
    let a = js_sys::Date::new(&format!("{}T00:00:00", from.get(..10)?).into()).get_time();
    let b = js_sys::Date::new(&format!("{}T00:00:00", to.get(..10)?).into()).get_time();
    if a.is_finite() && b.is_finite() {
        Some(((b - a) / 86400000.).round() as i64)
    } else {
        None
    }
}
fn style(e: &web_sys::Element, name: &str, value: &str) {
    if let Some(e) = e.dyn_ref::<web_sys::HtmlElement>() {
        let _ = e.style().set_property(name, value);
    }
}
fn selected_count(data: &V, id: &str) -> usize {
    m::array(&data["retrievals"]["items"])
        .iter()
        .filter(|r| m::array(&r["chunks"]).iter().any(|v| v == id))
        .count()
}
fn historical_count(data: &V, id: &str) -> u64 {
    m::array(&data["history"]["days"])
        .iter()
        .filter_map(|d| d["chunks"][id].as_u64())
        .sum()
}
fn chunk_matches(chunk: &V, data: &V, filter: &str) -> bool {
    let id = m::text(chunk, "id");
    let days = day_distance(
        data["today"].as_str().unwrap_or(""),
        chunk["review_after"].as_str().unwrap_or(""),
    );
    match filter {
        "past" => days.is_some_and(|d| d < 0),
        "due" => days.is_some_and(|d| (0..=30).contains(&d)),
        "changed" => chunk["sources_changed"] == true,
        "unattested" => {
            chunk["auto"] == true
                && chunk["last_verified"].as_str().unwrap_or("")
                    <= chunk["created"].as_str().unwrap_or("")
        }
        "never" => selected_count(data, &id) == 0 && historical_count(data, &id) == 0,
        _ => true,
    }
}
fn memory_table_sort_dir(sort: &V, default_field: &str, field: &str) -> Option<&'static str> {
    let active = sort["field"].as_str().unwrap_or(default_field);
    if active != field {
        return None;
    }
    Some(if sort["desc"] == true {
        "descending"
    } else {
        "ascending"
    })
}

fn analytics(data: &V, _model: &V) {
    let raw = m::array(&data["retrievals"]["items"]);
    let mobile = dom::window()
        .match_media("(max-width:760px)")
        .ok()
        .flatten()
        .map(|m| m.matches())
        .unwrap_or(false);
    let shown = if mobile && raw.len() > 80 {
        &raw[raw.len() - 80..]
    } else {
        &raw[..]
    };
    let offset = raw.len() - shown.len();

    if let Some(section) = dom::get("memory-use-strip") {
        let selected_pos = m::data("memory-pinned-retrieval")
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or_else(|| raw.len().saturating_sub(1));
        let strip_sig = json!([shown, mobile, selected_pos]).to_string();
        if section.get_attribute("data-signature").as_deref() != Some(strip_sig.as_str()) {
            let _ = section.set_attribute("data-signature", &strip_sig);
            section.set_text_content(None);
            dom::add(
                &section,
                &dom::el("h3", "", &format!("Last {} retrievals", shown.len())),
            );
            let strip = dom::el("div", "memory-strip", "");
            let labels = dom::el("div", "memory-strip-labels", "");
            let columns = dom::el("div", "memory-strip-columns", "");
            let _ = columns.set_attribute("role", "listbox");
            let _ = columns.set_attribute("data-memory-options", "retrieval");
            let _ = columns.set_attribute("aria-label", "Recorded retrievals");
            for (kind, label) in [
                ("brain", "Project Brain"),
                ("bank", "Memory bank"),
                ("rules", "Rules & docs"),
            ] {
                let most = shown
                    .iter()
                    .map(|r| {
                        r[kind].as_u64().unwrap_or(0)
                            + if kind == "bank" {
                                m::array(&r["cuts"]).len() as u64
                            } else {
                                0
                            }
                    })
                    .max()
                    .unwrap_or(1)
                    .max(1);
                let e = dom::el("span", "memory-strip-label", label);
                style(&e, "--row", &format!("{}px", most * 4));
                dom::add(&labels, &e);
            }
            for (index, row) in shown.iter().enumerate() {
                let position = index + offset;
                let col = dom::button("", "memory-retrieval", json!(position));
                col.set_class_name("memory-column");
                let _ = col.set_attribute("role", "option");
                let selected = m::data("memory-pinned-retrieval")
                    .as_u64()
                    .map(|n| n as usize == position)
                    .unwrap_or(position + 1 == raw.len());
                let _ = col.set_attribute("tabindex", if selected { "0" } else { "-1" });
                let _ = col.set_attribute("aria-selected", if selected { "true" } else { "false" });
                let _ = col.set_attribute(
                    "aria-label",
                    &format!(
                        "{} · {} · {} chunks",
                        m::text(row, "at"),
                        m::text(row, "route"),
                        m::array(&row["chunks"]).len()
                    ),
                );
                for kind in ["brain", "bank", "rules"] {
                    let most = shown
                        .iter()
                        .map(|r| {
                            r[kind].as_u64().unwrap_or(0)
                                + if kind == "bank" {
                                    m::array(&r["cuts"]).len() as u64
                                } else {
                                    0
                                }
                        })
                        .max()
                        .unwrap_or(1)
                        .max(1);
                    let cell = dom::el("span", "memory-cell", "");
                    let _ = cell.set_attribute("data-kind", kind);
                    style(&cell, "--row", &format!("{}px", most * 4));
                    if kind == "bank" && !m::array(&row["cuts"]).is_empty() {
                        let cuts = dom::el("span", "memory-units cut", "");
                        style(
                            &cuts,
                            "--units",
                            &format!("{}px", m::array(&row["cuts"]).len() * 4),
                        );
                        dom::add(&cell, &cuts);
                    }
                    let units = dom::el("span", "memory-units", "");
                    let count = row[kind].as_u64().unwrap_or(0);
                    style(&units, "--units", &format!("{}px", count * 4));
                    dom::hide(&units, count == 0);
                    dom::add(&cell, &units);
                    dom::add(&col, &cell);
                }
                dom::add(&columns, &col);
            }
            let scroller = dom::el("div", "memory-strip-scroll", "");
            dom::add(&scroller, &columns);
            dom::add(&strip, &labels);
            dom::add(&strip, &scroller);
            dom::add(&section, &strip);
            let detail = dom::el("div", "memory-detail", "");
            let _ = detail.set_attribute("id", "memory-strip-detail");
            dom::add(&section, &detail);
            if !raw.is_empty() {
                retrieval_detail(selected_pos);
            }
        }
    }

    let mut chunks = m::array(&data["chunks"]["items"])
        .into_iter()
        .filter(|c| c["status"] == "active" && c["review_after"].is_string())
        .collect::<Vec<_>>();
    chunks.sort_by_key(|c| (m::text(c, "review_after"), m::text(c, "id")));
    let filters = m::array(&m::data("memory-filters"));

    if let Some(horizon) = dom::get("memory-use-horizon") {
        let base_sig = json!([
            chunks,
            data["today"],
            data["retrievals"]["items"],
            data["history"]["days"],
            mobile
        ])
        .to_string();
        let filters_sig = json!(filters).to_string();
        if horizon.get_attribute("data-base-signature").as_deref() == Some(base_sig.as_str()) {
            if horizon.get_attribute("data-filters-signature").as_deref()
                != Some(filters_sig.as_str())
            {
                let _ = horizon.set_attribute("data-filters-signature", &filters_sig);
                if let Ok(chips) =
                    horizon.query_selector_all("button[data-action=\"memory-filter-chip\"]")
                {
                    for i in 0..chips.length() {
                        if let Some(b) = chips
                            .item(i)
                            .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
                        {
                            let filter = b
                                .get_attribute("data-payload")
                                .and_then(|s| serde_json::from_str::<V>(&s).ok())
                                .and_then(|v| v.as_str().map(str::to_owned))
                                .unwrap_or_default();
                            let on = if filter == "all" {
                                filters.is_empty()
                            } else {
                                filters.iter().any(|v| v == &filter)
                            };
                            let _ =
                                b.set_attribute("aria-pressed", if on { "true" } else { "false" });
                        }
                    }
                }
                let by_id = chunks
                    .iter()
                    .map(|c| (m::text(c, "id"), c))
                    .collect::<BTreeMap<_, _>>();
                if let Ok(dots) = horizon.query_selector_all(".memory-dot[data-id]") {
                    for i in 0..dots.length() {
                        if let Some(dot) = dots
                            .item(i)
                            .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
                        {
                            if let Some(chunk) =
                                dot.get_attribute("data-id").and_then(|id| by_id.get(&id))
                            {
                                let dim = !filters
                                    .iter()
                                    .all(|f| chunk_matches(chunk, data, f.as_str().unwrap_or("")));
                                let _ = dot
                                    .set_attribute("data-dim", if dim { "true" } else { "false" });
                            }
                        }
                    }
                }
            }
        } else {
            let _ = horizon.set_attribute("data-base-signature", &base_sig);
            let _ = horizon.set_attribute("data-filters-signature", &filters_sig);
            horizon.set_text_content(None);
            dom::add(&horizon, &dom::el("h3", "", "Review horizon"));
            let chips = dom::el("div", "memory-chips", "");
            for (filter, label) in [
                ("all", "All"),
                ("past", "Past review"),
                ("due", "Due ≤30d"),
                ("changed", "Sources changed"),
                ("unattested", "Not re-attested since promotion"),
                ("never", "Never selected"),
            ] {
                let count = chunks
                    .iter()
                    .filter(|c| chunk_matches(c, data, filter))
                    .count();
                let b = dom::button(
                    &format!("{label} {count}"),
                    "memory-filter-chip",
                    json!(filter),
                );
                b.set_class_name("chip");
                let on = if filter == "all" {
                    filters.is_empty()
                } else {
                    filters.iter().any(|v| v == filter)
                };
                let _ = b.set_attribute("aria-pressed", if on { "true" } else { "false" });
                if count == 0 && filter != "all" {
                    let _ = b.set_attribute("disabled", "");
                }
                dom::add(&chips, &b);
            }
            dom::add(&horizon, &chips);
            let chart = dom::el("div", "memory-horizon", "");
            let past = dom::el("div", "memory-horizon-past", "Past review date");
            style(&past, "width", &format!("{}%", 30. / 395. * 100.));
            dom::add(&chart, &past);
            let zero = dom::el("div", "memory-horizon-zero", "");
            style(&zero, "left", &format!("{}%", 30. / 395. * 100.));
            dom::add(&chart, &zero);
            let bins = dom::el("div", "memory-horizon-bins", "");
            let _ = bins.set_attribute("role", "group");
            let _ = bins.set_attribute("data-memory-options", "chunks");
            let bin = if mobile { 14 } else { 7 };
            let mut groups: BTreeMap<i64, Vec<V>> = BTreeMap::new();
            for chunk in &chunks {
                if let Some(day) = day_distance(
                    data["today"].as_str().unwrap_or(""),
                    chunk["review_after"].as_str().unwrap_or(""),
                ) {
                    groups
                        .entry((day.clamp(-30, 365) + 30) / bin)
                        .or_default()
                        .push(chunk.clone());
                }
            }
            for (key, members) in groups {
                let column = dom::el("div", "memory-bin", "");
                style(
                    &column,
                    "left",
                    &format!("{}%", key as f64 * bin as f64 / 395. * 100.),
                );
                style(&column, "width", &format!("{}%", bin as f64 / 395. * 100.));
                for (index, chunk) in members.iter().enumerate() {
                    let id = m::text(chunk, "id");
                    let dot = dom::button("", "memory-horizon-chunk", chunk.clone());
                    dot.set_class_name("memory-dot");
                    let provenance = if chunk["auto"] == true {
                        if m::text(chunk, "last_verified") > m::text(chunk, "created") {
                            "auto-attested"
                        } else {
                            "auto"
                        }
                    } else {
                        "person"
                    };
                    let _ = dot.set_attribute("data-provenance", provenance);
                    let _ = dot.set_attribute("data-id", &id);
                    let _ = dot.set_attribute(
                        "tabindex",
                        if m::data("memory-selected-chunk") == id
                            || m::data("memory-selected-chunk").is_null()
                                && chunks.first().is_some_and(|v| v["id"] == id)
                        {
                            "0"
                        } else {
                            "-1"
                        },
                    );
                    let _ = dot.set_attribute(
                        "aria-label",
                        &format!(
                            "{id}, {provenance}, review {}, selected in {} retrievals",
                            m::text(chunk, "review_after"),
                            selected_count(data, &id)
                        ),
                    );
                    let _ = dot.set_attribute(
                        "data-dim",
                        if filters
                            .iter()
                            .all(|f| chunk_matches(chunk, data, f.as_str().unwrap_or("")))
                        {
                            "false"
                        } else {
                            "true"
                        },
                    );
                    if index >= 4 {
                        let _ = dot.set_attribute("data-overflow", "true");
                    }
                    dom::add(&column, &dot);
                }
                if members.len() > 4 {
                    dom::add(
                        &column,
                        &dom::el(
                            "span",
                            "memory-bin-more",
                            &format!("+{}", members.len() - 4),
                        ),
                    );
                }
                dom::add(&bins, &column);
            }
            dom::add(&chart, &bins);
            dom::add(&horizon, &chart);
            let ticks = dom::el("div", "memory-horizon-ticks", "");
            for day in if mobile {
                vec![0, 180, 365]
            } else {
                vec![-30, 0, 90, 180, 270, 365]
            } {
                let tick = dom::el(
                    "span",
                    "",
                    &format!("{day}{}", if day == 365 { " d" } else { "" }),
                );
                style(
                    &tick,
                    "left",
                    &format!("{}%", (day + 30) as f64 / 395. * 100.),
                );
                dom::add(&ticks, &tick);
            }
            dom::add(&horizon, &ticks);
            let detail = dom::el(
                "div",
                "memory-detail",
                "Focus or point at a dot to see its chunk.",
            );
            let _ = detail.set_attribute("id", "memory-horizon-detail");
            dom::add(&horizon, &detail);
            if let Some(chunk) = chunks
                .iter()
                .find(|c| c["id"] == m::data("memory-selected-chunk"))
            {
                chunk_detail(chunk);
            }
        }
    }

    if let Some(section) = dom::get("memory-use-history") {
        let days = m::array(&data["history"]["days"]);
        let hist_sig = json!(days).to_string();
        if section.get_attribute("data-signature").as_deref() != Some(hist_sig.as_str()) {
            let _ = section.set_attribute("data-signature", &hist_sig);
            section.set_text_content(None);
            dom::add(&section, &dom::el("h4", "", "Retrievals per day"));
            let plot = dom::el("div", "memory-history-plot", "");
            let history = dom::el("div", "memory-history-chart", "");
            let _ = history.set_attribute("role", "img");
            let most = days
                .iter()
                .filter_map(|d| d["retrievals"].as_u64())
                .max()
                .unwrap_or(1)
                .max(1);
            let total = days
                .iter()
                .filter_map(|d| d["retrievals"].as_u64())
                .sum::<u64>();
            let delivered = days
                .iter()
                .filter_map(|d| d["with_chunk"].as_u64())
                .sum::<u64>();
            let _ = history.set_attribute(
                "aria-label",
                &format!(
                    "{total} retrievals over {} days, {delivered} of them with a Memory Bank chunk",
                    days.len()
                ),
            );
            for day in &days {
                let column = dom::el("div", "memory-history-day", "");
                let _ = column.set_attribute(
                    "title",
                    &format!(
                        "{} · {} retrievals · {} with a chunk",
                        m::text(day, "day"),
                        m::string(&day["retrievals"]),
                        day["with_chunk"].as_u64().unwrap_or(0)
                    ),
                );
                let (rest, bank) = history_day_heights(day, most);
                for (class, height) in
                    [("memory-history-rest", rest), ("memory-history-bank", bank)]
                {
                    if height > 0. {
                        let part = dom::el("span", class, "");
                        style(&part, "height", &format!("{height:.2}%"));
                        dom::add(&column, &part);
                    }
                }
                dom::add(&history, &column);
            }
            dom::add(&plot, &history);
            if let (Some(first), Some(last)) = (days.first(), days.last()) {
                let axis = dom::el("div", "memory-history-axis", "");
                dom::add(&axis, &dom::el("span", "", &m::text(first, "day")));
                dom::add(&axis, &dom::el("span", "", &m::text(last, "day")));
                dom::add(&plot, &axis);
            }
            dom::add(&section, &plot);
        }
    }

    sortable(
        "chunk",
        &chunks,
        &[
            ("id", "Chunk"),
            ("status", "Status"),
            ("review_after", "Review date"),
            ("last_verified", "Last verified"),
            ("bytes", "Bytes"),
        ],
    );
    sortable(
        "retrieval",
        &raw,
        &[
            ("at", "When"),
            ("route", "Route"),
            ("brain", "Brain"),
            ("bank", "Bank"),
            ("rules", "Rules"),
        ],
    );
}
fn sortable(kind: &str, rows: &[V], columns: &[(&str, &str)]) {
    let sort = m::data(&format!("memory-sort-{kind}"));
    let Some(details) = dom::get(&format!("memory-table-{kind}")) else {
        return;
    };
    let table_sig = json!([rows, sort]).to_string();
    if details.get_attribute("data-signature").as_deref() == Some(table_sig.as_str()) {
        return;
    }
    let _ = details.set_attribute("data-signature", &table_sig);
    let key = sort["field"].as_str().unwrap_or(columns[0].0);
    let desc = sort["desc"] == true;
    let mut values = rows.to_vec();
    values.sort_by(|a, b| {
        let order = if let (Some(a), Some(b)) = (m::number(&a[key]), m::number(&b[key])) {
            a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            m::string(&a[key]).cmp(&m::string(&b[key]))
        };
        if desc {
            order.reverse()
        } else {
            order
        }
    });
    set_text_if_changed(
        &format!("memory-table-{kind}-summary"),
        &format!("{} table ({})", m::label(kind), values.len()),
    );
    let Some(host) = dom::get(&format!("memory-table-{kind}-host")) else {
        return;
    };
    let focus_key = dom::document()
        .active_element()
        .filter(|el| host.contains(Some(el)))
        .and_then(|el| el.get_attribute("data-focus-key"));
    host.set_text_content(None);
    let table = dom::el("table", "memory-table", "");
    let head = dom::el("tr", "", "");
    for (field, label) in columns {
        let th = dom::el("th", "", "");
        let _ = th.set_attribute("scope", "col");
        if let Some(dir) = memory_table_sort_dir(&sort, columns[0].0, field) {
            let _ = th.set_attribute("aria-sort", dir);
        }
        let btn = dom::button(label, "memory-sort", json!({"kind":kind,"field":field}));
        btn.set_class_name("memory-sort");
        let _ = btn.set_attribute("data-focus-key", &format!("sort:{kind}:{field}"));
        dom::add(&th, &btn);
        dom::add(&head, &th);
    }
    dom::add(&table, &head);
    for row in values {
        let tr = dom::el("tr", "", "");
        for (field, _) in columns {
            dom::add(&tr, &dom::el("td", "", &m::string(&row[*field])));
        }
        dom::add(&table, &tr);
    }
    dom::add(&host, &table);
    if let Some(fk) = focus_key {
        if let Ok(nodes) = host.query_selector_all("[data-focus-key]") {
            for i in 0..nodes.length() {
                if let Some(e) = nodes
                    .item(i)
                    .and_then(|v| v.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    if e.get_attribute("data-focus-key").as_deref() == Some(fk.as_str()) {
                        let _ = e.focus();
                        break;
                    }
                }
            }
        }
    }
}
fn retrieval_detail(index: usize) {
    let data = m::data("memory-use");
    let rows = m::array(&data["retrievals"]["items"]);
    let Some(row) = rows.get(index) else { return };
    dom::clear("memory-strip-detail");
    let title = format!(
        "{} · {} · {} chunks",
        m::text(row, "at"),
        m::text(row, "route"),
        m::array(&row["chunks"]).len()
    );
    dom::append(
        "memory-strip-detail",
        &dom::el("p", "memory-card-title", &title),
    );
    for (kind, label) in [
        ("brain", "Project Brain"),
        ("bank", "Memory bank"),
        ("rules", "Rules & docs"),
    ] {
        dom::append(
            "memory-strip-detail",
            &dom::el(
                "p",
                "memory-card-line",
                &format!("{label}: {} selected", m::string(&row[kind])),
            ),
        );
    }
    for id in m::array(&row["chunks"]) {
        if let Some(chunk) = m::array(&data["chunks"]["items"])
            .into_iter()
            .find(|c| c["id"] == id)
        {
            dom::append(
                "memory-strip-detail",
                &dom::button(&m::summary(&chunk), "memory-horizon-chunk", chunk),
            );
        }
    }
    for cut in m::array(&row["cuts"]) {
        dom::append(
            "memory-strip-detail",
            &dom::el(
                "p",
                "memory-card-line",
                &format!("Cut {} · {}", m::string(&cut[0]), m::string(&cut[1])),
            ),
        );
    }
}
fn chunk_detail(chunk: &V) {
    let data = m::data("memory-use");
    let id = m::text(chunk, "id");
    let days = day_distance(
        data["today"].as_str().unwrap_or(""),
        chunk["review_after"].as_str().unwrap_or(""),
    );
    let review = days
        .map(|d| {
            if d < 0 {
                format!("{}d overdue", -d)
            } else if d == 0 {
                "today".into()
            } else {
                format!("in {d}d")
            }
        })
        .unwrap_or_else(|| "unknown".into());
    dom::clear("memory-horizon-detail");
    dom::append(
        "memory-horizon-detail",
        &dom::el(
            "p",
            "memory-card-title",
            &format!("{id} · {}", m::summary(chunk)),
        ),
    );
    dom::append(
        "memory-horizon-detail",
        &dom::el(
            "p",
            "memory-card-line",
            &format!(
                "Review {review} ({}) · last verified {}",
                m::text(chunk, "review_after"),
                m::text(chunk, "last_verified")
            ),
        ),
    );
    dom::append(
        "memory-horizon-detail",
        &dom::el(
            "p",
            "memory-card-line",
            &format!(
                "Sources: {} · selected in {} of {} retrievals · {} in retained history · {}",
                match chunk["sources_changed"].as_bool() {
                    Some(true) => "a cited file changed",
                    Some(false) => "unchanged",
                    None => "not tracked",
                },
                selected_count(&data, &id),
                m::array(&data["retrievals"]["items"]).len(),
                historical_count(&data, &id),
                bytes(&chunk["bytes"])
            ),
        ),
    );
    for (label, action) in [
        ("Open in Memory bank", None),
        ("Re-attest…", Some("bank-reverify")),
        ("Retire…", Some("bank-retire")),
    ] {
        dom::append(
            "memory-horizon-detail",
            &dom::button(
                label,
                "memory-chunk-action",
                json!({"chunk":chunk,"action":action}),
            ),
        );
    }
}
pub fn key_event(event: &web_sys::KeyboardEvent) -> bool {
    let Some(target) = event
        .target()
        .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
    else {
        return false;
    };
    let Some(container) = target
        .closest("[data-memory-options],#memory-use-window,#context-turns")
        .ok()
        .flatten()
    else {
        return false;
    };
    let buttons = dom::children(&container)
        .into_iter()
        .flat_map(|e| {
            if e.tag_name() == "BUTTON" {
                vec![e]
            } else {
                e.query_selector_all("button")
                    .ok()
                    .map(|l| {
                        (0..l.length())
                            .filter_map(|i| l.item(i)?.dyn_into::<web_sys::Element>().ok())
                            .collect()
                    })
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>();
    let Some(index) = buttons.iter().position(|e| e == &target) else {
        return false;
    };
    let next = match event.key().as_str() {
        "ArrowRight" | "ArrowDown" => Some((index + 1).min(buttons.len() - 1)),
        "ArrowLeft" | "ArrowUp" => Some(index.saturating_sub(1)),
        "Home" => Some(0),
        "End" => Some(buttons.len() - 1),
        "Enter" => Some(index),
        _ => None,
    };
    let Some(next) = next else { return false };
    for (i, b) in buttons.iter().enumerate() {
        let _ = b.set_attribute("tabindex", if i == next { "0" } else { "-1" });
    }
    if let Some(e) = buttons[next].dyn_ref::<web_sys::HtmlElement>() {
        let _ = e.focus();
        e.click();
    }
    true
}

fn memory_changes(data: &V, before: &V) {
    if !before["at"].is_string() {
        return;
    }
    let mut counts = BTreeMap::<&str, usize>::new();
    for chunk in m::array(&data["chunks"]["items"]) {
        let id = m::text(&chunk, "id");
        let prior = &before["chunks"][&id];
        for (key, changed) in [
            ("added", chunk["status"] == "active" && prior.is_null()),
            (
                "retired",
                prior[0] == "active" && chunk["status"] != "active",
            ),
            (
                "re-attested",
                prior.is_array()
                    && chunk["last_verified"]
                        .as_str()
                        .zip(prior[1].as_str())
                        .map(|(a, b)| a > b)
                        .unwrap_or(false),
            ),
            (
                "review moved",
                prior.is_array()
                    && chunk["status"] == "active"
                    && prior[2].is_string()
                    && prior[2] != chunk["review_after"],
            ),
            (
                "crossed review",
                chunk["status"] == "active"
                    && chunk["review_after"]
                        .as_str()
                        .zip(data["today"].as_str())
                        .map(|(a, b)| a < b)
                        .unwrap_or(false)
                    && prior.is_array()
                    && !prior[2]
                        .as_str()
                        .zip(before["today"].as_str())
                        .map(|(a, b)| a < b)
                        .unwrap_or(false),
            ),
        ] {
            if changed {
                *counts.entry(key).or_default() += 1;
            }
        }
    }
    let applied = m::array(&data["promotions"]["items"])
        .iter()
        .filter(|p| {
            p["status"] == "applied"
                && before["promotions"]
                    [format!("{}|{}", m::text(p, "created_at"), m::text(p, "mode"))]
                    != "applied"
        })
        .count();
    let retrievals = m::array(&data["retrievals"]["items"])
        .iter()
        .filter(|r| {
            before["newest"]
                .as_str()
                .map(|at| r["at"].as_str().unwrap_or("") > at)
                .unwrap_or(true)
        })
        .count();
    let mut parts = counts
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(k, n)| format!("{n} {k}"))
        .collect::<Vec<_>>();
    if applied > 0 {
        parts.push(format!("{applied} promotions applied"));
    }
    if retrievals > 0 {
        parts.push(format!("{retrievals} retrievals"));
    }
    set_text_if_changed(
        "memory-use-since",
        &format!(
            "Since {}: {}",
            m::text(before, "at"),
            if parts.is_empty() {
                "no recorded changes".into()
            } else {
                parts.join(" · ")
            }
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::{
        capsule_segments, context_turns_model, history_day_heights, memory_stage_metrics,
        render_capsule_bar_html, render_capsule_legend_html, render_detail_omnigraph_action_html,
        render_linked_omnigraph_html, render_list_entry_inner_html, render_promotion_queue_html,
        render_usage_legend_html, render_usage_turn_button_html,
    };
    use crate::test_support::{assert_no_inline_hex_colors, payloads};
    use serde_json::json;

    #[test]
    fn promotion_queue_offers_review_apply_and_brain_by_id_only() {
        let id = |n: u8| format!("{n}{n}{n}{n}{n}{n}{n}{n}-0000-4000-8000-00000000000{n}");
        let source = |n: u8| format!("aaaaaaa{n}-bbbb-4ccc-8ddd-eeeeeeeeeeee");
        let items = vec![
            json!({"id": id(1), "source_ids": [source(1)], "reason": null, "status": "proposed", "mode": "human", "created_at": "2026-11-10T09:00:00+00:00", "reviewed_at": null, "applied_at": null, "memory_id": null, "title": "Rotate <session> keys", "path": "project-brain/dynamic/findings/rotate.md"}),
            json!({"id": id(2), "source_ids": [source(2), source(3)], "reason": "approved", "status": "reviewed", "mode": "human", "created_at": "2026-11-09T09:00:00+00:00", "reviewed_at": "2026-11-11T09:00:00+00:00"}),
            json!({"id": id(3), "source_ids": [source(4)], "status": "proposed", "mode": "automatic", "created_at": "2026-11-12T09:00:00+00:00"}),
            json!({"id": id(4), "source_ids": [source(5)], "reason": "approved-without-review", "status": "reviewed", "mode": "automatic", "created_at": "2026-11-12T10:00:00+00:00", "reviewed_at": "2026-11-12T10:00:01+00:00"}),
            json!({"id": id(5), "source_ids": [source(6)], "reason": "approved-without-review", "status": "applied", "mode": "automatic", "applied_at": "2026-11-15T10:00:00+00:00", "memory_id": "MEM-20261115-aabbccdd"}),
            json!({"id": id(6), "source_ids": [source(7)], "status": "applied", "mode": "automatic", "applied_at": "2026-09-01T10:00:00+00:00", "memory_id": "MEM-20260901-00112233"}),
            json!({"id": null, "source_ids": [], "status": "proposed", "mode": "human", "created_at": "2026-11-13T09:00:00+00:00"}),
            json!({"id": id(7), "source_ids": [source(8)], "status": "rejected", "mode": "human"}),
            json!({"id": id(8), "source_ids": [source(9)], "status": "applied", "mode": "human", "applied_at": "2026-11-16T10:00:00+00:00"}),
        ];
        let html = render_promotion_queue_html(
            &items,
            "2026-10-18T09:30:00.000Z",
            "Jane \"QA\"",
            false,
            "Approved promotion x.",
        );
        assert!(
            html.contains("3 waiting · 2 stalled · 1 auto-promoted"),
            "{html}"
        );
        // A person signs only a human proposal, oldest first; one without a
        // readable ID shows its buttons disabled.
        assert_eq!(
            payloads(&html, "memory-promotion-review"),
            vec![
                json!({"promotion_id": id(1), "reject": false}),
                json!({"promotion_id": id(1), "reject": true}),
                json!({"promotion_id": "", "reject": false}),
                json!({"promotion_id": "", "reject": true}),
            ]
        );
        assert_eq!(html.matches(" disabled>").count(), 2, "{html}");
        assert!(html.contains("Without a readable ID"), "{html}");
        // Apply publishes an approved proposal, a stalled automatic one too.
        assert_eq!(
            payloads(&html, "memory-promotion-apply"),
            vec![
                json!({"promotion_id": id(2)}),
                json!({"promotion_id": id(4)})
            ]
        );
        // An automatic proposal is never signed by a person.
        assert!(
            html.contains("An automatic proposal takes no reviewer"),
            "{html}"
        );
        // Every listed proposal opens its first source record in Project Brain.
        assert_eq!(
            payloads(&html, "memory-promotion-brain"),
            [
                source(2),
                source(1),
                String::new(),
                source(4),
                source(5),
                source(6)
            ]
            .map(|record| json!({"record_id": record}))
            .to_vec()
        );
        // IDs, outcome codes, dates and chunk IDs only: no Brain title or path.
        assert!(!html.contains("Rotate"), "{html}");
        assert!(!html.contains("project-brain/"), "{html}");
        assert!(
            html.contains("outcome <code>approved-without-review</code>"),
            "{html}"
        );
        assert!(html.contains("chunk <code>MEM-20261115-aabbccdd</code>"));
        assert!(html.contains("proposed 2026-11-10"), "{html}");
        // Outside the window, rejected and applied-after-review proposals stay off.
        for absent in ["MEM-20260901-00112233".to_owned(), id(7), id(8)] {
            assert!(!html.contains(&absent), "{absent}: {html}");
        }
        assert!(html.contains("value=\"Jane &quot;QA&quot;\""), "{html}");
        assert!(html.contains("role=\"status\">Approved promotion x.</p>"));
        assert_no_inline_hex_colors(&html);

        // While one action runs, every review and apply button waits.
        let busy = render_promotion_queue_html(&items, "", "", true, "");
        assert!(busy.contains("2 auto-promoted"), "{busy}");
        assert_eq!(busy.matches(" disabled>").count(), 6, "{busy}");
        assert!(!busy.contains("promotion-status"), "{busy}");

        let empty = render_promotion_queue_html(&[], "", "", false, "");
        assert!(empty.contains("Nothing waits for review"), "{empty}");
        assert!(!empty.contains("memory-promotion-reviewer"), "{empty}");
    }

    #[test]
    fn brain_followup_changes_review_and_reviewed_query_only() {
        use super::{followup_brain_from_fields, render_brain_followup_html};
        let automatic = json!({"task_id": "harness/fix-login-abc123", "review": false, "query": "fix login redirect"});
        // An automatic turn takes the next message as its query.
        assert_eq!(
            followup_brain_from_fields(&automatic, false, "anything"),
            Ok(None)
        );
        assert_eq!(
            followup_brain_from_fields(&automatic, true, " fix login redirect "),
            Ok(Some(json!({"review": true})))
        );
        assert_eq!(
            followup_brain_from_fields(&automatic, true, "session cookie expiry"),
            Ok(Some(
                json!({"review": true, "query": "session cookie expiry"})
            ))
        );
        assert!(followup_brain_from_fields(&automatic, true, "  ").is_err());
        let reviewed = json!({"task_id": "TASK-7", "review": true, "query": "retry policy"});
        assert_eq!(
            followup_brain_from_fields(&reviewed, true, "retry policy"),
            Ok(None)
        );
        assert_eq!(
            followup_brain_from_fields(&reviewed, true, "backoff caps"),
            Ok(Some(json!({"query": "backoff caps"})))
        );
        assert_eq!(
            followup_brain_from_fields(&reviewed, false, ""),
            Ok(Some(json!({"review": false})))
        );
        assert_eq!(
            followup_brain_from_fields(&json!(null), true, "x"),
            Ok(None)
        );

        let html = render_brain_followup_html("TASK-<7>", true, "retry & backoff");
        assert!(html.contains("<code>TASK-&lt;7&gt;</code>"), "{html}");
        assert!(
            html.contains("id=\"brain-followup-review\" type=\"checkbox\" checked>"),
            "{html}"
        );
        assert!(
            html.contains("id=\"brain-followup-query-field\">"),
            "{html}"
        );
        assert!(html.contains(">retry &amp; backoff</textarea>"), "{html}");
        assert!(html.contains("waits until you approve"), "{html}");
        let automatic_html = render_brain_followup_html("TASK-7", false, "");
        assert!(automatic_html.contains("type=\"checkbox\">Review context by hand"));
        assert!(automatic_html.contains("id=\"brain-followup-query-field\" hidden>"));
        assert!(automatic_html.contains("The next message is the retrieval query"));
        assert_no_inline_hex_colors(&html);
    }

    #[test]
    fn promotion_queue_lists_the_newest_automatic_promotions() {
        let items = (0..25)
            .map(|day| {
                json!({"id": format!("00000000-0000-4000-8000-0000000000{day:02}"), "status": "applied", "mode": "automatic", "applied_at": format!("2026-11-{:02}T08:00:00+00:00", day + 1), "source_ids": []})
            })
            .collect::<Vec<_>>();
        let html = render_promotion_queue_html(&items, "", "", false, "");
        assert!(html.contains("Showing the 20 newest of 25."), "{html}");
        assert!(html.contains("applied 2026-11-25"), "{html}");
        assert!(!html.contains("applied 2026-11-05"), "{html}");
        // Nothing to sign, so no reviewer field.
        assert!(!html.contains("memory-promotion-reviewer"), "{html}");
    }

    #[test]
    fn capsule_segments_and_legend_format_nonzero_parts_and_swatches() {
        let meter = json!({"characters": 1577, "limit": 8000, "kinds": {"brain": 752, "rules": 423, "bank": 0}, "items": {"brain": 1, "rules": 2, "bank": 0}, "repeats": 824, "dropped": {"episodic": 1}});
        let segments = capsule_segments(&meter);
        assert_eq!(
            segments
                .iter()
                .map(|(kind, _, characters, _)| (*kind, *characters))
                .collect::<Vec<_>>(),
            [("brain", 752), ("bank", 0), ("rules", 423)]
        );
        assert!((segments[0].3 - 9.4).abs() < 1e-9);
        let bar = render_capsule_bar_html(&meter);
        assert!(
            bar.contains("data-kind=\"brain\" style=\"width:9.40%\""),
            "{bar}"
        );
        assert!(bar.contains("data-kind=\"bank\" hidden"), "{bar}");
        let legend = render_capsule_legend_html(&meter);
        assert!(
            legend.contains("<i class=\"capsule-swatch\" data-kind=\"brain\""),
            "{legend}"
        );
        assert!(
            legend.contains("Rules &amp; docs · 423 characters · 2 items")
                || legend.contains("Rules & docs · 423 characters · 2 items"),
            "{legend}"
        );
        assert!(!legend.contains("Memory bank"), "{legend}");
        assert!(legend.contains("Dropped to fit: 1 episodic"), "{legend}");
        // No raw JSON reaches the legend.
        assert!(!legend.contains('{'), "{legend}");
    }

    #[test]
    fn retrieval_trace_renders_chunks_cuts_and_feedback_toggles() {
        let row = json!({"manifest_id": "manifest-1", "brain": 1, "bank": 2, "rules": 0, "chunks": ["MEM-20261117-aabbccdd", "MEM-20261117-eeff0011"], "cuts": [["MEM-20261117-99998888", "budget"]]});
        let feedback = json!({"manifest-1": {"MEM-20261117-aabbccdd": {"vote": "stale"}}});
        let html = super::render_retrieval_trace_html(&row, &feedback, true);
        assert!(
            html.contains("<code>MEM-20261117-aabbccdd</code>"),
            "{html}"
        );
        assert!(
            html.contains("MEM-20261117-99998888 — cut: budget"),
            "{html}"
        );
        // The recorded verdict is pressed, and pressing it again clears it.
        assert!(
            html.contains(
                "&quot;vote&quot;:&quot;clear&quot;}\" aria-pressed=\"true\">Stale</button>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "&quot;vote&quot;:&quot;helped&quot;}\" aria-pressed=\"false\">Helped</button>"
            ),
            "{html}"
        );
        assert!(html.contains("data-action=\"memory-feedback\""));
        // Without the server's support, or without a manifest, nothing is offered.
        assert!(!super::render_retrieval_trace_html(&row, &feedback, false).contains("<button"));
        let mut anonymous = row.clone();
        anonymous["manifest_id"] = json!(null);
        assert!(
            !super::render_retrieval_trace_html(&anonymous, &feedback, true).contains("<button")
        );
    }

    #[test]
    fn open_from_omnigraph_finds_its_record_without_a_filter() {
        let entries = vec![
            json!({"id": "11111111-2222-4333-8444-555555555555", "external_id": "finding-1", "path": "dynamic/a.md"}),
            json!({"id": "MEM-20261117-aabbccdd", "path": "memory-bank/chunks/MEM-20261117-aabbccdd-ttl.md"}),
        ];
        assert_eq!(
            super::jump_entry(&entries, "finding-1").map(|v| &v["path"]),
            Some(&json!("dynamic/a.md"))
        );
        assert!(super::jump_entry(&entries, "MEM-20261117-aabbccdd").is_some());
        assert!(super::jump_entry(&entries, "").is_none());
        assert!(super::jump_entry(&entries, "missing").is_none());
    }

    #[test]
    fn history_day_heights_stacks_with_chunk_under_retrievals() {
        let (rest, bank) = history_day_heights(&json!({"retrievals": 6, "with_chunk": 2}), 8);
        assert_eq!((rest, bank), (50.0, 25.0));
        // A count above the day's retrievals is held to them; an empty day is flat.
        assert_eq!(
            history_day_heights(&json!({"retrievals": 2, "with_chunk": 5}), 4),
            (0.0, 50.0)
        );
        assert_eq!(history_day_heights(&json!({}), 0), (0.0, 0.0));
    }

    #[test]
    fn render_usage_turn_and_legend_emit_track_parts_and_swatches() {
        let turn = json!({"ordinal": 3, "window": 200000, "end": 50000, "parts": {"brain": 1000, "bank": 500, "rules": 0, "rest": 40000, "added": 8500, "free": 150000}, "compactions": []});
        let button = render_usage_turn_button_html(&turn);
        assert!(button.contains("class=\"context-turn-track\""), "{button}");
        assert!(
            button.contains("data-kind=\"brain\" style=\"height:0.50%\""),
            "{button}"
        );
        assert!(!button.contains("data-kind=\"rules\""), "{button}");
        assert!(
            button.ends_with("<span class=\"context-turn-label\">3</span>"),
            "{button}"
        );
        let compacted = json!({"ordinal": 4, "window": 200000, "end": 20000, "parts": {"brain": 1000, "rest": 1}, "compactions": [{"at": "x"}]});
        let only_added = render_usage_turn_button_html(&compacted);
        assert!(
            only_added.contains("data-kind=\"added\" style=\"height:10.00%\""),
            "{only_added}"
        );
        assert!(!only_added.contains("data-kind=\"brain\""), "{only_added}");
        let legend = render_usage_legend_html(&turn);
        assert!(
            legend.contains("<i class=\"context-swatch\" data-kind=\"bank\""),
            "{legend}"
        );
        assert!(
            legend.contains("<td class=\"context-number\">≈ 1000</td>"),
            "{legend}"
        );
        assert!(
            legend.contains("<td class=\"context-number\">—</td>"),
            "{legend}"
        );
    }

    #[test]
    fn render_list_entry_inner_html_separates_title_path_and_meta() {
        let html = render_list_entry_inner_html(
            &json!({"title": "Retry <policy>", "path": "memory-bank/chunks/MEM-1.md", "status": "active", "bytes": 2048}),
        );
        assert!(
            html.starts_with("<span class=\"memory-entry-title\">Retry &lt;policy&gt;</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"memory-entry-path\">memory-bank/chunks/MEM-1.md</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"memory-entry-meta\">active · "),
            "{html}"
        );
    }

    #[test]
    fn selected_stage_metrics_follow_semantic_order_and_sentence_case() {
        let stage = json!({
            "cut_total": 0,
            "distinct": 2,
            "dropped": 0,
            "measured": 1,
            "retrievals": 4,
            "reused": 1,
            "selections": 5,
            "with_chunk": 3,
            "since": "2026-11-01",
            "routes": {"context": 4}
        });
        let rows = memory_stage_metrics("selected", &stage).expect("object yields rows");
        let labels = rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![
                "Retrievals",
                "With chunk",
                "Selections",
                "Distinct",
                "Reused",
                "Cut total",
                "Dropped",
                "Measured",
            ]
        );
        assert_eq!(rows[0], ("Retrievals".into(), "4".into(), false));
        assert_eq!(rows[5], ("Cut total".into(), "0".into(), true));
    }

    #[test]
    fn stage_metrics_format_booleans_and_preserve_unknown_keys() {
        let stage = json!({
            "total": 9,
            "open": 0,
            "truncated": true,
            "extra_flag": false
        });
        let rows = memory_stage_metrics("brain", &stage).expect("object yields rows");
        assert_eq!(
            rows,
            vec![
                ("Total".into(), "9".into(), false),
                ("Open".into(), "0".into(), true),
                ("Truncated".into(), "Yes".into(), false),
                ("Extra flag".into(), "No".into(), true),
            ]
        );
        assert!(memory_stage_metrics("brain", &json!(null)).is_none());
    }

    #[test]
    fn context_turns_model_includes_omnigraph_ledger_characters() {
        let usage = json!({
            "launches": [
                {
                    "id": "l1",
                    "kind": "native",
                    "started_at": "2026-11-17T09:00:00Z",
                    "context": {
                        "fill": { "start": 5000, "end": 6200, "window": 100000 },
                        "ledger": {
                            "capsule": { "kinds": { "brain": 360, "bank": 720, "rules": 180 } },
                            "excerpts": [],
                            "omnigraph": 1800
                        }
                    }
                },
                {
                    "id": "l2",
                    "kind": "native",
                    "started_at": "2026-11-17T09:05:00Z",
                    "context": {
                        "fill": { "start": 1000, "end": 1200, "window": 100000 },
                        "ledger": {
                            "capsule": { "kinds": { "brain": 360, "bank": 0, "rules": 0 } },
                            "excerpts": []
                        }
                    }
                }
            ]
        });
        let turns = context_turns_model(&usage);
        assert_eq!(turns.len(), 2);
        let t1 = &turns[0];
        assert!((t1["parts"]["brain"].as_f64().unwrap() - 100.0).abs() < 1e-6);
        assert!((t1["parts"]["bank"].as_f64().unwrap() - 200.0).abs() < 1e-6);
        assert!((t1["parts"]["rules"].as_f64().unwrap() - 50.0).abs() < 1e-6);
        assert!((t1["parts"]["omnigraph"].as_f64().unwrap() - 500.0).abs() < 1e-6);
        assert!((t1["memory"].as_f64().unwrap() - 850.0).abs() < 1e-6);
        assert!((t1["parts"]["rest"].as_f64().unwrap() - 4150.0).abs() < 1e-6);

        let t2 = &turns[1];
        assert!((t2["parts"]["omnigraph"].as_f64().unwrap() - 0.0).abs() < 1e-6);
        assert!((t2["memory"].as_f64().unwrap() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn linked_omnigraph_summary_renders_review_metrics_and_unverified_warning() {
        let disabled = json!({ "status": "awaiting_context", "omnigraph": { "enabled": false } });
        assert!(!render_linked_omnigraph_html(&disabled).0);

        let session = json!({
            "status": "awaiting_context",
            "omnigraph": {
                "enabled": true,
                "branch": "session/auth-1",
                "capsule_text": "## Omnigraph Shared Context\n- [unverified reference] Avoid raw tokens",
                "injected_atoms": [
                    { "id": "a1", "title": "Brain rule", "verified": true },
                    { "id": "a2", "title": "Draft note", "verified": false }
                ]
            }
        });
        let (show, html) = render_linked_omnigraph_html(&session);
        assert!(show);
        assert!(html.contains("session/auth-1"));
        assert!(html.contains("2 atom(s)"));
        assert!(html.contains("69 / 6,000 chars"));
        assert!(html.contains("1 verified"));
        assert!(html.contains("1 unverified reference"));
        assert!(html.contains("Included before run"));
        assert!(html.contains("Omnigraph capsule preview"));
    }

    #[test]
    fn knowledge_detail_action_renders_open_in_context_graph_button() {
        let brain_rec = json!({ "id": "rec-uuid-123", "title": "Auth decision" });
        let brain_html = render_detail_omnigraph_action_html("brain", "proj-1", &brain_rec);
        assert!(brain_html.contains("data-action=\"knowledge-open-omnigraph\""));
        assert!(brain_html.contains("data-project-id=\"proj-1\""));
        assert!(brain_html.contains("data-atom-id=\"brain-rec-uuid-123\""));
        assert!(brain_html.contains("Open in Context Graph"));

        let mem_chunk = json!({ "id": "MEM-20261117-abcd1234-auth", "title": "Token rules" });
        let mem_html = render_detail_omnigraph_action_html("memory", "proj-1", &mem_chunk);
        assert!(mem_html.contains("data-action=\"knowledge-open-omnigraph\""));
        assert!(mem_html.contains("data-project-id=\"proj-1\""));
        assert!(mem_html.contains("data-atom-id=\"mem-MEM-20261117-abcd1234-auth\""));

        let no_id = json!({ "title": "INDEX.md" });
        assert!(render_detail_omnigraph_action_html("memory", "proj-1", &no_id).is_empty());
    }

    #[test]
    fn resolve_initial_selection_prefers_jump_then_saved_then_default_active_record() {
        let entries = vec![
            json!({"id": "arch", "type": "task", "kind": "doc", "path": "archive/old.md"}),
            json!({"id": "t1", "type": "task", "kind": "chunk", "path": "dynamic/tasks/t1.md"}),
            json!({"id": "t2", "type": "finding", "kind": "chunk", "path": "dynamic/findings/f2.md"}),
        ];
        assert_eq!(
            super::resolve_initial_selection(
                "brain",
                &entries,
                "dynamic/findings/f2.md",
                Some("arch")
            ),
            Some("archive/old.md".to_owned())
        );
        assert_eq!(
            super::resolve_initial_selection("brain", &entries, "dynamic/findings/f2.md", None),
            Some("dynamic/findings/f2.md".to_owned())
        );
        assert_eq!(
            super::resolve_initial_selection("brain", &entries, "", None),
            Some("dynamic/tasks/t1.md".to_owned())
        );
        assert_eq!(
            super::resolve_initial_selection("memory", &entries, "", None),
            Some("dynamic/tasks/t1.md".to_owned())
        );
        assert_eq!(
            super::resolve_initial_selection("brain", &[], "", None),
            None
        );
    }

    #[test]
    fn context_turn_roving_states_and_detail_items_decouple_selection_and_preserve_tab_stop() {
        let shown = vec![
            json!({"id": "t1", "ordinal": 1}),
            json!({"id": "t2", "ordinal": 2}),
            json!({"id": "t3", "ordinal": 3}),
        ];
        // Selected turn on current page gets aria-checked=true and tabindex=0
        assert_eq!(
            super::context_turn_roving_states(&shown, &json!("t2")),
            vec![
                ("t1".into(), false, "-1"),
                ("t2".into(), true, "0"),
                ("t3".into(), false, "-1"),
            ]
        );
        // When selected turn is on another page, last shown turn remains tab-reachable
        assert_eq!(
            super::context_turn_roving_states(&shown, &json!("t99")),
            vec![
                ("t1".into(), false, "-1"),
                ("t2".into(), false, "-1"),
                ("t3".into(), false, "0"),
            ]
        );

        let turn = json!({
            "id": "t2",
            "ledger": {"chars": 100},
            "hooks": null,
            "cli_files": ["a.rs"],
            "compactions": []
        });
        let details = super::build_usage_detail_items(&turn, &shown);
        let keys: Vec<String> = details
            .iter()
            .map(|d| d["key"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            keys,
            vec!["ledger", "cli_files", "compactions", "turn-table"]
        );
    }

    #[test]
    fn memory_chunk_and_trace_items_key_stably_and_table_sort_reports_aria_sort() {
        let chunks = vec![
            json!({"id": "c1", "status": "active", "review_after": "2026-10-01", "sources_changed": false}),
            json!({"id": "c2", "status": "archived", "review_after": "2026-12-01", "sources_changed": true}),
            json!({"id": "", "path": "memory-bank/c3.md", "status": "active", "review_after": "2026-12-01", "sources_changed": true}),
        ];
        let past = super::build_memory_chunk_items(&chunks, "2026-11-01", "past");
        assert_eq!(past.len(), 1);
        assert_eq!(past[0]["key"], "chunk:c1");

        let changed = super::build_memory_chunk_items(&chunks, "2026-11-01", "changed");
        assert_eq!(changed.len(), 2);
        assert_eq!(changed[0]["key"], "chunk:c2");
        assert_eq!(changed[1]["key"], "chunk:memory-bank/c3.md");

        let traces = vec![
            json!({"manifest_id": "m1", "at": "2026-11-17T10:00:00Z"}),
            json!({"manifest_id": "m1", "at": "2026-11-17T10:01:00Z"}),
        ];
        let feedback = json!({"m1": {"c1": {"vote": "helped"}}});
        let trace_items = super::build_memory_trace_items(&traces, &feedback, true);
        assert_eq!(trace_items[0]["key"], "trace:m1");
        assert_eq!(trace_items[1]["key"], "trace:m1#1");

        assert_eq!(
            super::memory_table_sort_dir(&json!({}), "id", "id"),
            Some("ascending")
        );
        assert_eq!(
            super::memory_table_sort_dir(&json!({"field": "bytes", "desc": true}), "id", "bytes"),
            Some("descending")
        );
        assert_eq!(
            super::memory_table_sort_dir(&json!({"field": "bytes", "desc": true}), "id", "id"),
            None
        );
    }
}
