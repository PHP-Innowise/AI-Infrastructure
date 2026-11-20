use super::constellation::extract_canvas_edge;
use super::*;

#[allow(dead_code)]
pub fn render_merge_diff(diff: &Value) -> String {
    let source = str_field(diff, &["source", "source_branch"]).if_empty("source");
    let target = str_field(diff, &["target", "target_branch"]).if_empty("main");
    let merge_base = str_field(diff, &["merge_base", "base_commit", "base"]);

    let mut added_atoms = array_items(diff, &["added_atoms", "added"]);
    let mut updated_atoms = array_items(diff, &["updated_atoms", "updated", "modified_atoms"]);
    let mut deleted_atoms = array_items(diff, &["deleted_atoms", "removed_atoms", "deleted"]);
    let mut added_edges = array_items(diff, &["added_edges", "edges", "edge_changes"]);
    let mut deleted_edges = array_items(diff, &["deleted_edges", "removed_edges"]);
    if added_atoms.is_empty()
        && updated_atoms.is_empty()
        && deleted_atoms.is_empty()
        && added_edges.is_empty()
        && deleted_edges.is_empty()
    {
        partition_entity_changes(
            diff,
            &mut added_atoms,
            &mut updated_atoms,
            &mut deleted_atoms,
            &mut added_edges,
            &mut deleted_edges,
        );
    }
    let conflicts = array_items(diff, &["conflicts", "merge_conflicts"]);

    let raw_outcome = str_field(diff, &["outcome", "status", "code"]);
    let outcome = normalize_outcome(
        &raw_outcome,
        !conflicts.is_empty(),
        added_atoms.is_empty()
            && updated_atoms.is_empty()
            && deleted_atoms.is_empty()
            && added_edges.is_empty()
            && deleted_edges.is_empty(),
    );

    let mut out = format!(
        "<div class=\"omni-diff\" data-outcome=\"{}\" data-source=\"{}\" data-target=\"{}\">",
        esc(outcome),
        esc(&source),
        esc(&target),
    );

    // Outcome banner
    let (outcome_title, outcome_icon) = match outcome {
        "conflict" => (
            "409 Conflict — Manual Resolution Needed",
            "<svg class=\"omni-outcome-icon\" viewBox=\"0 0 16 16\" width=\"16\" height=\"16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><path d=\"M8 2 L14.5 13.5 H1.5 Z\"/><path d=\"M8 6 V9.2\"/><circle cx=\"8\" cy=\"11.4\" r=\"0.7\" fill=\"currentColor\" stroke=\"none\"/></svg>",
        ),
        "fast_forward" => (
            "Fast-Forward Merge Ready",
            "<svg class=\"omni-outcome-icon\" viewBox=\"0 0 16 16\" width=\"16\" height=\"16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><path d=\"M2.5 4.5 L7.5 8 L2.5 11.5 V4.5 Z M8.5 4.5 L13.5 8 L8.5 11.5 V4.5 Z\"/></svg>",
        ),
        "merged" => (
            "3-Way Merge Clean",
            "<svg class=\"omni-outcome-icon\" viewBox=\"0 0 16 16\" width=\"16\" height=\"16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><circle cx=\"4.5\" cy=\"3.5\" r=\"1.8\"/><circle cx=\"11.5\" cy=\"4.5\" r=\"1.8\"/><circle cx=\"4.5\" cy=\"12.5\" r=\"1.8\"/><path d=\"M4.5 5.3 V10.7 M11.5 6.3 C11.5 9.2 6.5 8.8 4.5 10.7\"/></svg>",
        ),
        _ => (
            "Already Up to Date",
            "<svg class=\"omni-outcome-icon\" viewBox=\"0 0 16 16\" width=\"16\" height=\"16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><circle cx=\"8\" cy=\"8\" r=\"6\"/><path d=\"M5.3 8.1 L7.2 10 L10.8 6.2\"/></svg>",
        ),
    };

    out.push_str(&format!(
        "<div class=\"omni-diff-banner\" data-outcome=\"{}\"><div class=\"omni-diff-banner-main\">{outcome_icon}<div class=\"omni-diff-banner-copy\"><span class=\"omni-diff-outcome-badge\" data-outcome=\"{}\">{outcome_title}</span><div class=\"omni-diff-route\"><span class=\"omni-branch-pill\">{}</span><span class=\"omni-diff-arrow\" aria-label=\"into\">→</span><span class=\"omni-branch-pill\">{}</span>",
        esc(outcome),
        esc(outcome),
        esc(&source),
        esc(&target),
    ));
    if !merge_base.is_empty() {
        out.push_str(&format!(
            "<span class=\"omni-diff-base\">Base <code>{}</code></span>",
            esc(&short_hash(&merge_base))
        ));
    }
    out.push_str("</div></div></div>");

    out.push_str("<div class=\"omni-diff-counts\">");
    out.push_str(&format!(
        "<span class=\"omni-diff-count\" data-change=\"added\">+{} added</span>",
        added_atoms.len()
    ));
    out.push_str(&format!(
        "<span class=\"omni-diff-count\" data-change=\"updated\">~{} updated</span>",
        updated_atoms.len()
    ));
    out.push_str(&format!(
        "<span class=\"omni-diff-count\" data-change=\"deleted\">-{} deleted</span>",
        deleted_atoms.len()
    ));
    let total_edge_changes = added_edges.len() + deleted_edges.len();
    if total_edge_changes > 0 {
        out.push_str(&format!(
            "<span class=\"omni-diff-count\" data-change=\"edges\">{total_edge_changes} edges</span>"
        ));
    }
    if !conflicts.is_empty() {
        out.push_str(&format!(
            "<span class=\"omni-diff-count\" data-change=\"conflict\">! {} conflicts</span>",
            conflicts.len()
        ));
    }
    out.push_str("</div></div>");

    // 409 Conflict cards
    if !conflicts.is_empty() {
        out.push_str("<section class=\"omni-diff-conflicts\" aria-label=\"Merge conflicts\">");
        out.push_str(&format!(
            "<h4 class=\"omni-diff-heading\">Typed 409 Conflicts ({})</h4>",
            conflicts.len()
        ));
        for conflict in &conflicts {
            out.push_str(&render_conflict_card(conflict, &source, &target));
        }
        out.push_str("</section>");
    }

    // Added atoms
    if !added_atoms.is_empty() {
        out.push_str("<section class=\"omni-diff-section\" data-change=\"added\">");
        out.push_str(&format!(
            "<h4 class=\"omni-diff-heading\"><span class=\"omni-diff-mark\" aria-hidden=\"true\">+</span> Added Context Atoms ({})</h4>",
            added_atoms.len()
        ));
        for item in &added_atoms {
            out.push_str(&render_diff_atom_card(item, "added", "+", "Added"));
        }
        out.push_str("</section>");
    }

    // Updated atoms
    if !updated_atoms.is_empty() {
        out.push_str("<section class=\"omni-diff-section\" data-change=\"updated\">");
        out.push_str(&format!(
            "<h4 class=\"omni-diff-heading\"><span class=\"omni-diff-mark\" aria-hidden=\"true\">~</span> Updated Context Atoms ({})</h4>",
            updated_atoms.len()
        ));
        for item in &updated_atoms {
            out.push_str(&render_diff_atom_card(item, "updated", "~", "Updated"));
        }
        out.push_str("</section>");
    }

    // Deleted atoms
    if !deleted_atoms.is_empty() {
        out.push_str("<section class=\"omni-diff-section\" data-change=\"deleted\">");
        out.push_str(&format!(
            "<h4 class=\"omni-diff-heading\"><span class=\"omni-diff-mark\" aria-hidden=\"true\">-</span> Deleted Context Atoms ({})</h4>",
            deleted_atoms.len()
        ));
        for item in &deleted_atoms {
            out.push_str(&render_diff_atom_card(item, "deleted", "-", "Deleted"));
        }
        out.push_str("</section>");
    }

    // Edge changes
    if total_edge_changes > 0 {
        out.push_str("<section class=\"omni-diff-section\" data-change=\"edges\">");
        out.push_str(&format!(
            "<h4 class=\"omni-diff-heading\">Context Edge Changes ({total_edge_changes})</h4>"
        ));
        for edge in &added_edges {
            let change = str_field(edge, &["change", "op", "status"]);
            let (kind, mark, label) = if change.eq_ignore_ascii_case("deleted")
                || change.eq_ignore_ascii_case("removed")
            {
                ("deleted", "-", "Deleted")
            } else {
                ("added", "+", "Added")
            };
            out.push_str(&render_diff_edge_card(edge, kind, mark, label));
        }
        for edge in &deleted_edges {
            out.push_str(&render_diff_edge_card(edge, "deleted", "-", "Deleted"));
        }
        out.push_str("</section>");
    }

    if added_atoms.is_empty()
        && updated_atoms.is_empty()
        && deleted_atoms.is_empty()
        && total_edge_changes == 0
        && conflicts.is_empty()
    {
        out.push_str(&format!(
            "<div class=\"omni-diff-empty\"><p>Branches <strong>{}</strong> and <strong>{}</strong> share the same context state with no diverging atoms or edges.</p></div>",
            esc(&source),
            esc(&target),
        ));
    }

    out.push_str("</div>");
    out
}

