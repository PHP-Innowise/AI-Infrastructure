use super::*;

pub(super) fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
pub(super) fn plural(value: u64, one: &str, many: &str) -> String {
    format!("{} {}", count(value), if value == 1 { one } else { many })
}
pub(super) fn escape(text: &str) -> String {
    crate::dom::escape_html(text)
}
pub(super) fn payload(value: &V) -> String {
    escape(&value.to_string())
}
/// How a link reads: how many, and how many of them the tool only inferred.
pub(super) fn weight(links: u64, guessed: u64) -> String {
    match guessed {
        0 => plural(links, "link", "links"),
        guessed if guessed == links => format!("{} guessed", plural(links, "link", "links")),
        guessed => format!(
            "{}, {} guessed",
            plural(links, "link", "links"),
            count(guessed)
        ),
    }
}
pub(super) fn changes(view: &V) -> bool {
    view["mode"] == "changes"
}
/// A file opened to its classes, methods and functions.
pub(super) fn symbols(view: &V) -> bool {
    view["mode"] == "file"
}
/// The links of one place followed step by step, wherever they lead.
/// Whether the picture is one where a folder can be spread out among the
/// others: a folder of the whole project.
pub(super) fn spreads(view: &V) -> bool {
    view["frames"].is_array()
}
pub(super) fn follows(view: &V) -> bool {
    view["mode"] == "follow"
}
/// The shortest ways from one place to another.
pub(super) fn paths(view: &V) -> bool {
    view["mode"] == "path"
}
pub(super) fn cycles(view: &V) -> bool {
    view["mode"] == "cycle"
}
pub(super) fn at_base(view: &V) -> bool {
    view["base"].is_object()
}
/// What two places share: what both use, or what uses both.
pub(super) fn shares(view: &V) -> bool {
    view["mode"] == "shared"
}
/// One found pattern: its parts, each in the frame of what it is in it.
pub(super) fn patterned(view: &V) -> bool {
    view["mode"] == "pattern"
}
/// The layers of a folder: no finding among its types, but all its files by
/// the layer each is filed in.
pub(super) fn layered(view: &V) -> bool {
    patterned(view) && view["pattern"]["sort"] == "layers"
}
/// Of what two places share, what uses both is drawn, not what both use.
pub(super) fn above(view: &V) -> bool {
    view["shared"]["side"] == "uses_both"
}
/// The boxes are symbols of whatever files they lie in: the links of a
/// class, a method or a function are followed, a path runs from or to one,
/// or one of two places that share something is one.
pub(super) fn names(view: &V) -> bool {
    (follows(view) && m::yes(&view["followed"]["symbol"]))
        || (paths(view) && m::yes(&view["path"]["names"]))
        || (shares(view) && m::yes(&view["shared"]["names"]))
        || (cycles(view) && m::yes(&view["cycle"]["names"]))
        || (patterned(view) && !layered(view))
}
/// The letter a symbol's box carries for what it is. The file's own node
/// and a name the graph says nothing more about carry none.
pub(super) fn letter(kind: &str) -> &'static str {
    match kind.trim().to_ascii_lowercase().as_str() {
        "type" => "T",
        "property" | "prop" | "field" => "P",
        "const" | "constant" | "class_const" => "K",
        "method" => "m",
        "function" | "fn" => "f",
        "case" => "c",
        "heading" => "#",
        "semantic" | "contract" => "◈",
        "table" | "schema" => "⊞",
        "queue" | "topic" | "event" => "⚡",
        _ => "",
    }
}
pub(super) fn resolved_member_kind(value: &V) -> &str {
    value["member_kind"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            value["kind"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("")
}
/// What a symbol is, in words: its kind and what holds it.
pub(super) fn what(symbol: &V) -> String {
    let (word, joined) = match resolved_member_kind(symbol) {
        "file" => {
            return "the file itself, for what belongs to no symbol in it, such as its imports"
                .to_owned();
        }
        "type" => ("type", "in"),
        "property" | "prop" => ("property", "of"),
        "field" => ("field", "of"),
        "const" | "constant" | "class_const" => ("constant", "of"),
        "method" => ("method", "of"),
        "function" | "fn" => ("function", "in"),
        "case" => ("case", "of"),
        "heading" => ("heading", "under"),
        _ => ("name (a constant, a field or a plain type)", "in"),
    };
    match symbol["owner"].as_str() {
        Some(owner) => format!("{word} {joined} {owner}"),
        None => word.to_owned(),
    }
}
pub(super) fn normalize_visibility(
    vis: Option<&str>,
) -> Option<(&'static str, &'static str, &'static str)> {
    let raw = vis?.trim();
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_ascii_lowercase();
    if lower == "public" || lower == "pub" || lower == "export" || lower == "open" {
        Some(("public", "◎ pub", "pub"))
    } else if lower == "protected" || lower.starts_with("protected(") {
        Some(("protected", "◈ prot", "prot"))
    } else if lower == "private"
        || lower == "#private"
        || lower.starts_with("private(")
        || lower == "pub(self)"
    {
        Some(("private", "⊗ priv", "priv"))
    } else if lower == "internal"
        || lower == "package"
        || lower == "package-private"
        || lower == "package private"
        || lower == "fileprivate"
        || lower == "pub(crate)"
        || lower == "pub(super)"
        || lower.starts_with("pub(in")
    {
        Some(("internal", "◉ pkg", "pkg"))
    } else {
        None
    }
}
pub(super) fn visibility_pill_html(vis: Option<&str>, compact: bool) -> String {
    let Some(raw) = vis.map(str::trim).filter(|s| !s.is_empty()) else {
        return String::new();
    };
    let Some((bucket, full_label, compact_label)) = normalize_visibility(Some(raw)) else {
        return String::new();
    };
    let label = if compact { compact_label } else { full_label };
    let compact_attr = if compact { "true" } else { "false" };
    let title = format!("Visibility: {}", escape(raw));
    format!(
        "<span class=\"graph-vis-pill\" data-vis=\"{bucket}\" data-compact=\"{compact_attr}\" title=\"{title}\" aria-label=\"{title}\">{}</span>",
        escape(label),
    )
}
pub(super) fn member_kind_tag(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "property" | "prop" => Some(("prop", "prop")),
        "field" => Some(("field", "field")),
        "const" | "constant" | "class_const" => Some(("const", "const")),
        "method" => Some(("method", "method")),
        "function" | "fn" => Some(("fn", "fn")),
        _ => None,
    }
}
pub(super) fn member_kind_pill_html(kind: &str) -> String {
    let Some((attr, label)) = member_kind_tag(kind) else {
        return String::new();
    };
    format!("<span class=\"graph-member-kind\" data-member-kind=\"{attr}\">{label}</span>")
}
pub(super) fn vis_delta_badge_html(
    prev: Option<&str>,
    curr: Option<&str>,
    narrowed: bool,
) -> String {
    let prev_text = prev.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("?");
    let curr_text = curr.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("?");
    let status = if narrowed { "broken_risk" } else { "modified" };
    let title = format!("{} → {}", escape(prev_text), escape(curr_text));
    let aria = if narrowed {
        format!("Visibility narrowed: {title}")
    } else {
        format!("Visibility changed: {title}")
    };
    format!(
        "<span class=\"graph-status-pill graph-member-vis-delta\" data-status=\"{status}\" title=\"{title}\" aria-label=\"{aria}\">vis!</span>"
    )
}
pub(super) fn member_has_risk(member: &V) -> bool {
    let change = m::text(member, "change").trim().to_ascii_lowercase();
    let access_risk = m::text(member, "access_risk").trim().to_ascii_lowercase();
    m::yes(&member["visibility_narrowed"])
        || m::yes(&member["visibility_changed"])
        || m::yes(&member["signature_changed"])
        || !access_risk.is_empty()
        || (!change.is_empty() && change != "unchanged")
        || (member["previous_visibility"].as_str().is_some()
            && member["visibility"].as_str().is_some()
            && member["previous_visibility"].as_str() != member["visibility"].as_str())
}
pub(super) fn member_has_severe_risk(member: &V) -> bool {
    let change = m::text(member, "change").trim().to_ascii_lowercase();
    let access_risk = m::text(member, "access_risk").trim().to_ascii_lowercase();
    m::yes(&member["visibility_narrowed"])
        || matches!(
            access_risk.as_str(),
            "deleted" | "inaccessible" | "visibility_narrowed" | "signature_changed"
        )
        || change == "removed"
}
pub(super) fn member_rows_html(members: &[V], member_filter: &str) -> String {
    let risk_only = member_filter == "risk";
    let filtered: Vec<&V> = members
        .iter()
        .filter(|member| !risk_only || member_has_risk(member))
        .collect();
    if filtered.is_empty() {
        return String::new();
    }
    let mut html = String::from("<ul class=\"graph-member-list\">");
    for member in filtered {
        let vis_pill = visibility_pill_html(member["visibility"].as_str(), false);
        let kind_pill = member_kind_pill_html(resolved_member_kind(member));
        let name = m::text(member, "name").trim().to_owned();
        let signature = m::text(member, "signature").trim().to_owned();
        let display_sig = if signature.is_empty() {
            name.clone()
        } else if !name.is_empty() && !signature.contains(&name) {
            format!("{name}{signature}")
        } else {
            signature.clone()
        };
        let sig_title = if signature.is_empty() {
            name.clone()
        } else {
            signature
        };
        let access = m::text(member, "access").trim().to_owned();
        let access_html = if access.is_empty() {
            String::new()
        } else {
            let access_esc = escape(&access);
            format!(
                "<span class=\"graph-member-access\" data-access=\"{access_esc}\">{access_esc}</span>"
            )
        };
        let prev_vis = member["previous_visibility"].as_str();
        let curr_vis = member["visibility"].as_str();
        let change = m::text(member, "change").trim().to_ascii_lowercase();
        let access_risk = m::text(member, "access_risk").trim().to_ascii_lowercase();
        let vis_changed = m::yes(&member["visibility_narrowed"])
            || m::yes(&member["visibility_changed"])
            || change == "visibility_changed"
            || access_risk == "visibility_narrowed"
            || access_risk == "inaccessible"
            || (prev_vis.is_some() && curr_vis.is_some() && prev_vis != curr_vis);
        let narrowed = m::yes(&member["visibility_narrowed"])
            || matches!(access_risk.as_str(), "visibility_narrowed" | "inaccessible");
        let mut badges = String::new();
        if vis_changed {
            badges.push_str(&vis_delta_badge_html(prev_vis, curr_vis, narrowed));
        }
        match access_risk.as_str() {
            "deleted" => badges.push_str(
                "<span class=\"graph-status-pill\" data-status=\"broken_risk\" title=\"Referenced member was deleted\">deleted</span>",
            ),
            "inaccessible" => badges.push_str(
                "<span class=\"graph-status-pill\" data-status=\"broken_risk\" title=\"Referenced member is no longer accessible\">inaccessible</span>",
            ),
            "signature_changed" => badges.push_str(
                "<span class=\"graph-status-pill\" data-status=\"modified\" title=\"Signature changed\">sig</span>",
            ),
            _ if m::yes(&member["signature_changed"]) => badges.push_str(
                "<span class=\"graph-status-pill\" data-status=\"modified\" title=\"Signature changed\">sig</span>",
            ),
            _ => {}
        }
        match change.as_str() {
            "added" => {
                badges.push_str("<span class=\"graph-status-pill\" data-status=\"added\">+</span>")
            }
            "removed" => badges
                .push_str("<span class=\"graph-status-pill\" data-status=\"removed\">-</span>"),
            "modified" => badges
                .push_str("<span class=\"graph-status-pill\" data-status=\"modified\">~</span>"),
            _ => {}
        }
        let line = member["line"]
            .as_u64()
            .filter(|&l| l > 0)
            .map(|l| format!("<span class=\"graph-member-line\">:{l}</span>"))
            .unwrap_or_default();
        let risk_attr = if member_has_risk(member) {
            " data-risk=\"true\""
        } else {
            ""
        };
        let narrowed_attr = if narrowed {
            " data-narrowed=\"true\""
        } else {
            ""
        };
        let change_attr = if change.is_empty() {
            String::new()
        } else {
            format!(" data-change=\"{}\"", escape(&change))
        };
        html.push_str(&format!(
            "<li class=\"graph-member-row\"{risk_attr}{narrowed_attr}{change_attr}>{vis_pill}{kind_pill}<span class=\"graph-member-sig\" title=\"{}\">{}</span>{access_html}{badges}{line}</li>",
            escape(&sig_title),
            escape(&display_sig),
        ));
    }
    html.push_str("</ul>");
    html
}
pub(super) fn vis_warnings_html(warnings: &[V]) -> String {
    if warnings.is_empty() {
        return String::new();
    }
    let mut html = String::from("<div class=\"graph-vis-warning-box\" role=\"note\">");
    for w in warnings {
        let member = m::text(w, "member");
        let kind_pill = member_kind_pill_html(resolved_member_kind(w));
        let vis_pill = visibility_pill_html(w["visibility"].as_str(), true);
        let prev_vis = w["previous_visibility"].as_str();
        let curr_vis = w["visibility"].as_str();
        let delta = if prev_vis.is_some() && curr_vis.is_some() && prev_vis != curr_vis {
            vis_delta_badge_html(prev_vis, curr_vis, true)
        } else {
            String::new()
        };
        let reason = m::text(w, "reason");
        let detail = if reason.is_empty() {
            "visibility risk"
        } else {
            reason.as_str()
        };
        let line = w["line"]
            .as_u64()
            .filter(|&l| l > 0)
            .map(|l| format!(":{l}"))
            .unwrap_or_default();
        html.push_str(&format!(
            "<div class=\"graph-vis-warning-row\">{vis_pill}{kind_pill}<strong class=\"graph-vis-warning-member\">{}{}</strong>{delta}<span class=\"graph-vis-warning-detail\">{}</span></div>",
            escape(&member),
            line,
            escape(detail),
        ));
    }
    html.push_str("</div>");
    html
}
pub(super) struct MemberDetailsOpts<'a> {
    pub(super) member_filter: &'a str,
    pub(super) used_title: &'a str,
    pub(super) exposed_title: &'a str,
    pub(super) summary_title: &'a str,
    pub(super) force_open: bool,
}
pub(super) fn impact_members_details_html(
    used: &[V],
    exposed: &[V],
    warnings: &[V],
    opts: MemberDetailsOpts<'_>,
) -> String {
    if used.is_empty() && exposed.is_empty() && warnings.is_empty() {
        return String::new();
    }
    let filter = match opts.member_filter {
        "used" | "exposed" | "risk" => opts.member_filter,
        _ => "all",
    };
    let show_used = matches!(filter, "all" | "used" | "risk");
    let show_exposed = matches!(filter, "all" | "exposed" | "risk");
    let used_list = if show_used {
        member_rows_html(used, filter)
    } else {
        String::new()
    };
    let exposed_list = if show_exposed {
        member_rows_html(exposed, filter)
    } else {
        String::new()
    };
    let warnings_html = if matches!(filter, "all" | "used" | "risk") {
        vis_warnings_html(warnings)
    } else {
        String::new()
    };
    let shown_used = if show_used {
        used.iter()
            .filter(|item| filter != "risk" || member_has_risk(item))
            .count()
    } else {
        0
    };
    let shown_exposed = if show_exposed {
        exposed
            .iter()
            .filter(|item| filter != "risk" || member_has_risk(item))
            .count()
    } else {
        0
    };
    let has_risk = !warnings.is_empty()
        || used.iter().any(member_has_severe_risk)
        || exposed.iter().any(member_has_severe_risk);
    let open_attr = if opts.force_open || has_risk || filter == "risk" {
        " open"
    } else {
        ""
    };
    let mut body = String::new();
    if !warnings_html.is_empty() {
        body.push_str(&warnings_html);
    }
    if !used_list.is_empty() {
        body.push_str(&format!(
            "<div class=\"graph-member-section-title\">{} ({shown_used})</div>{used_list}",
            opts.used_title,
        ));
    }
    if !exposed_list.is_empty() {
        body.push_str(&format!(
            "<div class=\"graph-member-section-title\">{} ({shown_exposed})</div>{exposed_list}",
            opts.exposed_title,
        ));
    }
    if body.is_empty() {
        body.push_str("<p class=\"knowledge-note\">No members match this filter.</p>");
    }
    let total_shown = shown_used + shown_exposed;
    let count_badge = if filter == "all" {
        if !used.is_empty() && !exposed.is_empty() {
            format!("{} used · {} exposed", used.len(), exposed.len())
        } else if !used.is_empty() {
            format!("{} used", used.len())
        } else if !exposed.is_empty() {
            format!("{} exposed", exposed.len())
        } else {
            format!("{} warnings", warnings.len())
        }
    } else {
        format!("{total_shown}")
    };
    format!(
        "<details class=\"graph-impact-members\"{open_attr}><summary class=\"graph-impact-members-summary\"><span class=\"graph-impact-members-label\">{} ({count_badge})</span></summary>{body}</details>",
        opts.summary_title,
    )
}
pub(super) fn capital(text: &str) -> String {
    let mut letters = text.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}
