use super::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct StreamMdEntry {
    settled_bytes: usize,
    settled_dom_blocks: usize,
    tail_blocks: Vec<String>,
}

const TOOL_OUTPUT_PREVIEW_CHARS: usize = 8_000;
const TOOL_OUTPUT_MAX_STORED_CHARS: usize = 512_000;

thread_local! {
    static NEXT_COPY_SEQ: Cell<u64> = const { Cell::new(1) };
    static NEXT_STREAM_SEQ: Cell<u64> = const { Cell::new(1) };
    static STREAM_MD_CACHE: RefCell<BTreeMap<String, StreamMdEntry>> = const { RefCell::new(BTreeMap::new()) };
    static STREAM_RAW_TEXT: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    static CALL_BLOCK_CACHE: RefCell<BTreeMap<String, web_sys::Element>> = const { RefCell::new(BTreeMap::new()) };
    static TOOL_FULL_OUTPUTS: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
}

pub(crate) fn clear_stream_md_cache() {
    STREAM_MD_CACHE.with(|cache| cache.borrow_mut().clear());
    STREAM_RAW_TEXT.with(|raw| raw.borrow_mut().clear());
    CALL_BLOCK_CACHE.with(|cache| cache.borrow_mut().clear());
    TOOL_FULL_OUTPUTS.with(|cache| cache.borrow_mut().clear());
}

fn ensure_block_stream_key(block: &web_sys::Element) -> String {
    if let Some(existing) = block
        .get_attribute("data-stream-key")
        .filter(|s| !s.is_empty())
    {
        return existing;
    }
    let seq = NEXT_STREAM_SEQ.with(|c| {
        let n = c.get();
        c.set(n.saturating_add(1));
        n
    });
    let key = format!("stream-md-{seq}");
    let _ = block.set_attribute("data-stream-key", &key);
    key
}

fn read_block_raw_text(block: &web_sys::Element) -> String {
    let Some(key) = block.get_attribute("data-stream-key") else {
        return String::new();
    };
    STREAM_RAW_TEXT.with(|raw| raw.borrow().get(&key).cloned().unwrap_or_default())
}

fn write_block_raw_text(block: &web_sys::Element, text: &str) {
    let key = ensure_block_stream_key(block);
    STREAM_RAW_TEXT.with(|raw| {
        raw.borrow_mut().insert(key, text.to_owned());
    });
}

pub(crate) fn copy_timer_key(existing: Option<&str>, seq: u64) -> String {
    match existing.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => format!("md-copy-reset-{id}"),
        None => format!("md-copy-reset-{seq}"),
    }
}

fn parse_fence_opener(trimmed: &str) -> Option<(char, usize)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = trimmed.chars().take_while(|c| *c == first).count();
    if count >= 3 {
        if first == '`' && trimmed[count..].contains('`') {
            return None;
        }
        Some((first, count))
    } else {
        None
    }
}

fn is_matching_fence_closer(trimmed: &str, open_ch: char, open_len: usize) -> bool {
    let count = trimmed.chars().take_while(|c| *c == open_ch).count();
    count >= open_len && trimmed[count * open_ch.len_utf8()..].trim().is_empty()
}

fn is_setext_underline(trimmed: &str) -> bool {
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    (first == '=' || first == '-') && trimmed.len() >= 2 && trimmed.chars().all(|c| c == first)
}

