use super::*;

/// What the whole project says of a box and the circles of files: a file is
/// part of one, a folder holds files of some, and of those some lead out of
/// the folder and back.
pub(in crate::graph_map) fn circle_told(circle: &V, folder: bool) -> Option<String> {
    let files = circle["files"].as_u64().filter(|files| *files > 0)?;
    Some(if !folder {
        format!(
            ", part of a cycle of {}",
            plural(circle["largest"].as_u64().unwrap_or(0), "file", "files")
        )
    } else if m::yes(&circle["leaves"]) {
        format!(
            ", holds {} of cycles that lead out of it and back",
            plural(files, "file", "files")
        )
    } else {
        format!(
            ", holds {} that use each other in cycles",
            plural(files, "file", "files")
        )
    })
}
/// The circles of files that touch what is open, the largest first. Each
/// is shown as the shortest cycle through the file that has most to do
/// with the others.
pub(in crate::graph_map) fn blast_side(view: &V) -> String {
    let blast = &view["focus"]["blast"];
    if !blast.is_object() {
        return String::new();
    }
    let number = |key: &str| blast[key].as_u64().unwrap_or(0);
    let (files, hops) = (number("files"), number("hops"));
    let mut html = format!(
        "<h4>A change here reaches <span class=\"graph-map-total\">{}</span></h4>",
        count(files)
    );
    if files == 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">Nothing else uses it within {}.</p>",
            plural(hops, "link", "links")
        ));
        return html;
    }
    // How many files each link further first reaches.
    let steps = m::array(&blast["by_hop"])
        .iter()
        .filter(|row| row["files"].as_u64().unwrap_or(0) > 0)
        .map(|row| {
            format!(
                "{} at {}",
                count(row["files"].as_u64().unwrap_or(0)),
                plural(row["hop"].as_u64().unwrap_or(0), "link", "links")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let tests = number("tests");
    html.push_str(&format!(
        "<p class=\"knowledge-note\">Files that use it, and what uses those, up to {} away: {steps}.{} Each link passes on less of a change than it receives: a type built on it and a call most, an import little, a guessed link less; past an interface the type implements, less again. The bar under a box says how much is left there.{}</p>",
        plural(hops, "link", "links"),
        match tests {
            0 => String::new(),
            1 => " 1 of them is a test.".to_owned(),
            tests => format!(" {} of them are tests.", count(tests)),
        },
        if m::yes(&blast["cut"]) {
            " The walk stopped before the end: there is more."
        } else {
            ""
        },
    ));
    html.push_str("<ul class=\"graph-map-rows\">");
    for row in m::array(&blast["listed"]) {
        let path = m::text(&row, "path");
        let folder = parent(&path);
        let hop = row["hop"].as_u64().unwrap_or(0);
        let left = (row["left"].as_f64().unwrap_or(0.0) * 100.0).round() as u64;
        let through = m::yes(&row["through"]);
        html.push_str(&format!(
            "<li><button type=\"button\" class=\"graph-map-row\" data-blast-hop=\"{hop}\" data-action=\"graph-map-open\" data-payload=\"{}\" title=\"{} · reached at {} by {}{} · select to open it\"><span class=\"graph-map-row-name\">{}</span><span class=\"graph-map-row-links\">{}{} · {left} %{}{}</span></button></li>",
            payload(&row["path"]),
            escape(&path),
            escape(&m::text(&row, "name")),
            escape(&m::text(&row, "via")),
            if through { " through an interface" } else { "" },
            escape(base(&path)),
            if folder.is_empty() {
                String::new()
            } else {
                format!("{} · ", escape(folder))
            },
            plural(hop, "link", "links"),
            if through { " · past an interface" } else { "" },
            if m::yes(&row["test"]) { " · test" } else { "" },
        ));
    }
    html.push_str("</ul>");
    let more = number("more");
    if more > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">and {} more</p>",
            count(more)
        ));
    }
    html
}
/// What a change to the selected place reaches of each box of a picture:
/// the fewest links to it and the most of the change left there. A file is
/// told by itself, a name by its file, a folder by the most reached file in
/// it. The selected box is told nothing.
pub(in crate::graph_map) fn blast_marks(view: &V, nodes: &[V]) -> Vec<Option<(u64, f64)>> {
    let marked = m::array(&view["focus"]["blast"]["marked"])
        .iter()
        .filter_map(|row| {
            Some((
                row[0].as_str()?.to_owned(),
                row[1].as_u64()?,
                row[2].as_f64()?,
            ))
        })
        .collect::<Vec<_>>();
    if marked.is_empty() {
        return vec![None; nodes.len()];
    }
    let focus = m::text(&view["focus"], "path");
    nodes
        .iter()
        .map(|node| {
            let path = m::text(node, "path");
            if path == focus {
                return None;
            }
            let folder = m::yes(&node["folder"]);
            let file = node["file"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| strip_symbol_suffix(&path).to_owned());
            marked
                .iter()
                .filter(|(reached, ..)| {
                    if folder {
                        reached
                            .strip_prefix(path.as_str())
                            .is_some_and(|rest| rest.starts_with('/'))
                    } else {
                        *reached == file
                    }
                })
                .fold(None, |best: Option<(u64, f64)>, (_, hop, left)| {
                    Some(best.map_or((*hop, *left), |(near, most)| {
                        (near.min(*hop), most.max(*left))
                    }))
                })
        })
        .collect()
}
pub(in crate::graph_map) fn circles_side(view: &V) -> String {
    let circles = &view["circles"];
    let listed = m::array(&circles["listed"]);
    if listed.is_empty() || cycles(view) {
        return String::new();
    }
    let number = |key: &str| circles[key].as_u64().unwrap_or(0);
    let (groups, here) = (number("groups"), number("here"));
    let mut html = format!(
        "<h4>Cycles of files <span class=\"graph-map-total\">{}</span></h4><p class=\"knowledge-note\">{}</p><ul class=\"graph-map-rows\">",
        count(here),
        if here == groups {
            format!(
                "{} of the project use each other in {}; the largest holds {}. A box marked ↻ is part of one, wherever the rest of it lies.",
                plural(number("files"), "file", "files"),
                plural(groups, "cycle", "cycles"),
                count(number("largest")),
            )
        } else {
            format!(
                "{} of the {} of the project touch what is open. A box marked ↻ is part of one, wherever the rest of it lies.",
                count(here),
                plural(groups, "cycle", "cycles"),
            )
        }
    );
    for circle in &listed {
        let start = m::text(circle, "start");
        let more = circle["more"].as_u64().unwrap_or(0);
        let names = m::array(&circle["names"])
            .iter()
            .filter_map(|name| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        html.push_str(&format!(
            "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-map-cycle\" data-payload=\"{}\" title=\"Show the shortest cycle through {}\"><span class=\"graph-map-row-name\">{}{}</span><span class=\"graph-map-row-links\">{}</span></button></li>",
            payload(&circle["start"]),
            escape(&start),
            escape(&names),
            if more > 0 {
                format!(", +{}", count(more))
            } else {
                String::new()
            },
            plural(circle["files"].as_u64().unwrap_or(0), "file", "files"),
        ));
    }
    html.push_str("</ul>");
    if here > listed.len() as u64 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">and {} more</p>",
            count(here - listed.len() as u64)
        ));
    }
    html.push_str(&cycle_cut_hints_html(view));
    html
}

/// Renders an optional confidence badge summarizing extracted vs inferred edge ratios.
pub(in crate::graph_map) fn confidence_pill_html(view: &V) -> String {
    let conf = if view["confidence"].is_object() {
        &view["confidence"]
    } else if view["ir_summary"]["confidence"].is_object() {
        &view["ir_summary"]["confidence"]
    } else {
        return String::new();
    };
    let extracted = conf["extracted"]
        .as_u64()
        .or_else(|| conf["extracted_links"].as_u64());
    let inferred = conf["inferred"]
        .as_u64()
        .or_else(|| conf["inferred_links"].as_u64());
    let ratio = conf["extracted_ratio"]
        .as_f64()
        .or_else(|| conf["ratio"].as_f64())
        .or_else(|| match (extracted, inferred) {
            (Some(e), Some(i)) if e + i > 0 => Some(e as f64 / (e + i) as f64),
            _ => None,
        });
    let Some(r) = ratio.filter(|v| v.is_finite()) else {
        return String::new();
    };
    let pct = (r.clamp(0.0, 1.0) * 100.0).round() as u64;
    let title = match (extracted, inferred) {
        (Some(e), Some(i)) => format!("{e} extracted links, {i} inferred links"),
        _ => format!("{pct}% extracted code confidence"),
    };
    format!(
        "<span class=\"graph-confidence-pill\" title=\"{}\">{pct}% extracted</span>",
        escape(&title),
    )
}

