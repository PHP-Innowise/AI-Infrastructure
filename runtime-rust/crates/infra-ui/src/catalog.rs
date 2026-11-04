use crate::{dom, model as m};
use serde_json::{json, Value as V};
use std::collections::BTreeSet;
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};
fn truthy(v: &V) -> bool {
    !v.is_null() && v != &V::Bool(false) && v.as_str() != Some("") && v.as_f64() != Some(0.)
}
fn resources() -> Vec<V> {
    m::array(&m::data("catalog-resources"))
}
fn category(id: &str) -> String {
    match id {
        "skills" => "Skills",
        "subagents" => "Subagents",
        "mcp" => "MCP servers",
        "hooks" => "Hooks",
        "marketplace" => "Marketplaces",
        "cli-tool" => "CLI tools",
        "discovery-index" => "Discovery lists",
        _ => id,
    }
    .into()
}
fn client(id: &str) -> String {
    match id {
        "codex" => "Codex",
        "cursor" => "Cursor",
        "claude-code" => "Claude Code",
        _ => id,
    }
    .into()
}
fn status(v: &V) -> &str {
    v["review_status"].as_str().unwrap_or("unreviewed")
}
fn status_label(id: &str) -> &str {
    match id {
        "known_risks" => "Known risks",
        "open_questions" => "Open questions",
        "clear" => "Reviewed · Clear",
        _ => "Not reviewed",
    }
}
pub fn start() {
    let raw = dom::get("kit3-data")
        .and_then(|e| e.text_content())
        .unwrap_or_default();
    let data: V = serde_json::from_str(&raw).unwrap_or(json!([]));
    m::set("catalog-resources", data);
    let theme = dom::storage("harness.theme.v1").unwrap_or_else(|| "system".into());
    theme_apply(&theme);
    let items = resources();
    let sample = items
        .iter()
        .find(|x| x["id"] == "obra-superpowers" && truthy(&x["skills_source"]))
        .or_else(|| items.iter().find(|x| truthy(&x["skills_source"])));
    dom::append(
        "hero-command",
        &terminal(
            sample
                .and_then(|v| v["command"].as_str())
                .unwrap_or("./kit3 find"),
        ),
    );
    dom::append("how-command", &terminal("./kit3 --help"));
    let categories = items
        .iter()
        .filter_map(|x| x["category"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    for id in categories {
        let e = dom::el("option", "", &category(&id));
        let _ = e.set_attribute("value", &id);
        dom::append("category", &e);
    }
    let clients = items
        .iter()
        .flat_map(|x| {
            m::array(&x["tool_compatibility"])
                .iter()
                .map(m::string)
                .collect::<Vec<_>>()
        })
        .collect::<BTreeSet<_>>();
    for id in clients {
        let e = dom::el("option", "", &client(&id));
        let _ = e.set_attribute("value", &id);
        dom::append("client", &e);
    }
    for kind in ["input", "change", "click", "submit"] {
        let k = kind.to_owned();
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(target) = event
                .target()
                .and_then(|e| e.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            if k == "submit" {
                event.prevent_default();
                return;
            }
            if k == "input" || k == "change" {
                render();
                return;
            }
            let button = target
                .closest("[data-action],button")
                .ok()
                .flatten()
                .unwrap_or(target);
            let action = button
                .get_attribute("data-action")
                .unwrap_or_else(|| button.id());
            match action.as_str() {
                "reset" | "empty-reset" => {
                    for id in ["search", "category", "client", "review"] {
                        dom::set_value(id, "");
                    }
                    render();
                    dom::focus("search");
                }
                "catalog-copy" => {
                    let command = button
                        .get_attribute("data-payload")
                        .and_then(|v| serde_json::from_str::<V>(&v).ok())
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default();
                    spawn_local(async move {
                        let result = JsFuture::from(
                            dom::window().navigator().clipboard().write_text(&command),
                        )
                        .await;
                        if result.is_ok() {
                            notify("Command copied. Run it in your terminal.");
                        } else {
                            notify("Clipboard unavailable. Select and copy the command manually.");
                        }
                    });
                }
                _ => {}
            }
        });
        let _ = dom::document().add_event_listener_with_callback(kind, cb.as_ref().unchecked_ref());
        cb.forget();
    }
    let cb = Closure::<dyn FnMut(web_sys::Event)>::new(|_| route(true));
    let _ =
        dom::window().add_event_listener_with_callback("hashchange", cb.as_ref().unchecked_ref());
    cb.forget();
    let cb = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
        theme_apply(&dom::storage("harness.theme.v1").unwrap_or_else(|| "system".into()))
    });
    let _ = dom::window().add_event_listener_with_callback("storage", cb.as_ref().unchecked_ref());
    cb.forget();
    if dom::window().frame_element().ok().flatten().is_some() {
        if let Some(e) = dom::document().document_element() {
            let _ = e.set_attribute("data-embedded", "true");
        }
    }
    if let Ok(Some(media)) = dom::window().match_media("(prefers-color-scheme: dark)") {
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
            theme_apply(&dom::storage("harness.theme.v1").unwrap_or_else(|| "system".into()))
        });
        let _ = media.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref());
        cb.forget();
    }
    render();
    route(false);
}
fn theme_apply(preference: &str) {
    let dark = dom::window()
        .match_media("(prefers-color-scheme: dark)")
        .ok()
        .flatten()
        .map(|m| m.matches())
        .unwrap_or(false);
    let theme = if ["dark", "light"].contains(&preference) {
        preference
    } else if dark {
        "dark"
    } else {
        "light"
    };
    if let Some(e) = dom::document().document_element() {
        let _ = e.set_attribute("data-theme", theme);
    }
}
fn notify(message: &str) {
    dom::text("toast", message);
    dom::hidden("toast", false);
    dom::later("catalog-toast", 4000, || dom::hidden("toast", true));
}
fn terminal(command: &str) -> web_sys::Element {
    let block = dom::el("div", "terminal", "");
    let code = dom::el("code", "", &format!("$ {command}"));
    dom::add(&block, &code);
    let button = dom::button("Copy", "catalog-copy", json!(command));
    button.set_class_name("copy");
    dom::add(&block, &button);
    block
}
fn render() {
    let q = dom::value("search").trim().to_lowercase();
    let cat = dom::value("category");
    let cli = dom::value("client");
    let review = dom::value("review");
    let items = resources();
    let mut visible = items
        .iter()
        .filter(|v| {
            let haystack = ["id", "name", "description", "url"]
                .iter()
                .map(|k| m::text(v, k))
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            (q.is_empty() || haystack.contains(&q))
                && (cat.is_empty() || v["category"] == cat)
                && (cli.is_empty() || m::array(&v["tool_compatibility"]).iter().any(|x| x == &cli))
                && (review.is_empty() || status(v) == review)
        })
        .cloned()
        .collect::<Vec<_>>();
    visible.sort_by_key(|v| !truthy(&v["skills_source"]));
    dom::clear("resources");
    for v in &visible {
        let a = dom::el("a", "resource-row", "");
        let _ = a.set_attribute(
            "href",
            &format!("#resource/{}", dom::enc(&m::text(v, "id"))),
        );
        let info = dom::el("div", "resource-info", "");
        dom::add(&info, &dom::el("h2", "resource-title", &m::text(v, "name")));
        dom::add(
            &info,
            &dom::el("p", "resource-description", &m::text(v, "description")),
        );
        dom::add(&a, &info);
        dom::add(
            &a,
            &dom::el("span", "category", &category(&m::text(v, "category"))),
        );
        dom::add(
            &a,
            &dom::el(
                "span",
                "clients",
                &m::array(&v["tool_compatibility"])
                    .iter()
                    .map(|v| client(v.as_str().unwrap_or("")))
                    .collect::<Vec<_>>()
                    .join(" · "),
            ),
        );
        dom::add(
            &a,
            &dom::el(
                "span",
                &format!("review {}", status(v)),
                status_label(status(v)),
            ),
        );
        dom::append("resources", &a);
    }
    dom::text(
        "result-count",
        &format!(
            "{} resources{}",
            visible.len(),
            if visible.len() == items.len() {
                String::new()
            } else {
                format!(" of {}", items.len())
            }
        ),
    );
    dom::hidden("empty", !visible.is_empty());
    dom::hidden(
        "reset",
        q.is_empty() && cat.is_empty() && cli.is_empty() && review.is_empty(),
    );
}
fn route(focus: bool) {
    let hash = dom::window().location().hash().unwrap_or_default();
    let hash = hash.trim_start_matches('#');
    let detail = hash.starts_with("resource/");
    let how = hash == "how-it-works";
    dom::hidden("catalog-view", detail || how);
    dom::hidden("detail-view", !detail);
    dom::hidden("how-view", !how);
    if let Some(e) = dom::get("catalog-nav") {
        if !how {
            let _ = e.set_attribute("aria-current", "page");
        } else {
            let _ = e.remove_attribute("aria-current");
        }
    }
    if let Some(e) = dom::get("how-nav") {
        if how {
            let _ = e.set_attribute("aria-current", "page");
        } else {
            let _ = e.remove_attribute("aria-current");
        }
    }
    let mut title = if how {
        "How it works".into()
    } else {
        "Tools for your agents".into()
    };
    if detail {
        let id = js_sys::decode_uri_component(hash.trim_start_matches("resource/"))
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        let resource = resources().into_iter().find(|x| x["id"] == id);
        title = resource
            .as_ref()
            .map(|x| m::text(x, "name"))
            .unwrap_or_else(|| "Resource not found".into());
        render_detail(resource);
    }
    dom::document().set_title(&format!("{title} — Open Source Kit"));
    if focus && hash != "main" {
        dom::focus("main");
        dom::window().scroll_to_with_x_and_y(0., 0.);
    }
}
fn render_detail(resource: Option<V>) {
    dom::clear("detail-view");
    let back = dom::el("a", "back", "← Back to catalog");
    let _ = back.set_attribute("href", "#");
    dom::append("detail-view", &back);
    let Some(v) = resource else {
        dom::append("detail-view", &dom::el("h1", "", "Resource not found"));
        dom::append(
            "detail-view",
            &dom::el(
                "p",
                "detail-intro",
                "This link does not match a resource in the current catalog.",
            ),
        );
        return;
    };
    dom::append("detail-view", &dom::el("h1", "", &m::text(&v, "name")));
    dom::append(
        "detail-view",
        &dom::el("p", "detail-intro", &m::text(&v, "description")),
    );
    let meta = dom::el("div", "detail-meta", "");
    dom::add(
        &meta,
        &dom::el("span", "category", &category(&m::text(&v, "category"))),
    );
    dom::add(
        &meta,
        &dom::el(
            "span",
            &format!("review {}", status(&v)),
            status_label(status(&v)),
        ),
    );
    let link = dom::el("a", "external", "View upstream");
    let url = m::text(&v, "url");
    if url.starts_with("https://") || url.starts_with("http://") {
        let _ = link.set_attribute("href", &url);
        let _ = link.set_attribute("target", "_blank");
        let _ = link.set_attribute("rel", "noopener noreferrer");
    }
    dom::add(&meta, &link);
    dom::append("detail-view", &meta);
    let grid = dom::el("div", "detail-grid", "");
    let content = dom::el("div", "", "");
    let aside = dom::el("aside", "", "");
    dom::add(
        &content,
        &dom::el(
            "h2",
            "",
            if truthy(&v["skills_source"]) {
                "Install skills"
            } else {
                "Open the installation guide"
            },
        ),
    );
    dom::add(&content, &terminal(&m::text(&v, "command")));
    dom::add(
        &content,
        &dom::el(
            "p",
            "subtle",
            if truthy(&v["skills_source"]) {
                "Run from your Kit 3 checkout. Add --target /path/to/project to choose the project; use --agent cursor or --agent claude-code for another client. This installs skills only."
            } else {
                "This resource uses a manual installation path. The command opens its guide; it does not install the resource."
            },
        ),
    );
    dom::add(
        &content,
        &dom::el("h3", "", "Upstream installation guidance"),
    );
    dom::add(
        &content,
        &dom::el("pre", "guidance", &m::text(&v, "install_guidance")),
    );
    let review = dom::el("section", "review-section", "");
    dom::add(&review, &dom::el("h2", "", "Review & evidence"));
    dom::add(&review,&dom::el("p","",v["dossier"]["status_summary"].as_str().unwrap_or("No registry review is recorded. Catalog descriptions and compatibility are reference information, not an approval.")));
    if let Some(notes) = v["risk_notes"].as_str() {
        dom::add(&review, &dom::el("h3", "", "Catalog notes"));
        dom::add(&review, &dom::el("p", "", notes));
    }
    if let Some(gates) = v["dossier"]["binary_gates"].as_object() {
        for (name, gate) in gates {
            let d = dom::details(&m::label(name), &m::text(gate, "evidence"));
            dom::add(
                &d,
                &dom::el(
                    "span",
                    "gate-status",
                    match gate["status"].as_str() {
                        Some("unknown") => "Open question",
                        Some("fail") => "Risk recorded",
                        _ => "Passed",
                    },
                ),
            );
            if let Some(text) = gate["resolves_by"].as_str() {
                dom::add(&d, &dom::el("h3", "", "What would resolve it"));
                dom::add(&d, &dom::el("p", "", text));
            }
            dom::add(&review, &d);
        }
    }
    dom::add(&content, &review);
    dom::add(&aside, &dom::el("h2", "", "Resource details"));
    let facts = dom::el("dl", "facts", "");
    let clients = m::array(&v["tool_compatibility"])
        .iter()
        .map(|x| client(x.as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join(" · ");
    for (label, value) in [
        (
            "Supported clients",
            if clients.is_empty() {
                "Discovery only".into()
            } else {
                clients
            },
        ),
        ("Install method", m::text(&v, "install_type")),
        (
            "License",
            v["license"]
                .as_str()
                .unwrap_or("Not confirmed — check upstream")
                .into(),
        ),
        ("Automation", m::text(&v, "automation_depth")),
        ("Context footprint", m::text(&v, "token_footprint")),
        ("Catalog reviewed", m::text(&v, "reviewed_date")),
        ("Command checked", m::text(&v, "command_verified_date")),
        ("Registry reviewed", m::text(&v["dossier"], "reviewed_date")),
        ("Review scope", m::text(&v["dossier"], "reviewed_by")),
    ] {
        let group = dom::el("div", "", "");
        dom::add(&group, &dom::el("dt", "", label));
        dom::add(
            &group,
            &dom::el(
                "dd",
                "",
                if value.is_empty() {
                    "Not recorded"
                } else {
                    &value
                },
            ),
        );
        dom::add(&facts, &group);
    }
    dom::add(&aside, &facts);
    dom::add(&grid, &content);
    dom::add(&grid, &aside);
    dom::append("detail-view", &grid);
}
