use super::*;
use crate::api;

/// Whether a request of the selected session waits for the person.
pub(crate) fn permission_waiting() -> bool {
    m::active(&m::read(|s| s.selected.clone()))
        && m::data("permission-pending")
            .as_object()
            .is_some_and(|pending| !pending.is_empty())
}

pub(super) fn permission_tool(event: &V) -> String {
    let tool = m::text(event, "tool");
    if tool.is_empty() {
        "Tool".to_owned()
    } else {
        tool
    }
}

pub(crate) fn permission_button_aria_label(decision: &str, label: &str, tool: &str) -> String {
    match decision {
        "allow" => format!("Allow {tool}"),
        "allow_session" => format!("Allow {tool} for this session"),
        "deny" => format!("Deny {tool}"),
        _ => format!("{label} {tool}"),
    }
}

pub(super) fn permission_choices(session_option: bool) -> &'static [(&'static str, &'static str)] {
    if session_option {
        &[
            ("allow", "Allow"),
            ("allow_session", "Allow for this session"),
            ("deny", "Deny"),
        ]
    } else {
        &[("allow", "Allow"), ("deny", "Deny")]
    }
}

pub(crate) fn is_question_permission(event: &V) -> bool {
    event["ask_kind"].as_str() == Some("question")
        && event["questions"]
            .as_array()
            .is_some_and(|questions| !questions.is_empty())
}

pub(crate) fn permission_rule_hint_text(event: &V) -> String {
    let summary = clean(&m::text(event, "rule_summary"));
    if summary.is_empty() {
        return String::new();
    }
    if m::yes(&event["session_option"]) {
        format!("Allow for this session: {summary}")
    } else {
        summary
    }
}

pub(crate) fn question_answer_key(question: &V, idx: usize) -> String {
    question["id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            question["question"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            question["header"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .map(str::to_owned)
        .unwrap_or_else(|| format!("q{idx}"))
}

pub(crate) fn question_is_multi_select(question: &V) -> bool {
    question["multi_select"]
        .as_bool()
        .or_else(|| question["multiSelect"].as_bool())
        .unwrap_or(false)
}

pub(crate) fn question_allows_other(question: &V) -> bool {
    question["is_other"]
        .as_bool()
        .or_else(|| question["isOther"].as_bool())
        .unwrap_or(true)
}

pub(crate) fn permission_decide_payload(container_id: &str, request_id: &str, decision: &str) -> V {
    if container_id == "events" {
        json!({"request_id": request_id, "decision": decision})
    } else {
        json!({"request_id": request_id, "decision": decision, "container": container_id})
    }
}

pub(crate) fn permission_question_other_payload(
    container_id: &str,
    request_id: &str,
    index: usize,
) -> V {
    if container_id == "events" {
        json!({"request_id": request_id, "index": index})
    } else {
        json!({"request_id": request_id, "index": index, "container": container_id})
    }
}

pub(crate) fn question_radio_group_name(request_id: &str, idx: usize) -> String {
    let safe_id: String = request_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("pq-{safe_id}-{idx}")
}

pub(crate) fn question_radio_group_name_in(
    container_id: &str,
    request_id: &str,
    idx: usize,
) -> String {
    if container_id == "events" {
        return question_radio_group_name(request_id, idx);
    }
    let safe_container: String = container_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = question_radio_group_name(request_id, idx);
    let suffix = base.strip_prefix("pq-").unwrap_or(&base);
    format!("pq-{safe_container}-{suffix}")
}

pub(crate) fn render_question_card_html_in(
    container_id: &str,
    request_id: &str,
    questions: &[V],
) -> String {
    let mut out = String::new();
    for (idx, q) in questions.iter().enumerate() {
        let key = question_answer_key(q, idx);
        let header = clean(&m::text(q, "header"));
        let prompt = clean(&m::text(q, "question"));
        let multi = question_is_multi_select(q);
        let options = m::array(&q["options"]);
        let allows_other = question_allows_other(q) || options.is_empty();
        let group_name = question_radio_group_name_in(container_id, request_id, idx);
        let input_type = if multi { "checkbox" } else { "radio" };
        out.push_str(&format!(
            "<fieldset class=\"permission-question\" data-question-index=\"{idx}\" data-question-key=\"{}\" data-multi-select=\"{multi}\">",
            files_map::escape_html(&key)
        ));
        if !header.is_empty() {
            out.push_str(&format!(
                "<legend class=\"permission-question-header\">{}</legend>",
                files_map::escape_html(&header)
            ));
        }
        if !prompt.is_empty() {
            out.push_str(&format!(
                "<p class=\"permission-question-prompt\">{}</p>",
                files_map::escape_html(&prompt)
            ));
        }
        out.push_str("<div class=\"permission-question-options\">");
        for opt in &options {
            let label = clean(opt["label"].as_str().or_else(|| opt.as_str()).unwrap_or(""));
            let desc = clean(&m::text(opt, "description"));
            if label.is_empty() {
                continue;
            }
            out.push_str("<label class=\"permission-question-option\">");
            out.push_str(&format!(
                "<input type=\"{input_type}\" class=\"permission-question-choice\" name=\"{}\" value=\"{}\" data-question-index=\"{idx}\">",
                files_map::escape_html(&group_name),
                files_map::escape_html(&label)
            ));
            out.push_str("<span class=\"permission-question-option-body\">");
            out.push_str(&format!(
                "<span class=\"permission-question-option-label\">{}</span>",
                files_map::escape_html(&label)
            ));
            if !desc.is_empty() {
                out.push_str(&format!(
                    "<span class=\"permission-question-option-desc\">{}</span>",
                    files_map::escape_html(&desc)
                ));
            }
            out.push_str("</span></label>");
        }
        if allows_other {
            if !multi && !options.is_empty() {
                out.push_str(
                    "<label class=\"permission-question-option permission-question-other-row\">",
                );
                out.push_str(&format!(
                    "<input type=\"radio\" class=\"permission-question-choice permission-question-other-radio\" name=\"{}\" value=\"__other__\" data-question-index=\"{idx}\">",
                    files_map::escape_html(&group_name)
                ));
                out.push_str("<span class=\"permission-question-option-body\"><span class=\"permission-question-option-label\">Other</span></span></label>");
            }
            let payload =
                permission_question_other_payload(container_id, request_id, idx).to_string();
            let placeholder = if options.is_empty() {
                "Type your answer…"
            } else {
                "Other (custom answer)…"
            };
            let aria_target = if !header.is_empty() {
                header.as_str()
            } else if !prompt.is_empty() {
                prompt.as_str()
            } else {
                ""
            };
            let aria_label = if aria_target.is_empty() {
                format!("Other answer for question {}", idx + 1)
            } else {
                format!("Other answer for {aria_target}")
            };
            out.push_str(&format!(
                "<input type=\"text\" class=\"permission-question-other-input\" data-question-index=\"{idx}\" data-action=\"permission-question-other\" data-payload=\"{}\" maxlength=\"2000\" placeholder=\"{placeholder}\" aria-label=\"{}\">",
                files_map::escape_html(&payload),
                files_map::escape_html(&aria_label)
            ));
        }
        out.push_str("</div></fieldset>");
    }
    out
}

#[cfg(test)]
pub(crate) fn render_question_card_html(request_id: &str, questions: &[V]) -> String {
    render_question_card_html_in("events", request_id, questions)
}

fn format_answer_value(val: &V) -> String {
    match val {
        V::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
            .collect::<Vec<_>>()
            .join(", "),
        V::String(s) => s.trim().to_owned(),
        _ => clean(&m::string(val)),
    }
}

pub(crate) fn render_answers_summary_html(questions: &[V], answers: &V) -> String {
    let Some(map) = answers.as_object() else {
        return String::new();
    };
    if map.is_empty() {
        return String::new();
    }
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut used_keys = std::collections::BTreeSet::new();
    for (idx, q) in questions.iter().enumerate() {
        let key = question_answer_key(q, idx);
        let alt_question = clean(&m::text(q, "question"));
        let found = map.get(&key).map(|v| (key.clone(), v)).or_else(|| {
            if !alt_question.is_empty() {
                map.get(&alt_question).map(|v| (alt_question.clone(), v))
            } else {
                None
            }
        });
        if let Some((matched_key, val)) = found {
            used_keys.insert(matched_key);
            let label = clean(&m::text(q, "header"));
            let title = if !label.is_empty() {
                label
            } else if !alt_question.is_empty() {
                alt_question
            } else {
                key
            };
            let formatted = format_answer_value(val);
            if !formatted.is_empty() {
                rows.push((title, formatted));
            }
        }
    }
    for (k, v) in map {
        if !used_keys.contains(k) {
            let formatted = format_answer_value(v);
            if !formatted.is_empty() {
                rows.push((k.clone(), formatted));
            }
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "<p class=\"permission-answers-heading\">Submitted answers</p><dl class=\"permission-answers-list\">",
    );
    for (label, val) in rows {
        out.push_str(&format!(
            "<div class=\"permission-answer-row\"><dt>{}</dt><dd>{}</dd></div>",
            files_map::escape_html(&label),
            files_map::escape_html(&val)
        ));
    }
    out.push_str("</dl>");
    out
}

pub(crate) fn build_permission_decision_body(decision: &str, answers: Option<V>) -> V {
    if decision == "allow" {
        if let Some(ans) = answers {
            return json!({"decision": "allow", "answers": ans});
        }
    }
    json!({"decision": decision})
}

pub(super) fn permission_notification_body(event: &V) -> String {
    let summary = clean(&m::text(event, "summary"));
    if !summary.is_empty() {
        return summary;
    }
    let detail = clean(&m::text(event, "detail"));
    if !detail.is_empty() {
        return detail;
    }
    "Waiting for your answer in Harness.".to_owned()
}

/// The heading of a permission request in each state it can reach.
pub(crate) fn permission_heading(event: &V) -> String {
    let tool = permission_tool(event);
    let host = match event["provider"].as_str() {
        Some("claude") => "Claude Code",
        Some("codex") => "Codex",
        Some("cursor") => "Cursor",
        _ => "The agent",
    };
    let is_question =
        is_question_permission(event) || event["ask_kind"].as_str() == Some("question");
    match event["state"].as_str().unwrap_or("") {
        "pending" => {
            if is_question {
                format!("{host} asks a question · {tool}")
            } else {
                format!("{host} asks permission · {tool}")
            }
        }
        "allowed" => {
            if is_question {
                format!("Answered · {tool}")
            } else {
                format!("Allowed · {tool}")
            }
        }
        "allowed_session" => format!("Allowed for this session · {tool}"),
        "denied" => {
            if is_question {
                format!("Declined · {tool}")
            } else {
                format!("Denied · {tool}")
            }
        }
        "declined" => format!("Declined automatically · {tool}"),
        "withdrawn" => format!("Withdrawn by {host} · {tool}"),
        "expired" => format!("No longer waiting · {tool}"),
        other => format!("{} · {tool}", m::label(other)),
    }
}

fn permission_child(
    block: &web_sys::Element,
    class: &str,
    role: Option<&str>,
) -> Option<web_sys::Element> {
    dom::children(block).into_iter().find(|child| {
        child.class_list().contains(class)
            && role.is_none_or(|expected| {
                child.get_attribute("data-role").as_deref() == Some(expected)
            })
    })
}

fn sync_permission_text_slot(
    block: &web_sys::Element,
    tag: &str,
    class: &str,
    role: Option<&str>,
    text: &str,
    before: Option<&web_sys::Element>,
) -> Option<web_sys::Element> {
    let existing = permission_child(block, class, role);
    if text.is_empty() {
        if let Some(el) = existing {
            el.remove();
        }
        return None;
    }
    match existing {
        Some(el) => {
            if el.text_content().as_deref() != Some(text) {
                el.set_text_content(Some(text));
            }
            Some(el)
        }
        None => {
            let el = dom::el(tag, class, text);
            if let Some(r) = role {
                let _ = el.set_attribute("data-role", r);
            }
            let _ = block.insert_before(&el, before.map(|b| b.as_ref()));
            Some(el)
        }
    }
}

fn notify_permission(event: &V) {
    if m::text(event, "state") != "pending" || !m::active(&m::read(|s| s.selected.clone())) {
        return;
    }
    if !dom::document().hidden()
        || web_sys::Notification::permission() != web_sys::NotificationPermission::Granted
    {
        return;
    }
    let sid = m::read(|s| s.sid.clone());
    let request = m::text(event, "request_id");
    if sid.is_empty() || request.is_empty() {
        return;
    }
    let key = format!("{sid}:{request}");
    let mut notified = m::data("permission-notified")
        .as_object()
        .cloned()
        .unwrap_or_default();
    if notified.contains_key(&key) {
        return;
    }
    if notified.len() >= 200 {
        notified.clear();
    }
    notified.insert(key, json!(true));
    m::set("permission-notified", V::Object(notified));
    let title = permission_heading(event);
    let body = permission_notification_body(event);
    let tag = format!("harness-permission-{sid}-{request}");
    fire_browser_notification(&title, &body, &tag, false, Some(request));
}

pub(crate) struct PermissionButtonSpec {
    pub(crate) decision: &'static str,
    pub(crate) label: &'static str,
    pub(crate) class_name: &'static str,
    pub(crate) payload: V,
    pub(crate) aria_label: String,
    pub(crate) is_question_submit: bool,
}

pub(crate) fn current_now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0)
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parses an RFC3339 timestamp string (e.g., `2026-11-19T12:05:30Z` or `+00:00`) into Unix epoch ms.
pub(crate) fn parse_rfc3339_utc_ms(raw: &str) -> Option<f64> {
    let s = raw.trim();
    if s.len() < 19 {
        return None;
    }
    let bytes = s.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' || !matches!(bytes[10], b'T' | b't' | b' ') {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let year: i64 = s[0..4].parse().ok()?;
    let month: i64 = s[5..7].parse().ok()?;
    let day: i64 = s[8..10].parse().ok()?;
    let hour: i64 = s[11..13].parse().ok()?;
    let min: i64 = s[14..16].parse().ok()?;
    let sec: i64 = s[17..19].parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&min)
        || !(0..=60).contains(&sec)
    {
        return None;
    }
    let mut rest = &s[19..];
    let mut millis = 0.0f64;
    if let Some(after_dot) = rest.strip_prefix('.') {
        let digits_len = after_dot.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digits_len == 0 {
            return None;
        }
        let frac_str = &after_dot[..digits_len.min(3)];
        let mut frac_val: f64 = frac_str.parse().ok()?;
        for _ in frac_str.len()..3 {
            frac_val *= 10.0;
        }
        millis = frac_val;
        rest = &after_dot[digits_len..];
    }
    let offset_secs: i64 = if rest.is_empty() || rest == "Z" || rest == "z" {
        0
    } else if rest.len() == 6
        && matches!(rest.as_bytes()[0], b'+' | b'-')
        && rest.as_bytes()[3] == b':'
    {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let oh: i64 = rest[1..3].parse().ok()?;
        let om: i64 = rest[4..6].parse().ok()?;
        sign * (oh * 3600 + om * 60)
    } else {
        return None;
    };
    let days = days_from_civil(year, month, day);
    let total_secs = days * 86_400 + hour * 3600 + min * 60 + sec - offset_secs;
    Some((total_secs as f64) * 1000.0 + millis)
}