/// Finds the furthest safe byte boundary in `markdown` (starting at `start_from`,
/// which must be outside any code fence) where all preceding blocks are complete
/// and `render_markdown_html(&markdown[..b]) + render_markdown_html(&markdown[b..])`
/// equals `render_markdown_html(markdown)`.
pub(crate) fn find_settled_markdown_boundary(markdown: &str, start_from: usize) -> usize {
    if start_from >= markdown.len() || !markdown.is_char_boundary(start_from) {
        return start_from.min(markdown.len());
    }
    let mut fence: Option<(char, usize)> = None;
    let mut best = start_from;
    let mut pos = start_from;
    for chunk in markdown[start_from..].split_inclusive('\n') {
        let ended_with_nl = chunk.ends_with('\n');
        let line = chunk.trim_end_matches(&['\r', '\n'][..]);
        let trimmed = line.trim();
        let next_pos = pos + chunk.len();
        if let Some((open_ch, open_len)) = fence {
            if is_matching_fence_closer(trimmed, open_ch, open_len) {
                fence = None;
                if ended_with_nl {
                    best = next_pos;
                }
            }
        } else if let Some((open_ch, open_len)) = parse_fence_opener(trimmed) {
            fence = Some((open_ch, open_len));
        } else if trimmed.is_empty() && ended_with_nl {
            let rest = &markdown[next_pos..];
            let next_line = rest.split('\n').next().unwrap_or("");
            let next_trimmed = next_line.trim();
            let indented_continuation = next_line.starts_with("  ") || next_line.starts_with('\t');
            let looks_like_setext = !next_trimmed.is_empty() && is_setext_underline(next_trimmed);
            if !indented_continuation && !looks_like_setext {
                best = next_pos;
            }
        }
        pos = next_pos;
    }
    best
}

fn is_void_html_tag(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

/// Splits top-level HTML block elements emitted by `render_assistant_markdown_html`.
pub(crate) fn split_top_level_html_blocks(html: &str) -> Vec<String> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut block_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let mut quote: Option<u8> = None;
        while j < bytes.len() {
            let b = bytes[j];
            if let Some(q) = quote {
                if b == q {
                    quote = None;
                }
            } else if b == b'"' || b == b'\'' {
                quote = Some(b);
            } else if b == b'>' {
                break;
            }
            j += 1;
        }
        if j >= bytes.len() {
            break;
        }
        let tag_inner = html[i + 1..j].trim();
        if !tag_inner.starts_with('!') && !tag_inner.starts_with('?') {
            if tag_inner.starts_with('/') {
                depth = depth.saturating_sub(1);
            } else {
                let name_end = tag_inner
                    .find(|c: char| c.is_ascii_whitespace() || c == '/')
                    .unwrap_or(tag_inner.len());
                let tag_name = tag_inner[..name_end].to_ascii_lowercase();
                let self_closing = tag_inner.ends_with('/') || is_void_html_tag(&tag_name);
                if !self_closing {
                    depth += 1;
                }
            }
        }
        let end_pos = j + 1;
        if depth == 0 && end_pos > block_start {
            let slice = &html[block_start..end_pos];
            if !slice.is_empty() {
                out.push(slice.to_owned());
            }
            block_start = end_pos;
        }
        i = end_pos;
    }
    if block_start < html.len() {
        let rest = &html[block_start..];
        if !rest.is_empty() {
            out.push(rest.to_owned());
        }
    }
    out
}

pub(crate) fn is_near_bottom(scroll_height: i32, scroll_top: i32, client_height: i32) -> bool {
    scroll_height - scroll_top - client_height < 48
}

/// Joins consecutive streaming `text`/`thinking` chunks verbatim without injecting spurious newlines mid-word.
pub(crate) fn coalesce_chunk_text(existing: &str, incoming: &str) -> String {
    if existing.is_empty() {
        return incoming.to_owned();
    }
    if incoming.is_empty() {
        return existing.to_owned();
    }
    format!("{existing}{incoming}")
}