pub(super) fn render_conflict_card(conflict: &Value, source: &str, target: &str) -> String {
    let raw_kind = str_field(conflict, &["kind", "conflict_type", "type", "code"])
        .if_empty("divergent_update");
    let kind_token = escape_attr_token(&raw_kind);
    let kind_label = title_case_token(&raw_kind);
    let entity_id = str_field(conflict, &["entity_id", "atom_id", "id", "target_id"]);
    let entity_kind = str_field(conflict, &["entity_kind"]);
    let type_name = str_field(conflict, &["type_name", "node_type"]);
    let field = str_field(conflict, &["field", "property"]);
    let message = str_field(conflict, &["message", "reason", "title", "summary"]);
    let hint = str_field(conflict, &["resolution_hint", "hint"]);

    let base_val = pick_first_value(conflict, &["base", "base_value"]);
    let source_val = pick_first_value(conflict, &CONFLICT_SOURCE_KEYS);
    let target_val = pick_first_value(conflict, &CONFLICT_TARGET_KEYS);
    let has_compare = base_val.is_some() || source_val.is_some() || target_val.is_some();

    let mut html = format!(
        "<article class=\"omni-conflict-card\" data-conflict-kind=\"{kind_token}\" data-entity-id=\"{}\">",
        esc(&entity_id),
    );
    html.push_str("<div class=\"omni-conflict-head\">");
    html.push_str(&render_kind_icon("warning"));
    html.push_str(&format!(
        "<span class=\"omni-conflict-badge\" data-conflict-kind=\"{kind_token}\">{}</span>",
        esc(&kind_label)
    ));
    if !type_name.is_empty() || !entity_kind.is_empty() {
        let type_badge = match (entity_kind.is_empty(), type_name.is_empty()) {
            (false, false) => format!("{entity_kind} · {type_name}"),
            (false, true) => entity_kind.clone(),
            (true, false) => type_name.clone(),
            (true, true) => String::new(),
        };
        html.push_str(&format!(
            "<span class=\"omni-conflict-type\">{}</span>",
            esc(&type_badge)
        ));
    }
    if !entity_id.is_empty() {
        html.push_str(&format!(
            "<code class=\"omni-conflict-entity\">{}</code>",
            esc(&entity_id)
        ));
    }
    if !field.is_empty() {
        html.push_str(&format!(
            "<span class=\"omni-conflict-field\">field: <code>{}</code></span>",
            esc(&field)
        ));
    }
    html.push_str("</div>");

    if !message.is_empty() {
        html.push_str(&format!(
            "<p class=\"omni-conflict-message\">{}</p>",
            esc(&message)
        ));
    }

    if has_compare {
        html.push_str("<div class=\"omni-conflict-compare\">");
        html.push_str(&render_conflict_pane("base", "Base (Ancestor)", base_val));
        html.push_str(&render_conflict_pane(
            "source",
            &format!("Source ({source})"),
            source_val,
        ));
        html.push_str(&render_conflict_pane(
            "target",
            &format!("Target ({target})"),
            target_val,
        ));
        html.push_str("</div>");
    }

    if !hint.is_empty() {
        html.push_str(&format!(
            "<p class=\"omni-conflict-hint\">{}</p>",
            esc(&hint)
        ));
    }
    html.push_str(&render_conflict_keep_html(
        conflict, &entity_id, &field, source, target,
    ));
    html.push_str("</article>");
    html
}