// Caches first-seen `timeout_seconds` deadlines per request_id so polls count down continuously.
thread_local! {
    static TIMEOUT_DEADLINES: std::cell::RefCell<std::collections::BTreeMap<String, (f64, f64)>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// Computes approval expiry deadline in Unix epoch milliseconds when `expires_at` or `timeout_seconds` is provided.
pub(crate) fn resolve_permission_expiry_ms(
    cache: &mut std::collections::BTreeMap<String, (f64, f64)>,
    request_id: &str,
    event: &V,
    now_ms: f64,
) -> Option<f64> {
    if let Some(s) = event["expires_at"].as_str() {
        if let Some(ms) = parse_rfc3339_utc_ms(s) {
            return Some(ms);
        }
        if let Ok(n) = s.trim().parse::<f64>().map_err(|_| ()).and_then(|n| {
            if n.is_finite() && n > 0.0 {
                Ok(n)
            } else {
                Err(())
            }
        }) {
            return Some(if n < 10_000_000_000.0 { n * 1000.0 } else { n });
        }
    } else if let Some(n) = event["expires_at"]
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.0)
    {
        return Some(if n < 10_000_000_000.0 { n * 1000.0 } else { n });
    }
    if let Some(timeout_secs) = event["timeout_seconds"]
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.0)
    {
        if !request_id.is_empty() {
            if let Some(&(cached_timeout, deadline)) = cache.get(request_id) {
                if (cached_timeout - timeout_secs).abs() < f64::EPSILON {
                    return Some(deadline);
                }
            }
            let deadline = now_ms + timeout_secs * 1000.0;
            cache.insert(request_id.to_owned(), (timeout_secs, deadline));
            return Some(deadline);
        }
        return Some(now_ms + timeout_secs * 1000.0);
    }
    None
}

pub(crate) fn permission_expiry_ms(event: &V, now_ms: f64) -> Option<f64> {
    let request_id = m::text(event, "request_id");
    let state = m::text(event, "state");
    TIMEOUT_DEADLINES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !state.is_empty() && state != "pending" && !request_id.is_empty() {
            cache.remove(&request_id);
        }
        resolve_permission_expiry_ms(&mut cache, &request_id, event, now_ms)
    })
}

/// Formats remaining time before approval expiry into `(tone, display_label)`.
pub(crate) fn format_countdown_badge(remaining_ms: f64) -> (&'static str, String) {
    if remaining_ms <= 0.0 {
        return ("expired", "Expired".to_owned());
    }
    let total_secs = (remaining_ms / 1000.0).ceil() as u64;
    if total_secs <= 30 {
        ("urgent", format!("Expires in {total_secs}s"))
    } else if total_secs < 3600 {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        ("normal", format!("Expires in {mins}:{secs:02}"))
    } else {
        let hrs = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        ("normal", format!("Expires in {hrs}h {mins}m"))
    }
}

pub(crate) struct PermissionCardSpec {
    pub(crate) request_id: String,
    pub(crate) event_id: String,
    pub(crate) provider: String,
    pub(crate) tool: String,
    pub(crate) state: String,
    pub(crate) heading: String,
    pub(crate) summary: String,
    pub(crate) detail: String,
    pub(crate) raw_rule: String,
    pub(crate) rule_hint: String,
    pub(crate) ask_kind: String,
    pub(crate) diff_preview: String,
    pub(crate) expires_at_ms: Option<f64>,
    pub(crate) is_question: bool,
    pub(crate) questions: Vec<V>,
    pub(crate) questions_sig: String,
    pub(crate) actions_key: String,
    pub(crate) buttons: Vec<PermissionButtonSpec>,
}

pub(crate) fn permission_panel_lead(waiting: &[V]) -> String {
    match waiting.len() {
        0 => String::new(),
        1 => "Waiting for your answer to a permission request…".to_owned(),
        n => format!("Waiting for your answers to {n} permission requests…"),
    }
}

