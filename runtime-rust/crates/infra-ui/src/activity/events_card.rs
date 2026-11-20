use crate::{dom, model as m};
use serde_json::{json, Value as V};

pub(crate) const EVENT_FILTER_KINDS: &[(&str, &str)] = &[
    ("all", "All"),
    ("error", "Errors"),
    ("permission", "Permissions"),
    ("tool", "Tools"),
    ("status", "Status"),
];

/// Exponential backoff delay in milliseconds for transient polling errors:
/// attempt 0 -> 1s, 1 -> 2s, 2 -> 4s, >=3 -> 8s.
pub(crate) fn retry_backoff_ms(attempt: u32) -> i32 {
    1000_i32.saturating_mul(1_i32 << attempt.min(3))
}

/// Terminal HTTP client errors where retrying the same poll cannot succeed.
pub(crate) fn is_terminal_poll_status(status: u16) -> bool {
    matches!(status, 401 | 403 | 404 | 410)
}

/// Whether the multiplexed `/api/events/stream` SSE connection is active.
pub(crate) fn sse_is_connected() -> bool {
    crate::api::sse_connected() || m::yes(&m::data("sse-connected"))
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn set_sse_connected_for_tests(connected: bool) {
    m::set("sse-connected", json!(connected));
}

/// Dispatch a normalized failure localization payload across map surfaces.
/// Matches `@ui-chat` (`#20`/`#22`) and `@ui-dep` (`#23`) JSON contract:
/// `{"project_id", "session_id"?, "config_path"?, "run_id"?, "service_id"?,
///   "failing_queries": [...], "hint_paths": [...], "prefer_full_modal": bool}`.
pub(crate) fn localize_failure_payload(req: &V) {
    m::set("last-localize-failure-request", req.clone());
    crate::session::localize_failure_on_map(req);
}

#[cfg(test)]
pub(crate) fn escape_html(raw: &str) -> String {
    crate::dom::escape_html(raw)
}

fn looks_like_file_path(token: &str) -> bool {
    let clean = token.trim_matches(|c: char| {
        matches!(
            c,
            '\'' | '"' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
        )
    });
    if clean.is_empty()
        || clean.contains("://")
        || clean.starts_with("http")
        || !clean.contains('/') && !clean.contains('.')
    {
        return false;
    }
    let path_part = clean.split(':').next().unwrap_or(clean);
    if path_part.len() < 3 || path_part.starts_with("--") {
        return false;
    }
    let has_ext = [
        ".php", ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".yml", ".yaml", ".json", ".twig",
        ".sql", ".sh", ".md", ".toml", ".css", ".html",
    ]
    .iter()
    .any(|ext| path_part.ends_with(ext));
    has_ext || path_part.contains('/')
}

pub(crate) fn extract_hint_paths(texts: &[&str], extra_paths: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut push_path = |raw: &str| {
        let stripped = raw
            .trim()
            .trim_matches(|c: char| {
                matches!(
                    c,
                    '\'' | '"' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
                )
            })
            .split(':')
            .next()
            .unwrap_or("")
            .trim_start_matches("./");
        if !stripped.is_empty()
            && !out.iter().any(|existing| existing == stripped)
            && out.len() < 12
        {
            out.push(stripped.to_owned());
        }
    };
    for p in extra_paths {
        if !p.trim().is_empty() {
            push_path(p);
        }
    }
    for text in texts {
        for word in text.split_whitespace() {
            if looks_like_file_path(word) {
                push_path(word);
            }
        }
    }
    out
}

fn push_failure_query(out: &mut Vec<String>, line: &str) {
    let trimmed = line.trim();
    if trimmed.is_empty() || out.iter().any(|q| q == trimmed) || out.len() >= 8 {
        return;
    }
    let clipped: String = trimmed.chars().take(160).collect();
    out.push(clipped);
}

pub(crate) fn extract_failure_queries(texts: &[&str], fallback: &str) -> Vec<String> {
    let mut out = Vec::new();
    for text in texts {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let lower = trimmed.to_lowercase();
            let high_signal = lower.contains("error")
                || lower.contains("fail")
                || lower.contains("exception")
                || lower.contains("assert")
                || lower.contains("panic")
                || lower.contains("undefined")
                || lower.contains("cannot ")
                || trimmed.contains("::")
                || looks_like_file_path(trimmed);
            if high_signal {
                push_failure_query(&mut out, trimmed);
            }
        }
    }
    if out.is_empty() {
        for text in texts {
            for line in text.lines().rev() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    push_failure_query(&mut out, trimmed);
                    if out.len() >= 4 {
                        break;
                    }
                }
            }
        }
    }
    if out.is_empty() && !fallback.trim().is_empty() {
        push_failure_query(&mut out, fallback);
    }
    out
}