pub(super) const CONFLICT_SOURCE_KEYS: [&str; 3] = ["source", "source_value", "ours"];
pub(super) const CONFLICT_TARGET_KEYS: [&str; 3] = ["target", "target_value", "theirs"];

/// Why a conflict cannot be settled by keeping one side's atom, or `None`
/// when it can: only a Context Atom both branches changed
/// (`divergent_update`) whose source and target values came with the
/// conflict. A deletion against an update, an edge, or a conflict without
/// both values stays with the person.
pub(super) fn conflict_keep_blocker(conflict: &Value) -> Option<&'static str> {
    let kind = str_field(conflict, &["kind", "conflict_type", "type", "code"]);
    if str_field(conflict, &["entity_kind"]).eq_ignore_ascii_case("edge") || kind.contains("edge") {
        return Some("Edge conflicts are not resolved here: change the link on one branch, then preview the merge again.");
    }
    if kind == "delete_vs_update" {
        return Some("One branch deleted this atom while the other changed it. Keeping a side would delete or restore it, so decide in the inspector.");
    }
    if kind != "divergent_update"
        || !matches!(
            str_field(conflict, &["type_name", "node_type"]).as_str(),
            "ContextAtom" | "context_atom"
        )
        || str_field(conflict, &["entity_id", "atom_id", "id", "target_id"]).is_empty()
    {
        return Some("Only two different updates of one Context Atom can be resolved here.");
    }
    let present = |keys: &[&str]| pick_first_value(conflict, keys).is_some_and(|v| !v.is_null());
    if !present(&CONFLICT_SOURCE_KEYS) || !present(&CONFLICT_TARGET_KEYS) {
        return Some("The merge preview did not return both values. Open the atom on each branch to compare them.");
    }
    None
}

