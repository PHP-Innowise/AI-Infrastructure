use super::*;

pub(in crate::graph_map) fn side_tabs_html(
    side_tab: &str,
    changes_count: u64,
    surface: &str,
) -> String {
    let tab = match side_tab {
        "changes" => "changes",
        "analysis" => "analysis",
        _ => "details",
    };
    let btn = |key: &str, label: String| {
        let sel = tab == key;
        let p = payload(&json!({"tab": key, "surface": surface}));
        format!(
            "<button type=\"button\" class=\"graph-side-tab\" role=\"tab\" aria-selected=\"{sel}\" tabindex=\"{}\" data-action=\"graph-side-tab\" data-tab=\"{key}\" data-payload=\"{p}\">{label}</button>",
            if sel { "0" } else { "-1" }
        )
    };
    format!(
        "<div class=\"graph-side-tabs graph-map-side-tabs\" role=\"tablist\" aria-label=\"Graph inspector tabs\">{}{}{}</div>",
        btn("details", "Details".to_owned()),
        btn(
            "changes",
            format!("Changes <span class=\"graph-map-total\">({changes_count})</span>")
        ),
        btn("analysis", "Analysis".to_owned()),
    )
}
pub(in crate::graph_map) fn side_top_html(
    side_tab: &str,
    changes_count: u64,
    surface: &str,
    show_close: bool,
) -> String {
    let tabs = side_tabs_html(side_tab, changes_count, surface);
    if !show_close {
        return format!("<div class=\"graph-side-top\">{tabs}</div>");
    }
    let close_action = match surface {
        "system-member" => "system-member-graph-side-close",
        "system-change" => "system-change-graph-side-close",
        _ => "session-graph-side-close",
    };
    format!(
        "<div class=\"graph-side-top\">{tabs}<button type=\"button\" class=\"button compact graph-side-close\" data-action=\"{close_action}\" title=\"Close details\" aria-label=\"Close details\">Close</button></div>"
    )
}
pub fn changes_side_html(changes: &V, filter: &str, active_focus: &str, surface: &str) -> String {
    if !changes.is_object() {
        return "<div class=\"graph-side-panel\" data-tab=\"changes\"><p class=\"knowledge-note\">Reading workspace changes…</p></div>".to_owned();
    }
    let files = m::array(&changes["files"]);
    let mut created_count = 0u64;
    let mut modified_count = 0u64;
    let mut deleted_count = 0u64;
    for file in &files {
        match m::text(file, "status").to_ascii_lowercase().as_str() {
            "created" => created_count += 1,
            "deleted" => deleted_count += 1,
            "modified" | "renamed" => modified_count += 1,
            _ => {}
        }
    }
    let total_files = changes["total_files"]
        .as_u64()
        .unwrap_or(files.len() as u64);
    let created_entities = changes["created_entities"].as_u64().unwrap_or(0);
    let modified_entities = changes["modified_entities"].as_u64().unwrap_or(0);
    let deleted_entities = changes["deleted_entities"].as_u64().unwrap_or(0);
    let total_vis_warnings: u64 = changes["visibility_warnings"]
        .as_u64()
        .or_else(|| changes["summary"]["visibility_warnings"].as_u64())
        .unwrap_or_else(|| {
            files
                .iter()
                .map(|file| file["summary"]["visibility_warnings"].as_u64().unwrap_or(0))
                .sum()
        });
    let vis_warning_badge = if total_vis_warnings > 0 {
        format!(
            "<span class=\"graph-status-pill\" data-status=\"broken_risk\">!{} visibility {}</span>",
            count(total_vis_warnings),
            if total_vis_warnings == 1 {
                "warning"
            } else {
                "warnings"
            }
        )
    } else {
        String::new()
    };
    let mut html = format!(
        "<div class=\"graph-side-panel\" data-tab=\"changes\"><div class=\"graph-changes-summary\"><span>{} changed {}</span><span>+{} ~{} -{} entities</span>{vis_warning_badge}</div>",
        count(total_files),
        if total_files == 1 { "file" } else { "files" },
        count(created_entities),
        count(modified_entities),
        count(deleted_entities),
    );
    let active_filter = match filter {
        "created" | "modified" | "deleted" => filter,
        _ => "all",
    };
    html.push_str(
        "<div class=\"graph-changes-filters\" role=\"group\" aria-label=\"Filter changed files\">",
    );
    for (key, label, n) in [
        ("all", "All", files.len() as u64),
        ("created", "Created", created_count),
        ("modified", "Modified", modified_count),
        ("deleted", "Deleted", deleted_count),
    ] {
        let fpayload = payload(&json!({"filter": key, "surface": surface}));
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-changes-filter\" aria-pressed=\"{}\" data-action=\"graph-changes-filter\" data-filter=\"{key}\" data-payload=\"{fpayload}\">{label} <span class=\"graph-map-total\">{}</span></button>",
            if active_filter == key { "true" } else { "false" },
            count(n),
        ));
    }
    html.push_str("</div>");
    let matching: Vec<&V> = files
        .iter()
        .filter(|file| {
            let status = m::text(file, "status").to_ascii_lowercase();
            match active_filter {
                "created" => status == "created",
                "modified" => matches!(status.as_str(), "modified" | "renamed"),
                "deleted" => status == "deleted",
                _ => true,
            }
        })
        .collect();
    if matching.is_empty() {
        html.push_str("<p class=\"knowledge-note\">No changed files match this filter.</p></div>");
        return html;
    }
    let norm_active = normalize_rel_path(strip_symbol_suffix(active_focus));
    html.push_str("<div class=\"graph-changes-list\">");
    for file in matching {
        let path = normalize_rel_path(&m::text(file, "path"));
        let prev = normalize_rel_path(&m::text(file, "previous_path"));
        let status = m::text(file, "status").to_ascii_lowercase();
        let (badge, status_word) = change_status_badge(&status);
        let exists = file["exists"].as_bool().unwrap_or(status != "deleted");
        let in_graph = file["in_graph"].as_bool().unwrap_or(true);
        let sum = &file["summary"];
        let uses_added = sum["uses_added"].as_u64().unwrap_or(0);
        let uses_removed = sum["uses_removed"].as_u64().unwrap_or(0);
        let at_risk = sum["callers_at_risk"].as_u64().unwrap_or(0);
        let impacted = sum["impacted_callers"].as_u64().unwrap_or(0);
        let vis_warnings = sum["visibility_warnings"].as_u64().unwrap_or(0);
        let selected = !norm_active.is_empty() && norm_active == path;
        let file_payload = payload(&json!({"path": path, "surface": surface}));
        html.push_str(&format!(
            "<div class=\"graph-changes-file\" data-selected=\"{selected}\"><button type=\"button\" class=\"graph-changes-file-btn\" data-action=\"graph-map-impact-focus\" data-path=\"{}\" data-payload=\"{file_payload}\" aria-label=\"{}, {status_word}\"><div class=\"graph-changes-file-head\"><span class=\"graph-changes-file-path\">{}</span><span class=\"graph-status-pill\" data-status=\"{}\">{badge}</span></div><div class=\"graph-changes-file-meta\">",
            escape(&path),
            escape(&path),
            escape(&path),
            escape(&status),
        ));
        if !prev.is_empty() && prev != path {
            html.push_str(&format!("<span>from {}</span>", escape(&prev)));
        }
        if !exists {
            html.push_str("<span class=\"graph-status-pill\" data-status=\"broken_risk\">Missing on disk</span>");
        } else if !in_graph {
            html.push_str(
                "<span class=\"graph-status-pill\" data-status=\"unchanged\">Not indexed</span>",
            );
        }
        if uses_added > 0 {
            html.push_str(&format!(
                "<span class=\"graph-status-pill\" data-status=\"added\">+{}</span>",
                count(uses_added)
            ));
        }
        if uses_removed > 0 {
            html.push_str(&format!(
                "<span class=\"graph-status-pill\" data-status=\"removed\">-{}</span>",
                count(uses_removed)
            ));
        }
        if at_risk > 0 {
            html.push_str(&format!(
                "<span class=\"graph-status-pill\" data-status=\"broken_risk\">!{} {}</span>",
                count(at_risk),
                if at_risk == 1 { "caller" } else { "callers" }
            ));
        } else if impacted > 0 {
            html.push_str(&format!(
                "<span class=\"graph-status-pill\" data-status=\"impacted\">~{} callers</span>",
                count(impacted)
            ));
        }
        if vis_warnings > 0 {
            html.push_str(&format!(
                "<span class=\"graph-status-pill\" data-status=\"broken_risk\">!{} visibility {}</span>",
                count(vis_warnings),
                if vis_warnings == 1 {
                    "warning"
                } else {
                    "warnings"
                }
            ));
        }
        html.push_str("</div></button>");
        let entities = m::array(&file["entities"]);
        if !entities.is_empty() {
            html.push_str("<div class=\"graph-changes-entities\">");
            for entity in &entities {
                let ename = m::text(entity, "name");
                let ekind = normalize_entity_kind(&m::text(entity, "kind"));
                let echange = m::text(entity, "change").to_ascii_lowercase();
                let (ebadge, _) = change_status_badge(&echange);
                let line = entity["line"].as_u64().unwrap_or(0);
                let sig = m::yes(&entity["signature_changed"]);
                let vis_pill = visibility_pill_html(entity["visibility"].as_str(), true);
                let prev_vis = entity["previous_visibility"].as_str();
                let curr_vis = entity["visibility"].as_str();
                let vis_changed = m::yes(&entity["visibility_narrowed"])
                    || m::yes(&entity["visibility_changed"])
                    || (prev_vis.is_some() && curr_vis.is_some() && prev_vis != curr_vis);
                let vis_delta = if vis_changed {
                    vis_delta_badge_html(prev_vis, curr_vis, m::yes(&entity["visibility_narrowed"]))
                } else {
                    String::new()
                };
                let epayload = payload(&json!({
                    "path": path,
                    "entity_id": m::text(entity, "id"),
                    "line": line,
                    "surface": surface,
                }));
                let loc = if line > 0 {
                    format!(":{line}")
                } else {
                    String::new()
                };
                html.push_str(&format!(
                    "<button type=\"button\" class=\"graph-changes-entity\" data-action=\"graph-map-impact-focus\" data-path=\"{}\" data-payload=\"{epayload}\"><span class=\"graph-changes-entity-kind\">{}</span>{vis_pill}<span class=\"graph-changes-entity-name\">{}{}</span>{vis_delta}{}<span class=\"graph-status-pill\" data-status=\"{}\">{ebadge}</span></button>",
                    escape(&path),
                    escape(entity_kind_badge(&ekind)),
                    escape(&ename),
                    loc,
                    if sig {
                        "<span class=\"graph-status-pill\" data-status=\"modified\">sig</span>"
                    } else {
                        ""
                    },
                    escape(if echange.is_empty() { "modified" } else { &echange }),
                ));
            }
            html.push_str("</div>");
        }
        html.push_str("</div>");
    }
    html.push_str("</div></div>");
    html
}
pub(in crate::graph_map) fn metric_count(val: &V) -> u64 {
    val.as_u64()
        .or_else(|| val.as_array().map(|arr| arr.len() as u64))
        .unwrap_or(0)
}
#[allow(clippy::too_many_arguments)]
pub(in crate::graph_map) fn impact_dep_group_html(
    title: &str,
    diff_key: &str,
    deps: &[V],
    self_path: &str,
    surface: &str,
    member_filter: &str,
    default_open: bool,
    collapsible: bool,
) -> String {
    if deps.is_empty() {
        return String::new();
    }
    let mut html = String::new();
    if collapsible {
        html.push_str(&format!(
            "<details class=\"graph-impact-group\"{}><summary><span class=\"graph-impact-group-title\">{title} <span class=\"graph-map-total\">{}</span></span></summary>",
            if default_open { " open" } else { "" },
            count(deps.len() as u64)
        ));
    } else {
        html.push_str(&format!(
            "<div class=\"graph-impact-group\"><h4>{title} <span class=\"graph-map-total\">{}</span></h4>",
            count(deps.len() as u64)
        ));
    }
    for dep in deps {
        let target = normalize_rel_path(strip_symbol_suffix(&m::text(dep, "target")));
        let resolved = dep["resolved"].as_bool().unwrap_or(!target.is_empty());
        let exists = dep["exists"].as_bool().unwrap_or(true);
        let kind = m::text(dep, "kind").replace('_', " ");
        let line = dep["line"].as_u64().unwrap_or(0);
        let evidence = m::text(dep, "evidence");
        let symbols: Vec<String> = m::array(&dep["symbols"])
            .iter()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect();
        let member_refs = {
            let refs = m::array(&dep["member_refs"]);
            if refs.is_empty() {
                m::array(&dep["referenced_members"])
            } else {
                refs
            }
        };
        let target_members = {
            let exposed = m::array(&dep["target_members"]);
            if exposed.is_empty() {
                m::array(&dep["declared_members"])
            } else {
                exposed
            }
        };
        let warnings = m::array(&dep["visibility_warnings"]);
        let has_vis_risk = !warnings.is_empty()
            || member_refs.iter().any(member_has_severe_risk)
            || target_members.iter().any(member_has_severe_risk);
        let jump_payload = payload(&json!({"path": target, "surface": surface}));
        let link_payload = payload(&json!({"from": self_path, "to": target, "surface": surface}));
        let can_jump = !target.is_empty() && (resolved || exists);
        html.push_str(&format!(
            "<div class=\"graph-impact-row\" data-diff=\"{diff_key}\"><div class=\"graph-impact-row-main\">"
        ));
        if can_jump {
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-impact-row-btn\" data-action=\"graph-map-impact-focus\" data-path=\"{}\" data-payload=\"{jump_payload}\"><span class=\"graph-status-pill\" data-status=\"{diff_key}\">{}</span> <span>{}</span></button>",
                escape(&target),
                diff_badge_sign(diff_key),
                escape(&target),
            ));
        } else {
            html.push_str(&format!(
                "<span class=\"graph-impact-row-btn\"><span class=\"graph-status-pill\" data-status=\"{diff_key}\">{}</span> <span>{}</span></span>",
                diff_badge_sign(diff_key),
                escape(&target),
            ));
        }
        html.push_str("<div class=\"graph-impact-row-actions\">");
        if !exists {
            html.push_str(
                "<span class=\"graph-status-pill\" data-status=\"broken_risk\">missing</span>",
            );
        } else if !resolved {
            html.push_str(
                "<span class=\"graph-status-pill\" data-status=\"unchanged\">unresolved</span>",
            );
        } else if !target.is_empty() && target != self_path && looks_like_file_path(&target) {
            html.push_str(&format!(
                "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-link\" data-payload=\"{link_payload}\">Links</button>"
            ));
        }
        html.push_str("</div></div>");
        let mut meta = Vec::new();
        if !kind.is_empty() {
            meta.push(escape(&kind));
        }
        if line > 0 {
            meta.push(format!("L{line}"));
        }
        if !symbols.is_empty() {
            meta.push(escape(&symbols.join(", ")));
        }
        if !member_refs.is_empty() || !target_members.is_empty() {
            meta.push(format!(
                "{} used · {} exposed",
                member_refs.len(),
                target_members.len()
            ));
        }
        if has_vis_risk {
            meta.push(
                "<span class=\"graph-status-pill\" data-status=\"broken_risk\">vis risk</span>"
                    .to_owned(),
            );
        }
        if !meta.is_empty() {
            html.push_str(&format!(
                "<div class=\"graph-impact-symbols\">{}</div>",
                meta.join(" · ")
            ));
        }
        if !evidence.is_empty() {
            html.push_str(&format!(
                "<p class=\"graph-impact-evidence\">{}</p>",
                escape(&evidence)
            ));
        }
        html.push_str(&impact_members_details_html(
            &member_refs,
            &target_members,
            &warnings,
            MemberDetailsOpts {
                member_filter,
                used_title: "Called / Referenced",
                exposed_title: "Exposed by target",
                summary_title: "Properties &amp; methods",
                force_open: has_vis_risk,
            },
        ));
        html.push_str("</div>");
    }
    html.push_str(if collapsible { "</details>" } else { "</div>" });
    html
}
pub(in crate::graph_map) fn caller_rank(impact: &str) -> u8 {
    match impact {
        "broken_risk" => 0,
        "impacted" => 1,
        "added_caller" => 2,
        _ => 3,
    }
}
pub(in crate::graph_map) fn member_filter_bar_html(active: &str, surface: &str) -> String {
    let items = [
        ("all", "All"),
        ("used", "Used"),
        ("exposed", "Exposed"),
        ("risk", "Δ/Risk"),
    ];
    let mut html = String::from(
        "<div class=\"graph-member-filters\" role=\"group\" aria-label=\"Member filter\">",
    );
    for (key, label) in items {
        let pressed = if active == key { "true" } else { "false" };
        let filter_payload = payload(&json!({ "filter": key, "surface": surface }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-member-filter\" data-action=\"graph-map-member-filter\" data-payload=\"{filter_payload}\" aria-pressed=\"{pressed}\">{label}</button>"
        ));
    }
    html.push_str("</div>");
    html
}
#[cfg(test)]
pub fn impact_side_html(impact: &V, view: &V, surface: &str) -> String {
    impact_side_html_with_filter(impact, view, surface, "all")
}
pub fn impact_side_html_with_filter(
    impact: &V,
    view: &V,
    surface: &str,
    member_filter: &str,
) -> String {
    let member_filter = match member_filter {
        "used" | "exposed" | "risk" => member_filter,
        _ => "all",
    };
    let path = normalize_rel_path(strip_symbol_suffix(&m::text(impact, "path")));
    if path.is_empty() {
        return String::new();
    }
    let prev = normalize_rel_path(&m::text(impact, "previous_path"));
    let status = m::text(impact, "status").to_ascii_lowercase();
    let status = if status.is_empty() {
        "unchanged".to_owned()
    } else {
        status
    };
    let exists = impact["exists"].as_bool().unwrap_or(status != "deleted");
    let in_graph = impact["in_graph"].as_bool().unwrap_or(true);
    let (_, status_label) = change_status_badge(&status);
    let mut html = format!(
        "<section class=\"graph-impact-card\"><div class=\"graph-impact-head\"><h3>{}</h3><span class=\"graph-status-pill\" data-status=\"{}\">{status_label}</span></div><p class=\"graph-map-path\">{}</p>",
        escape(base(&path)),
        escape(&status),
        escape(&path),
    );
    if !prev.is_empty() && prev != path {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">Renamed from {}</p>",
            escape(&prev)
        ));
    }
    html.push_str("<div class=\"graph-impact-meta\">");
    if exists {
        html.push_str(
            "<span class=\"graph-status-pill\" data-status=\"created\">Exists in workspace</span>",
        );
    } else {
        html.push_str("<span class=\"graph-status-pill\" data-status=\"broken_risk\">Deleted — missing on disk</span>");
    }
    if m::yes(&impact["baseline"]["available"]) {
        let label = m::text(&impact["baseline"], "label");
        if !label.is_empty() {
            html.push_str(&format!("<span>Baseline: {}</span>", escape(&label)));
        }
    } else {
        let reason = m::text(&impact["baseline"], "reason");
        if !reason.is_empty() {
            html.push_str(&format!("<span>{}</span>", escape(&reason)));
        }
    }
    html.push_str("</div>");
    if !exists {
        html.push_str("<p class=\"graph-impact-missing\">File does not exist on disk in the current workspace.</p>");
    } else if !in_graph {
        html.push_str("<p class=\"graph-impact-missing\">Not yet in Graphify index — workspace dependency evidence shown below.</p>");
    }
    let sum = &impact["summary"];
    let uses_added_n = sum["uses_added"]
        .as_u64()
        .unwrap_or_else(|| m::array(&impact["uses"]["added"]).len() as u64);
    let uses_removed_n = sum["uses_removed"]
        .as_u64()
        .unwrap_or_else(|| m::array(&impact["uses"]["removed"]).len() as u64);
    let uses_unchanged_n = sum["uses_unchanged"]
        .as_u64()
        .unwrap_or_else(|| m::array(&impact["uses"]["unchanged"]).len() as u64);
    let callers_risk_n = sum["callers_at_risk"].as_u64().unwrap_or(0);
    let callers_impacted_n = sum["callers_impacted"].as_u64().unwrap_or(0);
    let added_callers_n = sum["added_callers"].as_u64().unwrap_or(0);
    let vis_warnings_n = sum["visibility_warnings"].as_u64().unwrap_or(0);
    let arch = &impact["architecture_impact"];
    let new_cycles = metric_count(&arch["new_cycles"]).max(sum["new_cycles"].as_u64().unwrap_or(0));
    let resolved_cycles =
        metric_count(&arch["resolved_cycles"]).max(sum["resolved_cycles"].as_u64().unwrap_or(0));
    let new_violations =
        metric_count(&arch["new_violations"]).max(sum["new_violations"].as_u64().unwrap_or(0));
    let resolved_violations = metric_count(&arch["resolved_violations"])
        .max(sum["resolved_violations"].as_u64().unwrap_or(0));
    html.push_str("<div class=\"graph-impact-summary\">");
    html.push_str(&format!(
        "<span class=\"graph-status-pill\" data-status=\"added\">+{} added</span><span class=\"graph-status-pill\" data-status=\"removed\">-{} removed</span><span class=\"graph-status-pill\" data-status=\"unchanged\">={} unchanged</span>",
        count(uses_added_n),
        count(uses_removed_n),
        count(uses_unchanged_n),
    ));
    if callers_risk_n > 0 {
        html.push_str(&format!(
            "<span class=\"graph-status-pill\" data-status=\"broken_risk\">!{} callers at risk</span>",
            count(callers_risk_n)
        ));
    }
    if vis_warnings_n > 0 {
        html.push_str(&format!(
            "<span class=\"graph-status-pill\" data-status=\"broken_risk\">!{} visibility warnings</span>",
            count(vis_warnings_n)
        ));
    }
    if callers_impacted_n > 0 {
        html.push_str(&format!(
            "<span class=\"graph-status-pill\" data-status=\"impacted\">~{} impacted callers</span>",
            count(callers_impacted_n)
        ));
    }
    if added_callers_n > 0 {
        html.push_str(&format!(
            "<span class=\"graph-status-pill\" data-status=\"added_caller\">+{} new callers</span>",
            count(added_callers_n)
        ));
    }
    if new_cycles > 0 || resolved_cycles > 0 {
        html.push_str(&format!(
            "<span class=\"graph-impact-arch\">↻ {} new / {} resolved cycles</span>",
            count(new_cycles),
            count(resolved_cycles)
        ));
    }
    html.push_str("</div>");
    html.push_str(&member_filter_bar_html(member_filter, surface));
    // Preserve Graphify open/follow/path actions when indexed and focused in the current view.
    let view_focus = &view["focus"];
    if view_focus.is_object()
        && normalize_rel_path(&m::text(view_focus, "path")) == path
        && !m::yes(&view_focus["folder"])
        && !at_base(view)
    {
        html.push_str("<div class=\"graph-impact-row-actions\">");
        if view_focus["symbols"].as_u64().unwrap_or(0) > 0 {
            html.push_str(&open_file(&view_focus["path"]));
        }
        if follows(view) || paths(view) || shares(view) || !m::yes(&view_focus["start"]) {
            html.push_str(&follow_links(&view_focus["path"]));
        }
        html.push_str(&path_to(view_focus));
        html.push_str("</div>");
    }
    let entities = m::array(&impact["entities"]);
    if !entities.is_empty() {
        html.push_str(&format!(
            "<div class=\"graph-impact-group\"><h4>Changed entities <span class=\"graph-map-total\">{}</span></h4>",
            count(entities.len() as u64)
        ));
        for ent in &entities {
            let echange = m::text(ent, "change").to_ascii_lowercase();
            let (ebadge, _) = change_status_badge(&echange);
            let ekind = normalize_entity_kind(&m::text(ent, "kind"));
            let line = ent["line"].as_u64().unwrap_or(0);
            let sig = m::yes(&ent["signature_changed"]);
            let vis_pill = visibility_pill_html(ent["visibility"].as_str(), false);
            let kind_pill = member_kind_pill_html(&ekind);
            let prev_vis = ent["previous_visibility"].as_str();
            let curr_vis = ent["visibility"].as_str();
            let vis_changed = m::yes(&ent["visibility_narrowed"])
                || m::yes(&ent["visibility_changed"])
                || (prev_vis.is_some() && curr_vis.is_some() && prev_vis != curr_vis);
            let vis_delta = if vis_changed {
                vis_delta_badge_html(prev_vis, curr_vis, m::yes(&ent["visibility_narrowed"]))
            } else {
                String::new()
            };
            html.push_str(&format!(
                "<div class=\"graph-impact-row\" data-diff=\"{}\"><div class=\"graph-impact-row-main\"><span class=\"graph-impact-row-btn\">{vis_pill}{kind_pill}<span class=\"graph-changes-entity-kind\">{}</span> <span class=\"graph-impact-entity-name\">{}{}</span></span><div class=\"graph-impact-row-actions\">{vis_delta}{}<span class=\"graph-status-pill\" data-status=\"{}\">{ebadge}</span></div></div></div>",
                escape(if echange.is_empty() { "unchanged" } else { &echange }),
                escape(entity_kind_badge(&ekind)),
                escape(&m::text(ent, "name")),
                if line > 0 { format!(":{line}") } else { String::new() },
                if sig {
                    "<span class=\"graph-status-pill\" data-status=\"modified\" title=\"signature changed\" aria-label=\"signature changed\">sig</span>"
                } else {
                    ""
                },
                escape(if echange.is_empty() { "unchanged" } else { &echange }),
            ));
        }
        html.push_str("</div>");
    }
    let added_deps = m::array(&impact["uses"]["added"]);
    let removed_deps = m::array(&impact["uses"]["removed"]);
    let unchanged_deps = m::array(&impact["uses"]["unchanged"]);
    html.push_str(&impact_dep_group_html(
        "Added dependencies (+)",
        "added",
        &added_deps,
        &path,
        surface,
        member_filter,
        true,
        false,
    ));
    html.push_str(&impact_dep_group_html(
        "Removed dependencies (-)",
        "removed",
        &removed_deps,
        &path,
        surface,
        member_filter,
        true,
        false,
    ));
    let mut callers = m::array(&impact["used_by"]);
    callers.sort_by(|a, b| {
        let ra = caller_rank(&m::text(a, "impact"));
        let rb = caller_rank(&m::text(b, "impact"));
        ra.cmp(&rb)
            .then_with(|| {
                b["links"]
                    .as_u64()
                    .unwrap_or(0)
                    .cmp(&a["links"].as_u64().unwrap_or(0))
            })
            .then_with(|| m::text(a, "path").cmp(&m::text(b, "path")))
    });
    if !callers.is_empty() {
        html.push_str(&format!(
            "<div class=\"graph-impact-group\"><h4>Used by (reverse dependents) <span class=\"graph-map-total\">{}</span></h4>",
            count(callers.len() as u64)
        ));
        for caller in &callers {
            let cpath = normalize_rel_path(strip_symbol_suffix(&m::text(caller, "path")));
            let cimpact = m::text(caller, "impact").to_ascii_lowercase();
            let cimpact = if cimpact.is_empty() {
                "unchanged".to_owned()
            } else {
                cimpact
            };
            let pill_label = match cimpact.as_str() {
                "broken_risk" => "Broken risk",
                "impacted" => "Impacted",
                "added_caller" => "Added caller",
                _ => "Unchanged",
            };
            let cexists = caller["exists"].as_bool().unwrap_or(true);
            let reason = m::text(caller, "reason");
            let links = caller["links"].as_u64().unwrap_or(0);
            let guessed = m::yes(&caller["guessed"]);
            let used_members = {
                let direct = m::array(&caller["used_members"]);
                if direct.is_empty() {
                    m::array(&caller["member_refs"])
                } else {
                    direct
                }
            };
            let caller_target_members = m::array(&caller["target_members"]);
            let vis_warnings = m::array(&caller["visibility_warnings"]);
            let has_vis_risk = !vis_warnings.is_empty()
                || used_members.iter().any(member_has_severe_risk)
                || caller_target_members.iter().any(member_has_severe_risk);
            let mut syms: Vec<String> = Vec::new();
            for sym in m::array(&caller["changed_symbols"])
                .into_iter()
                .chain(m::array(&caller["symbols"]))
                .filter_map(|item| {
                    item.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
            {
                if !syms.iter().any(|existing| existing == &sym) {
                    syms.push(sym);
                }
            }
            let jump_payload = payload(&json!({"path": cpath, "surface": surface}));
            let link_payload = payload(&json!({"from": cpath, "to": path, "surface": surface}));
            html.push_str(&format!(
                "<div class=\"graph-impact-row\" data-risk=\"{}\" data-diff=\"{}\"><div class=\"graph-impact-row-main\"><button type=\"button\" class=\"graph-impact-row-btn\" data-action=\"graph-map-impact-focus\" data-path=\"{}\" data-payload=\"{jump_payload}\"><span class=\"graph-status-pill\" data-status=\"{}\">{pill_label}</span> <span>{}</span></button><div class=\"graph-impact-row-actions\">",
                cimpact == "broken_risk",
                if cimpact == "broken_risk" {
                    "risk"
                } else if cimpact == "added_caller" {
                    "added"
                } else {
                    "unchanged"
                },
                escape(&cpath),
                escape(&cimpact),
                escape(&cpath),
            ));
            if !cexists {
                html.push_str(
                    "<span class=\"graph-status-pill\" data-status=\"broken_risk\">missing</span>",
                );
            }
            if cexists && !cpath.is_empty() && cpath != path {
                html.push_str(&format!(
                    "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-link\" data-payload=\"{link_payload}\">Links</button>"
                ));
            }
            html.push_str("</div></div>");
            let mut meta = Vec::new();
            if links > 0 {
                meta.push(format!(
                    "{}{}",
                    plural(links, "link", "links"),
                    if guessed { " · guessed" } else { "" }
                ));
            }
            if !syms.is_empty() {
                meta.push(escape(&syms.join(", ")));
            }
            if !used_members.is_empty() {
                meta.push(format!("{} used", used_members.len()));
            }
            if has_vis_risk {
                meta.push(
                    "<span class=\"graph-status-pill\" data-status=\"broken_risk\">vis risk</span>"
                        .to_owned(),
                );
            }
            if !meta.is_empty() {
                html.push_str(&format!(
                    "<div class=\"graph-impact-symbols\">{}</div>",
                    meta.join(" · ")
                ));
            }
            if !reason.is_empty() {
                html.push_str(&format!(
                    "<p class=\"graph-impact-evidence\">{}</p>",
                    escape(&reason)
                ));
            }
            html.push_str(&impact_members_details_html(
                &used_members,
                &caller_target_members,
                &vis_warnings,
                MemberDetailsOpts {
                    member_filter,
                    used_title: "Referenced here",
                    exposed_title: "Exposed by target",
                    summary_title: "Properties &amp; methods",
                    force_open: cimpact == "broken_risk" || has_vis_risk,
                },
            ));
            html.push_str("</div>");
        }
        html.push_str("</div>");
    }
    html.push_str(&contract_consumers_html(impact, surface));
    html.push_str(&format!(
        "<div class=\"graph-impact-group\"><h4>Architecture &amp; cycles</h4><div class=\"graph-impact-arch\"><span>↻ {} new cycles · {} resolved</span><span>· {} new violations · {} resolved</span></div></div>",
        count(new_cycles),
        count(resolved_cycles),
        count(new_violations),
        count(resolved_violations),
    ));
    html.push_str(&impact_dep_group_html(
        "Unchanged dependencies (=)",
        "unchanged",
        &unchanged_deps,
        &path,
        surface,
        member_filter,
        added_deps.is_empty() && removed_deps.is_empty(),
        true,
    ));
    html.push_str("</section>");
    html
}
pub fn contract_consumers_html(impact: &V, surface: &str) -> String {
    let consumers = {
        let primary = m::array(&impact["contract_consumers"]);
        if primary.is_empty() {
            m::array(&impact["cross_service_consumers"])
        } else {
            primary
        }
    };
    if consumers.is_empty() {
        return String::new();
    }
    let mut html = format!(
        "<div class=\"graph-impact-group\" data-contract-consumers=\"true\"><h4>Cross-service contract consumers <span class=\"graph-map-total\">{}</span></h4>",
        count(consumers.len() as u64)
    );
    for item in &consumers {
        let service = {
            let s = m::text(item, "service");
            if !s.is_empty() {
                s
            } else {
                let cs = m::text(item, "consumer_service");
                if !cs.is_empty() {
                    cs
                } else {
                    m::text(item, "consumer")
                }
            }
        };
        let contract = {
            let c = m::text(item, "contract");
            if !c.is_empty() {
                c
            } else {
                m::text(item, "name")
            }
        };
        let kind = {
            let k = m::text(item, "kind");
            if !k.is_empty() {
                k
            } else {
                m::text(item, "protocol")
            }
        };
        let version = {
            let v = m::text(item, "version");
            if !v.is_empty() {
                v
            } else {
                let cv = m::text(item, "consumer_version");
                if !cv.is_empty() {
                    cv
                } else {
                    m::text(item, "provider_version")
                }
            }
        };
        let unopened_sources: Vec<String> = m::array(&item["unopened_sources"])
            .into_iter()
            .filter_map(|v| v.as_str().map(normalize_rel_path).filter(|s| !s.is_empty()))
            .collect();
        let mut consumer_paths = Vec::new();
        for v in m::array(&item["consumer_paths"])
            .into_iter()
            .chain(m::array(&item["consumer_sources"]))
            .chain(m::array(&item["unopened_sources"]))
            .chain(m::array(&item["files"]))
            .chain(m::array(&item["sources"]))
        {
            if let Some(norm) = v.as_str().map(normalize_rel_path).filter(|s| !s.is_empty()) {
                if !consumer_paths.contains(&norm) {
                    consumer_paths.push(norm);
                }
            }
        }
        let direct_path = {
            let p = normalize_rel_path(strip_symbol_suffix(&m::text(item, "path")));
            if !p.is_empty() {
                p
            } else {
                let cp = normalize_rel_path(strip_symbol_suffix(&m::text(item, "consumer_path")));
                if !cp.is_empty() {
                    cp
                } else {
                    unopened_sources
                        .first()
                        .or_else(|| consumer_paths.first())
                        .cloned()
                        .unwrap_or_default()
                }
            }
        };
        let raw_impact = m::text(item, "impact").to_ascii_lowercase();
        let unopened_flag = m::yes(&item["unopened"])
            || item["opened"].as_bool() == Some(false)
            || !unopened_sources.is_empty();
        let risk = m::yes(&item["at_risk"])
            || m::yes(&item["broken_risk"])
            || raw_impact == "broken_risk"
            || m::text(item, "risk") == "broken_risk";
        let (status_attr, pill_label) = if risk {
            ("broken_risk", "Broken risk")
        } else if unopened_flag || raw_impact == "unopened" {
            ("broken_risk", "Unopened consumer")
        } else {
            ("impacted", "Contract consumer")
        };
        let head_label = if !service.is_empty() && !direct_path.is_empty() {
            format!("{service} · {direct_path}")
        } else if !service.is_empty() {
            service.clone()
        } else if !direct_path.is_empty() {
            direct_path.clone()
        } else {
            contract.clone()
        };
        let jump_payload = payload(&json!({
            "service": service,
            "path": direct_path,
            "surface": surface,
        }));
        html.push_str(&format!(
            "<div class=\"graph-impact-row\" data-risk=\"{}\" data-diff=\"{}\"><div class=\"graph-impact-row-main\">",
            risk || unopened_flag,
            if risk || unopened_flag { "risk" } else { "impacted" },
        ));
        if !service.is_empty() {
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-impact-row-btn\" data-action=\"graph-map-switch-service-file\" data-service=\"{}\" data-path=\"{}\" data-payload=\"{jump_payload}\"><span class=\"graph-status-pill\" data-status=\"{status_attr}\">{pill_label}</span> <span>{}</span></button>",
                escape(&service),
                escape(&direct_path),
                escape(&head_label),
            ));
        } else {
            html.push_str(&format!(
                "<span class=\"graph-impact-row-btn\"><span class=\"graph-status-pill\" data-status=\"{status_attr}\">{pill_label}</span> <span>{}</span></span>",
                escape(&head_label),
            ));
        }
        html.push_str("</div>");
        let mut meta = Vec::new();
        if !contract.is_empty() {
            meta.push(format!("contract: {}", escape(&contract)));
        }
        if !kind.is_empty() {
            meta.push(escape(&kind));
        }
        if !version.is_empty() {
            let trimmed_ver = version.strip_prefix('v').unwrap_or(&version);
            meta.push(format!("v{}", escape(trimmed_ver)));
        }
        if unopened_flag {
            meta.push("not opened in run".to_owned());
        }
        if consumer_paths.len() > 1 {
            meta.push(escape(&consumer_paths.join(", ")));
        }
        if !meta.is_empty() {
            html.push_str(&format!(
                "<div class=\"graph-impact-symbols\">{}</div>",
                meta.join(" · ")
            ));
        }
        let reason = {
            let r = m::text(item, "reason");
            if !r.is_empty() {
                r
            } else {
                let rr = m::text(item, "risk_reason");
                if !rr.is_empty() {
                    rr
                } else {
                    m::text(item, "note")
                }
            }
        };
        if !reason.is_empty() {
            html.push_str(&format!(
                "<p class=\"graph-impact-evidence\">{}</p>",
                escape(&reason)
            ));
        }
        html.push_str("</div>");
    }
    html.push_str("</div>");
    html
}
pub fn build_scoped_changes_path_for_state(
    scope: &GraphTargetScope,
    state: &V,
    focus_session: &str,
) -> String {
    let raw_target = effective_target(state);
    let target = if raw_target.is_empty() {
        "work"
    } else {
        raw_target.as_str()
    };
    build_scoped_changes_path(scope, target, focus_session, m::yes(&state["notests"]))
}
pub(in crate::graph_map) async fn fetch_surface_changes(surface: MapSurface, state: V) {
    let scope = surface_scope(surface);
    if scope.is_empty() {
        m::set(surface.changes_key, V::Null);
        update_surface_changes_badge(surface, None);
        return;
    }
    let scope_key = scope.scope_key();
    let raw_target = effective_target(&state);
    let effective_target = if raw_target.is_empty() {
        "work"
    } else {
        raw_target.as_str()
    };
    let sid = if surface.name == "session" {
        m::read(|s| s.sid.clone())
    } else {
        m::text(&state, "focus_session")
    };
    let url = build_scoped_changes_path_for_state(&scope, &state, &sid);
    match api::scoped(surface.changes_epoch, &url).await {
        Ok((epoch, doc)) => {
            if !m::current(surface.changes_epoch, epoch)
                || surface_scope(surface).scope_key() != scope_key
            {
                return;
            }
            m::set(
                surface.changes_key,
                json!({
                    "scope_key": scope_key,
                    "project": scope.project_id(),
                    "target": effective_target,
                    "data": doc
                }),
            );
            rerender_surface_preserving_camera(surface);
        }
        Err(error) if error.aborted => {}
        Err(error) => {
            if surface_scope(surface).scope_key() != scope_key {
                return;
            }
            m::set(
                surface.changes_key,
                json!({
                    "scope_key": scope_key,
                    "project": scope.project_id(),
                    "target": effective_target,
                    "error": error.message,
                    "data": {}
                }),
            );
            if m::text(&surface_state(surface), "side_tab") == "changes" {
                rerender_surface_side_only(surface);
            }
        }
    }
}
pub(in crate::graph_map) async fn fetch_surface_impact(
    surface: MapSurface,
    state: V,
    path: String,
    key: String,
) {
    let scope = surface_scope(surface);
    if scope.is_empty() || path.is_empty() {
        return;
    }
    let scope_key = scope.scope_key();
    let raw_target = effective_target(&state);
    let effective_target = if raw_target.is_empty() {
        "work"
    } else {
        raw_target.as_str()
    };
    let sid = if surface.name == "session" {
        m::read(|s| s.sid.clone())
    } else {
        m::text(&state, "focus_session")
    };
    let certain = m::yes(&state["certain"]);
    let url = build_scoped_file_impact_path(&scope, &path, effective_target, &sid, certain);
    match api::scoped(surface.impact_epoch, &url).await {
        Ok((epoch, doc)) => {
            if !m::current(surface.impact_epoch, epoch)
                || surface_scope(surface).scope_key() != scope_key
            {
                return;
            }
            m::set(
                surface.impact_key,
                json!({"key": key, "loading": false, "data": doc}),
            );
            rerender_surface_preserving_camera(surface);
            if dom::get("graph-link").is_some_and(|d| d.has_attribute("open")) {
                draw_link();
            }
        }
        Err(error) if error.aborted => {}
        Err(_) => {
            let cur = m::data(surface.impact_key);
            if m::text(&cur, "key") == key {
                m::set(surface.impact_key, V::Null);
            }
        }
    }
}
pub(in crate::graph_map) fn ensure_surface_impact(
    surface: MapSurface,
    state: &V,
    view: Option<&V>,
    force: bool,
) {
    let scope = surface_scope(surface);
    if scope.is_empty() {
        return;
    }
    let changes = surface_changes_doc(surface, state);
    let Some(path) = selected_impact_file(state, view, changes.as_ref()) else {
        return;
    };
    let wanted_key = impact_cache_key(&scope.scope_key(), &path, state);
    let cached = m::data(surface.impact_key);
    if !force && m::text(&cached, "key") == wanted_key {
        return;
    }
    m::set(
        surface.impact_key,
        json!({"key": wanted_key.clone(), "loading": true}),
    );
    let state_clone = state.clone();
    spawn_local(async move {
        fetch_surface_impact(surface, state_clone, path, wanted_key).await;
    });
}
pub(in crate::graph_map) fn surface_unindexed_side_html(
    surface: MapSurface,
    state: &V,
    changes: Option<&V>,
) -> String {
    if surface_scope(surface).is_empty() {
        return String::new();
    }
    let side_tab = m::text(state, "side_tab");
    let changes_count = changes_count_of(changes);
    if side_tab == "changes" {
        let active = selected_impact_file(state, None, changes);
        return format!(
            "{}{}",
            side_top_html("changes", changes_count, surface.name, true),
            changes_side_html(
                changes.unwrap_or(&V::Null),
                &m::text(state, "changes_filter"),
                active.as_deref().unwrap_or(""),
                surface.name,
            ),
        );
    }
    if let Some(impact) = cached_impact_for_selection(surface, state, None) {
        return format!(
            "{}{}",
            side_top_html("details", changes_count, surface.name, true),
            impact_side_html_with_filter(
                &impact,
                &V::Null,
                surface.name,
                &m::text(state, "member_filter")
            ),
        );
    }
    String::new()
}
pub(in crate::graph_map) fn build_surface_side_html(
    surface: MapSurface,
    view: &V,
    state: &V,
) -> String {
    let prefix = surface.id_prefix;
    let attach_action = format!("{prefix}-attach");
    let changes_tab = m::text(state, "side_tab") == "changes";
    let module_mode = m::yes(&state["modules"]);
    let scenario_mode = view["mode"] == "scenario";
    if scenario_mode && !changes_tab {
        m::take(surface.attachment_key);
        dom::attr(surface.canvas, "data-picking", "false");
        scenario_side(view)
    } else if module_mode && !changes_tab {
        m::take(surface.attachment_key);
        dom::attr(surface.canvas, "data-picking", "false");
        modules_side(view)
    } else {
        dom::attr(
            surface.canvas,
            "data-picking",
            if state["pick"].is_object() {
                "true"
            } else {
                "false"
            },
        );
        let action = if cycles(view) {
            let mut actions = base_cycle_button(view, at_base(view));
            if state["cycle_return"].is_object() {
                actions.push_str(CYCLE_BACK);
            }
            actions
        } else if patterned(view) && state["pattern_return"].is_object() {
            PATTERN_BACK.to_owned()
        } else {
            String::new()
        };
        let test_back = if paths(view) && state["test_path_return"].is_object() {
            "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-test-back\">Back to changed files</button>"
        } else {
            ""
        };
        set_surface_attrs(surface, view);
        map_side_html(
            surface,
            view,
            state,
            &action,
            &scenario_button(view),
            test_back,
            &attach_action,
        )
    }
}

