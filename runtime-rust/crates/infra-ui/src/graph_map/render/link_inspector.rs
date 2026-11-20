use super::*;

/// The handles at the top corners of a box whose links are followed. The
/// right corner is for what the box uses and the left one for what uses it,
/// as the links leave on the right and arrive on the left. A side that is
/// unfolded has − to fold it back; a side with places not drawn has + with
/// how many, which unfolds the next of them.
pub(in crate::graph_map) fn handles(node: &V, at: usize, x: f64, y: f64, dim: bool) -> String {
    let name = escape(&m::text(node, "name"));
    let mut html = String::new();
    for (side, corner, outward, hang) in [
        ("out", x + BOX_WIDTH, 1.0, "after"),
        ("in", x, -1.0, "before"),
    ] {
        let what = if side == "out" {
            format!("what {name} uses")
        } else {
            format!("what uses {name}")
        };
        let more = node[format!("more_{side}").as_str()].as_u64().unwrap_or(0);
        let unfolded = m::yes(&node[format!("unfolded_{side}").as_str()]);
        let asked = payload(&json!({"path": node["path"], "side": side}));
        if unfolded {
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-more\" data-of=\"{at}\" data-side=\"{side}\" data-dim=\"{dim}\" data-action=\"graph-map-fold\" data-payload=\"{asked}\" style=\"left:{corner:.0}px;top:{y:.0}px\" title=\"Hide {what}\" aria-label=\"Hide {what}\">−</button>"
            ));
        }
        if more > 0 {
            // Beside a − the + hangs outside the corner, clear of the box.
            let (btn_x, hang) = if unfolded {
                (corner + outward * HANDLE_GAP, hang)
            } else {
                (corner, "")
            };
            let told = if unfolded {
                format!("Show more of {what}: {} not drawn", count(more))
            } else {
                format!("Show {what}: {} not drawn", count(more))
            };
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-more\" data-of=\"{at}\" data-side=\"{side}\" data-hang=\"{hang}\" data-dim=\"{dim}\" data-action=\"graph-map-unfold\" data-payload=\"{asked}\" style=\"left:{btn_x:.0}px;top:{y:.0}px\" title=\"{told}\" aria-label=\"{told}\">+{}</button>",
                count(more)
            ));
        }
    }
    html
}
/// Where the names of a looked-into link stand: what uses in a column on
/// the left, what is used in a column on the right, each file a frame around
/// its names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ends {
    /// Where each name stands, in the order the names were given: those
    /// that use, file by file, then those that are used.
    pub names: Vec<(f64, f64)>,
    /// The frame of each file, in the same order.
    pub files: Vec<Framed>,
    /// The curve of each link.
    pub wires: Vec<String>,
    pub width: f64,
    pub height: f64,
}
/// Lays out the two ends of a link. `users` and `used` say how many names
/// each file of an end holds; a link names its two names by their numbers
/// among all of them, those that use first. Files and the names in them are
/// ordered so that the links cross as little as a few rounds can make them.
/// Both ends begin at the top, so a long end does not push the short one
/// out of sight.
pub fn ends(users: &[usize], used: &[usize], links: &[(usize, usize)]) -> Ends {
    let left = users.iter().sum::<usize>();
    let count = left + used.iter().sum::<usize>();
    let mut file = Vec::with_capacity(count);
    for (group, names) in users.iter().chain(used).enumerate() {
        file.extend(std::iter::repeat_n(group, *names));
    }
    let mut near = vec![vec![]; count];
    for (from, to) in links {
        if *from < left && (left..count).contains(to) {
            near[*from].push(*to);
            near[*to].push(*from);
        }
    }
    let mut order = [(0..left).collect::<Vec<_>>(), (left..count).collect()];
    let mut place = (0..count)
        .map(|name| (if name < left { name } else { name - left }) as f64)
        .collect::<Vec<_>>();
    // Each name moves towards the average place of what it is linked with
    // at the other end, and a file towards that of its names.
    for round in 0..6 {
        let side = 1 - round % 2;
        let mut keyed = order[side]
            .iter()
            .map(|name| {
                let linked = &near[*name];
                let middle = if linked.is_empty() {
                    place[*name]
                } else {
                    linked.iter().map(|other| place[*other]).sum::<f64>() / linked.len() as f64
                };
                (middle, place[*name], *name)
            })
            .collect::<Vec<_>>();
        let mut middles: BTreeMap<usize, (f64, f64)> = BTreeMap::new();
        for (middle, _, name) in &keyed {
            let of_file = middles.entry(file[*name]).or_insert((0.0, 0.0));
            of_file.0 += middle;
            of_file.1 += 1.0;
        }
        let middle = |name: usize| middles[&file[name]].0 / middles[&file[name]].1;
        keyed.sort_by(|a, b| {
            middle(a.2)
                .total_cmp(&middle(b.2))
                .then(file[a.2].cmp(&file[b.2]))
                .then(a.0.total_cmp(&b.0))
                .then(a.1.total_cmp(&b.1))
        });
        order[side] = keyed.into_iter().map(|(.., name)| name).collect();
        for (index, name) in order[side].iter().enumerate() {
            place[*name] = index as f64;
        }
    }
    let mut names = vec![(0.0, 0.0); count];
    let mut files = vec![Framed::default(); users.len() + used.len()];
    let mut tall = [0.0f64; 2];
    for (side, order) in order.iter().enumerate() {
        let x = MARGIN + FRAME_PAD + side as f64 * (LINK_BOX + LINK_GAP);
        let mut y = MARGIN;
        let mut open: Option<usize> = None;
        for name in order {
            let of = file[*name];
            if open == Some(of) {
                y += LINK_ROW;
            } else {
                // The frame before ends under its last name; this one
                // begins with the room for the name of its file.
                if open.is_some() {
                    y += FRAME_PAD + LINK_BETWEEN;
                }
                open = Some(of);
                files[of] = Framed {
                    x: x - FRAME_PAD,
                    y,
                    width: LINK_BOX + 2.0 * FRAME_PAD,
                    height: 0.0,
                };
                y += FRAME_HEAD;
            }
            names[*name] = (x, y);
            y += BOX_HEIGHT;
            files[of].height = y + FRAME_PAD - files[of].y;
        }
        if open.is_some() {
            y += FRAME_PAD;
        }
        tall[side] = y;
    }
    let tallest = tall[0].max(tall[1]);
    let wires = links
        .iter()
        .map(|(from, to)| {
            if *from >= left || !(left..count).contains(to) {
                return String::new();
            }
            let start = (names[*from].0 + LINK_BOX, names[*from].1 + BOX_HEIGHT / 2.0);
            let end = (names[*to].0, names[*to].1 + BOX_HEIGHT / 2.0);
            let mut path = format!("M{:.1} {:.1}", start.0, start.1);
            curve(&mut path, start, end);
            path
        })
        .collect();
    Ends {
        names,
        files,
        wires,
        width: 2.0 * (MARGIN + FRAME_PAD + LINK_BOX) + LINK_GAP,
        height: (tallest + MARGIN).max(BOX_HEIGHT + 2.0 * MARGIN),
    }
}
/// The names of a looked-into link in one list, those that use first, each
/// with the file it lies in; and how many names each file of each end holds.
pub(in crate::graph_map) fn link_names(view: &V) -> (Vec<(V, V)>, [Vec<usize>; 2]) {
    let mut names = vec![];
    let mut held = [vec![], vec![]];
    for (side, end) in ["users", "used"].iter().enumerate() {
        for mut group in m::array(&view[*end]["groups"]) {
            let of_group = m::array(&group["names"]);
            held[side].push(of_group.len());
            // A name carries its file, not the file's other names.
            group["names"] = V::Null;
            names.extend(of_group.into_iter().map(|name| (name, group.clone())));
        }
    }
    (names, held)
}
#[derive(Default)]
pub(in crate::graph_map) struct LinkDiffInfo {
    pub(in crate::graph_map) pair_diff: Option<&'static str>,
    pub(in crate::graph_map) symbol_diffs: BTreeMap<String, &'static str>,
    pub(in crate::graph_map) symbol_risks: BTreeMap<String, String>,
    pub(in crate::graph_map) warnings: Vec<V>,
}