/// Keep source copies the source branch's atom onto the target branch, and
/// Keep target copies the target's onto the source, so both branches hold
/// the same atom before the merge runs again. Each button carries the kept
/// value and, as `base`, the value the written branch holds now.
pub(super) fn render_conflict_keep_html(
    conflict: &Value,
    entity_id: &str,
    field: &str,
    source: &str,
    target: &str,
) -> String {
    let blocker = conflict_keep_blocker(conflict);
    let mut html = String::from(
        "<div class=\"omni-conflict-actions\" role=\"group\" aria-label=\"Resolve this conflict\">",
    );
    let value = |keys: &[&str]| {
        pick_first_value(conflict, keys)
            .cloned()
            .unwrap_or(Value::Null)
    };
    for (keep, label, kept, base, write) in [
        (
            "source",
            "Keep source",
            CONFLICT_SOURCE_KEYS,
            CONFLICT_TARGET_KEYS,
            target,
        ),
        (
            "target",
            "Keep target",
            CONFLICT_TARGET_KEYS,
            CONFLICT_SOURCE_KEYS,
            source,
        ),
    ] {
        if blocker.is_some() {
            html.push_str(&format!(
                "<button type=\"button\" class=\"button compact\" disabled>{label}</button>"
            ));
            continue;
        }
        let payload = serde_json::json!({
            "atom_id": entity_id,
            "base": value(&base),
            "field": if field.is_empty() { Value::Null } else { Value::from(field) },
            "keep": keep,
            "source_branch": source,
            "target_branch": target,
            "value": value(&kept),
            "write_branch": write,
        });
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"omnigraph-conflict-keep\" data-payload=\"{}\">{label}</button>",
            esc(&payload.to_string())
        ));
    }
    if blocker.is_some() && !entity_id.is_empty() {
        let focus_payload = serde_json::json!({ "node_id": entity_id });
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"omnigraph-focus-hit\" data-payload=\"{}\">Inspect in Constellation</button>",
            esc(&focus_payload.to_string())
        ));
    }
    html.push_str("</div>");
    html.push_str(&match blocker {
        Some(note) => format!("<p class=\"omni-conflict-note\">{}</p>", esc(note)),
        None => format!(
            "<p class=\"omni-conflict-note\">Keep source copies the source's atom to <strong>{}</strong> as it is stored; Keep target copies the target's to <strong>{}</strong>. The merge preview then runs again.</p>",
            esc(target),
            esc(source)
        ),
    });
    html
}

