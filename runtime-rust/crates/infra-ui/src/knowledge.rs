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
    dom::text(&format!("{scope}-knowledge-note"),match action.as_str(){"bank-reverify"=>"By running this action you attest that you checked this chunk against its current sources.","brain-update"=>"Revision must match the current record; refresh if it changed.","complete"=>"Record the task outcome and verification evidence.","reindex-bank"=>"This regenerates the shared INDEX.md.","compact"=>"Archive eligible terminal records.","export"=>"Create a downloadable knowledge ZIP.",_=>"Run the selected operation against the selected knowledge root."});
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
    dom::text(&format!("{scope}-message"), "Choose a record to read.");
    dom::hidden(&format!("{scope}-message"), false);
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
            let selected =
                if !bank_changed && old["project_id"] == p && old["bank_id"] == v["bank_id"] {
                    m::text(&old, "selected_path")
                } else {
                    String::new()
                };
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
                for e in m::array(&v["entries"]) {
                    cats.insert(category(&e));
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
            let entries = m::array(&v["entries"]);
            let selected = entries
                .iter()
                .find(|x| x["path"] == selected)
                .or_else(|| {
                    entries.iter().find(|x| {
                        if scope == "memory" {
                            x["kind"] == "chunk"
                        } else {
                            x["type"] == "task" && !m::text(x, "path").starts_with("archive/")
                        }
                    })
                })
                .or_else(|| entries.first());
            if let Some(v) = selected {
                select(scope, &m::text(v, "path")).await;
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
        let b = dom::button(
            &format!(
                "{} · {} · {} · {}",
                m::summary(v),
                p,
                m::text(v, "status"),
                bytes(&v["bytes"])
            ),
            "knowledge-file",
            json!({"scope":scope,"path":p}),
        );
        b.set_class_name("memory-entry");
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
    render_list(scope);
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
}
pub fn render_linked(session: &V) {
    let brain = &session["brain"];
    dom::hidden("linked-context", !brain.is_object());
    if !brain.is_object() {
        return;
    }
    let waiting = session["status"] == "awaiting_context";
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
    let meter = session["capsule_delivery_meter"]
        .as_object()
        .map(|_| &session["capsule_delivery_meter"])
        .unwrap_or(&session["capsule_meter"]);
    dom::hidden("capsule-meter", !meter.is_object());
    dom::text(
        "capsule-facts",
        &format!(
            "{} of {} characters",
            m::string(&meter["characters"]),
            m::string(&meter["limit"])
        ),
    );
    dom::pretty("capsule-legend", &meter["kinds"]);
    dom::text(
        "capsule-repeats",
        &format!("{} characters repeat", m::string(&meter["repeats"])),
    );
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
fn render_memory() {
    let v = m::data("memory-use");
    let window = m::data("memory-window").as_u64().unwrap_or(30);
    let model = memory_model(&v, window);
    m::set("memory-use-model", model.clone());
    dom::clear("memory-use-body");
    let flow = dom::el("ol", "memory-flow", "");
    for (stage, title) in [
        ("brain", "Project Brain"),
        ("promotion", "Promotion"),
        ("bank", "Memory bank"),
        ("selected", "Selected by retrieval"),
    ] {
        let card = dom::el("li", "memory-stage", "");
        let _ = card.set_attribute("data-stage", stage);
        dom::add(&card, &dom::el("h3", "", title));
        if let Some(map) = model[stage].as_object() {
            for (k, value) in map {
                if value.is_number() || value.is_boolean() {
                    dom::add(
                        &card,
                        &dom::el(
                            "p",
                            "memory-lead",
                            &format!("{}: {}", m::label(k), m::string(value)),
                        ),
                    );
                }
            }
        } else {
            dom::add(
                &card,
                &dom::el("p", "knowledge-note", "Not recorded for this memory root."),
            );
        }
        dom::add(&flow, &card);
    }
    dom::append("memory-use-body", &flow);
    if m::yes(&v["check_available"]) {
        dom::append(
            "memory-use-body",
            &dom::button("Check current memory state", "memory-use-check", V::Null),
        );
    }
    if !m::data("memory-use-check").is_null() {
        dom::append(
            "memory-use-body",
            &dom::details(
                "Current check",
                &serde_json::to_string_pretty(&m::data("memory-use-check")).unwrap_or_default(),
            ),
        );
    }
    let anchor_key = format!(
        "harness.memory-use.v1:{}:{}",
        dom::value("memory-use-project"),
        m::text(&v, "bank_id")
    );
    let before = dom::json_storage(&anchor_key);
    let mut added = 0;
    let chunks = m::array(&v["chunks"]["items"]);
    for c in &chunks {
        if before["chunks"][m::text(c, "id")].is_null() {
            added += 1;
        }
    }
    dom::text(
        "memory-use-since",
        &if before["at"].is_string() {
            format!("{added} chunks added since {}", m::text(&before, "at"))
        } else {
            "First visit to this knowledge root".into()
        },
    );
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
    dom::set_element_value(&filter, m::data("memory-filter").as_str().unwrap_or("all"));
    dom::append("memory-use-body", &filter);
    let mode = m::data("memory-filter");
    for c in chunks {
        if mode == "past"
            && !c["review_after"]
                .as_str()
                .map(|s| s < v["today"].as_str().unwrap_or(""))
                .unwrap_or(false)
            || mode == "changed" && c["sources_changed"] != true
            || mode == "active" && c["status"] != "active"
            || mode == "retired"
                && !["superseded", "archived"].contains(&c["status"].as_str().unwrap_or(""))
        {
            continue;
        }
        let row = dom::details(
            &format!(
                "{} · {} · review {}",
                m::summary(&c),
                m::text(&c, "status"),
                m::text(&c, "review_after")
            ),
            &serde_json::to_string_pretty(&c).unwrap_or_default(),
        );
        dom::add(
            &row,
            &dom::button("Open chunk", "memory-chunk-open", c.clone()),
        );
        dom::append("memory-use-body", &row);
    }
    analytics(&v, &model);
    let history = dom::details(
        "Daily retrieval history",
        &serde_json::to_string_pretty(&v["history"]).unwrap_or_default(),
    );
    dom::append("memory-use-body", &history);
    for row in m::array(&v["retrievals"]["items"]) {
        dom::append(
            "memory-use-body",
            &dom::details(
                &format!(
                    "{} · {} · {} chunks",
                    m::text(&row, "at"),
                    m::text(&row, "route"),
                    m::array(&row["chunks"]).len()
                ),
                &serde_json::to_string_pretty(&row).unwrap_or_default(),
            ),
        );
    }
    memory_changes(&v, &before);
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
pub fn usage(v: &V) {
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
        let memory = brain + bank + rules;
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
        let t = json!({"id":launch["id"],"ordinal":turns.len()+1,"at":launch["started_at"],"start":fill["start"],"end":fill["end"],"window":fill["window"],"peak":fill["peak"],"calls":fill["calls"],"cache":fill["cache_share"],"recorded":ledger.is_object(),"memory":memory,"parts":{"brain":brain,"bank":bank,"rules":rules,"rest":rest,"added":added,"free":free},"earlier_memory_upper_bound":if prior_unknown{V::Null}else{json!(earlier)},"compactions":fill["compactions"],"ledger":ledger,"hooks":context["hooks"],"cli_files":context["cli_files"]});
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
    m::set("context-turns", json!(turns));
    render_usage();
}
fn render_usage() {
    let turns = m::array(&m::data("context-turns"));
    dom::hidden("context-usage", turns.is_empty());
    if turns.is_empty() {
        return;
    }
    let selected = m::data("context-turn-selected");
    let turn = turns
        .iter()
        .find(|v| v["id"] == selected)
        .or_else(|| turns.last())
        .cloned()
        .unwrap_or(V::Null);
    dom::clear("context-usage");
    dom::append("context-usage", &dom::el("h3", "", "Context"));
    let headline =
        if let (Some(end), Some(window)) = (m::number(&turn["end"]), m::number(&turn["window"])) {
            format!(
                "{end:.0} of {window:.0} tokens · {:.0}%",
                end / window * 100.
            )
        } else {
            "Context size not reported by this provider".into()
        };
    dom::append(
        "context-usage",
        &dom::el("p", "context-headline", &headline),
    );
    dom::append("context-usage",&dom::el("p","knowledge-note",&format!("Memory sent this turn ≈ {:.0} tokens. Provider fill is exact; memory shares are character-based estimates.",turn["memory"].as_f64().unwrap_or(0.))));
    let bar = dom::el("div", "context-bar", "");
    let _ = bar.set_attribute("role", "img");
    for kind in ["brain", "bank", "rules", "rest", "added"] {
        let compacted = !m::array(&turn["compactions"]).is_empty();
        let value = if compacted {
            if kind == "added" {
                m::number(&turn["end"]).unwrap_or(0.)
            } else {
                0.
            }
        } else if turn["parts"]["rest"].is_null() && ["brain", "bank", "rules"].contains(&kind) {
            0.
        } else {
            m::number(&turn["parts"][kind]).unwrap_or(0.)
        };
        let window = m::number(&turn["window"]).unwrap_or(0.);
        if value <= 0. || window <= 0. {
            continue;
        }
        let part = dom::el("span", "context-part", "");
        let _ = part.set_attribute("data-kind", kind);
        let _ = part.set_attribute("title", &format!("{} ≈ {value:.0} tokens", m::label(kind)));
        if let Some(e) = part.dyn_ref::<web_sys::HtmlElement>() {
            let _ = e
                .style()
                .set_property("width", &format!("{}%", (value / window * 100.).max(0.33)));
        }
        dom::add(&bar, &part);
    }
    dom::append("context-usage", &bar);
    let legend = dom::el("table", "context-legend", "");
    for kind in ["brain", "bank", "rules", "rest", "added", "free"] {
        let row = dom::el("tr", "", "");
        dom::add(&row, &dom::el("th", "", &m::label(kind)));
        dom::add(
            &row,
            &dom::el(
                "td",
                "",
                &m::number(&turn["parts"][kind])
                    .map(|n| {
                        format!(
                            "{} {n:.0}",
                            if ["brain", "bank", "rules", "rest"].contains(&kind) {
                                "≈"
                            } else {
                                ""
                            }
                        )
                    })
                    .unwrap_or_else(|| "—".into()),
            ),
        );
        dom::add(&legend, &row);
    }
    dom::append("context-usage", &legend);
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
    if end > size {
        dom::append(
            "context-usage",
            &dom::button("‹ Earlier", "context-page", json!(page + 1)),
        );
    }
    if page > 0 {
        dom::append(
            "context-usage",
            &dom::button("Newer ›", "context-page", json!(page - 1)),
        );
    }
    let strip = dom::el("div", "context-turns", "");
    let _ = strip.set_attribute("id", "context-turns");
    let _ = strip.set_attribute("role", "radiogroup");
    for t in shown {
        let b = dom::button(
            &format!("Turn {}", m::string(&t["ordinal"])),
            "context-turn",
            t["id"].clone(),
        );
        let _ = b.set_attribute("role", "radio");
        let _ = b.set_attribute(
            "aria-checked",
            if t["id"] == turn["id"] {
                "true"
            } else {
                "false"
            },
        );
        dom::add(&strip, &b);
    }
    dom::append("context-usage", &strip);
    dom::append(
        "context-usage",
        &dom::button("Latest", "context-latest", V::Null),
    );
    for (field, title) in [
        ("ledger", "Exact delivered context ledger"),
        ("hooks", "Hook context"),
        ("cli_files", "CLI files"),
        ("compactions", "Compactions"),
    ] {
        if !turn[field].is_null() {
            dom::append(
                "context-usage",
                &dom::details(
                    title,
                    &serde_json::to_string_pretty(&turn[field]).unwrap_or_default(),
                ),
            );
        }
    }
    let all = dom::details("Turn table", "");
    for t in turns {
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
    dom::append("context-usage", &all);
}
pub async fn event(kind: &str, id: &str, action: &str, payload: &V, value: &str) -> bool {
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
            render_usage();
        }
        "context-meter" => session::open_view("usage", true).await,
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
    let section = dom::el("section", "memory-section memory-strip-section", "");
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
    dom::append("memory-use-body", &section);
    if !raw.is_empty() {
        retrieval_detail(
            m::data("memory-pinned-retrieval")
                .as_u64()
                .map(|n| n as usize)
                .unwrap_or(raw.len() - 1),
        );
    }
    let horizon = dom::el("section", "memory-section memory-horizon-section", "");
    dom::add(&horizon, &dom::el("h3", "", "Review horizon"));
    let chips = dom::el("div", "memory-chips", "");
    let mut chunks = m::array(&data["chunks"]["items"])
        .into_iter()
        .filter(|c| c["status"] == "active" && c["review_after"].is_string())
        .collect::<Vec<_>>();
    chunks.sort_by_key(|c| (m::text(c, "review_after"), m::text(c, "id")));
    let filters = m::array(&m::data("memory-filters"));
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
    dom::append("memory-use-body", &horizon);
    if let Some(chunk) = chunks
        .iter()
        .find(|c| c["id"] == m::data("memory-selected-chunk"))
    {
        chunk_detail(chunk);
    }
    let history = dom::el("div", "memory-history-chart", "");
    let days = m::array(&data["history"]["days"]);
    let most = days
        .iter()
        .filter_map(|d| d["retrievals"].as_u64())
        .max()
        .unwrap_or(1)
        .max(1);
    for day in days {
        let column = dom::el("div", "memory-history-day", "");
        let _ = column.set_attribute(
            "title",
            &format!(
                "{} · {} retrievals",
                m::text(&day, "day"),
                m::string(&day["retrievals"])
            ),
        );
        style(
            &column,
            "--height",
            &format!(
                "{}%",
                day["retrievals"].as_u64().unwrap_or(0) as f64 / most as f64 * 100.
            ),
        );
        dom::add(&history, &column);
    }
    dom::append("memory-use-body", &history);
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
    let details = dom::el("details", "fleet-report", "");
    dom::add(
        &details,
        &dom::el(
            "summary",
            "",
            &format!("{} table ({})", m::label(kind), values.len()),
        ),
    );
    let table = dom::el("table", "memory-table", "");
    let head = dom::el("tr", "", "");
    for (field, label) in columns {
        let th = dom::el("th", "", "");
        let _ = th.set_attribute("scope", "col");
        dom::add(
            &th,
            &dom::button(label, "memory-sort", json!({"kind":kind,"field":field})),
        );
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
    dom::add(&details, &table);
    dom::append("memory-use-body", &details);
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
    dom::text(
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