pub(in crate::graph_map) fn paths_overlap(a: &str, b: &str) -> bool {
    let na = normalize_rel_path(strip_symbol_suffix(a));
    let nb = normalize_rel_path(strip_symbol_suffix(b));
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    na == nb || nb.starts_with(&format!("{na}/")) || na.starts_with(&format!("{nb}/"))
}

pub(in crate::graph_map) fn merge_diff_priority(
    current: Option<&'static str>,
    next: &'static str,
) -> &'static str {
    match (current, next) {
        (Some("added"), _) | (_, "added") => "added",
        (Some("removed"), _) | (_, "removed") => "removed",
        _ => "unchanged",
    }
}

pub(in crate::graph_map) fn norm_sym_token(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('.')
        .trim_end_matches("()")
        .to_ascii_lowercase()
}

impl LinkDiffInfo {
    pub(in crate::graph_map) fn record_member_ref(
        &mut self,
        mref: &V,
        fallback_diff: Option<&'static str>,
    ) {
        let tok = norm_sym_token(&m::text(mref, "name"));
        if tok.is_empty() {
            return;
        }
        let change = m::text(mref, "change").trim().to_ascii_lowercase();
        let mapped = match change.as_str() {
            "added" => Some("added"),
            "removed" => Some("removed"),
            "modified" | "visibility_changed" | "unchanged" => Some("unchanged"),
            _ => fallback_diff,
        };
        if let Some(diff) = mapped {
            let merged = merge_diff_priority(self.symbol_diffs.get(&tok).copied(), diff);
            self.symbol_diffs.insert(tok.clone(), merged);
        }
        let risk = m::text(mref, "access_risk").trim().to_owned();
        if !risk.is_empty() {
            self.symbol_risks.insert(tok, risk);
        } else if m::yes(&mref["visibility_narrowed"]) {
            self.symbol_risks
                .insert(tok, "visibility_narrowed".to_owned());
        }
    }