pub(crate) fn build_permission_card_spec(
    container_id: &str,
    event: &V,
) -> Option<PermissionCardSpec> {
    let request_id = m::text(event, "request_id");
    if request_id.is_empty() {
        return None;
    }
    let tool = permission_tool(event);
    let is_question = is_question_permission(event);
    let questions = if is_question {
        m::array(&event["questions"])
    } else {
        Vec::new()
    };
    let questions_sig = if is_question {
        serde_json::to_string(&questions).unwrap_or_default()
    } else {
        String::new()
    };
    let session_option = m::yes(&event["session_option"]);
    let actions_key = if is_question {
        format!("question:{request_id}")
    } else {
        format!("{session_option}:{tool}")
    };
    let choices: Vec<(&'static str, &'static str)> = if is_question {
        vec![("allow", "Submit answers"), ("deny", "Decline")]
    } else {
        permission_choices(session_option).to_vec()
    };
    let buttons = choices
        .into_iter()
        .map(|(decision, label)| {
            let aria_label = if is_question {
                if decision == "allow" {
                    format!("Submit answers for {tool}")
                } else {
                    format!("Decline {tool}")
                }
            } else {
                permission_button_aria_label(decision, label, &tool)
            };
            PermissionButtonSpec {
                decision,
                label,
                class_name: if decision == "allow" {
                    "button primary compact"
                } else {
                    "button compact"
                },
                payload: permission_decide_payload(container_id, &request_id, decision),
                aria_label,
                is_question_submit: is_question && decision == "allow",
            }
        })
        .collect();
    Some(PermissionCardSpec {
        request_id,
        event_id: m::string(&event["id"]),
        provider: event["provider"]
            .as_str()
            .filter(|p| !p.is_empty())
            .unwrap_or("")
            .to_owned(),
        tool,
        state: m::text(event, "state"),
        heading: permission_heading(event),
        summary: clean(&m::text(event, "summary")),
        detail: clean(&m::text(event, "detail")),
        raw_rule: clean(&m::text(event, "rule_summary")),
        rule_hint: permission_rule_hint_text(event),
        ask_kind: event["ask_kind"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("")
            .to_owned(),
        diff_preview: event["diff_preview"].as_str().unwrap_or("").to_owned(),
        expires_at_ms: permission_expiry_ms(event, current_now_ms()),
        is_question,
        questions,
        questions_sig,
        actions_key,
        buttons,
    })
}

#[cfg(test)]
pub(crate) fn render_permission_card_html(
    container_id: &str,
    event: &V,
    run_active: bool,
) -> String {
    let mut normalized = event.clone();
    if normalized["state"]
        .as_str()
        .is_none_or(|s| s.trim().is_empty())
    {
        normalized["state"] = json!("pending");
    }
    let Some(spec) = build_permission_card_spec(container_id, &normalized) else {
        return String::new();
    };
    let mut attrs = format!(
        "class=\"conversation-permission\" data-permission-id=\"{}\" role=\"group\" data-tool=\"{}\" data-state=\"{}\" aria-label=\"{}\"",
        files_map::escape_html(&spec.request_id),
        files_map::escape_html(&spec.tool),
        files_map::escape_html(&spec.state),
        files_map::escape_html(&spec.heading),
    );
    if !spec.provider.is_empty() {
        attrs.push_str(&format!(
            " data-provider=\"{}\"",
            files_map::escape_html(&spec.provider)
        ));
    }
    if !spec.raw_rule.is_empty() {
        attrs.push_str(&format!(
            " data-rule-summary=\"{}\"",
            files_map::escape_html(&spec.raw_rule)
        ));
    }
    if !spec.ask_kind.is_empty() {
        attrs.push_str(&format!(
            " data-ask-kind=\"{}\"",
            files_map::escape_html(&spec.ask_kind)
        ));
    }
    if spec.state == "pending" {
        if let Some(exp_ms) = spec.expires_at_ms {
            HAS_TIMED_EXPIRIES.with(|flag| flag.set(true));
            attrs.push_str(&format!(" data-expires-ms=\"{exp_ms:.0}\""));
        }
    }
    let mut body = format!(
        "<div class=\"permission-card-head\"><p class=\"permission-heading\">{}</p>",
        files_map::escape_html(&spec.heading)
    );
    if spec.state == "pending" {
        if let Some(exp_ms) = spec.expires_at_ms {
            let (tone, text) = format_countdown_badge(exp_ms - current_now_ms());
            body.push_str(&format!(
                "<span class=\"permission-expiry-badge\" data-tone=\"{tone}\">{}</span>",
                files_map::escape_html(&text)
            ));
        }
    }
    body.push_str("</div>");
    if !spec.summary.is_empty() {
        body.push_str(&format!(
            "<code class=\"permission-summary\">{}</code>",
            files_map::escape_html(&spec.summary)
        ));
    }
    if !spec.detail.is_empty() {
        body.push_str(&format!(
            "<p class=\"permission-detail\" data-role=\"detail\">{}</p>",
            files_map::escape_html(&spec.detail)
        ));
    }
    if !spec.rule_hint.is_empty() {
        body.push_str(&format!(
            "<p class=\"permission-rule-hint\">{}</p>",
            files_map::escape_html(&spec.rule_hint)
        ));
    }
    if !spec.diff_preview.trim().is_empty() {
        body.push_str(&format!(
            "<details class=\"permission-diff-wrap\" data-diff-raw=\"{}\"><summary>View proposed changes</summary><pre class=\"permission-diff\">{}</pre></details>",
            files_map::escape_html(&spec.diff_preview),
            render_diff_preview_lines_html(&spec.diff_preview),
        ));
    }
    if spec.state == "pending" {
        if !run_active {
            body.push_str(
                "<p class=\"permission-detail\" data-role=\"ended\">The run that asked has ended.</p>",
            );
        } else {
            if spec.is_question {
                body.push_str(&format!(
                    "<div class=\"permission-question-card\" data-questions-sig=\"{}\">{}</div>",
                    files_map::escape_html(&spec.questions_sig),
                    render_question_card_html_in(container_id, &spec.request_id, &spec.questions),
                ));
            }
            body.push_str(&format!(
                "<div class=\"permission-actions\" data-actions-key=\"{}\">",
                files_map::escape_html(&spec.actions_key)
            ));
            for button in &spec.buttons {
                let extra = if button.is_question_submit {
                    " data-question-submit=\"true\""
                } else {
                    ""
                };
                body.push_str(&format!(
                    "<button type=\"button\" class=\"{}\" data-action=\"permission-decide\" data-payload=\"{}\" data-decision=\"{}\" data-label=\"{}\"{extra} aria-label=\"{}\">{}</button>",
                    button.class_name,
                    files_map::escape_html(&button.payload.to_string()),
                    button.decision,
                    files_map::escape_html(button.label),
                    files_map::escape_html(&button.aria_label),
                    files_map::escape_html(button.label),
                ));
            }
            body.push_str("</div><p class=\"permission-error error-text\" role=\"alert\"></p>");
        }
    }
    format!("<div {attrs}>{body}</div>")
}

pub(crate) fn waiting_permission_reconcile_plan(
    existing_ids: &[String],
    waiting: &[V],
) -> (Vec<String>, Vec<(String, V)>) {
    let mut seen = std::collections::BTreeSet::new();
    let mut live = Vec::new();
    for ask in waiting {
        let id = m::text(ask, "request_id");
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        let mut pending_ask = ask.clone();
        if pending_ask["state"]
            .as_str()
            .is_none_or(|s| s.trim().is_empty())
        {
            pending_ask["state"] = json!("pending");
        }
        live.push((id, pending_ask));
    }
    let remove = existing_ids
        .iter()
        .filter(|id| !seen.contains(*id))
        .cloned()
        .collect();
    (remove, live)
}

fn permission_container_blocks(container: &web_sys::Element) -> Vec<web_sys::Element> {
    dom::children(container)
        .into_iter()
        .filter(|el| el.has_attribute("data-permission-id"))
        .collect()
}

pub(super) fn find_permission_block(
    container: &web_sys::Element,
    request_id: &str,
) -> Option<web_sys::Element> {
    permission_container_blocks(container)
        .into_iter()
        .find(|block| block.get_attribute("data-permission-id").as_deref() == Some(request_id))
}

fn sync_card_expiry_slot(
    block: &web_sys::Element,
    expires_at_ms: Option<f64>,
    is_pending: bool,
    after_heading: Option<&web_sys::Element>,
) {
    let existing = permission_child(block, "permission-expiry-badge", None);
    if !is_pending {
        let _ = block.remove_attribute("data-expires-ms");
        if let Some(el) = existing {
            el.remove();
        }
        return;
    }
    let Some(exp_ms) = expires_at_ms else {
        let _ = block.remove_attribute("data-expires-ms");
        if let Some(el) = existing {
            el.remove();
        }
        return;
    };
    HAS_TIMED_EXPIRIES.with(|flag| flag.set(true));
    let _ = block.set_attribute("data-expires-ms", &format!("{exp_ms:.0}"));
    let (tone, text) = format_countdown_badge(exp_ms - current_now_ms());
    let badge = match existing {
        Some(el) => el,
        None => {
            let el = dom::el("span", "permission-expiry-badge", &text);
            if let Some(h) = after_heading {
                let _ = block.insert_before(&el, h.next_sibling().as_ref());
            } else {
                let _ = block.insert_before(&el, block.first_child().as_ref());
            }
            el
        }
    };
    if badge.get_attribute("data-tone").as_deref() != Some(tone) {
        let _ = badge.set_attribute("data-tone", tone);
    }
    if badge.text_content().as_deref() != Some(text.as_str()) {
        badge.set_text_content(Some(&text));
    }
}

pub(super) fn upsert_permission_card_in(
    container: &web_sys::Element,
    container_id: &str,
    event: &V,
    run_active: bool,
    cache_questions: bool,
) -> Option<(web_sys::Element, bool)> {
    let spec = build_permission_card_spec(container_id, event)?;
    let request = &spec.request_id;
    let found = find_permission_block(container, request);
    let is_new = found.is_none();
    let block = match found {
        Some(block) => block,
        None => {
            let block = dom::el("div", "conversation-permission", "");
            let _ = block.set_attribute("data-permission-id", request);
            let _ = block.set_attribute("role", "group");
            dom::add(container, &block);
            block
        }
    };
    if !spec.event_id.is_empty()
        && block.get_attribute("data-event-id").as_deref() != Some(spec.event_id.as_str())
    {
        let _ = block.set_attribute("data-event-id", &spec.event_id);
    }
    if !spec.provider.is_empty()
        && block.get_attribute("data-provider").as_deref() != Some(spec.provider.as_str())
    {
        let _ = block.set_attribute("data-provider", &spec.provider);
    }
    if block.get_attribute("data-tool").as_deref() != Some(spec.tool.as_str()) {
        let _ = block.set_attribute("data-tool", &spec.tool);
    }
    if block.get_attribute("data-state").as_deref() != Some(spec.state.as_str()) {
        let _ = block.set_attribute("data-state", &spec.state);
    }
    if block.get_attribute("aria-label").as_deref() != Some(spec.heading.as_str()) {
        let _ = block.set_attribute("aria-label", &spec.heading);
    }
    match permission_child(&block, "permission-heading", None) {
        Some(existing) if existing.text_content().as_deref() == Some(spec.heading.as_str()) => {}
        Some(existing) => {
            let fresh = dom::el("p", "permission-heading", &spec.heading);
            let _ = block.replace_child(&fresh, &existing);
        }
        None => {
            let first = dom::children(&block).into_iter().next();
            let fresh = dom::el("p", "permission-heading", &spec.heading);
            let _ = block.insert_before(&fresh, first.as_ref().map(|e| e.as_ref()));
        }
    }
    let heading_el = permission_child(&block, "permission-heading", None);
    sync_card_expiry_slot(
        &block,
        spec.expires_at_ms,
        spec.state == "pending" && run_active,
        heading_el.as_ref(),
    );
    let existing_actions = permission_child(&block, "permission-actions", None);
    let existing_error = permission_child(&block, "permission-error", None);
    let existing_ended = permission_child(&block, "permission-detail", Some("ended"));
    let existing_question = permission_child(&block, "permission-question-card", None);
    let existing_answers = permission_child(&block, "permission-answers-summary", None);
    let existing_diff = permission_child(&block, "permission-diff-wrap", None);
    let existing_rule = permission_child(&block, "permission-rule-hint", None);
    let existing_detail = permission_child(&block, "permission-detail", Some("detail"));
    let before_summary = existing_detail
        .as_ref()
        .or(existing_rule.as_ref())
        .or(existing_diff.as_ref())
        .or(existing_question.as_ref())
        .or(existing_answers.as_ref())
        .or(existing_ended.as_ref())
        .or(existing_actions.as_ref())
        .or(existing_error.as_ref());
    sync_permission_text_slot(
        &block,
        "code",
        "permission-summary",
        None,
        &spec.summary,
        before_summary,
    );
    let before_detail = existing_rule
        .as_ref()
        .or(existing_diff.as_ref())
        .or(existing_question.as_ref())
        .or(existing_answers.as_ref())
        .or(existing_ended.as_ref())
        .or(existing_actions.as_ref())
        .or(existing_error.as_ref());
    sync_permission_text_slot(
        &block,
        "p",
        "permission-detail",
        Some("detail"),
        &spec.detail,
        before_detail,
    );
    if spec.raw_rule.is_empty() {
        let _ = block.remove_attribute("data-rule-summary");
    } else {
        let _ = block.set_attribute("data-rule-summary", &spec.raw_rule);
    }
    if !spec.ask_kind.is_empty() {
        let _ = block.set_attribute("data-ask-kind", &spec.ask_kind);
    }
    let before_rule = existing_diff
        .as_ref()
        .or(existing_question.as_ref())
        .or(existing_answers.as_ref())
        .or(existing_ended.as_ref())
        .or(existing_actions.as_ref())
        .or(existing_error.as_ref());
    sync_permission_text_slot(
        &block,
        "p",
        "permission-rule-hint",
        None,
        &spec.rule_hint,
        before_rule,
    );
    let before_diff = existing_question
        .as_ref()
        .or(existing_answers.as_ref())
        .or(existing_ended.as_ref())
        .or(existing_actions.as_ref())
        .or(existing_error.as_ref());
    if spec.diff_preview.trim().is_empty() {
        if let Some(el) = existing_diff {
            el.remove();
        }
    } else {
        let keep_diff = existing_diff.as_ref().is_some_and(|el| {
            el.get_attribute("data-diff-raw").as_deref() == Some(spec.diff_preview.as_str())
        });
        if !keep_diff {
            let wrap = dom::el("details", "permission-diff-wrap", "");
            let _ = wrap.set_attribute("data-diff-raw", &spec.diff_preview);
            if existing_diff
                .as_ref()
                .is_some_and(|old| old.has_attribute("open"))
            {
                let _ = wrap.set_attribute("open", "");
            }
            let sum = dom::el("summary", "", "View proposed changes");
            let pre = dom::el("pre", "permission-diff", "");
            pre.set_inner_html(&render_diff_preview_lines_html(&spec.diff_preview));
            dom::add(&wrap, &sum);
            dom::add(&wrap, &pre);
            if let Some(old_diff) = existing_diff {
                let _ = block.replace_child(&wrap, &old_diff);
            } else {
                let _ = block.insert_before(&wrap, before_diff.map(|e| e.as_ref()));
            }
        }
    }
    if spec.state != "pending" {
        if let Some(el) = existing_question {
            el.remove();
        }
        if let Some(el) = existing_ended {
            el.remove();
        }
        if let Some(el) = existing_actions {
            el.remove();
        }
        if let Some(el) = existing_error {
            el.remove();
        }
        let resolved_answers = if event["answers"].is_object() {
            event["answers"].clone()
        } else if event["resolved_answers"].is_object() {
            event["resolved_answers"].clone()
        } else {
            m::data("permission-submitted-answers")[request].clone()
        };
        let event_questions = m::array(&event["questions"]);
        let questions = if !event_questions.is_empty() {
            event_questions
        } else {
            m::array(&m::data("permission-questions-cache")[request])
        };
        let summary_html = render_answers_summary_html(&questions, &resolved_answers);
        if summary_html.is_empty() {
            if let Some(el) = existing_answers {
                el.remove();
            }
        } else {
            let sig = json!([questions, resolved_answers]).to_string();
            let keep_answers = existing_answers
                .as_ref()
                .is_some_and(|el| el.get_attribute("data-answers-sig").as_deref() == Some(&sig));
            if !keep_answers {
                let wrap = dom::el("div", "permission-answers-summary", "");
                let _ = wrap.set_attribute("data-answers-sig", &sig);
                wrap.set_inner_html(&summary_html);
                if let Some(old_answers) = existing_answers {
                    let _ = block.replace_child(&wrap, &old_answers);
                } else {
                    dom::add(&block, &wrap);
                }
            }
        }
        return Some((block, is_new));
    }
    if let Some(el) = existing_answers {
        el.remove();
    }
    if !run_active {
        if let Some(el) = existing_question {
            el.remove();
        }
        sync_permission_text_slot(
            &block,
            "p",
            "permission-detail",
            Some("ended"),
            "The run that asked has ended.",
            existing_actions.as_ref().or(existing_error.as_ref()),
        );
        if let Some(el) = existing_actions {
            el.remove();
        }
        if let Some(el) = existing_error {
            el.remove();
        }
        return Some((block, is_new));
    }
    if let Some(el) = existing_ended {
        el.remove();
    }
    if spec.is_question {
        if cache_questions {
            let mut cache = m::data("permission-questions-cache")
                .as_object()
                .cloned()
                .unwrap_or_default();
            if cache.len() >= 200 {
                cache.clear();
            }
            cache.insert(request.clone(), V::Array(spec.questions.clone()));
            m::set("permission-questions-cache", V::Object(cache));
        }
        let keep_question = existing_question.as_ref().is_some_and(|el| {
            el.get_attribute("data-questions-sig").as_deref() == Some(spec.questions_sig.as_str())
        });
        if !keep_question {
            let card = dom::el("div", "permission-question-card", "");
            let _ = card.set_attribute("data-questions-sig", &spec.questions_sig);
            card.set_inner_html(&render_question_card_html_in(
                container_id,
                request,
                &spec.questions,
            ));
            if let Some(old_card) = existing_question {
                let _ = block.replace_child(&card, &old_card);
            } else {
                let _ = block.insert_before(
                    &card,
                    existing_actions
                        .as_ref()
                        .or(existing_error.as_ref())
                        .map(|e| e.as_ref()),
                );
            }
        }
    } else if let Some(el) = existing_question {
        el.remove();
    }
    let keep_actions = existing_actions.as_ref().is_some_and(|el| {
        el.get_attribute("data-actions-key").as_deref() == Some(spec.actions_key.as_str())
    });
    if !keep_actions {
        let actions = dom::el("div", "permission-actions", "");
        let _ = actions.set_attribute("data-actions-key", &spec.actions_key);
        for spec_button in &spec.buttons {
            let button = dom::button(
                spec_button.label,
                "permission-decide",
                spec_button.payload.clone(),
            );
            let _ = button.set_attribute("class", spec_button.class_name);
            let _ = button.set_attribute("data-decision", spec_button.decision);
            let _ = button.set_attribute("data-label", spec_button.label);
            if spec_button.is_question_submit {
                let _ = button.set_attribute("data-question-submit", "true");
            }
            let _ = button.set_attribute("aria-label", &spec_button.aria_label);
            dom::add(&actions, &button);
        }
        if let Some(old_actions) = existing_actions {
            let _ = block.replace_child(&actions, &old_actions);
        } else {
            let _ = block.insert_before(&actions, existing_error.as_ref().map(|e| e.as_ref()));
        }
    }
    match existing_error {
        Some(err) => {
            if err.get_attribute("role").as_deref() != Some("alert") {
                let _ = err.set_attribute("role", "alert");
            }
        }
        None => {
            let err = dom::el("p", "permission-error error-text", "");
            let _ = err.set_attribute("role", "alert");
            dom::add(&block, &err);
        }
    }
    Some((block, is_new))
}

/// A request the agent's host asked permission for, and what became of it. Every
/// event of one request patches the existing block in place without dropping focus.
pub(super) fn permission(event: &V) {
    let request = m::text(event, "request_id");
    if request.is_empty() {
        return;
    }
    let state = m::text(event, "state");
    let mut pending = m::data("permission-pending")
        .as_object()
        .cloned()
        .unwrap_or_default();
    if state == "pending" {
        pending.insert(request.clone(), json!(true));
    } else {
        pending.remove(&request);
    }
    m::set("permission-pending", V::Object(pending));
    let Some(container) = dom::get("events") else {
        return;
    };
    let run_active = m::active(&m::read(|s| s.selected.clone()));
    if let Some((_, is_new)) =
        upsert_permission_card_in(&container, "events", event, run_active, true)
    {
        if is_new {
            notify_permission(event);
        }
    }
    schedule_permission_countdowns();
    sync_approvals_badge();
}

/// Renders waiting permission ask events into a dedicated container without
/// disturbing unsent question inputs or focus on polls.
pub(crate) fn sync_waiting_permissions(container_id: &str, waiting: &[V], run_active: bool) {
    let Some(container) = dom::get(container_id) else {
        return;
    };
    let existing_blocks = permission_container_blocks(&container);
    let existing_ids: Vec<String> = existing_blocks
        .iter()
        .filter_map(|block| block.get_attribute("data-permission-id"))
        .collect();
    let (to_remove, live) = waiting_permission_reconcile_plan(&existing_ids, waiting);
    let remove_set: std::collections::BTreeSet<&str> =
        to_remove.iter().map(String::as_str).collect();
    for block in existing_blocks {
        if block
            .get_attribute("data-permission-id")
            .is_some_and(|id| remove_set.contains(id.as_str()))
        {
            block.remove();
        }
    }
    for (idx, (_, pending_ask)) in live.iter().enumerate() {
        if let Some((block, _)) =
            upsert_permission_card_in(&container, container_id, pending_ask, run_active, false)
        {
            let current = container.children().item(idx as u32);
            if current.as_ref() != Some(&block) {
                let _ = container.insert_before(&block, current.as_ref().map(|e| e.as_ref()));
            }
        }
    }
    schedule_permission_countdowns();
    sync_approvals_badge();
}

pub(super) fn expired_permission_event(
    request_id: &str,
    provider: &str,
    tool: &str,
    summary: &str,
    detail: &str,
) -> V {
    json!({
        "request_id": request_id,
        "state": "expired",
        "provider": provider,
        "tool": tool,
        "summary": summary,
        "detail": detail,
    })
}

/// Applies the authoritative waiting-permission count for one session to both `sessions` and `approvals.waiting_counts`.
pub(crate) fn apply_session_waiting_count(
    sessions: &mut [V],
    waiting_counts: &mut std::collections::BTreeMap<String, usize>,
    sid: &str,
    count: usize,
) {
    if sid.is_empty() {
        return;
    }
    if count == 0 {
        waiting_counts.remove(sid);
    } else {
        waiting_counts.insert(sid.to_owned(), count);
    }
    for sess in sessions.iter_mut() {
        if sess["id"].as_str() == Some(sid) && sess.is_object() {
            sess["waiting_permissions"] = json!(count);
        }
    }
}

/// Reconciles `approvals.waiting_counts` directly from a fresh `/api/sessions` snapshot.
pub(crate) fn sync_waiting_counts_from_sessions(
    sessions: &[V],
    waiting_counts: &mut std::collections::BTreeMap<String, usize>,
) {
    let mut next = std::collections::BTreeMap::new();
    for sess in sessions {
        let sid = m::text(sess, "id");
        if sid.is_empty() {
            continue;
        }
        let count = sess["waiting_permissions"].as_u64().unwrap_or(0) as usize;
        if count > 0 {
            next.insert(sid, count);
        }
    }
    *waiting_counts = next;
}

/// Reconciles live waiting permission requests returned by `sessions.get`.
pub(crate) fn reconcile_waiting_permissions(waiting: &[V]) {
    let changed_waiting_count = m::write(|s| {
        let sid = s.sid.clone();
        if sid.is_empty() {
            return false;
        }
        let prev = s
            .approvals
            .waiting_counts
            .get(&sid)
            .copied()
            .or_else(|| {
                s.sessions
                    .iter()
                    .find(|sess| sess["id"].as_str() == Some(sid.as_str()))
                    .and_then(|sess| sess["waiting_permissions"].as_u64())
                    .map(|n| n as usize)
            })
            .unwrap_or(0);
        let next = waiting.len();
        apply_session_waiting_count(&mut s.sessions, &mut s.approvals.waiting_counts, &sid, next);
        if s.selected["id"].as_str() == Some(sid.as_str()) && s.selected.is_object() {
            s.selected["waiting_permissions"] = json!(next);
        }
        prev != next
    });
    let mut live_ids = std::collections::BTreeSet::new();
    for ask in waiting {
        let id = m::text(ask, "request_id");
        if id.is_empty() {
            continue;
        }
        live_ids.insert(id.clone());
        let mut pending_ask = ask.clone();
        if pending_ask["state"].is_null() {
            pending_ask["state"] = json!("pending");
        }
        permission(&pending_ask);
    }
    let stale_ids: Vec<String> = m::data("permission-pending")
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, _)| id.clone())
        .filter(|id| !live_ids.contains(id))
        .collect();
    if !stale_ids.is_empty() {
        for id in stale_ids {
            let card = dom::get("events").and_then(|c| find_permission_block(&c, &id));
            let provider = card
                .as_ref()
                .and_then(|block| block.get_attribute("data-provider"))
                .unwrap_or_default();
            let tool = card
                .as_ref()
                .and_then(|block| block.get_attribute("data-tool"))
                .unwrap_or_default();
            let summary = card
                .as_ref()
                .and_then(|block| permission_child(block, "permission-summary", None))
                .and_then(|el| el.text_content())
                .unwrap_or_default();
            let detail = card
                .as_ref()
                .and_then(|block| permission_child(block, "permission-detail", Some("detail")))
                .and_then(|el| el.text_content())
                .unwrap_or_default();
            let mut event = expired_permission_event(&id, &provider, &tool, &summary, &detail);
            if let Some(ask_kind) = card
                .as_ref()
                .and_then(|block| block.get_attribute("data-ask-kind"))
                .filter(|s| !s.is_empty())
            {
                event["ask_kind"] = json!(ask_kind);
            }
            if let Some(rule_summary) = card
                .as_ref()
                .and_then(|block| block.get_attribute("data-rule-summary"))
                .filter(|s| !s.is_empty())
            {
                event["rule_summary"] = json!(rule_summary);
            }
            if let Some(diff_raw) = card
                .as_ref()
                .and_then(|block| permission_child(block, "permission-diff-wrap", None))
                .and_then(|el| el.get_attribute("data-diff-raw"))
                .filter(|s| !s.is_empty())
            {
                event["diff_preview"] = json!(diff_raw);
            }
            permission(&event);
        }
    }
    sync_approvals_badge();
    if changed_waiting_count {
        crate::session::history();
    }
}

