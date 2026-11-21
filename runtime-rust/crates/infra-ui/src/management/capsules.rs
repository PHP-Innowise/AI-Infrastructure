//! The Skill capsules view: the capsules of the current project, and one
//! capsule as a person inspects it: what it is for (μ), what it needs (B),
//! what it runs (P), in which order (π) and how it is checked (V), with its
//! gate history, its compilation and how its warm turns went.
//!
//! Program sources and observed values are never shown: the server does not
//! send them. What a reuse saves is not known here and is shown as unknown.
use crate::{api, dom, model as m};
use dom::escape_html as esc;
use serde_json::{json, Value as V};

pub(crate) fn status_label(status: &str) -> &'static str {
    match status {
        "active" => "Active",
        "compiling" => "Compiling",
        "stale" => "Stale",
        "needs_recompile" => "Needs recompile",
        "failed" => "Failed",
        "retired" => "Retired",
        _ => "Unknown",
    }
}
pub(crate) fn status_tone(status: &str) -> &'static str {
    match status {
        "active" => "ok",
        "stale" | "needs_recompile" => "warn",
        "failed" => "bad",
        _ => "muted",
    }
}
pub(crate) fn form_label(form: &str) -> &'static str {
    match form {
        "hybrid" => "Hybrid",
        "textual" => "Written procedure",
        _ => "",
    }
}
/// A capsule's name: what it was compiled into, or the skill it is being
/// compiled from.
pub(crate) fn capsule_title(capsule: &V) -> String {
    capsule["mu"]["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let skill = m::text(capsule, "skill_name");
            if skill.is_empty() {
                "Capsule".into()
            } else {
                format!("From skill `{skill}`")
            }
        })
}
/// An amount of money, or a dash when it is not known. An unknown amount is
/// never shown as zero.
pub(crate) fn money(value: &V) -> String {
    match value.as_f64() {
        Some(amount) => format!("${amount:.4}"),
        None => "—".into(),
    }
}
fn count(value: &V) -> String {
    value
        .as_i64()
        .map_or_else(|| "—".into(), |count| count.to_string())
}
fn chip(label: &str, tone: &str) -> String {
    if label.is_empty() {
        return String::new();
    }
    format!(
        "<span class=\"tool-chip skill-capsule-chip\" data-tone=\"{tone}\">{}</span>",
        esc(label)
    )
}
fn names(value: &V) -> String {
    let names = m::array(value)
        .iter()
        .filter_map(|name| name.as_str().map(esc))
        .collect::<Vec<_>>();
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}
fn button(label: &str, action: &str, payload: &V, class: &str, disabled: bool) -> String {
    format!(
        "<button type=\"button\" class=\"button compact{}\" data-action=\"{action}\" data-payload=\"{}\"{}>{}</button>",
        if class.is_empty() {
            String::new()
        } else {
            format!(" {class}")
        },
        esc(&payload.to_string()),
        if disabled { " disabled" } else { "" },
        esc(label)
    )
}

pub(crate) fn render_capsule_row_html(capsule: &V, selected: bool) -> String {
    let status = capsule["status"].as_str().unwrap_or("");
    let reason = m::text(capsule, "reason");
    format!(
        "<button type=\"button\" class=\"skill-capsule-row\" data-action=\"skill-capsule-open\" data-payload=\"{}\" aria-current=\"{}\"><span class=\"skill-capsule-name\">{}</span><span class=\"installed-tools\">{}{}</span><span class=\"skills-installed-path\">{}</span>{}</button>",
        esc(&capsule["capsule_id"].to_string()),
        if selected { "true" } else { "false" },
        esc(&capsule_title(capsule)),
        chip(status_label(status), status_tone(status)),
        chip(form_label(capsule["form"].as_str().unwrap_or("")), "muted"),
        esc(&m::text(capsule, "skill_path")),
        if reason.is_empty() || status == "active" {
            String::new()
        } else {
            format!("<span class=\"skill-capsule-reason\">{}</span>", esc(&reason))
        },
    )
}