pub(super) fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
/// Git's letter for what happened to a file, in words.
pub(super) fn happened(status: &str) -> &'static str {
    match status {
        "A" => "added",
        "M" => "modified",
        "D" => "deleted",
        "R" => "renamed",
        "C" => "copied",
        "T" => "changed in kind",
        "U" => "in conflict",
        _ => "changed",
    }
}
/// Adds to the Show list what the run of the session on the screen changed,
/// as that session recorded it. Where that run is the one shown, the list
/// has it already and calls it by what it is here. With no session on the
/// screen the list stays as it is.
pub fn with_run(list: String, chosen: &str, session: &str) -> String {
    if session.is_empty() {
        return list;
    }
    let own = format!("run:{session}");
    if chosen == own {
        return list.replace(
            ">A run of an agent</option>",
            ">The run of this session</option>",
        );
    }
    format!(
        "{list}<option value=\"{}\">The run of this session</option>",
        escape(&own)
    )
}
/// Adds the active System Change (`sysrun:{run_id}`) option to `#graph-map-target` (`Show:`).
pub fn with_system_run(
    list: String,
    chosen: &str,
    run_id: Option<&str>,
    run_title: Option<&str>,
) -> String {
    let Some(rid) = run_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return list;
    };
    let own = format!("sysrun:{rid}");
    let label = match run_title.map(str::trim).filter(|s| !s.is_empty()) {
        Some(title) => format!("System change: {title}"),
        None => format!("System change: {rid}"),
    };
    let generic = format!(">System change {}</option>", escape(rid));
    if chosen == own && list.contains(&generic) {
        return list.replace(&generic, &format!(">{}</option>", escape(&label)));
    }
    if list.contains(&format!("value=\"{}\"", escape(&own))) {
        return list;
    }
    format!(
        "{list}<option value=\"{}\"{}>{}</option>",
        escape(&own),
        if chosen == own { " selected" } else { "" },
        escape(&label),
    )
}
/// Renders `<option>` items for the `#graph-map-service` header switcher when in `SystemService` scope.
pub fn service_switcher_options_html(scope: &GraphTargetScope) -> String {
    let GraphTargetScope::SystemService {
        service_id,
        services,
        ..
    } = scope
    else {
        return String::new();
    };
    let mut html = "<option value=\"__topology__\">System topology</option>".to_owned();
    html.push_str(&format!(
        "<option value=\"__system__\"{}>__system__ (System root)</option>",
        if service_id == "__system__" {
            " selected"
        } else {
            ""
        },
    ));
    let mut seen = BTreeSet::new();
    seen.insert("__system__".to_owned());
    let mut all = services.clone();
    if !service_id.is_empty() && !all.iter().any(|s| s == service_id) {
        all.push(service_id.clone());
    }
    for svc in all {
        let trimmed = svc.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_owned()) {
            continue;
        }
        html.push_str(&format!(
            "<option value=\"{}\"{}>Service: {}</option>",
            escape(trimmed),
            if trimmed == service_id {
                " selected"
            } else {
                ""
            },
            escape(trimmed),
        ));
    }
    html
}
/// What the Show list offers: the whole project and the checkout's own
/// work. A commit, a pull request or a branch chosen in the picker stays in
/// the list while it is the one shown.
pub fn targets(place: &V, chosen: &str) -> String {
    let option = |value: &str, label: &str| {
        format!(
            "<option value=\"{}\"{}>{}</option>",
            escape(value),
            if value == chosen { " selected" } else { "" },
            escape(label)
        )
    };
    let mut html = option("", "The whole project");
    if !m::yes(&place["git"]) {
        if let Some(("sysrun", rid)) = chosen.split_once(':') {
            html.push_str(&option(chosen, &format!("System change {rid}")));
        }
        return html;
    }
    let branch = place["branch"].as_str().unwrap_or("this checkout");
    let against = place["against"].as_str().or(place["base"].as_str());
    html.push_str(&option(
        "work",
        &match (against, place["pull"].as_u64()) {
            (Some(base), Some(pull)) => {
                format!("Current work: {branch} against {base} (pull request #{pull})")
            }
            (Some(base), None) => format!("Current work: {branch} against {base}"),
            (None, _) => format!("Current work: {branch}"),
        },
    ));
    html.push_str(&option("uncommitted", "Uncommitted changes only"));
    if !["", "work", "uncommitted"].contains(&chosen) {
        let label = match chosen.split_once(':') {
            Some(("pr", number)) => m::array(&place["pulls"])
                .iter()
                .find(|pull| pull["number"].as_u64() == number.parse().ok())
                .map_or_else(
                    || format!("Pull request #{number}"),
                    |pull| format!("#{number} {}", m::text(pull, "title")),
                ),
            Some(("commit", hash)) => {
                format!("Commit {}", hash.chars().take(8).collect::<String>())
            }
            Some(("run", _)) => "A run of an agent".to_owned(),
            Some(("sysrun", rid)) => format!("System change {rid}"),
            Some((_, name)) => name.to_owned(),
            None => chosen.to_owned(),
        };
        html.push_str(&option(chosen, &label));
    }
    html
}
/// What a typed text names: a short number, with or without `#`, is a pull
/// request; anything else is a branch, a tag or a commit.
pub fn typed(text: &str) -> Option<String> {
    let text = text.trim();
    let digits = text.strip_prefix('#').unwrap_or(text);
    let number = !digits.is_empty()
        && !digits.starts_with('0')
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits.len() <= 6 || text.starts_with('#'))
        && digits.len() <= 9;
    if number {
        return Some(format!("pr:{digits}"));
    }
    (!text.is_empty()
        && text.len() <= 200
        && !text.starts_with(['-', '#'])
        && !text
            .chars()
            .any(|letter| letter.is_whitespace() || letter.is_control()))
    .then(|| format!("ref:{text}"))
}
/// The two sides of the chosen change, in one line.
pub fn chosen(view: &V) -> String {
    let chosen = &view["chosen"];
    if !chosen.is_object() {
        return String::new();
    }
    let side = |side: &V| {
        side.is_object().then(|| {
            format!(
                "{} at {}",
                m::text(side, "name"),
                m::text(side, "commit").chars().take(8).collect::<String>()
            )
        })
    };
    let (head, base) = (side(&chosen["head"]), side(&chosen["base"]));
    let name = m::text(chosen, "name");
    let text = match (chosen["kind"].as_str(), head, base) {
        (Some("uncommitted"), ..) => format!("What is not committed on {name}"),
        (Some("work"), Some(head), Some(base)) => {
            format!("{head} and what is not committed, against {base}")
        }
        (Some("work"), ..) => format!("{name} and what is not committed"),
        // What a run of an agent changed, as its session recorded it.
        (Some("run"), ..) if name.is_empty() => "What a run changed".to_owned(),
        (Some("run"), ..) => format!("What the run changed: {name}"),
        (Some("sysrun" | "system_run"), ..) if name.is_empty() => {
            "What the system change touched".to_owned()
        }
        (Some("sysrun" | "system_run"), ..) => format!("System change: {name}"),
        // One commit of the history: its own change, against its parent.
        (Some("commit"), ..) => format!(
            "commit {}",
            m::text(&chosen["head"], "commit")
                .chars()
                .take(8)
                .collect::<String>()
        ),
        (_, Some(head), Some(base)) => format!("{head} against {base}"),
        (_, Some(head), None) => head,
        _ => name,
    };
    match chosen["title"].as_str() {
        Some(title) => format!("{title} · {text}"),
        None => text,
    }
}
/// The sentence above the map: what the graph holds and how sure it is, or,
/// for a change, how much changed and how far it reaches.
/// What the map shows, in a line: how much the graph holds, or what the
/// opened file, the followed place or the change is, and, without tests,
/// how much that leaves out.
pub(super) fn is_native_ir_marker(raw: &str) -> bool {
    let lower = raw.trim().to_ascii_lowercase();
    lower.contains("native-tree-sitter-ir")
        || lower.contains("native tree-sitter sqlite ir")
        || lower.contains("native-sqlite-ir")
        || lower.contains("native_tree_sitter_ir")
}

/// Normalizes internal code-graph backend identifiers into concise user-facing labels.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn format_graph_backend_label(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if is_native_ir_marker(trimmed) {
        "Native tree-sitter IR".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Replaces raw internal native-IR markers inside status/banner strings with a concise readable label.
pub(crate) fn format_graph_status_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !is_native_ir_marker(trimmed) {
        return trimmed.to_owned();
    }
    let mut out = trimmed.to_owned();
    for needle in [
        "native tree-sitter SQLite IR",
        "native tree-sitter sqlite ir",
        "Native tree-sitter SQLite IR",
        "native-tree-sitter-ir",
        "native_tree_sitter_ir",
        "native-sqlite-ir",
    ] {
        out = out.replace(needle, "Native tree-sitter IR");
    }
    if is_native_ir_marker(&out) {
        let lower = out.to_ascii_lowercase();
        for needle in [
            "native tree-sitter sqlite ir",
            "native-tree-sitter-ir",
            "native_tree_sitter_ir",
            "native-sqlite-ir",
        ] {
            if let Some(at) = lower.find(needle) {
                let end = at + needle.len();
                if out.is_char_boundary(at) && out.is_char_boundary(end) {
                    out.replace_range(at..end, "Native tree-sitter IR");
                }
                break;
            }
        }
    }
    out
}

pub(super) fn graph_backend_label(view: &V) -> Option<String> {
    let candidates = [
        view["backend"].as_str(),
        view["built"]["tool"].as_str(),
        view["built"]["backend"].as_str(),
        view["tool"]["backend"].as_str(),
        view["tool"]["name"].as_str(),
        view["tool"]["version"].as_str(),
        view["status"]["backend"].as_str(),
        view["status"]["tool"]["version"].as_str(),
        view["message"].as_str(),
    ];
    for candidate in candidates.into_iter().flatten() {
        if is_native_ir_marker(candidate) {
            return Some("Native tree-sitter IR".to_owned());
        }
    }
    None
}

