use super::{usage_page_size, viewport_is_mobile};
use crate::{dom, model as m};
use serde_json::{json, Value as V};
use wasm_bindgen::JsCast;

pub(super) const CAPSULE_KINDS: [(&str, &str); 3] = [
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
pub(super) fn capsule_bar_label(meter: &V) -> String {
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
pub(super) const USAGE_KINDS: [(&str, &str); 7] = [
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
pub(super) fn turn_part(turn: &V, kind: &str) -> f64 {
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
pub(super) fn context_turn_roving_states(shown: &[V], selected_id: &V) -> Vec<(String, bool, &'static str)> {
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

pub(super) fn build_usage_detail_items(turn: &V, turns: &[V]) -> Vec<V> {
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

pub(super) fn ensure_usage_layout() {
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

pub(super) fn sync_context_turns_selection(shown: &[V], selected_id: &V) {
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

pub(super) fn render_usage() {
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
    let size = usage_page_size(viewport_is_mobile());
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