fn section(title: &str, body: &str) -> String {
    if body.is_empty() {
        return String::new();
    }
    format!("<section class=\"skill-capsule-section\"><h4>{title}</h4>{body}</section>")
}

fn step_html(step: &V) -> String {
    let source = &step["source"];
    let from = if source.is_object() {
        format!(
            " <span class=\"skill-capsule-source\">{}{}</span>",
            esc(&m::text(source, "path")),
            source["section"]
                .as_str()
                .map(|section| format!(" § {}", esc(section)))
                .unwrap_or_default()
        )
    } else {
        String::new()
    };
    match step["kind"].as_str().unwrap_or("") {
        "program" => format!(
            "<li data-kind=\"program\"><span class=\"skill-capsule-kind\">Program</span> <code>{}</code></li>",
            esc(&m::text(step, "program"))
        ),
        "inspection" => format!(
            "<li data-kind=\"inspection\"><span class=\"skill-capsule-kind\">Agent inspects</span> {}{from}</li>",
            esc(&m::text(step, "text"))
        ),
        _ => format!(
            "<li data-kind=\"instruction\"><span class=\"skill-capsule-kind\">Agent</span> {}{}{from}</li>",
            esc(&m::text(step, "text")),
            if m::array(&step["produces"]).is_empty() {
                String::new()
            } else {
                format!(
                    " <span class=\"skill-capsule-produces\">produces {}</span>",
                    names(&step["produces"])
                )
            }
        ),
    }
}

