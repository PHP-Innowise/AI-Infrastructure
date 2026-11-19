use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffFileSection {
    pub path: String,
    pub old_path: String,
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
    pub raw: String,
}

pub(super) fn diff_line_kind(line: &str) -> &'static str {
    if line.starts_with("+++")
        || line.starts_with("---")
        || line.starts_with("diff --git")
        || line.starts_with("index ")
        || line.starts_with("new file mode")
        || line.starts_with("deleted file mode")
        || line.starts_with("rename ")
        || line.starts_with("similarity index")
    {
        "meta"
    } else if line.starts_with("@@") {
        "hunk"
    } else if line.starts_with('+') {
        "add"
    } else if line.starts_with('-') {
        "del"
    } else {
        "ctx"
    }
}

pub(super) fn parse_hunk_start_lines(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix("@@")?.trim_start();
    let minus_part = rest.strip_prefix('-')?;
    let (old_tok, after_old) = minus_part.split_once(' ')?;
    let old_start = old_tok
        .split(',')
        .next()
        .and_then(|n| n.parse::<usize>().ok())?;
    let plus_part = after_old.trim_start().strip_prefix('+')?;
    let new_tok = plus_part.split(&[' ', '@'][..]).next()?;
    let new_start = new_tok
        .split(',')
        .next()
        .and_then(|n| n.parse::<usize>().ok())?;
    Some((old_start, new_start))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffNumberedLine {
    pub kind: &'static str,
    pub old_ln: Option<usize>,
    pub new_ln: Option<usize>,
    pub text: String,
}

pub(crate) fn compute_diff_numbered_lines(diff: &str) -> Vec<DiffNumberedLine> {
    let normalized = clean(diff).replace("\r\n", "\n");
    let mut out = Vec::new();
    let mut old_ln: Option<usize> = None;
    let mut new_ln: Option<usize> = None;
    for line in normalized.lines() {
        let kind = diff_line_kind(line);
        match kind {
            "hunk" => {
                if let Some((o, n)) = parse_hunk_start_lines(line) {
                    old_ln = Some(o);
                    new_ln = Some(n);
                }
                out.push(DiffNumberedLine {
                    kind,
                    old_ln: None,
                    new_ln: None,
                    text: line.to_owned(),
                });
            }
            "meta" => out.push(DiffNumberedLine {
                kind,
                old_ln: None,
                new_ln: None,
                text: line.to_owned(),
            }),
            "del" => {
                let ln = old_ln;
                if let Some(v) = &mut old_ln {
                    *v = v.saturating_add(1);
                }
                out.push(DiffNumberedLine {
                    kind,
                    old_ln: ln,
                    new_ln: None,
                    text: line.to_owned(),
                });
            }
            "add" => {
                let ln = new_ln;
                if let Some(v) = &mut new_ln {
                    *v = v.saturating_add(1);
                }
                out.push(DiffNumberedLine {
                    kind,
                    old_ln: None,
                    new_ln: ln,
                    text: line.to_owned(),
                });
            }
            _ => {
                let ol = old_ln;
                let nl = new_ln;
                if let Some(v) = &mut old_ln {
                    *v = v.saturating_add(1);
                }
                if let Some(v) = &mut new_ln {
                    *v = v.saturating_add(1);
                }
                out.push(DiffNumberedLine {
                    kind,
                    old_ln: ol,
                    new_ln: nl,
                    text: line.to_owned(),
                });
            }
        }
    }
    out
}

pub(crate) fn render_diff_file_lines_html(
    diff: &str,
    max_lines: Option<usize>,
) -> (String, usize, bool) {
    let lines = compute_diff_numbered_lines(diff);
    let total = lines.len();
    let limit = max_lines.unwrap_or(total);
    let truncated = total > limit;
    let mut out = String::new();
    for entry in lines.iter().take(limit) {
        let escaped = if entry.text.is_empty() {
            " ".to_owned()
        } else {
            files_map::escape_html(&entry.text)
        };
        if entry.kind == "hunk" {
            out.push_str(&format!(
                "<span class=\"diff-hunk-bar permission-diff-line\" data-kind=\"hunk\">\
                 <span class=\"diff-hunk-head\">{escaped}</span>\
                 <button type=\"button\" class=\"button compact diff-copy-hunk\" data-action=\"diff-copy-hunk\">Copy hunk</button>\
                 </span>"
            ));
            continue;
        }
        let old_col = entry.old_ln.map(|n| n.to_string()).unwrap_or_default();
        let new_col = entry.new_ln.map(|n| n.to_string()).unwrap_or_default();
        out.push_str(&format!(
            "<span class=\"diff-line permission-diff-line\" data-kind=\"{}\">\
             <span class=\"diff-ln diff-ln-old\" aria-hidden=\"true\">{old_col}</span>\
             <span class=\"diff-ln diff-ln-new\" aria-hidden=\"true\">{new_col}</span>\
             <span class=\"diff-code\">{escaped}</span>\
             </span>",
            entry.kind
        ));
    }
    (out, total, truncated)
}

pub(crate) fn render_diff_preview_lines_html(diff: &str) -> String {
    let normalized = clean(diff).replace("\r\n", "\n");
    let mut out = String::new();
    for line in normalized.lines() {
        let kind = diff_line_kind(line);
        let content = if line.is_empty() {
            " ".to_owned()
        } else {
            files_map::escape_html(line)
        };
        out.push_str(&format!(
            "<span class=\"permission-diff-line\" data-kind=\"{kind}\">{content}</span>"
        ));
    }
    out
}

fn strip_diff_prefix(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"');
    let without_tab = trimmed.split('\t').next().unwrap_or(trimmed).trim();
    if without_tab == "/dev/null" {
        return String::new();
    }
    without_tab
        .strip_prefix("a/")
        .or_else(|| without_tab.strip_prefix("b/"))
        .unwrap_or(without_tab)
        .to_owned()
}

fn parse_git_header_paths(header: &str) -> (String, String) {
    let rest = header.trim_start_matches("diff --git ").trim();
    if let Some((left, right)) = rest.split_once(" b/") {
        return (strip_diff_prefix(left), strip_diff_prefix(right));
    }
    let mut parts = rest.split_whitespace();
    let old = parts.next().map(strip_diff_prefix).unwrap_or_default();
    let new = parts.next().map(strip_diff_prefix).unwrap_or_default();
    (old, new)
}

/// Splits a unified git diff into per-file sections with +/- stats and change status.
pub(crate) fn parse_diff_files(diff: &str) -> Vec<DiffFileSection> {
    let normalized = clean(diff).replace("\r\n", "\n");
    if normalized.trim().is_empty() {
        return Vec::new();
    }
    let mut sections = Vec::<DiffFileSection>::new();
    let mut current_lines = Vec::<&str>::new();
    let mut old_path = String::new();
    let mut new_path = String::new();
    let mut status = String::from("modified");
    let mut additions = 0usize;
    let mut deletions = 0usize;

    let flush = |sections: &mut Vec<DiffFileSection>,
                 lines: &mut Vec<&str>,
                 old_p: &mut String,
                 new_p: &mut String,
                 st: &mut String,
                 adds: &mut usize,
                 dels: &mut usize| {
        if lines.is_empty() {
            return;
        }
        let path = if !new_p.is_empty() {
            new_p.clone()
        } else if !old_p.is_empty() {
            old_p.clone()
        } else {
            "Changes".to_owned()
        };
        sections.push(DiffFileSection {
            path,
            old_path: old_p.clone(),
            status: st.clone(),
            additions: *adds,
            deletions: *dels,
            raw: lines.join("\n"),
        });
        lines.clear();
        old_p.clear();
        new_p.clear();
        *st = "modified".into();
        *adds = 0;
        *dels = 0;
    };

    for line in normalized.lines() {
        if line.starts_with("diff --git ") {
            flush(
                &mut sections,
                &mut current_lines,
                &mut old_path,
                &mut new_path,
                &mut status,
                &mut additions,
                &mut deletions,
            );
            let (old, new) = parse_git_header_paths(line);
            old_path = old;
            new_path = new;
        } else if line.starts_with("--- ") {
            let p = strip_diff_prefix(line.trim_start_matches("--- "));
            if p.is_empty() {
                status = "added".into();
            } else {
                if !current_lines.is_empty()
                    && !current_lines
                        .first()
                        .is_some_and(|first| first.starts_with("diff --git "))
                    && current_lines.iter().any(|prev| prev.starts_with("+++ "))
                {
                    flush(
                        &mut sections,
                        &mut current_lines,
                        &mut old_path,
                        &mut new_path,
                        &mut status,
                        &mut additions,
                        &mut deletions,
                    );
                }
                if old_path.is_empty() {
                    old_path = p;
                }
            }
        } else if let Some(after) = line.strip_prefix("+++ ") {
            let p = strip_diff_prefix(after);
            if p.is_empty() {
                status = "deleted".into();
            } else {
                new_path = p;
            }
        } else if line.starts_with("new file mode") {
            status = "added".into();
        } else if line.starts_with("deleted file mode") {
            status = "deleted".into();
        } else if line.starts_with("rename ") {
            status = "renamed".into();
        } else if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
        current_lines.push(line);
    }
    flush(
        &mut sections,
        &mut current_lines,
        &mut old_path,
        &mut new_path,
        &mut status,
        &mut additions,
        &mut deletions,
    );
    sections
}

const DIFF_PREVIEW_MAX_LINES: usize = 250;
const DIFF_AUTO_OPEN_MAX_FILES: usize = 8;
const DIFF_AUTO_OPEN_MAX_LINES: usize = 400;

thread_local! {
    static EXPANDED_DIFF_FILES: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    static DIFF_TOGGLE_WIRED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static DIFF_FULL_FILES: std::cell::RefCell<std::collections::BTreeMap<String, String>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    static NEXT_DIFF_COPY_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

pub(crate) fn should_default_open_diff_file(
    file_count: usize,
    idx: usize,
    line_count: usize,
) -> bool {
    if file_count <= DIFF_AUTO_OPEN_MAX_FILES {
        line_count <= DIFF_AUTO_OPEN_MAX_LINES || (idx == 0 && line_count <= 600)
    } else {
        idx == 0 && line_count <= DIFF_AUTO_OPEN_MAX_LINES
    }
}

fn diff_collapse_key(scope_id: &str, path: &str) -> String {
    format!("{scope_id}:{path}")
}

pub(crate) fn is_diff_file_open(
    default_open: bool,
    key: &str,
    expanded: &std::collections::HashSet<String>,
    collapsed: &std::collections::BTreeSet<String>,
) -> bool {
    if expanded.contains(key) {
        true
    } else if collapsed.contains(key) {
        false
    } else {
        default_open
    }
}

pub(crate) fn diff_content_hash(scope_id: &str, diff: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in scope_id
        .as_bytes()
        .iter()
        .chain(b":")
        .chain(diff.as_bytes().iter())
    {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}:{}", diff.len())
}

pub(crate) fn set_diff_file_open(scope_id: &str, path: &str, open: bool) {
    if path.is_empty() {
        return;
    }
    let key = diff_collapse_key(scope_id, path);
    EXPANDED_DIFF_FILES.with(|set| {
        let mut set = set.borrow_mut();
        if open {
            set.insert(key.clone());
        } else {
            set.remove(&key);
        }
    });
    m::write(|s| {
        if open {
            s.diff_view.collapsed_files.remove(&key);
        } else {
            s.diff_view.collapsed_files.insert(key);
        }
    });
}

pub(crate) fn reset_diff_view_state() {
    EXPANDED_DIFF_FILES.with(|set| set.borrow_mut().clear());
    DIFF_FULL_FILES.with(|map| map.borrow_mut().clear());
    m::write(|s| {
        s.diff_view.collapsed_files.clear();
        s.diff_view.focused_file.clear();
    });
    for id in [
        "results-diff",
        "workspace-diff-viewer",
        "results-files",
        "workspace-diff-files",
    ] {
        if let Some(el) = dom::get(id) {
            let _ = el.remove_attribute("data-diff-hash");
            let _ = el.remove_attribute("data-diff-empty");
            let _ = el.remove_attribute("data-files-hash");
        }
    }
}

fn ensure_diff_toggle_listener() {
    if DIFF_TOGGLE_WIRED.with(|wired| wired.replace(true)) {
        return;
    }
    let cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        |event: web_sys::Event| {
            let Some(target) = event
                .target()
                .and_then(|v| v.dyn_into::<web_sys::HtmlDetailsElement>().ok())
            else {
                return;
            };
            if !target.class_list().contains("diff-file-group") {
                return;
            }
            let scope = target.get_attribute("data-diff-scope").unwrap_or_default();
            let path = target.get_attribute("data-diff-file").unwrap_or_default();
            set_diff_file_open(&scope, &path, target.open());
        },
    );
    let _ = dom::document().add_event_listener_with_callback_and_bool(
        "toggle",
        cb.as_ref().unchecked_ref(),
        true,
    );
    cb.forget();
}

/// Renders a file-grouped collapsible diff viewer with jump anchors and zero inline hex colors.
pub(crate) fn render_grouped_diff_html(diff: &str, scope_id: &str) -> String {
    let sections = parse_diff_files(diff);
    if sections.is_empty() {
        return "<p class=\"muted\">No diff output recorded.</p>".into();
    }
    let (expanded, collapsed) = EXPANDED_DIFF_FILES.with(|set| {
        (
            set.borrow().clone(),
            m::read(|s| s.diff_view.collapsed_files.clone()),
        )
    });
    let total_adds: usize = sections.iter().map(|s| s.additions).sum();
    let total_dels: usize = sections.iter().map(|s| s.deletions).sum();
    let file_word = if sections.len() == 1 { "file" } else { "files" };
    let scope_esc = files_map::escape_html(scope_id);

    let mut out = format!(
        "<div class=\"diff-viewer\" data-diff-scope=\"{scope_esc}\" data-total-files=\"{}\" data-total-adds=\"{total_adds}\" data-total-dels=\"{total_dels}\">\
         <div class=\"diff-viewer-summary\">\
           <span class=\"diff-viewer-counts\">{} {file_word} · <span class=\"diff-stat-add\">+{total_adds}</span> <span class=\"diff-stat-del\">-{total_dels}</span></span>\
           <span class=\"diff-viewer-controls\">\
             <input type=\"search\" class=\"diff-file-filter\" data-action=\"diff-filter\" data-payload='{{\"scope\":\"{scope_esc}\"}}' placeholder=\"Filter files…\" aria-label=\"Filter diff files\">\
             <button type=\"button\" class=\"button compact\" data-action=\"diff-expand-all\" data-payload='{{\"scope\":\"{scope_esc}\"}}'>Expand all</button>\
             <button type=\"button\" class=\"button compact\" data-action=\"diff-collapse-all\" data-payload='{{\"scope\":\"{scope_esc}\"}}'>Collapse all</button>\
           </span>\
         </div>",
        sections.len(),
        sections.len()
    );

    DIFF_FULL_FILES.with(|map| {
        let mut map = map.borrow_mut();
        let prefix = format!("{scope_id}:");
        map.retain(|k, _| !k.starts_with(&prefix));
        for section in &sections {
            map.insert(
                diff_collapse_key(scope_id, &section.path),
                section.raw.clone(),
            );
        }
    });

    for (idx, section) in sections.iter().enumerate() {
        let key = diff_collapse_key(scope_id, &section.path);
        let (lines_html, total_lines, truncated) =
            render_diff_file_lines_html(&section.raw, Some(DIFF_PREVIEW_MAX_LINES));
        let default_open = should_default_open_diff_file(sections.len(), idx, total_lines);
        let is_open = is_diff_file_open(default_open, &key, &expanded, &collapsed);
        let open_attr = if is_open { " open" } else { "" };
        let path_esc = files_map::escape_html(&section.path);
        let status_esc = files_map::escape_html(&section.status);
        let copy_path_payload = files_map::escape_html(&json!(section.path).to_string());
        let show_more_payload =
            files_map::escape_html(&json!({"scope": scope_id, "file": section.path}).to_string());
        let status_label = match section.status.as_str() {
            "added" => "Added",
            "deleted" => "Deleted",
            "renamed" => "Renamed",
            _ => "Modified",
        };
        let focused = m::read(|s| s.diff_view.focused_file == section.path);
        let focus_attr = if focused {
            " data-focused=\"true\""
        } else {
            ""
        };
        let trunc_bar = if truncated {
            format!(
                "<div class=\"diff-file-trunc\">\
                 <span class=\"muted\">Showing first {DIFF_PREVIEW_MAX_LINES} of {total_lines} lines</span>\
                 <button type=\"button\" class=\"button compact diff-show-more\" data-action=\"diff-show-more\" data-payload=\"{show_more_payload}\">Show all {total_lines} lines</button>\
                 </div>"
            )
        } else {
            String::new()
        };
        out.push_str(&format!(
            "<details class=\"diff-file-group\" data-diff-scope=\"{scope_esc}\" data-diff-file=\"{path_esc}\" data-status=\"{status_esc}\"{focus_attr}{open_attr}>\
             <summary class=\"diff-file-summary\">\
               <span class=\"diff-file-status\" data-status=\"{status_esc}\">{status_label}</span>\
               <span class=\"diff-file-path\">{path_esc}</span>\
               <button type=\"button\" class=\"button compact diff-copy-path\" data-action=\"diff-copy-path\" data-payload=\"{copy_path_payload}\" title=\"Copy file path\">Copy path</button>\
               <span class=\"diff-file-stats\"><span class=\"diff-stat-add\">+{}</span> <span class=\"diff-stat-del\">-{}</span></span>\
             </summary>\
             <pre class=\"diff-file-lines permission-diff\">{lines_html}</pre>\
             {trunc_bar}\
             </details>",
            section.additions,
            section.deletions,
        ));
    }
    out.push_str("</div>");
    out
}

pub(crate) fn filter_diff_files(scope_id: &str, query: &str) {
    let needle = query.trim().to_ascii_lowercase();
    let scope_escaped = super::stream::escape_css_attr_value(scope_id);
    let viewer_sel = format!(".diff-viewer[data-diff-scope=\"{scope_escaped}\"]");
    let Ok(Some(viewer)) = dom::document().query_selector(&viewer_sel) else {
        return;
    };
    let group_sel = format!("details.diff-file-group[data-diff-scope=\"{scope_escaped}\"]");
    let Ok(groups) = viewer.query_selector_all(&group_sel) else {
        return;
    };
    let total = groups.length() as usize;
    let mut visible = 0usize;
    for i in 0..groups.length() {
        let Some(group) = groups
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        let path = group
            .get_attribute("data-diff-file")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let matched = needle.is_empty() || path.contains(&needle);
        dom::hide(&group, !matched);
        if matched {
            visible += 1;
        }
    }
    if let Ok(Some(counts_el)) = viewer.query_selector(".diff-viewer-counts") {
        let adds = viewer.get_attribute("data-total-adds").unwrap_or_default();
        let dels = viewer.get_attribute("data-total-dels").unwrap_or_default();
        let file_word = if total == 1 { "file" } else { "files" };
        let html = if needle.is_empty() {
            format!(
                "{total} {file_word} · <span class=\"diff-stat-add\">+{adds}</span> <span class=\"diff-stat-del\">-{dels}</span>"
            )
        } else {
            format!(
                "{visible} of {total} {file_word} · <span class=\"diff-stat-add\">+{adds}</span> <span class=\"diff-stat-del\">-{dels}</span>"
            )
        };
        counts_el.set_inner_html(&html);
    }
}

pub(crate) fn show_all_diff_lines(scope_id: &str, path: &str) {
    if path.is_empty() {
        return;
    }
    let key = diff_collapse_key(scope_id, path);
    let Some(raw) = DIFF_FULL_FILES.with(|map| map.borrow().get(&key).cloned()) else {
        return;
    };
    let selector = format!(
        "details.diff-file-group[data-diff-scope=\"{}\"][data-diff-file=\"{}\"]",
        super::stream::escape_css_attr_value(scope_id),
        super::stream::escape_css_attr_value(path),
    );
    let Ok(Some(group)) = dom::document().query_selector(&selector) else {
        return;
    };
    let (full_html, _, _) = render_diff_file_lines_html(&raw, None);
    if let Ok(Some(pre)) = group.query_selector("pre.diff-file-lines") {
        pre.set_inner_html(&full_html);
    }
    if let Ok(Some(trunc)) = group.query_selector(".diff-file-trunc") {
        trunc.remove();
    }
}

fn flash_diff_copy_button(button: &web_sys::Element, label: &str, default_label: &'static str) {
    button.set_text_content(Some(label));
    let seq = NEXT_DIFF_COPY_SEQ.with(|c| {
        let n = c.get();
        c.set(n.saturating_add(1));
        n
    });
    let btn = button.clone();
    dom::later(&format!("diff-copy-reset-{seq}"), 1600, move || {
        btn.set_text_content(Some(default_label));
    });
}

pub(crate) async fn copy_diff_path_button(button: &web_sys::Element) {
    let raw_payload = button.get_attribute("data-payload").unwrap_or_default();
    let path = serde_json::from_str::<V>(&raw_payload)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or(raw_payload);
    if path.is_empty() {
        return;
    }
    let copying = dom::window().navigator().clipboard().write_text(&path);
    let copied = wasm_bindgen_futures::JsFuture::from(copying).await.is_ok();
    flash_diff_copy_button(
        button,
        if copied { "Copied" } else { "Copy failed" },
        "Copy path",
    );
}

pub(crate) async fn copy_diff_hunk_button(button: &web_sys::Element) {
    let Ok(Some(bar)) = button.closest(".diff-hunk-bar") else {
        return;
    };
    let mut lines = Vec::<String>::new();
    if let Ok(Some(head)) = bar.query_selector(".diff-hunk-head") {
        if let Some(txt) = head.text_content() {
            lines.push(txt);
        }
    }
    let mut cursor = bar.next_element_sibling();
    while let Some(el) = cursor {
        if el.class_list().contains("diff-hunk-bar") {
            break;
        }
        if let Ok(Some(code)) = el.query_selector(".diff-code") {
            lines.push(code.text_content().unwrap_or_default());
        }
        cursor = el.next_element_sibling();
    }
    let hunk_text = lines.join("\n");
    if hunk_text.is_empty() {
        return;
    }
    let copying = dom::window().navigator().clipboard().write_text(&hunk_text);
    let copied = wasm_bindgen_futures::JsFuture::from(copying).await.is_ok();
    flash_diff_copy_button(
        button,
        if copied { "Copied" } else { "Copy failed" },
        "Copy hunk",
    );
}

pub(crate) fn render_empty_diff_into(container_id: &str, message: &str) {
    let Some(el) = dom::get(container_id) else {
        return;
    };
    if el.get_attribute("data-diff-empty").as_deref() == Some("true")
        && el.text_content().as_deref() == Some(message)
    {
        return;
    }
    el.set_text_content(Some(message));
    let _ = el.remove_attribute("data-diff-hash");
    let _ = el.set_attribute("data-diff-empty", "true");
}

pub(crate) fn render_grouped_diff_into(container_id: &str, diff: &str, scope_id: &str) {
    ensure_diff_toggle_listener();
    let Some(el) = dom::get(container_id) else {
        return;
    };
    if diff.trim().is_empty() {
        el.set_text_content(None);
        let _ = el.remove_attribute("data-diff-hash");
        let _ = el.remove_attribute("data-diff-empty");
        return;
    }
    let hash = diff_content_hash(scope_id, diff);
    let has_viewer = el
        .first_element_child()
        .is_some_and(|c| c.class_list().contains("diff-viewer"));
    if has_viewer && el.get_attribute("data-diff-hash").as_deref() == Some(hash.as_str()) {
        return;
    }
    el.set_inner_html(&render_grouped_diff_html(diff, scope_id));
    let _ = el.set_attribute("data-diff-hash", &hash);
    let _ = el.remove_attribute("data-diff-empty");
}

pub(crate) fn toggle_all_diff_groups(scope_id: &str, expand: bool) {
    let selector = format!(
        "details.diff-file-group[data-diff-scope=\"{}\"]",
        scope_id.replace('"', "")
    );
    let groups = dom::all(&selector);
    for group in &groups {
        let path = group.get_attribute("data-diff-file").unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        set_diff_file_open(scope_id, &path, expand);
        if expand {
            let _ = group.set_attribute("open", "");
        } else {
            let _ = group.remove_attribute("open");
        }
    }
}

pub(crate) fn jump_to_diff_file(scope_id: &str, path: &str) {
    if path.is_empty() {
        return;
    }
    set_diff_file_open(scope_id, path, true);
    m::write(|s| {
        s.diff_view.focused_file = path.to_owned();
    });
    for group in dom::all("details.diff-file-group") {
        let same_scope = scope_id.is_empty()
            || group.get_attribute("data-diff-scope").as_deref() == Some(scope_id);
        let same_path = group.get_attribute("data-diff-file").as_deref() == Some(path);
        if same_scope && same_path {
            let _ = group.set_attribute("open", "");
            let _ = group.set_attribute("data-focused", "true");
            group.scroll_into_view();
            if let Ok(Some(summary)) = group.query_selector("summary") {
                focus_node(&summary);
            }
        } else if same_scope {
            let _ = group.remove_attribute("data-focused");
        }
    }
}