/// The `POST /api/omnigraph/branches` body a Keep button sends: the `keep`
/// action, which copies the kept branch's atom onto the written branch as
/// that branch stores it. Omnigraph settles a `divergent_update` only once
/// both branches hold the same row, and an ordinary atom update stamps its
/// own branch and time, so it would leave the conflict in place. The
/// `updated_at` of the kept value and of the replaced one (`base`) go along:
/// the server refuses when either branch holds another version than the
/// preview showed.
pub(crate) fn conflict_keep_request(project_id: &str, payload: &Value) -> Result<Value, String> {
    let atom_id = str_field(payload, &["atom_id"]);
    let to = str_field(payload, &["write_branch"]);
    let from = match str_field(payload, &["keep"]).as_str() {
        "source" => str_field(payload, &["source_branch"]),
        "target" => str_field(payload, &["target_branch"]),
        _ => String::new(),
    };
    if project_id.is_empty() || atom_id.is_empty() || from.is_empty() || to.is_empty() || from == to
    {
        return Err("This conflict names no project, atom or pair of branches to keep.".into());
    }
    let mut body = serde_json::json!({
        "action": "keep",
        "project_id": project_id,
        "atom_id": atom_id,
        "from": from,
        "to": to,
    });
    for (key, value) in [
        ("expected_updated_at", &payload["value"]),
        ("replaced_updated_at", &payload["base"]),
    ] {
        let atom = value
            .get("properties")
            .filter(|p| p.is_object())
            .unwrap_or(value);
        if let Some(updated_at) = atom["updated_at"].as_str().filter(|s| !s.trim().is_empty()) {
            body[key] = Value::from(updated_at);
        }
    }
    Ok(body)
}

pub(super) fn render_conflict_pane(side: &str, heading: &str, val: Option<&Value>) -> String {
    let text = format_diff_value(val);
    let is_deleted = text == "(deleted)" || text == "—" || val.is_some_and(Value::is_null);
    format!(
        "<div class=\"omni-conflict-pane\" data-side=\"{side}\" data-empty=\"{}\"><span class=\"omni-conflict-pane-label\">{}</span><pre class=\"omni-conflict-value\">{}</pre></div>",
        bool_str(is_deleted),
        esc(heading),
        esc(&text),
    )
}