pub(in crate::graph_map) fn rerender_surface_side_only(surface: MapSurface) {
    let view = m::data(surface.view_key);
    let state = surface_state(surface);
    if surface.embedded {
        let changes = surface_changes_doc(surface, &state);
        sync_surface_toolbar(surface, &state, changes.as_ref());
        let was_empty = dom::get(surface.side).is_none_or(|el| el.inner_html().is_empty());
        let side_html = if view["nodes"].is_array() {
            build_surface_side_html(surface, &view, &state)
        } else {
            surface_unindexed_side_html(surface, &state, changes.as_ref())
        };
        html(surface.side, &side_html);
        let now_empty = side_html.is_empty();
        if was_empty != now_empty {
            let mut cam = load_surface_camera(surface);
            cam.needs_measure = true;
            save_surface_camera(surface, &cam);
            dom::later(surface.refit_timer_key, 60, move || {
                refit_surface_if_visible(surface);
            });
        }
    } else if open() && view.is_object() {
        let modal_changes = surface_changes_doc(MODAL_SURFACE, &state);
        let changes_tab = m::text(&state, "side_tab") == "changes";
        let module_mode = m::yes(&state["modules"]);
        let scenario_mode = view["mode"] == "scenario";
        if scenario_mode && !changes_tab {
            html(
                "graph-map-side",
                &format!(
                    "{}{}",
                    side_tabs_html("details", changes_count_of(modal_changes.as_ref()), "modal"),
                    scenario_side(&view)
                ),
            );
        } else if module_mode && !changes_tab {
            html(
                "graph-map-side",
                &format!(
                    "{}{}",
                    side_tabs_html("details", changes_count_of(modal_changes.as_ref()), "modal"),
                    modules_side(&view)
                ),
            );
        } else {
            let side_html = build_surface_side_html(MODAL_SURFACE, &view, &state);
            html("graph-map-side", &side_html);
        }
        sync_modal_explain_button(Some(&view), &state);
    }
}