fn collect_question_answers(question_card: &web_sys::Element) -> Result<V, &'static str> {
    let mut answers = serde_json::Map::new();
    let Ok(fieldsets) = question_card.query_selector_all("fieldset.permission-question") else {
        return Ok(V::Object(answers));
    };
    for i in 0..fieldsets.length() {
        let Some(fieldset) = fieldsets
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        let key = fieldset
            .get_attribute("data-question-key")
            .unwrap_or_else(|| format!("q{i}"));
        let multi = fieldset.get_attribute("data-multi-select").as_deref() == Some("true");
        let other_text = fieldset
            .query_selector("input.permission-question-other-input")
            .ok()
            .flatten()
            .map(|el| dom::value_of(&el).trim().to_owned())
            .unwrap_or_default();
        let choices = fieldset
            .query_selector_all("input.permission-question-choice")
            .ok()
            .map(|list| {
                (0..list.length())
                    .filter_map(|j| list.item(j)?.dyn_into::<web_sys::Element>().ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if multi {
            let mut selected = Vec::new();
            for choice in &choices {
                if dom::checked_of(choice) {
                    let val = dom::value_of(choice);
                    if !val.is_empty() && val != "__other__" {
                        selected.push(val);
                    }
                }
            }
            if !other_text.is_empty() && !selected.contains(&other_text) {
                selected.push(other_text);
            }
            if selected.is_empty() {
                return Err(
                    "Select at least one option or enter a custom answer, or choose Decline.",
                );
            }
            answers.insert(key, V::Array(selected.into_iter().map(V::String).collect()));
        } else {
            let checked_choice = choices.iter().find(|c| dom::checked_of(c));
            if let Some(choice) = checked_choice {
                let val = dom::value_of(choice);
                if val == "__other__" {
                    if other_text.is_empty() {
                        return Err("Enter a custom answer for Other.");
                    }
                    answers.insert(key, V::String(other_text));
                } else {
                    answers.insert(key, V::String(val));
                }
            } else if !other_text.is_empty() {
                answers.insert(key, V::String(other_text));
            } else {
                return Err("Answer each question or choose Decline.");
            }
        }
    }
    Ok(V::Object(answers))
}

/// Sends the person's answer to a waiting request inside `container_id` for `sid`.
pub(crate) async fn decide_permission_for(container_id: &str, sid: &str, payload: &V) -> bool {
    let request = m::text(payload, "request_id");
    let decision = m::text(payload, "decision");
    if sid.is_empty() || request.is_empty() || decision.is_empty() || m::read(|s| s.auth_failed) {
        return false;
    }
    let block = dom::get(container_id).and_then(|c| find_permission_block(&c, &request));
    let buttons: Vec<web_sys::Element> = block
        .iter()
        .flat_map(dom::children)
        .filter(|child| child.class_list().contains("permission-actions"))
        .flat_map(|actions| dom::children(&actions))
        .collect();
    if buttons.iter().any(|button| {
        button.has_attribute("disabled")
            || button.get_attribute("aria-busy").as_deref() == Some("true")
    }) {
        return false;
    }
    let error_el = block
        .iter()
        .flat_map(dom::children)
        .find(|child| child.class_list().contains("permission-error"));
    if let Some(note) = &error_el {
        if note.get_attribute("role").as_deref() != Some("alert") {
            let _ = note.set_attribute("role", "alert");
        }
        note.set_text_content(Some(""));
    }
    let question_card = block
        .as_ref()
        .and_then(|b| permission_child(b, "permission-question-card", None));
    let submitted_answers = if decision == "allow" {
        if let Some(card) = &question_card {
            match collect_question_answers(card) {
                Ok(ans) => Some(ans),
                Err(msg) => {
                    if let Some(note) = &error_el {
                        note.set_text_content(Some(msg));
                    }
                    return false;
                }
            }
        } else {
            None
        }
    } else {
        None
    };
    let saved_labels: Vec<(web_sys::Element, String)> = buttons
        .iter()
        .map(|button| {
            let original = button
                .get_attribute("data-label")
                .unwrap_or_else(|| button.text_content().unwrap_or_default());
            (button.clone(), original)
        })
        .collect();
    for button in &buttons {
        let _ = button.set_attribute("disabled", "");
        if button.get_attribute("data-decision").as_deref() == Some(decision.as_str()) {
            let _ = button.set_attribute("aria-busy", "true");
            button.set_text_content(Some("Sending…"));
        }
    }
    let question_inputs: Vec<web_sys::Element> = question_card
        .as_ref()
        .and_then(|card| card.query_selector_all("input,select,textarea").ok())
        .map(|list| {
            (0..list.length())
                .filter_map(|i| list.item(i)?.dyn_into::<web_sys::Element>().ok())
                .collect()
        })
        .unwrap_or_default();
    for input in &question_inputs {
        let _ = input.set_attribute("disabled", "");
    }
    let body = build_permission_decision_body(&decision, submitted_answers.clone());
    let answered = crate::api::post(
        &format!(
            "/api/sessions/{}/permissions/{}",
            dom::enc(sid),
            dom::enc(&request)
        ),
        body,
    )
    .await;
    match answered {
        Ok(_) => {
            if let Some(ans) = submitted_answers {
                let mut cache = m::data("permission-submitted-answers")
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                if cache.len() >= 200 {
                    cache.clear();
                }
                cache.insert(request.clone(), ans);
                m::set("permission-submitted-answers", V::Object(cache));
            }
            true
        }
        Err(error) => {
            for (button, label) in &saved_labels {
                let _ = button.remove_attribute("disabled");
                let _ = button.remove_attribute("aria-busy");
                button.set_text_content(Some(label));
            }
            for input in &question_inputs {
                let _ = input.remove_attribute("disabled");
            }
            if let Some(note) = &error_el {
                note.set_text_content(Some(&error.message));
            }
            false
        }
    }
}

/// Sends the person's answer to a waiting request in the active session feed.
pub(super) async fn decide_permission(payload: &V) {
    let container_id = permission_event_container(payload);
    let sid = payload["session_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| m::read(|s| s.sid.clone()));
    let origin_kind = m::text(payload, "origin_kind");
    let origin_run_id = m::text(payload, "origin_run_id");
    if decide_permission_for(container_id, &sid, payload).await {
        invalidate_approvals_cache_for(&sid);
        let current_sid = m::read(|s| s.sid.clone());
        let epoch = m::read(|s| s.epoch);
        if sid == current_sid {
            session::poll(epoch).await;
        }
        if origin_kind == "system" && !origin_run_id.is_empty() {
            crate::systems::read_system_run(&origin_run_id).await;
        } else if origin_kind == "creator" && !origin_run_id.is_empty() {
            crate::management::creator_read(&origin_run_id).await;
        }
        if m::read(|s| s.approvals.drawer_open) {
            refresh_approvals_queue().await;
            if sid == current_sid {
                load_permission_rules(&sid).await;
            }
        }
        sync_approvals_badge();
    }
}

pub(crate) fn permission_target_container(
    payload: &V,
    view: &str,
    active_container: Option<&str>,
) -> &'static str {
    match payload["container"].as_str() {
        Some("system-permissions-list") => return "system-permissions-list",
        Some("creator-permissions-list") => return "creator-permissions-list",
        Some("approvals-drawer-list") => return "approvals-drawer-list",
        Some("events") => return "events",
        _ => {}
    }
    match active_container {
        Some("system-permissions-list") => return "system-permissions-list",
        Some("creator-permissions-list") => return "creator-permissions-list",
        Some("approvals-drawer-list") => return "approvals-drawer-list",
        Some("events") => return "events",
        _ => {}
    }
    match view {
        "system-changes" | "systems" => "system-permissions-list",
        "creator" => "creator-permissions-list",
        _ => "events",
    }
}

fn active_permission_container() -> Option<&'static str> {
    let active = dom::document().active_element()?;
    let container = active
        .closest(
            "#system-permissions-list, #creator-permissions-list, #approvals-drawer-list, #events",
        )
        .ok()
        .flatten()?;
    match container.id().as_str() {
        "system-permissions-list" => Some("system-permissions-list"),
        "creator-permissions-list" => Some("creator-permissions-list"),
        "approvals-drawer-list" => Some("approvals-drawer-list"),
        "events" => Some("events"),
        _ => None,
    }
}

pub(crate) fn permission_event_container(payload: &V) -> &'static str {
    let view = m::read(|s| s.view.clone());
    permission_target_container(payload, &view, active_permission_container())
}