pub fn summary(view: &V) -> String {
    let mut text = format!("{}{}", shown(view), left_out(view));
    if let Some(backend) = graph_backend_label(view) {
        text.push_str(&format!(" · {backend}"));
    }
    text
}
/// What a map without tests leaves out: the files that are tests, and the
/// tests inside the files it keeps. Nothing where tests are not left out.
pub(super) fn left_out(view: &V) -> String {
    let tests = &view["tests"];
    if !tests.is_object() {
        return String::new();
    }
    let number = |key: &str| tests[key].as_u64().unwrap_or(0);
    match (number("files"), number("inside")) {
        (0, 0) => " · no tests found to leave out".to_owned(),
        (files, 0) => format!(" · without {}", plural(files, "test file", "test files")),
        (0, inside) => format!(
            " · without {} inside its files",
            plural(inside, "test", "tests")
        ),
        (files, inside) => format!(
            " · without {} and {} inside other files",
            plural(files, "test file", "test files"),
            plural(inside, "test", "tests")
        ),
    }
}
pub(super) fn shown(view: &V) -> String {
    if view["mode"] == "scenario" {
        let scenario = &view["scenario"];
        return if m::yes(&scenario["missing"]) {
            "Scenario starting point unavailable".to_owned()
        } else {
            format!(
                "Scenario from {} · {}",
                m::text(&scenario["origin"], "name"),
                plural(
                    scenario["reachable"].as_u64().unwrap_or(0),
                    "reachable place",
                    "reachable places"
                )
            )
        };
    }
    if view["mode"] == "modules" {
        let all = view["children"].as_u64().unwrap_or(0);
        let found = view["matching"].as_u64().unwrap_or(0);
        let shown = m::array(&view["nodes"]).len() as u64;
        return format!(
            "{} · {shown} shown · {found} matching",
            plural(all, "module", "modules")
        );
    }
    if symbols(view) {
        let file = &view["file"];
        let number = |key: &str| file[key].as_u64().unwrap_or(0);
        let held = [
            ("types", "type", "types"),
            ("methods", "method", "methods"),
            ("functions", "function", "functions"),
            ("headings", "heading", "headings"),
            ("others", "other", "others"),
        ]
        .iter()
        .filter(|(key, ..)| number(key) > 0)
        .map(|(key, one, many)| plural(number(key), one, many))
        .collect::<Vec<_>>();
        let mut text = m::text(file, "name");
        if let Some(status) = file["status"].as_str() {
            text.push_str(&format!(" ({})", happened(status)));
        }
        text.push_str(&if held.is_empty() {
            " · the graph names nothing inside it".to_owned()
        } else {
            format!(
                " · {} · {} between them",
                held.join(", "),
                plural(number("inside"), "link", "links")
            )
        });
        text.push_str(&match number("uses") {
            0 => " · uses no other file".to_owned(),
            used => format!(" · uses {}", plural(used, "other file", "other files")),
        });
        text.push_str(&match number("used_by") {
            0 => " · no other file uses it".to_owned(),
            1 => " · 1 other file uses it".to_owned(),
            users => format!(" · {} other files use it", count(users)),
        });
        return text;
    }
    if layered(view) {
        // The folder, how many layers hold how many of its files, and how
        // much runs against their order.
        let pattern = &view["pattern"];
        let layers = m::array(&pattern["layers"]);
        return format!(
            "Layers: {} · {} · {} · {}",
            m::text(&pattern["of"], "name"),
            plural(layers.len() as u64, "layer", "layers"),
            plural(
                layers
                    .iter()
                    .map(|layer| layer["files"].as_u64().unwrap_or(0))
                    .sum(),
                "file",
                "files"
            ),
            match (
                m::yes(&pattern["declared"]),
                pattern["against"]["links"].as_u64().unwrap_or(0)
            ) {
                (true, links) => plural(links, "rule violation", "rule violations"),
                (false, 0) => "nothing against their order".to_owned(),
                (false, links) => format!("{} against their order", plural(links, "link", "links")),
            }
        ) + &changed_order(&pattern["change"]);
    }
    if patterned(view) {
        // The pattern, the type it is named after, and its other parts by
        // what they are in it.
        let pattern = &view["pattern"];
        let mut text = format!(
            "{}: {}",
            m::text(pattern, "name"),
            m::text(&pattern["of"], "name")
        );
        for part in m::array(&pattern["parts"]) {
            let number = |key: &str| part[key].as_u64().unwrap_or(0);
            text.push_str(&format!(
                " · {}",
                plural(
                    number("found"),
                    &m::text(&part, "one"),
                    &m::text(&part, "many")
                )
            ));
            if number("drawn") < number("found") {
                text.push_str(&format!(", {} drawn", count(number("drawn"))));
            }
        }
        return text;
    }
    if cycles(view) {
        let cycle = &view["cycle"];
        let name = m::text(&cycle["origin"], "name");
        if m::yes(&cycle["origin"]["missing"]) {
            return format!("{name} is absent from this graph");
        }
        let scope = if cycle["origin"]["what"] == "folder" {
            "folder-level cycle"
        } else {
            "cycle"
        };
        let steps = cycle["steps"].as_u64().unwrap_or(0);
        return if steps == 0 {
            let links = if m::yes(&view["certain"]) {
                " using links read from the code"
            } else {
                ""
            };
            format!("No {scope} found through {name}{links}")
        } else {
            format!(
                "Shortest {scope} through {name} · {}",
                plural(steps, "link", "links")
            )
        };
    }
    if paths(view) {
        let path = &view["path"];
        let number = |key: &str| path[key].as_u64().unwrap_or(0);
        let (from, to) = (m::text(&path["from"], "name"), m::text(&path["to"], "name"));
        if !m::yes(&path["found"]) {
            return format!("No path between {from} and {to}");
        }
        // Where nothing leads from the first to the second, the way that
        // is drawn runs from the second to the first.
        let (first, last) = if m::yes(&path["reversed"]) {
            (to, from)
        } else {
            (from, to)
        };
        let mut text = format!(
            "Path from {first} to {last} · {} · {} places on the shortest ways",
            plural(number("steps"), "link", "links"),
            count(number("places"))
        );
        if number("shown") < number("places") {
            text.push_str(&format!(", {} drawn", count(number("shown"))));
        }
        // The ways round that were asked for and found.
        if number("detours") > 0 {
            text.push_str(&format!(
                " · {}{}",
                if m::yes(&path["detours_more"]) {
                    "at least "
                } else {
                    ""
                },
                plural(number("detours"), "longer way", "longer ways")
            ));
        }
        return text;
    }
    if shares(view) {
        let shared = &view["shared"];
        let number = |key: &str| shared[key].as_u64().unwrap_or(0);
        let (from, to) = (
            m::text(&shared["from"], "name"),
            m::text(&shared["to"], "name"),
        );
        let (used, users) = (number("both_use"), number("uses_both"));
        if used + users == 0 {
            return format!("{from} and {to} share nothing");
        }
        // Among symbols the others are names, else files.
        let (one, many) = if names(view) {
            ("other name", "other names")
        } else {
            ("other file", "other files")
        };
        // The kind that is drawn comes first, with the two by name, how
        // many of it a filter leaves, and which of those are drawn where
        // that is not all: so many of the nearest, or the places after
        // those that were passed over.
        let drawn = |all: u64| {
            let (matching, skip, shown) = (number("matching"), number("skip"), number("shown"));
            let mut text = match matching {
                left if left == all => String::new(),
                0 => ", none matches the filter".to_owned(),
                1 => ", 1 matches the filter".to_owned(),
                left => format!(", {} match the filter", count(left)),
            };
            if skip > 0 && shown == 1 {
                text.push_str(&format!(", place {} drawn", count(skip + 1)));
            } else if skip > 0 {
                text.push_str(&format!(
                    ", places {} to {} drawn",
                    count(skip + 1),
                    count(skip + shown)
                ));
            } else if shown < matching {
                text.push_str(&format!(", {} drawn", count(shown)));
            }
            text
        };
        return if above(view) {
            format!(
                "{} · {}",
                match users {
                    0 => format!("Nothing uses both {from} and {to}"),
                    1 => format!("1 {one} uses both {from} and {to}{}", drawn(1)),
                    users => format!(
                        "{} {many} use both {from} and {to}{}",
                        count(users),
                        drawn(users)
                    ),
                },
                match used {
                    0 => "they use nothing in common".to_owned(),
                    used => format!("both use {}", plural(used, one, many)),
                }
            )
        } else {
            format!(
                "{} · {}",
                match used {
                    0 => format!("{from} and {to} use nothing in common"),
                    used => format!(
                        "{from} and {to} both use {}{}",
                        plural(used, one, many),
                        drawn(used)
                    ),
                },
                match users {
                    0 => "nothing uses both".to_owned(),
                    1 => format!("1 {one} uses both"),
                    users => format!("{} {many} use both", count(users)),
                }
            )
        };
    }
    if follows(view) {
        let followed = &view["followed"];
        let number = |key: &str| followed[key].as_u64().unwrap_or(0);
        // A file is followed among files, a folder among folders and
        // files, and a symbol among symbols.
        let (one, many) = if m::yes(&followed["symbol"]) {
            ("other name", "other names")
        } else if m::yes(&followed["folder"]) {
            ("other", "others")
        } else {
            ("other file", "other files")
        };
        let mut text = format!("Links of {}, step by step", m::text(followed, "name"));
        text.push_str(&match number("uses") {
            0 => " · uses nothing else".to_owned(),
            used => format!(" · uses {}", plural(used, one, many)),
        });
        text.push_str(&match number("users") {
            0 => " · nothing else uses it".to_owned(),
            1 => format!(" · 1 {one} uses it"),
            users => format!(" · {} {many} use it", count(users)),
        });
        text.push_str(&format!(" · {} drawn", count(number("shown"))));
        return text;
    }
    if changes(view) {
        let number = |key: &str| view["change"][key].as_u64().unwrap_or(0);
        let mut text = plural(number("files"), "changed file", "changed files");
        if number("graphed") < number("files") {
            text.push_str(&format!(", {} in the graph", count(number("graphed"))));
        }
        if number("here") < number("graphed") {
            text.push_str(&format!(", {} in this folder", count(number("here"))));
        }
        text.push_str(&match number("affected") {
            0 => " · no other file uses them".to_owned(),
            1 => " · 1 other file uses them".to_owned(),
            users => format!(" · {} other files use them", count(users)),
        });
        text.push_str(&match number("needed") {
            0 => " · they use no other file".to_owned(),
            used => format!(" · they use {}", plural(used, "other file", "other files")),
        });
        if number("removed") > 0 {
            text.push_str(&format!(" · {} deleted", count(number("removed"))));
        }
        text.push_str(&run_reach(view));
        return text;
    }
    let totals = &view["totals"];
    let number = |key: &str| totals[key].as_u64().unwrap_or(0);
    let mut text = format!(
        "{} · {} between files: {} read from the code, {} guessed by name",
        plural(number("files"), "file", "files"),
        plural(number("links"), "link", "links"),
        count(number("read")),
        count(number("guessed")),
    );
    if number("unplaced") > 0 {
        text.push_str(&format!(
            " · {} skipped, their file is unclear",
            count(number("unplaced"))
        ));
    }
    // Links from a Rust crate into one it is not built with: the graph
    // holds them, and no map draws or counts them.
    if number("ruled_out") > 0 {
        text.push_str(&format!(
            " · {} ruled out by Cargo manifests",
            count(number("ruled_out"))
        ));
    }
    text
}

pub(super) fn cycle_comparison(view: &V) -> String {
    let comparison = &view["cycle"]["comparison"];
    let commit = m::text(&comparison["base"], "commit");
    let at = if commit.is_empty() {
        "the PR base".to_owned()
    } else {
        format!("base {commit}")
    };
    match comparison["state"].as_str() {
        Some("waiting_head") => {
            "Waiting for the PR graph before comparing with its base.".to_owned()
        }
        Some("building") => format!("Preparing the graph at {at}…"),
        Some("unavailable") => match comparison["reason"].as_str() {
            Some("tests") => {
                "Base comparison is unavailable with Without tests for this cached revision."
                    .to_owned()
            }
            Some("no_base") => "The base revision of this PR could not be determined.".to_owned(),
            Some("head") => "Base comparison requires the graph of the PR revision.".to_owned(),
            _ => format!("Could not compare with {at}."),
        },
        Some("ready") => match comparison["origin"].as_str() {
            Some("present") => format!("A cycle through this place already existed at {at}."),
            Some("absent") => format!("No cycle through this place at {at}."),
            Some("missing") => format!("This place is absent from the graph at {at}."),
            _ => format!("Could not match this place at {at}."),
        },
        _ => String::new(),
    }
}
pub(super) fn cycle_revision(view: &V) -> String {
    if at_base(view) {
        format!(
            "Showing the cycle at PR base {}.",
            m::text(&view["base"], "commit")
        )
    } else {
        cycle_comparison(view)
    }
}
/// How much of what uses a run's change the run left unopened, for the line
/// above the map. Nothing where no run is shown, or where nothing uses its
/// change here.
pub(super) fn run_reach(view: &V) -> String {
    let reach = &view["run"]["reach"];
    let number = |key: &str| reach[key].as_u64().unwrap_or(0);
    match (number("users"), number("unopened")) {
        (0, _) => String::new(),
        (_, 0) => " · the run opened all that use them".to_owned(),
        (_, unopened) => format!(
            " · {} of those that use them not opened by the run",
            count(unopened)
        ),
    }
}
/// What a picture of a run says of the run itself: how much it opened and
/// changed, and what of its looking no step records.
pub(super) fn run_notes(view: &V) -> Vec<String> {
    let run = &view["run"];
    if !run.is_object() {
        return vec![];
    }
    let number = |key: &str| run[key].as_u64().unwrap_or(0);
    let mut parts = vec![format!(
        "The run opened {} and changed {}.",
        plural(number("opened"), "file", "files"),
        plural(number("edited"), "file", "files")
    )];
    // A command, a search and a helper show files that no step names.
    if number("shell_opened") > 0 {
        parts.push(format!(
            "{} read via shell.",
            plural(number("shell_opened"), "file", "files")
        ));
    }
    let unseen = [
        ("commands", "ran", "command", "commands"),
        ("searched", "searched", "time", "times"),
        ("helpers", "started", "helper", "helpers"),
    ]
    .iter()
    .filter(|(key, ..)| number(key) > 0)
    .map(|(key, did, one, many)| format!("{did} {}", plural(number(key), one, many)))
    .collect::<Vec<_>>();
    if let Some((last, first)) = unseen.split_last() {
        parts.push(format!(
            "It also {}: some file reads may be missing.",
            if first.is_empty() {
                last.clone()
            } else {
                format!("{} and {last}", first.join(", "))
            }
        ));
    }
    if m::yes(&run["limited"]) {
        parts.push("Its record stops before its end: what it did later is not known.".to_owned());
    }
    match number("outside") {
        0 => {}
        1 => parts.push("1 file it touched lies outside the project.".to_owned()),
        outside => parts.push(format!(
            "{} files it touched lie outside the project.",
            count(outside)
        )),
    }
    if m::yes(&run["elsewhere"]) {
        parts.push(
            "It worked in a folder of its own; the map is of the project's graph.".to_owned(),
        );
    }
    parts
}
/// Beside a picture of what a run changed: the files that use the change
/// where the picture is open and that the run never opened, the most linked
/// first. Each opens to what it holds, and the mark beside it looks into its
/// link with the changed file it uses most. Nothing where no run is shown
/// or nothing uses its change here.
pub(super) fn extract_cross_service_unopened(view: &V) -> (Vec<V>, u64) {
    let sibling_more = view["run"]["reach"]["cross_service_more"]
        .as_u64()
        .or_else(|| view["run"]["cross_service_more"].as_u64())
        .or_else(|| view["run"]["cross_service_unopened_more"].as_u64())
        .or_else(|| view["cross_service_unopened_more"].as_u64())
        .unwrap_or(0);
    let candidates = [
        &view["run"]["cross_service_unopened"],
        &view["run"]["unopened_contract_consumers"],
        &view["run"]["reach"]["cross_service_unopened"],
        &view["run"]["reach"]["unopened_contract_consumers"],
        &view["cross_service_unopened"],
        &view["unopened_contract_consumers"],
    ];
    for candidate in candidates {
        if let Some(arr) = candidate.as_array() {
            if !arr.is_empty() || sibling_more > 0 {
                return (arr.clone(), sibling_more);
            }
        } else if candidate.is_object() {
            let rows = m::array(&candidate["rows"]);
            let more = candidate["more"].as_u64().unwrap_or(sibling_more);
            if !rows.is_empty() || more > 0 {
                return (rows, more);
            }
        }
    }
    (Vec::new(), 0)
}

/// Renders sibling-service contract consumer files left unopened by a System Change run.
pub fn cross_service_unopened_html(view: &V) -> String {
    if view["run"]["comparison"].is_object() {
        return String::new();
    }
    let (rows, more) = extract_cross_service_unopened(view);
    if rows.is_empty() {
        return String::new();
    }
    let total = rows.len() as u64 + more;
    let mut html = format!(
        "<h4>Cross-service consumers, not opened by the run <span class=\"graph-map-total\">{}</span></h4><ul class=\"graph-map-rows\">",
        count(total)
    );
    for row in &rows {
        let svc = {
            let s = m::text(row, "service");
            if !s.is_empty() {
                s
            } else {
                let cs = m::text(row, "consumer_service");
                if !cs.is_empty() {
                    cs
                } else {
                    m::text(row, "consumer")
                }
            }
        };
        let path = {
            let p = m::text(row, "path");
            if !p.is_empty() {
                p
            } else {
                m::array(&row["unopened_sources"])
                    .into_iter()
                    .chain(m::array(&row["consumer_sources"]))
                    .chain(m::array(&row["sources"]))
                    .find_map(|v| v.as_str().map(str::to_owned).filter(|s| !s.is_empty()))
                    .unwrap_or_default()
            }
        };
        let contract = m::text(row, "contract");
        let kind = m::text(row, "kind");
        let display_name = if !path.is_empty() {
            format!("{svc} · {}", base(&path))
        } else if !contract.is_empty() {
            format!("{svc} · {contract}")
        } else {
            svc.clone()
        };
        let mut details = Vec::new();
        if !path.is_empty() && !parent(&path).is_empty() {
            details.push(escape(parent(&path)));
        }
        if !contract.is_empty() {
            details.push(escape(&contract));
        }
        if !kind.is_empty() {
            details.push(escape(&kind));
        }
        let jump_payload = payload(&json!({
            "service": svc,
            "path": path,
        }));
        html.push_str(&format!(
            "<li><button type=\"button\" class=\"graph-map-row\" data-unopened=\"true\" data-cross-service=\"true\" data-action=\"graph-map-switch-service-file\" data-payload=\"{jump_payload}\" title=\"Open {} in service {}\"><span class=\"graph-map-row-name\">{}</span><span class=\"graph-map-row-links\">{}</span></button></li>",
            escape(if path.is_empty() { &contract } else { &path }),
            escape(&svc),
            escape(&display_name),
            details.join(" · "),
        ));
    }
    html.push_str("</ul>");
    if more > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">and {} more cross-service consumers</p>",
            count(more)
        ));
    }
    html
}