pub(in crate::graph_map) fn rerender_surface_preserving_camera(surface: MapSurface) {
    let view = m::data(surface.view_key);
    if surface.embedded {
        let state = surface_state(surface);
        let changes = surface_changes_doc(surface, &state);
        sync_surface_toolbar(surface, &state, changes.as_ref());
        if view["nodes"].is_array() {
            render_embedded_surface(surface, &view, false, false);
        } else {
            html(
                surface.side,
                &surface_unindexed_side_html(surface, &state, changes.as_ref()),
            );
        }
    } else if open() && view.is_object() {
        render(&view);
    }
}

pub(in crate::graph_map) fn combine_note(before: String, view: &V) -> String {
    format_graph_status_text(
        &[
            before,
            m::text(view, "architecture_error"),
            note(view),
            cycle_revision(view),
        ]
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" "),
    )
}
pub(in crate::graph_map) fn map_side_html(
    surface: MapSurface,
    view: &V,
    state: &V,
    action: &str,
    scenario_action: &str,
    test_back: &str,
    attach_action: &str,
) -> String {
    let attachment = map_attachment(view);
    let scope = surface_scope(surface);
    m::set(
        surface.attachment_key,
        json!({
            "project": if view["project_id"].is_string() { view["project_id"].clone() } else { json!(scope.project_id()) },
            "scope": scope.to_json(),
            "text": attachment
        }),
    );
    let attach = if attachment.is_some() {
        format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"{attach_action}\">Attach map context</button>"
        )
    } else {
        String::new()
    };
    let focused_node_key = {
        let p = m::text(&view["focus"], "path");
        if !p.is_empty() {
            p
        } else {
            m::text(state, "focus")
        }
    };
    let unpin_btn = if !focused_node_key.is_empty()
        && graph_viewport::load_pins(&surface_pin_key(surface, state))
            .contains_key(&focused_node_key)
    {
        let unpin_action = if surface.embedded {
            format!("{}-unpin-node", surface.id_prefix)
        } else {
            "graph-map-unpin-node".to_owned()
        };
        format!(
            "<button type=\"button\" class=\"button compact graph-map-unpin-btn\" data-action=\"{unpin_action}\" data-payload=\"{}\">Unpin node</button>",
            payload(&json!(focused_node_key)),
        )
    } else {
        String::new()
    };
    let changes_doc = surface_changes_doc(surface, state);
    let changes_count = changes_count_of(changes_doc.as_ref());
    let side_tab = m::text(state, "side_tab");
    let changes_tab = side_tab == "changes";
    let analysis_tab = side_tab == "analysis";
    let active_focus = selected_impact_file(state, Some(view), changes_doc.as_ref());
    let impact_doc = cached_impact_for_selection(surface, state, Some(view));
    if surface.embedded
        && !changes_tab
        && !analysis_tab
        && !m::yes(&state["side_pinned"])
        && !view["focus"].is_object()
        && active_focus.is_none()
        && impact_doc.is_none()
        && !cycles(view)
        && !patterned(view)
        && view["mode"] != "scenario"
        && !state["pick"].is_object()
        && !view["chosen"].is_object()
    {
        return String::new();
    }
    let show_close = surface.embedded
        && (view["focus"].is_object()
            || active_focus.is_some()
            || changes_tab
            || analysis_tab
            || m::yes(&state["side_pinned"]));
    let header = side_top_html(
        if changes_tab {
            "changes"
        } else if analysis_tab {
            "analysis"
        } else {
            "details"
        },
        changes_count,
        surface.name,
        show_close,
    );
    if changes_tab {
        let changes_raw = m::data(surface.changes_key);
        let changes_err = m::text(&changes_raw, "error");
        let err_note = if !changes_err.is_empty() && changes_doc.is_none() {
            format!(
                "<p class=\"error-text\" role=\"alert\">Could not load changes: {}</p>",
                escape(&changes_err),
            )
        } else {
            String::new()
        };
        return format!(
            "{header}{err_note}{}",
            changes_side_html(
                changes_doc.as_ref().unwrap_or(&V::Null),
                &m::text(state, "changes_filter"),
                active_focus.as_deref().unwrap_or(""),
                surface.name,
            ),
        );
    }
    if analysis_tab {
        return format!(
            "{header}<div class=\"graph-side-panel\" data-tab=\"analysis\">{}{}{}{}{}{}{}{}{}{}</div>",
            choosing(state),
            run_comparison(view),
            blast_side(view),
            without_incoming_side(view),
            suspects_side(view, state, surface),
            test_paths(view),
            history_side(view),
            circles_side(view),
            architecture_side(view),
            found_patterns(view, &unfolded(state, "kinds")),
        );
    }
    let conf_badge = confidence_pill_html(view);
    let details_body = if let Some(impact) = impact_doc.as_ref() {
        format!(
            "{conf_badge}{}{}{test_back}{action}{scenario_action}{}{}{}{attach}{unpin_btn}{}{}{}{}{}{}{}{}",
            choosing(state),
            paging(view),
            run_comparison(view),
            shell_reads(view),
            impact_side_html_with_filter(
                impact,
                view,
                surface.name,
                &m::text(state, "member_filter")
            ),
            blast_side(view),
            without_incoming_side(view),
            suspects_side(view, state, surface),
            test_paths(view),
            history_side(view),
            circles_side(view),
            architecture_side(view),
            found_patterns(view, &unfolded(state, "kinds")),
        )
    } else if let Some(focused_path) = active_focus.as_deref() {
        if view["focus"].is_object() {
            format!(
                "{conf_badge}{}{}{test_back}{action}{scenario_action}{}{}{}{attach}{unpin_btn}{}<p class=\"knowledge-note\">Reading dependency impact…</p>{}{}{}{}{}{}{}",
                choosing(state),
                paging(view),
                run_comparison(view),
                shell_reads(view),
                side(view),
                blast_side(view),
                without_incoming_side(view),
                suspects_side(view, state, surface),
                test_paths(view),
                history_side(view),
                circles_side(view),
                architecture_side(view),
                found_patterns(view, &unfolded(state, "kinds")),
            )
        } else {
            format!(
                "{}{attach}{unpin_btn}<section class=\"graph-impact-card\"><div class=\"graph-impact-head\"><h3>{}</h3></div><p class=\"graph-map-path\">{}</p><p class=\"knowledge-note\">Reading file dependency impact…</p></section>",
                choosing(state),
                escape(base(focused_path)),
                escape(focused_path),
            )
        }
    } else if surface.embedded
        && !view["focus"].is_object()
        && !cycles(view)
        && !patterned(view)
        && view["mode"] != "scenario"
        && !state["pick"].is_object()
        && !view["chosen"].is_object()
    {
        format!(
            "{conf_badge}<p class=\"knowledge-note\">Select a file or folder on the map to inspect its dependencies and change impact.</p>"
        )
    } else {
        format!(
            "{conf_badge}{}{}{test_back}{action}{scenario_action}{}{}{}{attach}{unpin_btn}{}{}{}{}{}{}{}{}",
            choosing(state),
            paging(view),
            run_comparison(view),
            shell_reads(view),
            side(view),
            blast_side(view),
            without_incoming_side(view),
            suspects_side(view, state, surface),
            test_paths(view),
            history_side(view),
            circles_side(view),
            architecture_side(view),
            found_patterns(view, &unfolded(state, "kinds")),
        )
    };
    // What the project's memory says about the selected file fills in when
    // it arrives; the map does not wait for it.
    let memory = active_focus
        .as_deref()
        .map(|path| normalize_rel_path(strip_symbol_suffix(path)))
        .filter(|path| !path.is_empty() && !path.ends_with('/'))
        .map(|path| {
            let project = view["project_id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| scope.project_id().to_string());
            memory_links_section(&project, &path)
        })
        .unwrap_or_default();
    format!(
        "{header}<div class=\"graph-side-panel\" data-tab=\"details\">{details_body}{memory}</div>"
    )
}
pub(in crate::graph_map) const MEMORY_LINKS: &str = "graph-memory-links";
pub(in crate::graph_map) const MEMORY_LINKS_LOOKING: &str = "<h4>Linked memory</h4><p class=\"knowledge-note\">Looking for Project Brain records and Memory Bank chunks that cite this file…</p>";
/// A file's links, or a request for them, are good for a minute: a record
/// saved since then shows on the next selection.
pub(in crate::graph_map) fn memory_links_fresh(cached: &V) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        cached["at"]
            .as_f64()
            .is_none_or(|at| js_sys::Date::now() - at < 60_000.)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = cached;
        true
    }
}
/// The linked-memory section of a selected file: drawn from what an earlier
/// request returned, or a placeholder a request fills in.
pub(in crate::graph_map) fn memory_links_section(project: &str, path: &str) -> String {
    let key = format!("{project}\u{1f}{path}");
    let cached = m::data(MEMORY_LINKS)[&key].clone();
    let fresh = cached.is_object() && memory_links_fresh(&cached);
    let body = if fresh && cached["pending"] != true {
        render_side_memory_bridge_html(project, &cached)
    } else if fresh {
        // The map redraws often; one request per file is in flight at a time.
        MEMORY_LINKS_LOOKING.to_owned()
    } else {
        #[cfg(target_arch = "wasm32")]
        {
            let mut cache = m::data(MEMORY_LINKS);
            if !cache.is_object() {
                cache = json!({});
            }
            cache[&key] = json!({"pending": true, "at": js_sys::Date::now()});
            m::set(MEMORY_LINKS, cache);
            let (project, path, key) = (project.to_owned(), path.to_owned(), key.clone());
            spawn_local(async move {
                let url = format!(
                    "/api/projects/{}/graph/memory-links?path={}",
                    dom::enc(&project),
                    dom::enc(&path)
                );
                let mut links = match crate::api::get(&url).await {
                    Ok(links) => links,
                    Err(error) => json!({"error": error.message}),
                };
                links["at"] = json!(js_sys::Date::now());
                let mut cache = m::data(MEMORY_LINKS);
                if !cache.is_object() {
                    cache = json!({});
                }
                cache[&key] = links.clone();
                m::set(MEMORY_LINKS, cache);
                for section in dom::all(".graph-memory-links") {
                    if section.get_attribute("data-path").as_deref() == Some(path.as_str()) {
                        section.set_inner_html(&render_side_memory_bridge_html(&project, &links));
                    }
                }
            });
        }
        MEMORY_LINKS_LOOKING.to_owned()
    };
    format!(
        "<section class=\"graph-memory-links\" data-path=\"{}\" aria-live=\"polite\" aria-label=\"Linked memory\">{body}</section>",
        escape(path)
    )
}
/// What the project's memory says about one file, with a way to open each
/// record or chunk and warnings to review. A failed request says so and
/// leaves the rest of the panel alone.
pub(crate) fn render_side_memory_bridge_html(project: &str, links: &V) -> String {
    let mut html = String::from("<h4>Linked memory</h4>");
    if let Some(error) = links["error"].as_str() {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">Linked memory is unavailable: {}</p>",
            escape(error)
        ));
        return html;
    }
    for warning in m::array(&links["hygiene_warnings"]) {
        html.push_str(&format!(
            "<p class=\"graph-memory-warning\" data-kind=\"{}\" role=\"note\">Review: {}</p>",
            escape(warning["kind"].as_str().unwrap_or("")),
            escape(warning["text"].as_str().unwrap_or(""))
        ));
    }
    // Two atoms of the file that contradict each other: both stay as they
    // are, and the record or chunk behind either opens for a person to judge.
    for warning in m::array(&links["contradiction_warnings"]) {
        let opens = m::array(&warning["open"])
            .iter()
            .filter_map(|open| {
                let scope = open["scope"].as_str().filter(|s| matches!(*s, "brain" | "memory"))?;
                let target = open["target_id"].as_str().filter(|t| !t.is_empty())?;
                Some(format!(
                    " <button type=\"button\" class=\"button compact\" data-action=\"graph-map-open-memory\" data-payload=\"{}\">Open {}</button>",
                    payload(&json!({"scope": scope, "project_id": project, "target_id": target})),
                    escape(open["title"].as_str().unwrap_or(target))
                ))
            })
            .collect::<String>();
        html.push_str(&format!(
            "<p class=\"graph-memory-warning\" data-kind=\"{}\" role=\"note\">Review: {}{opens}</p>",
            escape(warning["kind"].as_str().unwrap_or("atom-contradiction")),
            escape(warning["text"].as_str().unwrap_or(""))
        ));
    }
    let open = |scope: &str, target: &str, label: String| {
        format!(
            "<li><button type=\"button\" class=\"graph-memory-link\" data-action=\"graph-map-open-memory\" data-payload=\"{}\">{label}</button></li>",
            payload(&json!({"scope": scope, "project_id": project, "target_id": target}))
        )
    };
    let records = m::array(&links["brain_records"]);
    let chunks = m::array(&links["memory_chunks"]);
    let atoms = m::array(&links["omnigraph_atoms"]);
    if records.is_empty() && chunks.is_empty() && atoms.is_empty() {
        html.push_str("<p class=\"knowledge-note\">No Project Brain record, Memory Bank chunk or shared context atom cites this file.</p>");
        return html;
    }
    if !records.is_empty() {
        html.push_str(
            "<p class=\"graph-memory-group\">Project Brain</p><ul class=\"graph-memory-list\">",
        );
        for record in &records {
            let target = record["external_id"]
                .as_str()
                .or_else(|| record["id"].as_str())
                .unwrap_or("");
            html.push_str(&open(
                "brain",
                target,
                format!(
                    "<span class=\"graph-memory-kind\">{}</span> {}",
                    escape(record["type"].as_str().unwrap_or("record")),
                    escape(record["title"].as_str().unwrap_or(target))
                ),
            ));
        }
        html.push_str("</ul>");
    }
    if !chunks.is_empty() {
        html.push_str(
            "<p class=\"graph-memory-group\">Memory Bank</p><ul class=\"graph-memory-list\">",
        );
        for chunk in &chunks {
            let id = chunk["id"].as_str().unwrap_or("");
            html.push_str(&open(
                "memory",
                id,
                format!(
                    "<span class=\"graph-memory-kind\">{}</span> {}{}",
                    escape(id),
                    escape(chunk["title"].as_str().unwrap_or("")),
                    if chunk["auto_promoted"] == true {
                        " <span class=\"graph-memory-badge\">auto-promoted</span>"
                    } else {
                        ""
                    }
                ),
            ));
        }
        html.push_str("</ul>");
    }
    if !atoms.is_empty() {
        html.push_str(
            "<p class=\"graph-memory-group\">Shared context</p><ul class=\"graph-memory-list\">",
        );
        for atom in &atoms {
            // The declarations of this file the atom names, as it named them.
            let symbols = m::array(&atom["symbols"])
                .iter()
                .filter_map(V::as_str)
                .take(6)
                .map(escape)
                .collect::<Vec<_>>();
            html.push_str(&format!(
                "<li class=\"graph-memory-atom\">{}{}</li>",
                escape(
                    atom["capsule_line"]
                        .as_str()
                        .or_else(|| atom["title"].as_str())
                        .unwrap_or("")
                        .trim_start_matches("- ")
                ),
                if symbols.is_empty() {
                    String::new()
                } else {
                    format!(
                        " <span class=\"graph-memory-badge\">names {}</span>",
                        symbols.join(", ")
                    )
                }
            ));
        }
        html.push_str("</ul>");
    }
    let linked_path = links["path"].as_str().unwrap_or("").trim();
    if !linked_path.is_empty() {
        let omni_payload = payload(&json!({"project_id": project, "path": linked_path}));
        html.push_str(&format!(
            "<div class=\"graph-memory-actions\"><button type=\"button\" class=\"button compact\" data-action=\"graph-map-open-omnigraph\" data-payload=\"{omni_payload}\">Open in Context Graph</button></div>"
        ));
    }
    html
}