thread_local! {
    static HAS_TIMED_EXPIRIES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static APPROVALS_SESSION_CACHE: std::cell::RefCell<BTreeMap<String, (usize, Vec<V>)>> =
        const { std::cell::RefCell::new(BTreeMap::new()) };
}

pub(crate) fn invalidate_approvals_cache_for(sid: &str) {
    if sid.is_empty() {
        return;
    }
    APPROVALS_SESSION_CACHE.with(|cache| {
        cache.borrow_mut().remove(sid);
    });
}

pub(crate) fn approvals_sessions_to_fetch(
    waiting_sids: &[(String, String, usize)],
    cache: &BTreeMap<String, (usize, Vec<V>)>,
    force_all: bool,
) -> Vec<(String, String)> {
    waiting_sids
        .iter()
        .filter(|(sid, _, count)| {
            force_all
                || cache
                    .get(sid)
                    .is_none_or(|(cached_count, _)| *cached_count != *count)
        })
        .map(|(sid, label, _)| (sid.clone(), label.clone()))
        .collect()
}

/// Schedules permission expiry countdowns only when at least one card carries `data-expires-ms`.
pub(crate) fn schedule_permission_countdowns() {
    if !HAS_TIMED_EXPIRIES.with(|flag| flag.get()) {
        dom::cancel_timer("permission-countdown");
        return;
    }
    let timed = dom::all(".conversation-permission[data-state=\"pending\"][data-expires-ms]");
    if timed.is_empty() {
        HAS_TIMED_EXPIRIES.with(|flag| flag.set(false));
        dom::cancel_timer("permission-countdown");
        return;
    }
    sync_permission_countdown_badges_for(&timed);
    dom::later("permission-countdown", 1000, || {
        schedule_permission_countdowns();
    });
}