pub(super) fn unopened(view: &V) -> String {
    if view["run"]["comparison"].is_object() {
        return String::new();
    }
    let cross_html = cross_service_unopened_html(view);
    let reach = &view["run"]["reach"];
    let number = |key: &str| reach[key].as_u64().unwrap_or(0);
    if number("users") == 0 {
        return cross_html;
    }
    let mut html = format!(
        "<h4>Use the change, not opened by the run <span class=\"graph-map-total\">{}</span></h4>",
        count(number("unopened"))
    );
    let rows = m::array(&reach["rows"]);
    if rows.is_empty() {
        html.push_str("<p class=\"knowledge-note\">The run opened every file that uses what it changed here.</p>");
        html.push_str(&cross_html);
        return html;
    }
    html.push_str("<ul class=\"graph-map-rows\">");
    for row in &rows {
        let uses = row["uses"].as_u64().unwrap_or(0);
        // The name of the file first: a long path would be cut at its end,
        // where the name is. Its folder stands under it, with the links.
        let path = m::text(row, "path");
        let folder = parent(&path);
        html.push_str(&format!(
            "<li><button type=\"button\" class=\"graph-map-row\" data-unopened=\"true\" data-action=\"graph-map-open\" data-payload=\"{}\" title=\"{} · select to open it\"><span class=\"graph-map-row-name\">{}</span><span class=\"graph-map-row-links\">{}{}{}{}</span></button><button type=\"button\" class=\"graph-map-row-look\" data-action=\"graph-map-link\" data-payload=\"{}\" title=\"See what makes up this link\" aria-label=\"See what makes up the link with {}\">↗</button></li>",
            payload(&row["path"]),
            escape(&path),
            escape(base(&path)),
            if folder.is_empty() {
                String::new()
            } else {
                format!("{} · ", escape(folder))
            },
            weight(
                row["links"].as_u64().unwrap_or(0),
                row["guessed"].as_u64().unwrap_or(0)
            ),
            if uses > 1 {
                format!(" · over {} changed files", count(uses))
            } else {
                String::new()
            },
            if m::yes(&row["test"]) { " · test" } else { "" },
            payload(&json!({"from": row["path"], "to": row["to"]})),
            escape(base(&m::text(row, "to"))),
        ));
    }
    html.push_str("</ul>");
    if number("more") > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">and {} more</p>",
            count(number("more"))
        ));
    }
    html.push_str(&cross_html);
    html
}
/// What the map of a change says first: the notes of the change, what a
/// run did beside changing it, and whether the graph is behind the files.
pub(super) fn context(view: &V) -> Vec<String> {
    let mut parts = m::array(&view["chosen"]["notes"])
        .iter()
        .filter_map(|note| note.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    parts.extend(run_notes(view));
    if m::yes(&view["stale"]) {
        parts.push(
            if m::yes(&view["rebuilding"]) {
                "The graph is older than the files: it is being rebuilt and the map will follow."
            } else {
                "The graph is older than the files, so the links of edited files may be out of date. Rebuild the graph to refresh them."
            }
            .to_owned(),
        );
    }
    parts
}
/// What an opened file leaves out or has nothing to show for.
/// How far a change reached into an opened file: which of its symbols it
/// touched, or why that is not told.
pub(super) fn reach(view: &V) -> Option<String> {
    let file = &view["file"];
    file["status"].as_str()?;
    // A file that holds no symbol has none to mark.
    if file["symbols"].as_u64().unwrap_or(0) == 0 {
        return None;
    }
    let touched = &file["touched"];
    if !touched.is_object() {
        return Some(
            if m::yes(&view["stale"]) || m::yes(&view["preliminary"]) {
                "The symbols the change touched are marked once the graph is of these files."
            } else {
                "Which symbols the change touched could not be told from the file."
            }
            .to_owned(),
        );
    }
    if matches!(file["status"].as_str(), Some("A" | "C")) {
        return Some(
            "The whole file is new in this change, so every symbol in it is added.".to_owned(),
        );
    }
    let number = |key: &str| touched[key].as_u64().unwrap_or(0);
    let outside = m::yes(&touched["top"]);
    let mut text = match number("symbols") {
        0 if outside => "The change touched no symbol here, only lines outside them.".to_owned(),
        0 => "The change touched no line of a symbol here.".to_owned(),
        symbols => {
            let kinds = [("added", "added"), ("modified", "modified")]
                .iter()
                .filter(|(key, _)| number(key) > 0)
                .map(|(key, word)| format!("{} {word}", count(number(key))))
                .collect::<Vec<_>>();
            format!(
                "The change touched {} of {}: {}{}.",
                count(symbols),
                plural(file["symbols"].as_u64().unwrap_or(0), "symbol", "symbols"),
                kinds.join(", "),
                if outside {
                    ", and lines outside them"
                } else {
                    ""
                }
            )
        }
    };
    match number("between") {
        0 => {}
        1 => text.push_str(" Lines were also removed between symbols in 1 place."),
        places => text.push_str(&format!(
            " Lines were also removed between symbols in {} places.",
            count(places)
        )),
    }
    Some(text)
}
pub(super) fn file_note(view: &V) -> String {
    let number = |value: &V| value.as_u64().unwrap_or(0);
    let shown = m::array(&view["nodes"])
        .iter()
        .filter(|node| m::yes(&node["symbol"]))
        .count() as u64;
    // The selected symbol is drawn whatever the limit and the filter say.
    let beyond = u64::from(m::yes(&view["beyond"]));
    let most = shown.saturating_sub(beyond);
    let drawn = m::array(&view["edges"]).len() as u64;
    let mut parts = context(view);
    parts.extend(reach(view));
    if number(&view["children"]) == 0 {
        parts.push("The graph names nothing inside this file.".to_owned());
    } else if number(&view["matching"]) == 0 {
        parts.push("No name in this file matches the filter.".to_owned());
    } else if most < number(&view["matching"]) {
        parts.push(format!(
            "Showing the {} most linked of {} symbols in this file; filter by name to reach the rest.",
            count(most),
            count(number(&view["matching"]))
        ));
    }
    if beyond > 0 {
        parts.push("The selected one is drawn as well.".to_owned());
    }
    let around = &view["around"];
    if number(&around["shown"]) < number(&around["files"]) {
        parts.push(format!(
            "Showing the {} most linked of {} files around them.",
            count(number(&around["shown"])),
            count(number(&around["files"]))
        ));
    }
    if shown > 0 && drawn == 0 {
        parts.push(
            if m::yes(&view["certain"]) {
                "No link read from the code touches these."
            } else {
                "Nothing is linked to these."
            }
            .to_owned(),
        );
    } else if drawn > 0 && drawn < number(&view["between"]) {
        parts.push(format!(
            "Drawing the {} strongest of {} links.",
            count(drawn),
            count(number(&view["between"]))
        ));
    }
    parts.join(" ")
}
pub(super) fn change_note(view: &V) -> String {
    let change = &view["change"];
    let number = |value: &V| value.as_u64().unwrap_or(0);
    let nodes = m::array(&view["nodes"]);
    let around = nodes
        .iter()
        .filter(|node| number(&node["changed"]) == 0)
        .count() as u64;
    let mut parts = context(view);
    if number(&change["files"]) == 0 {
        parts.push("Nothing is changed here.".to_owned());
    } else if number(&change["graphed"]) == 0 {
        parts.push(
            "None of the changed files is in the graph: the tool found no code in them.".to_owned(),
        );
    } else {
        if !m::yes(&change["single"]) {
            parts.push(
                "The change is too wide to draw file by file, so it is drawn by folder. Select a folder twice to open it."
                    .to_owned(),
            );
        }
        match number(&change["hidden"]) {
            0 => {}
            1 => parts.push("1 more place with changes is not drawn.".to_owned()),
            hidden => parts.push(format!(
                "{} more places with changes are not drawn.",
                count(hidden)
            )),
        }
        if number(&view["children"]) > 0 && number(&view["matching"]) == 0 {
            parts.push("No linked file matches the filter.".to_owned());
        } else if around < number(&view["matching"]) {
            parts.push(format!(
                "Showing the {} most linked of {} files around the change; filter by name to reach the rest.",
                count(around),
                count(number(&view["matching"]))
            ));
        }
        let drawn = m::array(&view["edges"]).len() as u64;
        if !nodes.is_empty() && drawn == 0 {
            parts.push(if m::yes(&view["certain"]) {
                "No link read from the code touches the change.".to_owned()
            } else {
                "Nothing links the change to other files.".to_owned()
            });
        } else if drawn < number(&view["between"]) {
            parts.push(format!(
                "Drawing the {} strongest of {} links.",
                count(drawn),
                count(number(&view["between"]))
            ));
        }
    }
    parts.join(" ")
}
/// What the map leaves out or has nothing to show for, if anything.
/// How the links followed from a place are unfolded, and what the picture
/// leaves out.
pub(super) fn follow_note(view: &V) -> String {
    let followed = &view["followed"];
    let number = |key: &str| followed[key].as_u64().unwrap_or(0);
    let mut parts = context(view);
    // Two symbols are linked once: there the order is what was read from
    // the code, then where it lies.
    parts.push(format!(
        "+ at the right corner of a box shows more of what it uses, + at the left corner more of what uses it, {} at a time, {}; − folds them back.",
        count(number("step")),
        if names(view) {
            "what was read from the code first"
        } else {
            "strongest first"
        }
    ));
    if m::yes(&followed["full"]) {
        parts.push(format!(
            "The picture holds {} boxes: fold something to unfold more.",
            count(number("limit"))
        ));
    }
    // A link shows where one of its sides is unfolded; the rest are counted.
    match number("aside") {
        0 => {}
        1 => parts.push(
            "1 more link runs between these boxes; it shows where a side that holds it is unfolded."
                .to_owned(),
        ),
        aside => parts.push(format!(
            "{} more links run between these boxes; they show where a side that holds them is unfolded.",
            count(aside)
        )),
    }
    let drawn = m::array(&view["edges"]).len() as u64;
    let between = view["between"].as_u64().unwrap_or(0);
    if drawn > 0 && drawn < between {
        parts.push(format!(
            "Drawing the {} strongest of {} links.",
            count(drawn),
            count(between)
        ));
    }
    parts.join(" ")
}
/// What a path is made of, which way it runs, and what the picture
/// leaves out; or why there is none.
pub(super) fn path_note(view: &V) -> String {
    let path = &view["path"];
    let number = |key: &str| path[key].as_u64().unwrap_or(0);
    let (from, to) = (m::text(&path["from"], "name"), m::text(&path["to"], "name"));
    let mut parts = context(view);
    if m::yes(&path["within"]) {
        parts.push(format!(
            "One of {from} and {to} lies in the other, so no path runs between them. Choose two places that lie apart."
        ));
        return parts.join(" ");
    }
    if !m::yes(&path["found"]) {
        parts.push(format!(
            "No chain of links leads from {from} to {to}, nor back. The two may still share something that both use, or that uses both: What they share, beside the path above the picture, shows it."
        ));
        if m::yes(&view["certain"]) {
            parts.push(
                "Links guessed by name are left out; with them there may be a way.".to_owned(),
            );
        }
        return parts.join(" ");
    }
    if m::yes(&path["reversed"]) {
        parts.push(format!(
            "Nothing leads from {from} to {to}: this is the way from {to} to {from}."
        ));
    }
    parts.push(
        "Each box uses the next: these are the ways that take the fewest links. Select a link to see what makes it up."
            .to_owned(),
    );
    // With them the ways round that were asked for, as far as there are any.
    let longer = number("longer");
    if longer > 0 {
        let by = plural(longer, "link", "links");
        parts.push(match number("detours") {
            0 => format!("No way that is longer by {by} at most leads there."),
            found => format!(
                "Drawn lighter with them: {} longer by {by} at most, each whole{}",
                plural(found, "way", "ways"),
                if m::yes(&path["detours_more"]) {
                    "; there are more."
                } else {
                    "."
                }
            ),
        });
    }
    if number("shown") < number("places") {
        parts.push(format!(
            "{} places lie on such ways; each step shows its {} most linked.",
            count(number("places")),
            count(number("wide"))
        ));
    }
    let drawn = m::array(&view["edges"]).len() as u64;
    let between = view["between"].as_u64().unwrap_or(0);
    if drawn > 0 && drawn < between {
        // No box is left without its way: the strongest link into each and
        // out of each is drawn first.
        parts.push(format!(
            "Drawing {} of {} links: the strongest into each box and out of it, then the strongest of the rest.",
            count(drawn),
            count(between)
        ));
    }
    parts.join(" ")
}
/// What the picture of what two places share shows and what counts as
/// shared, and, where there is more than it shows, in what order; or why
/// there is nothing.
pub(super) fn shared_note(view: &V) -> String {
    let shared = &view["shared"];
    let number = |key: &str| shared[key].as_u64().unwrap_or(0);
    let (from, to) = (
        m::text(&shared["from"], "name"),
        m::text(&shared["to"], "name"),
    );
    let mut parts = context(view);
    if m::yes(&shared["within"]) {
        parts.push(format!(
            "One of {from} and {to} lies in the other, so the two are not apart. Choose two places that lie apart."
        ));
        return parts.join(" ");
    }
    // What one of the two reaches only through the other is the path's to
    // show.
    let through =
        "A way that passes the other of the two does not count: the path between them shows those.";
    let (used, users) = (number("both_use"), number("uses_both"));
    if used + users == 0 {
        parts.push(format!(
            "Nothing is used by both {from} and {to}, and nothing uses both. {through}"
        ));
        if m::yes(&view["certain"]) {
            parts.push(
                "Links guessed by name are left out; with them there may be something.".to_owned(),
            );
        }
        return parts.join(" ");
    }
    let above = above(view);
    if (if above { users } else { used }) == 0 {
        // The kind that was asked for has nothing, and the other has: the
        // line above says how much, and this where it is shown.
        parts.push(if above {
            format!("Nothing uses both {from} and {to}. {through} What they use in common is another picture: What both use, beside the path above this one.")
        } else {
            format!("{from} and {to} use nothing in common. {through} What uses both is another picture: What uses both, beside the path above this one.")
        });
        return parts.join(" ");
    }
    let (matching, skip, shown) = (number("matching"), number("skip"), number("shown"));
    if matching == 0 {
        // There are places of the kind, and the filter leaves none.
        parts.push(if above {
            format!(
                "None of the {} that use both matches the filter.",
                count(users)
            )
        } else {
            format!(
                "None of the {} that both use matches the filter.",
                count(used)
            )
        });
        return parts.join(" ");
    }
    parts.push(format!(
        "{} {through}",
        if above {
            "What uses both stands to the left of the two, each with one shortest way to either of them, the strongest of those as short."
        } else {
            "What both use stands to the right of the two, each with one shortest way from either of them, the strongest of those as short."
        }
    ));
    // Where one picture does not show them all, the line above and the
    // strip beside the picture say which it shows, and this in what order
    // they stand and how one of them is reached.
    if skip > 0 || shown < matching {
        parts.push(format!(
            "The nearest come first: the fewest links {} the two taken together, then the most linked. Filter names reaches a place by its name.",
            if above { "to" } else { "from" }
        ));
    }
    parts.join(" ")
}
/// What a chosen change did to the order of the layers, for the line above
/// their picture: through how many links it set files against the order,
/// and how many such links it took away. Nothing where no change is chosen
/// or the two ends of it are not compared yet.
pub(super) fn changed_order(change: &V) -> String {
    if change["state"] != "ready" {
        return String::new();
    }
    let links = |key: &str| change[key]["links"].as_u64().unwrap_or(0);
    match (links("added"), links("removed")) {
        (0, 0) => " · this change adds none".to_owned(),
        (added, 0) => format!(" · this change adds {}", plural(added, "link", "links")),
        (0, removed) => format!(
            " · this change takes {} away",
            plural(removed, "link", "links")
        ),
        (added, removed) => format!(
            " · this change adds {} and takes {} away",
            plural(added, "link", "links"),
            count(removed)
        ),
    }
}
/// One piece of a drawn link: a curve by its two handles and its end, or a
/// straight run to a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Piece {
    Curve([(f64, f64); 3]),
    Line((f64, f64)),
}
/// The pieces of a link as the pictures draw them: `M x y`, then curves
/// `C x y x y x y` and straight runs `L x y` in any order. None for any
/// other path.
pub(super) fn pieces(path: &str) -> Option<((f64, f64), Vec<Piece>)> {
    let mut runs: Vec<(char, Vec<f64>)> = Vec::new();
    for token in path.split_whitespace() {
        let number = match token.chars().next()? {
            command @ ('M' | 'C' | 'L') => {
                runs.push((command, Vec::new()));
                &token[1..]
            }
            _ => token,
        };
        if !number.is_empty() {
            runs.last_mut()?.1.push(number.parse().ok()?);
        }
    }
    let mut runs = runs.into_iter();
    let start = match runs.next()? {
        ('M', at) if at.len() == 2 => (at[0], at[1]),
        _ => return None,
    };
    let pieces = runs
        .map(|(command, at)| match (command, &at[..]) {
            ('C', [x1, y1, x2, y2, x3, y3]) => {
                Some(Piece::Curve([(*x1, *y1), (*x2, *y2), (*x3, *y3)]))
            }
            ('L', [x, y]) => Some(Piece::Line((*x, *y))),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    (!pieces.is_empty()).then_some((start, pieces))
}
/// A point of a curve and the way it runs there, `t` of the way along it.
pub(super) fn on_curve(
    from: (f64, f64),
    [a, b, to]: [(f64, f64); 3],
    t: f64,
) -> ((f64, f64), (f64, f64)) {
    let s = 1.0 - t;
    let at = |p0: f64, p1: f64, p2: f64, p3: f64| {
        s * s * s * p0 + 3.0 * s * s * t * p1 + 3.0 * s * t * t * p2 + t * t * t * p3
    };
    let way = |p0: f64, p1: f64, p2: f64, p3: f64| {
        3.0 * s * s * (p1 - p0) + 6.0 * s * t * (p2 - p1) + 3.0 * t * t * (p3 - p2)
    };
    (
        (at(from.0, a.0, b.0, to.0), at(from.1, a.1, b.1, to.1)),
        (way(from.0, a.0, b.0, to.0), way(from.1, a.1, b.1, to.1)),
    )
}
/// How finely a curve is measured.
pub(super) const CURVE_STEPS: usize = 16;
/// The point halfway along a drawn link, where a word about it can stand,
/// and the angle the link runs at there, in degrees clockwise from the
/// right. A link of one curve is halved where the curve is; a link of
/// several pieces, as one that passes layers in slots of its own, is
/// halved by its length, so the point lies on it whatever its shape.
pub(super) fn halfway(path: &str) -> Option<(f64, f64, f64)> {
    let (start, pieces) = pieces(path)?;
    let angle = |(dx, dy): (f64, f64)| dy.atan2(dx).to_degrees();
    if let [Piece::Curve(curve)] = pieces[..] {
        let ((x, y), way) = on_curve(start, curve, 0.5);
        return Some((x, y, angle(way)));
    }
    // The link as short straight steps, each with where it ends.
    let mut steps: Vec<((f64, f64), (f64, f64))> = Vec::new();
    let mut from = start;
    for piece in &pieces {
        match *piece {
            Piece::Line(to) => {
                steps.push((from, to));
                from = to;
            }
            Piece::Curve(curve) => {
                let mut last = from;
                for step in 1..=CURVE_STEPS {
                    let (point, _) = on_curve(from, curve, step as f64 / CURVE_STEPS as f64);
                    steps.push((last, point));
                    last = point;
                }
                from = curve[2];
            }
        }
    }
    let long = |(a, b): &((f64, f64), (f64, f64))| (b.0 - a.0).hypot(b.1 - a.1);
    let mut left = steps.iter().map(long).sum::<f64>() / 2.0;
    for step in &steps {
        let length = long(step);
        if length >= left && length > 0.0 {
            let (a, b) = *step;
            let part = left / length;
            return Some((
                a.0 + (b.0 - a.0) * part,
                a.1 + (b.1 - a.1) * part,
                angle((b.0 - a.0, b.1 - a.1)),
            ));
        }
        left -= length;
    }
    let (a, b) = *steps.last()?;
    Some((b.0, b.1, angle((b.0 - a.0, b.1 - a.1))))
}
/// The middle of a drawn link, where a word about it can stand.
pub(super) fn middle(path: &str) -> Option<(f64, f64)> {
    halfway(path).map(|(x, y, _)| (x, y))
}
/// Whether a drawn link passes layers on its way: it then runs in several
/// pieces, straight through the slot of each layer it passes. Such a link
/// is long, and its head is far from where it leaves.
pub(super) fn passes_layers(path: &str) -> bool {
    path.contains(" L") && path.contains(" C")
}
/// Whether what a change did to the order of the layers is still being found
/// out, so that the picture is asked for again.
pub(super) fn comparing_order(view: &V) -> bool {
    matches!(
        view["pattern"]["change"]["state"].as_str(),
        Some("building" | "waiting_head")
    )
}
/// Where a change is chosen and what it did to the order of the layers is
/// not told: why not, or what is waited for.
pub(super) fn order_comparison(change: &V) -> String {
    let commit = m::text(&change["base"], "commit");
    let at = if commit.is_empty() {
        "the base of the change".to_owned()
    } else {
        format!("base {commit}")
    };
    match change["state"].as_str() {
        Some("waiting_head") => {
            "Waiting for the graph of the change before telling what it did to the order."
                .to_owned()
        }
        Some("building") => {
            format!("Preparing the graph at {at} to tell what the change did to the order…")
        }
        Some("unavailable") => match change["reason"].as_str() {
            Some("tests") => {
                "What the change did to the order cannot be told with Without tests for this cached revision."
                    .to_owned()
            }
            Some("no_base") => {
                "Where this change started could not be determined, so what it did to the order is not told."
                    .to_owned()
            }
            Some("head") => {
                "What the change did to the order needs the graph of the change itself."
                    .to_owned()
            }
            Some("stale") => {
                "What the change did to the order is told once the graph is of these files."
                    .to_owned()
            }
            _ => format!("Could not compare with {at}."),
        },
        _ => String::new(),
    }
}
/// Above the picture of one found pattern: what the pattern is, what tells
/// it here, and how far that is to be believed.
pub(super) fn pattern_note(view: &V) -> String {
    let pattern = &view["pattern"];
    let mut parts = vec![m::text(pattern, "tells")];
    parts.extend(
        m::array(&pattern["told"])
            .iter()
            .filter_map(|told| told.as_str().map(str::to_owned)),
    );
    if m::yes(&pattern["guessed"]) {
        parts.push("A hollow link is one the tool only guessed.".to_owned());
    }
    let comparison = order_comparison(&pattern["change"]);
    if !comparison.is_empty() {
        parts.push(comparison);
    }
    parts.join(" ")
}
pub fn note(view: &V) -> String {
    if view["mode"] == "scenario" {
        return String::new();
    }
    if view["mode"] == "modules" {
        let shown = m::array(&view["nodes"]).len() as u64;
        let matching = view["matching"].as_u64().unwrap_or(0);
        return if matching > shown {
            "Filter by module name, ID or file path to find other modules.".to_owned()
        } else {
            String::new()
        };
    }
    if symbols(view) {
        return file_note(view);
    }
    if patterned(view) {
        return pattern_note(view);
    }
    if paths(view) {
        return path_note(view);
    }
    if cycles(view) {
        let mut parts = context(view);
        if view["cycle"]["places"].is_array() {
            parts.push("The full chain is listed beside the map.".to_owned());
        }
        if view["cycle"]["steps"] == 0 && !m::yes(&view["cycle"]["origin"]["missing"]) {
            parts.push(
                "Arrows between grouped folders may join different files. The cycle search follows connected file or symbol links."
                    .to_owned(),
            );
        } else if view["cycle"]["origin"]["what"] == "folder" {
            parts.push(
                "The chain can leave and re-enter the folder through different files.".to_owned(),
            );
        }
        return parts.join(" ");
    }
    if shares(view) {
        return shared_note(view);
    }
    if follows(view) {
        return follow_note(view);
    }
    if changes(view) {
        return change_note(view);
    }
    let number = |key: &str| view[key].as_u64().unwrap_or(0);
    let shown = m::array(&view["nodes"]).len() as u64;
    // The selected place is drawn whatever the limit and the filter say.
    let beyond = u64::from(m::yes(&view["beyond"]));
    let most = shown.saturating_sub(beyond);
    let drawn = m::array(&view["edges"]).len() as u64;
    let mut parts = vec![];
    if number("children") == 0 {
        parts.push("The graph names no files here.".to_owned());
    } else if number("matching") == 0 {
        parts.push("No name here matches the filter.".to_owned());
    } else if most < number("matching") {
        parts.push(format!(
            "Showing the {} most connected of {} here; filter by name to reach the rest.",
            count(most),
            count(number("matching"))
        ));
    }
    if beyond > 0 {
        parts.push("The selected one is drawn as well.".to_owned());
    }
    if shown > 0 && drawn == 0 {
        parts.push(if m::yes(&view["certain"]) {
            "No link read from the code connects these.".to_owned()
        } else {
            "Nothing here is linked to anything else here.".to_owned()
        });
    } else if drawn > 0 && drawn < number("between") {
        parts.push(if m::yes(&view["backbone"]) {
            format!(
                "Too many links to draw at once: showing each one's 3 strongest, {} of {}. Select one to list all of its links.",
                count(drawn),
                count(number("between"))
            )
        } else {
            format!(
                "Drawing the {} strongest of {} links.",
                count(drawn),
                count(number("between"))
            )
        });
    }
    if view["lod"].is_string()
        || view["lod"].is_object()
        || m::yes(&view["truncated"])
        || m::yes(&view["telemetry"]["truncated"])
    {
        let lod_label = view["lod"]
            .as_str()
            .or_else(|| view["lod"]["tier"].as_str())
            .unwrap_or("overview");
        parts.push(format!(
            "Progressive LOD ({lod_label}): zoom in or focus a node for full detail."
        ));
    }
    parts.join(" ")
}
/// Which a box does more through the links drawn here, in five even steps
/// from only using to only being used. A box no link here touches leans
/// neither way.
pub(super) fn leaning(uses: u64, used: u64) -> &'static str {
    // Wide enough that the largest counts still compare as they are.
    let (fifths, whole) = (u128::from(used) * 5, u128::from(uses) + u128::from(used));
    if whole == 0 {
        ""
    } else if fifths <= whole {
        "uses"
    } else if fifths < whole * 2 {
        "uses-more"
    } else if fifths <= whole * 3 {
        "even"
    } else if fifths < whole * 4 {
        "used-more"
    } else {
        "used"
    }
}
/// Classifies a node in an overview map (`patterns` enabled) by which pattern
/// families (`arch`, `design`, or `both`) have listed findings inside it.
pub(super) fn pattern_family_of(view: &V, path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return None;
    }
    let prefix = format!("{path}/");
    let (mut arch, mut design) = (false, false);
    for kind in m::array(&view["patterns"]["kinds"]) {
        let is_arch = kind["family"] == "architecture";
        let hits = m::array(&kind["instances"]).into_iter().any(|row| {
            let file = row["file"].as_str().unwrap_or("");
            let id = row["id"].as_str().unwrap_or("");
            file == path
                || file.starts_with(&prefix)
                || (is_arch && kind["id"] == "layers" && id == format!("layers@{path}"))
        });
        if hits {
            if is_arch {
                arch = true;
            } else {
                design = true;
            }
        }
    }
    match (arch, design) {
        (true, true) => Some("both"),
        (true, false) => Some("arch"),
        (false, true) => Some("design"),
        (false, false) => None,
    }
}
/// Shortens verbose edge relationship phrases for compact canvas badges.
pub(super) fn short_tie(tie: &str) -> String {
    tie.replace(" and ", " & ")
}
/// Finds a non-overlapping label coordinate near an edge's midpoint, or
/// returns `None` if all small vertical offsets would collide.
pub(super) fn place_tie(placed: &mut Vec<(f64, f64)>, (x, y): (f64, f64)) -> Option<(f64, f64)> {
    for dy in [0.0, -10.0, 10.0] {
        let candidate = (x, y + dy);
        let clear = placed
            .iter()
            .all(|&(px, py)| (px - candidate.0).abs() >= 64.0 || (py - candidate.1).abs() >= 14.0);
        if clear {
            placed.push(candidate);
            return Some(candidate);
        }
    }
    None
}
/// Accent color shared by overview layer chips and dedicated layer view nodes.
pub(super) fn layer_accent(id: u64) -> String {
    format!("hsl({} 68% 46%)", (id * 137 + 220) % 360)
}
/// The map itself: links as curves underneath, one button per folder or
/// file, or, in an opened file, per symbol and per file around. Colour tells
/// the direction: a link runs from the colour of using to the colour of being
/// used, and a box is tinted by which of the two it does more here.
pub(super) fn entity_change_badge(
    change: Option<&str>,
) -> (&'static str, &'static str, &'static str) {
    match change.map(str::to_ascii_lowercase).as_deref() {
        Some("modified") => ("modified", "MOD", "Modified"),
        Some("deleted") => ("deleted", "DEL", "Deleted"),
        _ => ("created", "NEW", "New"),
    }
}