/// The plan as a person inspects it.
fn plan_html(capsule: &V) -> String {
    let mut out = String::new();
    let bindings = m::array(&capsule["bindings"]);
    if !bindings.is_empty() {
        let rows = bindings
            .iter()
            .map(|binding| {
                format!(
                    "<li><code>{}</code> <span class=\"skill-capsule-kind\">{}</span> {}</li>",
                    esc(&m::text(binding, "name")),
                    esc(&m::text(binding, "type")),
                    esc(&m::text(binding, "description"))
                )
            })
            .collect::<String>();
        out.push_str(&section(
            "Values it needs from the request (B)",
            &format!("<ul class=\"skill-capsule-list\">{rows}</ul>"),
        ));
    }
    let programs = m::array(&capsule["programs"]);
    if !programs.is_empty() {
        let rows = programs
            .iter()
            .map(|program| {
                let entry = m::array(&program["entry"])
                    .iter()
                    .filter_map(|word| word.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!(
                    "<li><code>{}</code> {}<span class=\"skill-capsule-facts\">runs <code>{}</code> · reads {} · gives {} · up to {} s{}</span></li>",
                    esc(&m::text(program, "id")),
                    esc(&m::text(program, "description")),
                    esc(&entry),
                    names(&program["inputs"]),
                    names(&program["outputs"]),
                    count(&program["timeout_seconds"]),
                    if m::yes(&program["read_only"]) {
                        " · read-only"
                    } else {
                        ""
                    }
                )
            })
            .collect::<String>();
        out.push_str(&section(
            "Programs (P)",
            &format!("<ul class=\"skill-capsule-list\">{rows}</ul>"),
        ));
    }
    for (title, field) in [("Plan (π)", "steps"), ("Verification (V)", "verification")] {
        let steps = m::array(&capsule[field]);
        if !steps.is_empty() {
            out.push_str(&section(
                title,
                &format!(
                    "<ol class=\"skill-capsule-steps\">{}</ol>",
                    steps.iter().map(step_html).collect::<String>()
                ),
            ));
        }
    }
    out
}

fn gates_html(gates: &[V]) -> String {
    if gates.is_empty() {
        return String::new();
    }
    let rows = gates
        .iter()
        .map(|gate| {
            let passed = m::yes(&gate["passed"]);
            let violations = m::array(&gate["report"]["violations"]);
            let details = violations
                .iter()
                .take(12)
                .map(|violation| {
                    format!(
                        "<li><code>{}</code> {}</li>",
                        esc(&m::text(violation, "code")),
                        esc(&m::text(violation, "message"))
                    )
                })
                .collect::<String>();
            format!(
                "<li data-passed=\"{}\"><span class=\"skill-capsule-kind\">Round {} · {}</span> {}{}{}</li>",
                if passed { "true" } else { "false" },
                count(&gate["round"]),
                esc(&m::text(gate, "gate")),
                if passed { "passed" } else { "failed" },
                if violations.len() > 12 {
                    format!(" ({} findings, 12 shown)", violations.len())
                } else {
                    String::new()
                },
                if details.is_empty() {
                    String::new()
                } else {
                    format!("<ul class=\"skill-capsule-violations\">{details}</ul>")
                }
            )
        })
        .collect::<String>();
    section(
        "Gate history",
        &format!("<ul class=\"skill-capsule-list skill-capsule-gates\">{rows}</ul>"),
    )
}

fn runs_html(detail: &V) -> String {
    let runs = &detail["runs"];
    let amortization = &detail["amortization"];
    let recent = m::array(&runs["recent"])
        .iter()
        .take(8)
        .map(|run| {
            let handed = run["deopts"].as_i64().unwrap_or_default();
            format!(
                "<li>{} · {}{} <span class=\"skill-capsule-source\">{} {}</span></li>",
                esc(&m::text(run, "created_at")),
                esc(&m::text(run, "outcome")),
                if handed > 0 {
                    format!(" · {handed} step(s) handed back")
                } else {
                    String::new()
                },
                esc(&m::text(run, "provider")),
                esc(&m::text(run, "model")),
            )
        })
        .collect::<String>();
    let body = format!(
        "<dl class=\"skill-capsule-facts-grid\"><dt>Warm turns</dt><dd>{}</dd><dt>Completed</dt><dd>{}</dd><dt>Handed a step back</dt><dd>{}</dd><dt>Compilation cost</dt><dd>{}</dd><dt>Saving per reuse</dt><dd>{}</dd><dt>Break-even</dt><dd>{}</dd></dl><p class=\"skills-note\">A saving is measured by a sealed evaluation (<code>infra-engine eval skillcache-ab</code>), not read off these turns. Until one exists it is unknown.</p>{}",
        count(&runs["count"]),
        count(&runs["complete"]),
        count(&runs["handed_back"]),
        money(&amortization["compile_usd"]),
        money(&amortization["saving_per_reuse_usd"]),
        count(&amortization["breakeven_reuses"]),
        if recent.is_empty() {
            String::new()
        } else {
            format!("<ul class=\"skill-capsule-list\">{recent}</ul>")
        }
    );
    section("Warm turns and cost", &body)
}

fn compile_html(compile: &V) -> String {
    if !compile.is_object() {
        return String::new();
    }
    let usage = &compile["usage"];
    let tokens = [
        ("input_tokens", "input"),
        ("output_tokens", "output"),
        ("cache_read_input_tokens", "cache read"),
        ("cache_creation_input_tokens", "cache write"),
    ]
    .iter()
    .map(|(field, label)| format!("{} {label}", count(&usage[*field])))
    .collect::<Vec<_>>()
    .join(" · ");
    section(
        "Compilation",
        &format!(
            "<dl class=\"skill-capsule-facts-grid\"><dt>Status</dt><dd>{}</dd><dt>Rounds</dt><dd>{}</dd><dt>Compiler</dt><dd>{} {}</dd><dt>Tokens</dt><dd>{}</dd><dt>Cost</dt><dd>{}</dd></dl><p class=\"skills-note\">{}</p>",
            esc(&m::text(compile, "status")),
            count(&compile["rounds"]),
            esc(&m::text(compile, "provider")),
            esc(&m::text(compile, "model")),
            esc(&tokens),
            money(&usage["cost_usd"]),
            esc(&m::text(compile, "outcome")),
        ),
    )
}

/// One capsule: its identity and status, what can be done with it, its plan
/// and its history. `armed` is a delete waiting for its confirmation;
/// `pending` is an action in flight.
pub(crate) fn render_capsule_detail_html(detail: &V, armed: bool, pending: bool) -> String {
    let id = &detail["capsule_id"];
    let status = detail["status"].as_str().unwrap_or("");
    let capsule = &detail["capsule"];
    let mut actions = String::new();
    if ["active", "stale", "needs_recompile", "retired"].contains(&status) {
        actions.push_str(&button(
            if status == "retired" {
                "Use again"
            } else {
                "Requalify"
            },
            "skill-capsule-requalify",
            id,
            "",
            pending,
        ));
    }
    if ["active", "stale", "needs_recompile"].contains(&status) {
        actions.push_str(&button(
            "Stop using",
            "skill-capsule-retire",
            id,
            "",
            pending,
        ));
    }
    if status != "compiling" {
        if armed {
            actions.push_str(&button(
                "Delete this capsule",
                "skill-capsule-delete",
                id,
                "danger",
                pending,
            ));
            actions.push_str(&button("Keep it", "skill-capsule-keep", id, "", pending));
        } else {
            actions.push_str(&button("Delete…", "skill-capsule-arm", id, "", pending));
        }
    }
    let reason = m::text(detail, "reason");
    let description = m::text(&detail["mu"], "description");
    let capabilities = &capsule["capabilities"];
    let skills = m::array(&detail["skill_files"]["skills"])
        .iter()
        .map(|skill| {
            format!(
                "<li><code>{}</code> <span class=\"skill-capsule-source\">{} · {} files</span></li>",
                esc(&m::text(skill, "name")),
                esc(&m::text(skill, "path")),
                count(&skill["files"])
            )
        })
        .collect::<String>();
    let files = m::array(&detail["files"])
        .iter()
        .map(|file| {
            format!(
                "<li><code>{}</code> <span class=\"skill-capsule-source\">{} bytes</span></li>",
                esc(&m::text(file, "path")),
                count(&file["bytes"])
            )
        })
        .collect::<String>();
    let mut out = format!(
        "<header class=\"skill-capsule-head\"><h3>{}</h3><p class=\"installed-tools\">{}{}</p>{}{}<div class=\"skill-capsule-actions\">{}</div></header>",
        esc(&capsule_title(detail)),
        chip(status_label(status), status_tone(status)),
        chip(form_label(detail["form"].as_str().unwrap_or("")), "muted"),
        if description.is_empty() {
            String::new()
        } else {
            format!("<p class=\"skill-capsule-description\">{}</p>", esc(&description))
        },
        if reason.is_empty() {
            String::new()
        } else {
            format!(
                "<p class=\"skill-capsule-reason\" role=\"status\">{}</p>",
                esc(&reason)
            )
        },
        actions,
    );
    if capabilities.is_object() {
        out.push_str(&section(
            "What it needs",
            &format!(
                "<dl class=\"skill-capsule-facts-grid\"><dt>Network</dt><dd>{}</dd><dt>Runtimes</dt><dd>{}</dd></dl>",
                esc(&m::text(capabilities, "network")),
                names(&capabilities["runtimes"])
            ),
        ));
    }
    out.push_str(&plan_html(capsule));
    if let Some(markdown) = detail["markdown"].as_str().filter(|text| !text.is_empty()) {
        out.push_str(&section(
            "Written procedure",
            &format!(
                "<pre class=\"create-skill-preview-content\">{}</pre>",
                esc(markdown)
            ),
        ));
    }
    if !skills.is_empty() {
        out.push_str(&section(
            "Compiled from",
            &format!("<ul class=\"skill-capsule-list\">{skills}</ul>"),
        ));
    }
    out.push_str(&runs_html(detail));
    out.push_str(&compile_html(&detail["compile"]));
    out.push_str(&gates_html(&m::array(&detail["gates"])));
    if !files.is_empty() {
        out.push_str(&section(
            "Package files",
            &format!(
                "<ul class=\"skill-capsule-list\">{files}</ul><p class=\"skills-note\">The package is read by the plan executor, not by the agent. Its contents are not shown here.</p>"
            ),
        ));
    }
    out
}

fn list_query(status: &str, form: &str) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if status.is_empty() {
        // The server excludes retired capsules by default; "All" shows them
        // so retiring the open capsule keeps it visible with "Use again".
        query.push(("include_retired", "true".to_owned()));
    } else {
        query.push(("status", status.to_owned()));
    }
    if !form.is_empty() {
        query.push(("form", form.to_owned()));
    }
    query
}

