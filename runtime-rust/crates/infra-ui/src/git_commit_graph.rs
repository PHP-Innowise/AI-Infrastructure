#![allow(dead_code)]

use serde_json::Value;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

thread_local! {
    static COMMIT_LAYOUT_CACHE: RefCell<Option<(u64, Rc<Vec<CommitRowLayout>>)>> =
        const { RefCell::new(None) };
}

fn commit_layouts_cache_key(commits: &[Value], selected_commit: Option<&str>) -> u64 {
    let mut h = DefaultHasher::new();
    commits.len().hash(&mut h);
    selected_commit.hash(&mut h);
    for commit in commits {
        commit["commit"].as_str().hash(&mut h);
        commit["short"].as_str().hash(&mut h);
        commit["target"].as_str().hash(&mut h);
        commit["merge"].as_bool().hash(&mut h);
        if let Some(parents) = commit["parents"].as_array() {
            parents.len().hash(&mut h);
            for p in parents {
                p.as_str().hash(&mut h);
            }
        }
        if let Some(refs) = commit["refs"].as_array() {
            refs.len().hash(&mut h);
            for r in refs {
                r["name"].as_str().hash(&mut h);
                r["kind"].as_str().hash(&mut h);
            }
        }
    }
    h.finish()
}

fn build_layouts_cached(
    commits: &[Value],
    selected_commit: Option<&str>,
) -> Rc<Vec<CommitRowLayout>> {
    let key = commit_layouts_cache_key(commits, selected_commit);
    if let Some(hit) = COMMIT_LAYOUT_CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .and_then(|(cached_key, layouts)| (*cached_key == key).then(|| Rc::clone(layouts)))
    }) {
        return hit;
    }
    let built = Rc::new(build_layouts(commits, selected_commit));
    COMMIT_LAYOUT_CACHE.with(|c| {
        *c.borrow_mut() = Some((key, Rc::clone(&built)));
    });
    built
}

pub const ROW_HEIGHT: usize = 36;
const MID_Y: usize = ROW_HEIGHT / 2;
const TOP_CTRL_Y: usize = MID_Y / 2;
const BOT_CTRL_Y: usize = MID_Y + (ROW_HEIGHT - MID_Y) / 2;
const PAD_X: usize = 12;
const LANE_WIDTH: usize = 14;
const LANE_COLORS: usize = 8;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Surface where Git commit DAG rows are rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitGraphSurface {
    /// Used inside `#graph-pick` ("Commits" tab):
    /// row id="graph-pick-row-{index}", class="graph-pick-row has-git-graph", role="option",
    /// aria-selected="{is_active}", data-action="graph-pick-choose", data-payload=<json string "commit:{sha}">,
    /// Load more button: <li class="graph-pick-more" role="presentation"><button type="button" class="button compact" data-action="graph-pick-more">Show older commits</button></li>
    Picker,
    /// Used inside docked `#graph-commit-graph-list`:
    /// row id="graph-commit-row-{index}", class="git-graph-row", role="option",
    /// aria-selected="{is_selected_commit}", data-current="{is_selected_commit}",
    /// data-action="graph-commit-select", data-payload=<json string "commit:{sha}">,
    /// Load more button: <li class="graph-pick-more" role="presentation"><button type="button" class="button compact" data-action="graph-commit-graph-more">Show older commits</button></li>
    Dock,
}

/// Computed DAG rail geometry and inline SVG for a single commit row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitRowLayout {
    pub lane: usize,
    pub color: usize,
    pub rail_width: usize,
    pub rail_svg: String,
}

#[derive(Clone, Debug)]
struct LaneSlot {
    sha: String,
    color: usize,
}

/// Computes deterministic DAG lane assignments and per-row inline rail SVGs for a slice of commits ordered newest-first.
pub fn compute_commit_rail_layouts(commits: &[Value]) -> Vec<CommitRowLayout> {
    build_layouts(commits, None)
}

/// Finds the index of `selected_commit` inside `commits`, if loaded.
pub fn selected_commit_row_index(
    commits: &[Value],
    selected_commit: Option<&str>,
) -> Option<usize> {
    commits.iter().position(|commit| {
        let sha = commit_sha(commit);
        let short = commit_short(commit, &sha);
        is_commit_selected(&sha, &short, selected_commit)
    })
}

/// Computes the next keyboard-active Commit Dock row index for `ArrowDown`, `ArrowUp`, `Home`, and `End`.
pub fn next_commit_dock_index(
    current: Option<usize>,
    selected: Option<usize>,
    total: usize,
    key: &str,
) -> Option<usize> {
    if total == 0 {
        return None;
    }
    let anchor = current.or(selected).map(|idx| idx.min(total - 1));
    match key {
        "ArrowDown" => Some(anchor.map_or(0, |idx| (idx + 1).min(total - 1))),
        "ArrowUp" => Some(anchor.map_or(0, |idx| idx.saturating_sub(1))),
        "Home" => Some(0),
        "End" => Some(total - 1),
        _ => None,
    }
}

/// Resolves ordered `(older_base, newer_head)` commit range when Shift+clicking a second commit.
pub fn resolve_commit_range_selection(
    commits: &[Value],
    anchor_sha: Option<&str>,
    target_sha: &str,
) -> Option<(String, String)> {
    let anchor_norm = normalize_commit_sha(anchor_sha.unwrap_or(""));
    let target_norm = normalize_commit_sha(target_sha);
    if target_norm.is_empty() {
        return None;
    }
    if anchor_norm.is_empty() || anchor_norm == target_norm {
        return None;
    }
    let anchor_idx = selected_commit_row_index(commits, Some(&anchor_norm));
    let target_idx = selected_commit_row_index(commits, Some(&target_norm));
    match (anchor_idx, target_idx) {
        (Some(ai), Some(ti)) => {
            // Commits are newest-first: larger row index is older (`base`), smaller is newer (`head`).
            if ai < ti {
                Some((target_norm, anchor_norm))
            } else {
                Some((anchor_norm, target_norm))
            }
        }
        _ => Some((anchor_norm, target_norm)),
    }
}

fn normalize_commit_sha(raw: &str) -> String {
    raw.trim()
        .strip_prefix("commit:")
        .unwrap_or(raw.trim())
        .to_owned()
}