pub(super) fn change_status_badge(status: &str) -> (&'static str, &'static str) {
    match status.to_ascii_lowercase().as_str() {
        "created" | "added" => ("NEW", "Created"),
        "modified" => ("MOD", "Modified"),
        "deleted" | "removed" => ("DEL", "Deleted"),
        "renamed" => ("REN", "Renamed"),
        _ => ("", "Unchanged"),
    }
}

pub(super) fn path_matches_node(node_path: &str, is_folder: bool, file_path: &str) -> bool {
    let norm_node = normalize_rel_path(node_path);
    let norm_file = normalize_rel_path(strip_symbol_suffix(file_path));
    if norm_node.is_empty() || norm_file.is_empty() {
        return false;
    }
    if norm_node == norm_file {
        return true;
    }
    is_folder && norm_file.starts_with(&format!("{norm_node}/"))
}

pub(super) fn resolve_canvas_node_by_path(nodes: &[V], target_path: &str) -> Option<usize> {
    let norm_target = normalize_rel_path(strip_symbol_suffix(target_path));
    if norm_target.is_empty() {
        return None;
    }
    if let Some(exact) = nodes
        .iter()
        .position(|node| normalize_rel_path(&m::text(node, "path")) == norm_target)
    {
        return Some(exact);
    }
    nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| m::yes(&node["folder"]))
        .filter_map(|(idx, node)| {
            let folder_path = normalize_rel_path(&m::text(node, "path"));
            if !folder_path.is_empty() && norm_target.starts_with(&format!("{folder_path}/")) {
                Some((folder_path.len(), idx))
            } else {
                None
            }
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, idx)| idx)
}

pub(super) fn normalize_change_status(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "created" | "added" | "a" | "c" => "created".to_owned(),
        "modified" | "m" => "modified".to_owned(),
        "deleted" | "removed" | "d" => "deleted".to_owned(),
        "renamed" | "r" => "renamed".to_owned(),
        other => other.to_owned(),
    }
}

pub(super) fn change_status_letter(status: &str) -> &'static str {
    match status {
        "created" => "A",
        "modified" => "M",
        "deleted" => "D",
        "renamed" => "R",
        _ => "",
    }
}

pub(super) fn clean_symbol_token(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(stripped) = s.strip_suffix("()") {
        s = stripped;
    }
    if let Some(stripped) = s.strip_prefix('.') {
        s = stripped;
    }
    s.trim().to_owned()
}

pub(super) fn symbol_name_matches(node_name: &str, node_path: &str, entity_name: &str) -> bool {
    let target = clean_symbol_token(entity_name);
    if target.is_empty() {
        return false;
    }
    let clean_node = clean_symbol_token(node_name);
    if clean_node == target
        || clean_node
            .rsplit_once('.')
            .is_some_and(|(_, member)| member == target)
        || clean_node
            .rsplit_once("::")
            .is_some_and(|(_, member)| member == target)
    {
        return true;
    }
    if let Some((_, suffix)) = node_path.split_once('#') {
        let after_line = suffix.split_once(':').map_or(suffix, |(_, name)| name);
        let clean_suffix = clean_symbol_token(after_line);
        if clean_suffix == target
            || clean_suffix
                .rsplit_once('.')
                .is_some_and(|(_, member)| member == target)
            || clean_suffix
                .rsplit_once("::")
                .is_some_and(|(_, member)| member == target)
        {
            return true;
        }
    }
    false
}

pub(super) struct ChangedEntityDiff {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) line: u64,
    pub(super) change: String,
}

pub(super) struct ChangesLookup {
    pub(super) exact: BTreeMap<String, String>,
    pub(super) files: Vec<(String, String)>,
    pub(super) entities: BTreeMap<String, Vec<ChangedEntityDiff>>,
}