fn list_path(project: &str) -> String {
    let status = dom::value("skill-capsules-status");
    let form = dom::value("skill-capsules-form");
    let query = list_query(&status, &form);
    let refs = query
        .iter()
        .map(|(k, v)| (*k, v.clone()))
        .collect::<Vec<_>>();
    format!(
        "/api/projects/{}/skillcache/capsules{}",
        dom::enc(project),
        dom::query(&refs)
    )
}

fn render_list() {
    let listed = m::data("skill-capsules");
    let capsules = m::array(&listed["capsules"]);
    let open = m::string(&m::data("skill-capsule-open"));
    if let Some(list) = dom::get("skill-capsules-list") {
        crate::skillcache::set_markup(
            &list,
            &if capsules.is_empty() {
                "<p class=\"empty-note\">No capsule yet. Turn SkillCache to Observe in a session, run a turn that uses a skill, and compile it from the run's SkillCache tab.</p>".to_owned()
            } else {
                capsules
                    .iter()
                    .map(|capsule| {
                        render_capsule_row_html(capsule, m::text(capsule, "capsule_id") == open)
                    })
                    .collect::<String>()
            },
        );
    }
    dom::text(
        "skill-capsules-status-note",
        &match capsules.len() {
            0 => String::new(),
            1 => "1 capsule".into(),
            count => format!("{count} capsules"),
        },
    );
}

