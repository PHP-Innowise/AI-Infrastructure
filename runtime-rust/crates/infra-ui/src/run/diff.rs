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

pub(crate) fn render_diff_file_lines_range_html(
    lines: &[DiffNumberedLine],
    start: usize,
    count: usize,
) -> String {
    let mut out = String::new();
    for entry in lines.iter().skip(start).take(count) {
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
    let out = render_diff_file_lines_range_html(&lines, 0, limit);
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

fn unescape_git_c_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(after_open) = trimmed.strip_prefix('"') {
        let bytes = after_open.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => break,
                b'\\' if i + 1 < bytes.len() => {
                    i += 1;
                    match bytes[i] {
                        b'\\' => out.push(b'\\'),
                        b'"' => out.push(b'"'),
                        b't' => out.push(b'\t'),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b'a' => out.push(0x07),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'v' => out.push(0x0b),
                        d @ b'0'..=b'7' => {
                            let mut val = (d - b'0') as u16;
                            for _ in 0..2 {
                                if i + 1 < bytes.len() && (b'0'..=b'7').contains(&bytes[i + 1]) {
                                    i += 1;
                                    val = val * 8 + (bytes[i] - b'0') as u16;
                                } else {
                                    break;
                                }
                            }
                            out.push((val & 0xff) as u8);
                        }
                        other => out.push(other),
                    }
                }
                b => out.push(b),
            }
            i += 1;
        }
        return String::from_utf8_lossy(&out).into_owned();
    }
    trimmed
        .split('\t')
        .next()
        .unwrap_or(trimmed)
        .trim()
        .to_owned()
}

fn strip_diff_prefix(raw: &str) -> String {
    let clean_path = unescape_git_c_path(raw);
    if clean_path == "/dev/null" {
        return String::new();
    }
    clean_path
        .strip_prefix("a/")
        .or_else(|| clean_path.strip_prefix("b/"))
        .unwrap_or(&clean_path)
        .to_owned()
}

fn scan_quoted_git_token(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    if bytes.first().copied() != Some(b'"') {
        return None;
    }
    let mut i = 1usize;
    let mut escaped = false;
    while i < bytes.len() {
        if escaped {
            escaped = false;
        } else if bytes[i] == b'\\' {
            escaped = true;
        } else if bytes[i] == b'"' {
            return Some((&s[..=i], s[i + 1..].trim_start()));
        }
        i += 1;
    }
    None
}