pub(crate) fn bounded_tool_output_preview(
    full: &str,
    max_chars: usize,
) -> (String, Option<(usize, usize)>) {
    let total_chars = full.chars().count();
    if total_chars <= max_chars || max_chars == 0 {
        return (full.to_owned(), None);
    }
    let skip_chars = total_chars - max_chars;
    let mut byte_idx = full
        .char_indices()
        .nth(skip_chars)
        .map(|(i, _)| i)
        .unwrap_or(full.len());
    let snap_window = (full.len() - byte_idx).min(max_chars / 4);
    if let Some(rel_nl) = full[byte_idx..byte_idx + snap_window].find('\n') {
        byte_idx += rel_nl + 1;
    }
    let tail = &full[byte_idx..];
    let total_lines = full.lines().count().max(1);
    let shown_lines = tail.lines().count().max(1);
    (tail.to_owned(), Some((shown_lines, total_lines)))
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

fn find_tool_block_by_call_id(
    container: &web_sys::Element,
    call_id: &str,
) -> Option<web_sys::Element> {
    if call_id.is_empty() {
        return None;
    }
    if container.id() == "events" {
        if let Some(cached) = CALL_BLOCK_CACHE.with(|c| c.borrow().get(call_id).cloned()) {
            if cached.is_connected()
                && container.contains(Some(&cached))
                && cached.get_attribute("data-call-id").as_deref() == Some(call_id)
            {
                return Some(cached);
            }
        }
    }
    let selector = format!(
        "details.conversation-tool[data-call-id=\"{}\"]",
        crate::dom::escape_css_attr_value(call_id)
    );
    let found = container.query_selector(&selector).ok().flatten();
    if container.id() == "events" {
        if let Some(el) = &found {
            CALL_BLOCK_CACHE.with(|c| {
                c.borrow_mut().insert(call_id.to_owned(), el.clone());
            });
        }
    }
    found
}

fn store_tool_output(key: &str, incoming: &str, reset: bool) -> String {
    TOOL_FULL_OUTPUTS.with(|map| {
        let mut map = map.borrow_mut();
        let combined = if reset {
            incoming.to_owned()
        } else if let Some(prev) = map.get(key) {
            if prev.is_empty() {
                incoming.to_owned()
            } else if prev == incoming {
                prev.clone()
            } else {
                format!("{prev}\n{incoming}")
            }
        } else {
            incoming.to_owned()
        };
        let char_count = combined.chars().count();
        let bounded = if char_count > TOOL_OUTPUT_MAX_STORED_CHARS {
            let skip = char_count - TOOL_OUTPUT_MAX_STORED_CHARS;
            let byte_idx = combined
                .char_indices()
                .nth(skip)
                .map(|(i, _)| i)
                .unwrap_or(0);
            format!("[… earlier output omitted …]\n{}", &combined[byte_idx..])
        } else {
            combined
        };
        map.insert(key.to_owned(), bounded.clone());
        bounded
    })
}

fn apply_tool_output_dom(block: &web_sys::Element, output_key: &str, full_text: &str) {
    let expanded = block.get_attribute("data-output-expanded").as_deref() == Some("true");
    let (preview, trunc_info) = bounded_tool_output_preview(full_text, TOOL_OUTPUT_PREVIEW_CHARS);
    if let Ok(Some(pre)) = block.query_selector("pre.tool-output") {
        let shown = if expanded || trunc_info.is_none() {
            full_text
        } else {
            preview.as_str()
        };
        if pre.text_content().as_deref() != Some(shown) {
            pre.set_text_content(Some(shown));
        }
    }
    let existing_trunc = block.query_selector(".tool-output-trunc").ok().flatten();
    if let Some((shown_lines, total_lines)) = trunc_info {
        let bar = existing_trunc.unwrap_or_else(|| {
            let b = dom::el("div", "tool-output-trunc", "");
            let _ = block.set_attribute("data-output-key", output_key);
            if let Ok(Some(pre)) = block.query_selector("pre.tool-output") {
                let _ = pre.after_with_node_1(&b);
            } else {
                dom::add(block, &b);
            }
            b
        });
        bar.set_text_content(None);
        let note_text = if expanded {
            format!("Showing full output ({total_lines} lines)")
        } else {
            format!("Showing last {shown_lines} of {total_lines} lines")
        };
        dom::add(&bar, &dom::el("span", "muted", &note_text));
        let btn = dom::button(
            if expanded {
                "Show tail only"
            } else {
                "Show full output"
            },
            "tool-output-expand",
            json!(output_key),
        );
        let _ = btn.class_list().add_1("compact");
        dom::add(&bar, &btn);
    } else if let Some(bar) = existing_trunc {
        bar.remove();
    }
}

pub(crate) fn toggle_tool_output_expand(output_key: &str) {
    if output_key.is_empty() {
        return;
    }
    let selector = format!(
        "details.conversation-tool[data-output-key=\"{}\"]",
        crate::dom::escape_css_attr_value(output_key)
    );
    let Ok(Some(block)) = dom::document().query_selector(&selector) else {
        return;
    };
    let expanded = block.get_attribute("data-output-expanded").as_deref() == Some("true");
    let _ = block.set_attribute(
        "data-output-expanded",
        if expanded { "false" } else { "true" },
    );
    let full_text = TOOL_FULL_OUTPUTS.with(|map| map.borrow().get(output_key).cloned());
    if let Some(full) = full_text {
        apply_tool_output_dom(&block, output_key, &full);
    }
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

fn parse_single_html_node(html: &str) -> Option<web_sys::Element> {
    let wrapper = dom::el("div", "", "");
    wrapper.set_inner_html(html);
    wrapper.first_element_child()
}

fn try_patch_streaming_code_block(existing: &web_sys::Element, new_html: &str) -> bool {
    if !existing.class_list().contains("md-code-block") {
        return false;
    }
    let Some(fresh) = parse_single_html_node(new_html) else {
        return false;
    };
    if !fresh.class_list().contains("md-code-block") {
        return false;
    }
    let old_lang = existing
        .query_selector(".md-code-lang")
        .ok()
        .flatten()
        .and_then(|e| e.text_content());
    let new_lang = fresh
        .query_selector(".md-code-lang")
        .ok()
        .flatten()
        .and_then(|e| e.text_content());
    if old_lang != new_lang {
        return false;
    }
    let (Ok(Some(old_code)), Ok(Some(new_code))) = (
        existing.query_selector("pre code"),
        fresh.query_selector("pre code"),
    ) else {
        return false;
    };
    old_code.set_text_content(new_code.text_content().as_deref());
    true
}

fn init_stream_markdown_body(block: &web_sys::Element, body: &web_sys::Element, text: &str) {
    let stream_key = ensure_block_stream_key(block);

    let html = render_assistant_markdown_html(text);
    body.set_inner_html(&html);
    let settled_bytes = find_settled_markdown_boundary(text, 0);
    let entry = if settled_bytes > 0 {
        let settled_count =
            split_top_level_html_blocks(&render_assistant_markdown_html(&text[..settled_bytes]))
                .len();
        let tail_blocks =
            split_top_level_html_blocks(&render_assistant_markdown_html(&text[settled_bytes..]));
        StreamMdEntry {
            settled_bytes,
            settled_dom_blocks: settled_count,
            tail_blocks,
        }
    } else {
        StreamMdEntry {
            settled_bytes: 0,
            settled_dom_blocks: 0,
            tail_blocks: split_top_level_html_blocks(&html),
        }
    };
    STREAM_MD_CACHE.with(|cache| {
        cache.borrow_mut().insert(stream_key, entry);
    });
}

fn patch_stream_markdown_body(block: &web_sys::Element, body: &web_sys::Element, combined: &str) {
    let stream_key = block.get_attribute("data-stream-key").unwrap_or_default();
    if stream_key.is_empty() {
        init_stream_markdown_body(block, body, combined);
        return;
    }
    let mut entry = STREAM_MD_CACHE
        .with(|cache| cache.borrow().get(&stream_key).cloned())
        .unwrap_or_default();
    if entry.settled_bytes > combined.len() || !combined.is_char_boundary(entry.settled_bytes) {
        init_stream_markdown_body(block, body, combined);
        return;
    }

    let tail_md = &combined[entry.settled_bytes..];
    let tail_html = render_assistant_markdown_html(tail_md);
    let new_tail_blocks = split_top_level_html_blocks(&tail_html);
    let keep = entry
        .tail_blocks
        .iter()
        .zip(new_tail_blocks.iter())
        .take_while(|(old_b, new_b)| old_b == new_b)
        .count();

    let children = body.children();
    let expected_total = entry.settled_dom_blocks + entry.tail_blocks.len();
    if (children.length() as usize) != expected_total {
        // DOM diverged unexpectedly; fall back cleanly and re-seed cache.
        init_stream_markdown_body(block, body, combined);
        return;
    }

    let start_idx = entry.settled_dom_blocks + keep;
    let existing_replace_end =
        (entry.settled_dom_blocks + new_tail_blocks.len()).min(expected_total);
    let mut idx = start_idx;
    while idx < existing_replace_end {
        let tail_idx = idx - entry.settled_dom_blocks;
        if let Some(existing_el) = children.item(idx as u32) {
            if !try_patch_streaming_code_block(&existing_el, &new_tail_blocks[tail_idx]) {
                if let Some(fresh_el) = parse_single_html_node(&new_tail_blocks[tail_idx]) {
                    let _ = existing_el.replace_with_with_node_1(&fresh_el);
                }
            }
        }
        idx += 1;
    }

    let target_total = entry.settled_dom_blocks + new_tail_blocks.len();
    while (body.children().length() as usize) > target_total {
        if let Some(last_child) = body.last_element_child() {
            last_child.remove();
        } else {
            break;
        }
    }
    for new_block in new_tail_blocks
        .iter()
        .skip(existing_replace_end - entry.settled_dom_blocks)
    {
        if let Some(fresh_el) = parse_single_html_node(new_block) {
            let _ = body.append_child(&fresh_el);
        }
    }

    let next_settled = find_settled_markdown_boundary(combined, entry.settled_bytes);
    if next_settled > entry.settled_bytes && combined.is_char_boundary(next_settled) {
        let newly_settled_count = split_top_level_html_blocks(&render_assistant_markdown_html(
            &combined[entry.settled_bytes..next_settled],
        ))
        .len();
        if newly_settled_count <= new_tail_blocks.len() {
            entry.settled_bytes = next_settled;
            entry.settled_dom_blocks += newly_settled_count;
            entry.tail_blocks = new_tail_blocks[newly_settled_count..].to_vec();
        } else {
            entry.tail_blocks = new_tail_blocks;
        }
    } else {
        entry.tail_blocks = new_tail_blocks;
    }

    STREAM_MD_CACHE.with(|cache| {
        cache.borrow_mut().insert(stream_key, entry);
    });
}

#[allow(dead_code)]
pub(crate) fn conversation(v: &V) {
    conversation_with_call(v, None);
}

pub(super) fn conversation_with_call(v: &V, resolved_call: Option<&str>) {
    let Some(events_el) = dom::get("events") else {
        return;
    };
    let was_sticky = dom::get("conversation")
        .map(|c| {
            m::read(|s| s.stream.sticky_bottom)
                || is_near_bottom(c.scroll_height(), c.scroll_top(), c.client_height())
        })
        .unwrap_or(true);
    if !render_event_into(&events_el, v, resolved_call, true) {
        return;
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

fn render_event_into(
    container: &web_sys::Element,
    v: &V,
    resolved_call: Option<&str>,
    live_mode: bool,
) -> bool {
    let kind = m::text(v, "kind");
    if kind == "permission" {
        if live_mode {
            permission(v);
            if v["state"].as_str() == Some("pending") || v["state"].is_null() {
                dom::announce("run-announcer", &permission_heading(v));
            }
        } else {
            let request = m::text(v, "request_id");
            let already_in_events = !request.is_empty()
                && dom::get("events")
                    .and_then(|ev| {
                        ev.query_selector(&format!(
                            "[data-permission-id=\"{}\"]",
                            crate::dom::escape_css_attr_value(&request)
                        ))
                        .ok()
                        .flatten()
                    })
                    .is_some();
            if !request.is_empty() && !already_in_events {
                super::permissions::upsert_permission_card_in(container, "events", v, false, false);
            }
        }
        return false;
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
        return false;
    }
    if matches!(kind.as_str(), "text" | "thinking") && text.is_empty() {
        return false;
    }

    let mut mutated_existing = false;
    if kind == "tool" {
        let call_id = conversation_event_call_id(v, resolved_call);
        if let Some(block) = find_tool_block_by_call_id(container, &call_id) {
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
            let output_key = if !call_id.is_empty() {
                call_id.clone()
            } else {
                format!("ev:{id}")
            };
            let _ = block.set_attribute("data-output-key", &output_key);
            let full_text = store_tool_output(&output_key, &text, is_start);
            apply_tool_output_dom(&block, &output_key, &full_text);
            sync_tool_failure_button(&block, v, &full_text, failed);
            mutated_existing = true;
        }
    } else if matches!(kind.as_str(), "text" | "thinking") {
        let can_coalesce = container
            .last_element_child()
            .filter(|last| last.get_attribute("data-chunk-kind").as_deref() == Some(kind.as_str()));
        if let Some(last) = can_coalesce {
            let prev = read_block_raw_text(&last);
            let combined = coalesce_chunk_text(&prev, &text);
            write_block_raw_text(&last, &combined);
            let _ = last.set_attribute("data-event-id", &id);
            if kind == "text" {
                if let Ok(Some(body)) = last.query_selector(".md-body") {
                    if live_mode {
                        patch_stream_markdown_body(&last, &body, &combined);
                    } else {
                        body.set_inner_html(&render_assistant_markdown_html(&combined));
                    }
                } else {
                    let body = dom::el("div", "md-body", "");
                    if live_mode {
                        init_stream_markdown_body(&last, &body, &combined);
                    } else {
                        body.set_inner_html(&render_assistant_markdown_html(&combined));
                    }
                    last.set_text_content(None);
                    dom::add(&last, &body);
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
                    if container.id() == "events" {
                        CALL_BLOCK_CACHE.with(|c| {
                            c.borrow_mut().insert(call_id.clone(), block.clone());
                        });
                    }
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
                let output_key = if !call_id.is_empty() {
                    call_id.clone()
                } else {
                    format!("ev:{id}")
                };
                let _ = block.set_attribute("data-tool-state", state_str);
                let _ = block.set_attribute("data-output-key", &output_key);
                dom::add(&block, &dom::el("summary", "", &format_tool_summary(v)));
                dom::add(&block, &dom::el("pre", "tool-output", ""));
                let full_text = store_tool_output(&output_key, &text, true);
                apply_tool_output_dom(&block, &output_key, &full_text);
                sync_tool_failure_button(&block, v, &full_text, failed);
            }
            "text" => {
                let _ = block.set_attribute("data-chunk-kind", "text");
                write_block_raw_text(&block, &text);
                let body = dom::el("div", "md-body", "");
                if live_mode {
                    init_stream_markdown_body(&block, &body, &text);
                } else {
                    body.set_inner_html(&render_assistant_markdown_html(&text));
                }
                dom::add(&block, &body);
            }
            "thinking" => {
                let _ = block.set_attribute("data-chunk-kind", "thinking");
                write_block_raw_text(&block, &text);
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
                if live_mode {
                    dom::announce("run-announcer", &format!("Error: {text}"));
                }
            }
            "result" => {
                if !text.trim().is_empty() {
                    let body = dom::el("div", "md-body", "");
                    body.set_inner_html(&render_assistant_markdown_html(&text));
                    dom::add(&block, &body);
                }
                if live_mode {
                    let msg = if text.is_empty() {
                        format!("Turn {}", m::text(v, "status"))
                    } else {
                        text.clone()
                    };
                    dom::announce("run-announcer", &msg);
                }
            }
            "status" => {
                dom::add(&block, &dom::el("p", "", &text));
                if live_mode
                    && matches!(
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
            "user" => {
                if !text.trim().is_empty() {
                    let body = dom::el("div", "md-body", "");
                    body.set_inner_html(&render_assistant_markdown_html(&text));
                    dom::add(&block, &body);
                }
            }
            _ => {
                dom::add(&block, &dom::el("p", "", &text));
            }
        }
        dom::add(container, &block);
    }
    true
}

fn prepend_events_range(start: usize, end: usize) {
    let Some(events_el) = dom::get("events") else {
        return;
    };
    if start >= end {
        return;
    }
    let events = m::read(|s| s.events.clone());
    let end = end.min(events.len());
    if start >= end {
        return;
    }
    let staging = dom::el("div", "", "");
    let mut scan_run = initial();
    for (idx, ev) in events.iter().enumerate().take(end) {
        let (next_run, _, resolved_call) = advance_with_call(scan_run, ev);
        scan_run = next_run;
        if idx >= start {
            render_event_into(&staging, ev, resolved_call.as_deref(), false);
        }
    }
    let anchor = events_el.first_child();
    while let Some(node) = staging.first_element_child() {
        let _ = events_el.insert_before(&node, anchor.as_ref());
    }
}

/// Re-renders `#events` from `model::events` honoring the current `visible_limit` window
/// with a single scroll-sticky measurement before and after the loop.
pub(crate) fn replay_visible_stream() {
    clear_stream_md_cache();
    dom::clear("events");
    let was_sticky = dom::get("conversation")
        .map(|c| {
            m::read(|s| s.stream.sticky_bottom)
                || is_near_bottom(c.scroll_height(), c.scroll_top(), c.client_height())
        })
        .unwrap_or(true);
    let events_el = dom::get("events");
    let (events, limit) = m::read(|s| (s.events.clone(), s.stream.visible_limit));
    let start = events.len().saturating_sub(limit);
    m::take("run");
    m::take("run-before");
    m::take("agents");
    let mut rendered_any = false;
    for (idx, ev) in events.iter().enumerate() {
        let (run, replaced, resolved_call) = advance_with_call(m::data("run"), ev);
        if let Some(replaced) = replaced {
            m::set("run-before", replaced);
        }
        m::set("run", run);
        activity(ev);
        if idx >= start {
            if let Some(ref container) = events_el {
                if render_event_into(container, ev, resolved_call.as_deref(), true) {
                    rendered_any = true;
                }
            }
        }
    }
    if rendered_any {
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
        let old_start = total.saturating_sub(limit);
        let new_limit = m::write(|s| {
            s.stream.visible_limit = s.stream.visible_limit.saturating_add(120);
            s.stream.visible_limit
        });
        let new_start = total.saturating_sub(new_limit);
        prepend_events_range(new_start, old_start);
        sync_load_earlier_control();
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
    match res {
        Ok(v) => {
            dom::error("events-error", "");
            let older = m::array(&v["events"]);
            let (new_start, old_start_after) = m::write(|s| {
                let prev_total = s.events.len();
                let prev_old_start = prev_total.saturating_sub(s.stream.visible_limit);
                let mut merged = Vec::with_capacity(older.len() + prev_total);
                for item in &older {
                    let id = m::string(&item["id"]);
                    if id.is_empty() || s.seen.insert(id) {
                        merged.push(item.clone());
                    }
                }
                let added_len = merged.len();
                s.stream.visible_limit = s.stream.visible_limit.saturating_add(added_len.max(120));
                merged.append(&mut s.events);
                s.events = merged;
                s.stream.has_older = v["has_older"].as_bool().unwrap_or(false);
                s.stream.oldest_event_id =
                    v["oldest_event_id"].as_u64().or(s.stream.oldest_event_id);
                let total_after = s.events.len();
                let new_start = total_after.saturating_sub(s.stream.visible_limit);
                let old_start_after = added_len + prev_old_start;
                (new_start, old_start_after)
            });
            prepend_events_range(new_start, old_start_after);
            if let (Some((prev_h, prev_top)), Some(conv)) = (prev_metrics, dom::get("conversation"))
            {
                let new_h = conv.scroll_height();
                conv.set_scroll_top(prev_top + (new_h - prev_h));
            }
        }
        Err(e) if !e.aborted => {
            dom::error("events-error", &e.message);
        }
        Err(_) => {}
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
    let copy_id = match target
        .get_attribute("data-copy-id")
        .filter(|s| !s.is_empty())
    {
        Some(id) => id,
        None => {
            let seq = NEXT_COPY_SEQ.with(|c| {
                let n = c.get();
                c.set(n.saturating_add(1));
                n
            });
            let id = seq.to_string();
            let _ = target.set_attribute("data-copy-id", &id);
            id
        }
    };
    let timer_key = copy_timer_key(Some(&copy_id), 0);
    let btn = target.clone();
    dom::later(&timer_key, 1500, move || {
        btn.set_text_content(Some("Copy"));
    });
}
