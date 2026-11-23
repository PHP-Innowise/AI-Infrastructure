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
            format!(
                "<span class=\"skill-capsule-reason\">{}</span>",
                esc(&reason)
            )
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

fn gate_stderr_block(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!(
            "<pre class=\"skill-capsule-gate-stderr\">{}</pre>",
            esc(trimmed)
        )
    }
}

fn gate_programs_summary_html(programs: &[V]) -> String {
    if programs.is_empty() {
        return String::new();
    }
    let rows = programs
        .iter()
        .map(|prog| {
            let id = m::text(prog, "id");
            let passed = m::yes(&prog["passed"]);
            let exit = prog["exit_code"]
                .as_i64()
                .map_or_else(|| "—".to_owned(), |c| c.to_string());
            let secs = prog["seconds"]
                .as_f64()
                .map_or_else(String::new, |s| format!(" · {s:.2}s"));
            let denied = if m::yes(&prog["sandbox_denied"]) {
                " · sandbox denied"
            } else {
                ""
            };
            let stderr_html = gate_stderr_block(prog["stderr"].as_str().unwrap_or(""));
            format!(
                "<li><code>{}</code> {} (exit {}{secs}{denied}){stderr_html}</li>",
                esc(&id),
                if passed { "passed" } else { "failed" },
                esc(&exit)
            )
        })
        .collect::<String>();
    format!("<ul class=\"skill-capsule-violations\">{rows}</ul>")
}

fn gate_comparison_summary_html(report: &V) -> String {
    let comparison = &report["result_comparison"];
    let mut parts = Vec::new();
    if let Some(secs) = report["seconds"].as_f64() {
        parts.push(format!("{secs:.2}s"));
    }
    if let Some(net) = report["network"].as_str().filter(|s| !s.is_empty()) {
        parts.push(format!("network {}", esc(net)));
    }
    if comparison.is_object() {
        let cmp_status = comparison["status"].as_str().unwrap_or("unknown");
        parts.push(format!("comparison {}", esc(cmp_status)));
    }
    let summary_line = if parts.is_empty() {
        String::new()
    } else {
        format!(
            "<p class=\"skill-capsule-source\">{}</p>",
            parts.join(" · ")
        )
    };
    let report_stderr = gate_stderr_block(report["stderr"].as_str().unwrap_or(""));
    let report_details = gate_stderr_block(report["details"].as_str().unwrap_or(""));
    let cmp_stderr = gate_stderr_block(comparison["stderr"].as_str().unwrap_or(""));
    let cmp_details = gate_stderr_block(comparison["details"].as_str().unwrap_or(""));
    format!("{summary_line}{report_stderr}{report_details}{cmp_stderr}{cmp_details}")
}