fn flatten_circle_remediation(raw: &V) -> Vec<V> {
    let mut out = Vec::new();
    for group in m::array(raw) {
        let cuts = m::array(&group["cuts"]);
        if !cuts.is_empty() {
            out.extend(cuts);
            continue;
        }
        let feedback = m::array(&group["feedback_arc_cut"]);
        if !feedback.is_empty() {
            out.extend(feedback);
            continue;
        }
        if group["from"].is_string() && group["to"].is_string() {
            out.push(group);
        }
    }
    out
}

fn extract_witness_chain(raw: &V) -> Vec<String> {
    if let Some(arr) = raw.as_array() {
        return arr
            .iter()
            .filter_map(|item| {
                item.as_str().map(str::to_owned).or_else(|| {
                    let from = item["from"].as_str().unwrap_or("");
                    let to = item["to"].as_str().unwrap_or("");
                    (!from.is_empty() && !to.is_empty()).then(|| format!("{from} -> {to}"))
                })
            })
            .collect();
    }
    if let Some(files) = raw["files"].as_array() {
        let chain: Vec<String> = files
            .iter()
            .filter_map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned))
            .collect();
        if !chain.is_empty() {
            return chain;
        }
    }
    if let Some(edges) = raw["edges"].as_array() {
        return edges
            .iter()
            .filter_map(|item| {
                let from = item["from"].as_str().unwrap_or("");
                let to = item["to"].as_str().unwrap_or("");
                (!from.is_empty() && !to.is_empty()).then(|| format!("{from} -> {to}"))
            })
            .collect();
    }
    Vec::new()
}

fn cut_hints_list_html(heading: &str, hints: &[V]) -> String {
    if hints.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "<div class=\"graph-cycle-cut-hint\"><h5>{}</h5><ul class=\"graph-map-rows\">",
        escape(heading),
    );
    for hint in hints.iter().take(6) {
        let from = m::text(hint, "from");
        let to = m::text(hint, "to");
        let note = ["hint", "reason", "remediation", "summary"]
            .into_iter()
            .map(|k| m::text(hint, k))
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        let cycles_broken = hint["cycles_broken"]
            .as_u64()
            .or_else(|| hint["breaks_cycles"].as_u64())
            .or_else(|| hint["score"].as_u64());
        let breaks_flag = hint["breaks_cycles"].as_bool().unwrap_or(false)
            || hint["breaks_cycle"].as_bool().unwrap_or(false);
        let links = hint["links"]
            .as_u64()
            .or_else(|| hint["weight"].as_u64())
            .filter(|n| Some(*n) != cycles_broken);
        let from_layer = m::text(hint, "from_layer");
        let to_layer = m::text(hint, "to_layer");
        let label = if !from.is_empty() && !to.is_empty() {
            format!("{} → {}", escape(base(&from)), escape(base(&to)))
        } else if !note.is_empty() {
            escape(&note)
        } else {
            continue;
        };
        let mut meta_parts: Vec<String> = Vec::new();
        if let Some(n) = cycles_broken {
            meta_parts.push(format!("breaks {n}"));
        } else if breaks_flag {
            meta_parts.push("breaks cycle".to_owned());
        }
        if let Some(l) = links {
            meta_parts.push(plural(l, "link", "links"));
        }
        if m::yes(&hint["guessed"]) {
            meta_parts.push("inferred".to_owned());
        }
        if !note.is_empty() && !from.is_empty() {
            meta_parts.push(escape(&note));
        } else if !from_layer.is_empty() && !to_layer.is_empty() {
            meta_parts.push(format!("{} → {}", escape(&from_layer), escape(&to_layer)));
        }
        let meta = meta_parts.join(" · ");
        if !from.is_empty() && !to.is_empty() {
            out.push_str(&format!(
                "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-map-link\" data-payload=\"{}\" title=\"{} → {}\"><span class=\"graph-map-row-name\">{label}</span><span class=\"graph-map-row-links\">{meta}</span></button></li>",
                payload(&json!({"from": from, "to": to})),
                escape(&from),
                escape(&to),
            ));
        } else {
            out.push_str(&format!(
                "<li><span class=\"graph-map-row-name\">{label}</span></li>"
            ));
        }
    }
    out.push_str("</ul></div>");
    out
}

/// Renders ranked feedback-edge-set cycle cut / remediation hints when supplied by the backend.
pub(in crate::graph_map) fn cycle_cut_hints_html(view: &V) -> String {
    let witness_chain: Vec<String> = [
        &view["cycle_witness"],
        &view["cycle"]["witness"],
        &view["circle_remediation"][0]["witness_cycle"],
    ]
    .into_iter()
    .map(extract_witness_chain)
    .find(|chain| !chain.is_empty())
    .unwrap_or_default();
    let hints = [
        &view["cycle"]["cut_hints"],
        &view["cycle"]["cuts"],
        &view["circles"]["cut_hints"],
    ]
    .into_iter()
    .map(m::array)
    .find(|arr| !arr.is_empty())
    .unwrap_or_else(|| {
        let from_circles = flatten_circle_remediation(&view["circle_remediation"]);
        if !from_circles.is_empty() {
            return from_circles;
        }
        [
            &view["pattern"]["remediation"],
            &view["architecture"]["cut_hints"],
            &view["architecture"]["remediation"],
        ]
        .into_iter()
        .map(m::array)
        .find(|arr| !arr.is_empty())
        .unwrap_or_default()
    });
    let mut html = cut_hints_list_html("Suggested cycle breaks", &hints);
    if !witness_chain.is_empty() {
        html.push_str("<div class=\"graph-cycle-witness\"><strong>Cycle witness</strong>: ");
        html.push_str(
            &witness_chain
                .iter()
                .map(|s| format!("<code>{}</code>", escape(s)))
                .collect::<Vec<_>>()
                .join(" → "),
        );
        html.push_str("</div>");
    }
    html
}

/// Renders architecture-specific remediation hints when supplied by the backend.
pub(in crate::graph_map) fn architecture_cut_hints_html(view: &V) -> String {
    let hints = [
        &view["architecture"]["remediation"],
        &view["architecture"]["cut_hints"],
    ]
    .into_iter()
    .map(m::array)
    .find(|arr| !arr.is_empty())
    .unwrap_or_default();
    cut_hints_list_html("Suggested layer breaks", &hints)
}

/// Renders pattern/layer remediation hints when supplied by the backend.
pub(in crate::graph_map) fn pattern_cut_hints_html(view: &V) -> String {
    let hints = [
        &view["pattern"]["remediation"],
        &view["pattern"]["cut_hints"],
    ]
    .into_iter()
    .map(m::array)
    .find(|arr| !arr.is_empty())
    .unwrap_or_default();
    cut_hints_list_html("Suggested layer breaks", &hints)
}

