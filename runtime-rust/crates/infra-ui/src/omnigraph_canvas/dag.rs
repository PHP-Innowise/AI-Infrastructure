use super::*;

#[allow(dead_code)]
pub fn render_branch_dag(
    graph: &Value,
    active_branch: &str,
    active_snapshot: Option<&str>,
) -> String {
    let active_branch = if active_branch.trim().is_empty() {
        str_field(graph, &["active_branch"]).if_empty("main")
    } else {
        active_branch.trim().to_owned()
    };
    let active_snapshot_clean = active_snapshot
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let (branches, commits) = extract_dag_data(graph, &active_branch);

    // Build deterministic lane assignment: lane 0 is always "main".
    let mut lane_of: BTreeMap<String, usize> = BTreeMap::new();
    let mut lane_names: Vec<String> = Vec::new();
    let mut assign_lane = |name: &str| {
        let trimmed = name.trim();
        if trimmed.is_empty() || lane_of.contains_key(trimmed) {
            return;
        }
        let idx = lane_names.len();
        lane_of.insert(trimmed.to_owned(), idx);
        lane_names.push(trimmed.to_owned());
    };
    assign_lane("main");
    if active_branch != "main" {
        assign_lane(&active_branch);
    }
    for br in &branches {
        assign_lane(&br.name);
    }
    for cm in &commits {
        assign_lane(&cm.branch);
    }

    let lane_count = lane_names.len().max(1);
    let rail_w = DAG_RAIL_PAD_X * 2.0 + (lane_count.saturating_sub(1) as f64) * DAG_LANE_W + 8.0;

    let mut out = format!(
        "<div class=\"omni-dag\" data-active-branch=\"{}\" data-active-snapshot=\"{}\" data-lane-count=\"{lane_count}\">",
        esc(&active_branch),
        esc(active_snapshot_clean.as_deref().unwrap_or("")),
    );

    // Branch header rail chips with quick actions
    out.push_str("<div class=\"omni-dag-branches\" role=\"list\" aria-label=\"Context branches\">");
    for (lane, name) in lane_names.iter().enumerate() {
        let info = branches.iter().find(|b| b.name == *name);
        let is_active = *name == active_branch;
        let is_default = name == "main" || info.is_some_and(|b| b.is_default);
        let ahead = info.map_or(0, |b| b.ahead_count);
        let head_id = info
            .map(|b| b.head_commit.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                commits
                    .iter()
                    .find(|c| c.branch == *name)
                    .map(|c| c.id.as_str())
            })
            .unwrap_or("");

        out.push_str(&format!(
            "<div class=\"omni-dag-branch-chip\" role=\"listitem\" data-branch=\"{}\" data-lane=\"{lane}\" data-lane-tone=\"{}\" data-active=\"{}\">",
            esc(name),
            lane % 6,
            bool_str(is_active),
        ));
        out.push_str(&format!(
            "<span class=\"omni-dag-branch-swatch\" data-lane-tone=\"{}\" aria-hidden=\"true\"></span>",
            lane % 6
        ));
        out.push_str(&format!(
            "<span class=\"omni-dag-branch-name\">{}</span>",
            esc(name)
        ));
        if is_default {
            out.push_str("<span class=\"omni-dag-badge\" data-kind=\"default\">Default</span>");
        }
        if is_active {
            out.push_str("<span class=\"omni-dag-badge\" data-kind=\"active\">Active</span>");
        }
        if ahead > 0 && name != "main" {
            out.push_str(&format!(
                "<span class=\"omni-dag-badge\" data-kind=\"ahead\">+{ahead}</span>"
            ));
        }
        if !head_id.is_empty() {
            out.push_str(&format!(
                "<code class=\"omni-dag-branch-head\">{}</code>",
                esc(&short_hash(head_id))
            ));
        }
        out.push_str("<span class=\"omni-dag-branch-actions\">");
        if !is_active {
            out.push_str(&format!(
                "<button type=\"button\" class=\"omni-branch-action\" data-action=\"omnigraph-switch-branch\" data-branch=\"{}\">Switch</button>",
                esc(name)
            ));
        }
        if name != "main" {
            out.push_str(&format!(
                "<button type=\"button\" class=\"omni-branch-action\" data-action=\"omnigraph-preview-diff\" data-branch=\"{}\">Preview Diff</button>",
                esc(name)
            ));
        }
        out.push_str("</span></div>");
    }
    out.push_str("</div>");

    if commits.is_empty() {
        out.push_str("<div class=\"omni-dag-empty\"><p>No commits recorded on these branches yet. Publish context or run a session to build history.</p></div></div>");
        return out;
    }

    let total_h = (commits.len() as f64) * DAG_ROW_H;
    let mut row_of_commit: BTreeMap<String, usize> = BTreeMap::new();
    for (idx, cm) in commits.iter().enumerate() {
        if !cm.id.is_empty() {
            row_of_commit.insert(cm.id.clone(), idx);
        }
    }

    let lane_x = |lane: usize| -> f64 { DAG_RAIL_PAD_X + (lane as f64) * DAG_LANE_W };
    let row_y = |row: usize| -> f64 { (row as f64) * DAG_ROW_H + DAG_ROW_H * 0.5 };

    out.push_str("<div class=\"omni-dag-timeline\">");
    out.push_str(&format!(
        "<svg class=\"omni-dag-svg\" width=\"{:.0}\" height=\"{:.0}\" viewBox=\"0 0 {:.0} {:.0}\" aria-hidden=\"true\">",
        rail_w, total_h, rail_w, total_h
    ));

    // Vertical swimlane rail guides
    for (lane, name) in lane_names.iter().enumerate() {
        let lx = lane_x(lane);
        out.push_str(&format!(
            "<line class=\"omni-dag-lane-guide\" x1=\"{lx:.1}\" y1=\"0\" x2=\"{lx:.1}\" y2=\"{total_h:.1}\" data-lane=\"{lane}\" data-lane-tone=\"{}\" data-branch=\"{}\"/>",
            lane % 6,
            esc(name),
        ));
    }

    // Commit-to-parent and fork/merge Bezier rails
    for (row, cm) in commits.iter().enumerate() {
        let c_lane = *lane_of.get(&cm.branch).unwrap_or(&0);
        let cx = lane_x(c_lane);
        let cy = row_y(row);

        let mut drew_parent = false;
        for (p_idx, pid) in cm.parents.iter().enumerate() {
            if let Some(&prow) = row_of_commit.get(pid) {
                drew_parent = true;
                let p_branch = commits[prow].branch.as_str();
                let p_lane = *lane_of.get(p_branch).unwrap_or(&0);
                let px = lane_x(p_lane);
                let py = row_y(prow);
                let kind = if p_lane == c_lane {
                    "straight"
                } else if p_idx > 0 || cm.parents.len() > 1 && p_lane != c_lane {
                    "merge"
                } else {
                    "fork"
                };
                let tone = if kind == "merge" {
                    p_lane % 6
                } else {
                    c_lane % 6
                };
                let mid_y = (cy + py) * 0.5;
                out.push_str(&format!(
                    "<path class=\"omni-dag-rail\" d=\"M {cx:.1} {cy:.1} C {cx:.1} {mid_y:.1}, {px:.1} {mid_y:.1}, {px:.1} {py:.1}\" data-kind=\"{kind}\" data-lane=\"{c_lane}\" data-lane-tone=\"{tone}\"/>"
                ));
            }
        }

        // If a merge commit names `merged_from` without listing a second parent row, draw a merge curve from that branch lane.
        if !cm.merged_from.is_empty() && cm.parents.len() < 2 {
            if let Some(&src_lane) = lane_of.get(&cm.merged_from) {
                if src_lane != c_lane {
                    let sx = lane_x(src_lane);
                    let sy = (cy + DAG_ROW_H * 0.85).min(total_h - 6.0);
                    let mid_y = (cy + sy) * 0.5;
                    out.push_str(&format!(
                        "<path class=\"omni-dag-rail\" d=\"M {cx:.1} {cy:.1} C {cx:.1} {mid_y:.1}, {sx:.1} {mid_y:.1}, {sx:.1} {sy:.1}\" data-kind=\"merge\" data-lane=\"{src_lane}\" data-lane-tone=\"{}\"/>",
                        src_lane % 6
                    ));
                }
            }
        }

        // If this is the oldest commit on a non-main branch without a visible parent commit, show its fork curve from parent branch (or lane 0).
        if !drew_parent && c_lane > 0 {
            let parent_branch = branches
                .iter()
                .find(|b| b.name == cm.branch)
                .map(|b| b.parent_branch.as_str())
                .filter(|p| !p.is_empty())
                .unwrap_or("main");
            let p_lane = *lane_of.get(parent_branch).unwrap_or(&0);
            if p_lane != c_lane {
                let px = lane_x(p_lane);
                let py = (cy + DAG_ROW_H * 0.75).min(total_h - 4.0);
                let mid_y = (cy + py) * 0.5;
                out.push_str(&format!(
                    "<path class=\"omni-dag-rail\" d=\"M {cx:.1} {cy:.1} C {cx:.1} {mid_y:.1}, {px:.1} {mid_y:.1}, {px:.1} {py:.1}\" data-kind=\"fork\" data-lane=\"{c_lane}\" data-lane-tone=\"{}\"/>",
                    c_lane % 6
                ));
            }
        }
    }

    // Commit dots on top of rails
    for (row, cm) in commits.iter().enumerate() {
        let c_lane = *lane_of.get(&cm.branch).unwrap_or(&0);
        let cx = lane_x(c_lane);
        let cy = row_y(row);
        let is_snapshot = active_snapshot_clean.as_deref() == Some(cm.id.as_str());
        let is_active_branch = cm.branch == active_branch;
        let is_merge = cm.parents.len() > 1 || !cm.merged_from.is_empty();
        let is_head = branches
            .iter()
            .any(|b| b.name == cm.branch && b.head_commit == cm.id);

        if is_snapshot || is_head {
            out.push_str(&format!(
                "<circle class=\"omni-dag-dot-halo\" cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"9.5\" data-snapshot=\"{}\" data-head=\"{}\" data-lane-tone=\"{}\"/>",
                bool_str(is_snapshot),
                bool_str(is_head),
                c_lane % 6,
            ));
        }
        out.push_str(&format!(
            "<circle class=\"omni-dag-dot\" cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{}\" data-lane=\"{c_lane}\" data-lane-tone=\"{}\" data-active-branch=\"{}\" data-snapshot=\"{}\" data-merge=\"{}\"/>",
            if is_merge { "6.2" } else { "5.0" },
            c_lane % 6,
            bool_str(is_active_branch),
            bool_str(is_snapshot),
            bool_str(is_merge),
        ));
    }

    out.push_str("</svg>");

    // Interactive commit rows aligned beside the SVG rail
    out.push_str("<div class=\"omni-dag-rows\" role=\"list\" aria-label=\"Commit history\">");
    for cm in &commits {
        let c_lane = *lane_of.get(&cm.branch).unwrap_or(&0);
        let is_active_branch = cm.branch == active_branch;
        let is_snapshot = active_snapshot_clean.as_deref() == Some(cm.id.as_str());
        let is_head = branches
            .iter()
            .any(|b| b.name == cm.branch && b.head_commit == cm.id);
        let is_merge = cm.parents.len() > 1 || !cm.merged_from.is_empty();

        let delta_sign = if cm.atom_delta > 0 {
            "pos"
        } else if cm.atom_delta < 0 {
            "neg"
        } else {
            "zero"
        };
        let delta_text = if cm.atom_delta > 0 {
            format!("+{} atoms", cm.atom_delta)
        } else if cm.atom_delta < 0 {
            format!("{} atoms", cm.atom_delta)
        } else {
            "0 atoms".to_owned()
        };

        let aria_label = format!(
            "Commit {} on {}: {}, by {}, {}",
            short_hash(&cm.id),
            cm.branch,
            cm.summary,
            cm.actor,
            delta_text
        );

        out.push_str(&format!(
            "<div class=\"omni-dag-row\" role=\"listitem\" style=\"padding-left:{rail_w:.0}px;min-height:{DAG_ROW_H:.0}px\" data-commit-id=\"{}\" data-branch=\"{}\" data-lane=\"{c_lane}\" data-lane-tone=\"{}\" data-active-branch=\"{}\" data-snapshot=\"{}\" data-head=\"{}\">",
            esc(&cm.id),
            esc(&cm.branch),
            c_lane % 6,
            bool_str(is_active_branch),
            bool_str(is_snapshot),
            bool_str(is_head),
        ));

        out.push_str(&format!(
            "<button type=\"button\" class=\"omni-commit-node\" data-action=\"omnigraph-select-commit\" data-commit-id=\"{}\" data-branch=\"{}\" data-active-branch=\"{}\" data-snapshot=\"{}\" data-head=\"{}\" aria-current=\"{}\" aria-label=\"{}\">",
            esc(&cm.id),
            esc(&cm.branch),
            bool_str(is_active_branch),
            bool_str(is_snapshot),
            bool_str(is_head),
            if is_snapshot { "step" } else { "false" },
            esc(&aria_label),
        ));
        out.push_str("<span class=\"omni-commit-head\">");
        out.push_str(&format!(
            "<code class=\"omni-commit-hash\">{}</code>",
            esc(&short_hash(&cm.id))
        ));
        if is_merge {
            out.push_str("<span class=\"omni-commit-kind-badge\">Merge</span>");
        }
        out.push_str(&format!(
            "<span class=\"omni-commit-summary\">{}</span>",
            esc(&cm.summary)
        ));
        out.push_str("</span>");

        out.push_str("<span class=\"omni-commit-meta\">");
        out.push_str(&format!(
            "<span class=\"omni-branch-pill\" data-lane-tone=\"{}\">{}</span>",
            c_lane % 6,
            esc(&cm.branch)
        ));
        out.push_str(&format!(
            "<span class=\"omni-actor-badge\">{}</span>",
            esc(&cm.actor)
        ));
        out.push_str(&format!(
            "<span class=\"omni-delta-badge\" data-sign=\"{delta_sign}\">{delta_text}</span>"
        ));
        if is_snapshot {
            out.push_str("<span class=\"omni-snapshot-badge\">Snapshot</span>");
        } else if is_head {
            out.push_str("<span class=\"omni-head-badge\">HEAD</span>");
        }
        if !cm.created_at.is_empty() {
            out.push_str(&format!(
                "<span class=\"omni-commit-time\">{}</span>",
                esc(&cm.created_at)
            ));
        }
        out.push_str("</span></button>");

        out.push_str("<span class=\"omni-commit-actions\">");
        if !is_active_branch {
            out.push_str(&format!(
                "<button type=\"button\" class=\"omni-branch-action\" data-action=\"omnigraph-switch-branch\" data-branch=\"{}\" title=\"Switch active branch to {}\">Switch</button>",
                esc(&cm.branch),
                esc(&cm.branch),
            ));
        }
        if cm.branch != "main" {
            out.push_str(&format!(
                "<button type=\"button\" class=\"omni-branch-action\" data-action=\"omnigraph-preview-diff\" data-branch=\"{}\" title=\"Preview diff of {} against main\">Diff</button>",
                esc(&cm.branch),
                esc(&cm.branch),
            ));
        }
        out.push_str("</span></div>");
    }
    out.push_str("</div></div></div>");
    out
}