fn render_detail() {
    let detail = m::data("skill-capsule-detail");
    let open = m::string(&m::data("skill-capsule-open"));
    let Some(pane) = dom::get("skill-capsule-detail") else {
        return;
    };
    if !detail.is_object() || m::text(&detail, "capsule_id") != open {
        crate::skillcache::set_markup(&pane, "");
        dom::hide(&pane, true);
        return;
    }
    crate::skillcache::set_markup(
        &pane,
        &render_capsule_detail_html(
            &detail,
            m::data("skill-capsule-armed") == open.as_str(),
            m::yes(&m::data("skill-capsule-pending")),
        ),
    );
    dom::hide(&pane, false);
}

/// Loads the project's capsules and, when one is open, that capsule.
pub async fn load_capsules() {
    let project = m::current_project();
    if project.is_empty() {
        dom::text(
            "skill-capsules-status-note",
            "Choose a project to see its capsules.",
        );
        return;
    }
    match api::scoped("skill-capsules", &list_path(&project)).await {
        Ok((epoch, listed))
            if m::current("skill-capsules", epoch) && m::current_project() == project =>
        {
            dom::error("skill-capsules-error", "");
            // A capsule of another project, or one that was deleted, closes.
            let open = m::string(&m::data("skill-capsule-open"));
            let filtered = !dom::value("skill-capsules-status").is_empty()
                || !dom::value("skill-capsules-form").is_empty();
            if !open.is_empty()
                && !filtered
                && !m::array(&listed["capsules"])
                    .iter()
                    .any(|capsule| m::text(capsule, "capsule_id") == open)
            {
                m::take("skill-capsule-open");
                m::take("skill-capsule-detail");
            }
            m::set("skill-capsules", listed);
            render_list();
            let open = m::string(&m::data("skill-capsule-open"));
            if open.is_empty() {
                render_detail();
            } else {
                load_capsule(&open).await;
            }
        }
        Ok(_) => {}
        Err(error) if !error.aborted => dom::error("skill-capsules-error", &error.message),
        Err(_) => {}
    }
}