impl ChangesLookup {
    pub(super) fn from_value(doc: &V) -> Option<Self> {
        let arr = doc["files"].as_array()?;
        let mut exact = BTreeMap::new();
        let mut files = Vec::new();
        let mut entities: BTreeMap<String, Vec<ChangedEntityDiff>> = BTreeMap::new();
        for item in arr {
            let path = normalize_rel_path(&m::text(item, "path"));
            let status = normalize_change_status(&m::text(item, "status"));
            if path.is_empty() || status.is_empty() {
                continue;
            }
            exact.insert(path.clone(), status.clone());
            files.push((path.clone(), status));
            let mut file_entities = Vec::new();
            for entity in m::array(&item["entities"]) {
                let raw_change = entity["change"]
                    .as_str()
                    .or_else(|| entity["status"].as_str())
                    .unwrap_or("");
                let change = normalize_change_status(raw_change);
                if !matches!(
                    change.as_str(),
                    "created" | "modified" | "deleted" | "renamed"
                ) {
                    continue;
                }
                let id = m::text(&entity, "id");
                let name = m::text(&entity, "name");
                let line = entity["line"].as_u64().unwrap_or(0);
                if id.is_empty() && name.is_empty() {
                    continue;
                }
                file_entities.push(ChangedEntityDiff {
                    id,
                    name,
                    line,
                    change,
                });
            }
            if !file_entities.is_empty() {
                entities.insert(path, file_entities);
            }
        }
        Some(Self {
            exact,
            files,
            entities,
        })
    }

    pub(super) fn entity_status_for(&self, node: &V) -> Option<String> {
        let node_path = m::text(node, "path");
        let parent_file = {
            let from_path = normalize_rel_path(strip_symbol_suffix(&node_path));
            if from_path.is_empty() {
                normalize_rel_path(&m::text(node, "file"))
            } else {
                from_path
            }
        };
        if parent_file.is_empty() {
            return None;
        }
        let node_id = m::text(node, "id");
        let node_name = m::text(node, "name");
        let node_line = node["line"].as_u64().unwrap_or(0);
        if let Some(diffs) = self.entities.get(&parent_file) {
            for entity in diffs {
                let id_match = !entity.id.is_empty()
                    && (node_path == entity.id || (!node_id.is_empty() && node_id == entity.id));
                let line_ok = entity.line == 0 || node_line == 0 || entity.line == node_line;
                if id_match
                    || (line_ok && symbol_name_matches(&node_name, &node_path, &entity.name))
                {
                    return Some(entity.change.clone());
                }
            }
        }
        let node_changed = node["changed"].as_u64().unwrap_or(0) > 0 || m::yes(&node["changed"]);
        if !node_changed {
            return None;
        }
        if let Some(raw_status) = node["status"].as_str() {
            let norm = normalize_change_status(raw_status);
            if matches!(
                norm.as_str(),
                "created" | "modified" | "deleted" | "renamed"
            ) {
                return Some(norm);
            }
        }
        self.exact.get(&parent_file).cloned()
    }

    pub(super) fn folder_status_for(
        &self,
        folder_path: &str,
        node: Option<&V>,
        all_nodes: &[V],
    ) -> Option<String> {
        let prefix = format!("{folder_path}/");
        let mut seen = BTreeSet::new();
        for (file, status) in &self.files {
            if file.starts_with(&prefix) {
                seen.insert(status.as_str());
            }
        }
        if seen.is_empty() {
            return None;
        }
        if seen.len() > 1 {
            return Some("modified".to_owned());
        }
        let only = seen.into_iter().next()?;
        if only != "deleted" {
            return Some(only.to_owned());
        }
        if let Some(node) = node {
            if m::yes(&node["synthetic_deleted"]) || node["status"].as_str() == Some("D") {
                return Some("deleted".to_owned());
            }
            if let Some(files_count) = node["files"].as_u64() {
                return Some(if files_count == 0 {
                    "deleted".to_owned()
                } else {
                    "modified".to_owned()
                });
            }
        }
        let has_surviving_child = all_nodes.iter().any(|other| {
            if m::yes(&other["synthetic_deleted"]) {
                return false;
            }
            let other_path = normalize_rel_path(&m::text(other, "path"));
            other_path.starts_with(&prefix)
                && self.exact.get(&other_path).map(String::as_str) != Some("deleted")
        });
        if has_surviving_child || node.is_some_and(|n| n["changed"].as_u64().unwrap_or(0) > 0) {
            Some("modified".to_owned())
        } else {
            Some("deleted".to_owned())
        }
    }

    pub(super) fn status_for_node(&self, node: &V, all_nodes: &[V]) -> Option<String> {
        let is_symbol = m::yes(&node["symbol"]) || node["kind"].as_str() == Some("symbol");
        if is_symbol {
            return self.entity_status_for(node);
        }
        let folder = m::yes(&node["folder"]);
        let norm = normalize_rel_path(&m::text(node, "path"));
        if norm.is_empty() {
            return None;
        }
        if let Some(status) = self.exact.get(&norm) {
            return Some(status.clone());
        }
        if !folder {
            return None;
        }
        self.folder_status_for(&norm, Some(node), all_nodes)
    }

    #[allow(dead_code)]
    pub(super) fn status_for(&self, node_path: &str, folder: bool) -> Option<String> {
        if node_path.contains('#') {
            return None;
        }
        let norm = normalize_rel_path(node_path);
        if norm.is_empty() {
            return None;
        }
        if let Some(status) = self.exact.get(&norm) {
            return Some(status.clone());
        }
        if !folder {
            return None;
        }
        self.folder_status_for(&norm, None, &[])
    }
}

pub(super) struct ImpactLookup {
    pub(super) selected_path: String,
    pub(super) selected_idx: Option<usize>,
    pub(super) uses_added: Vec<String>,
    pub(super) uses_removed: Vec<String>,
    pub(super) uses_unchanged: Vec<String>,
    pub(super) callers_risk: Vec<String>,
    pub(super) callers_impacted: Vec<String>,
    pub(super) callers_added: Vec<String>,
    pub(super) callers_unchanged: Vec<String>,
    /// The kinds of the dependencies a change added and took away, by
    /// their target: one target may be called, read and written.
    pub(super) kinds_added: BTreeMap<String, [u64; 7]>,
    pub(super) kinds_removed: BTreeMap<String, [u64; 7]>,
    /// Commit-wide edge diffs & caller risks parsed from `/graph/changes` `edge_diffs`
    /// and deleted file `callers[]`.
    pub(super) commit_edges_added: Vec<(String, String, &'static str)>,
    pub(super) commit_edges_removed: Vec<(String, String, &'static str)>,
    pub(super) commit_risk_callers: Vec<(String, String, String)>,
}

pub(super) fn edge_diff_primary_kind(entry: &V) -> &'static str {
    let mut kinds = m::array(&entry["kinds"]);
    if kinds.is_empty() {
        if let Some(single) = entry["kind"].as_str() {
            kinds.push(json!(single));
        }
    }
    let pseudo = json!({ "kinds": kinds.into_iter().map(|k| {
        if k.is_string() { json!({"kind": k, "links": 1}) } else { k }
    }).collect::<Vec<_>>() });
    pick_precedence_kind(&EdgeKinds::of(&pseudo).counts)
}

impl ImpactLookup {
    #[allow(dead_code)]
    pub(super) fn from_value(doc: &V, nodes: &[V]) -> Option<Self> {
        Self::from_sources(Some(doc), None, nodes)
    }

    pub(super) fn from_sources(
        impact_doc: Option<&V>,
        changes_doc: Option<&V>,
        nodes: &[V],
    ) -> Option<Self> {
        let selected_path = impact_doc
            .map(|doc| normalize_rel_path(strip_symbol_suffix(&m::text(doc, "path"))))
            .unwrap_or_default();
        let has_selected = !selected_path.is_empty();
        let selected_idx = has_selected
            .then(|| resolve_canvas_node_by_path(nodes, &selected_path))
            .flatten();
        let collect_deps = |arr: &V| -> Vec<String> {
            m::array(arr)
                .into_iter()
                .filter_map(|dep| {
                    let target = normalize_rel_path(strip_symbol_suffix(&m::text(&dep, "target")));
                    (!target.is_empty()).then_some(target)
                })
                .collect()
        };
        let collect_kinds = |arr: &V| -> BTreeMap<String, [u64; 7]> {
            let mut kinds: BTreeMap<String, [u64; 7]> = BTreeMap::new();
            for dep in m::array(arr) {
                let target = normalize_rel_path(strip_symbol_suffix(&m::text(&dep, "target")));
                if target.is_empty() {
                    continue;
                }
                let held = EdgeKinds::of(&dep).counts;
                for (count, more) in kinds.entry(target).or_default().iter_mut().zip(held) {
                    *count += more;
                }
            }
            kinds
        };
        let mut callers_risk = Vec::new();
        let mut callers_impacted = Vec::new();
        let mut callers_added = Vec::new();
        let mut callers_unchanged = Vec::new();
        let (uses_added, uses_removed, uses_unchanged, kinds_added, kinds_removed) =
            if let Some(doc) = impact_doc.filter(|_| has_selected) {
                for caller in m::array(&doc["used_by"]) {
                    let path = normalize_rel_path(strip_symbol_suffix(&m::text(&caller, "path")));
                    if path.is_empty() {
                        continue;
                    }
                    match m::text(&caller, "impact").as_str() {
                        "broken_risk" => callers_risk.push(path),
                        "impacted" => callers_impacted.push(path),
                        "added_caller" => callers_added.push(path),
                        _ => callers_unchanged.push(path),
                    }
                }
                (
                    collect_deps(&doc["uses"]["added"]),
                    collect_deps(&doc["uses"]["removed"]),
                    collect_deps(&doc["uses"]["unchanged"]),
                    collect_kinds(&doc["uses"]["added"]),
                    collect_kinds(&doc["uses"]["removed"]),
                )
            } else {
                (
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    BTreeMap::new(),
                    BTreeMap::new(),
                )
            };

        let mut commit_edges_added = Vec::new();
        let mut commit_edges_removed = Vec::new();
        let mut commit_risk_callers = Vec::new();
        let mut seen_risk: BTreeSet<(String, String, String)> = BTreeSet::new();
        if let Some(changes) = changes_doc {
            let edge_diffs = &changes["edge_diffs"];
            if edge_diffs.is_object() {
                for item in m::array(&edge_diffs["added"]) {
                    let from = normalize_rel_path(strip_symbol_suffix(&m::text(&item, "from")));
                    let to = normalize_rel_path(strip_symbol_suffix(&m::text(&item, "to")));
                    if !from.is_empty() && !to.is_empty() {
                        commit_edges_added.push((from, to, edge_diff_primary_kind(&item)));
                    }
                }
                for item in m::array(&edge_diffs["removed"]) {
                    let from = normalize_rel_path(strip_symbol_suffix(&m::text(&item, "from")));
                    let to = normalize_rel_path(strip_symbol_suffix(&m::text(&item, "to")));
                    if !from.is_empty() && !to.is_empty() {
                        commit_edges_removed.push((from, to, edge_diff_primary_kind(&item)));
                    }
                }
                for item in m::array(&edge_diffs["risk_callers"]) {
                    let raw_from = m::text(&item, "from");
                    let fallback_from = m::text(&item, "path");
                    let from = normalize_rel_path(strip_symbol_suffix(if raw_from.is_empty() {
                        &fallback_from
                    } else {
                        &raw_from
                    }));
                    let to = normalize_rel_path(strip_symbol_suffix(&m::text(&item, "to")));
                    let impact = m::text(&item, "impact").to_ascii_lowercase();
                    if !from.is_empty() && !impact.is_empty() {
                        let key = (from.clone(), to.clone(), impact.clone());
                        if seen_risk.insert(key) {
                            commit_risk_callers.push((from, to, impact));
                        }
                    }
                }
            }
            for file in m::array(&changes["files"]) {
                if normalize_change_status(&m::text(&file, "status")) != "deleted" {
                    continue;
                }
                let del_path = normalize_rel_path(&m::text(&file, "path"));
                if del_path.is_empty() {
                    continue;
                }
                for caller in m::array(&file["callers"]) {
                    if let Some(caller_str) = caller.as_str() {
                        let from = normalize_rel_path(strip_symbol_suffix(caller_str));
                        if !from.is_empty() {
                            let key = (from.clone(), del_path.clone(), "broken_risk".to_owned());
                            if seen_risk.insert(key) {
                                commit_risk_callers.push((
                                    from,
                                    del_path.clone(),
                                    "broken_risk".to_owned(),
                                ));
                            }
                        }
                    }
                }
            }
        }

        if !has_selected
            && commit_edges_added.is_empty()
            && commit_edges_removed.is_empty()
            && commit_risk_callers.is_empty()
        {
            return None;
        }

        Some(Self {
            selected_path,
            selected_idx,
            uses_added,
            uses_removed,
            uses_unchanged,
            callers_risk,
            callers_impacted,
            callers_added,
            callers_unchanged,
            kinds_added,
            kinds_removed,
            commit_edges_added,
            commit_edges_removed,
            commit_risk_callers,
        })
    }

    /// The kind a dependency on `target` that a change added, or took
    /// away, is drawn as: the strongest of the kinds it had.
    pub(super) fn dep_kind(&self, target: &str, added: bool) -> &'static str {
        let kinds = if added {
            &self.kinds_added
        } else {
            &self.kinds_removed
        };
        kinds.get(target).map_or("calls", pick_precedence_kind)
    }

    pub(super) fn node_matches_any(&self, node: &V, paths: &[String]) -> bool {
        let node_path = m::text(node, "path");
        let is_folder = m::yes(&node["folder"]);
        paths
            .iter()
            .any(|path| path_matches_node(&node_path, is_folder, path))
    }

    pub(super) fn pair_matches(
        nodes: &[V],
        from: usize,
        to: usize,
        expected_from: &str,
        expected_to: &str,
    ) -> bool {
        if expected_from.is_empty() || expected_to.is_empty() {
            return false;
        }
        let from_node = &nodes[from];
        let to_node = &nodes[to];
        path_matches_node(
            &m::text(from_node, "path"),
            m::yes(&from_node["folder"]),
            expected_from,
        ) && path_matches_node(
            &m::text(to_node, "path"),
            m::yes(&to_node["folder"]),
            expected_to,
        )
    }

    pub(super) fn edge_diff(&self, nodes: &[V], from: usize, to: usize) -> Option<&'static str> {
        let mut selected_unchanged = false;
        if !self.selected_path.is_empty() {
            let from_node = &nodes[from];
            let to_node = &nodes[to];
            let from_is_selected = path_matches_node(
                &m::text(from_node, "path"),
                m::yes(&from_node["folder"]),
                &self.selected_path,
            );
            let to_is_selected = path_matches_node(
                &m::text(to_node, "path"),
                m::yes(&to_node["folder"]),
                &self.selected_path,
            );
            if from_is_selected {
                if self.node_matches_any(to_node, &self.uses_added) {
                    return Some("added");
                }
                if self.node_matches_any(to_node, &self.uses_removed) {
                    return Some("removed");
                }
                if self.node_matches_any(to_node, &self.uses_unchanged) {
                    selected_unchanged = true;
                }
            }
            if to_is_selected {
                if self.node_matches_any(from_node, &self.callers_added) {
                    return Some("added");
                }
                if self.node_matches_any(from_node, &self.callers_risk) {
                    return Some("removed");
                }
                if self.node_matches_any(from_node, &self.callers_impacted)
                    || self.node_matches_any(from_node, &self.callers_unchanged)
                {
                    selected_unchanged = true;
                }
            }
        }
        if self
            .commit_edges_added
            .iter()
            .any(|(ef, et, _)| Self::pair_matches(nodes, from, to, ef, et))
        {
            return Some("added");
        }
        if self
            .commit_edges_removed
            .iter()
            .any(|(ef, et, _)| Self::pair_matches(nodes, from, to, ef, et))
        {
            return Some("removed");
        }
        if selected_unchanged {
            Some("unchanged")
        } else {
            None
        }
    }

    pub(super) fn edge_impact(&self, nodes: &[V], from: usize, to: usize) -> Option<&'static str> {
        let from_node = &nodes[from];
        let to_node = &nodes[to];
        let to_is_selected = !self.selected_path.is_empty()
            && path_matches_node(
                &m::text(to_node, "path"),
                m::yes(&to_node["folder"]),
                &self.selected_path,
            );
        if to_is_selected && self.node_matches_any(from_node, &self.callers_risk) {
            return Some("broken_risk");
        }
        if self.commit_risk_callers.iter().any(|(cf, ct, imp)| {
            if imp != "broken_risk" {
                return false;
            }
            if ct.is_empty() {
                to_is_selected
                    && path_matches_node(
                        &m::text(from_node, "path"),
                        m::yes(&from_node["folder"]),
                        cf,
                    )
            } else {
                Self::pair_matches(nodes, from, to, cf, ct)
            }
        }) {
            return Some("broken_risk");
        }
        None
    }

    pub(super) fn node_impact(&self, nodes: &[V], idx: usize) -> Option<&'static str> {
        if Some(idx) == self.selected_idx {
            return None;
        }
        let node = &nodes[idx];
        if self.node_matches_any(node, &self.callers_risk) {
            return Some("caller-risk");
        }
        if self.node_matches_any(node, &self.uses_removed) {
            return Some("removed-dep");
        }
        if self.node_matches_any(node, &self.uses_added) {
            return Some("added-dep");
        }
        if self.node_matches_any(node, &self.callers_impacted)
            || self.node_matches_any(node, &self.callers_added)
        {
            return Some("caller-impacted");
        }
        let node_path = m::text(node, "path");
        let is_folder = m::yes(&node["folder"]);
        if self.commit_risk_callers.iter().any(|(cf, _, imp)| {
            imp == "broken_risk" && path_matches_node(&node_path, is_folder, cf)
        }) {
            return Some("caller-risk");
        }
        if self.commit_risk_callers.iter().any(|(cf, _, imp)| {
            matches!(imp.as_str(), "impacted" | "added_caller")
                && path_matches_node(&node_path, is_folder, cf)
        }) {
            return Some("caller-impacted");
        }
        None
    }
}