pub(crate) fn normalize_event_kind(raw: &str) -> &'static str {
    let k = raw.trim().to_ascii_lowercase();
    if k.contains("err") || k.contains("fail") || k == "blocked" {
        "error"
    } else if k.contains("perm") || k.contains("approv") || k.contains("wait") {
        "permission"
    } else if k.contains("tool")
        || k.contains("cmd")
        || k.contains("command")
        || k.contains("shell")
    {
        "tool"
    } else if k.contains("think") || k.contains("reason") {
        "thinking"
    } else if k.contains("step") || k.contains("stage") || k.contains("phase") {
        "step"
    } else if k.contains("stat")
        || k.contains("start")
        || k.contains("complet")
        || k.contains("progress")
        || k.contains("queue")
        || k.contains("plan")
    {
        "status"
    } else {
        "text"
    }
}

pub(crate) fn active_event_filter(scope: &str) -> (String, String) {
    let saved = m::data(&format!("{scope}-filter"));
    let kind = saved["kind"]
        .as_str()
        .filter(|k| EVENT_FILTER_KINDS.iter().any(|(id, _)| id == k))
        .unwrap_or("all")
        .to_owned();
    let query = saved["query"].as_str().unwrap_or("").to_owned();
    (kind, query)
}

pub(crate) fn set_event_filter_kind(scope: &str, kind: &str) {
    let (_, query) = active_event_filter(scope);
    let next = if EVENT_FILTER_KINDS.iter().any(|(id, _)| *id == kind) {
        kind
    } else {
        "all"
    };
    m::set(
        &format!("{scope}-filter"),
        json!({ "kind": next, "query": query }),
    );
}

pub(crate) fn set_event_filter_query(scope: &str, query: &str) {
    let (kind, _) = active_event_filter(scope);
    m::set(
        &format!("{scope}-filter"),
        json!({ "kind": kind, "query": query }),
    );
}

pub(crate) fn build_structured_event_items(
    raw_events: &[V],
    filter_kind: &str,
    query: &str,
) -> (usize, Vec<V>) {
    let q = query.trim().to_ascii_lowercase();
    let total = raw_events.len();
    let mut items = Vec::new();
    for (idx, ev) in raw_events.iter().enumerate() {
        let raw_kind = ["kind", "type", "level"]
            .iter()
            .find_map(|k| ev[*k].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("status");
        let norm_kind = normalize_event_kind(raw_kind);
        let body = ["text", "message", "summary", "detail", "title"]
            .iter()
            .find_map(|k| ev[*k].as_str().filter(|s| !s.trim().is_empty()))
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if ev.is_string() {
                    ev.as_str().unwrap_or("").to_owned()
                } else {
                    m::summary(ev)
                }
            });
        if body.trim().is_empty() && raw_kind == "user" {
            continue;
        }
        let service = m::text(ev, "service");
        let step = m::text(ev, "step");
        let path = m::text(ev, "path");
        let time = ["time", "timestamp", "created_at", "ts", "at"]
            .iter()
            .find_map(|k| ev[*k].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("")
            .to_owned();
        if filter_kind != "all" && !filter_kind.is_empty() && norm_kind != filter_kind {
            continue;
        }
        if !q.is_empty() {
            let matches = body.to_ascii_lowercase().contains(&q)
                || raw_kind.to_ascii_lowercase().contains(&q)
                || norm_kind.contains(&q)
                || service.to_ascii_lowercase().contains(&q)
                || step.to_ascii_lowercase().contains(&q)
                || path.to_ascii_lowercase().contains(&q);
            if !matches {
                continue;
            }
        }
        let mut meta_parts = Vec::new();
        if !service.is_empty() {
            meta_parts.push(service);
        }
        if !step.is_empty() {
            meta_parts.push(step);
        }
        if !path.is_empty() {
            meta_parts.push(path);
        }
        let seq_or_id = ev["id"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| ev["seq"].as_u64().map(|n| n.to_string()))
            .unwrap_or_else(|| idx.to_string());
        items.push(json!({
            "key": format!("ev:{seq_or_id}:{norm_kind}"),
            "kind": norm_kind,
            "badge": m::label(raw_kind),
            "meta": meta_parts.join(" · "),
            "time": time,
            "body": body,
        }));
    }
    (total, items)
}