/// The rows of one side of a selected place. `look` names the selected
/// place and whether the rows are what it uses: each row then stands for a
/// link between two places, and a button beside it looks into that link.
pub(in crate::graph_map) fn rows(
    title: &str,
    rows: &V,
    more: &V,
    look: Option<(&V, bool)>,
) -> String {
    let rows = m::array(rows);
    let more = more.as_u64().unwrap_or(0);
    let mut html = format!(
        "<h4>{title} <span class=\"graph-map-total\">{}</span></h4>",
        count(rows.len() as u64 + more)
    );
    if rows.is_empty() {
        html.push_str("<p class=\"knowledge-note\">Nothing.</p>");
        return html;
    }
    html.push_str("<ul class=\"graph-map-rows\">");
    for row in &rows {
        // A symbol says which file it lies in where that is another than
        // the open one, and is reached by opening that file unless the
        // picture draws it, as among followed symbols.
        let home = row["file"].as_str().filter(|_| m::yes(&row["symbol"]));
        let elsewhere = home.filter(|_| !m::yes(&row["inside"]));
        let guessed = row["guessed"].as_u64().unwrap_or(0);
        // Two symbols are linked once, in one way; a place stands for many
        // links; what a type holds has a line.
        let told = match (row["relation"].as_str(), row["links"].as_u64()) {
            (Some(relation), _) => format!(
                "{}{}",
                escape(&relation.replace('_', " ")),
                if guessed > 0 { ", guessed" } else { "" }
            ),
            (None, Some(links)) => format!(
                "{}{}{}",
                weight(links, guessed),
                if m::yes(&row["against"]) {
                    " · against the order"
                } else {
                    ""
                },
                if m::yes(&row["opened"]) {
                    " · opened by the run"
                } else {
                    ""
                }
            ),
            _ => match row["line"].as_u64() {
                Some(line) if line > 0 => format!("line {line}"),
                _ => String::new(),
            },
        };
        let vis_pill = visibility_pill_html(row["visibility"].as_str(), true);
        let row_kind = resolved_member_kind(row);
        let kind_pill = if !vis_pill.is_empty()
            || matches!(
                row_kind.trim().to_ascii_lowercase().as_str(),
                "property" | "prop" | "field" | "const" | "constant" | "class_const"
            ) {
            member_kind_pill_html(row_kind)
        } else {
            String::new()
        };
        let ref_members = {
            let r = m::array(&row["referenced_members"]);
            if !r.is_empty() {
                r
            } else {
                let refs = m::array(&row["member_refs"]);
                if !refs.is_empty() {
                    refs
                } else {
                    m::array(&row["used_members"])
                }
            }
        };
        let decl_members = {
            let d = m::array(&row["declared_members"]);
            if !d.is_empty() {
                d
            } else {
                m::array(&row["target_members"])
            }
        };
        let row_warnings = m::array(&row["visibility_warnings"]);
        let members_details = impact_members_details_html(
            &ref_members,
            &decl_members,
            &row_warnings,
            MemberDetailsOpts {
                member_filter: "all",
                used_title: "Referenced members",
                exposed_title: "Exposed members",
                summary_title: "Properties &amp; methods",
                force_open: false,
            },
        );
        html.push_str(&format!(
            "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-map-go\" data-changed=\"{}\" data-payload=\"{}\" title=\"{}\"><span class=\"graph-map-row-name\">{vis_pill}{kind_pill}{}{}{}</span><span class=\"graph-map-row-links\">{}</span></button>{}{members_details}</li>",
            m::yes(&row["changed"]),
            payload(&json!({"path": row["path"], "inside": row["inside"], "open": elsewhere})),
            escape(&m::text(row, "path")),
            escape(&m::text(row, "name")),
            if m::yes(&row["folder"]) { "/" } else { "" },
            match home {
                Some(file) => format!(
                    "<span class=\"graph-map-row-file\">{}</span>",
                    escape(base(file))
                ),
                None => String::new(),
            },
            told,
            match look {
                Some((place, uses)) => {
                    let (from, to) = if uses {
                        (place, &row["path"])
                    } else {
                        (&row["path"], place)
                    };
                    format!(
                        "<button type=\"button\" class=\"graph-map-row-look\" data-action=\"graph-map-link\" data-payload=\"{}\" title=\"See what makes up this link\" aria-label=\"See what makes up the link with {}\">↗</button>",
                        payload(&json!({"from": from, "to": to})),
                        escape(&m::text(row, "name")),
                    )
                }
                _ => String::new(),
            },
        ));
    }
    html.push_str("</ul>");
    if more > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">and {} more</p>",
            count(more)
        ));
    }
    html
}
pub(in crate::graph_map) fn kinds(focus: &V) -> String {
    let kinds = m::array(&focus["kinds"])
        .iter()
        .map(|kind| {
            format!(
                "{} {}",
                escape(&m::text(kind, "kind").replace('_', " ")),
                count(kind["links"].as_u64().unwrap_or(0))
            )
        })
        .collect::<Vec<_>>();
    if kinds.is_empty() {
        String::new()
    } else {
        format!(
            "<p class=\"knowledge-note\">Uses by kind: {}</p>",
            kinds.join(" · ")
        )
    }
}
pub(in crate::graph_map) fn open_file(path: &V) -> String {
    format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"{}\">Open file</button>",
        payload(path)
    )
}
/// Follows the links of a folder, a file or a symbol step by step, wherever
/// they lead.
pub(in crate::graph_map) fn follow_links(path: &V) -> String {
    format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"{}\">Follow links</button>",
        payload(path)
    )
}
/// Asks for the shortest ways from a place to another, which is chosen
/// next: a box of any picture, or a place the search finds.
pub(in crate::graph_map) fn path_to(place: &V) -> String {
    // The keys stand as the alphabet has them, which is how a payload is
    // written whether or not the build's JSON keeps their order.
    format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{}\">Path to…</button>",
        payload(&json!({"name": place["name"], "path": place["path"]}))
    )
}
/// What a symbol is, with what the change did to it: to its own lines, and
/// to how many symbols inside it.
pub(in crate::graph_map) fn reached(symbol: &V, mut text: String) -> String {
    if let Some(status) = symbol["status"].as_str() {
        text.push_str(&format!(" · {} by the change", happened(status)));
    }
    match symbol["touched"].as_u64() {
        Some(inside) if inside > 0 => {
            text.push_str(&format!(" · {} changed inside", count(inside)));
        }
        _ => {}
    }
    text
}
pub(in crate::graph_map) fn symbol_focus_payload(member: &V, nodes: &[V]) -> Option<V> {
    let explicit_path = m::text(member, "path");
    if !explicit_path.is_empty() {
        return Some(json!({ "path": explicit_path, "inside": true }));
    }
    let mname = m::text(member, "name");
    let mline = member["line"].as_u64().unwrap_or(0);
    if mname.is_empty() {
        return None;
    }
    nodes.iter().find_map(|node| {
        if !m::yes(&node["symbol"]) {
            return None;
        }
        let nname = m::text(node, "name");
        let nline = node["line"].as_u64().unwrap_or(0);
        let name_matches = nname == mname
            || nname.ends_with(&format!("::{mname}"))
            || nname.ends_with(&format!(".{mname}"));
        let line_matches = mline > 0 && nline == mline;
        if name_matches || line_matches {
            Some(json!({ "path": node["path"], "inside": true }))
        } else {
            None
        }
    })
}
pub(in crate::graph_map) fn file_declared_members_html(members: &[V], view: &V) -> String {
    if members.is_empty() {
        return String::new();
    }
    let nodes = m::array(&view["nodes"]);
    let mut html = format!(
        "<details class=\"graph-impact-members\" open><summary class=\"graph-impact-members-summary\"><span class=\"graph-impact-members-label\">File properties &amp; methods ({})</span></summary><ul class=\"graph-member-list\">",
        members.len()
    );
    for member in members {
        let vis_pill = visibility_pill_html(member["visibility"].as_str(), false);
        let kind_pill = member_kind_pill_html(resolved_member_kind(member));
        let name = m::text(member, "name");
        let sig = m::text(member, "signature");
        let display_sig = if sig.is_empty() {
            name.clone()
        } else if !name.is_empty() && !sig.contains(&name) {
            format!("{name}{sig}")
        } else {
            sig.clone()
        };
        let title = if sig.is_empty() { &name } else { &sig };
        let line = member["line"]
            .as_u64()
            .filter(|&l| l > 0)
            .map(|l| format!("<span class=\"graph-member-line\">:{l}</span>"))
            .unwrap_or_default();
        if let Some(go_payload) = symbol_focus_payload(member, &nodes) {
            html.push_str(&format!(
                "<li><button type=\"button\" class=\"graph-map-row graph-member-row graph-member-row-btn\" data-action=\"graph-map-go\" data-payload=\"{}\" title=\"{}\">{vis_pill}{kind_pill}<span class=\"graph-member-sig\">{}</span>{line}</button></li>",
                payload(&go_payload),
                escape(title),
                escape(&display_sig),
            ));
        } else {
            html.push_str(&format!(
                "<li class=\"graph-member-row\">{vis_pill}{kind_pill}<span class=\"graph-member-sig\" title=\"{}\">{}</span>{line}</li>",
                escape(title),
                escape(&display_sig),
            ));
        }
    }
    html.push_str("</ul></details>");
    html
}
pub(in crate::graph_map) fn focus_member_meta_html(focus: &V) -> String {
    let vis_pill = visibility_pill_html(focus["visibility"].as_str(), false);
    let kind_str = resolved_member_kind(focus);
    let kind_pill = if !vis_pill.is_empty()
        || matches!(
            kind_str.trim().to_ascii_lowercase().as_str(),
            "property" | "prop" | "field" | "const" | "constant" | "class_const"
        ) {
        member_kind_pill_html(kind_str)
    } else {
        String::new()
    };
    let prev_vis = focus["previous_visibility"].as_str();
    let curr_vis = focus["visibility"].as_str();
    let vis_changed = m::yes(&focus["visibility_narrowed"])
        || m::yes(&focus["visibility_changed"])
        || (prev_vis.is_some() && curr_vis.is_some() && prev_vis != curr_vis);
    let vis_delta = if vis_changed {
        vis_delta_badge_html(prev_vis, curr_vis, m::yes(&focus["visibility_narrowed"]))
    } else {
        String::new()
    };
    if vis_pill.is_empty() && kind_pill.is_empty() && vis_delta.is_empty() {
        String::new()
    } else {
        format!("<div class=\"graph-member-meta\">{vis_pill}{kind_pill}{vis_delta}</div>")
    }
}
/// The side panel of an opened file: what the selected symbol uses and what
/// uses it, or which symbols of a file around are linked with this one.
pub(in crate::graph_map) fn file_side(view: &V) -> String {
    let focus = &view["focus"];
    if !focus.is_object() {
        let declared = m::array(&view["declared_members"]);
        let members_html = file_declared_members_html(&declared, view);
        return format!(
            "<p class=\"knowledge-note\">Select a class, a method or a function to see what it uses and what uses it; Follow links then walks on from it, a step at a time, into whatever files they lead to. A box without a letter is another file that this one uses or is used by: select it again to open it.</p>{members_html}{}",
            unopened(view)
        );
    }
    if m::yes(&focus["symbol"]) {
        let line = focus["line"].as_u64().unwrap_or(0);
        let mut html = format!(
            "<h3>{}</h3><p class=\"graph-map-path\">{}{}</p><p class=\"knowledge-note\">{}</p>{}",
            escape(&m::text(focus, "name")),
            escape(&m::text(focus, "file")),
            if line > 0 {
                format!(":{line}")
            } else {
                String::new()
            },
            escape(&reached(focus, capital(&what(focus)))),
            focus_member_meta_html(focus),
        );
        // What is linked with nothing has no links to follow, and no path
        // leads from it.
        if focus["out"].as_u64().unwrap_or(0) + focus["in"].as_u64().unwrap_or(0) > 0 {
            html.push_str(&follow_links(&focus["path"]));
            html.push_str(&path_to(focus));
        }
        if !m::array(&focus["members"]).is_empty() {
            html.push_str(&rows(
                "Members",
                &focus["members"],
                &focus["members_more"],
                None,
            ));
        }
        let ref_members = m::array(&focus["referenced_members"]);
        let decl_members = m::array(&focus["declared_members"]);
        let vis_warnings = m::array(&focus["visibility_warnings"]);
        html.push_str(&impact_members_details_html(
            &ref_members,
            &decl_members,
            &vis_warnings,
            MemberDetailsOpts {
                member_filter: "all",
                used_title: "Referenced members",
                exposed_title: "Declared members",
                summary_title: "Properties &amp; methods",
                force_open: false,
            },
        ));
        html.push_str(&rows("Uses", &focus["uses"], &focus["uses_more"], None));
        html.push_str(&rows(
            "Used by",
            &focus["used_by"],
            &focus["used_by_more"],
            None,
        ));
        html.push_str(&kinds(focus));
        return html;
    }
    let held = focus["symbols"].as_u64().unwrap_or(0);
    let mut html = format!(
        "<h3>{}</h3><p class=\"graph-map-path\">{}</p><p class=\"knowledge-note\">{}{} · {}</p>",
        escape(&m::text(focus, "name")),
        escape(&m::text(focus, "path")),
        match focus["status"].as_str() {
            Some(status) => format!("Changed file · {}", happened(status)),
            None => "Another file".to_owned(),
        },
        match focus["role"].as_str() {
            Some("uses") => " · uses this file",
            Some("used") => " · used by this file",
            _ => " · uses this file and is used by it",
        },
        plural(held, "symbol", "symbols"),
    );
    if held > 0 {
        html.push_str(&open_file(&focus["path"]));
    }
    html.push_str(&follow_links(&focus["path"]));
    html.push_str(&path_to(focus));
    let vis_warnings = m::array(&focus["visibility_warnings"]);
    html.push_str(&vis_warnings_html(&vis_warnings));
    let ref_members = m::array(&focus["referenced_members"]);
    let decl_members = m::array(&focus["declared_members"]);
    html.push_str(&impact_members_details_html(
        &ref_members,
        &decl_members,
        &[],
        MemberDetailsOpts {
            member_filter: "all",
            used_title: "Referenced members",
            exposed_title: "Declared members",
            summary_title: "Properties &amp; methods",
            force_open: !vis_warnings.is_empty(),
        },
    ));
    // Its own symbols at the other end of the links with this file.
    html.push_str(&rows(
        "Uses this file",
        &focus["uses"],
        &focus["uses_more"],
        None,
    ));
    html.push_str(&rows(
        "Used by this file",
        &focus["used_by"],
        &focus["used_by_more"],
        None,
    ));
    html.push_str(&kinds(focus));
    html
}
/// What a selected box of a path is to the path: its start or its end.
pub(in crate::graph_map) fn path_end(focus: &V) -> &'static str {
    match (m::yes(&focus["start"]), m::yes(&focus["end"])) {
        (true, _) => " · the path starts here",
        (_, true) => " · the path ends here",
        _ => "",
    }
}
/// What a selected box is to the picture it is selected in: where followed
/// links start, an end of a path, or, of two places that share something,
/// one of the two or what they share, with how many links each of them is
/// away.
pub(in crate::graph_map) fn role(view: &V, focus: &V) -> String {
    if cycles(view) {
        return if m::yes(&focus["start"]) {
            " · the cycle starts and ends here".to_owned()
        } else {
            String::new()
        };
    }
    if paths(view) {
        return path_end(focus).to_owned();
    }
    if !shares(view) {
        return if m::yes(&focus["start"]) {
            " · the links are followed from here".to_owned()
        } else {
            String::new()
        };
    }
    if m::yes(&focus["start"]) {
        return " · the first of the two places".to_owned();
    }
    if m::yes(&focus["end"]) {
        return " · the second of the two places".to_owned();
    }
    let shared = &view["shared"];
    let far = m::array(&focus["shares"]);
    let [first, second] = far.as_slice() else {
        return String::new();
    };
    let links = |far: &V| plural(far.as_u64().unwrap_or(0), "link", "links");
    let (told, joined) = if above(view) {
        ("uses both", "to")
    } else {
        ("both use it", "from")
    };
    format!(
        " · {told}: {} {joined} {}, {} {joined} {}",
        links(first),
        m::text(&shared["from"], "name"),
        links(second),
        m::text(&shared["to"], "name")
    )
}
/// The side panel of a symbol selected among followed symbols, on a path or
/// in what two places share: what it is and where it lies, with what `role`
/// says it is to the picture, the way to its file, to its own links and to
/// a path from it, and what it uses and what uses it, each with the file it
/// lies in. Where the boxes lie on ways (`ways`) those are the next and the
/// one before.
pub(in crate::graph_map) fn name_side(view: &V, focus: &V, ways: bool, role: &str) -> String {
    let line = focus["line"].as_u64().unwrap_or(0);
    let mut told = reached(focus, capital(&what(focus)));
    told.push_str(role);
    let mut html = format!(
        "<h3>{}</h3><p class=\"graph-map-path\">{}{}</p><p class=\"knowledge-note\">{}</p>{}",
        escape(&m::text(focus, "name")),
        escape(&m::text(focus, "file")),
        if line > 0 {
            format!(":{line}")
        } else {
            String::new()
        },
        escape(&told),
        focus_member_meta_html(focus),
    );
    if !at_base(view) {
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-go\" data-payload=\"{}\">Open in its file</button>",
            payload(&json!({"open": focus["file"], "path": focus["path"]})),
        ));
        // The symbol the links are followed from is followed already.
        if ways || !m::yes(&focus["start"]) {
            html.push_str(&follow_links(&focus["path"]));
        }
        html.push_str(&path_to(focus));
    }
    let (uses, used) = if ways { WAY } else { ("Uses", "Used by") };
    let look = |outgoing| at_base(view).then_some((&focus["path"], outgoing));
    html.push_str(&rows(uses, &focus["uses"], &focus["uses_more"], look(true)));
    html.push_str(&rows(
        used,
        &focus["used_by"],
        &focus["used_by_more"],
        look(false),
    ));
    let ref_members = m::array(&focus["referenced_members"]);
    let decl_members = m::array(&focus["declared_members"]);
    let vis_warnings = m::array(&focus["visibility_warnings"]);
    html.push_str(&impact_members_details_html(
        &ref_members,
        &decl_members,
        &vis_warnings,
        MemberDetailsOpts {
            member_filter: "all",
            used_title: "Referenced members",
            exposed_title: "Declared members",
            summary_title: "Properties &amp; methods",
            force_open: false,
        },
    ));
    html.push_str(&kinds(focus));
    html
}
/// Asks another question of the same two places: what they share (`true`),
/// one kind of it by its name, or the path between them again (`false`).
pub(in crate::graph_map) fn ask_shared(asked: V, class: &str) -> String {
    format!(
        "<button type=\"button\" class=\"{class}\" data-action=\"graph-map-shared\" data-payload=\"{}\">{}</button>",
        payload(&asked),
        match (asked.as_str(), asked.as_bool()) {
            (Some("uses_both"), _) => "What uses both",
            (Some(_), _) => "What both use",
            (None, Some(true)) => "What they share",
            _ => "The path between them",
        }
    )
}
/// The other questions that two places shown can be asked, as their
/// payloads: of a path, what the two share; of what they share, the other
/// kind where there is any of it, and the path between them. Two places of
/// which one lies in the other can be asked nothing more.
pub(in crate::graph_map) fn asks(view: &V) -> Vec<V> {
    if paths(view) {
        return if m::yes(&view["path"]["within"]) {
            vec![]
        } else {
            vec![json!(true)]
        };
    }
    let shared = &view["shared"];
    if !shares(view) || m::yes(&shared["within"]) {
        return vec![];
    }
    let other = if above(view) { "both_use" } else { "uses_both" };
    let mut asks = vec![];
    if shared[other].as_u64().unwrap_or(0) > 0 {
        asks.push(json!(other));
    }
    asks.push(json!(false));
    asks
}
/// Beside a picture of two places that draws nothing: a path, because no
/// chain of links joins its two ends; what the two share, because there is
/// nothing of the kind; or either, because one of the two lies in the
/// other. The other questions that can be asked of the two come first,
/// then each of the two with the way to its own links and to a path from
/// it to another place.
pub(in crate::graph_map) fn apart(view: &V) -> String {
    let asks = asks(view);
    let ends = if paths(view) {
        &view["path"]
    } else {
        &view["shared"]
    };
    let mut html = "<p class=\"knowledge-note\">Nothing is drawn.".to_owned();
    for ask in &asks {
        html.push_str(match (ask.as_str(), ask.as_bool()) {
            (Some("uses_both"), _) => " What uses both is another picture.",
            (Some(_), _) => " What both use is another picture.",
            (None, Some(true)) => " What they share shows what both use and what uses both.",
            _ => " The path between them asks how one of the two reaches the other.",
        });
    }
    html.push_str(" Follow links shows what each of the two reaches, and Path to… asks for the way from it to another place.</p>");
    for ask in asks {
        html.push_str(&ask_shared(ask, "button compact"));
    }
    for end in [&ends["from"], &ends["to"]] {
        // A symbol lies at a line of its file; a file or a folder is its path.
        let lies = if m::yes(&end["symbol"]) {
            let line = end["line"].as_u64().unwrap_or(0);
            format!(
                "{}{}",
                m::text(end, "file"),
                if line > 0 {
                    format!(":{line}")
                } else {
                    String::new()
                }
            )
        } else {
            m::text(end, "path")
        };
        html.push_str(&format!(
            "<h4>{}</h4><p class=\"graph-map-path\">{}</p>{}{}",
            escape(&m::text(end, "name")),
            escape(&lies),
            follow_links(&end["path"]),
            path_to(end)
        ));
    }
    html
}
/// Pairs of files of two layers of which the first needs the second: each a
/// row that opens what makes up their link. `layers` says of each pair which
/// layers its two files are filed in, where the pairs are not all of the
/// same two; `none` is said where there are no pairs.
pub(in crate::graph_map) fn pair_rows(title: &str, pairs: &V, none: &str, layers: bool) -> String {
    pair_rows_of(title, pairs, none, layers, false)
}
/// `gone` is for pairs a change took away: the link is no longer there to
/// look into, so such a row is no button.
pub(in crate::graph_map) fn pair_rows_of(
    title: &str,
    pairs: &V,
    none: &str,
    layers: bool,
    gone: bool,
) -> String {
    let rows = m::array(&pairs["rows"]);
    let more = pairs["more"].as_u64().unwrap_or(0);
    let mut html = format!(
        "<h4>{title} <span class=\"graph-map-total\">{}</span></h4>",
        count(rows.len() as u64 + more)
    );
    if rows.is_empty() {
        html.push_str(&format!("<p class=\"knowledge-note\">{none}</p>"));
        return html;
    }
    html.push_str("<ul class=\"graph-map-rows\">");
    for row in &rows {
        let (from, to) = (&row["from"], &row["to"]);
        let told = format!(
            "<span class=\"graph-map-row-name\">{} → {}</span><span class=\"graph-map-row-links\">{}{}{}</span>",
            escape(&m::text(from, "name")),
            escape(&m::text(to, "name")),
            if layers {
                format!(
                    "{} → {} · ",
                    escape(&m::text(from, "layer")),
                    escape(&m::text(to, "layer"))
                )
            } else {
                String::new()
            },
            weight(
                row["links"].as_u64().unwrap_or(0),
                row["guessed"].as_u64().unwrap_or(0)
            ),
            if m::yes(&row["added"]) {
                " · added by this change"
            } else {
                ""
            },
        );
        html.push_str(&if gone {
            format!(
                "<li><span class=\"graph-map-row\" data-pair=\"true\" data-gone=\"true\" title=\"{} used {}\">{told}</span></li>",
                escape(&m::text(from, "path")),
                escape(&m::text(to, "path")),
            )
        } else {
            format!(
                "<li><button type=\"button\" class=\"graph-map-row\" data-pair=\"true\" data-action=\"graph-map-link\" data-payload=\"{}\" title=\"{} uses {} · select to see what makes up this link\">{told}</button></li>",
                payload(&json!({"from": from["path"], "to": to["path"]})),
                escape(&m::text(from, "path")),
                escape(&m::text(to, "path")),
            )
        });
    }
    html.push_str("</ul>");
    if more > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">and {} more</p>",
            count(more)
        ));
    }
    html
}
/// The side panel of the layers of a folder: what runs against their order;
/// of a selected layer, what places a file in it, what it has with the other
/// layers, what of that runs against the order, and its files; or, of a
/// selected link between two layers, the pairs of files it is made of.
pub(in crate::graph_map) fn layers_side(view: &V) -> String {
    let declared = m::yes(&view["pattern"]["declared"]);
    let focus = &view["focus"];
    if !focus.is_object() {
        // What a chosen change did to the order comes first: it is what
        // the change is looked at for.
        let change = &view["pattern"]["change"];
        let did = if change["state"] == "ready" {
            let mut did = pair_rows(
                "Added by this change",
                &change["added"],
                "This change sets nothing against the order.",
                true,
            );
            if change["removed"]["pairs"].as_u64().unwrap_or(0) > 0 {
                did.push_str(&pair_rows_of(
                    "Gone with this change",
                    &change["removed"],
                    "",
                    true,
                    true,
                ));
            }
            did
        } else {
            String::new()
        };
        let intro = if declared {
            "Select a layer to see its files or a link to inspect its dependencies."
        } else {
            "A box is a layer: the files a name or a folder places in it. Select a layer to see its files and what it has with the others, or a link to see the pairs of files it is made of. Select it again to let go of it."
        };
        return format!(
            "<p class=\"knowledge-note\">{intro}</p>{did}{}{}",
            pair_rows(
                if declared {
                    "Rule violations"
                } else {
                    "Against the order"
                },
                &view["pattern"]["against"],
                if declared {
                    "No forbidden dependencies found."
                } else {
                    "No layer here needs one before it."
                },
                true
            ),
            pattern_cut_hints_html(view),
        );
    }
    if m::yes(&focus["link"]) {
        // The two layers are said once, above the pairs.
        return format!(
            "<h3>{} → {}</h3><p class=\"knowledge-note\">Link between two layers · {} · {}{}</p>{}",
            escape(&m::text(&focus["from"], "name")),
            escape(&m::text(&focus["to"], "name")),
            weight(
                focus["links"].as_u64().unwrap_or(0),
                focus["guessed"].as_u64().unwrap_or(0)
            ),
            plural(
                focus["pairs"].as_u64().unwrap_or(0),
                "pair of files",
                "pairs of files"
            ),
            if m::yes(&focus["against"]) {
                if declared {
                    " · forbidden by project rules"
                } else {
                    " · against the order"
                }
            } else {
                ""
            },
            pair_rows("Pairs of files", focus, "None.", false),
        );
    }
    let mut html = format!(
        "<h3>{}</h3><p class=\"knowledge-note\">Layer · {} · placed by {}</p>",
        escape(&m::text(focus, "name")),
        plural(focus["files"].as_u64().unwrap_or(0), "file", "files"),
        escape(&m::text(focus, "signs")),
    );
    // Each row is a link of the layer with another: looked into, it is
    // selected, and lists its pairs of files.
    let place = &focus["path"];
    html.push_str(&rows("Uses", &focus["uses"], &V::Null, Some((place, true))));
    html.push_str(&rows(
        "Used by",
        &focus["used_by"],
        &V::Null,
        Some((place, false)),
    ));
    html.push_str(&pair_rows(
        if declared {
            "Rule violations from this layer"
        } else {
            "Against the order, from here"
        },
        &focus["against"],
        if declared {
            "No forbidden dependencies found."
        } else {
            "Nothing here needs a layer before this one."
        },
        true,
    ));
    html.push_str(&rows(
        "Files",
        &focus["members"],
        &focus["members_more"],
        None,
    ));
    html
}
/// Structured summary card shown in the side panel for one focused pattern
/// (`architecture` or `design`), including its family, confidence badges, and
/// interactive role participant chips.
pub(in crate::graph_map) fn pattern_brief(view: &V) -> String {
    let pattern = &view["pattern"];
    if !pattern.is_object() {
        return String::new();
    }
    let family = match pattern["family"].as_str() {
        Some("architecture") => "architecture",
        _ => "design",
    };
    let family_label = if family == "architecture" {
        "Architecture pattern"
    } else {
        "Design pattern"
    };
    let mut badges = format!(
        "<span class=\"graph-map-family-pill\" data-family=\"{family}\">{family_label}</span>"
    );
    if m::yes(&pattern["named"]) {
        badges
            .push_str("<span class=\"graph-map-evidence-pill\" data-tone=\"named\">By name</span>");
    }
    if m::yes(&pattern["guessed"]) {
        badges.push_str("<span class=\"graph-map-evidence-pill\" data-tone=\"guessed\">Includes guessed links</span>");
    } else {
        badges.push_str("<span class=\"graph-map-evidence-pill\" data-tone=\"verified\">Verified from code</span>");
    }
    let mut html = format!(
        "<section class=\"graph-map-pattern-brief\" data-family=\"{family}\"><div class=\"graph-map-badges\">{badges}</div><h4>{} <span class=\"graph-map-total\">{}</span></h4>",
        escape(&m::text(pattern, "name")),
        escape(&m::text(&pattern["of"], "name")),
    );
    let tells = m::text(pattern, "tells");
    if !tells.is_empty() {
        html.push_str(&format!(
            "<p class=\"graph-map-pattern-tells\">{}</p>",
            escape(&tells)
        ));
    }
    let told = m::array(&pattern["told"]);
    if !told.is_empty() {
        html.push_str("<ul class=\"graph-map-told-list\">");
        for item in told {
            if let Some(text) = item.as_str().filter(|t| !t.is_empty()) {
                html.push_str(&format!("<li>{}</li>", escape(text)));
            }
        }
        html.push_str("</ul>");
    }
    let frames = m::array(&view["frames"]);
    let nodes = m::array(&view["nodes"]);
    if !frames.is_empty() && !nodes.is_empty() {
        html.push_str("<div class=\"graph-map-role-groups\">");
        for frame in frames {
            let holder_path = m::text(&frame, "path");
            let members: Vec<_> = nodes
                .iter()
                .filter(|node| m::text(node, "holder") == holder_path)
                .collect();
            if members.is_empty() {
                continue;
            }
            let anchor_frame = holder_path == "role:0";
            let (places, drawn) = (
                frame["places"].as_u64().unwrap_or(members.len() as u64),
                frame["drawn"].as_u64().unwrap_or(members.len() as u64),
            );
            let count_text = if drawn < places {
                format!("{} of {}", count(drawn), count(places))
            } else {
                count(places)
            };
            html.push_str(&format!(
                "<div class=\"graph-map-role-group\" data-anchor=\"{anchor_frame}\"><div class=\"graph-map-role-head\"><span>{}</span><span class=\"graph-map-total\">{count_text}</span></div><div class=\"graph-map-role-chips\">",
                escape(&m::text(&frame, "name")),
            ));
            for member in members {
                let start = m::yes(&member["start"]);
                html.push_str(&format!(
                    "<button type=\"button\" class=\"graph-map-part-chip\" data-action=\"graph-map-node\" data-payload=\"{}\" data-family=\"{family}\" data-anchor=\"{start}\" title=\"{}\">{}{}</button>",
                    payload(&member["path"]),
                    escape(&m::text(member, "file")),
                    if start { "★ " } else { "" },
                    escape(&m::text(member, "name")),
                ));
            }
            html.push_str("</div></div>");
        }
        html.push_str("</div>");
    }
    html.push_str("</section>");
    html
}
/// The side panel of one found pattern: the selected part with what it is
/// in the pattern and what joins it to the others, or what to do.
pub(in crate::graph_map) fn pattern_side(view: &V) -> String {
    if layered(view) {
        return layers_side(view);
    }
    let focus = &view["focus"];
    if !focus.is_object() {
        return format!(
            "<p class=\"knowledge-note\">A frame holds the parts that are one thing in the pattern. Select a part to see what joins it to the others, and select it again to let go of it. Select a link to see what makes it up.</p>{}{}",
            pattern_brief(view),
            pattern_cut_hints_html(view),
        );
    }
    name_side(
        view,
        focus,
        false,
        &format!(" · {} in this pattern", m::text(focus, "role")),
    )
}
/// Findings of one pattern a list shows before it is asked for all of them.
pub(in crate::graph_map) const FIRST: usize = 3;
/// One found pattern as a row that opens its picture. Beside a selected
/// place the row begins with the pattern, in a list of one pattern with the
/// type the finding is named after.
pub(in crate::graph_map) fn pattern_row(row: &V, whole: bool, family: Option<&str>) -> String {
    let line = row["line"].as_u64().unwrap_or(0);
    let mut told = m::text(row, "parts");
    for (key, tag) in [("named", "by name"), ("guessed", "guessed")] {
        if m::yes(&row[key]) {
            if !told.is_empty() {
                told.push_str(" · ");
            }
            told.push_str(tag);
        }
    }
    let family_attr = family.map_or_else(String::new, |fam| {
        format!(" data-family=\"{}\"", escape(fam))
    });
    let sort = m::text(row, "sort");
    let sort_attr = if sort.is_empty() {
        String::new()
    } else {
        format!(" data-sort=\"{}\"", escape(&sort))
    };
    format!(
        "<li><button type=\"button\" class=\"graph-map-row\"{family_attr}{sort_attr} data-action=\"graph-map-pattern\" data-payload=\"{}\" title=\"{}{}\"><span class=\"graph-map-row-name\">{}{}<span class=\"graph-map-row-file\">{}</span></span><span class=\"graph-map-row-links\">{}</span></button></li>",
        payload(&row["id"]),
        escape(&m::text(row, "file")),
        if line > 0 {
            format!(":{line}")
        } else {
            String::new()
        },
        if whole {
            format!("{}: ", escape(&m::text(row, "name")))
        } else {
            String::new()
        },
        escape(&m::text(row, "of")),
        escape(base(&m::text(row, "file"))),
        escape(&told),
    )
}
/// The patterns found where the picture is open, below the rest of the side
/// panel: those a selected place is part of, or, with nothing selected, all
/// of them, architecture before design, each pattern with its first few
/// findings. `open` names the patterns whose whole list is shown. Nothing
/// where patterns are not asked for.
pub fn found_patterns(view: &V, open: &[String]) -> String {
    let found = &view["patterns"];
    if !found.is_object() {
        return String::new();
    }
    let focus = &view["focus"];
    if focus.is_object() {
        let rows = m::array(&focus["patterns"]);
        let mut html = format!(
            "<h4>Patterns <span class=\"graph-map-total\">{}</span></h4>",
            count(rows.len() as u64)
        );
        if rows.is_empty() {
            html.push_str("<p class=\"knowledge-note\">No found pattern has a part here.</p>");
            return html;
        }
        html.push_str("<ul class=\"graph-map-rows\">");
        for row in &rows {
            let row_family = match row["sort"].as_str() {
                Some("layers" | "repository" | "events" | "commands") => Some("architecture"),
                Some(_) => Some("design"),
                None => None,
            };
            html.push_str(&pattern_row(row, true, row_family));
        }
        html.push_str("</ul>");
        return html;
    }
    // Beside followed links, a path, shared places or a cycle the side
    // panel is theirs: only a selected place says what it is part of.
    if follows(view) || paths(view) || shares(view) || cycles(view) {
        return String::new();
    }
    let kinds = m::array(&found["kinds"]);
    let mut html = format!(
        "<h4>Patterns found here <span class=\"graph-map-total\">{}</span></h4>",
        count(found["found"].as_u64().unwrap_or(0))
    );
    if kinds.is_empty() {
        html.push_str("<p class=\"knowledge-note\">None. A pattern is told by how types extend, implement, hold, take and make each other, and a few by a name.</p>");
        return html;
    }
    html.push_str("<p class=\"knowledge-note\">A box with ◆ holds a part of one. Select a finding to see its parts.</p>");
    for (family, title) in [("architecture", "Architecture"), ("design", "Design")] {
        let of = kinds
            .iter()
            .filter(|kind| kind["family"] == family)
            .collect::<Vec<_>>();
        if of.is_empty() {
            continue;
        }
        html.push_str(&format!(
            "<section class=\"graph-map-family-group\" data-family=\"{family}\"><p class=\"graph-map-family\">{title}</p>"
        ));
        for kind in of {
            let id = m::text(kind, "id");
            let tells = m::text(kind, "tells");
            let rows = m::array(&kind["instances"]);
            let all = open.contains(&id);
            let (found, more) = (
                kind["found"].as_u64().unwrap_or(0),
                kind["more"].as_u64().unwrap_or(0),
            );
            html.push_str(&format!(
                "<div class=\"graph-map-kind-card\" data-family=\"{family}\" data-kind=\"{}\"><p class=\"graph-map-pattern-name\" title=\"{}\">{} <span class=\"graph-map-total\">{}</span></p>",
                escape(&id),
                escape(&tells),
                escape(&m::text(kind, "name")),
                count(found),
            ));
            if !tells.is_empty() {
                html.push_str(&format!(
                    "<p class=\"graph-map-pattern-tells\">{}</p>",
                    escape(&tells),
                ));
            }
            html.push_str("<ul class=\"graph-map-rows\">");
            for row in rows.iter().take(if all { rows.len() } else { FIRST }) {
                html.push_str(&pattern_row(row, false, Some(family)));
            }
            html.push_str("</ul>");
            if rows.len() > FIRST {
                html.push_str(&format!(
                    "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-pattern-kind\" data-payload=\"{}\" aria-expanded=\"{all}\">{}</button>",
                    payload(&json!(id)),
                    if all {
                        format!("First {FIRST}")
                    } else {
                        format!("All {}", count(rows.len() as u64))
                    },
                ));
            }
            if all && more > 0 {
                html.push_str(&format!(
                    "<p class=\"knowledge-note\">and {} more: open a folder to see those of it.</p>",
                    count(more)
                ));
            }
            html.push_str("</div>");
        }
        html.push_str("</section>");
    }
    html
}
/// What the two lists beside a selected box of a path are called.
pub(in crate::graph_map) const WAY: (&str, &str) = ("Next on the way", "Before it on the way");
/// The side panel: what the selected place uses and what uses it.
pub fn side(view: &V) -> String {
    if symbols(view) {
        return file_side(view);
    }
    if patterned(view) {
        return pattern_side(view);
    }
    let focus = &view["focus"];
    let followed = follows(view);
    let pathed = paths(view);
    let shared = shares(view);
    // On a path and in what two places share a box lies on a way.
    let cycle = cycles(view);
    let ways = pathed || shared || cycle;
    if (followed || ways) && m::yes(&focus["symbol"]) {
        return name_side(view, focus, ways, &role(view, focus));
    }
    if cycle && !focus.is_object() {
        if let Some(places) = view["cycle"]["places"].as_array() {
            let mut html = String::from("<ol class=\"graph-map-cycle-places\">");
            for (at, place) in places.iter().enumerate() {
                let guessed = place["guessed"].as_u64().unwrap_or(0);
                let evidence = if guessed == 0 {
                    ""
                } else if place["links"] == guessed {
                    " · incoming link inferred"
                } else {
                    " · includes inferred links"
                };
                html.push_str(&format!(
                    "<li>{}{evidence}{}",
                    escape(&m::text(place, "path")),
                    if m::yes(&place["added"]) {
                        " · incoming link added in PR"
                    } else {
                        ""
                    },
                ));
                if at_base(view) && at > 0 {
                    html.push_str(&format!(
                        " <button type=\"button\" class=\"button compact\" data-action=\"graph-map-link\" data-payload=\"{}\">Inspect link</button>",
                        payload(&json!({"from": places[at - 1]["path"], "to": place["path"]})),
                    ));
                }
                html.push_str("</li>");
            }
            html.push_str("</ol>");
            html.push_str(&cycle_cut_hints_html(view));
            return html;
        }
        return String::new();
    }
    if !focus.is_object() && pathed {
        // A path that draws nothing leaves its two ends to go on from.
        if !m::yes(&view["path"]["found"]) {
            return apart(view);
        }
        return "<p class=\"knowledge-note\">Select a box to see what it leads to on the way and what leads to it. Select a link to see what makes it up. The path above the picture leads back; beside it, The other way asks for the ways from the second place to the first, and What they share for what both use and what uses both.</p>".to_owned();
    }
    if !focus.is_object() && shared {
        // So does a picture of what two places share with nothing in it.
        if m::array(&view["nodes"]).is_empty() {
            return apart(view);
        }
        return "<p class=\"knowledge-note\">Select a shared place to highlight both paths. Select a link to see what makes it up.</p>".to_owned();
    }
    if !focus.is_object() {
        return if followed {
            "<p class=\"knowledge-note\">Select a box to see what it uses and what uses it. + at the right corner of a box shows more of what it uses, + at the left corner more of what uses it, and − folds them back. Select a link to see what makes it up.</p>"
        } else if changes(view) {
            // A run says first what it left unopened of what uses its change.
            return format!(
                "<p class=\"knowledge-note\">Select a file to see what it uses and what uses it. The highlighted ones changed; the others use them or are used by them. Select a file again to open it to its classes, methods and functions. Select a link to see what makes it up.</p>{}",
                unopened(view)
            );
        } else {
            "<p class=\"knowledge-note\">Select a folder or a file to see what it uses and what uses it. Select it again to open it: a folder to what it holds, a file to its classes, methods and functions. + at the corner of a folder opens it here, among the others. Select a link to see what makes it up.</p>"
        }
        .to_owned();
    }
    let folder = m::yes(&focus["folder"]);
    let changed = focus["changed"].as_u64().unwrap_or(0);
    let held = focus["symbols"].as_u64().unwrap_or(0);
    // Beside a change, a file's links are the ones it has with the change.
    let beside = changes(view) && changed == 0;
    // What the place is, and what it is to the picture: that may name the
    // two places that share it, so it is text like the name above it.
    let told = if changes(view) {
        match (changed, focus["status"].as_str(), focus["role"].as_str()) {
            (0, _, role) => format!(
                "{} · {}",
                if m::yes(&focus["altered"]) {
                    "Changed too, outside the open folder"
                } else {
                    "Not changed"
                },
                match role {
                    Some("affected") => "uses the change",
                    Some("needed") => "used by the change",
                    _ => "uses the change and is used by it",
                }
            ),
            (_, Some(status), _) => format!("Changed file · {}", happened(status)),
            _ => format!(
                "Folder · {}",
                plural(changed, "changed file", "changed files")
            ),
        }
    } else if followed || ways {
        // Among followed links, on a path and in what two places share
        // a place may be one of a chosen change.
        let mut told = if folder {
            format!(
                "Folder · {}",
                plural(focus["files"].as_u64().unwrap_or(0), "file", "files")
            )
        } else if held > 0 {
            format!("File · {}", plural(held, "symbol", "symbols"))
        } else {
            "File".to_owned()
        };
        match (changed, focus["status"].as_str()) {
            (0, _) => {}
            (_, Some(status)) => told.push_str(&format!(" · {}", happened(status))),
            _ => told.push_str(&format!(
                " · {} inside",
                plural(changed, "changed file", "changed files")
            )),
        }
        told.push_str(&role(view, focus));
        told
    } else if folder {
        format!(
            "Folder · {} · {} inside it",
            plural(focus["files"].as_u64().unwrap_or(0), "file", "files"),
            plural(focus["within"].as_u64().unwrap_or(0), "link", "links")
        )
    } else if held > 0 {
        format!("File · {}", plural(held, "symbol", "symbols"))
    } else {
        "File".to_owned()
    };
    let mut html = format!(
        "<h3>{}</h3><p class=\"graph-map-path\">{}</p><p class=\"knowledge-note\">{}</p>",
        escape(&m::text(focus, "name")),
        escape(&m::text(focus, "path")),
        escape(&told),
    );
    if !at_base(view) && folder {
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"{}\">Open folder</button>",
            payload(&focus["path"])
        ));
        if spreads(view) && m::yes(&focus["inside"]) {
            html.push_str(&format!(
                "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-spread\" data-payload=\"{}\">Open here</button>",
                payload(&focus["path"])
            ));
        }
    } else if !at_base(view) && held > 0 {
        html.push_str(&open_file(&focus["path"]));
    }
    // The place the links are followed from is followed already.
    if !at_base(view) {
        if ways || !m::yes(&focus["start"]) {
            html.push_str(&follow_links(&focus["path"]));
        }
        html.push_str(&path_to(focus));
    }
    let (uses, used) = if ways {
        WAY
    } else if beside {
        ("Uses in the change", "Used by the change")
    } else {
        ("Uses", "Used by")
    };
    // Each row is a link of the selected place, and can be looked into.
    let place = &focus["path"];
    html.push_str(&rows(
        uses,
        &focus["uses"],
        &focus["uses_more"],
        Some((place, true)),
    ));
    html.push_str(&rows(
        used,
        &focus["used_by"],
        &focus["used_by_more"],
        Some((place, false)),
    ));
    html.push_str(&kinds(focus));
    html
}
pub(in crate::graph_map) fn crumbs(view: &V) -> String {
    let crumbs = m::array(&view["crumbs"]);
    let last = crumbs.len().saturating_sub(1);
    let mut html = crumbs
        .iter()
        .enumerate()
        .map(|(index, crumb)| {
            if index == last {
                format!(
                    "<span class=\"graph-map-crumb\" aria-current=\"location\">{}{}</span>",
                    escape(&m::text(crumb, "name")),
                    if follows(view) {
                        ": links"
                    } else if paths(view) {
                        ": path"
                    } else if cycles(view) {
                        ": cycle"
                    } else if !shares(view) {
                        ""
                    } else if above(view) {
                        ": what uses both"
                    } else {
                        ": what both use"
                    }
                )
            } else if at_base(view) {
                format!("<span class=\"graph-map-crumb\">{}</span><span aria-hidden=\"true\">/</span>", escape(&m::text(crumb, "name")))
            } else {
                format!(
                    "<button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-open\" data-payload=\"{}\">{}</button><span aria-hidden=\"true\">/</span>",
                    payload(&crumb["path"]),
                    escape(&m::text(crumb, "name"))
                )
            }
        })
        .collect::<String>();
    // A path that was found can be asked for the other way round, whatever
    // is selected in it: two places of a cycle lead to each other.
    if paths(view) && m::yes(&view["path"]["found"]) {
        html.push_str("<span aria-hidden=\"true\">·</span><button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-path-swap\">The other way</button>");
    }
    // Two places that lie apart can be asked what they share, whether or
    // not a path joins them, and from there for the other kind of it and
    // for the path again.
    for ask in asks(view) {
        html.push_str("<span aria-hidden=\"true\">·</span>");
        html.push_str(&ask_shared(ask, "graph-map-crumb"));
    }
    html
}
pub(in crate::graph_map) fn html(id: &str, markup: &str) {
    if let Some(element) = dom::get(id) {
        element.set_inner_html(markup);
    }
}
pub(in crate::graph_map) const LEGEND: &str =
    "Dotted: against the main direction, closing a cycle (↻).";