fn parse_git_header_paths(header: &str) -> (String, String) {
    let rest = header.trim_start_matches("diff --git ").trim();
    if rest.starts_with('"') {
        if let Some((lhs, after_lhs)) = scan_quoted_git_token(rest) {
            let rhs = if after_lhs.starts_with('"') {
                scan_quoted_git_token(after_lhs).map_or(after_lhs, |(tok, _)| tok)
            } else {
                after_lhs
            };
            return (strip_diff_prefix(lhs), strip_diff_prefix(rhs));
        }
    }
    let mut first_split: Option<(String, String)> = None;
    for (idx, _) in rest.match_indices(" b/") {
        let left = &rest[..idx];
        let right = &rest[idx + 1..];
        let old = strip_diff_prefix(left);
        let new = strip_diff_prefix(right);
        if !old.is_empty() && old == new {
            return (old, new);
        }
        if first_split.is_none() {
            first_split = Some((old, new));
        }
    }
    if let Some((idx, _)) = rest.match_indices(" \"b/").next() {
        let left = &rest[..idx];
        let right = &rest[idx + 1..];
        let old = strip_diff_prefix(left);
        let new = strip_diff_prefix(right);
        if !old.is_empty() && !new.is_empty() {
            return (old, new);
        }
    }
    if let Some(pair) = first_split {
        return pair;
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
        } else if let Some(after) = line.strip_prefix("rename from ") {
            status = "renamed".into();
            let p = unescape_git_c_path(after);
            if !p.is_empty() {
                old_path = p;
            }
        } else if let Some(after) = line.strip_prefix("rename to ") {
            status = "renamed".into();
            let p = unescape_git_c_path(after);
            if !p.is_empty() {
                new_path = p;
            }
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
const DIFF_EXPAND_CHUNK_LINES: usize = 300;
const DIFF_FILE_BATCH_SIZE: usize = 24;
const DIFF_EXPAND_ALL_BATCH_SIZE: usize = 16;
const DIFF_AUTO_OPEN_MAX_FILES: usize = 8;
const DIFF_AUTO_OPEN_MAX_LINES: usize = 400;

struct PendingDiffBatch {
    container_id: String,
    hash: String,
    generation: u64,
    focused_file: String,
    file_count: usize,
    next_idx: usize,
    sections: Vec<DiffFileSection>,
}

struct PendingExpandLines {
    scope_id: String,
    path: String,
    generation: u64,
    lines: Vec<DiffNumberedLine>,
    offset: usize,
}

struct PendingExpandAll {
    generation: u64,
    groups: Vec<web_sys::Element>,
    next_idx: usize,
}

thread_local! {
    static EXPANDED_DIFF_FILES: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    static DIFF_TOGGLE_WIRED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static DIFF_FULL_FILES: std::cell::RefCell<std::collections::BTreeMap<String, String>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    static NEXT_DIFF_COPY_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
    static DIFF_RENDER_GENERATIONS: std::cell::RefCell<std::collections::BTreeMap<String, u64>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    static PENDING_DIFF_BATCHES: std::cell::RefCell<std::collections::BTreeMap<String, PendingDiffBatch>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    static PENDING_DIFF_EXPANDS: std::cell::RefCell<std::collections::BTreeMap<String, PendingExpandLines>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    static PENDING_DIFF_EXPAND_ALL: std::cell::RefCell<std::collections::BTreeMap<String, PendingExpandAll>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

fn next_diff_generation(key: &str) -> u64 {
    DIFF_RENDER_GENERATIONS.with(|gens| {
        let mut gens = gens.borrow_mut();
        let next = gens.get(key).copied().unwrap_or(0).saturating_add(1);
        gens.insert(key.to_owned(), next);
        next
    })
}

fn diff_generation_matches(key: &str, generation: u64) -> bool {
    DIFF_RENDER_GENERATIONS.with(|gens| gens.borrow().get(key).copied() == Some(generation))
}

pub(crate) fn diff_next_chunk_end(offset: usize, total: usize, chunk_size: usize) -> usize {
    (offset + chunk_size.max(1)).min(total)
}

#[cfg(test)]
pub(crate) fn diff_chunk_ranges(total: usize, chunk_size: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    while start < total {
        let end = diff_next_chunk_end(start, total, chunk_size);
        ranges.push((start, end));
        start = end;
    }
    ranges
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

pub(crate) fn set_diff_files_open_batch(scope_id: &str, paths: &[String], open: bool) {
    if paths.is_empty() {
        return;
    }
    let keys: Vec<String> = paths
        .iter()
        .filter(|path| !path.is_empty())
        .map(|path| diff_collapse_key(scope_id, path))
        .collect();
    if keys.is_empty() {
        return;
    }
    EXPANDED_DIFF_FILES.with(|set| {
        let mut set = set.borrow_mut();
        for key in &keys {
            if open {
                set.insert(key.clone());
            } else {
                set.remove(key);
            }
        }
    });
    m::write(|s| {
        for key in keys {
            if open {
                s.diff_view.collapsed_files.remove(&key);
            } else {
                s.diff_view.collapsed_files.insert(key);
            }
        }
    });
}

fn clear_pending_expand_all(scope_filter: Option<&str>) {
    let expand_all_scopes: Vec<String> = PENDING_DIFF_EXPAND_ALL.with(|all| {
        let mut all = all.borrow_mut();
        let keys: Vec<String> = all
            .keys()
            .filter(|scope| scope_filter.is_none_or(|target| *scope == target))
            .cloned()
            .collect();
        for scope in &keys {
            all.remove(scope);
        }
        keys
    });
    if let Some(scope) = scope_filter {
        dom::cancel_frame(&format!("diff-expand-all:{scope}"));
        let _ = next_diff_generation(&format!("expand-all:{scope}"));
    } else {
        for scope in &expand_all_scopes {
            dom::cancel_frame(&format!("diff-expand-all:{scope}"));
            let _ = next_diff_generation(&format!("expand-all:{scope}"));
        }
    }
}

pub(crate) fn clear_pending_diff_work(scope_filter: Option<&str>) {
    clear_pending_expand_all(scope_filter);
    let batch_keys: Vec<String> = PENDING_DIFF_BATCHES.with(|batches| {
        let mut batches = batches.borrow_mut();
        let keys: Vec<String> = batches
            .keys()
            .filter(|scope| scope_filter.is_none_or(|target| *scope == target))
            .cloned()
            .collect();
        for scope in &keys {
            batches.remove(scope);
        }
        keys
    });
    for scope in &batch_keys {
        dom::cancel_frame(&format!("diff-files:{scope}"));
        let _ = next_diff_generation(&format!("files:{scope}"));
    }

    let expand_keys: Vec<String> = PENDING_DIFF_EXPANDS.with(|expands| {
        let mut expands = expands.borrow_mut();
        let keys: Vec<String> = expands
            .iter()
            .filter(|(_, pending)| scope_filter.is_none_or(|target| pending.scope_id == target))
            .map(|(key, _)| key.clone())
            .collect();
        for key in &keys {
            expands.remove(key);
        }
        keys
    });
    for key in &expand_keys {
        dom::cancel_frame(&format!("diff-expand:{key}"));
        let _ = next_diff_generation(&format!("expand:{key}"));
    }
}

#[cfg(test)]
pub(crate) fn seed_pending_expand_all_for_test(scope_id: &str) -> u64 {
    let generation = next_diff_generation(&format!("expand-all:{scope_id}"));
    PENDING_DIFF_EXPAND_ALL.with(|all| {
        all.borrow_mut().insert(
            scope_id.to_owned(),
            PendingExpandAll {
                generation,
                groups: Vec::new(),
                next_idx: 0,
            },
        );
    });
    generation
}

#[cfg(test)]
pub(crate) fn pending_expand_all_still_valid_for_test(scope_id: &str, generation: u64) -> bool {
    PENDING_DIFF_EXPAND_ALL.with(|all| {
        all.borrow()
            .get(scope_id)
            .is_some_and(|pending| pending.generation == generation)
    }) && diff_generation_matches(&format!("expand-all:{scope_id}"), generation)
}

#[cfg(test)]
pub(crate) fn seed_pending_expand_for_test(scope_id: &str, path: &str) -> (String, u64) {
    let key = diff_collapse_key(scope_id, path);
    let generation = next_diff_generation(&format!("expand:{key}"));
    PENDING_DIFF_EXPANDS.with(|expands| {
        expands.borrow_mut().insert(
            key.clone(),
            PendingExpandLines {
                scope_id: scope_id.to_owned(),
                path: path.to_owned(),
                generation,
                lines: Vec::new(),
                offset: 0,
            },
        );
    });
    (key, generation)
}

#[cfg(test)]
pub(crate) fn pending_expand_still_valid_for_test(key: &str, generation: u64) -> bool {
    PENDING_DIFF_EXPANDS.with(|expands| {
        expands
            .borrow()
            .get(key)
            .is_some_and(|pending| pending.generation == generation)
    }) && diff_generation_matches(&format!("expand:{key}"), generation)
}

pub(crate) fn reset_diff_view_state() {
    EXPANDED_DIFF_FILES.with(|set| set.borrow_mut().clear());
    DIFF_FULL_FILES.with(|map| map.borrow_mut().clear());
    clear_pending_diff_work(None);
    DIFF_RENDER_GENERATIONS.with(|gens| gens.borrow_mut().clear());
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
            let open = target.open();
            set_diff_file_open(&scope, &path, open);
            if open {
                hydrate_diff_file_group(target.as_ref());
            }
        },
    );
    let _ = dom::document().add_event_listener_with_callback_and_bool(
        "toggle",
        cb.as_ref().unchecked_ref(),
        true,
    );
    cb.forget();
}

fn append_html_children(target: &web_sys::Element, html: &str) {
    if html.is_empty() {
        return;
    }
    let staging = dom::el("div", "", "");
    staging.set_inner_html(html);
    while let Some(node) = staging.first_child() {
        let _ = target.append_child(&node);
    }
}

fn diff_trunc_bar_html(scope_id: &str, path: &str, total_lines: usize) -> String {
    let show_more_payload =
        files_map::escape_html(&json!({"scope": scope_id, "file": path}).to_string());
    format!(
        "<div class=\"diff-file-trunc\">\
         <span class=\"muted\">Showing first {DIFF_PREVIEW_MAX_LINES} of {total_lines} lines</span>\
         <button type=\"button\" class=\"button compact diff-show-more\" data-action=\"diff-show-more\" data-payload=\"{show_more_payload}\">Show all {total_lines} lines</button>\
         </div>"
    )
}

fn hydrate_diff_file_group(group: &web_sys::Element) {
    let Ok(Some(pre)) = group.query_selector("pre.diff-file-lines[data-diff-deferred=\"true\"]")
    else {
        return;
    };
    let scope = group.get_attribute("data-diff-scope").unwrap_or_default();
    let path = group.get_attribute("data-diff-file").unwrap_or_default();
    if path.is_empty() {
        return;
    }
    let key = diff_collapse_key(&scope, &path);
    let Some(raw) = DIFF_FULL_FILES.with(|map| map.borrow().get(&key).cloned()) else {
        let _ = pre.remove_attribute("data-diff-deferred");
        return;
    };
    let (lines_html, total_lines, truncated) =
        render_diff_file_lines_html(&raw, Some(DIFF_PREVIEW_MAX_LINES));
    let _ = pre.remove_attribute("data-diff-deferred");
    pre.set_inner_html(&lines_html);
    if truncated
        && group
            .query_selector(".diff-file-trunc")
            .ok()
            .flatten()
            .is_none()
    {
        append_html_children(group, &diff_trunc_bar_html(&scope, &path, total_lines));
    }
}

struct DiffGroupContext<'a> {
    scope_id: &'a str,
    scope_esc: &'a str,
    file_count: usize,
    expanded: &'a std::collections::HashSet<String>,
    collapsed: &'a std::collections::BTreeSet<String>,
    focused_file: &'a str,
}

fn render_diff_file_group_html(
    section: &DiffFileSection,
    idx: usize,
    ctx: &DiffGroupContext<'_>,
) -> String {
    let key = diff_collapse_key(ctx.scope_id, &section.path);
    let approx_lines = section.raw.lines().count();
    let default_open = should_default_open_diff_file(ctx.file_count, idx, approx_lines);
    let is_open = is_diff_file_open(default_open, &key, ctx.expanded, ctx.collapsed);
    let open_attr = if is_open { " open" } else { "" };
    let path_esc = files_map::escape_html(&section.path);
    let status_esc = files_map::escape_html(&section.status);
    let copy_path_payload = files_map::escape_html(&json!(section.path).to_string());
    let status_label = match section.status.as_str() {
        "added" => "Added",
        "deleted" => "Deleted",
        "renamed" => "Renamed",
        _ => "Modified",
    };
    let focus_attr = if ctx.focused_file == section.path {
        " data-focused=\"true\""
    } else {
        ""
    };
    let (pre_open_tag, lines_html, trunc_bar) = if is_open {
        let (rendered, total_lines, truncated) =
            render_diff_file_lines_html(&section.raw, Some(DIFF_PREVIEW_MAX_LINES));
        let bar = if truncated {
            diff_trunc_bar_html(ctx.scope_id, &section.path, total_lines)
        } else {
            String::new()
        };
        (
            "<pre class=\"diff-file-lines permission-diff\">",
            rendered,
            bar,
        )
    } else {
        // Defer line formatting and large DOM creation until collapsed file group is opened.
        (
            "<pre class=\"diff-file-lines permission-diff\" data-diff-deferred=\"true\">",
            String::new(),
            String::new(),
        )
    };
    format!(
        "<details class=\"diff-file-group\" data-diff-scope=\"{}\" data-diff-file=\"{path_esc}\" data-status=\"{status_esc}\"{focus_attr}{open_attr}>\
         <summary class=\"diff-file-summary\">\
           <span class=\"diff-file-status\" data-status=\"{status_esc}\">{status_label}</span>\
           <span class=\"diff-file-path\">{path_esc}</span>\
           <button type=\"button\" class=\"button compact diff-copy-path\" data-action=\"diff-copy-path\" data-payload=\"{copy_path_payload}\" title=\"Copy file path\">Copy path</button>\
           <span class=\"diff-file-stats\"><span class=\"diff-stat-add\">+{}</span> <span class=\"diff-stat-del\">-{}</span></span>\
         </summary>\
         {pre_open_tag}{lines_html}</pre>\
         {trunc_bar}\
         </details>",
        ctx.scope_esc,
        section.additions,
        section.deletions,
    )
}

fn diff_viewer_header_html(
    scope_esc: &str,
    file_count: usize,
    total_adds: usize,
    total_dels: usize,
) -> String {
    let file_word = if file_count == 1 { "file" } else { "files" };
    format!(
        "<div class=\"diff-viewer\" data-diff-scope=\"{scope_esc}\" data-total-files=\"{file_count}\" data-total-adds=\"{total_adds}\" data-total-dels=\"{total_dels}\">\
         <div class=\"diff-viewer-summary\">\
           <span class=\"diff-viewer-counts\">{file_count} {file_word} · <span class=\"diff-stat-add\">+{total_adds}</span> <span class=\"diff-stat-del\">-{total_dels}</span></span>\
           <span class=\"diff-viewer-controls\">\
             <input type=\"search\" class=\"diff-file-filter\" data-action=\"diff-filter\" data-payload='{{\"scope\":\"{scope_esc}\"}}' placeholder=\"Filter files…\" aria-label=\"Filter diff files\">\
             <button type=\"button\" class=\"button compact\" data-action=\"diff-expand-all\" data-payload='{{\"scope\":\"{scope_esc}\"}}'>Expand all</button>\
             <button type=\"button\" class=\"button compact\" data-action=\"diff-collapse-all\" data-payload='{{\"scope\":\"{scope_esc}\"}}'>Collapse all</button>\
           </span>\
         </div>"
    )
}

fn cache_diff_full_files(scope_id: &str, sections: &[DiffFileSection]) {
    DIFF_FULL_FILES.with(|map| {
        let mut map = map.borrow_mut();
        let prefix = format!("{scope_id}:");
        map.retain(|k, _| !k.starts_with(&prefix));
        for section in sections {
            map.insert(
                diff_collapse_key(scope_id, &section.path),
                section.raw.clone(),
            );
        }
    });
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
    let focused_file = m::read(|s| s.diff_view.focused_file.clone());
    let total_adds: usize = sections.iter().map(|s| s.additions).sum();
    let total_dels: usize = sections.iter().map(|s| s.deletions).sum();
    let scope_esc = files_map::escape_html(scope_id);

    cache_diff_full_files(scope_id, &sections);

    let ctx = DiffGroupContext {
        scope_id,
        scope_esc: &scope_esc,
        file_count: sections.len(),
        expanded: &expanded,
        collapsed: &collapsed,
        focused_file: &focused_file,
    };
    let mut out = diff_viewer_header_html(&scope_esc, sections.len(), total_adds, total_dels);
    for (idx, section) in sections.iter().enumerate() {
        out.push_str(&render_diff_file_group_html(section, idx, &ctx));
    }
    out.push_str("</div>");
    out
}

pub(crate) fn filter_diff_files(scope_id: &str, query: &str) {
    flush_pending_diff_batches(scope_id);
    let needle = query.trim().to_ascii_lowercase();
    let scope_escaped = crate::dom::escape_css_attr_value(scope_id);
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

fn schedule_expand_diff_chunk(key: String) {
    let frame_key = format!("diff-expand:{key}");
    dom::schedule_frame(&frame_key, move || {
        step_expand_diff_chunk(&key);
    });
}

fn step_expand_diff_chunk(key: &str) {
    enum StepOutcome {
        More {
            scope_id: String,
            path: String,
            chunk_html: String,
            rendered: usize,
            total: usize,
        },
        Done {
            scope_id: String,
            path: String,
            chunk_html: String,
        },
        Drop,
    }
    let gen_key = format!("expand:{key}");
    let outcome = PENDING_DIFF_EXPANDS.with(|expands| {
        let mut expands = expands.borrow_mut();
        let Some(pending) = expands.get_mut(key) else {
            return StepOutcome::Drop;
        };
        if !diff_generation_matches(&gen_key, pending.generation) {
            expands.remove(key);
            return StepOutcome::Drop;
        }
        let end = diff_next_chunk_end(pending.offset, pending.lines.len(), DIFF_EXPAND_CHUNK_LINES);
        let chunk_html = render_diff_file_lines_range_html(
            &pending.lines,
            pending.offset,
            end.saturating_sub(pending.offset),
        );
        pending.offset = end;
        if end >= pending.lines.len() {
            let done = StepOutcome::Done {
                scope_id: pending.scope_id.clone(),
                path: pending.path.clone(),
                chunk_html,
            };
            expands.remove(key);
            done
        } else {
            StepOutcome::More {
                scope_id: pending.scope_id.clone(),
                path: pending.path.clone(),
                chunk_html,
                rendered: end,
                total: pending.lines.len(),
            }
        }
    });
    match outcome {
        StepOutcome::Drop => {}
        StepOutcome::Done {
            scope_id,
            path,
            chunk_html,
        } => {
            let selector = format!(
                "details.diff-file-group[data-diff-scope=\"{}\"][data-diff-file=\"{}\"]",
                crate::dom::escape_css_attr_value(&scope_id),
                crate::dom::escape_css_attr_value(&path),
            );
            if let Ok(Some(group)) = dom::document().query_selector(&selector) {
                if let Ok(Some(pre)) = group.query_selector("pre.diff-file-lines") {
                    append_html_children(&pre, &chunk_html);
                }
                if let Ok(Some(trunc)) = group.query_selector(".diff-file-trunc") {
                    trunc.remove();
                }
            }
        }
        StepOutcome::More {
            scope_id,
            path,
            chunk_html,
            rendered,
            total,
        } => {
            let selector = format!(
                "details.diff-file-group[data-diff-scope=\"{}\"][data-diff-file=\"{}\"]",
                crate::dom::escape_css_attr_value(&scope_id),
                crate::dom::escape_css_attr_value(&path),
            );
            let Ok(Some(group)) = dom::document().query_selector(&selector) else {
                PENDING_DIFF_EXPANDS.with(|expands| {
                    expands.borrow_mut().remove(key);
                });
                return;
            };
            if let Ok(Some(pre)) = group.query_selector("pre.diff-file-lines") {
                append_html_children(&pre, &chunk_html);
            }
            if let Ok(Some(note)) = group.query_selector(".diff-file-trunc span") {
                note.set_text_content(Some(&format!("Rendering {rendered} of {total} lines…")));
            }
            schedule_expand_diff_chunk(key.to_owned());
        }
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
        crate::dom::escape_css_attr_value(scope_id),
        crate::dom::escape_css_attr_value(path),
    );
    let Ok(Some(group)) = dom::document().query_selector(&selector) else {
        return;
    };
    let frame_key = format!("diff-expand:{key}");
    dom::cancel_frame(&frame_key);
    let generation = next_diff_generation(&format!("expand:{key}"));
    let lines = compute_diff_numbered_lines(&raw);
    let total = lines.len();
    if total <= DIFF_EXPAND_CHUNK_LINES {
        PENDING_DIFF_EXPANDS.with(|expands| {
            expands.borrow_mut().remove(&key);
        });
        let full_html = render_diff_file_lines_range_html(&lines, 0, total);
        if let Ok(Some(pre)) = group.query_selector("pre.diff-file-lines") {
            let _ = pre.remove_attribute("data-diff-deferred");
            pre.set_inner_html(&full_html);
        }
        if let Ok(Some(trunc)) = group.query_selector(".diff-file-trunc") {
            trunc.remove();
        }
        return;
    }
    let first_html = render_diff_file_lines_range_html(&lines, 0, DIFF_EXPAND_CHUNK_LINES);
    if let Ok(Some(pre)) = group.query_selector("pre.diff-file-lines") {
        let _ = pre.remove_attribute("data-diff-deferred");
        pre.set_inner_html(&first_html);
    }
    if let Ok(Some(trunc)) = group.query_selector(".diff-file-trunc") {
        trunc.set_text_content(None);
        dom::add(
            &trunc,
            &dom::el(
                "span",
                "muted",
                &format!("Rendering {DIFF_EXPAND_CHUNK_LINES} of {total} lines…"),
            ),
        );
    }
    PENDING_DIFF_EXPANDS.with(|expands| {
        expands.borrow_mut().insert(
            key.clone(),
            PendingExpandLines {
                scope_id: scope_id.to_owned(),
                path: path.to_owned(),
                generation,
                lines,
                offset: DIFF_EXPAND_CHUNK_LINES,
            },
        );
    });
    schedule_expand_diff_chunk(key);
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
    if let Ok(Some(viewer)) = el.query_selector(".diff-viewer[data-diff-scope]") {
        if let Some(scope) = viewer
            .get_attribute("data-diff-scope")
            .filter(|s| !s.is_empty())
        {
            clear_pending_diff_work(Some(&scope));
        }
    }
    if el.get_attribute("data-diff-empty").as_deref() == Some("true")
        && el.text_content().as_deref() == Some(message)
    {
        return;
    }
    el.set_text_content(Some(message));
    let _ = el.remove_attribute("data-diff-hash");
    let _ = el.set_attribute("data-diff-empty", "true");
}

fn schedule_diff_batch(scope_id: String) {
    let frame_key = format!("diff-files:{scope_id}");
    dom::schedule_frame(&frame_key, move || {
        step_diff_batch(&scope_id, false);
    });
}

fn flush_pending_diff_batches(scope_id: &str) {
    if scope_id.is_empty() {
        return;
    }
    dom::cancel_frame(&format!("diff-files:{scope_id}"));
    step_diff_batch(scope_id, true);
}

fn step_diff_batch(scope_id: &str, flush_all: bool) {
    let taken = PENDING_DIFF_BATCHES.with(|batches| {
        let mut batches = batches.borrow_mut();
        let pending = batches.get_mut(scope_id)?;
        if !diff_generation_matches(&format!("files:{scope_id}"), pending.generation) {
            batches.remove(scope_id);
            return None;
        }
        let end = if flush_all {
            pending.sections.len()
        } else {
            diff_next_chunk_end(
                pending.next_idx,
                pending.sections.len(),
                DIFF_FILE_BATCH_SIZE,
            )
        };
        let slice_start = pending.next_idx;
        pending.next_idx = end;
        let container_id = pending.container_id.clone();
        let hash = pending.hash.clone();
        let focused_file = pending.focused_file.clone();
        let file_count = pending.file_count;
        let sections: Vec<DiffFileSection> = pending.sections[slice_start..end].to_vec();
        let more = end < pending.sections.len();
        if !more {
            batches.remove(scope_id);
        }
        Some((
            container_id,
            hash,
            focused_file,
            file_count,
            slice_start,
            sections,
            more,
        ))
    });
    let Some((container_id, hash, focused_file, file_count, slice_start, sections, more)) = taken
    else {
        return;
    };
    let Some(container) = dom::get(&container_id) else {
        PENDING_DIFF_BATCHES.with(|batches| {
            batches.borrow_mut().remove(scope_id);
        });
        return;
    };
    if container.get_attribute("data-diff-hash").as_deref() != Some(hash.as_str()) {
        PENDING_DIFF_BATCHES.with(|batches| {
            batches.borrow_mut().remove(scope_id);
        });
        return;
    }
    let Ok(Some(viewer)) = container.query_selector(".diff-viewer") else {
        PENDING_DIFF_BATCHES.with(|batches| {
            batches.borrow_mut().remove(scope_id);
        });
        return;
    };
    let (expanded, collapsed) = EXPANDED_DIFF_FILES.with(|set| {
        (
            set.borrow().clone(),
            m::read(|s| s.diff_view.collapsed_files.clone()),
        )
    });
    let scope_esc = files_map::escape_html(scope_id);
    let ctx = DiffGroupContext {
        scope_id,
        scope_esc: &scope_esc,
        file_count,
        expanded: &expanded,
        collapsed: &collapsed,
        focused_file: &focused_file,
    };
    let mut batch_html = String::new();
    for (offset, section) in sections.iter().enumerate() {
        batch_html.push_str(&render_diff_file_group_html(
            section,
            slice_start + offset,
            &ctx,
        ));
    }
    append_html_children(&viewer, &batch_html);
    if more {
        schedule_diff_batch(scope_id.to_owned());
    }
}

pub(crate) fn render_grouped_diff_into(container_id: &str, diff: &str, scope_id: &str) {
    ensure_diff_toggle_listener();
    let Some(el) = dom::get(container_id) else {
        return;
    };
    if diff.trim().is_empty() {
        clear_pending_diff_work(Some(scope_id));
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
    clear_pending_diff_work(Some(scope_id));
    let sections = parse_diff_files(diff);
    if sections.len() <= DIFF_FILE_BATCH_SIZE {
        let _ = next_diff_generation(&format!("files:{scope_id}"));
        el.set_inner_html(&render_grouped_diff_html(diff, scope_id));
        let _ = el.set_attribute("data-diff-hash", &hash);
        let _ = el.remove_attribute("data-diff-empty");
        return;
    }
    let generation = next_diff_generation(&format!("files:{scope_id}"));
    let (expanded, collapsed) = EXPANDED_DIFF_FILES.with(|set| {
        (
            set.borrow().clone(),
            m::read(|s| s.diff_view.collapsed_files.clone()),
        )
    });
    let focused_file = m::read(|s| s.diff_view.focused_file.clone());
    let total_adds: usize = sections.iter().map(|s| s.additions).sum();
    let total_dels: usize = sections.iter().map(|s| s.deletions).sum();
    let scope_esc = files_map::escape_html(scope_id);
    cache_diff_full_files(scope_id, &sections);
    let ctx = DiffGroupContext {
        scope_id,
        scope_esc: &scope_esc,
        file_count: sections.len(),
        expanded: &expanded,
        collapsed: &collapsed,
        focused_file: &focused_file,
    };
    let mut initial_html =
        diff_viewer_header_html(&scope_esc, sections.len(), total_adds, total_dels);
    for (idx, section) in sections.iter().take(DIFF_FILE_BATCH_SIZE).enumerate() {
        initial_html.push_str(&render_diff_file_group_html(section, idx, &ctx));
    }
    initial_html.push_str("</div>");
    el.set_inner_html(&initial_html);
    let _ = el.set_attribute("data-diff-hash", &hash);
    let _ = el.remove_attribute("data-diff-empty");
    let file_count = sections.len();
    PENDING_DIFF_BATCHES.with(|batches| {
        batches.borrow_mut().insert(
            scope_id.to_owned(),
            PendingDiffBatch {
                container_id: container_id.to_owned(),
                hash,
                generation,
                focused_file,
                file_count,
                next_idx: DIFF_FILE_BATCH_SIZE,
                sections,
            },
        );
    });
    schedule_diff_batch(scope_id.to_owned());
}

fn apply_expand_all_slice(scope_id: &str, groups: &[web_sys::Element]) {
    let mut opened_paths = Vec::with_capacity(groups.len());
    for group in groups {
        if !group.is_connected() {
            continue;
        }
        let path = group.get_attribute("data-diff-file").unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        opened_paths.push(path);
        let _ = group.set_attribute("open", "");
        hydrate_diff_file_group(group);
    }
    set_diff_files_open_batch(scope_id, &opened_paths, true);
}

fn schedule_expand_all_batch(scope_id: String) {
    let frame_key = format!("diff-expand-all:{scope_id}");
    dom::schedule_frame(&frame_key, move || {
        step_expand_all_batch(&scope_id);
    });
}

fn step_expand_all_batch(scope_id: &str) {
    let taken = PENDING_DIFF_EXPAND_ALL.with(|all| {
        let mut all = all.borrow_mut();
        let pending = all.get_mut(scope_id)?;
        if !diff_generation_matches(&format!("expand-all:{scope_id}"), pending.generation) {
            all.remove(scope_id);
            return None;
        }
        let start = pending.next_idx;
        let end = diff_next_chunk_end(start, pending.groups.len(), DIFF_EXPAND_ALL_BATCH_SIZE);
        pending.next_idx = end;
        let slice = pending.groups[start..end].to_vec();
        let more = end < pending.groups.len();
        if !more {
            all.remove(scope_id);
        }
        Some((slice, more))
    });
    let Some((slice, more)) = taken else {
        return;
    };
    apply_expand_all_slice(scope_id, &slice);
    if more {
        schedule_expand_all_batch(scope_id.to_owned());
    }
}

pub(crate) fn toggle_all_diff_groups(scope_id: &str, expand: bool) {
    clear_pending_expand_all(Some(scope_id));
    flush_pending_diff_batches(scope_id);
    let selector = format!(
        "details.diff-file-group[data-diff-scope=\"{}\"]",
        crate::dom::escape_css_attr_value(scope_id)
    );
    let groups: Vec<web_sys::Element> = dom::all(&selector)
        .into_iter()
        .filter(|g| {
            !g.get_attribute("data-diff-file")
                .unwrap_or_default()
                .is_empty()
        })
        .collect();
    if !expand {
        let paths: Vec<String> = groups
            .iter()
            .map(|g| g.get_attribute("data-diff-file").unwrap_or_default())
            .collect();
        set_diff_files_open_batch(scope_id, &paths, false);
        for group in &groups {
            let _ = group.remove_attribute("open");
        }
        return;
    }
    let first_end = groups.len().min(DIFF_EXPAND_ALL_BATCH_SIZE);
    apply_expand_all_slice(scope_id, &groups[..first_end]);
    if groups.len() > first_end {
        let generation = next_diff_generation(&format!("expand-all:{scope_id}"));
        PENDING_DIFF_EXPAND_ALL.with(|all| {
            all.borrow_mut().insert(
                scope_id.to_owned(),
                PendingExpandAll {
                    generation,
                    groups,
                    next_idx: first_end,
                },
            );
        });
        schedule_expand_all_batch(scope_id.to_owned());
    }
}

pub(crate) fn jump_to_diff_file(scope_id: &str, path: &str) {
    if path.is_empty() {
        return;
    }
    flush_pending_diff_batches(scope_id);
    set_diff_file_open(scope_id, path, true);
    m::write(|s| {
        s.diff_view.focused_file = path.to_owned();
    });
    if !scope_id.is_empty() {
        let scope_escaped = crate::dom::escape_css_attr_value(scope_id);
        let viewer_sel = format!(".diff-viewer[data-diff-scope=\"{scope_escaped}\"]");
        if let Ok(Some(viewer)) = dom::document().query_selector(&viewer_sel) {
            if let Ok(Some(filter_input)) = viewer.query_selector(".diff-file-filter") {
                if !dom::value_of(&filter_input).is_empty() {
                    dom::set_element_value(&filter_input, "");
                    filter_diff_files(scope_id, "");
                }
            }
        }
    }
    for group in dom::all("details.diff-file-group") {
        let same_scope = scope_id.is_empty()
            || group.get_attribute("data-diff-scope").as_deref() == Some(scope_id);
        let same_path = group.get_attribute("data-diff-file").as_deref() == Some(path);
        if same_scope && same_path {
            dom::hide(&group, false);
            let _ = group.set_attribute("open", "");
            hydrate_diff_file_group(&group);
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