fn ensure_structured_events_chrome(scope: &str, filter_kind: &str, query: &str) {
    let Some(host) = dom::get(scope) else {
        return;
    };
    let toolbar_id = format!("{scope}-toolbar");
    let list_id = format!("{scope}-list");
    let empty_id = format!("{scope}-empty");
    if dom::get(&toolbar_id).is_none() {
        host.set_text_content(None);
        let toolbar = dom::el("div", "structured-events-toolbar", "");
        let _ = toolbar.set_attribute("id", &toolbar_id);

        let filter_row = dom::el("div", "structured-events-filter-row", "");
        let _ = filter_row.set_attribute("id", &format!("{scope}-filters"));
        let _ = filter_row.set_attribute("role", "group");
        let _ = filter_row.set_attribute("aria-label", "Filter event kinds");
        for (fid, label) in EVENT_FILTER_KINDS {
            let btn = dom::button(
                label,
                "structured-events-kind",
                json!({ "scope": scope, "kind": fid }),
            );
            btn.set_class_name("run-filter");
            let _ = btn.set_attribute("data-kind", fid);
            let _ = btn.set_attribute(
                "aria-pressed",
                if *fid == filter_kind { "true" } else { "false" },
            );
            dom::add(&filter_row, &btn);
        }
        dom::add(&toolbar, &filter_row);

        let search = dom::el("input", "input structured-events-search", "");
        let _ = search.set_attribute("id", &format!("{scope}-search"));
        let _ = search.set_attribute("type", "search");
        let _ = search.set_attribute("placeholder", "Filter events…");
        let _ = search.set_attribute("aria-label", "Filter events");
        let _ = search.set_attribute("data-action", "structured-events-search");
        let _ = search.set_attribute("data-payload", &json!({ "scope": scope }).to_string());
        if !query.is_empty() {
            dom::set_element_value(&search, query);
        }
        dom::add(&toolbar, &search);
        dom::add(&host, &toolbar);

        let empty = dom::el("p", "structured-event-empty", "");
        let _ = empty.set_attribute("id", &empty_id);
        dom::hide(&empty, true);
        dom::add(&host, &empty);

        let list = dom::el("div", "structured-event-list", "");
        let _ = list.set_attribute("id", &list_id);
        dom::add(&host, &list);
    } else {
        if let Some(filters) = dom::get(&format!("{scope}-filters")) {
            for btn in dom::children(&filters) {
                let kind = btn.get_attribute("data-kind").unwrap_or_default();
                let _ = btn.set_attribute(
                    "aria-pressed",
                    if kind == filter_kind { "true" } else { "false" },
                );
            }
        }
        let search_id = format!("{scope}-search");
        if dom::value(&search_id) != query {
            if let Some(el) = dom::get(&search_id) {
                let is_focused = dom::document()
                    .active_element()
                    .is_some_and(|active| active == el);
                if !is_focused {
                    dom::set_element_value(&el, query);
                }
            }
        }
    }
}

fn build_structured_event_card(item: &V) -> web_sys::Element {
    let kind = m::text(item, "kind");
    let badge_text = m::text(item, "badge");
    let meta_text = m::text(item, "meta");
    let time_text = m::text(item, "time");
    let body_text = m::text(item, "body");

    let card = dom::el("article", "structured-event-card", "");
    let _ = card.set_attribute("data-kind", &kind);

    let head = dom::el("div", "structured-event-head", "");
    let badge = dom::el("span", "structured-event-badge", &badge_text);
    let _ = badge.set_attribute("data-kind", &kind);
    dom::add(&head, &badge);

    if !meta_text.is_empty() {
        dom::add(&head, &dom::el("span", "structured-event-meta", &meta_text));
    }
    if !time_text.is_empty() {
        dom::add(&head, &dom::el("span", "structured-event-time", &time_text));
    }
    dom::add(&card, &head);
    dom::add(&card, &dom::el("div", "structured-event-body", &body_text));
    card
}