pub(super) struct DagBranch {
    name: String,
    head_commit: String,
    parent_branch: String,
    is_default: bool,
    ahead_count: usize,
}

pub(super) struct DagCommit {
    id: String,
    branch: String,
    parents: Vec<String>,
    merged_from: String,
    actor: String,
    summary: String,
    created_at: String,
    atom_delta: i64,
}

pub(super) fn extract_dag_data(
    graph: &Value,
    active_branch: &str,
) -> (Vec<DagBranch>, Vec<DagCommit>) {
    let mut branches = Vec::new();
    let mut seen_branches: BTreeSet<String> = BTreeSet::new();

    if let Some(arr) = graph.get("branches").and_then(Value::as_array) {
        for item in arr {
            let (name, head, parent, is_default, ahead) = if let Some(s) = item.as_str() {
                (
                    s.trim().to_owned(),
                    String::new(),
                    String::new(),
                    s.trim() == "main",
                    0,
                )
            } else {
                let name = str_field(item, &["name", "branch", "id"]);
                let head = str_field(
                    item,
                    &["head_commit", "head_commit_id", "head", "commit_id"],
                )
                .if_empty_from(|| {
                    item.get("head")
                        .map(|h| str_field(h, &["graph_commit_id", "id", "commit_id"]))
                        .unwrap_or_default()
                });
                let parent = str_field(
                    item,
                    &["parent_branch", "from", "base_branch", "forked_from"],
                );
                let is_default = name == "main" || bool_field(item, &["is_default", "default"]);
                let ahead = i64_field(item, &["ahead", "ahead_count", "unmerged_count"])
                    .unwrap_or(0)
                    .max(0) as usize;
                (name, head, parent, is_default, ahead)
            };
            if name.is_empty() || !seen_branches.insert(name.clone()) {
                continue;
            }
            branches.push(DagBranch {
                name,
                head_commit: head,
                parent_branch: parent,
                is_default,
                ahead_count: ahead,
            });
        }
    }

    if !seen_branches.contains("main") {
        branches.insert(
            0,
            DagBranch {
                name: "main".to_owned(),
                head_commit: String::new(),
                parent_branch: String::new(),
                is_default: true,
                ahead_count: 0,
            },
        );
        seen_branches.insert("main".to_owned());
    }
    if !active_branch.is_empty() && !seen_branches.contains(active_branch) {
        branches.push(DagBranch {
            name: active_branch.to_owned(),
            head_commit: String::new(),
            parent_branch: "main".to_owned(),
            is_default: false,
            ahead_count: 0,
        });
    }

    let mut commits = Vec::new();
    if let Some(arr) = graph.get("commits").and_then(Value::as_array) {
        for item in arr {
            let id = str_field(item, &["id", "graph_commit_id", "commit_id"]);
            if id.is_empty() {
                continue;
            }
            let branch =
                str_field(item, &["branch", "source_branch", "graph_branch"]).if_empty("main");
            let mut parents = string_list(item, &["parents", "parent_ids"]);
            if parents.is_empty() {
                let single = str_field(item, &["parent_commit_id", "parent", "parent_id"]);
                if !single.is_empty() {
                    parents.push(single);
                }
            }
            let merged_parent = str_field(item, &["merged_parent_commit_id"]);
            if !merged_parent.is_empty() && !parents.contains(&merged_parent) {
                parents.push(merged_parent);
            }
            let merged_from = str_field(item, &["merged_from", "source_branch"]);
            let actor = str_field(item, &["actor", "actor_id"]).if_empty("harness");
            let summary = str_field(item, &["summary", "message", "title", "kind"])
                .if_empty("Context checkpoint");
            let created_at = str_field(item, &["created_at", "authored_at", "timestamp"])
                .if_empty_from(|| {
                    i64_field(item, &["created_at", "authored_at", "timestamp"])
                        .map(format_timestamp_i64)
                        .unwrap_or_default()
                });
            let atom_delta = i64_field(
                item,
                &[
                    "atom_delta",
                    "delta_atoms",
                    "atoms_added",
                    "added_count",
                    "changes_count",
                ],
            )
            .unwrap_or(0);

            commits.push(DagCommit {
                id,
                branch,
                parents,
                merged_from,
                actor,
                summary,
                created_at,
                atom_delta,
            });
        }
    }

    (branches, commits)
}