pub(in crate::graph_map) const FILE_LEGEND: &str = "T type · m method · f function · P property · K const · c case · # heading. A box without a letter is another file, a plain name, or the open file itself for its imports.";
pub(in crate::graph_map) const NAME_LEGEND: &str = "T type · m method · f function · P property · K const · c case · # heading. A box without a letter is a plain name, or a file itself for its imports.";
pub(in crate::graph_map) const LAYER_LEGEND: &str = "A box is a layer; the layers stand in their order, and each may use those after it. A link to the next layer runs straight, one that passes a layer runs over the row. A link under the row, in red, runs back: against the order.";
/// What the colours and the lines of the map mean, each colour beside a
/// patch of it, then what is particular to the picture: a folder, an opened
/// file, or a change (`change` says a change is chosen).
pub(in crate::graph_map) fn legend(view: &V, change: bool) -> String {
    let patch = |side: &str| {
        format!("<span class=\"graph-map-key graph-map-key-{side}\" aria-hidden=\"true\"></span>")
    };
    // The mark of what a run opened, where a run is shown.
    let run = if view["run"].is_object() {
        " ◉: opened by the run; on a folder, how many of its files."
    } else {
        ""
    };
    let particular = match (symbols(view), change) {
        (true, true) => format!("{FILE_LEGEND} Highlighted: touched by the change.{run}"),
        (true, false) => FILE_LEGEND.to_owned(),
        _ if changes(view) => {
            format!("Highlighted: changed. The rest use the change or are used by it.{run}")
        }
        _ if layered(view) => {
            let mut text = if m::yes(&view["pattern"]["declared"]) {
                "A box is a declared layer. A red arrow is a dependency forbidden by the project rules.".to_owned()
            } else {
                LAYER_LEGEND.to_owned()
            };
            if m::array(&view["edges"])
                .iter()
                .any(|edge| edge["added"].is_u64())
            {
                text.push_str(" The number beside one counts the links this change added.");
            }
            text
        }
        _ if patterned(view) => format!(
            "A frame holds the parts that are one thing in the pattern; the ringed box is the type it is named after. {NAME_LEGEND}"
        ),
        _ if cycles(view) => {
            let mut text = format!("{LEGEND} The ringed box is where the cycle starts and ends.");
            if m::array(&view["edges"])
                .iter()
                .any(|edge| m::yes(&edge["added"]))
            {
                text.push_str(" Green links were added in this PR.");
            }
            if names(view) {
                text.push(' ');
                text.push_str(NAME_LEGEND);
            }
            text
        }
        // What two places share can use each other, which a path cannot.
        _ if paths(view) || shares(view) => format!(
            "{}{}{}",
            if !shares(view) {
                "The ringed boxes are the two ends of the path.".to_owned()
            } else if above(view) {
                format!(
                    "{LEGEND} The ringed boxes are the two places. A name in bold is what uses both; a box with a plain name lies on its way to them."
                )
            } else {
                format!(
                    "{LEGEND} The ringed boxes are the two places. A name in bold is what both use; a box with a plain name lies on the way to it."
                )
            },
            if names(view) {
                format!(" {NAME_LEGEND}")
            } else {
                String::new()
            },
            match (change, names(view)) {
                (true, true) => " Highlighted: touched by the change.",
                (true, false) => " Highlighted: changed.",
                _ => "",
            }
        ),
        _ if names(view) => format!(
            "{LEGEND} + and − at the corners of a box unfold and fold what it uses, on the right, and what uses it, on the left. {NAME_LEGEND}{}",
            if change {
                " Highlighted: touched by the change."
            } else {
                ""
            }
        ),
        _ if follows(view) => format!(
            "{LEGEND} + and − at the corners of a box unfold and fold what it uses, on the right, and what uses it, on the left.{}",
            if change { " Highlighted: changed." } else { "" }
        ),
        _ if !m::array(&view["frames"]).is_empty() => {
            format!("{LEGEND} A frame is a folder opened here: − at its corner closes it.")
        }
        _ => LEGEND.to_owned(),
    };
    let diff_key = if change || changes(view) {
        format!(
            " Diff: {} Added/Created, {} Modified, {} Deleted, {} Renamed, {} Caller risk, {} Touched, {} Added / {} Removed edges.",
            patch("diff-added"),
            patch("diff-modified"),
            patch("diff-removed"),
            patch("diff-renamed"),
            patch("diff-risk"),
            patch("diff-touched"),
            patch("diff-added"),
            patch("diff-removed"),
        )
    } else {
        String::new()
    };
    format!(
        "An arrow runs from what uses {} to what is used {}, and a box is tinted by which it does more here. Its head and its dashes tell its kind, as the key of the links shows. Hollow: only guessed by name. Thicker: more links.{diff_key} {particular}{}",
        patch("uses"),
        patch("used"),
        if view["patterns"].is_object() {
            " ◆: a part of a found pattern, or a place that holds one."
        } else {
            ""
        },
    )
}