fn gates_html(gates: &[V]) -> String {
    if gates.is_empty() {
        return String::new();
    }
    let rows = gates
        .iter()
        .map(|gate| {
            let passed = m::yes(&gate["passed"]);
            let at_str = gate["at"]
                .as_str()
                .or_else(|| gate["created_at"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| format!(" <span class=\"skill-capsule-source\">{}</span>", esc(s)))
                .unwrap_or_default();
            let report = &gate["report"];
            let programs_html = gate_programs_summary_html(&m::array(&report["programs"]));
            let comparison_html = gate_comparison_summary_html(report);
            let violations = m::array(&report["violations"]);
            let details = violations
                .iter()
                .map(|violation| {
                    let at_v = violation["at"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| {
                            format!(
                                " <span class=\"skill-capsule-source\">at <code>{}</code></span>",
                                esc(s)
                            )
                        })
                        .unwrap_or_default();
                    let prog_v = violation["program"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| {
                            format!(
                                " <span class=\"skill-capsule-source\">program <code>{}</code></span>",
                                esc(s)
                            )
                        })
                        .unwrap_or_default();
                    let extra = format!(
                        "{}{}",
                        gate_stderr_block(violation["stderr"].as_str().unwrap_or("")),
                        gate_stderr_block(violation["details"].as_str().unwrap_or(""))
                    );
                    format!(
                        "<li><code>{}</code> {}{at_v}{prog_v}{extra}</li>",
                        esc(&m::text(violation, "code")),
                        esc(&m::text(violation, "message"))
                    )
                })
                .collect::<String>();
            format!(
                "<li data-passed=\"{}\"><span class=\"skill-capsule-kind\">Round {} · {}</span> {}{}{}{}{}</li>",
                if passed { "true" } else { "false" },
                count(&gate["round"]),
                esc(&m::text(gate, "gate")),
                if passed { "passed" } else { "failed" },
                at_str,
                comparison_html,
                programs_html,
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

fn session_link_html(project_id: &str, session_id: &str) -> String {
    let sid = session_id.trim();
    if sid.is_empty() {
        return String::new();
    }
    let route = dom::format_hash_str(&dom::HashRoute {
        view: "sessions".into(),
        project: project_id.to_owned(),
        session: sid.to_owned(),
        run: String::new(),
        path: String::new(),
    });
    let short_sid = if sid.chars().count() > 14 {
        let prefix: String = sid.chars().take(12).collect();
        format!("{prefix}…")
    } else {
        sid.to_owned()
    };
    let payload = json!({"project_id": project_id, "session_id": sid});
    format!(
        "<a class=\"skill-capsule-session-link\" href=\"{}\" data-action=\"skill-capsule-open-session\" data-payload=\"{}\">Session <code>{}</code></a>",
        esc(&route),
        esc(&payload.to_string()),
        esc(&short_sid)
    )
}

fn run_row_html(run: &V, project_id: &str) -> String {
    let handed = run["deopts"].as_i64().unwrap_or_default();
    let link = session_link_html(project_id, &m::text(run, "session_id"));
    format!(
        "<li>{} · {}{} <span class=\"skill-capsule-source\">{} {}</span>{}</li>",
        esc(&m::text(run, "created_at")),
        esc(&m::text(run, "outcome")),
        if handed > 0 {
            format!(" · {handed} step(s) handed back")
        } else {
            String::new()
        },
        esc(&m::text(run, "provider")),
        esc(&m::text(run, "model")),
        if link.is_empty() {
            String::new()
        } else {
            format!(" · {link}")
        }
    )
}

fn runs_html(detail: &V) -> String {
    let project_id = m::text(detail, "project_id");
    let runs = &detail["runs"];
    let amortization = &detail["amortization"];
    let all_recent = m::array(&runs["recent"]);
    let recent = all_recent
        .iter()
        .take(8)
        .map(|run| run_row_html(run, &project_id))
        .collect::<String>();
    let older = if all_recent.len() > 8 {
        let rows = all_recent[8..]
            .iter()
            .map(|run| run_row_html(run, &project_id))
            .collect::<String>();
        format!(
            "<details class=\"skill-capsule-more-runs\"><summary>Show {} earlier warm turns</summary><ul class=\"skill-capsule-list\">{rows}</ul></details>",
            all_recent.len() - 8
        )
    } else {
        String::new()
    };
    let warm_turns_display = match (runs["count"].as_i64(), runs["total"].as_i64()) {
        (Some(since), Some(total)) if total > since => format!("{since} ({total} total)"),
        _ => count(&runs["count"]),
    };
    let cost_per_turn = match (
        amortization["compile_usd"].as_f64(),
        runs["count"].as_i64().filter(|c| *c > 0),
    ) {
        (Some(usd), Some(n)) => format!("${:.4}", usd / n as f64),
        _ => "—".into(),
    };
    let body = format!(
        "<dl class=\"skill-capsule-facts-grid\"><dt>Warm turns</dt><dd>{}</dd><dt>Completed</dt><dd>{}</dd><dt>Handed a step back</dt><dd>{}</dd><dt>Compilation cost</dt><dd>{}</dd><dt>Compile cost / warm turn</dt><dd>{}</dd><dt>Saving per reuse</dt><dd>{}</dd><dt>Break-even</dt><dd>{}</dd></dl><p class=\"skills-note\">A saving is measured by a sealed evaluation (<code>infra-engine eval skillcache-ab</code>), not read off these turns. Until one exists it is unknown.</p>{}{}",
        warm_turns_display,
        count(&runs["complete"]),
        count(&runs["handed_back"]),
        money(&amortization["compile_usd"]),
        cost_per_turn,
        money(&amortization["saving_per_reuse_usd"]),
        count(&amortization["breakeven_reuses"]),
        if recent.is_empty() {
            String::new()
        } else {
            format!("<ul class=\"skill-capsule-list\">{recent}</ul>")
        },
        older
    );
    section("Warm turns and cost", &body)
}

fn compile_html(compile: &V, project_id: &str, source_session_id: &str) -> String {
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
    let source_link = session_link_html(project_id, source_session_id);
    let source_row = if source_link.is_empty() {
        String::new()
    } else {
        format!("<dt>Source session</dt><dd>{source_link}</dd>")
    };
    section(
        "Compilation",
        &format!(
            "<dl class=\"skill-capsule-facts-grid\"><dt>Status</dt><dd>{}</dd><dt>Rounds</dt><dd>{}</dd><dt>Compiler</dt><dd>{} {}</dd><dt>Tokens</dt><dd>{}</dd><dt>Cost</dt><dd>{}</dd>{}</dl><p class=\"skills-note\">{}</p>",
            esc(&m::text(compile, "status")),
            count(&compile["rounds"]),
            esc(&m::text(compile, "provider")),
            esc(&m::text(compile, "model")),
            esc(&tokens),
            money(&usage["cost_usd"]),
            source_row,
            esc(&m::text(compile, "outcome")),
        ),
    )
}

pub(crate) fn is_needs_trace_error(status: u16, message: &str) -> bool {
    if status == 409 {
        return true;
    }
    let lower = message.to_ascii_lowercase();
    lower.contains("needs_trace")
        || lower.contains("fresh observed trace")
        || lower.contains("needs a fresh")
        || lower.contains("no execution trace")
}

pub(crate) fn build_recompile_request_body(
    provider_override: &str,
    model_override: &str,
    selected: &V,
    session_state: &V,
    boot: &V,
) -> V {
    let provider = {
        let explicit = provider_override.trim();
        if !explicit.is_empty() {
            Some(explicit.to_owned())
        } else {
            crate::skillcache::configured_compiler_provider(selected, session_state, boot)
        }
    };
    let model = {
        let explicit = model_override.trim();
        if !explicit.is_empty() {
            Some(explicit.to_owned())
        } else {
            selected["skillcache"]["compiler"]["model"]
                .as_str()
                .or_else(|| session_state["compiler"]["model"].as_str())
                .or_else(|| boot["skillcache"]["compiler"]["model"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        }
    };
    if provider.is_none() && model.is_none() {
        return json!({});
    }
    let mut compiler = serde_json::Map::new();
    if let Some(p) = provider {
        compiler.insert("provider".into(), V::String(p));
    }
    if let Some(m_id) = model {
        compiler.insert("model".into(), V::String(m_id));
    }
    json!({"compiler": V::Object(compiler)})
}

fn compiler_controls_html(provider: &str, model: &str, disabled: bool) -> String {
    let mut opts = String::new();
    for (val, label) in [
        ("", "Default compiler"),
        ("claude", "Claude"),
        ("codex", "Codex"),
        ("cursor", "Cursor"),
        ("gemini", "Gemini"),
    ] {
        let sel = if provider.eq_ignore_ascii_case(val) {
            " selected"
        } else {
            ""
        };
        opts.push_str(&format!(
            "<option value=\"{val}\"{sel}>{}</option>",
            esc(label)
        ));
    }
    let dis = if disabled { " disabled" } else { "" };
    format!(
        "<div class=\"skill-capsule-compiler\">\
         <label class=\"field\">Compiler<select id=\"skill-capsule-compiler-provider\"{dis}>{opts}</select></label>\
         <label class=\"field\">Compiler model<input id=\"skill-capsule-compiler-model\" type=\"text\" maxlength=\"120\" autocomplete=\"off\" spellcheck=\"false\" placeholder=\"Default model\" value=\"{}\"{dis}></label>\
         </div>",
        esc(model)
    )
}

fn needs_trace_guidance_html(detail: &V, error_message: &str, disabled: bool) -> String {
    let message = if !error_message.trim().is_empty() {
        error_message.trim()
    } else if !detail["reason"].as_str().unwrap_or("").trim().is_empty() {
        detail["reason"].as_str().unwrap_or("")
    } else {
        "This capsule needs a fresh observed trace before it can be recompiled."
    };
    let payload = json!({
        "capsule_id": detail["capsule_id"],
        "project_id": detail["project_id"],
        "skill_name": detail["skill_name"],
        "skill_path": detail["skill_path"],
    });
    format!(
        "<div class=\"skill-capsule-needs-trace\" role=\"status\">\
         <p>{}</p>\
         {}</div>",
        esc(message),
        button(
            "Capture fresh trace in session",
            "skill-capsule-capture-trace",
            &payload,
            "primary",
            disabled
        )
    )
}

pub(crate) fn capsule_matches_query(capsule: &V, query: &str) -> bool {
    let trimmed = query.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return true;
    }
    let status = capsule["status"].as_str().unwrap_or("");
    let form = capsule["form"].as_str().unwrap_or("");
    let haystack = [
        capsule_title(capsule),
        m::text(capsule, "skill_name"),
        m::text(capsule, "skill_path"),
        m::text(capsule, "capsule_id"),
        status.to_owned(),
        status_label(status).to_owned(),
        form.to_owned(),
        form_label(form).to_owned(),
        m::text(&capsule["mu"], "description"),
        m::text(capsule, "reason"),
    ]
    .join("\n")
    .to_ascii_lowercase();
    trimmed
        .split_whitespace()
        .all(|term| haystack.contains(term))
}

/// One capsule: its identity and status, what can be done with it, its plan
/// and its history. `armed` is a delete waiting for its confirmation;
/// `pending` is an action in flight.
#[cfg(test)]
pub(crate) fn render_capsule_detail_html(detail: &V, armed: bool, pending: bool) -> String {
    render_capsule_detail_html_with_options(detail, armed, pending, "", "", "")
}

pub(crate) fn render_capsule_detail_html_with_options(
    detail: &V,
    armed: bool,
    pending: bool,
    compiler_provider: &str,
    compiler_model: &str,
    needs_trace_message: &str,
) -> String {
    let id = &detail["capsule_id"];
    let status = detail["status"].as_str().unwrap_or("");
    let capsule = &detail["capsule"];
    let mut actions = String::new();
    let can_recompile = matches!(
        status,
        "failed" | "interrupted" | "needs_recompile" | "stale"
    );
    if matches!(status, "failed" | "interrupted") {
        actions.push_str(&button(
            "Retry compile",
            "skill-capsule-recompile",
            id,
            "",
            pending,
        ));
    }
    if matches!(status, "needs_recompile" | "stale") {
        actions.push_str(&button(
            "Recompile",
            "skill-capsule-recompile",
            id,
            "",
            pending,
        ));
    }
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
    if ["active", "needs_recompile"].contains(&status) {
        actions.push_str(&button(
            "Invalidate",
            "skill-capsule-invalidate",
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
    let show_needs_trace = m::yes(&detail["needs_trace"]) || !needs_trace_message.trim().is_empty();
    let compiler_controls = if can_recompile {
        compiler_controls_html(compiler_provider, compiler_model, pending)
    } else {
        String::new()
    };
    let needs_trace_box = if show_needs_trace {
        needs_trace_guidance_html(detail, needs_trace_message, pending)
    } else {
        String::new()
    };
    let mut out = format!(
        "<header class=\"skill-capsule-head\"><h3>{}</h3><p class=\"installed-tools\">{}{}</p>{}{}<div class=\"skill-capsule-actions\">{}</div>{}{}</header>",
        esc(&capsule_title(detail)),
        chip(status_label(status), status_tone(status)),
        chip(form_label(detail["form"].as_str().unwrap_or("")), "muted"),
        if description.is_empty() {
            String::new()
        } else {
            format!(
                "<p class=\"skill-capsule-description\">{}</p>",
                esc(&description)
            )
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
        compiler_controls,
        needs_trace_box,
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
    out.push_str(&compile_html(
        &detail["compile"],
        &m::text(detail, "project_id"),
        &m::text(detail, "source_session_id"),
    ));
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

fn ensure_search_input() {
    if dom::get("skill-capsules-search").is_some() {
        return;
    }
    let Some(toolbar) = dom::all(".skill-capsules-toolbar").into_iter().next() else {
        return;
    };
    let field = dom::el("label", "field", "Search");
    let input = dom::el("input", "", "");
    let _ = input.set_attribute("id", "skill-capsules-search");
    let _ = input.set_attribute("type", "search");
    let _ = input.set_attribute("maxlength", "200");
    let _ = input.set_attribute("autocomplete", "off");
    let _ = input.set_attribute("spellcheck", "false");
    let _ = input.set_attribute("placeholder", "Filter by skill, capsule, path…");
    dom::add(&field, &input);
    let anchor = dom::get("skill-capsules-refresh");
    let _ = toolbar.insert_before(&field, anchor.as_deref());
}

fn render_list() {
    ensure_search_input();
    let listed = m::data("skill-capsules");
    let capsules = m::array(&listed["capsules"]);
    let query = dom::value("skill-capsules-search");
    let filtered: Vec<&V> = capsules
        .iter()
        .filter(|capsule| capsule_matches_query(capsule, &query))
        .collect();
    let open = m::string(&m::data("skill-capsule-open"));
    if let Some(list) = dom::get("skill-capsules-list") {
        crate::skillcache::set_markup(
            &list,
            &if capsules.is_empty() {
                "<p class=\"empty-note\">No capsule yet. Turn SkillCache to Observe in a session, run a turn that uses a skill, and compile it from the run's SkillCache tab.</p>".to_owned()
            } else if filtered.is_empty() {
                format!(
                    "<p class=\"empty-note\">No capsules match “{}”.</p>",
                    esc(query.trim())
                )
            } else {
                filtered
                    .iter()
                    .map(|capsule| {
                        render_capsule_row_html(capsule, m::text(capsule, "capsule_id") == open)
                    })
                    .collect::<String>()
            },
        );
    }
    let note = if capsules.is_empty() {
        String::new()
    } else if !query.trim().is_empty() {
        format!("{} of {} capsules", filtered.len(), capsules.len())
    } else if capsules.len() == 1 {
        "1 capsule".into()
    } else {
        format!("{} capsules", capsules.len())
    };
    dom::text("skill-capsules-status-note", &note);
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
    let compiler = m::data("skill-capsule-compiler");
    let needs_trace = m::data("skill-capsule-needs-trace");
    let trace_msg = if needs_trace["capsule_id"].as_str() == Some(open.as_str()) {
        needs_trace["message"].as_str().unwrap_or("")
    } else {
        ""
    };
    crate::skillcache::set_markup(
        &pane,
        &render_capsule_detail_html_with_options(
            &detail,
            m::data("skill-capsule-armed") == open.as_str(),
            m::yes(&m::data("skill-capsule-pending")),
            compiler["provider"].as_str().unwrap_or(""),
            compiler["model"].as_str().unwrap_or(""),
            trace_msg,
        ),
    );
    dom::hide(&pane, false);
}

/// Loads the project's capsules and, when one is open, that capsule.
pub async fn load_capsules() {
    ensure_search_input();
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
                || !dom::value("skill-capsules-form").is_empty()
                || !dom::value("skill-capsules-search").trim().is_empty();
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
    let compiler_saved = m::data("skill-capsule-compiler");
    let dom_provider = dom::value("skill-capsule-compiler-provider");
    let dom_model = dom::value("skill-capsule-compiler-model");
    let provider_override = if dom_provider.trim().is_empty() {
        compiler_saved["provider"].as_str().unwrap_or("")
    } else {
        dom_provider.as_str()
    };
    let model_override = if dom_model.trim().is_empty() {
        let saved_m = compiler_saved["model"].as_str().unwrap_or("");
        if saved_m.trim().is_empty() {
            dom::value("skillcache-compiler-model")
        } else {
            saved_m.to_owned()
        }
    } else {
        dom_model.clone()
    };
    m::set("skill-capsule-pending", json!(true));
    m::take("skill-capsule-armed");
    render_detail();
    let result = match action {
        "delete" => api::delete(&base).await,
        "recompile" => {
            let (selected, boot) = m::read(|s| (s.selected.clone(), s.boot.clone()));
            let body = build_recompile_request_body(
                provider_override,
                &model_override,
                &selected,
                &crate::skillcache::session_state(),
                &boot,
            );
            api::post(&format!("{base}/recompile"), body).await
        }
        _ => api::post(&format!("{base}/{action}"), json!({})).await,
    };
    m::take("skill-capsule-pending");
    match result {
        Ok(_) => {
            dom::error("skill-capsules-error", "");
            m::take("skill-capsule-needs-trace");
            if action == "delete" {
                m::take("skill-capsule-open");
                m::take("skill-capsule-detail");
            }
        }
        Err(error) => {
            if action == "recompile" && is_needs_trace_error(error.status, &error.message) {
                m::set(
                    "skill-capsule-needs-trace",
                    json!({"capsule_id": id, "message": error.message}),
                );
            }
            dom::error("skill-capsules-error", &error.message);
        }
    }
    load_capsules().await;
}

pub(super) async fn capsules_event(kind: &str, id: &str, action: &str, payload: &V) -> bool {
    if matches!(kind, "input" | "change") && id == "skill-capsules-search" {
        render_list();
        return true;
    }
    if matches!(kind, "input" | "change")
        && [
            "skill-capsule-compiler-provider",
            "skill-capsule-compiler-model",
        ]
        .contains(&id)
    {
        let mut compiler = m::data("skill-capsule-compiler");
        if !compiler.is_object() {
            compiler = json!({});
        }
        compiler["provider"] = json!(dom::value("skill-capsule-compiler-provider"));
        compiler["model"] = json!(dom::value("skill-capsule-compiler-model"));
        m::set("skill-capsule-compiler", compiler);
        return true;
    }
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
            m::take("skill-capsule-needs-trace");
            render_list();
            load_capsule(capsule).await;
            dom::focus("skill-capsule-detail");
        }
        "skill-capsule-open-session" => {
            let sid = m::text(payload, "session_id");
            let proj = m::text(payload, "project_id");
            if !sid.is_empty() {
                if !proj.is_empty() && proj != m::current_project() {
                    dom::set_value("project-switcher", &proj);
                }
                crate::session::select(&sid).await;
                crate::session::open_view("sessions", true).await;
            }
        }
        "skill-capsule-capture-trace" => {
            let proj = m::text(payload, "project_id");
            let skill = m::text(payload, "skill_name");
            m::take("skill-capsule-needs-trace");
            crate::session::open_view("sessions", true).await;
            if !proj.is_empty() {
                crate::session::new_session(Some(&proj));
            } else {
                crate::session::new_session(None);
            }
            dom::set_value("skillcache-mode", "observe");
            crate::skillcache::controls();
            if !skill.trim().is_empty() && dom::value("prompt").trim().is_empty() {
                dom::set_value("prompt", &format!("${} ", skill.trim()));
            }
            dom::focus("prompt");
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
        "skill-capsule-recompile" => act("recompile", capsule).await,
        "skill-capsule-invalidate" => act("invalidate", capsule).await,
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
            "source_session_id": "s1", "source_generation": "g1",
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
        assert!(html.contains("<dt>Compile cost / warm turn</dt><dd>$0.0167</dd>"));
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
                "skill-capsule-recompile",
                "skill-capsule-requalify",
                "skill-capsule-invalidate",
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
                "skill-capsule-invalidate",
                "skill-capsule-retire",
                "skill-capsule-arm"
            ]
        );
        assert_eq!(
            actions("stale", false),
            [
                "skill-capsule-recompile",
                "skill-capsule-requalify",
                "skill-capsule-retire",
                "skill-capsule-arm"
            ]
        );
        assert_eq!(
            actions("needs_recompile", false),
            [
                "skill-capsule-recompile",
                "skill-capsule-requalify",
                "skill-capsule-invalidate",
                "skill-capsule-retire",
                "skill-capsule-arm"
            ]
        );
        assert_eq!(
            actions("failed", false),
            ["skill-capsule-recompile", "skill-capsule-arm"]
        );
        assert!(html_for("failed", false).contains(">Retry compile<"));
        assert!(html_for("needs_recompile", false).contains(">Recompile<"));
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
            [
                "skill-capsule-recompile",
                "skill-capsule-delete",
                "skill-capsule-keep"
            ]
        );
        let pending = render_capsule_detail_html(&detail(), false, true);
        assert_eq!(pending.matches(" disabled>").count(), 4);
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

    #[test]
    fn template_admitted_and_needs_trace_capsules_render_controls_and_deep_links() {
        // Template-admitted capsule (empty source_session_id / source_generation) still gets Recompile.
        let mut tmpl = detail();
        tmpl["status"] = "needs_recompile".into();
        tmpl["source_session_id"] = "".into();
        tmpl["source_generation"] = "".into();
        tmpl["needs_trace"] = json!(true);
        let html = render_capsule_detail_html_with_options(
            &tmpl,
            false,
            false,
            "codex",
            "gpt-5-codex",
            "",
        );
        assert_no_inline_hex_colors(&html);
        assert_eq!(payloads(&html, "skill-capsule-recompile"), [json!("c1")]);
        assert!(html.contains("id=\"skill-capsule-compiler-provider\""));
        assert!(html.contains("id=\"skill-capsule-compiler-model\""));
        assert!(html.contains("value=\"gpt-5-codex\""));
        assert!(html.contains("skill-capsule-capture-trace"));
        assert_eq!(payloads(&html, "skill-capsule-capture-trace").len(), 1);
        // Compiler selector and trace CTA stay outside `.skill-capsule-actions`.
        let actions_start = html.find("<div class=\"skill-capsule-actions\">").unwrap();
        let actions_end = html[actions_start..].find("</div>").unwrap() + actions_start;
        let actions_slice = &html[actions_start..actions_end];
        assert!(!actions_slice.contains("skill-capsule-capture-trace"));
        assert!(!actions_slice.contains("skill-capsule-compiler-provider"));

        assert_eq!(
            build_recompile_request_body("codex", "gpt-5", &V::Null, &V::Null, &V::Null),
            json!({"compiler": {"provider": "codex", "model": "gpt-5"}})
        );
        assert_eq!(
            build_recompile_request_body("", "", &V::Null, &V::Null, &V::Null),
            json!({})
        );
        assert!(is_needs_trace_error(409, "conflict"));
        assert!(is_needs_trace_error(400, "needs a fresh observed trace"));
        assert!(!is_needs_trace_error(500, "compiler failed"));

        // Text search filters by title, skill_name, path, description, or status.
        assert!(capsule_matches_query(&detail(), ""));
        assert!(capsule_matches_query(&detail(), "pdf-rows"));
        assert!(capsule_matches_query(&detail(), "pdf active"));
        assert!(!capsule_matches_query(&detail(), "nonexistent-skill"));

        // Gates show timestamp, violation `at`/`program`, stderr/details, and all >12 violations.
        let mut rich = detail();
        rich["runs"]["recent"] = json!((0..10)
            .map(|idx| json!({
                "created_at": format!("2026-11-20T10:0{idx}:00Z"),
                "outcome": "complete",
                "deopts": 0,
                "provider": "codex",
                "model": "gpt-5",
                "session_id": format!("sess-run-{idx}")
            }))
            .collect::<Vec<_>>());
        let mut lots = Vec::new();
        for idx in 0..14 {
            lots.push(json!({
                "code": format!("v_{idx}"),
                "at": format!("programs[0].line_{idx}"),
                "program": "extract",
                "message": format!("Violation {idx}"),
                "stderr": if idx == 0 { "Traceback: module not found" } else { "" }
            }));
        }
        rich["gates"] = json!([{
            "round": 2,
            "gate": "gate2",
            "at": "2026-11-20T10:05:00Z",
            "passed": false,
            "report": {
                "programs": [{"id": "extract", "passed": false, "exit_code": 1, "seconds": 0.42, "sandbox_denied": true}],
                "result_comparison": {"status": "mismatch", "details": "Expected 3 rows, got 2"},
                "violations": lots
            }
        }]);
        let rich_html = render_capsule_detail_html(&rich, false, false);
        assert!(rich_html.contains("2026-11-20T10:05:00Z"));
        assert!(rich_html.contains("at <code>programs[0].line_13</code>"));
        assert!(rich_html.contains("program <code>extract</code>"));
        assert!(rich_html.contains("Traceback: module not found"));
        assert!(rich_html.contains("Expected 3 rows, got 2"));
        assert!(rich_html.contains("sandbox denied"));
        assert!(rich_html.contains("v_13"));
        assert!(!rich_html.contains("12 shown"));
        assert!(rich_html.contains("Show 2 earlier warm turns"));
        assert!(rich_html.contains("href=\"#/sessions?project=p1&amp;session=sess-run-0\""));
        assert!(rich_html.contains("href=\"#/sessions?project=p1&amp;session=s1\""));
        assert_eq!(payloads(&rich_html, "skill-capsule-open-session").len(), 11);
    }
}