pub(super) fn partition_entity_changes(
    diff: &Value,
    added_atoms: &mut Vec<Value>,
    updated_atoms: &mut Vec<Value>,
    deleted_atoms: &mut Vec<Value>,
    added_edges: &mut Vec<Value>,
    deleted_edges: &mut Vec<Value>,
) {
    let Some(changes) = diff.get("changes").and_then(Value::as_array) else {
        return;
    };
    for ch in changes {
        let entity_kind = str_field(ch, &["kind"]).to_ascii_lowercase();
        let op = str_field(ch, &["op"]).to_ascii_lowercase();
        let id = str_field(ch, &["id"]);
        let type_name = ch
            .get("type")
            .and_then(|t| t.as_str().or_else(|| t.get("name").and_then(Value::as_str)))
            .unwrap_or("");
        if entity_kind == "edge" {
            let img = ch
                .get("after")
                .filter(|v| !v.is_null())
                .or_else(|| ch.get("before").filter(|v| !v.is_null()));
            let endpoints = img.and_then(|v| v.get("endpoints"));
            let from = endpoints
                .and_then(|e| e.get("from"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let to = endpoints
                .and_then(|e| e.get("to"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let subkind = img
                .and_then(|v| v.get("properties"))
                .and_then(|p| p.get("rel_kind").or_else(|| p.get("label")))
                .and_then(Value::as_str)
                .unwrap_or("");
            let edge_val = serde_json::json!({
                "from": from,
                "to": to,
                "relation": type_name,
                "subkind": subkind,
            });
            if op == "delete" || op == "remove" {
                deleted_edges.push(edge_val);
            } else {
                added_edges.push(edge_val);
            }
        } else {
            let unwrap_img = |v: Option<&Value>| -> Value {
                match v {
                    Some(obj) if obj.is_object() => {
                        let props = obj
                            .get("properties")
                            .filter(|p| p.is_object())
                            .unwrap_or(obj);
                        let mut cloned = props.clone();
                        if let Some(m) = cloned.as_object_mut() {
                            m.entry("id".to_owned())
                                .or_insert_with(|| Value::String(id.clone()));
                            if !type_name.is_empty() {
                                m.entry("type_name".to_owned())
                                    .or_insert_with(|| Value::String(type_name.to_owned()));
                            }
                        }
                        cloned
                    }
                    _ => Value::Null,
                }
            };
            let before = unwrap_img(ch.get("before"));
            let after = unwrap_img(ch.get("after"));
            match op.as_str() {
                "delete" | "remove" => deleted_atoms.push(if before.is_object() {
                    before
                } else {
                    ch.clone()
                }),
                "update" | "upsert" => updated_atoms.push(serde_json::json!({
                    "id": id,
                    "before": before,
                    "after": after,
                })),
                _ => added_atoms.push(if after.is_object() { after } else { ch.clone() }),
            }
        }
    }
}

pub(super) fn render_diff_atom_card(
    item: &Value,
    change: &str,
    marker: &str,
    sr_label: &str,
) -> String {
    let primary_raw = item
        .get("after")
        .or_else(|| item.get("source"))
        .or_else(|| item.get("atom"))
        .filter(|v| v.is_object())
        .unwrap_or(item);
    let primary = primary_raw
        .get("properties")
        .filter(|v| v.is_object())
        .unwrap_or(primary_raw);
    let previous_raw = item
        .get("before")
        .or_else(|| item.get("target"))
        .or_else(|| item.get("base"))
        .filter(|v| v.is_object());
    let previous = previous_raw.map(|p| p.get("properties").filter(|v| v.is_object()).unwrap_or(p));

    let id = str_field(primary, &["id", "atom_id", "@id"])
        .if_empty_from(|| str_field(primary_raw, &["id", "atom_id", "@id"]))
        .if_empty_from(|| str_field(item, &["id", "atom_id", "entity_id"]));
    let raw_kind = str_field(primary, &["kind"])
        .if_empty_from(|| str_field(item, &["kind"]))
        .if_empty("insight");
    let kind_spec = atom_kind_spec(&raw_kind);
    let title = str_field(primary, &["title", "name", "label"])
        .if_empty_from(|| str_field(item, &["title", "name"]))
        .if_empty(&id);
    let summary = str_field(primary, &["summary", "body", "content"]);
    let branch = str_field(primary, &["branch", "source_branch", "graph_branch"])
        .if_empty_from(|| str_field(item, &["branch", "source_branch", "graph_branch"]));

    let mut html = format!(
        "<article class=\"omni-diff-card\" data-change=\"{change}\" data-kind=\"{}\" data-node-id=\"{}\">",
        kind_spec.key,
        esc(&id),
    );
    html.push_str("<div class=\"omni-diff-card-head\">");
    html.push_str(&format!(
        "<span class=\"omni-diff-mark\" data-change=\"{change}\" aria-hidden=\"true\">{marker}</span><span class=\"sr-only\">{sr_label}</span>"
    ));
    html.push_str(&render_kind_icon(kind_spec.key));
    html.push_str(&format!(
        "<span class=\"omni-kind-badge\" data-kind=\"{}\">{}</span>",
        kind_spec.key, kind_spec.label
    ));
    html.push_str(&format!(
        "<strong class=\"omni-diff-card-title\">{}</strong>",
        esc(&title)
    ));
    if !branch.is_empty() {
        html.push_str(&format!(
            "<span class=\"omni-branch-pill\">{}</span>",
            esc(&branch)
        ));
    }
    html.push_str("</div>");

    if change == "updated" {
        if let Some(prev) = previous {
            let prev_summary = str_field(prev, &["summary", "body", "content", "title"]);
            let next_summary = if summary.is_empty() {
                title.clone()
            } else {
                summary.clone()
            };
            html.push_str("<div class=\"omni-diff-before-after\">");
            if !prev_summary.is_empty() {
                html.push_str(&format!(
                    "<div class=\"omni-diff-side\" data-side=\"before\"><span class=\"omni-diff-side-label\">Before</span><p>{}</p></div>",
                    esc(&prev_summary)
                ));
            }
            if !next_summary.is_empty() {
                html.push_str(&format!(
                    "<div class=\"omni-diff-side\" data-side=\"after\"><span class=\"omni-diff-side-label\">After</span><p>{}</p></div>",
                    esc(&next_summary)
                ));
            }
            html.push_str("</div>");
        } else if !summary.is_empty() {
            html.push_str(&format!(
                "<p class=\"omni-diff-card-summary\">{}</p>",
                esc(&summary)
            ));
        }
    } else if !summary.is_empty() {
        html.push_str(&format!(
            "<p class=\"omni-diff-card-summary\">{}</p>",
            esc(&summary)
        ));
    }

    html.push_str("</article>");
    html
}
pub(super) fn render_diff_edge_card(
    edge: &Value,
    change: &str,
    marker: &str,
    sr_label: &str,
) -> String {
    let Some(parsed) = extract_canvas_edge(edge) else {
        return String::new();
    };
    format!(
        "<div class=\"omni-diff-edge-card\" data-change=\"{change}\" data-rel=\"{}\"><span class=\"omni-diff-mark\" data-change=\"{change}\" aria-hidden=\"true\">{marker}</span><span class=\"sr-only\">{sr_label}</span><span class=\"omni-diff-edge-rel\">{}</span><code class=\"omni-diff-edge-end\">{}</code><span class=\"omni-diff-arrow\" aria-label=\"to\">→</span><code class=\"omni-diff-edge-end\">{}</code></div>",
        escape_attr_token(&parsed.arrow_key),
        esc(&parsed.label),
        esc(&parsed.from),
        esc(&parsed.to),
    )
}
pub(super) fn normalize_outcome(raw: &str, has_conflicts: bool, all_empty: bool) -> &'static str {
    if has_conflicts {
        return "conflict";
    }
    match raw.trim().to_ascii_lowercase().as_str() {
        "conflict" | "conflicts" | "409" => "conflict",
        "fast_forward" | "fast-forward" | "ff" => "fast_forward",
        "merged" | "clean" => "merged",
        "already_up_to_date" | "up_to_date" | "uptodate" | "noop" => "already_up_to_date",
        _ if all_empty => "already_up_to_date",
        _ => "merged",
    }
}
pub(super) fn format_diff_value(val: Option<&Value>) -> String {
    match val {
        None => "—".to_owned(),
        Some(Value::Null) => "(deleted)".to_owned(),
        Some(Value::String(s)) => {
            if s.trim().is_empty() {
                "(empty)".to_owned()
            } else {
                s.clone()
            }
        }
        Some(Value::Object(map)) => {
            let mut lines = Vec::new();
            for key in [
                "title",
                "kind",
                "summary",
                "body",
                "confidence",
                "tags",
                "status",
            ] {
                if let Some(v) = map.get(key) {
                    if !v.is_null() {
                        lines.push(format!("{key}: {}", compact_val(v)));
                    }
                }
            }
            if lines.is_empty() {
                serde_json::to_string_pretty(map).unwrap_or_default()
            } else {
                lines.join("\n")
            }
        }
        Some(other) => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}
