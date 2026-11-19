use super::*;

pub(crate) fn is_near_bottom(scroll_height: i32, scroll_top: i32, client_height: i32) -> bool {
    scroll_height - scroll_top - client_height < 48
}

/// Joins consecutive streaming `text`/`thinking` chunks without splitting words or collapsing markdown blocks.
pub(crate) fn coalesce_chunk_text(existing: &str, incoming: &str) -> String {
    if existing.is_empty() {
        return incoming.to_owned();
    }
    if incoming.is_empty() {
        return existing.to_owned();
    }
    if existing.ends_with('\n')
        || existing.ends_with(' ')
        || existing.ends_with('\t')
        || incoming.starts_with('\n')
        || incoming.starts_with(' ')
        || incoming.starts_with('\t')
    {
        format!("{existing}{incoming}")
    } else {
        format!("{existing}\n{incoming}")
    }
}

/// Wraps fenced code blocks produced by `render_markdown_html` with a Copy button header.
pub(crate) fn render_assistant_markdown_html(markdown: &str) -> String {
    let raw_html = crate::graph_explanation::render_markdown_html(markdown);
    wrap_markdown_code_blocks(&raw_html)
}

pub(crate) fn wrap_markdown_code_blocks(html: &str) -> String {
    const PRE_PREFIX: &str = "<pre class=\"graph-md-pre\"><code class=\"graph-md-code\"";
    const PRE_SUFFIX: &str = "</code></pre>";
    let mut out = String::with_capacity(html.len() + 128);
    let mut rest = html;
    while let Some(start) = rest.find(PRE_PREFIX) {
        out.push_str(&rest[..start]);
        let after_prefix = &rest[start + PRE_PREFIX.len()..];
        let Some(tag_end) = after_prefix.find('>') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let attrs = &after_prefix[..tag_end];
        let lang = attrs
            .split("data-lang=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .filter(|s| !s.is_empty())
            .unwrap_or("code");
        let after_gt = &after_prefix[tag_end + 1..];
        let Some(close_pos) = after_gt.find(PRE_SUFFIX) else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let code_body = &after_gt[..close_pos];
        out.push_str(&format!(
            "<div class=\"md-code-block\"><div class=\"md-code-head\"><span class=\"md-code-lang\">{}</span><button type=\"button\" class=\"button compact md-copy-code\" data-action=\"copy-code\">Copy</button></div>{PRE_PREFIX}{attrs}>{code_body}{PRE_SUFFIX}</div>",
            files_map::escape_html(lang),
        ));
        rest = &after_gt[close_pos + PRE_SUFFIX.len()..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn conversation_event_call_id(v: &V, resolved_call: Option<&str>) -> String {
    v["call_id"]
        .as_str()
        .or_else(|| v["call"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| resolved_call.map(str::trim).filter(|s| !s.is_empty()))
        .unwrap_or("")
        .to_owned()
}

pub(super) fn extract_hint_paths(v: &V, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["path", "file", "detail"] {
        if let Some(p) = v[key].as_str().map(str::trim).filter(|s| !s.is_empty()) {
            let clean_p = tidy(p);
            if (clean_p.contains('/') || clean_p.contains('.')) && !out.contains(&clean_p) {
                out.push(clean_p);
            }
        }
    }
    for token in text.split_whitespace() {
        let trimmed =
            token.trim_matches(|c: char| matches!(c, '\'' | '"' | ',' | ';' | '(' | ')' | ':'));
        let path_part = trimmed.split(':').next().unwrap_or(trimmed);
        if (path_part.contains('/') || path_part.ends_with(".rs") || path_part.ends_with(".php"))
            && path_part.len() <= 240
            && !path_part.starts_with("http")
        {
            let clean_p = tidy(path_part);
            if !clean_p.is_empty() && !out.contains(&clean_p) {
                out.push(clean_p);
                if out.len() >= 8 {
                    break;
                }
            }
        }
    }
    out
}

pub(crate) fn failure_localize_payload(v: &V, text: &str) -> V {
    let mut queries = Vec::new();
    for key in ["command", "detail", "path"] {
        if let Some(val) = v[key].as_str().map(str::trim).filter(|s| !s.is_empty()) {
            let short: String = val.chars().take(160).collect();
            if !queries.contains(&short) {
                queries.push(short);
            }
        }
    }
    if queries.is_empty() && !text.trim().is_empty() {
        let first_line: String = text
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(160)
            .collect();
        if !first_line.is_empty() {
            queries.push(first_line);
        }
    }
    let hint_paths = extract_hint_paths(v, text);
    let project_id = {
        let p = m::text(&m::read(|s| s.selected.clone()), "project_id");
        if !p.is_empty() {
            p
        } else {
            m::current_project()
        }
    };
    json!({
        "project_id": project_id,
        "session_id": m::read(|s| s.sid.clone()),
        "failing_queries": queries,
        "hint_paths": hint_paths,
        "prefer_full_modal": false
    })
}

fn find_tool_block_by_call_id(call_id: &str) -> Option<web_sys::Element> {
    if call_id.is_empty() {
        return None;
    }
    dom::all("#events details.conversation-tool")
        .into_iter()
        .find(|el| el.get_attribute("data-call-id").as_deref() == Some(call_id))
}

fn format_tool_summary(v: &V) -> String {
    let target = v["path"]
        .as_str()
        .or_else(|| v["detail"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!(" · {}", s.chars().take(80).collect::<String>()))
        .unwrap_or_default();
    let state_label = if starts(v) {
        "Running"
    } else if v["ok"] == false {
        "Failed"
    } else {
        "Completed"
    };
    format!("{} · {state_label}{target}", tool_name(v))
}

fn sync_tool_failure_button(block: &web_sys::Element, v: &V, text: &str, failed: bool) {
    let existing = block
        .query_selector("[data-action=\"localize-run-failure\"]")
        .ok()
        .flatten();
    if !failed {
        if let Some(btn) = existing {
            btn.remove();
        }
        return;
    }
    let payload = failure_localize_payload(v, text);
    match existing {
        Some(btn) => {
            let _ = btn.set_attribute("data-payload", &payload.to_string());
        }
        None => {
            let btn = dom::button("Localize failure on map", "localize-run-failure", payload);
            let _ = btn.class_list().add_1("compact");
            dom::add(block, &btn);
        }
    }
}

pub(crate) fn update_scroll_ui() {
    let unseen = m::read(|s| s.stream.unseen_count);
    let sticky = m::read(|s| s.stream.sticky_bottom);
    let show = !sticky && unseen > 0;
    dom::hidden("events-jump-latest", !show);
    if show {
        let label = if unseen == 1 {
            "Jump to latest (1 new)".to_owned()
        } else {
            format!("Jump to latest ({unseen} new)")
        };
        dom::text("events-jump-latest", &label);
    } else {
        dom::text("events-jump-latest", "Jump to latest");
    }
}

pub(crate) fn on_conversation_scroll() {
    let Some(el) = dom::get("conversation") else {
        return;
    };
    let near = is_near_bottom(el.scroll_height(), el.scroll_top(), el.client_height());
    m::write(|s| {
        s.stream.sticky_bottom = near;
        if near {
            s.stream.unseen_count = 0;
        }
    });
    update_scroll_ui();
}

pub(crate) fn jump_to_latest() {
    if let Some(el) = dom::get("conversation") {
        el.set_scroll_top(el.scroll_height());
    }
    m::write(|s| {
        s.stream.sticky_bottom = true;
        s.stream.unseen_count = 0;
    });
    update_scroll_ui();
}

pub(crate) fn sync_load_earlier_control() {
    let (total, limit, has_older, loading) = m::read(|s| {
        (
            s.events.len(),
            s.stream.visible_limit,
            s.stream.has_older,
            s.stream.loading_older,
        )
    });
    let client_hidden = total.saturating_sub(limit);
    let show = has_older || client_hidden > 0;
    dom::hidden("events-load-earlier-wrap", !show);
    dom::disabled("events-load-earlier", loading);
    if show {
        let label = if loading {
            "Loading earlier messages…".to_owned()
        } else if client_hidden > 0 {
            format!("Load earlier messages ({client_hidden} loaded)")
        } else {
            "Load earlier messages".to_owned()
        };
        dom::text("events-load-earlier", &label);
    }
}

#[allow(dead_code)]
pub(crate) fn conversation(v: &V) {
    conversation_with_call(v, None);
}

pub(super) fn conversation_with_call(v: &V, resolved_call: Option<&str>) {
    let kind = m::text(v, "kind");
    if kind == "permission" {
        permission(v);
        if v["state"].as_str() == Some("pending") || v["state"].is_null() {
            dom::announce("run-announcer", &permission_heading(v));
        }
        return;
    }
    let text = clean(&m::text(v, "text"));
    let id = m::string(&v["id"]);
    if ![
        "text",
        "thinking",
        "tool",
        "status",
        "error",
        "memory",
        "omnigraph",
        "delegation",
        "clash_turn",
        "clash_state",
        "user",
        "result",
    ]
    .contains(&kind.as_str())
    {
        return;
    }
    if matches!(kind.as_str(), "text" | "thinking") && text.is_empty() {
        return;
    }

    let was_sticky = dom::get("conversation")
        .map(|c| {
            m::read(|s| s.stream.sticky_bottom)
                || is_near_bottom(c.scroll_height(), c.scroll_top(), c.client_height())
        })
        .unwrap_or(true);

    let mut mutated_existing = false;
    if kind == "tool" {
        let call_id = conversation_event_call_id(v, resolved_call);
        if let Some(block) = find_tool_block_by_call_id(&call_id) {
            let is_start = starts(v);
            let failed = !is_start && v["ok"] == false;
            let state_str = if is_start {
                "running"
            } else if failed {
                "failed"
            } else {
                "completed"
            };
            let _ = block.set_attribute("data-event-id", &id);
            let _ = block.set_attribute("data-tool-state", state_str);
            if let Ok(Some(summary)) = block.query_selector("summary") {
                summary.set_text_content(Some(&format_tool_summary(v)));
            }
            if let Ok(Some(pre)) = block.query_selector("pre") {
                let prev = pre.text_content().unwrap_or_default();
                let next_text = if prev.is_empty() || is_start {
                    text.clone()
                } else if prev == text {
                    prev
                } else {
                    format!("{prev}\n{text}")
                };
                pre.set_text_content(Some(&next_text));
            }
            sync_tool_failure_button(&block, v, &text, failed);
            mutated_existing = true;
        }
    } else if matches!(kind.as_str(), "text" | "thinking") {
        let can_coalesce = dom::get("events")
            .and_then(|el| el.last_element_child())
            .filter(|last| last.get_attribute("data-chunk-kind").as_deref() == Some(kind.as_str()));
        if let Some(last) = can_coalesce {
            let prev = last.get_attribute("data-raw-text").unwrap_or_default();
            let combined = coalesce_chunk_text(&prev, &text);
            let _ = last.set_attribute("data-raw-text", &combined);
            let _ = last.set_attribute("data-event-id", &id);
            if kind == "text" {
                if let Ok(Some(body)) = last.query_selector(".md-body") {
                    body.set_inner_html(&render_assistant_markdown_html(&combined));
                } else {
                    last.set_inner_html(&format!(
                        "<div class=\"md-body\">{}</div>",
                        render_assistant_markdown_html(&combined)
                    ));
                }
            } else if let Ok(Some(pre)) = last.query_selector("pre.thinking-text") {
                pre.set_text_content(Some(&combined));
            }
            mutated_existing = true;
        }
    }

    if !mutated_existing {
        let block = dom::el(
            if kind == "tool" { "details" } else { "div" },
            &format!("conversation-{kind}"),
            "",
        );
        let _ = block.set_attribute("data-event-id", &id);
        match kind.as_str() {
            "tool" => {
                let call_id = conversation_event_call_id(v, resolved_call);
                if !call_id.is_empty() {
                    let _ = block.set_attribute("data-call-id", &call_id);
                }
                let is_start = starts(v);
                let failed = !is_start && v["ok"] == false;
                let state_str = if is_start {
                    "running"
                } else if failed {
                    "failed"
                } else {
                    "completed"
                };
                let _ = block.set_attribute("data-tool-state", state_str);
                dom::add(&block, &dom::el("summary", "", &format_tool_summary(v)));
                dom::add(&block, &dom::el("pre", "tool-output", &text));
                sync_tool_failure_button(&block, v, &text, failed);
            }
            "text" => {
                let _ = block.set_attribute("data-chunk-kind", "text");
                let _ = block.set_attribute("data-raw-text", &text);
                let body = dom::el("div", "md-body", "");
                body.set_inner_html(&render_assistant_markdown_html(&text));
                dom::add(&block, &body);
            }
            "thinking" => {
                let _ = block.set_attribute("data-chunk-kind", "thinking");
                let _ = block.set_attribute("data-raw-text", &text);
                let details = dom::el("details", "thinking-details", "");
                dom::add(
                    &details,
                    &dom::el("summary", "thinking-summary", "Thinking"),
                );
                dom::add(&details, &dom::el("pre", "thinking-text", &text));
                dom::add(&block, &details);
            }
            "error" => {
                let _ = block.set_attribute("role", "alert");
                dom::add(&block, &dom::el("p", "", &text));
                if !text.trim().is_empty() {
                    let btn = dom::button(
                        "Localize failure on map",
                        "localize-run-failure",
                        failure_localize_payload(v, &text),
                    );
                    let _ = btn.class_list().add_1("compact");
                    dom::add(&block, &btn);
                }
                dom::announce("run-announcer", &format!("Error: {text}"));
            }
            "status" | "result" => {
                dom::add(&block, &dom::el("p", "", &text));
                if kind == "result"
                    || matches!(
                        v["status"].as_str(),
                        Some("completed" | "failed" | "cancelled")
                    )
                {
                    let msg = if text.is_empty() {
                        format!("Turn {}", m::text(v, "status"))
                    } else {
                        text.clone()
                    };
                    dom::announce("run-announcer", &msg);
                }
            }
            _ => {
                dom::add(&block, &dom::el("p", "", &text));
            }
        }
        dom::append("events", &block);
    }

    if let Some(conv) = dom::get("conversation") {
        if was_sticky {
            conv.set_scroll_top(conv.scroll_height());
            m::write(|s| {
                s.stream.sticky_bottom = true;
                s.stream.unseen_count = 0;
            });
        } else {
            m::write(|s| {
                s.stream.sticky_bottom = false;
                s.stream.unseen_count = s.stream.unseen_count.saturating_add(1);
            });
        }
    }
    update_scroll_ui();
    sync_load_earlier_control();
}

/// Re-renders `#events` from `model::events` honoring the current `visible_limit` window.
pub(crate) fn replay_visible_stream() {
    dom::clear("events");
    let (events, limit) = m::read(|s| (s.events.clone(), s.stream.visible_limit));
    let start = events.len().saturating_sub(limit);
    m::take("run");
    m::take("run-before");
    m::take("agents");
    for (idx, ev) in events.iter().enumerate() {
        let (run, replaced, resolved_call) = advance_with_call(m::data("run"), ev);
        if let Some(replaced) = replaced {
            m::set("run-before", replaced);
        }
        m::set("run", run);
        activity(ev);
        if idx >= start {
            conversation_with_call(ev, resolved_call.as_deref());
        }
    }
    sync_load_earlier_control();
}

pub(crate) async fn load_earlier_events() {
    let (sid, has_older, oldest_id, total, limit, loading) = m::read(|s| {
        (
            s.sid.clone(),
            s.stream.has_older,
            s.stream.oldest_event_id,
            s.events.len(),
            s.stream.visible_limit,
            s.stream.loading_older,
        )
    });
    if loading || sid.is_empty() {
        return;
    }
    let prev_metrics = dom::get("conversation").map(|c| (c.scroll_height(), c.scroll_top()));
    let client_hidden = total.saturating_sub(limit);
    if client_hidden > 0 {
        m::write(|s| s.stream.visible_limit = s.stream.visible_limit.saturating_add(120));
        replay_visible_stream();
        if let (Some((prev_h, prev_top)), Some(conv)) = (prev_metrics, dom::get("conversation")) {
            let new_h = conv.scroll_height();
            conv.set_scroll_top(prev_top + (new_h - prev_h));
        }
        return;
    }
    if !has_older {
        sync_load_earlier_control();
        return;
    }
    let Some(before) = oldest_id else {
        m::write(|s| s.stream.has_older = false);
        sync_load_earlier_control();
        return;
    };
    m::write(|s| s.stream.loading_older = true);
    sync_load_earlier_control();
    let epoch = m::read(|s| s.epoch);
    let url = format!("/api/sessions/{}?before={before}&limit=250", dom::enc(&sid));
    let res = crate::api::get(&url).await;
    m::write(|s| s.stream.loading_older = false);
    if m::read(|s| s.sid != sid || s.epoch != epoch) {
        return;
    }
    if let Ok(v) = res {
        let older = m::array(&v["events"]);
        m::write(|s| {
            let mut merged = Vec::with_capacity(older.len() + s.events.len());
            for item in &older {
                let id = m::string(&item["id"]);
                if id.is_empty() || s.seen.insert(id) {
                    merged.push(item.clone());
                }
            }
            s.stream.visible_limit = s.stream.visible_limit.saturating_add(merged.len().max(120));
            merged.append(&mut s.events);
            s.events = merged;
            s.stream.has_older = v["has_older"].as_bool().unwrap_or(false);
            s.stream.oldest_event_id = v["oldest_event_id"].as_u64().or(s.stream.oldest_event_id);
        });
        replay_visible_stream();
        if let (Some((prev_h, prev_top)), Some(conv)) = (prev_metrics, dom::get("conversation")) {
            let new_h = conv.scroll_height();
            conv.set_scroll_top(prev_top + (new_h - prev_h));
        }
    }
    sync_load_earlier_control();
}

pub(crate) async fn copy_code_block(target: &web_sys::Element) {
    let Some(wrapper) = target.closest(".md-code-block").ok().flatten() else {
        return;
    };
    let Some(code_el) = wrapper.query_selector("pre code").ok().flatten() else {
        return;
    };
    let code = code_el.text_content().unwrap_or_default();
    let copying = dom::window().navigator().clipboard().write_text(&code);
    let ok = wasm_bindgen_futures::JsFuture::from(copying).await.is_ok();
    target.set_text_content(Some(if ok { "Copied" } else { "Copy failed" }));
    let btn = target.clone();
    dom::later("md-copy-reset", 1500, move || {
        btn.set_text_content(Some("Copy"));
    });
}