pub(crate) fn render_structured_events(scope: &str, raw_events: &[V]) {
    let (filter_kind, query) = active_event_filter(scope);
    let (total, items) = build_structured_event_items(raw_events, &filter_kind, &query);
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    ensure_structured_events_chrome(scope, &filter_kind, &query);
    let list_id = format!("{scope}-list");
    dom::keyed_render(
        &list_id,
        &items,
        |item| m::text(item, "key"),
        build_structured_event_card,
    );
    let empty_id = format!("{scope}-empty");
    if items.is_empty() {
        let msg = if total == 0 {
            "No events recorded yet."
        } else {
            "No events match the active filter."
        };
        dom::text(&empty_id, msg);
        dom::hidden(&empty_id, false);
    } else {
        dom::hidden(&empty_id, true);
    }
}

/// Merge an incremental SSE event into a cached `events` array without duplicates.
pub(crate) fn merge_sse_event(events: &mut Vec<V>, ev: V) -> bool {
    if !ev.is_object() && !ev.is_string() {
        return false;
    }
    let id = m::text(&ev, "id");
    let seq = ev["seq"].as_u64();
    let kind = m::text(&ev, "kind");
    let text = m::text(&ev, "text");
    let ts = m::text(&ev, "time");
    let already = events.iter().rev().take(24).any(|existing| {
        if !id.is_empty() && m::text(existing, "id") == id {
            return true;
        }
        if seq.is_some() && existing["seq"].as_u64() == seq {
            return true;
        }
        m::text(existing, "kind") == kind
            && m::text(existing, "text") == text
            && m::text(existing, "time") == ts
    });
    if already {
        return false;
    }
    events.push(ev);
    true
}

#[cfg(test)]
pub(crate) fn render_structured_events_html_for_test(
    scope: &str,
    raw_events: &[V],
    filter_kind: &str,
    query: &str,
) -> String {
    let (total, items) = build_structured_event_items(raw_events, filter_kind, query);
    let mut out = format!(
        "<div id=\"{}\" class=\"structured-events-panel\">",
        escape_html(scope)
    );
    out.push_str("<div class=\"structured-events-toolbar\"><div class=\"structured-events-filter-row\" role=\"group\">");
    for (fid, label) in EVENT_FILTER_KINDS {
        let payload = escape_html(&json!({ "scope": scope, "kind": fid }).to_string());
        out.push_str(&format!(
            "<button type=\"button\" class=\"run-filter\" data-action=\"structured-events-kind\" data-kind=\"{fid}\" data-payload=\"{payload}\" aria-pressed=\"{}\">{label}</button>",
            if *fid == filter_kind { "true" } else { "false" }
        ));
    }
    out.push_str("</div></div><div class=\"structured-event-list\">");
    if items.is_empty() {
        let msg = if total == 0 {
            "No events recorded yet."
        } else {
            "No events match the active filter."
        };
        out.push_str(&format!("<p class=\"structured-event-empty\">{msg}</p>"));
    } else {
        for item in &items {
            let kind = escape_html(&m::text(item, "kind"));
            let badge = escape_html(&m::text(item, "badge"));
            let meta = escape_html(&m::text(item, "meta"));
            let time = escape_html(&m::text(item, "time"));
            let body = escape_html(&m::text(item, "body"));
            out.push_str(&format!(
                "<article class=\"structured-event-card\" data-kind=\"{kind}\"><div class=\"structured-event-head\"><span class=\"structured-event-badge\" data-kind=\"{kind}\">{badge}</span>"
            ));
            if !meta.is_empty() {
                out.push_str(&format!(
                    "<span class=\"structured-event-meta\">{meta}</span>"
                ));
            }
            if !time.is_empty() {
                out.push_str(&format!(
                    "<span class=\"structured-event-time\">{time}</span>"
                ));
            }
            out.push_str(&format!(
                "</div><div class=\"structured-event-body\">{body}</div></article>"
            ));
        }
    }
    out.push_str("</div></div>");
    out
}