async fn load_capsule(id: &str) {
    let project = m::current_project();
    let path = format!(
        "/api/projects/{}/skillcache/capsules/{}",
        dom::enc(&project),
        dom::enc(id)
    );
    match api::scoped("skill-capsule-detail", &path).await {
        Ok((epoch, detail))
            if m::current("skill-capsule-detail", epoch) && m::data("skill-capsule-open") == id =>
        {
            dom::error("skill-capsules-error", "");
            m::set("skill-capsule-detail", detail);
            render_list();
            render_detail();
        }
        Ok(_) => {}
        Err(error) if !error.aborted => {
            dom::error("skill-capsules-error", &error.message);
            if error.status == 404 {
                m::take("skill-capsule-open");
                m::take("skill-capsule-detail");
                render_list();
                render_detail();
            }
        }
        Err(_) => {}
    }
}

/// A capsule of this project changed: the list, and that capsule when it is
/// the open one, are read again.
pub async fn capsule_changed(_capsule_id: &str) {
    load_capsules().await;
}

async fn act(action: &str, id: &str) {
    let project = m::current_project();
    let base = format!(
        "/api/projects/{}/skillcache/capsules/{}",
        dom::enc(&project),
        dom::enc(id)
    );
    m::set("skill-capsule-pending", json!(true));
    m::take("skill-capsule-armed");
    render_detail();
    let result = match action {
        "delete" => api::delete(&base).await,
        _ => api::post(&format!("{base}/{action}"), json!({})).await,
    };
    m::take("skill-capsule-pending");
    match result {
        Ok(_) => {
            dom::error("skill-capsules-error", "");
            if action == "delete" {
                m::take("skill-capsule-open");
                m::take("skill-capsule-detail");
            }
        }
        Err(error) => dom::error("skill-capsules-error", &error.message),
    }
    load_capsules().await;
}