    pub(in crate::graph_map) fn record_syms(&mut self, syms: &V, diff: &'static str) {
        for sym in m::array(syms) {
            if let Some(name) = sym.as_str().map(norm_sym_token).filter(|s| !s.is_empty()) {
                let merged = merge_diff_priority(self.symbol_diffs.get(&name).copied(), diff);
                self.symbol_diffs.insert(name, merged);
            }
        }
    }

    pub(in crate::graph_map) fn record_warning(&mut self, w: V) {
        let tok = norm_sym_token(&m::text(&w, "member"));
        if !tok.is_empty() {
            let reason = m::text(&w, "reason");
            self.symbol_risks.insert(
                tok,
                if reason.is_empty() {
                    "visibility risk".to_owned()
                } else {
                    reason
                },
            );
        }
        self.warnings.push(w);
    }

    pub(in crate::graph_map) fn from_view(link_view: &V, impact: Option<&V>) -> Self {
        let mut info = Self::default();
        for w in m::array(&link_view["visibility_warnings"]) {
            info.record_warning(w);
        }
        for mref in m::array(&link_view["member_refs"]) {
            info.record_member_ref(&mref, None);
        }
        let Some(impact) = impact.filter(|v| v.is_object()) else {
            return info;
        };
        let impact_path = normalize_rel_path(strip_symbol_suffix(&m::text(impact, "path")));
        if impact_path.is_empty() {
            return info;
        }
        let from_path = normalize_rel_path(&m::text(&link_view["from"], "path"));
        let to_path = normalize_rel_path(&m::text(&link_view["to"], "path"));
        if paths_overlap(&from_path, &impact_path) {
            for (diff, list) in [
                ("added", &impact["uses"]["added"]),
                ("removed", &impact["uses"]["removed"]),
                ("unchanged", &impact["uses"]["unchanged"]),
            ] {
                for dep in m::array(list) {
                    let target = m::text(&dep, "target");
                    if paths_overlap(&to_path, &target) {
                        info.pair_diff = Some(merge_diff_priority(info.pair_diff, diff));
                        info.record_syms(&dep["symbols"], diff);
                        for mref in m::array(&dep["member_refs"]) {
                            info.record_member_ref(&mref, Some(diff));
                        }
                        for w in m::array(&dep["visibility_warnings"]) {
                            info.record_warning(w);
                        }
                    }
                }
            }
        }
        if paths_overlap(&to_path, &impact_path) {
            for caller in m::array(&impact["used_by"]) {
                let caller_path = m::text(&caller, "path");
                if paths_overlap(&from_path, &caller_path) {
                    let diff = match m::text(&caller, "impact").as_str() {
                        "broken_risk" => "removed",
                        "added_caller" => "added",
                        _ => "unchanged",
                    };
                    info.pair_diff = Some(merge_diff_priority(info.pair_diff, diff));
                    info.record_syms(&caller["changed_symbols"], diff);
                    info.record_syms(&caller["symbols"], diff);
                    for mref in m::array(&caller["used_members"]) {
                        info.record_member_ref(&mref, Some(diff));
                    }
                    for w in m::array(&caller["visibility_warnings"]) {
                        info.record_warning(w);
                    }
                }
            }
        }
        info
    }