pub(super) fn synthetic_edge_points(
    from_place: &Placed,
    to_place: &Placed,
) -> ((f64, f64), (f64, f64)) {
    let mid_y = BOX_HEIGHT / 2.0;
    if from_place.x <= to_place.x {
        (
            (from_place.x + BOX_WIDTH, from_place.y + mid_y),
            (to_place.x, to_place.y + mid_y),
        )
    } else {
        (
            (from_place.x, from_place.y + mid_y),
            (to_place.x + BOX_WIDTH, to_place.y + mid_y),
        )
    }
}

/// Routes a synthetic diff/risk edge around intermediate layer boxes using gap channels.
pub(super) fn synthetic_edge_path(
    from_idx: usize,
    to_idx: usize,
    placed: &[Placed],
    lane_slot: usize,
) -> String {
    let from_place = &placed[from_idx];
    let to_place = &placed[to_idx];
    let (fp, tp) = synthetic_edge_points(from_place, to_place);
    let min_x = fp.0.min(tp.0);
    let max_x = fp.0.max(tp.0);
    let min_y = from_place.y.min(to_place.y);
    let max_y = from_place.y.max(to_place.y) + BOX_HEIGHT;

    let crosses_box = placed.iter().enumerate().any(|(idx, node)| {
        idx != from_idx
            && idx != to_idx
            && node.x + BOX_WIDTH > min_x + 4.0
            && node.x < max_x - 4.0
            && node.y + BOX_HEIGHT > min_y - 2.0
            && node.y < max_y + 2.0
    });

    if !crosses_box && (from_place.x - to_place.x).abs() > 8.0 {
        let mut d = format!("M{:.1} {:.1}", fp.0, fp.1);
        curve(&mut d, fp, tp);
        return d;
    }

    // Route smoothly through an uncluttered horizontal channel above or below interfering cards,
    // staying below top folder headers (`MARGIN + FRAME_HEAD ~= 44`) and staggering >5 lanes cleanly.
    let gutter_step = (lane_slot % 14) as f64 * 6.0 + (lane_slot / 14) as f64 * 4.0;
    let above_y = min_y - 12.0 - gutter_step;
    let go_above = (from_place.y + to_place.y) * 0.5 < 240.0 && above_y >= 44.0;
    let channel_y = if (from_place.x - to_place.x).abs() <= 8.0 {
        // Same column: arc via right channel outside boxes
        let exit_x = from_place.x + BOX_WIDTH + 22.0 + gutter_step;
        let p0 = (from_place.x + BOX_WIDTH, from_place.y + BOX_HEIGHT * 0.5);
        let p1 = (exit_x, from_place.y + BOX_HEIGHT * 0.5);
        let p2 = (exit_x, to_place.y + BOX_HEIGHT * 0.5);
        let p3 = (to_place.x + BOX_WIDTH, to_place.y + BOX_HEIGHT * 0.5);
        return format!(
            "M{:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
            p0.0, p0.1, p1.0, p1.1, p2.0, p2.1, p3.0, p3.1
        );
    } else if go_above {
        above_y
    } else {
        max_y + 10.0 + gutter_step
    };

    let c1 = (fp.0 + (tp.0 - fp.0) * 0.25, channel_y);
    let c2 = (fp.0 + (tp.0 - fp.0) * 0.75, channel_y);
    format!(
        "M{:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
        fp.0, fp.1, c1.0, c1.1, c2.0, c2.1, tp.0, tp.1
    )
}

#[derive(Clone, Copy)]
pub(super) struct EdgeArrowSpec {
    pub(super) kind: &'static str,
    pub(super) label: &'static str,
    pub(super) description: &'static str,
    pub(super) dash_array: Option<&'static str>,
    pub(super) marker_body: &'static str,
    pub(super) swatch_line_end: f64,
}

pub(super) const EDGE_ARROW_SPECS: [EdgeArrowSpec; 7] = [
    EdgeArrowSpec {
        kind: "calls",
        label: "Calls",
        description: "Calls, instantiations, or default dependencies (solid, filled triangle)",
        dash_array: None,
        marker_body: "<path d=\"M0 0 L8 4 L0 8 z\"/>",
        swatch_line_end: 21.0,
    },
    EdgeArrowSpec {
        kind: "imports",
        label: "Imports",
        description: "Imports, includes, or requires (dashed, open chevron)",
        dash_array: Some("8 4"),
        marker_body: "<path d=\"M1 1 L7.5 4 L1 7\"/>",
        swatch_line_end: 26.5,
    },
    EdgeArrowSpec {
        kind: "extends",
        label: "Extends",
        description: "Inherits from a parent class or interface (solid, hollow triangle)",
        dash_array: None,
        marker_body: "<path d=\"M0.8 0.8 L7.8 4 L0.8 7.2 Z\"/>",
        swatch_line_end: 21.0,
    },
    EdgeArrowSpec {
        kind: "implements",
        label: "Implements",
        description: "Implements an interface or uses a trait (dash-dot, hollow triangle)",
        dash_array: Some("6 3 2 3"),
        marker_body: "<path d=\"M0.8 0.8 L7.8 4 L0.8 7.2 Z\"/>",
        swatch_line_end: 21.0,
    },
    EdgeArrowSpec {
        kind: "writes",
        label: "Writes",
        description: "Writes, mutates, or sets state (solid, filled diamond)",
        dash_array: None,
        marker_body: "<path d=\"M0.5 4 L4 1 L7.5 4 L4 7 Z\"/>",
        swatch_line_end: 21.0,
    },
    EdgeArrowSpec {
        kind: "reads",
        label: "Reads",
        description: "Reads properties, static state, or constants (dotted, hollow diamond)",
        dash_array: Some("2 3"),
        marker_body: "<path d=\"M0.8 4 L4 1.2 L7.2 4 L4 6.8 Z\"/>",
        swatch_line_end: 21.0,
    },
    EdgeArrowSpec {
        kind: "uses",
        label: "Uses",
        description: "Type hints or references (solid, barbed tip)",
        dash_array: None,
        marker_body: "<path d=\"M0.5 1 L7.8 4 L0.5 7 L2.4 4 z\"/>",
        swatch_line_end: 22.0,
    },
];

pub(super) fn edge_arrow_spec_index(kind: &str) -> usize {
    EDGE_ARROW_SPECS
        .iter()
        .position(|spec| spec.kind == kind)
        .unwrap_or(0)
}

/// Normalizes a raw dependency relation name to one of the 7 canonical arrow kinds.
pub fn normalize_edge_arrow_kind(raw: &str) -> &'static str {
    let key = raw.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    match key.as_str() {
        "calls" | "call" | "instantiates" | "indirect_call" => "calls",
        "imports" | "imports_from" | "includes" | "requires" | "re_exports"
        | "crate_depends_on" | "depends_on" => "imports",
        "extends" | "inherits" => "extends",
        "implements" | "uses_trait" | "trait" => "implements",
        "writes" | "mutates" | "sets" => "writes",
        "reads"
        | "static_read"
        | "uses_const"
        | "const"
        | "references_constant"
        | "uses_static_prop" => "reads",
        "uses" | "references" | "type_hint" | "uses_component" | "binds_method" | "bound_to" => {
            "uses"
        }
        _ => "calls",
    }
}

pub(super) fn pick_precedence_kind(counts: &[u64; 7]) -> &'static str {
    if counts[2] > 0 {
        return "extends";
    }
    if counts[3] > 0 {
        return "implements";
    }
    let mut best: Option<(&'static str, u64)> = None;
    for &(idx, kind) in &[(4usize, "writes"), (0, "calls"), (5, "reads"), (6, "uses")] {
        let count = counts[idx];
        if count == 0 {
            continue;
        }
        match best {
            None => best = Some((kind, count)),
            Some((_, best_count)) if count > best_count => best = Some((kind, count)),
            _ => {}
        }
    }
    if let Some((kind, _)) = best {
        return kind;
    }
    if counts[1] > 0 {
        return "imports";
    }
    "calls"
}

/// The kinds of links the key of a picture has switched off, in the order of
/// `EDGE_ARROW_SPECS`.
pub type KindsOff = [bool; 7];
/// The kinds a state keeps switched off: `links_off` holds their names.
pub fn kinds_off(state: &V) -> KindsOff {
    let mut off = [false; 7];
    for kind in state["links_off"].as_array().into_iter().flatten() {
        if let Some(at) = kind
            .as_str()
            .and_then(|kind| EDGE_ARROW_SPECS.iter().position(|spec| spec.kind == kind))
        {
            off[at] = true;
        }
    }
    off
}
/// What a link holds of each of the seven kinds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EdgeKinds {
    /// Links of each kind, in the order of `EDGE_ARROW_SPECS`.
    pub counts: [u64; 7],
    /// The kinds as the graph worded them, the most links first.
    pub breakdown: Option<String>,
}
impl EdgeKinds {
    /// Read from a link of a picture: its kinds with their counts, or its
    /// one kind, or how a pattern words what ties two parts. A link that
    /// says nothing is a call, as it is drawn.
    pub fn of(edge: &V) -> Self {
        let mut counts = [0u64; 7];
        let mut add = |raw: &str, links: u64| {
            let at = edge_arrow_spec_index(normalize_edge_arrow_kind(raw));
            counts[at] = counts[at].saturating_add(links.max(1));
        };
        if let Some(kinds) = edge["kinds"].as_array().filter(|arr| !arr.is_empty()) {
            let mut items: Vec<(String, u64)> = Vec::with_capacity(kinds.len());
            for entry in kinds {
                let raw = entry
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| m::text(entry, "kind"));
                if raw.is_empty() {
                    continue;
                }
                let links = entry["links"].as_u64().unwrap_or(1);
                add(&raw, links);
                items.push((raw.replace('_', " "), links));
            }
            if !items.is_empty() {
                items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                let breakdown = items
                    .into_iter()
                    .map(|(label, count)| format!("{label} {count}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Self {
                    counts,
                    breakdown: Some(breakdown),
                };
            }
        }
        if let Some(kind) = edge["kind"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            add(kind, 1);
        } else if let Some(tie) = edge["tie"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            for part in tie
                .split(" and ")
                .flat_map(|s| s.split(','))
                .flat_map(|s| s.split('&'))
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                add(part, 1);
            }
        }
        if counts == [0; 7] {
            counts[0] = 1;
        }
        Self {
            counts,
            breakdown: None,
        }
    }
    /// The counts of the kinds that are not switched off.
    pub(super) fn shown(&self, off: &KindsOff) -> [u64; 7] {
        let mut shown = self.counts;
        for (count, off) in shown.iter_mut().zip(off) {
            if *off {
                *count = 0;
            }
        }
        shown
    }
    /// The kind a link is drawn as: the strongest of those not switched
    /// off. None where every kind it holds is switched off, so that the
    /// link is not drawn.
    pub fn primary(&self, off: &KindsOff) -> Option<&'static str> {
        let shown = self.shown(off);
        (shown != [0; 7]).then(|| pick_precedence_kind(&shown))
    }
    /// Whether the link holds more than one kind that is not switched off.
    pub fn multi(&self, off: &KindsOff) -> bool {
        self.shown(off).iter().filter(|count| **count > 0).count() > 1
    }
    /// The names of every kind the link holds, whatever is switched off.
    pub fn held(&self) -> String {
        self.held_vec().join(" ")
    }

    pub fn held_vec(&self) -> Vec<String> {
        EDGE_ARROW_SPECS
            .iter()
            .zip(&self.counts)
            .filter(|(_, count)| **count > 0)
            .map(|(spec, _)| spec.kind.to_owned())
            .collect()
    }
}
#[cfg(test)]
pub(super) fn edge_kind_stats(edge: &V) -> (&'static str, bool, Option<String>) {
    let kinds = EdgeKinds::of(edge);
    let all = [false; 7];
    (
        kinds.primary(&all).unwrap_or("calls"),
        kinds.multi(&all),
        kinds.breakdown,
    )
}

/// Resolves the canonical arrow kind for an aggregated or link-level dependency edge.
#[cfg(test)]
pub fn arrow_kind_for_edge(edge: &V) -> &'static str {
    edge_kind_stats(edge).0
}

pub(super) fn arrow_marker_suffix(kind: &str, against: bool) -> String {
    if against {
        "-against".to_owned()
    } else if kind == "calls" {
        String::new()
    } else {
        format!("-{kind}")
    }
}

pub(super) fn arrow_class_for_kind(kind: &str) -> String {
    if kind == "calls" {
        "graph-map-arrow".to_owned()
    } else {
        format!("graph-map-arrow graph-map-arrow-{kind}")
    }
}

pub(super) fn arrow_defs_svg(id_prefix: &str, include_against: bool) -> String {
    let mut defs = String::new();
    for spec in EDGE_ARROW_SPECS {
        let suffix = arrow_marker_suffix(spec.kind, false);
        let class_name = arrow_class_for_kind(spec.kind);
        defs.push_str(&format!(
            "<marker id=\"{id_prefix}-arrow{suffix}\" class=\"{class_name}\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" markerUnits=\"userSpaceOnUse\" orient=\"auto\">{}</marker>",
            spec.marker_body,
        ));
    }
    if include_against {
        // The head of a link that runs against the order of the layers.
        defs.push_str(&format!(
            "<marker id=\"{id_prefix}-arrow-against\" class=\"graph-map-arrow graph-map-arrow-against\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" markerUnits=\"userSpaceOnUse\" orient=\"auto\"><path d=\"M0 0 L8 4 L0 8 z\"/></marker>"
        ));
    }
    // Where a link leaves: a dot for a link of one kind, a ring with an
    // eye for a link that holds several.
    defs.push_str(&format!(
        "<marker id=\"{id_prefix}-port\" class=\"graph-map-port\" viewBox=\"0 0 8 8\" refX=\"4\" refY=\"4\" markerWidth=\"6\" markerHeight=\"6\" markerUnits=\"userSpaceOnUse\"><circle cx=\"4\" cy=\"4\" r=\"2.6\"/></marker><marker id=\"{id_prefix}-port-multi\" class=\"graph-map-port graph-map-port-multi\" viewBox=\"0 0 8 8\" refX=\"4\" refY=\"4\" markerWidth=\"6\" markerHeight=\"6\" markerUnits=\"userSpaceOnUse\"><circle cx=\"4\" cy=\"4\" r=\"2.9\"/><circle class=\"graph-map-port-eye\" cx=\"4\" cy=\"4\" r=\"1.1\"/></marker>"
    ));
    defs
}