fn sync_permission_countdown_badges_for(cards: &[web_sys::Element]) {
    let now = current_now_ms();
    for card in cards {
        let Some(exp_ms) = card
            .get_attribute("data-expires-ms")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
        else {
            continue;
        };
        let (tone, text) = format_countdown_badge(exp_ms - now);
        if let Ok(Some(badge)) = card.query_selector(".permission-expiry-badge") {
            if badge.get_attribute("data-tone").as_deref() != Some(tone) {
                let _ = badge.set_attribute("data-tone", tone);
            }
            if badge.text_content().as_deref() != Some(text.as_str()) {
                badge.set_text_content(Some(&text));
            }
        }
    }
}

/// Debounces live SSE-driven queue refreshes across rapid `waiting_counts` updates.
pub(crate) fn schedule_approvals_queue_refresh() {
    dom::later("approvals-queue-refresh", 150, || {
        wasm_bindgen_futures::spawn_local(async {
            if m::read(|s| s.approvals.drawer_open) {
                refresh_approvals_queue_inner(false).await;
            }
        });
    });
}

/// Computes total pending approval count across sessions and extra runs.
#[cfg(test)]
pub(crate) fn compute_total_pending_approvals_count(
    sessions: &[V],
    waiting_counts: &std::collections::BTreeMap<String, usize>,
    active_sid: &str,
    current_pending: usize,
    extra_run_pending: usize,
) -> usize {
    let extra =
        (extra_run_pending > 0).then(|| json!({ "waiting_permissions": extra_run_pending }));
    let runs: Vec<&V> = extra.iter().collect();
    compute_total_pending_approvals_with_runs(
        sessions,
        waiting_counts,
        active_sid,
        current_pending,
        &runs,
    )
}

