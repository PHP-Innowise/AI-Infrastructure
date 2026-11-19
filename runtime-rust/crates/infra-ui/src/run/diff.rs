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

fn diff_collapse_key(scope_id: &str, path: &str) -> String {
    format!("{scope_id}:{path}")
}

/// Renders a file-grouped collapsible diff viewer with jump anchors and zero inline hex colors.
pub(crate) fn render_grouped_diff_html(diff: &str, scope_id: &str) -> String {
    let sections = parse_diff_files(diff);
    if sections.is_empty() {
        return "<p class=\"muted\">No diff output recorded.</p>".into();
    }
    let collapsed = m::read(|s| s.diff_view.collapsed_files.clone());
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
        let is_open = default_open && !collapsed.contains(&key);
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

pub(crate) fn render_grouped_diff_into(container_id: &str, diff: &str, scope_id: &str) {
    let Some(el) = dom::get(container_id) else {
        return;
    };
    if diff.trim().is_empty() {
        el.set_text_content(None);
        return;
    }
    el.set_inner_html(&render_grouped_diff_html(diff, scope_id));
}

pub(crate) fn toggle_all_diff_groups(scope_id: &str, expand: bool) {
    let selector = format!(
        "details.diff-file-group[data-diff-scope=\"{}\"]",
        scope_id.replace('"', "")
    );
    let groups = dom::all(&selector);
    m::write(|s| {
        for group in &groups {
            let path = group.get_attribute("data-diff-file").unwrap_or_default();
            if path.is_empty() {
                continue;
            }
            let key = diff_collapse_key(scope_id, &path);
            if expand {
                s.diff_view.collapsed_files.remove(&key);
                let _ = group.set_attribute("open", "");
            } else {
                s.diff_view.collapsed_files.insert(key);
                let _ = group.remove_attribute("open");
            }
        }
    });
}

pub(crate) fn jump_to_diff_file(scope_id: &str, path: &str) {
    if path.is_empty() {
        return;
    }
    let key = diff_collapse_key(scope_id, path);
    m::write(|s| {
        s.diff_view.collapsed_files.remove(&key);
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
