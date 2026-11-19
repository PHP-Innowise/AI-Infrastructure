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

thread_local! {
    static EXPANDED_DIFF_FILES: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    static DIFF_TOGGLE_WIRED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
        "<div class=\"diff-viewer\" data-diff-scope=\"{scope_esc}\">\
         <div class=\"diff-viewer-summary\">\
           <span class=\"diff-viewer-counts\">{} {file_word} · <span class=\"diff-stat-add\">+{total_adds}</span> <span class=\"diff-stat-del\">-{total_dels}</span></span>\
           <span class=\"diff-viewer-controls\">\
             <button type=\"button\" class=\"button compact\" data-action=\"diff-expand-all\" data-payload='{{\"scope\":\"{scope_esc}\"}}'>Expand all</button>\
             <button type=\"button\" class=\"button compact\" data-action=\"diff-collapse-all\" data-payload='{{\"scope\":\"{scope_esc}\"}}'>Collapse all</button>\
           </span>\
         </div>",
        sections.len()
    );

    for (idx, section) in sections.iter().enumerate() {
        let key = diff_collapse_key(scope_id, &section.path);
        let default_open = sections.len() <= 20 || idx == 0;
        let is_open = is_diff_file_open(default_open, &key, &expanded, &collapsed);
        let open_attr = if is_open { " open" } else { "" };
        let path_esc = files_map::escape_html(&section.path);
        let status_esc = files_map::escape_html(&section.status);
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
        out.push_str(&format!(
            "<details class=\"diff-file-group\" data-diff-scope=\"{scope_esc}\" data-diff-file=\"{path_esc}\" data-status=\"{status_esc}\"{focus_attr}{open_attr}>\
             <summary class=\"diff-file-summary\">\
               <span class=\"diff-file-status\" data-status=\"{status_esc}\">{status_label}</span>\
               <span class=\"diff-file-path\">{path_esc}</span>\
               <span class=\"diff-file-stats\"><span class=\"diff-stat-add\">+{}</span> <span class=\"diff-stat-del\">-{}</span></span>\
             </summary>\
             <pre class=\"diff-file-lines permission-diff\">{}</pre>\
             </details>",
            section.additions,
            section.deletions,
            render_diff_preview_lines_html(&section.raw),
        ));
    }
    out.push_str("</div>");
    out
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
