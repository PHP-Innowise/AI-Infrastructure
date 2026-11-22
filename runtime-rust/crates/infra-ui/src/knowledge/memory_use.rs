use super::{
    analytics_slice, bytes, horizon_bin_days, horizon_ticks, usage::history_day_heights,
    viewport_is_mobile,
};
use crate::{api, dom, model as m};
use serde_json::{json, Value as V};
use std::collections::{BTreeMap, BTreeSet};
use wasm_bindgen::JsCast;

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
pub(super) fn memory_model(v: &V, window: u64) -> V {
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
    let p = m::current_project();
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
        Ok((e, v)) if m::current("memory-use", e) && m::current_project() == p => {
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
pub(super) fn metric_label(key: &str) -> String {
    let label = m::label(key);
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => label,
    }
}

pub(super) fn memory_stage_metrics(
    stage: &str,
    stage_model: &V,
) -> Option<Vec<(String, String, bool)>> {
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

pub(super) fn set_text_if_changed(id: &str, value: &str) {
    if dom::get(id).and_then(|e| e.text_content()).as_deref() != Some(value) {
        dom::text(id, value);
    }
}

pub(super) fn memory_chunk_matches_ui_filter(chunk: &V, today: &str, mode: &str) -> bool {
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

pub(super) fn build_memory_chunk_items(chunks: &[V], today: &str, mode: &str) -> Vec<V> {
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

pub(super) fn build_memory_trace_items(rows: &[V], feedback: &V, supported: bool) -> Vec<V> {
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

pub(super) fn ensure_memory_layout() {
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

pub(super) fn sync_memory_promotions(v: &V, window: u64) {
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
    let base_sig = dom::json_signature(&json!([items, since]));
    let full_sig = dom::json_signature(&json!([items, since, reviewer, busy, status]));
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

pub(super) fn render_memory() {
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
        m::current_project(),
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

/// Where this browser keeps the reviewer name a person last typed on Memory
/// use.
pub(super) const PROMOTION_REVIEWER_KEY: &str = "harness.memory-reviewer.v1";
pub(super) fn promotion_reviewer() -> String {
    match m::data("memory-promotion-reviewer").as_str() {
        Some(name) => name.to_owned(),
        None => dom::storage(PROMOTION_REVIEWER_KEY).unwrap_or_default(),
    }
}
/// Approve, reject or apply one proposal through the knowledge operation,
/// then read Memory use again.
pub(super) async fn promotion_action(action: &str, payload: &V) {
    let Some(id) = payload["promotion_id"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let project = m::current_project();
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
    if m::current_project() == project {
        // Drawn again at once, so a failed reload leaves no button waiting.
        render_memory();
        load_memory_use().await;
    }
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
pub(super) fn style(e: &web_sys::Element, name: &str, value: &str) {
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
pub(super) fn memory_table_sort_dir(
    sort: &V,
    default_field: &str,
    field: &str,
) -> Option<&'static str> {
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

const RETRIEVAL_KINDS: [(&str, &str); 3] = [
    ("brain", "Project Brain"),
    ("bank", "Memory bank"),
    ("rules", "Rules & docs"),
];

pub(super) fn retrieval_strip_column_max(shown: &[V]) -> [u64; 3] {
    let mut maxes = [1u64; 3];
    for r in shown {
        for (idx, (kind, _)) in RETRIEVAL_KINDS.iter().enumerate() {
            let count = r[*kind].as_u64().unwrap_or(0)
                + if *kind == "bank" {
                    m::array(&r["cuts"]).len() as u64
                } else {
                    0
                };
            maxes[idx] = maxes[idx].max(count);
        }
    }
    maxes
}

pub(super) fn sync_retrieval_strip_selection(section: &web_sys::Element, selected_pos: usize) {
    if let Ok(cols) = section.query_selector_all(".memory-column[data-position]") {
        let target_pos = selected_pos.to_string();
        for i in 0..cols.length() {
            if let Some(col) = cols
                .item(i)
                .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
            {
                let selected =
                    col.get_attribute("data-position").as_deref() == Some(target_pos.as_str());
                let _ = col.set_attribute("tabindex", if selected { "0" } else { "-1" });
                let _ = col.set_attribute("aria-selected", if selected { "true" } else { "false" });
            }
        }
    }
}

pub(super) fn analytics(data: &V, _model: &V) {
    let raw = m::array(&data["retrievals"]["items"]);
    let mobile = viewport_is_mobile();
    let shown = analytics_slice(&raw, mobile);
    let offset = raw.len() - shown.len();

    if let Some(section) = dom::get("memory-use-strip") {
        let selected_pos = m::data("memory-pinned-retrieval")
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or_else(|| raw.len().saturating_sub(1));
        let base_sig = dom::json_signature(&json!([shown, mobile]));
        if section.get_attribute("data-base-signature").as_deref() == Some(base_sig.as_str())
            && section
                .query_selector(".memory-strip-columns")
                .ok()
                .flatten()
                .is_some()
        {
            sync_retrieval_strip_selection(&section, selected_pos);
            if !raw.is_empty() {
                retrieval_detail(selected_pos);
            }
        } else {
            let _ = section.set_attribute("data-base-signature", &base_sig);
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
            let most_by_kind = retrieval_strip_column_max(shown);
            for ((_, label), most) in RETRIEVAL_KINDS.iter().zip(most_by_kind) {
                let e = dom::el("span", "memory-strip-label", label);
                style(&e, "--row", &format!("{}px", most * 4));
                dom::add(&labels, &e);
            }
            for (index, row) in shown.iter().enumerate() {
                let position = index + offset;
                let col = dom::button("", "memory-retrieval", json!(position));
                col.set_class_name("memory-column");
                let _ = col.set_attribute("role", "option");
                let _ = col.set_attribute("data-position", &position.to_string());
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
                for ((kind, _), most) in RETRIEVAL_KINDS.iter().zip(most_by_kind) {
                    let cell = dom::el("span", "memory-cell", "");
                    let _ = cell.set_attribute("data-kind", kind);
                    style(&cell, "--row", &format!("{}px", most * 4));
                    let cuts = m::array(&row["cuts"]);
                    if *kind == "bank" && !cuts.is_empty() {
                        let cut_el = dom::el("span", "memory-units cut", "");
                        style(&cut_el, "--units", &format!("{}px", cuts.len() * 4));
                        dom::add(&cell, &cut_el);
                    }
                    let units = dom::el("span", "memory-units", "");
                    let count = row[*kind].as_u64().unwrap_or(0);
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
        let base_sig = dom::json_signature(&json!([
            chunks,
            data["today"],
            data["retrievals"]["items"],
            data["history"]["days"],
            mobile
        ]));
        let filters_sig = dom::json_signature(&json!(filters));
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
            let bin = horizon_bin_days(mobile);
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
            for &day in horizon_ticks(mobile) {
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
pub(super) fn sortable(kind: &str, rows: &[V], columns: &[(&str, &str)]) {
    let sort = m::data(&format!("memory-sort-{kind}"));
    let Some(details) = dom::get(&format!("memory-table-{kind}")) else {
        return;
    };
    let table_sig = dom::json_signature(&json!([rows, sort]));
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
pub(super) fn retrieval_detail(index: usize) {
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
pub(super) fn chunk_detail(chunk: &V) {
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

pub(super) fn memory_changes(data: &V, before: &V) {
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