fn background_run_session_id(run_val: &V) -> String {
    run_val["session_id"]
        .as_str()
        .or_else(|| run_val["session"]["id"].as_str())
        .or_else(|| run_val["run"]["session_id"].as_str())
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn background_run_pending_count(run_val: &V) -> usize {
    if let Some(arr) = run_val["waiting_permissions"].as_array() {
        arr.len()
    } else if let Some(n) = run_val["waiting_permissions"].as_u64() {
        n as usize
    } else {
        0
    }
}

pub(crate) fn compute_total_pending_approvals_with_runs(
    sessions: &[V],
    waiting_counts: &std::collections::BTreeMap<String, usize>,
    active_sid: &str,
    current_pending: usize,
    background_runs: &[&V],
) -> usize {
    let mut extra_by_sid = std::collections::BTreeMap::<String, usize>::new();
    let mut unnamed_extra = 0usize;
    for run_val in background_runs {
        if run_val.is_null() {
            continue;
        }
        let count = background_run_pending_count(run_val);
        if count == 0 {
            continue;
        }
        let sid = background_run_session_id(run_val);
        if sid.is_empty() {
            unnamed_extra += count;
        } else {
            extra_by_sid
                .entry(sid)
                .and_modify(|prev| *prev = (*prev).max(count))
                .or_insert(count);
        }
    }

    let mut total = 0usize;
    let mut counted_current = false;
    for session in sessions {
        let sid = m::text(session, "id");
        let mut count = session["waiting_permissions"].as_u64().unwrap_or(0) as usize;
        if let Some(&sse_count) = waiting_counts.get(&sid) {
            count = sse_count;
        }
        if !active_sid.is_empty() && sid == active_sid {
            count = count.max(current_pending);
            counted_current = true;
        }
        if let Some(extra) = extra_by_sid.remove(&sid) {
            count = count.max(extra);
        }
        total += count;
    }
    if !counted_current {
        let mut current = current_pending;
        if !active_sid.is_empty() {
            if let Some(extra) = extra_by_sid.remove(active_sid) {
                current = current.max(extra);
            }
        }
        total += current;
    }
    for (sid, &count) in waiting_counts {
        if !sessions.iter().any(|sess| m::text(sess, "id") == *sid)
            && (counted_current || *sid != active_sid)
        {
            let extra = extra_by_sid.remove(sid).unwrap_or(0);
            total += count.max(extra);
        }
    }
    total + extra_by_sid.into_values().sum::<usize>() + unnamed_extra
}

pub(crate) fn total_pending_approvals_count() -> usize {
    m::read(|s| {
        let current_pending = s
            .data
            .get("permission-pending")
            .and_then(|v| v.as_object())
            .map(|o| o.len())
            .unwrap_or(0);
        let bg_keys = ["system-run", "creator-detail"];
        let runs: Vec<&V> = bg_keys.iter().filter_map(|k| s.data.get(*k)).collect();
        compute_total_pending_approvals_with_runs(
            &s.sessions,
            &s.approvals.waiting_counts,
            &s.sid,
            current_pending,
            &runs,
        )
    })
}

pub(crate) fn sync_approvals_badge() {
    let count = total_pending_approvals_count();
    dom::attr("approvals-queue-toggle", "data-count", &count.to_string());
    dom::text("approvals-queue-count", &count.to_string());
    dom::hidden("approvals-queue-count", count == 0);
}

pub(crate) fn render_permission_rules_panel() {
    let (sid, rules, loading, error) = m::read(|s| {
        (
            s.sid.clone(),
            s.approvals.rules.clone(),
            s.approvals.rules_loading,
            s.approvals.rules_error.clone(),
        )
    });
    dom::error("permission-rules-error", &error);
    dom::hidden(
        "permission-rules-revoke-all",
        rules.is_empty() || sid.is_empty(),
    );
    let Some(list) = dom::get("permission-rules-list") else {
        return;
    };
    list.set_text_content(None);
    if sid.is_empty() {
        dom::add(
            &list,
            &dom::el(
                "p",
                "muted",
                "Open a session to inspect remembered approval rules.",
            ),
        );
        return;
    }
    if loading {
        dom::add(
            &list,
            &dom::el("p", "muted", "Loading remembered session rules…"),
        );
        return;
    }
    if rules.is_empty() {
        dom::add(
            &list,
            &dom::el("p", "muted", "No session approval rules active."),
        );
        return;
    }
    for rule in rules {
        let row = dom::el("div", "permission-rule-row", "");
        let copy = dom::el("div", "permission-rule-copy", "");
        let title_text = if rule.tool.is_empty() {
            rule.key.clone()
        } else {
            format!("{} · {}", rule.tool, rule.pattern)
        };
        dom::add(
            &copy,
            &dom::el("strong", "permission-rule-title", &title_text),
        );
        if !rule.key.is_empty() && rule.key != title_text {
            dom::add(&copy, &dom::el("code", "permission-rule-key", &rule.key));
        }
        let revoke = dom::button(
            "Revoke",
            "permission-rule-revoke",
            json!({"session_id": sid, "id": rule.id, "rule_key": rule.key}),
        );
        let _ = revoke.class_list().add_1("compact");
        dom::add(&row, &copy);
        dom::add(&row, &revoke);
        dom::add(&list, &row);
    }
}

pub(crate) async fn load_permission_rules(sid: &str) {
    if sid.is_empty() {
        m::write(|s| {
            s.approvals.rules.clear();
            s.approvals.rules_sid.clear();
            s.approvals.rules_loading = false;
            s.approvals.rules_error.clear();
        });
        render_permission_rules_panel();
        return;
    }
    m::write(|s| {
        s.approvals.rules_sid = sid.to_owned();
        s.approvals.rules_loading = true;
        s.approvals.rules_error.clear();
    });
    render_permission_rules_panel();
    let url = format!("/api/sessions/{}/permission-rules", dom::enc(sid));
    match api::get(&url).await {
        Ok(v) => {
            if m::read(|s| s.sid != sid) {
                return;
            }
            let raw = v["rules"]
                .as_array()
                .or_else(|| v.as_array())
                .cloned()
                .unwrap_or_default();
            let parsed: Vec<m::ApprovalRule> =
                raw.iter().filter_map(m::ApprovalRule::from_json).collect();
            m::write(|s| {
                s.approvals.rules = parsed;
                s.approvals.rules_loading = false;
                s.approvals.rules_error.clear();
            });
        }
        Err(e) if !e.aborted => {
            if m::read(|s| s.sid != sid) {
                return;
            }
            m::write(|s| {
                s.approvals.rules_loading = false;
                s.approvals.rules_error = if e.status == 404 {
                    String::new()
                } else {
                    e.message
                };
            });
        }
        Err(_) => {
            m::write(|s| s.approvals.rules_loading = false);
        }
    }
    render_permission_rules_panel();
}

pub(crate) async fn revoke_permission_rule(sid: &str, rule_id: &str, rule_key: &str) {
    if sid.is_empty() {
        return;
    }
    let q = if !rule_id.is_empty() && rule_id != rule_key {
        format!("?id={}&rule_key={}", dom::enc(rule_id), dom::enc(rule_key))
    } else if !rule_key.is_empty() {
        format!("?rule_key={}", dom::enc(rule_key))
    } else {
        format!("?id={}", dom::enc(rule_id))
    };
    let url = format!("/api/sessions/{}/permission-rules{q}", dom::enc(sid));
    match api::delete(&url).await {
        Ok(v) => {
            if let Some(arr) = v["rules"].as_array() {
                let parsed: Vec<m::ApprovalRule> =
                    arr.iter().filter_map(m::ApprovalRule::from_json).collect();
                m::write(|s| {
                    s.approvals.rules = parsed;
                    s.approvals.rules_error.clear();
                });
                render_permission_rules_panel();
            } else {
                load_permission_rules(sid).await;
            }
            dom::announce("run-announcer", "Approval rule revoked.");
        }
        Err(e) if !e.aborted => {
            m::write(|s| s.approvals.rules_error = e.message);
            render_permission_rules_panel();
        }
        Err(_) => {}
    }
}

pub(crate) async fn revoke_all_permission_rules(sid: &str) {
    if sid.is_empty() {
        return;
    }
    let url = format!("/api/sessions/{}/permission-rules?all=1", dom::enc(sid));
    match api::delete(&url).await {
        Ok(_) => {
            m::write(|s| {
                s.approvals.rules.clear();
                s.approvals.rules_error.clear();
            });
            render_permission_rules_panel();
            dom::announce("run-announcer", "All session approval rules revoked.");
        }
        Err(e) if !e.aborted => {
            m::write(|s| s.approvals.rules_error = e.message);
            render_permission_rules_panel();
        }
        Err(_) => {}
    }
}

pub(crate) fn approvals_queue_reconcile_plan(
    existing_keys: &[String],
    next_keys: &[String],
) -> Vec<String> {
    let next_set: std::collections::BTreeSet<&str> = next_keys.iter().map(String::as_str).collect();
    existing_keys
        .iter()
        .filter(|k| !next_set.contains(k.as_str()))
        .cloned()
        .collect()
}

pub(crate) fn is_shortcut_text_input(tag_name: &str, input_type: Option<&str>) -> bool {
    match tag_name {
        "TEXTAREA" | "SELECT" => true,
        "INPUT" => {
            let kind = input_type.unwrap_or("text").trim().to_ascii_lowercase();
            !matches!(kind.as_str(), "radio" | "checkbox" | "button" | "submit")
        }
        _ => false,
    }
}

pub(crate) fn can_target_drawer_shortcut(
    in_pending_card: bool,
    in_drawer_list: bool,
    in_queue_item: bool,
) -> bool {
    in_pending_card || in_drawer_list || in_queue_item
}

pub(crate) fn should_toggle_approvals_drawer_on_alt_a(
    drawer_open: bool,
    has_pending_action: bool,
    palette_open: bool,
) -> bool {
    !palette_open && (!drawer_open || !has_pending_action)
}

enum ApprovalQueueEntry {
    Card {
        key: String,
        label: String,
        sid: String,
        ask: V,
        jump_action: &'static str,
        jump_label: &'static str,
        jump_payload: V,
        origin_kind: &'static str,
        origin_run_id: String,
    },
    Notice {
        key: String,
        title: String,
        pending_count: usize,
        jump_action: &'static str,
        jump_payload: V,
    },
}

impl ApprovalQueueEntry {
    fn key(&self) -> &str {
        match self {
            Self::Card { key, .. } | Self::Notice { key, .. } => key,
        }
    }
}

fn inject_approval_button_payloads(
    block: &web_sys::Element,
    sid: &str,
    origin_kind: &str,
    origin_run_id: &str,
) {
    let Ok(buttons) = block.query_selector_all("button[data-action=\"permission-decide\"]") else {
        return;
    };
    for i in 0..buttons.length() {
        let Some(btn) = buttons
            .item(i)
            .and_then(|e| e.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        let Some(raw_p) = btn.get_attribute("data-payload") else {
            continue;
        };
        if let Ok(mut p) = serde_json::from_str::<V>(&raw_p) {
            p["session_id"] = json!(sid);
            p["container"] = json!("approvals-drawer-list");
            if !origin_kind.is_empty() {
                p["origin_kind"] = json!(origin_kind);
            }
            if !origin_run_id.is_empty() {
                p["origin_run_id"] = json!(origin_run_id);
            }
            let _ = btn.set_attribute("data-payload", &p.to_string());
        }
    }
}

const APPROVALS_DRAWER_SESSION_CAP: usize = 12;

pub(crate) fn approvals_cap_notice(shown: usize, total: usize) -> Option<String> {
    (total > shown).then(|| format!("Showing {shown} of {total} sessions with pending approvals."))
}

fn collect_background_run_approvals(
    entries: &mut Vec<ApprovalQueueEntry>,
    covered_sids: &mut std::collections::BTreeSet<String>,
) {
    for (run_key, origin_kind, title, target_view) in [
        ("system-run", "system", "System orchestration", "systems"),
        (
            "creator-detail",
            "creator",
            "Infrastructure Creator",
            "creator",
        ),
    ] {
        let run_val = m::data(run_key);
        if run_val.is_null() {
            continue;
        }
        let sid = background_run_session_id(&run_val);
        if !sid.is_empty() && covered_sids.contains(&sid) {
            continue;
        }
        let run_id = if run_key == "creator-detail" {
            m::text(&run_val["run"], "id")
        } else {
            m::text(&run_val, "id")
        };
        let waiting = m::array(&run_val["waiting_permissions"]);
        if !waiting.is_empty() && !sid.is_empty() {
            covered_sids.insert(sid.clone());
            for (idx, mut ask) in waiting.into_iter().enumerate() {
                if ask["state"].is_null() {
                    ask["state"] = json!("pending");
                }
                let req_id = m::text(&ask, "request_id");
                let key = if req_id.is_empty() {
                    format!("{origin_kind}:{run_id}:idx:{idx}")
                } else {
                    format!("{origin_kind}:{run_id}:{req_id}")
                };
                entries.push(ApprovalQueueEntry::Card {
                    key,
                    label: title.to_owned(),
                    sid: sid.clone(),
                    ask,
                    jump_action: "approvals-jump-view",
                    jump_label: "Open view",
                    jump_payload: json!(target_view),
                    origin_kind,
                    origin_run_id: run_id.clone(),
                });
            }
            continue;
        }
        let pending_count = background_run_pending_count(&run_val);
        if pending_count > 0 {
            if !sid.is_empty() {
                covered_sids.insert(sid);
            }
            entries.push(ApprovalQueueEntry::Notice {
                key: format!("notice:{run_key}"),
                title: title.to_owned(),
                pending_count,
                jump_action: "approvals-jump-view",
                jump_payload: json!(target_view),
            });
        }
    }
}

pub(crate) async fn refresh_approvals_queue() {
    dom::cancel_timer("approvals-queue-refresh");
    refresh_approvals_queue_inner(true).await;
}

async fn refresh_approvals_queue_inner(force_all: bool) {
    let Some(container) = dom::get("approvals-drawer-list") else {
        return;
    };
    let sessions = m::read(|s| s.sessions.clone());
    let waiting_counts = m::read(|s| s.approvals.waiting_counts.clone());
    let all_waiting_sids: Vec<(String, String, usize)> = sessions
        .iter()
        .filter_map(|s| {
            let sid = m::text(s, "id");
            let count = waiting_counts
                .get(&sid)
                .copied()
                .unwrap_or_else(|| s["waiting_permissions"].as_u64().unwrap_or(0) as usize);
            (count > 0).then(|| (sid, m::summary(s), count))
        })
        .collect();
    let total_waiting_sessions = all_waiting_sids.len();
    let waiting_sids: Vec<(String, String, usize)> = all_waiting_sids
        .into_iter()
        .take(APPROVALS_DRAWER_SESSION_CAP)
        .collect();

    APPROVALS_SESSION_CACHE.with(|cache| {
        let mut map = cache.borrow_mut();
        let active_set: std::collections::BTreeSet<&str> = waiting_sids
            .iter()
            .map(|(sid, _, _)| sid.as_str())
            .collect();
        map.retain(|k, _| active_set.contains(k.as_str()));
    });

    let to_fetch = APPROVALS_SESSION_CACHE
        .with(|cache| approvals_sessions_to_fetch(&waiting_sids, &cache.borrow(), force_all));

    if !to_fetch.is_empty()
        && container
            .query_selector("[data-approval-key]")
            .ok()
            .flatten()
            .is_none()
    {
        container.set_text_content(None);
        dom::add(
            &container,
            &dom::el(
                "p",
                "muted approvals-queue-loading",
                "Loading pending approvals…",
            ),
        );
    }

    let fetches = js_sys::Array::new();
    for (sid, _label) in to_fetch {
        let url = format!("/api/sessions/{}?limit=1&tail=1", dom::enc(&sid));
        let promise = wasm_bindgen_futures::future_to_promise(async move {
            let item = match api::get(&url).await {
                Ok(session) => json!({ "sid": sid, "ok": true, "session": session }),
                Err(e) => {
                    json!({ "sid": sid, "ok": false, "aborted": e.aborted, "error": e.message })
                }
            };
            Ok(wasm_bindgen::JsValue::from_str(&item.to_string()))
        });
        fetches.push(&promise);
    }
    let mut fetch_errors = Vec::<String>::new();
    if fetches.length() > 0 {
        match wasm_bindgen_futures::JsFuture::from(js_sys::Promise::all(&fetches)).await {
            Ok(resolved) => {
                for val in js_sys::Array::from(&resolved).iter() {
                    let Some(raw) = val.as_string() else {
                        continue;
                    };
                    let Ok(parsed) = serde_json::from_str::<V>(&raw) else {
                        continue;
                    };
                    let sid = m::text(&parsed, "sid");
                    if parsed["ok"] != true {
                        if parsed["aborted"] != true {
                            let err_msg = m::text(&parsed, "error");
                            if !err_msg.is_empty() {
                                fetch_errors.push(err_msg);
                            } else {
                                fetch_errors.push(format!("Could not load session {sid}"));
                            }
                        }
                        continue;
                    }
                    let waiting_for_sid = m::array(&parsed["session"]["waiting_permissions"]);
                    if parsed["session"].is_object() {
                        let count = waiting_for_sid.len();
                        m::write(|s| {
                            apply_session_waiting_count(
                                &mut s.sessions,
                                &mut s.approvals.waiting_counts,
                                &sid,
                                count,
                            );
                        });
                        APPROVALS_SESSION_CACHE.with(|cache| {
                            cache
                                .borrow_mut()
                                .insert(sid.clone(), (count, waiting_for_sid));
                        });
                    }
                }
            }
            Err(_) => {
                fetch_errors.push("Could not load pending session approvals.".into());
            }
        }
    }

    let mut entries = Vec::<ApprovalQueueEntry>::new();
    let mut covered_sids = std::collections::BTreeSet::<String>::new();
    let cached_map = APPROVALS_SESSION_CACHE.with(|cache| cache.borrow().clone());
    for (sid, label, _) in &waiting_sids {
        let Some((_, waiting_for_sid)) = cached_map.get(sid) else {
            continue;
        };
        covered_sids.insert(sid.clone());
        for (idx, mut ask) in waiting_for_sid.iter().cloned().enumerate() {
            ask["state"] = json!("pending");
            let req_id = m::text(&ask, "request_id");
            let key = if req_id.is_empty() {
                format!("session:{sid}:idx:{idx}")
            } else {
                format!("session:{sid}:{req_id}")
            };
            entries.push(ApprovalQueueEntry::Card {
                key,
                label: label.clone(),
                sid: sid.clone(),
                ask,
                jump_action: "approvals-jump-session",
                jump_label: "Open session",
                jump_payload: json!(sid),
                origin_kind: "",
                origin_run_id: String::new(),
            });
        }
    }
    collect_background_run_approvals(&mut entries, &mut covered_sids);

    let scroll_top = container.scroll_top();
    let focused_key = dom::document()
        .active_element()
        .filter(|active| container.contains(Some(active)))
        .and_then(|active| active.closest("[data-approval-key]").ok().flatten())
        .and_then(|item| item.get_attribute("data-approval-key"));

    let mut existing_map = std::collections::BTreeMap::<String, web_sys::Element>::new();
    let mut existing_keys = Vec::<String>::new();
    for child in dom::children(&container) {
        if let Some(key) = child.get_attribute("data-approval-key") {
            existing_keys.push(key.clone());
            existing_map.insert(key, child);
        } else {
            child.remove();
        }
    }
    let next_keys: Vec<String> = entries.iter().map(|e| e.key().to_owned()).collect();
    for obsolete in approvals_queue_reconcile_plan(&existing_keys, &next_keys) {
        if let Some(el) = existing_map.remove(&obsolete) {
            el.remove();
        }
    }

    if entries.is_empty() {
        if fetch_errors.is_empty() {
            dom::add(
                &container,
                &dom::el(
                    "p",
                    "muted approvals-queue-empty",
                    "No pending permission approvals across sessions.",
                ),
            );
        }
    } else {
        for (idx, entry) in entries.iter().enumerate() {
            let key = entry.key();
            let wrap = match existing_map.remove(key) {
                Some(existing) => existing,
                None => {
                    let el = dom::el("div", "approvals-queue-item", "");
                    let _ = el.set_attribute("data-approval-key", key);
                    el
                }
            };
            let current_at_idx = container.children().item(idx as u32);
            if current_at_idx.as_ref() != Some(&wrap) {
                let _ = container.insert_before(&wrap, current_at_idx.as_ref().map(|e| e.as_ref()));
            }
            match entry {
                ApprovalQueueEntry::Card {
                    label,
                    sid,
                    ask,
                    jump_action,
                    jump_label,
                    jump_payload,
                    origin_kind,
                    origin_run_id,
                    ..
                } => {
                    let header = wrap
                        .query_selector(".approvals-queue-item-head")
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| {
                            let h = dom::el("div", "approvals-queue-item-head", "");
                            dom::add(&wrap, &h);
                            h
                        });
                    match header.query_selector("strong").ok().flatten() {
                        Some(strong)
                            if strong.text_content().as_deref() == Some(label.as_str()) => {}
                        _ => {
                            header.set_text_content(None);
                            dom::add(&header, &dom::el("strong", "", label));
                            let jump = dom::button(jump_label, jump_action, jump_payload.clone());
                            let _ = jump.class_list().add_1("compact");
                            dom::add(&header, &jump);
                        }
                    }
                    let card_holder = wrap
                        .query_selector(".approvals-queue-card")
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| {
                            let c = dom::el("div", "approvals-queue-card", "");
                            dom::add(&wrap, &c);
                            c
                        });
                    if let Some((block, _)) = upsert_permission_card_in(
                        &card_holder,
                        "approvals-drawer-list",
                        ask,
                        true,
                        true,
                    ) {
                        inject_approval_button_payloads(&block, sid, origin_kind, origin_run_id);
                    }
                }
                ApprovalQueueEntry::Notice {
                    title,
                    pending_count,
                    jump_action,
                    jump_payload,
                    ..
                } => {
                    let text = format!("{title} · {pending_count} waiting");
                    let header = wrap
                        .query_selector(".approvals-queue-item-head")
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| {
                            let h = dom::el("div", "approvals-queue-item-head", "");
                            dom::add(&wrap, &h);
                            h
                        });
                    if header
                        .query_selector("strong")
                        .ok()
                        .flatten()
                        .is_none_or(|s| s.text_content().as_deref() != Some(text.as_str()))
                    {
                        header.set_text_content(None);
                        dom::add(&header, &dom::el("strong", "", &text));
                        let open_btn = dom::button("Open view", jump_action, jump_payload.clone());
                        let _ = open_btn.class_list().add_1("compact");
                        dom::add(&header, &open_btn);
                    }
                }
            }
        }
    }
    if !fetch_errors.is_empty() {
        let msg = if fetch_errors.len() == 1 {
            format!("Could not load pending approvals: {}", fetch_errors[0])
        } else {
            format!(
                "Could not load pending approvals for {} sessions ({}).",
                fetch_errors.len(),
                fetch_errors[0]
            )
        };
        let err_el = dom::el("p", "error-text approvals-queue-error", &msg);
        let _ = err_el.set_attribute("role", "alert");
        dom::add(&container, &err_el);
    }
    if let Some(cap_msg) = approvals_cap_notice(waiting_sids.len(), total_waiting_sessions) {
        dom::add(
            &container,
            &dom::el("p", "muted approvals-queue-cap", &cap_msg),
        );
    }
    container.set_scroll_top(scroll_top);
    if let Some(prev_key) = focused_key {
        if !next_keys.contains(&prev_key) && m::read(|s| s.approvals.drawer_open) {
            if let Some(close) = dom::get("approvals-drawer-close") {
                focus_node(&close);
            }
        }
    }
    schedule_permission_countdowns();
    sync_approvals_badge();
    crate::session::history();
}