pub(super) async fn capsules_event(kind: &str, id: &str, action: &str, payload: &V) -> bool {
    if kind == "change" && ["skill-capsules-status", "skill-capsules-form"].contains(&id) {
        load_capsules().await;
        return true;
    }
    if kind != "click" {
        return false;
    }
    if id == "skill-capsules-refresh" {
        load_capsules().await;
        return true;
    }
    let capsule = payload.as_str().unwrap_or("");
    match action {
        "skill-capsule-open" => {
            m::set("skill-capsule-open", json!(capsule));
            m::take("skill-capsule-armed");
            render_list();
            load_capsule(capsule).await;
            dom::focus("skill-capsule-detail");
        }
        "skill-capsule-arm" => {
            m::set("skill-capsule-armed", json!(capsule));
            render_detail();
        }
        "skill-capsule-keep" => {
            m::take("skill-capsule-armed");
            render_detail();
        }
        "skill-capsule-requalify" => act("requalify", capsule).await,
        "skill-capsule-retire" => act("retire", capsule).await,
        "skill-capsule-delete" => {
            // Only a delete that was armed for this capsule goes through.
            if m::data("skill-capsule-armed") == capsule {
                act("delete", capsule).await;
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{assert_no_inline_hex_colors, payloads};

    fn detail() -> V {
        json!({
            "capsule_id": "c1", "project_id": "p1", "skill_name": "pdf",
            "skill_path": ".agents/skills/pdf/SKILL.md",
            "mu": {"name": "pdf-rows", "description": "Rows of a folder of <invoice> PDFs"},
            "form": "hybrid", "status": "active", "reason": null,
            "capsule": {
                "form": "hybrid",
                "capabilities": {"network": "none", "runtimes": ["python3"]},
                "bindings": [{"name": "input_dir", "type": "path", "origin": "request",
                              "description": "Folder with the invoices"}],
                "programs": [{"id": "extract", "description": "Reads every PDF",
                              "entry": ["python3", "programs/extract/main.py"],
                              "inputs": ["input_dir"], "outputs": ["rows_file"],
                              "check": ["python3", "programs/extract/main.py", "--check"],
                              "timeout_seconds": 60, "read_only": false}],
                "steps": [
                    {"kind": "program", "program": "extract"},
                    {"kind": "instruction", "text": "Confirm the <totals>.",
                     "source": {"path": ".agents/skills/pdf/SKILL.md", "section": "Totals"},
                     "produces": ["confirmed"]}
                ],
                "verification": [{"kind": "program", "program": "verify"}]
            },
            "markdown": null,
            "files": [{"path": "programs/extract/main.py", "bytes": 512}],
            "skill_files": {"skills": [{"name": "pdf", "path": ".agents/skills/pdf/SKILL.md", "files": 3}]},
            "gates": [
                {"round": 1, "gate": "gate1", "passed": false,
                 "report": {"violations": [{"code": "output_unread", "at": "programs[0]", "message": "`rows` is never read."}]}},
                {"round": 2, "gate": "gate1", "passed": true, "report": {"violations": []}}
            ],
            "compile": {"status": "completed", "rounds": 2, "provider": "claude", "model": null,
                        "outcome": "Qualified as a hybrid capsule in round 2.",
                        "usage": {"input_tokens": 1000, "output_tokens": 200, "cost_usd": 0.05}},
            "runs": {"count": 3, "complete": 2, "handed_back": 1, "deopts": 1,
                     "recent": [{"created_at": "2026-11-20T10:00:00Z", "outcome": "complete", "deopts": 1,
                                 "provider": "codex", "model": null}]},
            "amortization": {"compile_usd": 0.05, "warm_turns": 3,
                             "saving_per_reuse_usd": null, "breakeven_reuses": null}
        })
    }

    #[test]
    fn a_row_names_the_capsule_and_its_state() {
        let row = render_capsule_row_html(&detail(), true);
        assert_no_inline_hex_colors(&row);
        assert_eq!(payloads(&row, "skill-capsule-open"), [json!("c1")]);
        assert!(row.contains("aria-current=\"true\""));
        assert!(row.contains(">pdf-rows<"));
        assert!(row.contains("data-tone=\"ok\">Active<"));
        assert!(row.contains(">Hybrid<"));
        // While it compiles it has no name of its own yet.
        let compiling = json!({"capsule_id": "c2", "skill_name": "pdf", "status": "compiling",
                               "mu": null, "form": null, "skill_path": ".agents/skills/pdf/SKILL.md"});
        let row = render_capsule_row_html(&compiling, false);
        assert!(row.contains("From skill `pdf`"));
        assert!(row.contains("data-tone=\"muted\">Compiling<"));
        // A stale capsule says why.
        let mut stale = detail();
        stale["status"] = "stale".into();
        stale["reason"] = "The skill `pdf` changed since this capsule was compiled.".into();
        let row = render_capsule_row_html(&stale, false);
        assert!(row.contains("data-tone=\"warn\">Stale<"));
        assert!(row.contains("changed since this capsule was compiled"));
    }

    #[test]
    fn the_inspector_shows_the_plan_and_never_an_unknown_as_zero() {
        let html = render_capsule_detail_html(&detail(), false, false);
        assert_no_inline_hex_colors(&html);
        // What it is for, needs, runs, in which order, and how it is checked.
        assert!(html.contains("Rows of a folder of &lt;invoice&gt; PDFs"));
        assert!(html.contains("Values it needs from the request (B)"));
        assert!(html.contains("<code>input_dir</code>"));
        assert!(html.contains("Programs (P)"));
        assert!(html.contains("runs <code>python3 programs/extract/main.py</code>"));
        assert!(html.contains("reads input_dir · gives rows_file · up to 60 s"));
        assert!(html.contains("Plan (π)") && html.contains("Verification (V)"));
        assert!(html.contains("Confirm the &lt;totals&gt;."));
        assert!(html.contains(".agents/skills/pdf/SKILL.md § Totals"));
        assert!(html.contains("produces confirmed"));
        assert!(!html.contains("<totals>") && !html.contains("<invoice>"));
        // Its history.
        assert!(html.contains("Round 1 · gate1</span> failed"));
        assert!(html.contains("<code>output_unread</code> `rows` is never read."));
        assert!(html.contains("Round 2 · gate1</span> passed"));
        assert!(html.contains("Qualified as a hybrid capsule in round 2."));
        assert!(html.contains("1000 input · 200 output · — cache read · — cache write"));
        // What compiling cost is known; what a reuse saves is not.
        assert!(html.contains("<dt>Compilation cost</dt><dd>$0.0500</dd>"));
        assert!(html.contains("<dt>Saving per reuse</dt><dd>—</dd>"));
        assert!(html.contains("<dt>Break-even</dt><dd>—</dd>"));
        assert!(html.contains("<dt>Handed a step back</dt><dd>1</dd>"));
        assert!(html.contains("1 step(s) handed back"));
        // The package is listed, not shown.
        assert!(html.contains("<code>programs/extract/main.py</code>"));
        assert!(html.contains("512 bytes"));
    }

    #[test]
    fn actions_follow_the_status_and_a_delete_is_confirmed() {
        let html_for = |status: &str, armed: bool| {
            let mut capsule = detail();
            capsule["status"] = status.into();
            render_capsule_detail_html(&capsule, armed, false)
        };
        let actions = |status: &str, armed: bool| {
            let html = html_for(status, armed);
            [
                "skill-capsule-requalify",
                "skill-capsule-retire",
                "skill-capsule-arm",
                "skill-capsule-delete",
                "skill-capsule-keep",
            ]
            .into_iter()
            .filter(|action| !payloads(&html, action).is_empty())
            .collect::<Vec<_>>()
        };
        assert_eq!(
            actions("active", false),
            [
                "skill-capsule-requalify",
                "skill-capsule-retire",
                "skill-capsule-arm"
            ]
        );
        assert_eq!(
            actions("stale", false),
            [
                "skill-capsule-requalify",
                "skill-capsule-retire",
                "skill-capsule-arm"
            ]
        );
        assert_eq!(actions("failed", false), ["skill-capsule-arm"]);
        assert_eq!(
            actions("retired", false),
            ["skill-capsule-requalify", "skill-capsule-arm"]
        );
        let retired_html = html_for("retired", false);
        assert!(retired_html.contains(">Use again<"));
        assert!(!retired_html.contains(">Requalify<"));
        assert!(!retired_html.contains(">Stop using<"));
        // Nothing is done to a capsule that is being compiled.
        assert!(actions("compiling", false).is_empty());
        // A delete is one more click, and it can be taken back.
        assert_eq!(
            actions("failed", true),
            ["skill-capsule-delete", "skill-capsule-keep"]
        );
        let pending = render_capsule_detail_html(&detail(), false, true);
        assert_eq!(pending.matches(" disabled>").count(), 3);
        assert_eq!(
            list_query("", ""),
            vec![("include_retired", "true".to_owned())]
        );
        assert_eq!(
            list_query("retired", "hybrid"),
            vec![
                ("status", "retired".to_owned()),
                ("form", "hybrid".to_owned())
            ]
        );
    }

    #[test]
    fn a_written_procedure_shows_its_text() {
        let mut textual = detail();
        textual["form"] = "textual".into();
        textual["capsule"] = json!({"form": "textual"});
        textual["markdown"] = "# Rows\n1. Read <every> file.".into();
        let html = render_capsule_detail_html(&textual, false, false);
        assert!(html.contains(">Written procedure<"));
        assert!(html.contains("1. Read &lt;every&gt; file."));
        assert!(!html.contains("Programs (P)"));
        assert_eq!(money(&V::Null), "—");
        assert_eq!(money(&json!(0.0)), "$0.0000");
    }
}