    pub(in crate::graph_map) fn diff_for_names(
        &self,
        from_name: &str,
        to_name: &str,
    ) -> Option<&'static str> {
        for candidate in [norm_sym_token(from_name), norm_sym_token(to_name)] {
            if let Some(diff) = self.symbol_diffs.get(&candidate) {
                return Some(diff);
            }
        }
        self.pair_diff
    }

    pub(in crate::graph_map) fn risk_for_names(
        &self,
        from_name: &str,
        to_name: &str,
    ) -> Option<&str> {
        for candidate in [norm_sym_token(to_name), norm_sym_token(from_name)] {
            if let Some(risk) = self.symbol_risks.get(&candidate) {
                return Some(risk.as_str());
            }
        }
        None
    }
}

pub(in crate::graph_map) fn diff_badge_sign(diff: &str) -> &'static str {
    match diff {
        "added" => "+",
        "removed" => "-",
        _ => "=",
    }
}

/// The picture of one link, looked into: the names that use on the left,
/// the names that are used on the right, each in the frame of its file, and
/// every link between two of them. `chosen` is the name whose links are lit.
#[cfg(test)]
pub fn link_canvas(view: &V, chosen: &str) -> String {
    link_canvas_with(view, chosen, None, &[false; 7])
}
/// Renders the link detail canvas with optional change-impact diff annotations (`+`/`-`/`=`).
/// A link of a kind the key has switched off (`off`) keeps its place and is not drawn.
pub fn link_canvas_with(view: &V, chosen: &str, impact: Option<&V>, off: &KindsOff) -> String {
    let diff_info = LinkDiffInfo::from_view(view, impact);
    let (names, held) = link_names(view);
    let mut name_key_index: HashMap<&str, usize> = HashMap::with_capacity(names.len());
    for (idx, (name, _)) in names.iter().enumerate() {
        if let Some(k) = name["key"].as_str() {
            name_key_index.entry(k).or_insert(idx);
        }
    }
    let at = |key: &V| {
        if let Some(s) = key.as_str() {
            name_key_index.get(s).copied()
        } else {
            names.iter().position(|(name, _)| &name["key"] == key)
        }
    };
    let edges = m::array(&view["edges"]);
    let pairs = edges
        .iter()
        .filter_map(|edge| Some((at(&edge["from"])?, at(&edge["to"])?, edge)))
        .collect::<Vec<_>>();
    let plan = ends(
        &held[0],
        &held[1],
        &pairs
            .iter()
            .map(|(from, to, _)| (*from, *to))
            .collect::<Vec<_>>(),
    );
    let users = held[0].iter().sum::<usize>();
    let picked = names.iter().position(|(name, _)| name["key"] == chosen);
    let near = |name: usize| {
        picked.is_some_and(|picked| {
            picked == name
                || pairs.iter().any(|(from, to, _)| {
                    (*from, *to) == (picked, name) || (*from, *to) == (name, picked)
                })
        })
    };
    // Every link runs between the same two columns, so the links of one
    // kind share one shading: from the colour of leaving to that of the kind.
    let link_kinds = pairs
        .iter()
        .map(|(_, _, edge)| EdgeKinds::of(edge))
        .collect::<Vec<_>>();
    let shadings: BTreeSet<&'static str> = link_kinds
        .iter()
        .filter_map(|kinds| kinds.primary(off))
        .chain(["calls"])
        .collect();
    let mut html = format!(
        "<div class=\"graph-map-plane\" style=\"width:{:.0}px;height:{:.0}px\"><svg width=\"{:.0}\" height=\"{:.0}\" aria-hidden=\"true\"><defs>{}",
        plan.width,
        plan.height,
        plan.width,
        plan.height,
        arrow_defs_svg("graph-link", false),
    );
    for kind in &shadings {
        let (suffix, kind_attr) = if *kind == "calls" {
            (String::new(), String::new())
        } else {
            (format!("-{kind}"), format!(" data-edge-kind=\"{kind}\""))
        };
        html.push_str(&format!(
            "<linearGradient id=\"graph-link-flow{suffix}\" class=\"graph-map-flow\"{kind_attr} gradientUnits=\"userSpaceOnUse\" x1=\"{:.0}\" y1=\"0\" x2=\"{:.0}\" y2=\"0\"><stop offset=\"0\" class=\"graph-map-flow-uses\"/><stop offset=\"0.5\" class=\"graph-map-flow-between\"/><stop offset=\"1\" class=\"graph-map-flow-used\"/></linearGradient>",
            MARGIN + FRAME_PAD + LINK_BOX,
            MARGIN + FRAME_PAD + LINK_BOX + LINK_GAP,
        ));
    }
    html.push_str("</defs>");
    for frame in &plan.files {
        if frame.width > 0.0 {
            html.push_str(&format!(
                "<rect class=\"graph-map-frame\" data-inner=\"false\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"8\"/>",
                frame.x, frame.y, frame.width, frame.height
            ));
        }
    }
    for (((from, to, edge), wire), kinds) in pairs.iter().zip(&plan.wires).zip(&link_kinds) {
        let state = match picked {
            Some(picked) if picked == *from || picked == *to => " graph-map-edge-on",
            Some(_) => " graph-map-edge-off",
            None => "",
        };
        let guessed = m::yes(&edge["guessed"]);
        let from_name = m::text(&names[*from].0, "name");
        let to_name = m::text(&names[*to].0, "name");
        let edge_diff = diff_info.diff_for_names(&from_name, &to_name);
        let diff_attr = edge_diff
            .map(|diff| format!(" data-diff=\"{diff}\""))
            .unwrap_or_default();
        let diff_note = match edge_diff {
            Some("added") => " · + added",
            Some("removed") => " · - removed",
            Some("unchanged") => " · = unchanged",
            _ => "",
        };
        let from_vis = names[*from].0["visibility"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let to_vis = names[*to].0["visibility"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let vis_note = match (from_vis, to_vis) {
            (Some(fv), Some(tv)) => format!(" · {fv} → {tv}"),
            (None, Some(tv)) => format!(" · visibility: {tv}"),
            (Some(fv), None) => format!(" · visibility: {fv}"),
            (None, None) => String::new(),
        };
        let edge_risk = edge["access_risk"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                edge["visibility_warning"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| diff_info.risk_for_names(&from_name, &to_name));
        let risk_note = edge_risk
            .map(|r| format!(" · risk: {r}"))
            .unwrap_or_default();
        let shown_kind = kinds.primary(off);
        let hidden_attr = if shown_kind.is_none() {
            " data-kind-hidden=\"true\""
        } else {
            ""
        };
        let arrow_kind = shown_kind.unwrap_or_else(|| pick_precedence_kind(&kinds.counts));
        let multi_kind = kinds.multi(off);
        let kinds_breakdown = kinds.breakdown.clone();
        let raw_kind = m::text(edge, "kind");
        let kind_label = match (raw_kind.is_empty(), kinds_breakdown) {
            (true, Some(breakdown)) => breakdown,
            (false, Some(breakdown)) => {
                if multi_kind {
                    breakdown
                } else {
                    raw_kind.replace('_', " ")
                }
            }
            (false, None) => raw_kind.replace('_', " "),
            (true, None) => arrow_kind.to_owned(),
        };
        let multi_attr = if multi_kind {
            " data-edge-multi=\"true\""
        } else {
            ""
        };
        let marker_suffix = arrow_marker_suffix(arrow_kind, false);
        html.push_str(&format!(
            "<path class=\"graph-map-edge{state}{}\" data-ends=\"{from} {to}\"{diff_attr}{hidden_attr} d=\"{wire}\" data-edge-kind=\"{arrow_kind}\" data-kinds=\"{}\"{multi_attr} stroke=\"url(#graph-link-flow{})\" stroke-width=\"{}\" marker-start=\"url(#graph-link-port{})\" marker-end=\"url(#graph-link-arrow{marker_suffix})\"><title>{} uses {} · {}{}{diff_note}{}{}</title></path>",
            if guessed { " graph-map-edge-guess" } else { "" },
            kinds.held(),
            if arrow_kind == "calls" { String::new() } else { format!("-{arrow_kind}") },
            if guessed { "3.4" } else { "1.4" },
            if multi_kind { "-multi" } else { "" },
            escape(&from_name),
            escape(&to_name),
            escape(&kind_label),
            if guessed { ", guessed by name" } else { "" },
            escape(&vis_note),
            escape(&risk_note),
        ));
        // What was only guessed by name is drawn hollow.
        if guessed {
            html.push_str(&format!(
                "<path class=\"graph-map-edge-core\" data-ends=\"{from} {to}\"{hidden_attr} d=\"{wire}\" data-edge-kind=\"{arrow_kind}\" stroke-width=\"1.4\"/>"
            ));
        }
    }
    html.push_str("</svg>");
    // The head of a frame: the file its names lie in.
    let mut first = 0;
    for (frame, size) in plan.files.iter().zip(held.iter().flatten()) {
        if let Some((_, file)) = names.get(first).filter(|_| *size > 0) {
            html.push_str(&format!(
                "<span class=\"graph-map-frame-name\" style=\"left:{:.0}px;top:{:.0}px;max-width:{:.0}px\" title=\"{}\">{}</span>",
                frame.x + FRAME_PAD,
                frame.y + 3.0,
                frame.width - 2.0 * FRAME_PAD,
                escape(&m::text(file, "path")),
                escape(&m::text(file, "name")),
            ));
        }
        first += size;
    }
    for (index, (name, file)) in names.iter().enumerate() {
        let (x, y) = plan.names[index];
        let uses = index < users;
        let kind = resolved_member_kind(name);
        let line = name["line"].as_u64().unwrap_or(0);
        let links = name["links"].as_u64().unwrap_or(0);
        let node_vis = name["visibility"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let node_vis_attr = node_vis
            .and_then(|v| normalize_visibility(Some(v)))
            .map(|(bucket, _, _)| format!(" data-visibility=\"{bucket}\""))
            .unwrap_or_default();
        let node_vis_pill = visibility_pill_html(node_vis, true);
        let node_risk = name["access_risk"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| diff_info.risk_for_names(&m::text(name, "name"), &m::text(name, "name")));
        let place = match line {
            0 => m::text(file, "path"),
            line => format!("{}:{line}", m::text(file, "path")),
        };
        let title_place = if let Some(vis) = node_vis {
            format!("{place} · {vis}")
        } else {
            place.clone()
        };
        let mut label = format!(
            "{}, {}, {}, {} here",
            m::text(name, "name"),
            what(name),
            place,
            if uses {
                format!("uses through {}", plural(links, "link", "links"))
            } else {
                format!("is used through {}", plural(links, "link", "links"))
            }
        );
        if let Some(vis) = node_vis {
            label.push_str(&format!(", visibility {vis}"));
        }
        if let Some(risk) = node_risk {
            label.push_str(&format!(", visibility risk: {risk}"));
        }
        let (badge_attrs, badges_strip) = if node_vis_pill.is_empty() {
            (String::new(), String::new())
        } else {
            build_node_badges_strip(&[node_vis_pill])
        };
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-map-node\" data-n=\"{index}\" data-action=\"graph-link-node\" data-payload=\"{}\" data-folder=\"false\" data-kind=\"{}\" data-selected=\"{}\" data-dim=\"{}\" data-flow=\"{}\" data-out=\"{uses}\" data-in=\"{}\"{node_vis_attr}{badge_attrs} style=\"left:{x:.0}px;top:{y:.0}px\" title=\"{}\" aria-label=\"{}\">{}<span class=\"graph-map-node-name\">{}</span>{}{badges_strip}</button>",
            payload(&name["key"]),
            escape(kind),
            picked == Some(index),
            picked.is_some() && !near(index),
            if uses { "uses" } else { "used" },
            !uses,
            escape(&title_place),
            escape(&label),
            match letter(kind) {
                "" => String::new(),
                letter => format!(
                    "<span class=\"graph-map-node-kind\" aria-hidden=\"true\">{letter}</span>"
                ),
            },
            escape(&m::text(name, "name")),
            if links > 1 {
                format!(
                    "<span class=\"graph-map-node-count\" aria-hidden=\"true\">{}</span>",
                    count(links)
                )
            } else {
                String::new()
            },
        ));
    }
    html.push_str("</div>");
    html
}
/// What the window of a link is called: what uses, and what is used.
pub fn link_title(view: &V) -> String {
    format!(
        "{} uses {}",
        m::text(&view["from"], "name"),
        m::text(&view["to"], "name")
    )
}
/// The link in a line: how many links it stands for and how they were
/// found, and how many names in how many files lie at each end.
pub fn link_summary(view: &V) -> String {
    if m::yes(&view["missing"]) {
        return "The graph does not know one end of this link any more.".to_owned();
    }
    let links = view["links"].as_u64().unwrap_or(0);
    let guessed = view["guessed"].as_u64().unwrap_or(0);
    if links == 0 {
        return if m::yes(&view["certain"]) {
            "No link read from the code runs this way.".to_owned()
        } else {
            "No link runs this way.".to_owned()
        };
    }
    let end = |side: &str| {
        let names = view[side]["names"].as_u64().unwrap_or(0);
        match view[side]["files"].as_u64().unwrap_or(0) {
            1 => plural(names, "name", "names"),
            files => format!(
                "{} in {} files",
                plural(names, "name", "names"),
                count(files)
            ),
        }
    };
    format!(
        "{}: {} read from the code, {} guessed by name · {} {} {}",
        plural(links, "link", "links"),
        count(links - guessed.min(links)),
        count(guessed),
        end("users"),
        if view["users"]["names"] == 1 {
            "uses"
        } else {
            "use"
        },
        end("used"),
    )
}
/// What of a link is not drawn, where it is more than a window holds.
pub fn link_note(view: &V) -> String {
    let (names, _) = link_names(view);
    let users = m::array(&view["users"]["groups"])
        .iter()
        .map(|group| m::array(&group["names"]).len() as u64)
        .sum::<u64>();
    let used = names.len() as u64 - users;
    let number = |side: &str| view[side]["names"].as_u64().unwrap_or(0);
    let drawn = m::array(&view["edges"]).len() as u64;
    let links = view["links"].as_u64().unwrap_or(0);
    let mut parts = vec![];
    if at_base(view) {
        parts.push(format!(
            "Links at PR base {}.",
            m::text(&view["base"], "commit")
        ));
    }
    if m::yes(&view["preliminary"]) {
        parts.push(
            "The links are those the files have in the checkout, not in that commit.".to_owned(),
        );
    }
    if users < number("users") || used < number("used") {
        parts.push(format!(
            "Showing the most linked names: {} of {} that use, {} of {} that are used, and {} of the {} between them. Look into a narrower link to see the rest: open the folder, or open it here.",
            count(users),
            count(number("users")),
            count(used),
            count(number("used")),
            count(drawn),
            plural(links, "link", "links"),
        ));
    } else if drawn < links {
        parts.push(format!(
            "Drawing {} of the {}.",
            count(drawn),
            plural(links, "link", "links")
        ));
    }
    parts.join(" ")
}
/// Beside the picture of a link: what the chosen name is and what it is
/// linked with here; with no name chosen, how to choose one and of which
/// kinds the links are.
#[cfg(test)]
pub fn link_side(view: &V, chosen: &str) -> String {
    link_side_with(view, chosen, None)
}
pub(in crate::graph_map) fn link_group_members_html(view: &V) -> String {
    let mut html = String::new();
    for (side_key, side_label) in [("users", "Caller"), ("used", "Target")] {
        for group in m::array(&view[side_key]["groups"]) {
            let ref_members = m::array(&group["referenced_members"]);
            let decl_members = m::array(&group["declared_members"]);
            let gwarnings = m::array(&group["visibility_warnings"]);
            if ref_members.is_empty() && decl_members.is_empty() && gwarnings.is_empty() {
                continue;
            }
            let gname = m::text(&group, "name");
            let label = if gname.is_empty() {
                side_label.to_owned()
            } else {
                format!("{} ({side_label})", escape(&gname))
            };
            html.push_str(&impact_members_details_html(
                &ref_members,
                &decl_members,
                &gwarnings,
                MemberDetailsOpts {
                    member_filter: "all",
                    used_title: "Referenced members",
                    exposed_title: "Declared members",
                    summary_title: &label,
                    force_open: !gwarnings.is_empty(),
                },
            ));
        }
    }
    html
}
/// Renders the link detail sidebar with optional change-impact diff badges (`+`/`-`/`=`).
pub fn link_side_with(view: &V, chosen: &str, impact: Option<&V>) -> String {
    let diff_info = LinkDiffInfo::from_view(view, impact);
    let banner = diff_info
        .pair_diff
        .map(|diff| {
            let label = match diff {
                "added" => "Added dependency in this change",
                "removed" => "Removed or changed dependency",
                _ => "Kept dependency",
            };
            format!(
                "<p class=\"knowledge-note\"><span class=\"graph-status-pill\" data-status=\"{diff}\">{}</span> {label}</p>",
                diff_badge_sign(diff)
            )
        })
        .unwrap_or_default();
    let link_refs = m::array(&view["member_refs"]);
    let link_warnings = if diff_info.warnings.is_empty() {
        String::new()
    } else {
        vis_warnings_html(&diff_info.warnings)
    };
    let link_refs_html = impact_members_details_html(
        &link_refs,
        &[],
        &[],
        MemberDetailsOpts {
            member_filter: "all",
            used_title: "Member references",
            exposed_title: "",
            summary_title: "Member references",
            force_open: !diff_info.warnings.is_empty(),
        },
    );
    let group_members_html = link_group_members_html(view);
    let (names, _) = link_names(view);
    let Some((name, file)) = names.iter().find(|(name, _)| name["key"] == chosen) else {
        let kinds = m::array(&view["kinds"])
            .iter()
            .map(|kind| {
                format!(
                    "{} {}",
                    escape(&m::text(kind, "kind").replace('_', " ")),
                    count(kind["links"].as_u64().unwrap_or(0))
                )
            })
            .collect::<Vec<_>>();
        return format!(
            "{banner}<p class=\"knowledge-note\">Select a name to light its links.{}</p>{}{link_warnings}{link_refs_html}{group_members_html}",
            if at_base(view) {
                ""
            } else {
                " Select it again to open its file on it in the map."
            },
            if kinds.is_empty() {
                String::new()
            } else {
                format!(
                    "<p class=\"knowledge-note\">Links by kind: {}</p>",
                    kinds.join(" · ")
                )
            }
        );
    };
    let line = name["line"].as_u64().unwrap_or(0);
    let mut html = format!(
        "{banner}<h3>{}</h3><p class=\"graph-map-path\">{}{}</p><p class=\"knowledge-note\">{}</p>{}{link_warnings}",
        escape(&m::text(name, "name")),
        escape(&m::text(file, "path")),
        if line > 0 {
            format!(":{line}")
        } else {
            String::new()
        },
        escape(&capital(&what(name))),
        focus_member_meta_html(name),
    );
    if !at_base(view) {
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"graph-link-open\" data-payload=\"{}\">Open in the map</button>",
            payload(&name["key"]),
        ));
    }
    // What it is linked with in this link, each with the way it is.
    let edges = m::array(&view["edges"]);
    let self_name = m::text(name, "name");
    for (title, own, other) in [("Uses here", "from", "to"), ("Used by here", "to", "from")] {
        let linked = edges
            .iter()
            .filter(|edge| edge[own] == name["key"])
            .filter_map(|edge| {
                let (other, _) = names.iter().find(|(name, _)| name["key"] == edge[other])?;
                Some((other, edge))
            })
            .collect::<Vec<_>>();
        if linked.is_empty() {
            continue;
        }
        html.push_str(&format!(
            "<h4>{title} <span class=\"graph-map-total\">{}</span></h4><ul class=\"graph-map-rows\">",
            count(linked.len() as u64)
        ));
        for (other, edge) in linked {
            let other_name = m::text(other, "name");
            let other_vis = visibility_pill_html(other["visibility"].as_str(), true);
            let okind = resolved_member_kind(other);
            let other_kind = if !other_vis.is_empty()
                || matches!(
                    okind.trim().to_ascii_lowercase().as_str(),
                    "property" | "prop" | "field" | "const" | "constant" | "class_const"
                ) {
                member_kind_pill_html(okind)
            } else {
                String::new()
            };
            let risk_pill = diff_info
                .risk_for_names(&self_name, &other_name)
                .or_else(|| edge["access_risk"].as_str().filter(|s| !s.is_empty()))
                .map(|risk| {
                    format!(
                        " <span class=\"graph-status-pill\" data-status=\"broken_risk\" title=\"{}\">vis!</span>",
                        escape(risk)
                    )
                })
                .unwrap_or_default();
            let diff_pill = diff_info
                .diff_for_names(&self_name, &other_name)
                .map(|diff| {
                    format!(
                        " <span class=\"graph-status-pill\" data-status=\"{diff}\">{}</span>",
                        diff_badge_sign(diff)
                    )
                })
                .unwrap_or_default();
            html.push_str(&format!(
                "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-link-node\" data-payload=\"{}\"><span class=\"graph-map-row-name\">{other_vis}{other_kind}{}</span><span class=\"graph-map-row-links\">{}{}{risk_pill}{diff_pill}</span></button></li>",
                payload(&other["key"]),
                escape(&other_name),
                escape(&m::text(edge, "kind").replace('_', " ")),
                if m::yes(&edge["guessed"]) { ", guessed" } else { "" },
            ));
        }
        html.push_str("</ul>");
    }
    html.push_str(&link_refs_html);
    html.push_str(&group_members_html);
    html
}