pub(crate) async fn open_approvals_drawer() {
    crate::commands::close_palette();
    m::write(|s| s.approvals.drawer_open = true);
    dom::save_focus_for_overlay("approvals-drawer");
    dom::hidden("approvals-drawer", false);
    dom::attr("approvals-queue-toggle", "aria-expanded", "true");
    if let Some(close) = dom::get("approvals-drawer-close") {
        focus_node(&close);
    }
    let sid = m::read(|s| s.sid.clone());
    let queue_promise = wasm_bindgen_futures::future_to_promise(async move {
        refresh_approvals_queue().await;
        if m::read(|s| s.approvals.drawer_open) {
            let active_is_close_or_outside =
                dom::document().active_element().is_none_or(|active| {
                    active.id() == "approvals-drawer-close" || active.tag_name() == "BODY"
                });
            if active_is_close_or_outside {
                if let Some(first_action) = dom::get("approvals-drawer-list").and_then(|list| {
                    list.query_selector(
                        ".conversation-permission[data-state=\"pending\"] button[data-action=\"permission-decide\"]",
                    )
                    .ok()
                    .flatten()
                }) {
                    focus_node(&first_action);
                }
            }
        }
        Ok(wasm_bindgen::JsValue::UNDEFINED)
    });
    let rules_promise = wasm_bindgen_futures::future_to_promise(async move {
        load_permission_rules(&sid).await;
        Ok(wasm_bindgen::JsValue::UNDEFINED)
    });
    let pair = js_sys::Array::new();
    pair.push(&queue_promise);
    pair.push(&rules_promise);
    let _ = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::all(&pair)).await;
}

pub(crate) fn close_approvals_drawer() {
    close_approvals_drawer_with_restore(true);
}

pub(crate) fn close_approvals_drawer_with_restore(restore_focus: bool) {
    let was_open = m::read(|s| s.approvals.drawer_open)
        || dom::get("approvals-drawer")
            .is_some_and(|e| !e.has_attribute("hidden") || e.has_attribute("open"));
    m::write(|s| s.approvals.drawer_open = false);
    dom::attr("approvals-queue-toggle", "aria-expanded", "false");
    if !was_open {
        return;
    }
    dom::hidden_with_restore("approvals-drawer", true, restore_focus);
}

pub(crate) async fn toggle_approvals_drawer() {
    if m::read(|s| s.approvals.drawer_open) {
        close_approvals_drawer();
    } else {
        open_approvals_drawer().await;
    }
}

/// Global Alt+A (open/toggle Approvals drawer when not acting on a pending approval card)
/// and scoped Alt+A (Allow), Alt+Shift+A (Allow session), and Alt+D (Deny) when focused in a pending card or Approvals drawer item.
pub(crate) fn handle_permission_shortcut(event: &web_sys::KeyboardEvent) -> bool {
    if !event.alt_key() || event.ctrl_key() || event.meta_key() || event.is_composing() {
        return false;
    }
    let key = event.key().to_ascii_lowercase();
    let is_a_key = key == "a" || event.code() == "KeyA";
    let is_d_key = key == "d" || event.code() == "KeyD";
    let target_decision = match (is_a_key, is_d_key, event.shift_key()) {
        (true, false, false) => "allow",
        (true, false, true) => "allow_session",
        (false, true, false) => "deny",
        _ => return false,
    };
    let Some(active) = dom::document().active_element() else {
        return false;
    };
    let input_type = active.get_attribute("type");
    if is_shortcut_text_input(&active.tag_name(), input_type.as_deref()) {
        return false;
    }
    let pending_card = active
        .closest(".conversation-permission[data-state=\"pending\"]")
        .ok()
        .flatten();
    let drawer_open = m::read(|s| s.approvals.drawer_open)
        || dom::get("approvals-drawer")
            .is_some_and(|e| !e.has_attribute("hidden") || e.has_attribute("open"));
    let in_open_drawer = drawer_open
        && (active.closest("#approvals-drawer").ok().flatten().is_some()
            || active.tag_name() == "BODY");
    let queue_item = active.closest(".approvals-queue-item").ok().flatten();
    let card =
        if can_target_drawer_shortcut(pending_card.is_some(), in_open_drawer, queue_item.is_some())
        {
            pending_card
                .as_ref()
                .cloned()
                .or_else(|| {
                    queue_item.as_ref().and_then(|item| {
                        item.query_selector(".conversation-permission[data-state=\"pending\"]")
                            .ok()
                            .flatten()
                    })
                })
                .or_else(|| {
                    dom::get("approvals-drawer-list").and_then(|list| {
                        list.query_selector(".conversation-permission[data-state=\"pending\"]")
                            .ok()
                            .flatten()
                    })
                })
        } else {
            None
        };
    let has_pending_card = card.is_some();
    let allow_btn = card.and_then(|c| {
        let sel = format!(
            "button[data-action=\"permission-decide\"][data-decision=\"{target_decision}\"]"
        );
        c.query_selector(&sel)
            .ok()
            .flatten()
            .filter(|btn| !dom::is_disabled(btn))
    });
    if let Some(btn) = allow_btn {
        if let Some(html) = btn.dyn_ref::<web_sys::HtmlElement>() {
            html.click();
            return true;
        }
        return false;
    }
    if target_decision == "allow"
        && should_toggle_approvals_drawer_on_alt_a(
            drawer_open,
            has_pending_card,
            crate::commands::palette_open(),
        )
    {
        wasm_bindgen_futures::spawn_local(async {
            toggle_approvals_drawer().await;
        });
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_runs_dedup_by_session_id_and_cap_notice_formats_cleanly() {
        let sessions = vec![
            json!({"id": "s-sys", "waiting_permissions": 1}),
            json!({"id": "s-chat", "waiting_permissions": 2}),
        ];
        let waiting_counts = std::collections::BTreeMap::from([
            ("s-sys".to_owned(), 1usize),
            ("s-chat".to_owned(), 2usize),
        ]);
        let sys_same_sid = json!({
            "id": "sr-1",
            "session_id": "s-sys",
            "waiting_permissions": [{"request_id": "p1"}]
        });
        let creator_unlisted_sid = json!({
            "run": {"id": "cr-1", "session_id": "s-creator"},
            "waiting_permissions": [{"request_id": "p2"}, {"request_id": "p3"}]
        });
        let unnamed = json!({
            "id": "sr-unnamed",
            "waiting_permissions": 1
        });
        // s-sys (1) + s-chat (2) + s-creator (2) + unnamed (1) = 6 (no double-count of s-sys).
        assert_eq!(
            compute_total_pending_approvals_with_runs(
                &sessions,
                &waiting_counts,
                "s-chat",
                0,
                &[&sys_same_sid, &creator_unlisted_sid, &unnamed],
            ),
            6
        );

        // When s.sessions has stale 0 for s-sys and waiting_counts lacks it, live run count wins via max().
        let stale_sessions = vec![json!({"id": "s-sys", "waiting_permissions": 0})];
        let empty_counts = std::collections::BTreeMap::new();
        assert_eq!(
            compute_total_pending_approvals_with_runs(
                &stale_sessions,
                &empty_counts,
                "",
                0,
                &[&sys_same_sid],
            ),
            1
        );

        assert_eq!(approvals_cap_notice(12, 12), None);
        assert_eq!(
            approvals_cap_notice(12, 17).as_deref(),
            Some("Showing 12 of 17 sessions with pending approvals.")
        );
    }
}