pub fn commit_range_state_for_index(
    index: usize,
    base_idx: Option<usize>,
    head_idx: Option<usize>,
) -> Option<&'static str> {
    match (base_idx, head_idx) {
        (Some(b), Some(h)) => {
            let (lo, hi) = (b.min(h), b.max(h));
            if index == h {
                Some("end")
            } else if index == b {
                Some("start")
            } else if index > lo && index < hi {
                Some("between")
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Renders compact range comparison banner bar when two commits `A..B` are compared.
pub fn render_commit_range_banner(base_sha: &str, head_sha: &str) -> String {
    let base_short = short_sha(base_sha);
    let head_short = short_sha(head_sha);
    if base_short.is_empty() || head_short.is_empty() {
        return String::new();
    }
    format!(
        "<div class=\"graph-commit-range-bar\"><span>Range diff: <code>{}</code> .. <code>{}</code></span><span class=\"graph-commit-range-hint\">(Shift+click any commit to set range)</span><button type=\"button\" class=\"button compact\" data-action=\"graph-commit-range-clear\">Clear range</button></div>",
        escape_html(&base_short),
        escape_html(&head_short),
    )
}

/// Renders HTML `<li>` commit rows with inline DAG rail SVGs for either the modal picker or the docked commit graph panel.
pub fn render_commit_graph_rows(
    commits: &[Value],
    active_index: Option<usize>,
    selected_commit: Option<&str>,
    more: bool,
    now: i64,
    surface: CommitGraphSurface,
) -> String {
    render_commit_graph_rows_with_options(
        commits,
        active_index,
        selected_commit,
        None,
        None,
        more,
        now,
        surface,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn render_commit_graph_rows_with_range(
    commits: &[Value],
    active_index: Option<usize>,
    selected_commit: Option<&str>,
    range: Option<(&str, &str)>,
    window: Option<(f64, f64)>,
    more: bool,
    now: i64,
    surface: CommitGraphSurface,
) -> String {
    render_commit_graph_rows_with_options(
        commits,
        active_index,
        selected_commit,
        range,
        window,
        more,
        now,
        surface,
    )
}

/// Computes the virtualized `[start_idx, end_idx)` slice of commit rows when `total > 80`.
pub fn commit_dock_window_range(total: usize, scroll_top: f64, viewport_h: f64) -> (usize, usize) {
    if total <= 80 || !(viewport_h.is_finite() && viewport_h > 0.0) {
        return (0, total);
    }
    let row_h = ROW_HEIGHT as f64;
    let safe_top = if scroll_top.is_finite() && scroll_top > 0.0 {
        scroll_top
    } else {
        0.0
    };
    let first = ((safe_top / row_h).floor() as isize - 18).max(0) as usize;
    let visible = (viewport_h / row_h).ceil() as usize + 36;
    let last = (first + visible).min(total);
    (first.min(total), last)
}

/// Computes the target `scroll_top` required so virtualized row `index` sits inside
/// `[scroll_top, scroll_top + viewport_h)`. Returns `None` if already within view.
pub fn scroll_top_for_commit_row(index: usize, scroll_top: f64, viewport_h: f64) -> Option<f64> {
    if !(viewport_h.is_finite() && viewport_h > 0.0) {
        return None;
    }
    let row_top = (index * ROW_HEIGHT) as f64;
    let row_bot = ((index + 1) * ROW_HEIGHT) as f64;
    let cur = if scroll_top.is_finite() && scroll_top >= 0.0 {
        scroll_top
    } else {
        0.0
    };
    if row_top < cur {
        Some(row_top)
    } else if row_bot > cur + viewport_h {
        Some((row_bot - viewport_h).max(0.0))
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn render_commit_graph_rows_with_options(
    commits: &[Value],
    active_index: Option<usize>,
    selected_commit: Option<&str>,
    range: Option<(&str, &str)>,
    window: Option<(f64, f64)>,
    more: bool,
    now: i64,
    surface: CommitGraphSurface,
) -> String {
    let layouts = build_layouts_cached(commits, selected_commit);
    let base_idx = range.and_then(|(b, _)| selected_commit_row_index(commits, Some(b)));
    let head_idx = range.and_then(|(_, h)| selected_commit_row_index(commits, Some(h)));
    let total = commits.len();

    let (start_idx, end_idx) = match window {
        Some((scroll_top, viewport_h)) => commit_dock_window_range(total, scroll_top, viewport_h),
        _ => (0, total),
    };

    let mut html = String::new();
    if start_idx > 0 {
        let top_h = start_idx * ROW_HEIGHT;
        html.push_str(&format!(
            "<li class=\"git-graph-spacer\" style=\"height:{top_h}px\" role=\"presentation\" aria-hidden=\"true\"></li>"
        ));
    }

    for (index, (commit, layout)) in commits
        .iter()
        .zip(layouts.iter())
        .enumerate()
        .skip(start_idx)
        .take(end_idx.saturating_sub(start_idx))
    {
        let sha = commit_sha(commit);
        let short = commit_short(commit, &sha);
        let subject = commit["subject"].as_str().unwrap_or("");
        let author = commit["author"].as_str().unwrap_or("");
        let when = commit["at"].as_i64().map(|at| ago(now, at));
        let is_active = active_index == Some(index);
        let is_selected = is_commit_selected(&sha, &short, selected_commit);
        let payload = commit_target_payload(commit, &sha);
        let badges = render_commit_badges(commit);
        let range_attr = commit_range_state_for_index(index, base_idx, head_idx)
            .map(|r| format!(" data-range=\"{r}\""))
            .unwrap_or_default();

        match surface {
            CommitGraphSurface::Picker => {
                let title_html = if subject.is_empty() {
                    String::new()
                } else {
                    format!(
                        "<span class=\"graph-pick-title\">{}</span>",
                        escape_html(subject)
                    )
                };
                let badges_html = if badges.is_empty() {
                    String::new()
                } else {
                    format!("<span class=\"graph-pick-badges\">{badges}</span>")
                };
                let who_html = if author.is_empty() {
                    String::new()
                } else {
                    format!(
                        "<span class=\"graph-pick-who\">{}</span>",
                        escape_html(author)
                    )
                };
                let when_html = match &when {
                    Some(label) => {
                        format!(
                            "<span class=\"graph-pick-when\">{}</span>",
                            escape_html(label)
                        )
                    }
                    None => String::new(),
                };
                let current_attr = if is_selected {
                    " data-current=\"true\""
                } else {
                    ""
                };
                html.push_str(&format!(
                    "<li class=\"graph-pick-row has-git-graph\" id=\"graph-pick-row-{index}\" role=\"option\" aria-selected=\"{is_active}\"{current_attr}{range_attr} data-action=\"graph-pick-choose\" data-payload=\"{payload}\"><span class=\"git-graph-rail\">{}</span><span class=\"graph-pick-main\">{title_html}{badges_html}</span>{who_html}{when_html}<code class=\"graph-pick-code\">{}</code></li>",
                    layout.rail_svg,
                    escape_html(&short),
                ));
            }
            CommitGraphSurface::Dock => {
                // A count of memory to review is drawn only where one was
                // supplied: rows are never asked for one each.
                let badges = if let Some(count) =
                    commit["memory_review"].as_u64().filter(|count| *count > 0)
                {
                    format!(
                        "{badges}<span class=\"graph-pick-badge\" data-kind=\"memory\" title=\"Memory citing a file this commit renamed\">{count} to review</span>"
                    )
                } else if commit["memory_review_error"] == true {
                    format!(
                        "{badges}<span class=\"graph-pick-badge\" data-kind=\"memory-error\" title=\"Could not check memory citing files this commit renamed\">check failed</span>"
                    )
                } else {
                    badges
                };
                let badges_html = if badges.is_empty() {
                    String::new()
                } else {
                    format!("<span class=\"git-graph-badges\">{badges}</span>")
                };
                let who_html = if author.is_empty() {
                    String::new()
                } else {
                    format!(
                        "<span class=\"git-graph-who\">{}</span>",
                        escape_html(author)
                    )
                };
                let when_html = match &when {
                    Some(label) => {
                        format!(
                            "<span class=\"git-graph-when\">{}</span>",
                            escape_html(label)
                        )
                    }
                    None => String::new(),
                };
                let is_row_selected = active_index.map_or(is_selected, |idx| idx == index);
                html.push_str(&format!(
                    "<li class=\"git-graph-row\" id=\"graph-commit-row-{index}\" role=\"option\" aria-selected=\"{is_row_selected}\" data-current=\"{is_selected}\"{range_attr} data-action=\"graph-commit-select\" data-payload=\"{payload}\"><span class=\"git-graph-rail\">{}</span><div class=\"git-graph-body\"><div class=\"git-graph-top\"><span class=\"git-graph-subject\">{}</span>{badges_html}</div><div class=\"git-graph-meta\"><code class=\"git-graph-hash\">{}</code>{who_html}{when_html}</div></div></li>",
                    layout.rail_svg,
                    escape_html(subject),
                    escape_html(&short),
                ));
            }
        }
    }

    if end_idx < total {
        let bot_h = (total - end_idx) * ROW_HEIGHT;
        html.push_str(&format!(
            "<li class=\"git-graph-spacer\" style=\"height:{bot_h}px\" role=\"presentation\" aria-hidden=\"true\"></li>"
        ));
    }

    if more {
        let action = match surface {
            CommitGraphSurface::Picker => "graph-pick-more",
            CommitGraphSurface::Dock => "graph-commit-graph-more",
        };
        html.push_str(&format!(
            "<li class=\"graph-pick-more\" role=\"presentation\"><button type=\"button\" class=\"button compact\" data-action=\"{action}\">Show older commits</button></li>"
        ));
    }

    html
}

/// Renders the active commit summary banner with change counts, parent jump buttons, and a Whole project reset action.
pub fn render_commit_banner(
    commit_info: Option<&Value>,
    chosen: &Value,
    changes_catalog: Option<&Value>,
) -> String {
    let is_commit_chosen = chosen["kind"].as_str() == Some("commit")
        || chosen["target"]
            .as_str()
            .is_some_and(|t| t.starts_with("commit:"));
    if commit_info.is_none() && !is_commit_chosen {
        return String::new();
    }

    let info = commit_info.unwrap_or(&Value::Null);
    let sha = info["commit"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            chosen["target"]
                .as_str()
                .and_then(|t| t.strip_prefix("commit:"))
                .filter(|s| !s.is_empty())
        })
        .or_else(|| chosen["head"]["commit"].as_str().filter(|s| !s.is_empty()))
        .or_else(|| chosen["name"].as_str())
        .unwrap_or("");
    let short = info["short"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            chosen["head"]["commit"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| short_sha(sha));
    let subject = info["subject"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| chosen["title"].as_str())
        .unwrap_or("");
    let author = info["author"].as_str().unwrap_or("");
    let badges = render_refs_badges(info["refs"].as_array());

    let mut parents: Vec<String> = info["parents"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    // When commit_info is omitted, fall back to chosen.base.commit for single-parent navigation.
    if commit_info.is_none() && parents.is_empty() {
        if let Some(base_sha) = chosen["base"]["commit"].as_str().filter(|s| !s.is_empty()) {
            parents.push(base_sha.to_owned());
        }
    }

    let mut html = String::from("<div class=\"graph-commit-banner-main\">");
    if !short.is_empty() {
        html.push_str(&format!(
            "<code class=\"graph-commit-banner-sha\">{}</code>",
            escape_html(&short)
        ));
    }
    if !subject.is_empty() {
        html.push_str(&format!(
            "<span class=\"graph-commit-banner-subject\">{}</span>",
            escape_html(subject)
        ));
    }
    if !badges.is_empty() {
        html.push_str(&format!(
            "<span class=\"graph-commit-banner-badges\">{badges}</span>"
        ));
    }
    if !author.is_empty() {
        html.push_str(&format!(
            "<span class=\"graph-commit-banner-author\">{}</span>",
            escape_html(author)
        ));
    }
    html.push_str("</div>");

    if let Some(catalog) = changes_catalog.filter(|v| v.is_object()) {
        let stats_html = render_catalog_stats(catalog);
        if !stats_html.is_empty() {
            html.push_str(&format!(
                "<div class=\"graph-commit-banner-stats\">{stats_html}</div>"
            ));
        }
    }

    html.push_str("<div class=\"graph-commit-banner-actions\">");
    for (idx, parent_sha) in parents.iter().enumerate() {
        let p_short = short_sha(parent_sha);
        let label = if parents.len() > 1 {
            format!("Parent {} {p_short}", idx + 1)
        } else {
            format!("Parent {p_short}")
        };
        let payload = json_string_attr(&format!("commit:{parent_sha}"));
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact graph-commit-parent-btn\" data-action=\"graph-commit-select\" data-payload=\"{payload}\">{}</button>",
            escape_html(&label)
        ));
    }
    html.push_str("<button type=\"button\" class=\"button compact graph-commit-reset-btn\" data-action=\"graph-commit-reset\">Whole project</button>");
    html.push_str("</div>");

    html
}

/// The renamed files of a commit's changes catalog as `(path, previous_path)`,
/// at most `limit`.
pub fn renamed_files(catalog: Option<&Value>, limit: usize) -> Vec<(String, String)> {
    catalog
        .and_then(|catalog| catalog["files"].as_array())
        .into_iter()
        .flatten()
        .filter(|file| file["status"].as_str() == Some("renamed"))
        .filter_map(|file| {
            let path = file["path"].as_str()?.trim();
            let previous = file["previous_path"].as_str()?.trim();
            (!path.is_empty() && !previous.is_empty() && path != previous)
                .then(|| (path.to_owned(), previous.to_owned()))
        })
        .take(limit)
        .collect()
}

/// How many warnings to review the memory-links answers for a commit's
/// renamed files raised.
pub fn rename_review_count(answers: &[Value]) -> u64 {
    answers
        .iter()
        .flat_map(|answer| answer["hygiene_warnings"].as_array().into_iter().flatten())
        .filter(|warning| warning["recommended_action"].as_str() == Some("review"))
        .count() as u64
}

/// The banner's word on the memory that cites a commit's renamed files under
/// their old names: `answers[i]` is the memory-links answer for `renamed[i]`
/// (`{"pending": true}` while it is asked). Warnings are for review only;
/// nothing is retired or changed. `unchecked` renamed files were not asked.
pub fn render_rename_memory_warnings(
    renamed: &[(String, String)],
    answers: &[Value],
    unchecked: usize,
) -> String {
    if renamed.is_empty() {
        return String::new();
    }
    let mut html = String::new();
    let mut pending = 0usize;
    for ((path, previous), answer) in renamed.iter().zip(answers) {
        if !answer.is_object() || answer["pending"] == true {
            pending += 1;
            continue;
        }
        if let Some(error) = answer["error"].as_str() {
            html.push_str(&format!(
                "<p class=\"knowledge-note\">Could not check the memory citing {}: {}</p>",
                escape_html(previous),
                escape_html(error)
            ));
            continue;
        }
        for warning in answer["hygiene_warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|warning| warning["recommended_action"].as_str() == Some("review"))
        {
            let linked = warning["linked"].as_array().map_or(0, Vec::len);
            html.push_str(&format!(
                "<p class=\"graph-memory-warning\" data-kind=\"{}\" role=\"note\">Review: <code>{}</code> → <code>{}</code>: {} ({linked} cited)</p>",
                escape_html(warning["kind"].as_str().unwrap_or("")),
                escape_html(previous),
                escape_html(path),
                escape_html(warning["text"].as_str().unwrap_or(""))
            ));
        }
    }
    if pending > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">Checking the memory that cites {} renamed {}…</p>",
            pending,
            if pending == 1 { "file" } else { "files" }
        ));
    }
    if unchecked > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">{unchecked} more renamed {} not checked for memory citing the old name.</p>",
            if unchecked == 1 { "file was" } else { "files were" }
        ));
    }
    html
}

fn build_layouts(commits: &[Value], selected_commit: Option<&str>) -> Vec<CommitRowLayout> {
    if commits.is_empty() {
        return Vec::new();
    }

    let any_has_parents = commits
        .iter()
        .any(|c| c.get("parents").and_then(Value::as_array).is_some());

    let row_shas: Vec<String> = commits.iter().map(commit_sha).collect();
    let mut sha_last_row: HashMap<&str, usize> = HashMap::with_capacity(row_shas.len());
    for (idx, sha) in row_shas.iter().enumerate() {
        if !sha.is_empty() {
            sha_last_row.insert(sha.as_str(), idx);
        }
    }

    struct PendingRow {
        lane: usize,
        color: usize,
        rail_width: usize,
        body_svg: String,
    }

    let mut lanes: Vec<Option<LaneSlot>> = Vec::new();
    let mut next_color: usize = 0;
    let mut pending = Vec::with_capacity(commits.len());

    for (row_idx, commit) in commits.iter().enumerate() {
        let sha = &row_shas[row_idx];
        let short = commit_short(commit, sha);
        let parents = extract_row_parents(commits, row_idx, sha, any_has_parents);
        let is_merge = commit["merge"].as_bool().unwrap_or(false) || parents.len() > 1;
        let is_head = commit["refs"]
            .as_array()
            .is_some_and(|refs| refs.iter().any(|r| r["kind"].as_str() == Some("head")));
        let is_selected = is_commit_selected(sha, &short, selected_commit);

        let lanes_in = lanes;
        let matching_cols: Vec<usize> = if sha.is_empty() {
            Vec::new()
        } else {
            lanes_in
                .iter()
                .enumerate()
                .filter_map(|(col, slot)| slot.as_ref().filter(|s| s.sha == *sha).map(|_| col))
                .collect()
        };

        let (c_lane, c_color) = if let Some(&first_col) = matching_cols.first() {
            let color = lanes_in[first_col]
                .as_ref()
                .map_or(0, |slot| slot.color % LANE_COLORS);
            (first_col, color)
        } else {
            let free_col = lanes_in
                .iter()
                .position(Option::is_none)
                .unwrap_or(lanes_in.len());
            let color = alloc_color(&mut next_color, None);
            (free_col, color)
        };

        let mut lanes_out = lanes_in.clone();
        for &col in &matching_cols {
            lanes_out[col] = None;
        }
        if c_lane >= lanes_out.len() {
            lanes_out.resize(c_lane + 1, None);
        }

        let mut pass_through_sources: Vec<(usize, usize, usize)> = Vec::new();
        let mut incoming_svg = String::new();
        let mut outgoing_svg = String::new();

        // 1. Collect pass-through lanes and draw incoming rails from y=0.
        for (col, slot_opt) in lanes_in.iter().enumerate() {
            let Some(slot) = slot_opt else {
                continue;
            };
            let color = slot.color % LANE_COLORS;
            let x = lane_x(col);
            if !sha.is_empty() && slot.sha == *sha {
                if col == c_lane {
                    incoming_svg.push_str(&straight_path(x, 0, MID_Y, color, "in"));
                } else {
                    let cx = lane_x(c_lane);
                    incoming_svg.push_str(&bezier_path(
                        x,
                        0,
                        TOP_CTRL_Y,
                        cx,
                        MID_Y,
                        color,
                        "in-branch",
                    ));
                }
            } else {
                pass_through_sources.push((col, col, color));
            }
        }

        // 2. Outgoing rails to parents or off-page continuation stubs.
        let mut max_visual_col = lanes_in.len().max(c_lane + 1);
        let mut drawn_targets: BTreeSet<String> = BTreeSet::new();

        for (p_idx, parent_sha) in parents.iter().enumerate() {
            if !drawn_targets.insert(parent_sha.clone()) {
                continue;
            }
            let target_below = sha_last_row
                .get(parent_sha.as_str())
                .is_some_and(|&idx| idx > row_idx);

            if target_below {
                if let Some(existing_col) = lanes_out
                    .iter()
                    .position(|s| s.as_ref().is_some_and(|slot| slot.sha == *parent_sha))
                {
                    if p_idx == 0 && existing_col > c_lane && lanes_out[c_lane].is_none() {
                        // Pull the shared parent back left into c_lane so primary trunks stay on lower lanes.
                        lanes_out[existing_col] = None;
                        lanes_out[c_lane] = Some(LaneSlot {
                            sha: parent_sha.clone(),
                            color: c_color,
                        });
                        for src in &mut pass_through_sources {
                            if src.0 == existing_col {
                                src.1 = c_lane;
                            }
                        }
                        outgoing_svg.push_str(&straight_path(
                            lane_x(c_lane),
                            MID_Y,
                            ROW_HEIGHT,
                            c_color,
                            "out",
                        ));
                        max_visual_col = max_visual_col.max(existing_col + 1);
                    } else {
                        let existing_color = lanes_out[existing_col]
                            .as_ref()
                            .map_or(c_color, |s| s.color % LANE_COLORS);
                        let stroke_color = if p_idx == 0 { c_color } else { existing_color };
                        let kind = if p_idx > 0 || parents.len() > 1 {
                            "merge"
                        } else {
                            "fork"
                        };
                        if existing_col == c_lane {
                            outgoing_svg.push_str(&straight_path(
                                lane_x(c_lane),
                                MID_Y,
                                ROW_HEIGHT,
                                stroke_color,
                                kind,
                            ));
                        } else {
                            outgoing_svg.push_str(&bezier_path(
                                lane_x(c_lane),
                                MID_Y,
                                BOT_CTRL_Y,
                                lane_x(existing_col),
                                ROW_HEIGHT,
                                stroke_color,
                                kind,
                            ));
                        }
                        max_visual_col = max_visual_col.max(existing_col + 1);
                    }
                } else if p_idx == 0 && lanes_out[c_lane].is_none() {
                    lanes_out[c_lane] = Some(LaneSlot {
                        sha: parent_sha.clone(),
                        color: c_color,
                    });
                    outgoing_svg.push_str(&straight_path(
                        lane_x(c_lane),
                        MID_Y,
                        ROW_HEIGHT,
                        c_color,
                        "out",
                    ));
                    max_visual_col = max_visual_col.max(c_lane + 1);
                } else {
                    let target_col = lanes_out
                        .iter()
                        .position(Option::is_none)
                        .unwrap_or(lanes_out.len());
                    if target_col >= lanes_out.len() {
                        lanes_out.resize(target_col + 1, None);
                    }
                    let branch_color = alloc_color(&mut next_color, Some(c_color));
                    lanes_out[target_col] = Some(LaneSlot {
                        sha: parent_sha.clone(),
                        color: branch_color,
                    });
                    if target_col == c_lane {
                        outgoing_svg.push_str(&straight_path(
                            lane_x(c_lane),
                            MID_Y,
                            ROW_HEIGHT,
                            branch_color,
                            "merge",
                        ));
                    } else {
                        outgoing_svg.push_str(&bezier_path(
                            lane_x(c_lane),
                            MID_Y,
                            BOT_CTRL_Y,
                            lane_x(target_col),
                            ROW_HEIGHT,
                            branch_color,
                            "merge",
                        ));
                    }
                    max_visual_col = max_visual_col.max(target_col + 1);
                }
            } else {
                // Off-page parent (not yet loaded in `commits`, or at the end of the slice):
                // emit a downward continuation stub without reserving a permanent lane.
                if p_idx == 0 {
                    outgoing_svg.push_str(&stub_straight_path(lane_x(c_lane), c_color));
                    max_visual_col = max_visual_col.max(c_lane + 1);
                } else {
                    let stub_col = lanes_out
                        .iter()
                        .position(Option::is_none)
                        .filter(|&col| col != c_lane)
                        .unwrap_or(lanes_out.len().max(c_lane + 1));
                    let stub_color = alloc_color(&mut next_color, Some(c_color));
                    outgoing_svg.push_str(&stub_bezier_path(
                        lane_x(c_lane),
                        lane_x(stub_col),
                        stub_color,
                    ));
                    max_visual_col = max_visual_col.max(stub_col + 1);
                }
            }
        }

        let mut pass_through_svg = String::new();
        for (from_col, to_col, color) in pass_through_sources {
            let x_from = lane_x(from_col);
            if from_col == to_col {
                pass_through_svg.push_str(&straight_path(x_from, 0, ROW_HEIGHT, color, "pass"));
            } else {
                pass_through_svg.push_str(&bezier_path(
                    x_from,
                    0,
                    BOT_CTRL_Y,
                    lane_x(to_col),
                    ROW_HEIGHT,
                    color,
                    "collapse",
                ));
            }
        }

        max_visual_col = max_visual_col.max(lanes_out.len()).max(1);
        let rail_width = PAD_X * 2 + (max_visual_col - 1) * LANE_WIDTH;
        let cx = lane_x(c_lane);

        let mut body_svg = String::new();
        body_svg.push_str(&pass_through_svg);
        body_svg.push_str(&incoming_svg);
        body_svg.push_str(&outgoing_svg);

        if is_selected || is_head {
            let ring_stroke = if is_selected {
                "var(--purple, var(--git-lane-0))".to_owned()
            } else {
                format!("var(--git-lane-{c_color})")
            };
            body_svg.push_str(&format!(
                "<circle class=\"git-graph-ring\" cx=\"{cx}\" cy=\"{MID_Y}\" r=\"7.5\" fill=\"none\" stroke=\"{ring_stroke}\" stroke-width=\"1.5\" data-selected=\"{is_selected}\" data-head=\"{is_head}\"/>"
            ));
        }

        if is_merge {
            body_svg.push_str(&format!(
                "<circle class=\"git-graph-dot git-graph-dot-merge\" cx=\"{cx}\" cy=\"{MID_Y}\" r=\"5.5\" fill=\"var(--paper, #ffffff)\" stroke=\"var(--git-lane-{c_color})\" stroke-width=\"2.2\" data-merge=\"true\"/>"
            ));
            body_svg.push_str(&format!(
                "<circle class=\"git-graph-dot-inner\" cx=\"{cx}\" cy=\"{MID_Y}\" r=\"2.2\" fill=\"var(--git-lane-{c_color})\"/>"
            ));
        } else {
            body_svg.push_str(&format!(
                "<circle class=\"git-graph-dot\" cx=\"{cx}\" cy=\"{MID_Y}\" r=\"4.2\" fill=\"var(--git-lane-{c_color})\" stroke=\"var(--paper, #ffffff)\" stroke-width=\"1.5\"/>"
            ));
        }

        pending.push(PendingRow {
            lane: c_lane,
            color: c_color,
            rail_width,
            body_svg,
        });

        while lanes_out.last().is_some_and(Option::is_none) {
            lanes_out.pop();
        }
        lanes = lanes_out;
    }

    let max_rail_width = pending
        .iter()
        .map(|row| row.rail_width)
        .max()
        .unwrap_or(PAD_X * 2);

    pending
        .into_iter()
        .map(|row| CommitRowLayout {
            lane: row.lane,
            color: row.color,
            rail_width: row.rail_width,
            rail_svg: format!(
                "<svg class=\"git-graph-svg\" width=\"{max_rail_width}\" height=\"{ROW_HEIGHT}\" viewBox=\"0 0 {max_rail_width} {ROW_HEIGHT}\" preserveAspectRatio=\"xMidYMid meet\" aria-hidden=\"true\">{}</svg>",
                row.body_svg
            ),
        })
        .collect()
}

fn extract_row_parents(
    commits: &[Value],
    row_idx: usize,
    self_sha: &str,
    any_has_parents: bool,
) -> Vec<String> {
    if any_has_parents {
        commits[row_idx]
            .get("parents")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
                    .filter(|s| *s != self_sha)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    } else if row_idx + 1 < commits.len() {
        let next_sha = commit_sha(&commits[row_idx + 1]);
        if next_sha.is_empty() || next_sha == self_sha {
            Vec::new()
        } else {
            vec![next_sha]
        }
    } else {
        Vec::new()
    }
}

fn lane_x(lane: usize) -> usize {
    PAD_X + lane * LANE_WIDTH
}

fn alloc_color(next_color: &mut usize, avoid: Option<usize>) -> usize {
    let mut color = *next_color % LANE_COLORS;
    if let Some(av) = avoid {
        if color == av % LANE_COLORS {
            *next_color += 1;
            color = *next_color % LANE_COLORS;
        }
    }
    *next_color += 1;
    color
}

fn straight_path(x: usize, y1: usize, y2: usize, color: usize, kind: &str) -> String {
    format!(
        "<path class=\"git-graph-rail\" d=\"M {x} {y1} L {x} {y2}\" stroke=\"var(--git-lane-{color})\" stroke-width=\"2\" fill=\"none\" data-kind=\"{kind}\"/>"
    )
}

fn bezier_path(
    x1: usize,
    y1: usize,
    ctrl_y: usize,
    x2: usize,
    y2: usize,
    color: usize,
    kind: &str,
) -> String {
    format!(
        "<path class=\"git-graph-rail\" d=\"M {x1} {y1} C {x1} {ctrl_y}, {x2} {ctrl_y}, {x2} {y2}\" stroke=\"var(--git-lane-{color})\" stroke-width=\"2\" fill=\"none\" data-kind=\"{kind}\"/>"
    )
}

fn stub_straight_path(x: usize, color: usize) -> String {
    format!(
        "<path class=\"git-graph-stub\" d=\"M {x} {MID_Y} L {x} 34\" stroke=\"var(--git-lane-{color})\" stroke-width=\"2\" stroke-dasharray=\"3 3\" stroke-linecap=\"round\" fill=\"none\" data-kind=\"stub\"/>"
    )
}

fn stub_bezier_path(x1: usize, x2: usize, color: usize) -> String {
    format!(
        "<path class=\"git-graph-stub\" d=\"M {x1} {MID_Y} C {x1} 26, {x2} 26, {x2} 33\" stroke=\"var(--git-lane-{color})\" stroke-width=\"2\" stroke-dasharray=\"3 3\" stroke-linecap=\"round\" fill=\"none\" data-kind=\"stub\"/>"
    )
}

fn commit_sha(commit: &Value) -> String {
    commit["commit"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            commit["target"]
                .as_str()
                .and_then(|t| t.strip_prefix("commit:"))
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            commit["short"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("")
        .to_owned()
}

fn commit_short(commit: &Value, sha: &str) -> String {
    commit["short"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| short_sha(sha))
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn commit_target_payload(commit: &Value, sha: &str) -> String {
    let target = commit["target"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("commit:{sha}"));
    json_string_attr(&target)
}

fn json_string_attr(raw: &str) -> String {
    let encoded = serde_json::to_string(raw).unwrap_or_else(|_| "\"\"".to_owned());
    escape_html(&encoded)
}

fn is_commit_selected(sha: &str, short: &str, selected_commit: Option<&str>) -> bool {
    let Some(sel_raw) = selected_commit else {
        return false;
    };
    let sel = sel_raw
        .trim()
        .strip_prefix("commit:")
        .unwrap_or(sel_raw.trim())
        .trim();
    if sel.is_empty() {
        return false;
    }
    if !sha.is_empty() {
        if sha.eq_ignore_ascii_case(sel) {
            return true;
        }
        if sel.len() >= 4 && sha.len() >= sel.len() && sha[..sel.len()].eq_ignore_ascii_case(sel) {
            return true;
        }
        if sha.len() >= 4 && sel.len() >= sha.len() && sel[..sha.len()].eq_ignore_ascii_case(sha) {
            return true;
        }
    }
    !short.is_empty() && short.eq_ignore_ascii_case(sel)
}

fn render_commit_badges(commit: &Value) -> String {
    let mut badges = render_refs_badges(commit["refs"].as_array());
    if commit["merge"].as_bool() == Some(true) {
        badges.push_str("<span class=\"graph-pick-badge\" data-kind=\"merge\">merge</span>");
    }
    badges
}

fn render_refs_badges(refs: Option<&Vec<Value>>) -> String {
    let Some(refs) = refs else {
        return String::new();
    };
    let mut out = String::new();
    for entry in refs {
        let name = entry["name"].as_str().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let kind = entry["kind"].as_str().unwrap_or("branch");
        out.push_str(&format!(
            "<span class=\"graph-pick-badge\" data-kind=\"{}\">{}</span>",
            escape_html(kind),
            escape_html(name)
        ));
    }
    out
}

fn render_catalog_stats(catalog: &Value) -> String {
    let files = catalog["files"].as_array();
    let count_status = |target: &str| -> u64 {
        files
            .map(|arr| {
                arr.iter()
                    .filter(|f| f["status"].as_str() == Some(target))
                    .count() as u64
            })
            .unwrap_or(0)
    };
    let created = catalog["created_files"]
        .as_u64()
        .unwrap_or_else(|| count_status("created"));
    let modified = catalog["modified_files"]
        .as_u64()
        .unwrap_or_else(|| count_status("modified"));
    let deleted = catalog["deleted_files"]
        .as_u64()
        .unwrap_or_else(|| count_status("deleted"));
    let renamed = catalog["renamed_files"]
        .as_u64()
        .unwrap_or_else(|| count_status("renamed"));

    let mut out = String::new();
    if created > 0 {
        out.push_str(&format!(
            "<span class=\"graph-commit-stat\" data-status=\"created\">+{created} created</span>"
        ));
    }
    if modified > 0 {
        out.push_str(&format!(
            "<span class=\"graph-commit-stat\" data-status=\"modified\">~{modified} modified</span>"
        ));
    }
    if deleted > 0 {
        out.push_str(&format!(
            "<span class=\"graph-commit-stat\" data-status=\"deleted\">-{deleted} deleted</span>"
        ));
    }
    if renamed > 0 {
        out.push_str(&format!(
            "<span class=\"graph-commit-stat\" data-status=\"renamed\">→{renamed} renamed</span>"
        ));
    }
    if out.is_empty() {
        out.push_str(
            "<span class=\"graph-commit-stat\" data-status=\"unchanged\">0 file changes</span>",
        );
    }
    out
}

fn escape_html(text: &str) -> String {
    crate::dom::escape_html(text)
}

fn day(at: i64) -> String {
    // Days since 1970 to a civil date (Hinnant's algorithm).
    let days = at.div_euclid(86_400) + 719_468;
    let (era, of_era) = (days.div_euclid(146_097), days.rem_euclid(146_097));
    let year = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year + year / 4 - year / 100);
    let shifted = (5 * of_year + 2) / 153;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    format!(
        "{} {} {}",
        of_year - (153 * shifted + 2) / 5 + 1,
        MONTHS[(month - 1) as usize],
        year + era * 400 + i64::from(month <= 2)
    )
}

fn ago(now: i64, at: i64) -> String {
    let passed = now - at;
    let span =
        |count: i64, unit: &str| format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" });
    match passed {
        ..60 => "just now".to_owned(),
        60..3_600 => span(passed / 60, "minute"),
        3_600..86_400 => span(passed / 3_600, "hour"),
        86_400..172_800 => "yesterday".to_owned(),
        172_800..2_592_000 => span(passed / 86_400, "day"),
        _ => day(at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn linear_history_uses_lane_zero_and_connects_top_to_bottom() {
        let commits = vec![
            json!({
                "commit": "3333333333333333",
                "short": "3333333",
                "subject": "Latest",
                "author": "Ann",
                "at": 1_794_396_400,
                "refs": [{"name": "main", "kind": "head"}],
                "merge": false,
                "parents": ["2222222222222222"]
            }),
            json!({
                "commit": "2222222222222222",
                "short": "2222222",
                "subject": "Middle",
                "author": "Bo",
                "at": 1_794_390_000,
                "refs": [],
                "merge": false,
                "parents": ["1111111111111111"]
            }),
            json!({
                "commit": "1111111111111111",
                "short": "1111111",
                "subject": "Root",
                "author": "Cy",
                "at": 1_794_380_000,
                "refs": [],
                "merge": false,
                "parents": []
            }),
        ];

        let layouts = compute_commit_rail_layouts(&commits);
        assert_eq!(layouts.len(), 3);
        for layout in &layouts {
            assert_eq!(layout.lane, 0);
            assert_eq!(layout.color, 0);
            assert_eq!(layout.rail_width, 24);
            assert!(layout.rail_svg.contains("class=\"git-graph-svg\""));
            assert!(layout.rail_svg.contains("viewBox=\"0 0 24 36\""));
        }
        assert!(layouts[0].rail_svg.contains("data-head=\"true\""));
        assert!(layouts[0].rail_svg.contains("M 12 18 L 12 36"));
        assert!(layouts[1].rail_svg.contains("M 12 0 L 12 18"));
        assert!(layouts[1].rail_svg.contains("M 12 18 L 12 36"));
        assert!(layouts[2].rail_svg.contains("M 12 0 L 12 18"));
        assert!(!layouts[2].rail_svg.contains("M 12 18 L 12 36"));
        assert!(!layouts[2].rail_svg.contains("git-graph-stub"));
    }

    #[test]
    fn missing_parents_field_falls_back_gracefully_to_linear() {
        let commits = vec![
            json!({"commit": "1a2b3c4d5e6f", "short": "1a2b3c4", "subject": "One", "author": "Ann", "at": 1_794_396_400, "refs": [], "merge": false}),
            json!({"commit": "9f8e7d6c5b4a", "short": "9f8e7d6", "subject": "Two", "author": "Bo", "at": 1_794_313_600, "refs": [], "merge": true}),
        ];
        let layouts = compute_commit_rail_layouts(&commits);
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].lane, 0);
        assert_eq!(layouts[1].lane, 0);
        assert!(layouts[0].rail_svg.contains("M 12 18 L 12 36"));
        assert!(layouts[1].rail_svg.contains("M 12 0 L 12 18"));
        assert!(layouts[1].rail_svg.contains("data-merge=\"true\""));
    }

    #[test]
    fn branch_and_merge_dag_routes_bezier_lanes_and_double_ring() {
        let commits = vec![
            json!({
                "commit": "c5",
                "short": "c5",
                "subject": "Merge feat into main",
                "author": "Ann",
                "refs": [{"name": "main", "kind": "head"}],
                "merge": true,
                "parents": ["c4", "c3"]
            }),
            json!({
                "commit": "c4",
                "short": "c4",
                "subject": "Main work",
                "author": "Ann",
                "refs": [],
                "merge": false,
                "parents": ["c2"]
            }),
            json!({
                "commit": "c3",
                "short": "c3",
                "subject": "Feat work",
                "author": "Bo",
                "refs": [{"name": "feat", "kind": "branch"}],
                "merge": false,
                "parents": ["c2"]
            }),
            json!({
                "commit": "c2",
                "short": "c2",
                "subject": "Base",
                "author": "Ann",
                "refs": [],
                "merge": false,
                "parents": ["c1"]
            }),
            json!({
                "commit": "c1",
                "short": "c1",
                "subject": "Init",
                "author": "Ann",
                "refs": [],
                "merge": false,
                "parents": []
            }),
        ];

        let layouts = compute_commit_rail_layouts(&commits);
        assert_eq!(layouts.len(), 5);
        // Merge commit on lane 0 forks a merge Bezier to lane 1.
        assert_eq!(layouts[0].lane, 0);
        assert_eq!(layouts[0].color, 0);
        assert_eq!(layouts[0].rail_width, 38);
        assert!(layouts[0].rail_svg.contains("git-graph-dot-merge"));
        assert!(layouts[0].rail_svg.contains("git-graph-dot-inner"));
        assert!(layouts[0]
            .rail_svg
            .contains("d=\"M 12 18 C 12 27, 26 27, 26 36\""));

        // c4 continues lane 0 while c3 passes through on lane 1.
        assert_eq!(layouts[1].lane, 0);
        assert!(layouts[1].rail_svg.contains("d=\"M 26 0 L 26 36\""));

        // c3 is on lane 1 and curves back into c2 on lane 0.
        assert_eq!(layouts[2].lane, 1);
        assert_ne!(layouts[2].color, layouts[0].color);
        assert!(layouts[2]
            .rail_svg
            .contains("d=\"M 26 18 C 26 27, 12 27, 12 36\""));

        // After lane 1 collapses into lane 0, c2 and c1 drop back to 1-lane width.
        assert_eq!(layouts[3].lane, 0);
        assert_eq!(layouts[3].rail_width, 24);
        assert_eq!(layouts[4].lane, 0);
        assert_eq!(layouts[4].rail_width, 24);
    }

    #[test]
    fn multi_lane_independent_branch_tips_and_reused_slots() {
        // Two parallel heads (main and topic) plus a feature branch merged into main.
        let commits = vec![
            json!({"commit": "m2", "parents": ["m1", "f1"], "merge": true}),
            json!({"commit": "t1", "parents": ["m1"], "merge": false}),
            json!({"commit": "f1", "parents": ["m0"], "merge": false}),
            json!({"commit": "m1", "parents": ["m0"], "merge": false}),
            json!({"commit": "m0", "parents": [], "merge": false}),
        ];

        let layouts = compute_commit_rail_layouts(&commits);
        assert_eq!(layouts[0].lane, 0); // m2
        assert_eq!(layouts[1].lane, 2); // t1 opens on lane 2 (lanes 0=m1, 1=f1 occupied)
        assert_eq!(layouts[2].lane, 1); // f1 on lane 1, curves down to m0 (new lane 2 after t1 merged into lane 0)
        assert_eq!(layouts[3].lane, 0); // m1 on lane 0
        assert_eq!(layouts[4].lane, 0); // m0 receives both branches cleanly
        assert!(layouts[1].rail_width >= 52);
        for layout in &layouts {
            assert!(layout.color < 8);
        }
    }

    #[test]
    fn off_page_parents_draw_continuation_stubs() {
        let commits = vec![
            json!({
                "commit": "top1",
                "short": "top1",
                "subject": "Merge older branch",
                "merge": true,
                "parents": ["mid1", "off_branch"]
            }),
            json!({
                "commit": "mid1",
                "short": "mid1",
                "subject": "Bottom of slice",
                "merge": false,
                "parents": ["off_ancestor"]
            }),
        ];

        let layouts = compute_commit_rail_layouts(&commits);
        assert_eq!(layouts.len(), 2);
        // top1 draws straight line to mid1 + dashed stub for off_branch without locking lane 1 on mid1.
        assert!(layouts[0].rail_svg.contains("class=\"git-graph-stub\""));
        assert!(layouts[0].rail_width >= 38);
        // mid1 draws downward continuation stub for off_ancestor and stays 1 lane wide.
        assert_eq!(layouts[1].lane, 0);
        assert_eq!(layouts[1].rail_width, 24);
        assert!(layouts[1].rail_svg.contains("class=\"git-graph-stub\""));
        assert!(layouts[1].rail_svg.contains("d=\"M 12 18 L 12 34\""));
    }

    #[test]
    fn picker_surface_renders_expected_dom_and_escapes_html() {
        let commits = vec![
            json!({
                "commit": "1a2b3c4d5e6f",
                "short": "1a2b3c4",
                "subject": "Round <totals> once",
                "author": "Ann & Bo",
                "at": 1_794_396_400,
                "refs": [
                    {"name": "main", "kind": "head"},
                    {"name": "origin/main", "kind": "remote"},
                    {"name": "v2", "kind": "tag"}
                ],
                "merge": false,
                "parents": ["9f8e7d6c5b4a"]
            }),
            json!({
                "commit": "9f8e7d6c5b4a",
                "short": "9f8e7d6",
                "subject": "Merge the cart",
                "author": "Bo",
                "at": 1_794_313_600,
                "refs": [],
                "merge": true,
                "parents": ["000011112222"]
            }),
        ];

        let html = render_commit_graph_rows(
            &commits,
            Some(1),
            Some("1a2b3c4d5e6f"),
            true,
            1_794_400_000,
            CommitGraphSurface::Picker,
        );

        assert!(html.contains("id=\"graph-pick-row-0\""));
        assert!(html.contains("class=\"graph-pick-row has-git-graph\""));
        assert!(html.contains("aria-selected=\"false\""));
        assert!(html.contains("data-action=\"graph-pick-choose\""));
        assert!(html.contains("data-payload=\"&quot;commit:1a2b3c4d5e6f&quot;\""));
        assert!(html.contains("<span class=\"git-graph-rail\"><svg class=\"git-graph-svg\""));
        assert!(html.contains("data-selected=\"true\""));
        assert!(html.contains("<span class=\"graph-pick-title\">Round &lt;totals&gt; once</span>"));
        assert!(html.contains("<span class=\"graph-pick-badge\" data-kind=\"head\">main</span>"));
        assert!(html.contains("<span class=\"graph-pick-who\">Ann &amp; Bo</span>"));
        assert!(html.contains("<span class=\"graph-pick-when\">1 hour ago</span>"));
        assert!(html.contains("<code class=\"graph-pick-code\">1a2b3c4</code>"));

        assert!(html.contains("id=\"graph-pick-row-1\" role=\"option\" aria-selected=\"true\""));
        assert!(html.contains("<span class=\"graph-pick-badge\" data-kind=\"merge\">merge</span>"));
        assert!(html.contains("<span class=\"graph-pick-when\">yesterday</span>"));
        assert!(html.ends_with(
            "<li class=\"graph-pick-more\" role=\"presentation\"><button type=\"button\" class=\"button compact\" data-action=\"graph-pick-more\">Show older commits</button></li>"
        ));
    }

    #[test]
    fn dock_surface_renders_compact_rows_selection_and_more_button() {
        let commits = vec![
            json!({
                "commit": "abcdef1234567890",
                "short": "abcdef1",
                "subject": "Add DAG renderer",
                "author": "Ada",
                "at": 1_794_399_970,
                "refs": [{"name": "feat/dag", "kind": "branch"}],
                "merge": false,
                "parents": ["1234567890abcdef"]
            }),
            json!({
                "commit": "1234567890abcdef",
                "short": "1234567",
                "subject": "Initial",
                "author": "Ada",
                "at": 1_790_000_000,
                "refs": [],
                "merge": false,
                "parents": []
            }),
        ];

        let html = render_commit_graph_rows(
            &commits,
            None,
            Some("commit:abcdef1234567890"),
            true,
            1_794_400_000,
            CommitGraphSurface::Dock,
        );

        assert!(html.contains("<li class=\"git-graph-row\" id=\"graph-commit-row-0\" role=\"option\" aria-selected=\"true\" data-current=\"true\" data-action=\"graph-commit-select\" data-payload=\"&quot;commit:abcdef1234567890&quot;\">"));
        assert!(html.contains("<div class=\"git-graph-body\"><div class=\"git-graph-top\"><span class=\"git-graph-subject\">Add DAG renderer</span><span class=\"git-graph-badges\"><span class=\"graph-pick-badge\" data-kind=\"branch\">feat/dag</span></span></div><div class=\"git-graph-meta\"><code class=\"git-graph-hash\">abcdef1</code><span class=\"git-graph-who\">Ada</span><span class=\"git-graph-when\">just now</span></div></div>"));
        assert!(html.contains("<li class=\"git-graph-row\" id=\"graph-commit-row-1\" role=\"option\" aria-selected=\"false\" data-current=\"false\" data-action=\"graph-commit-select\""));
        assert!(html.contains("data-selected=\"true\""));
        assert!(html
            .ends_with("data-action=\"graph-commit-graph-more\">Show older commits</button></li>"));

        let empty_no_more =
            render_commit_graph_rows(&[], None, None, false, 0, CommitGraphSurface::Dock);
        assert_eq!(empty_no_more, "");
    }

    #[test]
    fn commit_banner_renders_info_counts_parent_jumps_and_reset() {
        let commit_info = json!({
            "commit": "abc1234deadbeef",
            "short": "abc1234",
            "subject": "Merge <feature> branch",
            "author": "Linus",
            "at": 1_794_396_400,
            "parents": ["1111111aaaa", "2222222bbbb"],
            "refs": [{"name": "main", "kind": "head"}, {"name": "v1.0", "kind": "tag"}]
        });
        let chosen = json!({
            "kind": "commit",
            "target": "commit:abc1234deadbeef",
            "title": "Merge <feature> branch",
            "head": {"commit": "abc1234"}
        });
        let catalog = json!({
            "created_files": 2,
            "modified_files": 3,
            "deleted_files": 1,
            "renamed_files": 4,
            "files": []
        });

        let html = render_commit_banner(Some(&commit_info), &chosen, Some(&catalog));
        assert!(html.contains("<code class=\"graph-commit-banner-sha\">abc1234</code>"));
        assert!(html.contains(
            "<span class=\"graph-commit-banner-subject\">Merge &lt;feature&gt; branch</span>"
        ));
        assert!(html.contains("<span class=\"graph-commit-banner-author\">Linus</span>"));
        assert!(html.contains("<span class=\"graph-pick-badge\" data-kind=\"head\">main</span>"));
        assert!(html.contains("<span class=\"graph-pick-badge\" data-kind=\"tag\">v1.0</span>"));
        assert!(html.contains(
            "<span class=\"graph-commit-stat\" data-status=\"created\">+2 created</span>"
        ));
        assert!(html.contains(
            "<span class=\"graph-commit-stat\" data-status=\"modified\">~3 modified</span>"
        ));
        assert!(html.contains(
            "<span class=\"graph-commit-stat\" data-status=\"deleted\">-1 deleted</span>"
        ));
        assert!(html.contains(
            "<span class=\"graph-commit-stat\" data-status=\"renamed\">→4 renamed</span>"
        ));
        assert!(html.contains("data-action=\"graph-commit-select\" data-payload=\"&quot;commit:1111111aaaa&quot;\">Parent 1 1111111</button>"));
        assert!(html.contains("data-action=\"graph-commit-select\" data-payload=\"&quot;commit:2222222bbbb&quot;\">Parent 2 2222222</button>"));
        assert!(html.contains("<button type=\"button\" class=\"button compact graph-commit-reset-btn\" data-action=\"graph-commit-reset\">Whole project</button>"));

        // Non-commit chosen with no commit_info returns empty string.
        assert_eq!(
            render_commit_banner(None, &json!({"kind": "work"}), None),
            ""
        );

        // Commit chosen without commit_info falls back to chosen.base.commit.
        let fallback = render_commit_banner(
            None,
            &json!({
                "kind": "commit",
                "target": "commit:deadbeef0000",
                "title": "Fix bug",
                "head": {"commit": "deadbeef"},
                "base": {"commit": "cafebabe"}
            }),
            None,
        );
        assert!(fallback.contains("deadbeef"));
        assert!(fallback.contains("Fix bug"));
        assert!(fallback
            .contains("data-payload=\"&quot;commit:cafebabe&quot;\">Parent cafebab</button>"));
    }

    #[test]
    fn dock_rows_show_a_memory_count_only_where_one_is_supplied() {
        let row = |extra: Value| {
            let mut commit = json!({
                "commit": "abcdef1234567890",
                "short": "abcdef1",
                "subject": "Rename the guard",
                "refs": [],
                "parents": []
            });
            if let (Some(base), Some(more)) = (commit.as_object_mut(), extra.as_object()) {
                base.extend(more.clone());
            }
            render_commit_graph_rows(&[commit], None, None, false, 0, CommitGraphSurface::Dock)
        };
        let counted = row(json!({"memory_review": 2}));
        assert!(
            counted.contains("<span class=\"git-graph-badges\"><span class=\"graph-pick-badge\" data-kind=\"memory\" title=\"Memory citing a file this commit renamed\">2 to review</span></span>"),
            "{counted}"
        );
        // No count, a zero count, or a count that is not a number: no badge.
        for extra in [
            json!({}),
            json!({"memory_review": 0}),
            json!({"memory_review": "2"}),
        ] {
            let html = row(extra.clone());
            assert!(!html.contains("data-kind=\"memory\""), "{extra}: {html}");
            assert!(!html.contains("git-graph-badges"), "{extra}: {html}");
        }
        // The picker never shows it.
        let picker = render_commit_graph_rows(
            &[json!({"commit": "abcdef1234567890", "memory_review": 3})],
            None,
            None,
            false,
            0,
            CommitGraphSurface::Picker,
        );
        assert!(!picker.contains("to review"), "{picker}");

        let errored = row(json!({"memory_review_error": true}));
        assert!(errored.contains("data-kind=\"memory-error\""), "{errored}");
        assert!(errored.contains(">check failed</span>"), "{errored}");
    }

    #[test]
    fn renamed_files_feed_review_only_memory_warnings() {
        let catalog = json!({
            "renamed_files": 3,
            "files": [
                {"path": "src/Auth/Guard.php", "previous_path": "src/Auth/TokenGuard.php", "status": "renamed"},
                {"path": "src/new.rs", "previous_path": "src/new.rs", "status": "renamed"},
                {"path": "src/kept.rs", "status": "modified"},
                {"path": "src/b.rs", "previous_path": "src/a.rs", "status": "renamed"},
                {"path": "src/d.rs", "previous_path": "src/c.rs", "status": "renamed"}
            ]
        });
        let renamed = renamed_files(Some(&catalog), 2);
        assert_eq!(
            renamed,
            vec![
                (
                    "src/Auth/Guard.php".to_owned(),
                    "src/Auth/TokenGuard.php".to_owned()
                ),
                ("src/b.rs".to_owned(), "src/a.rs".to_owned()),
            ]
        );
        assert!(renamed_files(None, 8).is_empty());

        // Pending answers say so; nothing else is drawn for them.
        let pending = render_rename_memory_warnings(
            &renamed,
            &[json!({"pending": true}), json!({"pending": true})],
            0,
        );
        assert_eq!(
            pending,
            "<p class=\"knowledge-note\">Checking the memory that cites 2 renamed files…</p>"
        );
        let answers = [
            json!({"hygiene_warnings": [{"kind": "cited-file-renamed", "linked": ["MEM-20261117-aabbccdd", "finding-<7>"], "recommended_action": "review", "text": "This file was src/Auth/TokenGuard.php; the memory that cites the old name may need its citation updated."}]}),
            json!({"error": "Project not found"}),
        ];
        let html = render_rename_memory_warnings(&renamed, &answers, 1);
        assert!(
            html.contains("<p class=\"graph-memory-warning\" data-kind=\"cited-file-renamed\" role=\"note\">Review: <code>src/Auth/TokenGuard.php</code> → <code>src/Auth/Guard.php</code>: This file was src/Auth/TokenGuard.php; the memory that cites the old name may need its citation updated. (2 cited)</p>"),
            "{html}"
        );
        assert!(
            html.contains("Could not check the memory citing src/a.rs: Project not found"),
            "{html}"
        );
        assert!(
            html.ends_with("<p class=\"knowledge-note\">1 more renamed file was not checked for memory citing the old name.</p>"),
            "{html}"
        );
        // Nothing here acts on the memory: no buttons, no actions.
        assert!(!html.contains("data-action"), "{html}");
        assert_eq!(rename_review_count(&answers), 1);
        // Answers with nothing to review draw nothing.
        assert_eq!(
            render_rename_memory_warnings(
                &renamed,
                &[json!({"hygiene_warnings": []}), json!({})],
                0
            ),
            ""
        );
        assert_eq!(render_rename_memory_warnings(&[], &[], 4), "");
    }

    #[test]
    fn rail_svgs_pad_uniformly_to_max_rail_width_and_dock_nav_steps_cleanly() {
        let commits = vec![
            json!({"commit": "c3", "short": "c3", "subject": "Merge", "parents": ["c2", "c1"], "merge": true}),
            json!({"commit": "c2", "short": "c2", "subject": "Side", "parents": ["c0"], "merge": false}),
            json!({"commit": "c1", "short": "c1", "subject": "Main", "parents": ["c0"], "merge": false}),
            json!({"commit": "c0", "short": "c0", "subject": "Root", "parents": [], "merge": false}),
        ];
        let layouts = compute_commit_rail_layouts(&commits);
        assert_eq!(layouts[0].rail_width, 38);
        assert_eq!(layouts[3].rail_width, 24);
        for (idx, layout) in layouts.iter().enumerate() {
            assert!(
                layout
                    .rail_svg
                    .contains("width=\"38\" height=\"36\" viewBox=\"0 0 38 36\""),
                "row {idx} rail_svg was not padded uniformly: {}",
                layout.rail_svg
            );
        }

        let selected = selected_commit_row_index(&commits, Some("commit:c1"));
        assert_eq!(selected, Some(2));
        assert_eq!(
            next_commit_dock_index(None, selected, 4, "ArrowDown"),
            Some(3)
        );
        assert_eq!(
            next_commit_dock_index(None, selected, 4, "ArrowUp"),
            Some(1)
        );
        assert_eq!(
            next_commit_dock_index(Some(3), selected, 4, "ArrowDown"),
            Some(3)
        );
        assert_eq!(
            next_commit_dock_index(Some(0), selected, 4, "ArrowUp"),
            Some(0)
        );
        assert_eq!(next_commit_dock_index(None, None, 4, "ArrowDown"), Some(0));
        assert_eq!(next_commit_dock_index(None, None, 0, "ArrowDown"), None);

        let dock_html = render_commit_graph_rows(
            &commits,
            Some(1),
            Some("commit:c2"),
            false,
            0,
            CommitGraphSurface::Dock,
        );
        assert!(
            dock_html.contains("id=\"graph-commit-row-0\" role=\"option\" aria-selected=\"false\"")
        );
        assert!(dock_html.contains(
            "id=\"graph-commit-row-1\" role=\"option\" aria-selected=\"true\" data-current=\"true\""
        ));

        let range = resolve_commit_range_selection(&commits, Some("commit:c2"), "c0");
        assert_eq!(range, Some(("c0".to_owned(), "c2".to_owned())));
        let ranged_html = render_commit_graph_rows_with_range(
            &commits,
            None,
            Some("commit:c2"),
            Some(("c0", "c2")),
            None,
            false,
            0,
            CommitGraphSurface::Dock,
        );
        assert!(ranged_html.contains("data-range=\"end\""));
        assert!(ranged_html.contains("data-range=\"between\""));
        assert!(ranged_html.contains("data-range=\"start\""));
        let banner = render_commit_range_banner("c0", "c2");
        assert!(banner.contains("Range diff:"));
        assert!(banner.contains("graph-commit-range-clear"));

        let many: Vec<Value> = (0..120)
            .map(|i| json!({"commit": format!("sha{i}"), "short": format!("s{i}"), "subject": format!("C{i}"), "parents": []}))
            .collect();
        let windowed = render_commit_graph_rows_with_range(
            &many,
            None,
            None,
            None,
            Some((36.0 * 30.0, 360.0)),
            false,
            0,
            CommitGraphSurface::Dock,
        );
        assert!(windowed.contains("class=\"git-graph-spacer\""));
        assert!(windowed.contains("graph-commit-row-30"));
        assert!(!windowed.contains("graph-commit-row-0\""));

        let first = build_layouts_cached(&many, Some("sha5"));
        let second = build_layouts_cached(&many, Some("sha5"));
        assert!(Rc::ptr_eq(&first, &second));
        let changed = build_layouts_cached(&many, Some("sha6"));
        assert!(!Rc::ptr_eq(&first, &changed));
    }

    #[test]
    fn dock_window_prescroll_cache_refs_and_aspect_ratio_contracts() {
        assert_eq!(commit_dock_window_range(80, 360.0, 360.0), (0, 80));
        assert_eq!(commit_dock_window_range(120, 0.0, 360.0), (0, 46));
        assert_eq!(
            commit_dock_window_range(120, (119 * ROW_HEIGHT) as f64, 360.0),
            (101, 120)
        );
        assert_eq!(scroll_top_for_commit_row(119, 0.0, 360.0), Some(3960.0));
        assert_eq!(scroll_top_for_commit_row(0, 500.0, 360.0), Some(0.0));
        assert_eq!(scroll_top_for_commit_row(2, 0.0, 360.0), None);

        let base = vec![json!({
            "commit": "sha1",
            "short": "sha1",
            "subject": "Base",
            "parents": [],
            "refs": [{"name": "main", "kind": "head"}],
            "target": "commit:sha1",
        })];
        let refs_updated = vec![json!({
            "commit": "sha1",
            "short": "sha1",
            "subject": "Base",
            "parents": [],
            "refs": [{"name": "main", "kind": "head"}, {"name": "v1.0", "kind": "tag"}],
            "target": "commit:sha1",
        })];
        let a = build_layouts_cached(&base, Some("sha1"));
        let b = build_layouts_cached(&refs_updated, Some("sha1"));
        assert!(!Rc::ptr_eq(&a, &b));

        assert!(a[0]
            .rail_svg
            .contains("preserveAspectRatio=\"xMidYMid meet\""));
        assert!(!a[0].rail_svg.contains("preserveAspectRatio=\"none\""));
    }
}