/// The kinds the presets of the key leave shown, by the name of the preset.
pub(super) const KIND_PRESETS: [(&str, &str, &str, [&str; 7]); 4] = [
    (
        "all",
        "All",
        "Show every kind of link",
        [
            "calls",
            "imports",
            "extends",
            "implements",
            "writes",
            "reads",
            "uses",
        ],
    ),
    (
        "runtime",
        "Runtime",
        "What runs: calls, writes and reads",
        ["calls", "writes", "reads", "", "", "", ""],
    ),
    (
        "hierarchy",
        "Hierarchy",
        "What is built on what: extends and implements",
        ["extends", "implements", "", "", "", "", ""],
    ),
    (
        "no-imports",
        "No imports",
        "Everything but the imports",
        [
            "calls",
            "extends",
            "implements",
            "writes",
            "reads",
            "uses",
            "",
        ],
    ),
];
/// The kinds a preset switches off; none for a name that is no preset.
pub fn preset_off(preset: &str) -> Option<KindsOff> {
    let (.., shown) = KIND_PRESETS.iter().find(|(name, ..)| *name == preset)?;
    let mut off = [true; 7];
    for (at, spec) in EDGE_ARROW_SPECS.iter().enumerate() {
        off[at] = !shown.contains(&spec.kind);
    }
    Some(off)
}
/// The state with one kind of link switched on or off, or, with `only`,
/// with that kind alone left on. Switching the last shown kind off, or
/// asking again for the one kind that is alone, shows every kind: a
/// picture is never left without its links by its own key.
pub fn switched_kind(mut state: V, kind: &str, only: bool) -> V {
    let Some(at) = EDGE_ARROW_SPECS.iter().position(|spec| spec.kind == kind) else {
        return state;
    };
    let mut off = kinds_off(&state);
    if only {
        let alone = off[at] || off.iter().filter(|off| !**off).count() > 1;
        off = [alone; 7];
        off[at] = false;
    } else {
        off[at] = !off[at];
        if off == [true; 7] {
            off = [false; 7];
        }
    }
    state["links_off"] = json!(names_off(&off));
    state
}
/// The state with the kinds of a preset shown and the others switched off.
pub fn preset_kinds(mut state: V, preset: &str) -> V {
    if let Some(off) = preset_off(preset) {
        state["links_off"] = json!(names_off(&off));
    }
    state
}
pub(super) fn names_off(off: &KindsOff) -> Vec<&'static str> {
    EDGE_ARROW_SPECS
        .iter()
        .zip(off)
        .filter(|(_, off)| **off)
        .map(|(spec, _)| spec.kind)
        .collect()
}
/// How many links of a picture hold each kind, and how many of them are
/// not drawn because every kind they hold is switched off.
pub fn kind_counts(view: &V, off: &KindsOff) -> ([u64; 7], u64) {
    let nodes = m::array(&view["nodes"]);
    let boxed = view["nodes"].is_array();
    let drawn = |path: &V| !boxed || nodes.iter().any(|node| &node["path"] == path);
    let mut counts = [0u64; 7];
    let mut hidden = 0;
    for edge in m::array(&view["edges"])
        .iter()
        .filter(|edge| drawn(&edge["from"]) && drawn(&edge["to"]))
    {
        let kinds = EdgeKinds::of(edge);
        for (count, held) in counts.iter_mut().zip(&kinds.counts) {
            *count += u64::from(*held > 0);
        }
        hidden += u64::from(kinds.primary(off).is_none());
    }
    (counts, hidden)
}
/// The key of a picture's links, which is also their switchboard. Each of
/// the seven kinds is a button with the arrow it is drawn as and how many
/// links of the picture hold it: selecting it hides those links and shows
/// them again, and resting on it lights them. A link that holds several
/// kinds stays while one of them is shown, drawn as the strongest shown.
/// The presets beside them switch several kinds at once. `surface` names
/// the picture the buttons act on.
pub fn kinds_legend_html(view: &V, off: &KindsOff, surface: &str) -> String {
    let (counts, hidden) = kind_counts(view, off);
    let mut html = String::from(
        "<div class=\"graph-arrow-legend\" role=\"group\" aria-label=\"Kinds of links: select one to hide or show its links\"><span class=\"graph-arrow-legend-title\">Links:</span>",
    );
    for (at, spec) in EDGE_ARROW_SPECS.iter().enumerate() {
        let dash = spec
            .dash_array
            .map(|d| format!(" stroke-dasharray=\"{d}\""))
            .unwrap_or_default();
        let arrow_class = arrow_class_for_kind(spec.kind);
        let told = format!(
            "{} · {} here · select to {}, Alt+select to show it alone",
            spec.description,
            plural(counts[at], "link", "links"),
            if off[at] { "show" } else { "hide" },
        );
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-item\" data-action=\"graph-kind-toggle\" data-payload=\"{}\" data-edge-kind=\"{}\" data-empty=\"{}\" aria-pressed=\"{}\" title=\"{}\"><svg class=\"graph-arrow-swatch\" data-edge-kind=\"{}\" viewBox=\"0 0 30 12\" width=\"30\" height=\"12\" aria-hidden=\"true\"><line class=\"graph-arrow-swatch-line\" x1=\"2\" y1=\"6\" x2=\"{:.1}\" y2=\"6\"{dash}/><g transform=\"translate(20 2)\" class=\"{arrow_class}\">{}</g></svg><span class=\"graph-arrow-legend-name\">{}</span><span class=\"graph-arrow-legend-count\">{}</span></button>",
            payload(&json!({"kind": spec.kind, "surface": surface})),
            spec.kind,
            counts[at] == 0,
            !off[at],
            escape(&told),
            spec.kind,
            spec.swatch_line_end,
            spec.marker_body,
            spec.label,
            count(counts[at]),
        ));
    }
    html.push_str(
        "<span class=\"graph-arrow-legend-presets\" role=\"group\" aria-label=\"Presets\">",
    );
    for (name, label, told, _) in KIND_PRESETS {
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"graph-kind-preset\" data-payload=\"{}\" aria-pressed=\"{}\" title=\"{told}\">{label}</button>",
            payload(&json!({"preset": name, "surface": surface})),
            preset_off(name).as_ref() == Some(off),
        ));
    }
    html.push_str("</span>");
    html.push_str(&format!(
        "<span class=\"graph-arrow-legend-note\">{}<svg class=\"graph-arrow-swatch graph-arrow-swatch-hollow\" viewBox=\"0 0 22 12\" width=\"22\" height=\"12\" aria-hidden=\"true\"><line class=\"graph-arrow-swatch-line\" x1=\"2\" y1=\"6\" x2=\"20\" y2=\"6\"/><line class=\"graph-arrow-swatch-core\" x1=\"2\" y1=\"6\" x2=\"20\" y2=\"6\"/></svg> hollow: only inferred · ● leaves as one kind · ◎ holds several</span>",
        match hidden {
            0 => String::new(),
            hidden => format!(
                "<span class=\"graph-arrow-legend-hidden\">{} hidden by this key · </span>",
                plural(hidden, "link", "links")
            ),
        },
    ));
    html.push_str("</div>");
    html
}
/// What the key under a picture is set to: the kinds of links switched
/// off, whether links that longer ways already join are left out, and how
/// far the links of the selected box are lit (to what uses it, to what it
/// uses).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyState {
    pub off: KindsOff,
    pub hide_transitive: bool,
    pub cone: (usize, usize),
}
/// How far from a selected box its links can be lit.
pub const CONE_REACH: usize = 3;
/// The key of a picture of the map: the kinds of its links, and beside
/// them what else is chosen for the picture as a whole. **Hide transitive**
/// leaves out the links that a longer way already joins and says how many
/// that is. With a box selected, **Reach** says how many links from it are
/// lit on each side: one, as always, or two or three.
pub fn key_html(view: &V, key: &KeyState, surface: &str) -> String {
    let mut html = kinds_legend_html(view, &key.off, surface);
    let mut more = String::new();
    // A path that was found can be asked for the ways round as well.
    if paths(view) && m::yes(&view["path"]["found"]) {
        more.push_str(&longer_ways(view, surface));
    }
    let plain = plain_map(view);
    let said = transitive_links(view);
    if plain && (said > 0 || key.hide_transitive) {
        let told = if key.hide_transitive {
            format!(
                "{} left out: a longer way already joins their two ends. Select to draw them again",
                plural(said as u64, "link is", "links are")
            )
        } else {
            format!(
                "Leave out {}: a longer way already joins their two ends, so every box still reaches what it reached",
                plural(said as u64, "link", "links")
            )
        };
        more.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-preset graph-arrow-legend-switch\" data-action=\"graph-transitive-toggle\" data-payload=\"{}\" aria-pressed=\"{}\" title=\"{}\">Hide transitive <span class=\"graph-arrow-legend-count\">{}</span></button>",
            payload(&json!({"surface": surface})),
            key.hide_transitive,
            escape(&told),
            count(said as u64),
        ));
    }
    let nodes = m::array(&view["nodes"]);
    let selected = view["focus"]["path"]
        .as_str()
        .is_some_and(|path| nodes.iter().any(|node| node["path"] == path));
    if plain && selected {
        more.push_str("<span class=\"graph-arrow-legend-reach\" role=\"group\" aria-label=\"How many links from the selected box are lit\"><span class=\"graph-arrow-legend-title\">Reach:</span>");
        for (side, word, told, now) in [
            ("up", "used by", "what uses it", key.cone.0.max(1)),
            ("down", "uses", "what it uses", key.cone.1.max(1)),
        ] {
            more.push_str(&format!(
                "<span class=\"graph-arrow-legend-side\">{word}</span>"
            ));
            for hops in 1..=CONE_REACH {
                more.push_str(&format!(
                    "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"graph-cone\" data-payload=\"{}\" aria-pressed=\"{}\" title=\"Light {told} within {}\">{hops}</button>",
                    payload(&json!({"hops": hops, "side": side, "surface": surface})),
                    now == hops,
                    plural(hops as u64, "link", "links"),
                ));
            }
        }
        more.push_str("</span>");
    }
    if !more.is_empty() {
        // Before the note that ends the key.
        let at = html
            .rfind("<span class=\"graph-arrow-legend-note\">")
            .unwrap_or(html.len());
        html.insert_str(at, &more);
    }
    html
}
/// By how many links a way between two places may be longer than the
/// shortest and still be asked for.
pub(super) const LONGER: u64 = 3;
/// What the key of a path offers beside the kinds of its links: the
/// shortest ways alone, or with them the ways that are longer by one, two
/// or three links, each drawn whole as a way round. It says how many of
/// those the picture draws, with a + where there are more.
pub(super) fn longer_ways(view: &V, surface: &str) -> String {
    let path = &view["path"];
    let now = path["longer"].as_u64().unwrap_or(0);
    let mut html = String::from(
        "<span class=\"graph-arrow-legend-reach\" role=\"group\" aria-label=\"Ways longer than the shortest\"><span class=\"graph-arrow-legend-title\">Longer ways:</span>",
    );
    for longer in 0..=LONGER {
        let (word, told) = match longer {
            0 => ("none".to_owned(), "The shortest ways alone".to_owned()),
            longer => (
                format!("+{longer}"),
                format!(
                    "Also the ways longer than the shortest by {}",
                    plural(longer, "link", "links")
                ),
            ),
        };
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"graph-map-longer\" data-payload=\"{}\" aria-pressed=\"{}\" title=\"{told}\">{word}</button>",
            payload(&json!({"longer": longer, "surface": surface})),
            now == longer,
        ));
    }
    if now > 0 {
        let found = path["detours"].as_u64().unwrap_or(0);
        html.push_str(&format!(
            "<span class=\"graph-arrow-legend-side\">{}</span>",
            match found {
                0 => "none found".to_owned(),
                found => format!(
                    "{}{} drawn, lighter",
                    count(found),
                    if m::yes(&path["detours_more"]) {
                        "+"
                    } else {
                        ""
                    }
                ),
            }
        ));
    }
    html.push_str("</span>");
    html
}
/// The state with the reach of a selected box set on one side, one to
/// `CONE_REACH` links; the other side stays as it was.
pub fn reaching(mut state: V, side: &str, hops: u64) -> V {
    let hops = hops.clamp(1, CONE_REACH as u64);
    let (up, down) = cone_of(&state);
    state["cone"] = match side {
        "up" => json!({"up": hops, "down": down}),
        "down" => json!({"up": up, "down": hops}),
        _ => return state,
    };
    state
}
/// The reach a state holds: to what uses the selected box, and to what it
/// uses. One on each side where none was set.
pub fn cone_of(state: &V) -> (usize, usize) {
    let side = |key: &str| {
        state["cone"][key]
            .as_u64()
            .map_or(1, |hops| hops.clamp(1, CONE_REACH as u64) as usize)
    };
    (side("up"), side("down"))
}
/// The key of the links where no picture is there to count them.
#[cfg(test)]
pub fn arrow_kinds_legend_html() -> String {
    kinds_legend_html(&V::Null, &[false; 7], "modal")
}

/// Optional live focus, changed-files catalog, and selected file-impact overlays for `canvas_with_overlays`.
#[derive(Clone, Debug, Default)]
pub struct CanvasOverlayContext<'a> {
    pub live_focus: Option<&'a LiveFocus>,
    pub changes: Option<&'a V>,
    pub impact: Option<&'a V>,
    /// The kinds of links the key of the picture has switched off.
    pub links_off: KindsOff,
    /// Whether the links that a longer way already joins are left out.
    pub hide_transitive: bool,
    /// How many links from the selected box are lit: against the arrows,
    /// to what uses it, and along them, to what it uses. None is one.
    pub cone: (usize, usize),
    pub pinned_positions: Option<&'a BTreeMap<String, (f64, f64)>>,
    pub path_nodes: Option<&'a BTreeSet<usize>>,
    pub path_edges: Option<&'a BTreeSet<(usize, usize)>>,
    pub isolate_cone: Option<&'a BTreeSet<usize>>,
    pub viewport_bounds: Option<ViewportBounds>,
    pub roving_focus: Option<usize>,
}
/// Whether a picture is one of the map itself: a folder, a change, an
/// opened file or followed links. In a picture of a way, of a cycle, of
/// what two places share, of a pattern, of a scenario or of modules every
/// link is what was asked for, and none of them is held back or left out.
pub(super) fn plain_map(view: &V) -> bool {
    !(paths(view)
        || cycles(view)
        || shares(view)
        || patterned(view)
        || matches!(view["mode"].as_str(), Some("scenario" | "modules")))
}
/// The links a picture would leave out as said by longer ways: those whose
/// ends are drawn, and that break no rule.
pub(super) fn said_twice(view: &V) -> Vec<bool> {
    let nodes = m::array(&view["nodes"]);
    let mut index_by_path: HashMap<&str, usize> = HashMap::with_capacity(nodes.len());
    for (idx, node) in nodes.iter().enumerate() {
        if let Some(p) = node["path"].as_str() {
            index_by_path.entry(p).or_insert(idx);
        }
    }
    let edges = m::array(&view["edges"]);
    let drawn = edges
        .iter()
        .map(|edge| {
            edge["from"]
                .as_str()
                .and_then(|f| index_by_path.get(f).copied())
                .zip(
                    edge["to"]
                        .as_str()
                        .and_then(|t| index_by_path.get(t).copied()),
                )
        })
        .collect::<Vec<_>>();
    let pairs = drawn.iter().flatten().copied().collect::<Vec<_>>();
    let mut said = transitive(nodes.len(), &pairs).into_iter();
    edges
        .iter()
        .zip(&drawn)
        .map(|(edge, ends)| {
            ends.is_some()
                && said.next().unwrap_or(false)
                && edge["architecture_violation"].as_u64().unwrap_or(0) == 0
        })
        .collect()
}
/// How many links of a picture a longer way already joins.
pub fn transitive_links(view: &V) -> usize {
    if !plain_map(view) {
        return 0;
    }
    said_twice(view).into_iter().filter(|said| *said).count()
}
