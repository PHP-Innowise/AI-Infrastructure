use super::*;

pub(super) fn native_run(run: &V) -> bool {
    !run["kinds"].as_array().into_iter().flatten().any(|kind| {
        matches!(
            kind.as_str(),
            Some("fleet_stage" | "fleet_reviewer" | "clash_turn" | "agent" | "agent_activity")
        )
    })
}

#[allow(dead_code)]
pub(super) fn run_has_activity(run: &V) -> bool {
    run["steps"].as_u64().unwrap_or(0) > 0
        || run["messages"].as_u64().unwrap_or(0) > 0
        || run["files"].as_object().is_some_and(|f| !f.is_empty())
        || !m::array(&run["plan"]).is_empty()
        || !m::array(&run["commands"]).is_empty()
        || !m::array(&run["searches"]).is_empty()
}

#[allow(dead_code)]
pub fn run_view_open(run: &V, session: &V) -> bool {
    m::active(session) || (run.is_object() && native_run(run) && run_has_activity(run))
}

/// What the strip above the composer shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    /// Nothing runs and no finished turn is open: the view is collapsed and inert.
    Closed,
    /// The session is queued, preparing, or runs something the strip does not describe.
    Waiting,
    /// A native launch is running: the Now line and, on request, its details.
    Now,
    /// The details of the finished turn, opened from the line under the conversation.
    After,
}

/// Fleet, Clash, Creator and System sessions keep their own progress: the strip only says they run.
pub(super) fn plain(session: &V) -> bool {
    session["workflow"] == "fleet-review"
        || [
            "fleet",
            "clash",
            "creator",
            "system_run",
            "system_discovery",
        ]
        .iter()
        .any(|key| session[*key].is_object())
}

pub(super) fn kinds(run: &V) -> impl Iterator<Item = &str> {
    run["kinds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(V::as_str)
}

/// The provider has written something for this launch; until then the session is only preparing.
pub(super) fn cli_started(run: &V) -> bool {
    kinds(run).any(|kind| {
        matches!(
            kind,
            "session" | "text" | "tool" | "plan" | "usage" | "result" | "error"
        )
    })
}

/// The launch the model holds cannot be the one that runs now: it closed itself, the person has
/// asked for the next turn since it started, or the page has already seen the session at rest
/// with it. `idle` is the launch the page last saw at rest.
pub(super) fn over(run: &V, idle: &V) -> bool {
    let launch = run["launch"].as_str().unwrap_or("");
    m::yes(&run["finished"])
        || idle.as_str() == Some(launch)
        || (!launch.is_empty() && kinds(run).any(|kind| kind == "user"))
}

/// The two lines a Harness check writes: its result and its end.
pub(super) fn check_status(text: &str) -> bool {
    text == "Check finished."
        || text.strip_prefix("Check ").is_some_and(|rest| {
            rest.split_once(": ").is_some_and(|(word, _)| {
                !word.is_empty() && word.chars().all(|c| c.is_alphanumeric() || c == '_')
            })
        })
}

/// A Harness check runs under a launch id of its own and writes only its status lines and, when
/// it could not run, an error: it is not a turn. A launch the provider wrote to is never one.
pub(super) fn is_check(run: &V) -> bool {
    m::yes(&run["check"])
        && !kinds(run).any(|kind| {
            matches!(
                kind,
                "session" | "text" | "tool" | "plan" | "usage" | "result"
            )
        })
}

/// The launch a finished turn's line and details describe: the newest one, or the one a Harness
/// check replaced.
pub(super) fn turn<'a>(run: &'a V, before: &'a V) -> &'a V {
    if is_check(run) {
        before
    } else {
        run
    }
}

/// A launch the Run view describes: a native one, in a session that keeps no progress of its own.
pub(super) fn described(session: &V, run: &V) -> bool {
    run.is_object() && native_run(run) && !plain(session)
}

/// Whether the Run view is open, and as what. `before` is the launch the newest one replaced,
/// `after` the launch whose finished details the person opened, and `idle` the launch the page
/// last saw at rest.
pub(super) fn mode(session: &V, run: &V, before: &V, after: &V, idle: &V) -> Mode {
    if !m::active(session) {
        // A finished turn's details stay open only while nothing runs, and only for the launch
        // they were opened for.
        let last = turn(run, before);
        let opened = after.as_str().is_some() && after.as_str() == last["launch"].as_str();
        return if session.is_object() && described(session, last) && opened {
            Mode::After
        } else {
            Mode::Closed
        };
    }
    if session["status"] == "queued"
        || !described(session, run)
        || is_check(run)
        || over(run, idle)
        || !cli_started(run)
    {
        Mode::Waiting
    } else {
        Mode::Now
    }
}

pub(super) const TABS: [&str; 6] = [
    "files",
    "plan",
    "commands",
    "memory",
    "omnigraph",
    "skillcache",
];

/// The launch recorded something for this tab.
pub(super) fn has(run: &V, tab: &str) -> bool {
    match tab {
        "files" => run["files"].as_object().is_some_and(|map| !map.is_empty()),
        "memory" | "omnigraph" => run[tab].is_object(),
        _ => run[tab].as_array().is_some_and(|list| !list.is_empty()),
    }
}

/// What the turn's memory save did: the launch's last `memory` event.
pub(crate) fn render_run_memory_pane_html(event: &V) -> String {
    if !event.is_object() {
        return String::new();
    }
    let state = match event["ok"].as_bool() {
        Some(true) => "Saved",
        Some(false) => "Not saved",
        None => "Memory",
    };
    let mut facts = Vec::new();
    for (field, label) in [
        ("delivered_sources", "sources delivered"),
        ("opened_delivered_sources", "delivered sources opened"),
        ("reported_used_sources", "sources reported as used"),
    ] {
        if let Some(count) = event[field].as_u64() {
            facts.push(format!("<li>{count} {label}</li>"));
        }
    }
    if let Some(attestation) = event["attestation"].as_str().filter(|s| !s.is_empty()) {
        facts.push(format!(
            "<li>Attested by {}</li>",
            crate::omnigraph::esc(attestation)
        ));
    }
    format!(
        "<p class=\"run-pane-state\" data-ok=\"{}\">{state}</p><p class=\"run-pane-text\">{}</p>{}",
        event["ok"]
            .as_bool()
            .map_or("unknown", |ok| if ok { "true" } else { "false" }),
        crate::omnigraph::esc(event["text"].as_str().unwrap_or("")),
        if facts.is_empty() {
            String::new()
        } else {
            format!("<ul class=\"run-pane-facts\">{}</ul>", facts.concat())
        }
    )
}

/// What the turn did with the shared Omnigraph context: the launch's last
/// `omnigraph` event.
pub(crate) fn render_run_omnigraph_pane_html(event: &V) -> String {
    if !event.is_object() {
        return String::new();
    }
    let phase = event["phase"].as_str().unwrap_or("update");
    let mut facts = Vec::new();
    for (field, label) in [
        ("published_atoms", "atoms published"),
        ("imported_atoms", "atoms imported"),
        ("synced_atoms", "atoms synced"),
    ] {
        let count = event[field]
            .as_u64()
            .or_else(|| event[field].as_array().map(|list| list.len() as u64));
        if let Some(count) = count {
            facts.push(format!("<li>{count} {label}</li>"));
        }
    }
    if let Some(branch) = event["branch"]
        .as_str()
        .or_else(|| event["target_branch"].as_str())
        .filter(|s| !s.is_empty())
    {
        facts.push(format!(
            "<li>Branch <code>{}</code></li>",
            crate::omnigraph::esc(branch)
        ));
    }
    format!(
        "<p class=\"run-pane-state\" data-ok=\"{}\">{}</p><p class=\"run-pane-text\">{}</p>{}",
        event["ok"]
            .as_bool()
            .map_or("unknown", |ok| if ok { "true" } else { "false" }),
        crate::omnigraph::esc(&m::label(phase)),
        crate::omnigraph::esc(event["text"].as_str().unwrap_or("")),
        if facts.is_empty() {
            String::new()
        } else {
            format!("<ul class=\"run-pane-facts\">{}</ul>", facts.concat())
        }
    )
}

/// The tabs a finished turn can open from the line under the conversation: none while a turn
/// runs, for a session that keeps its own progress, or when the launch recorded nothing.
pub(super) fn openers(session: &V, run: &V) -> Vec<&'static str> {
    if !session.is_object() || m::active(session) || !described(session, run) {
        return vec![];
    }
    TABS.into_iter().filter(|tab| has(run, tab)).collect()
}

/// The tab the panel shows: the chosen one while the launch has something for it, else Files.
pub(super) fn shown_tab<'a>(run: &V, chosen: &'a str) -> &'a str {
    if TABS.contains(&chosen) && has(run, chosen) {
        chosen
    } else {
        "files"
    }
}

pub(super) fn counts(run: &V) -> String {
    format!(
        "{} · {}",
        count(run["steps"].as_u64().unwrap_or(0), "step"),
        count(run["messages"].as_u64().unwrap_or(0), "message")
    )
}

pub(super) fn number(file: &V, key: &str) -> u64 {
    file[key].as_u64().unwrap_or(0)
}

/// The launch's files in the order they were first touched.
pub(super) fn files_of(run: &V) -> Vec<V> {
    let mut files: Vec<V> = run["files"]
        .as_object()
        .map(|map| map.values().cloned().collect())
        .unwrap_or_default();
    files.sort_by_key(|file| number(file, "order"));
    files
}

/// Map layout: the top folder is a district (a WordPress plugin or theme is one), and up to
/// three more folders make a block.
pub(super) fn place_of(file: &V) -> (String, String) {
    place_of_path(&m::text(file, "path"), m::yes(&file["outside"]))
}

/// The patterns of the searches whose scope holds the file; a search without a scope looked
/// everywhere in the project.
pub(super) fn searched_in(run: &V, file: &V) -> Vec<String> {
    if m::yes(&file["outside"]) {
        return vec![];
    }
    let path = m::text(file, "path");
    m::array(&run["searches"])
        .iter()
        .filter(|search| {
            let scope = m::text(search, "scope");
            let scope = scope.trim_end_matches('/');
            scope.is_empty()
                || scope == "."
                || path == scope
                || path.starts_with(&format!("{scope}/"))
        })
        .map(|search| m::text(search, "pattern"))
        .filter(|pattern| !pattern.is_empty())
        .collect()
}

pub(super) fn entry_from_run_file(
    run: &V,
    file: &V,
    open_paths: &std::collections::BTreeSet<String>,
) -> FileTouchEntry {
    let path = m::text(file, "path");
    FileTouchEntry {
        service_id: None,
        service_folder: None,
        agents: vec![],
        reads: number(file, "reads"),
        edits: number(file, "edits"),
        created: m::yes(&file["created"]),
        deleted: m::yes(&file["deleted"]),
        failed: m::yes(&file["failed"]),
        outside: m::yes(&file["outside"]),
        helper: m::yes(&file["helper"]),
        main: m::yes(&file["main"]),
        now: open_paths.contains(&path),
        order: number(file, "order") as usize,
        first_step: number(file, "first_step"),
        last_step: number(file, "last_step"),
        searched_patterns: searched_in(run, file),
        event_id: file["event_id"].clone(),
        timeline_agent: None,
        path,
    }
}

pub(super) fn entries_of_run(run: &V, running: bool) -> Vec<FileTouchEntry> {
    let mut open = std::collections::BTreeSet::new();
    if running {
        for call in run["calls"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(_, call)| call)
        {
            if m::yes(&call["open"]) {
                open.extend(
                    call["touched"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .map(|(path, _)| path.clone()),
                );
            }
        }
    }
    files_of(run)
        .iter()
        .map(|file| entry_from_run_file(run, file, &open))
        .collect()
}

#[cfg(test)]
pub(super) fn matches(run: &V, file: &V, filter: &str) -> bool {
    let empty = std::collections::BTreeSet::new();
    files_map::matches_filter(&entry_from_run_file(run, file, &empty), filter)
}

/// The filters worth offering: All, and each kind that has a file. A choice whose kind has no
/// file any more reads as All.
pub(super) fn filters(run: &V, chosen: &str) -> Vec<V> {
    let entries = entries_of_run(run, false);
    let (_, pills) = files_map::offered_filters(&entries, chosen);
    pills
        .into_iter()
        .map(|pill| json!({"id": pill.id, "label": pill.label, "pressed": pill.pressed}))
        .collect()
}

/// What a chip says after its path to someone who cannot see its marks.
#[cfg(test)]
pub(super) fn chip_words(file: &V) -> Vec<String> {
    let empty = std::collections::BTreeSet::new();
    files_map::chip_words_for(&entry_from_run_file(&V::Null, file, &empty))
}

/// The facts on a file's card.
#[cfg(test)]
pub(super) fn card_facts(run: &V, file: &V) -> Vec<String> {
    let empty = std::collections::BTreeSet::new();
    files_map::card_facts_for(&entry_from_run_file(run, file, &empty))
}

/// The map as data: districts, their blocks and the chips in them, in the order the files were
/// first touched. The filter dims the chips it leaves out, and while the launch runs a ring
/// marks the files of the calls that are open.
pub(super) fn map(run: &V, filter: &str, running: bool) -> Vec<V> {
    let entries = entries_of_run(run, running);
    files_map::group_districts(&entries, filter, running, false, false)
        .into_iter()
        .map(|district| {
            let blocks: Vec<V> = district
                .blocks
                .into_iter()
                .map(|block| {
                    let files: Vec<V> = block
                        .files
                        .into_iter()
                        .map(|chip| {
                            json!({
                                "path": chip.path,
                                "name": chip.name,
                                "mark": chip.mark,
                                "times": chip.times,
                                "edited": chip.edited,
                                "failed": chip.failed,
                                "created": chip.created,
                                "deleted": chip.deleted,
                                "helper": chip.helper,
                                "dim": chip.dim,
                                "now": chip.now,
                                "label": chip.label,
                            })
                        })
                        .collect();
                    json!({
                        "name": block.name,
                        "label": block.label,
                        "searched": block.searched,
                        "files": files,
                    })
                })
                .collect();
            json!({
                "name": district.name,
                "label": district_label(&district.name),
                "blocks": blocks,
            })
        })
        .collect()
}

#[derive(Debug, PartialEq)]
pub(super) struct RunClock {
    pub(super) started: Option<String>,
    pub(super) running: bool,
}

pub(super) fn clock_state(run: &V, session: &V) -> Option<RunClock> {
    if !run.is_object() || !native_run(run) {
        return None;
    }
    Some(RunClock {
        started: run["started"].as_str().map(str::to_owned),
        running: !m::yes(&run["finished"]) && m::active(session),
    })
}

pub(super) fn update_clock() {
    let clock = m::read(|state| {
        state
            .data
            .get("run")
            .and_then(|run| clock_state(run, &state.selected))
    });
    let Some(clock) = clock else {
        dom::cancel_timer("run-clock");
        return;
    };
    let since = clock
        .started
        .as_deref()
        .map(js_sys::Date::parse)
        .filter(|n| n.is_finite())
        .map(|n| ((js_sys::Date::now() - n) / 1000.).max(0.) as u64);
    dom::text(
        "run-clock",
        &since
            .map(|n| format!("{}m {:02}s", n / 60, n % 60))
            .unwrap_or_default(),
    );
    if clock.running {
        dom::later("run-clock", 1000, update_clock);
    } else {
        dom::cancel_timer("run-clock");
    }
}

pub(super) fn set_inert(id: &str, value: bool) {
    if let Some(e) = dom::get(id) {
        if value {
            let _ = e.set_attribute("inert", "");
        } else {
            let _ = e.remove_attribute("inert");
        }
    }
}

pub(super) fn focus_within(ids: &[&str]) -> bool {
    let Some(focused) = dom::document().active_element() else {
        return false;
    };
    ids.iter()
        .any(|id| dom::get(id).is_some_and(|e| e.contains(Some(&focused))))
}

/// Focus without scrolling: the view clips its content while it opens, and a scrolling focus
/// would shift what it clips.
pub(super) fn focus(id: &str) {
    if let Some(node) = dom::get(id) {
        focus_node(&node);
    }
}

/// The Calm/Detailed switch that is on screen: a narrow window shows only the compact chip.
pub(super) fn detail_switch() -> &'static str {
    let narrow = dom::window()
        .match_media("(max-width:640px)")
        .ok()
        .flatten()
        .is_some_and(|media| media.matches());
    if narrow {
        "run-compact"
    } else {
        "run-calm"
    }
}

/// Opens or collapses the view and its panel. The markup ships both collapsed and inert, and the
/// stylesheet shows the view only as `data-open="true"` and the panel only as
/// `data-detailed="true"` on the card.
pub(super) fn reveal(mode: Mode, detailed: bool) {
    let strip = matches!(mode, Mode::Now | Mode::After);
    let panel = strip && detailed;
    // Keyboard focus in a part that is about to go inert would fall to <body>: it moves first,
    // to the Calm/Detailed switch while the strip stays, else to the composer.
    if !strip && focus_within(&["run-view", "run-file-card"]) {
        focus("prompt");
    } else if !panel && focus_within(&["run-panel", "run-file-card"]) {
        focus(detail_switch());
    }
    if !panel {
        hide_card(false);
    }
    dom::attr("run-view", "data-open", truth(mode != Mode::Closed));
    set_inert("run-view", mode == Mode::Closed);
    dom::attr("run-card", "data-detailed", truth(panel));
    dom::attr("run-view", "data-detailed", truth(panel));
    set_inert("run-panel", !panel);
    for (id, pressed) in [
        ("run-calm", !detailed),
        ("run-detailed", detailed),
        ("run-compact", detailed),
    ] {
        dom::attr(id, "aria-pressed", truth(pressed));
    }
    dom::hidden("waiting", mode != Mode::Waiting);
    dom::hidden("run-now", !strip);
    dom::hidden("run-line2", !strip);
    dom::hidden(
        "run-notify",
        mode != Mode::Now || !super::can_prompt_notifications(),
    );
    dom::hidden("run-close", mode != Mode::After);
    dom::attr(
        "run-dot",
        "data-state",
        if mode == Mode::Now { "running" } else { "done" },
    );
    dom::attr("run-dot", "data-pulse", truth(mode == Mode::Now));
}

pub(super) const TURN: &str = "run-turn";

/// The line under the conversation that opens a finished turn's details. The page keeps the
/// newest launch only, so there is one line, and it goes when the next turn starts.
pub(super) fn turn_line(session: &V, run: &V) {
    let tabs = openers(session, run);
    let old = dom::get(TURN);
    if tabs.is_empty() {
        if let Some(old) = old {
            old.remove();
        }
        return;
    }
    let text = format!("Last run · {}", counts(run));
    let signature = format!("{text}|{}", tabs.join(","));
    let Some(events) = dom::get("events") else {
        return;
    };
    let line = match old {
        Some(old) if old.get_attribute("data-signature").as_deref() == Some(&signature) => old,
        old => {
            if let Some(old) = old {
                old.remove();
            }
            let line = dom::el("article", "run-receipt", "");
            let _ = line.set_attribute("id", TURN);
            let _ = line.set_attribute("data-compact", "true");
            let _ = line.set_attribute("data-signature", &signature);
            let _ = line.set_attribute("aria-label", "Last run");
            dom::add(&line, &dom::el("span", "", &text));
            for tab in tabs {
                let open = dom::button(&format!("Show {tab}"), "run-open", json!(tab));
                open.set_class_name("run-link");
                dom::add(&line, &open);
            }
            line
        }
    };
    // It follows the log instead of living in it: the log is a live region and is cleared whole.
    if events.next_element_sibling().as_ref() != Some(&line) {
        let _ = events.after_with_node_1(&line);
    }
}

/// The away summary takes the counts' place on the strip's second line while it shows.
pub(super) fn away(shown: bool) {
    dom::hidden("run-away", !shown);
    dom::hidden("run-chips", shown);
}

pub(super) fn chip(text: &str) {
    let Some(chips) = dom::get("run-chips") else {
        return;
    };
    match chips.first_element_child() {
        Some(chip) => {
            if chip.text_content().as_deref() != Some(text) {
                chip.set_text_content(Some(text));
            }
        }
        None => dom::add(&chips, &dom::el("span", "run-chip", text)),
    }
}

/// The launch the newest one replaced, read only when the newest is a Harness check: nothing
/// else looks behind the newest launch.
pub(super) fn earlier(run: &V) -> V {
    if is_check(run) {
        m::data("run-before")
    } else {
        V::Null
    }
}

pub(super) fn chip_node(chip: &V) -> web_sys::Element {
    let path = m::text(chip, "path");
    let node = dom::el("button", "run-file", "");
    for (key, value) in [
        ("type", "button".to_owned()),
        ("data-action", "run-file".to_owned()),
        ("data-payload", json!(path).to_string()),
        ("data-focus-key", path.clone()),
        ("data-path", path.clone()),
        ("title", path.clone()),
        ("aria-label", m::text(chip, "label")),
    ] {
        let _ = node.set_attribute(key, &value);
    }
    for flag in ["edited", "failed", "created", "deleted", "helper", "dim"] {
        let _ = node.set_attribute(&format!("data-{flag}"), truth(m::yes(&chip[flag])));
    }
    if m::yes(&chip["now"]) {
        let _ = node.set_attribute("data-now", "static");
    }
    dom::add(
        &node,
        &hushed(dom::el("span", "run-file-mark", &m::text(chip, "mark"))),
    );
    dom::add(&node, &hushed(bdi("run-file-name", &m::text(chip, "name"))));
    dom::add(
        &node,
        &hushed(dom::el("span", "run-file-count", &m::text(chip, "times"))),
    );
    node
}

pub(super) fn district_node(district: &V) -> web_sys::Element {
    let wrap = dom::el("div", "run-district", "");
    let label = dom::el("div", "run-district-label", "");
    dom::add(&label, &bdi("", &m::text(district, "label")));
    dom::add(&wrap, &label);
    let blocks = dom::el("div", "run-blocks", "");
    for block in m::array(&district["blocks"]) {
        let node = dom::el("div", "run-block", "");
        let _ = node.set_attribute("role", "group");
        let _ = node.set_attribute("aria-label", &m::text(&block, "label"));
        let _ = node.set_attribute("data-searched", truth(m::yes(&block["searched"])));
        let name = m::text(&block, "name");
        if !name.is_empty() {
            let head = dom::el("div", "run-block-head", "");
            dom::add(&head, &bdi("run-block-name", &name));
            dom::add(&node, &head);
        }
        let files = dom::el("div", "run-files", "");
        for chip in m::array(&block["files"]) {
            dom::add(&files, &chip_node(&chip));
        }
        dom::add(&node, &files);
        dom::add(&blocks, &node);
    }
    dom::add(&wrap, &blocks);
    wrap
}

/// The filter buttons are kept and relabelled, not rebuilt: the one just pressed keeps the focus.
pub(super) fn filter_buttons(items: &[V]) {
    let Some(container) = dom::get("run-files-filters") else {
        return;
    };
    let ids: Vec<String> = items
        .iter()
        .map(|item| format!("run-filter-{}", m::text(item, "id")))
        .collect();
    for child in dom::children(&container) {
        if !ids.contains(&child.id()) {
            child.remove();
        }
    }
    for (index, item) in items.iter().enumerate() {
        let node = dom::get(&ids[index]).unwrap_or_else(|| {
            let node = dom::el("button", "run-filter", "");
            let _ = node.set_attribute("type", "button");
            let _ = node.set_attribute("id", &ids[index]);
            node
        });
        let label = m::text(item, "label");
        if node.text_content().as_deref() != Some(label.as_str()) {
            node.set_text_content(Some(&label));
        }
        let _ = node.set_attribute("aria-pressed", truth(m::yes(&item["pressed"])));
        let current = container.children().item(index as u32);
        if current.as_ref() != Some(&node) {
            let _ = container.insert_before(&node, current.as_ref().map(|e| e.as_ref()));
        }
    }
}

pub(super) const CARD: &str = "run-file-card";

/// The file whose card is open in Sessions Run view.
pub(super) fn card_path() -> Option<String> {
    m::data("run-card").as_str().map(str::to_owned)
}

pub(super) fn chip_of(path: &str) -> Option<web_sys::Element> {
    dom::all("#run-map-main .run-file")
        .into_iter()
        .find(|chip| chip.get_attribute("data-focus-key").as_deref() == Some(path))
}

pub(super) fn run_card_actions(file: &V) -> Vec<FileCardAction> {
    let path = m::text(file, "path");
    let outside = m::yes(&file["outside"]);
    let mut actions = vec![FileCardAction {
        label: "Show in conversation".into(),
        action: "run-reveal".into(),
        payload: file["event_id"].clone(),
        data_service: None,
        data_path: None,
    }];
    if !outside && !path.is_empty() {
        actions.push(FileCardAction {
            label: "Show on dependency map".into(),
            action: "inspect-file-graph".into(),
            payload: json!({"path": path}),
            data_service: None,
            data_path: Some(path.clone()),
        });
    }
    actions.push(FileCardAction {
        label: "Add path to message".into(),
        action: "run-add-path".into(),
        payload: json!(path),
        data_service: None,
        data_path: None,
    });
    actions.push(FileCardAction {
        label: "Copy path".into(),
        action: "run-copy-path".into(),
        payload: json!(path),
        data_service: None,
        data_path: None,
    });
    actions.push(FileCardAction {
        label: "Close".into(),
        action: "run-card-close".into(),
        payload: V::Null,
        data_service: None,
        data_path: None,
    });
    actions
}

/// Writes the card for a file. An unchanged card is left alone, and a rewritten one gives the
/// focus back to the action that had it.
pub(super) fn fill_card(run: &V, file: &V) {
    let empty = std::collections::BTreeSet::new();
    let entry = entry_from_run_file(run, file, &empty);
    let actions = run_card_actions(file);
    DISMISS_ON_GRAPH_CLICK.with(|cb| {
        files_map::fill_card_element(
            CARD,
            "run",
            &entry,
            &actions,
            Some(("inspect-file-graph", cb)),
        );
    });
}

pub(super) fn place_card(chip: &web_sys::Element) {
    files_map::position_card_element(CARD, chip);
}

/// Opens the card of a file beside its chip; the first of its actions takes the focus.
pub(super) fn show_card(run: &V, path: &str) {
    let Some(file) = run["files"].get(path) else {
        return;
    };
    if let Some(active) = dom::document().active_element() {
        let inside_card = dom::get(CARD).is_some_and(|card| card.contains(Some(&active)));
        if !inside_card {
            CARD_OPENER.with(|cell| *cell.borrow_mut() = Some(active));
        }
    }
    m::set("run-card", json!(path));
    if let Some(card) = dom::get(CARD) {
        let _ = card.remove_attribute("data-signature");
    }
    fill_card(run, file);
    dom::hidden(CARD, false);
    if let Some(chip) = chip_of(path) {
        place_card(&chip);
    }
    let selector = format!("#{CARD} button");
    if let Ok(Some(first)) = dom::document().query_selector(&selector) {
        focus_node(&first);
    } else if let Some(card) = dom::get(CARD) {
        focus_node(&card);
    }
}

/// Dismisses the file detail card when open, optionally restoring focus.
pub(crate) fn dismiss_file_card(restore: bool) {
    hide_card(restore);
}

/// Puts the card away; `restore` gives the focus back to the control that opened it (or its chip).
pub(super) fn hide_card(restore: bool) {
    let opener = CARD_OPENER.with(|cell| cell.borrow_mut().take());
    let path = m::take("run-card");
    let owned_by_run = path.is_string()
        || dom::get(CARD)
            .and_then(|c| c.get_attribute("data-owner"))
            .as_deref()
            == Some("run");
    if !owned_by_run {
        return;
    }
    if !restore {
        dom::hidden_with_restore(CARD, true, false);
        return;
    }
    let explicit_target = opener
        .filter(|el| dom::document().contains(Some(el)))
        .or_else(|| path.as_str().and_then(chip_of));
    if let Some(el) = explicit_target {
        dom::hidden_with_restore(CARD, true, false);
        focus_node(&el);
    } else {
        dom::hidden_with_restore(CARD, true, true);
    }
}

/// The launch the strip and the panel show: the one that runs, or at rest the last turn's.
pub(super) fn shown() -> V {
    let (session, run) = (m::read(|s| s.selected.clone()), m::data("run"));
    if m::active(&session) {
        run
    } else {
        let before = earlier(&run);
        turn(&run, &before).clone()
    }
}

/// The Files tab: the summary, the filters, the map of touched files and the same set as a list.
pub(super) fn files_pane(run: &V, running: bool) {
    let files = files_of(run);
    let entries = entries_of_run(run, running);
    let summary = files_map::files_summary(&entries);
    dom::text("run-badge-files", &files.len().to_string());
    dom::text("run-files-summary", &summary);
    let list = m::data("run-files-view") == "list";
    let offered = filters(run, m::data("run-file-filter").as_str().unwrap_or("all"));
    let filter = offered
        .iter()
        .find(|item| m::yes(&item["pressed"]))
        .map(|item| m::text(item, "id"))
        .unwrap_or_else(|| "all".into());
    dom::attr("run-view-map", "aria-pressed", truth(!list));
    dom::attr("run-view-list", "aria-pressed", truth(list));
    dom::hidden("run-map", list);
    dom::hidden("run-tree", !list);
    dom::hidden("run-files-empty", !files.is_empty());
    // The filters dim chips and the legend explains them: both belong to the map.
    dom::hidden("run-files-filters", list || files.is_empty());
    dom::hidden("run-legend", list || files.is_empty());
    filter_buttons(&offered);
    dom::keyed_render(
        "run-map-main",
        &map(run, &filter, running),
        |district| m::text(district, "name"),
        district_node,
    );
    let mut districts: BTreeMap<String, Vec<V>> = BTreeMap::new();
    for file in &files {
        let (district, _) = place_of(file);
        let name = match district.as_str() {
            OUTSIDE => "Outside project".to_owned(),
            "" => "Project root".to_owned(),
            _ => district,
        };
        districts.entry(name).or_default().push(file.clone());
    }
    let districts = districts
        .into_iter()
        .map(|(name, mut files)| {
            files.sort_by_key(|file| m::text(file, "path"));
            json!({"name":name,"files":files})
        })
        .collect::<Vec<_>>();
    dom::keyed_render(
        "run-tree",
        &districts,
        |v| m::text(v, "name"),
        |v| {
            let d = dom::el("details", "run-tree-folder", "");
            let _ = d.set_attribute("open", "");
            let _ = d.set_attribute("data-restore-key", &m::text(v, "name"));
            let files = m::array(&v["files"]);
            let summary = dom::el(
                "summary",
                "",
                &format!("{} · {}", m::text(v, "name"), files.len()),
            );
            let _ = summary.set_attribute("role", "treeitem");
            let _ = summary.set_attribute("aria-expanded", "true");
            let _ = summary.set_attribute("tabindex", "-1");
            dom::add(&d, &summary);
            for file in files {
                let path = m::text(&file, "path");
                let outside = m::yes(&file["outside"]);
                let row = dom::el("div", "run-row-actions", "");
                let label = format!(
                    "{} · {} · {}",
                    path,
                    count(number(&file, "reads"), "read"),
                    count(number(&file, "edits"), "edit")
                );
                let b = dom::button(&label, "run-reveal", file["event_id"].clone());
                let _ = b.set_attribute("data-focus-key", &path);
                let _ = b.set_attribute("role", "treeitem");
                let _ = b.set_attribute("tabindex", "-1");
                dom::add(&row, &b);
                if !outside && !path.is_empty() {
                    let map_btn = dom::button("Map", "inspect-file-graph", json!({"path": path}));
                    map_btn.set_class_name("button compact");
                    let _ = map_btn.set_attribute("data-path", &path);
                    let _ = map_btn.set_attribute("tabindex", "-1");
                    let _ = map_btn
                        .set_attribute("aria-label", &format!("Show {path} on dependency map"));
                    dom::add(&row, &map_btn);
                }
                dom::add(&d, &row);
            }
            d
        },
    );
    if let Some(tree) = dom::get("run-tree") {
        dom::sync_roving_widget(&tree);
    }
    // An open card follows its file; a file that is gone, or a map that is, takes it along.
    if let Some(path) = card_path() {
        match run["files"].get(&path) {
            Some(file) if !list => fill_card(run, file),
            _ => hide_card(false),
        }
    }
}

/// Puts a path into the message being written, at the caret; Send stays the person's.
pub(super) fn insert_draft(text: &str) {
    let Some(prompt) =
        dom::get("prompt").and_then(|e| e.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
    else {
        return;
    };
    if prompt.disabled() {
        return;
    }
    let value = prompt.value();
    let units = value.encode_utf16().count() as u32;
    let start = prompt
        .selection_start()
        .ok()
        .flatten()
        .unwrap_or(units)
        .min(units);
    let end = prompt
        .selection_end()
        .ok()
        .flatten()
        .unwrap_or(start)
        .clamp(start, units);
    let (draft, caret) = spliced(&value, start, end, text);
    prompt.set_value(&draft);
    let _ = prompt.set_selection_range(caret, caret);
    focus("prompt");
    session::controls();
    session::save_preferences();
}

/// Escape puts away the nearest thing the person opened: the approvals drawer, the file card, then the details.
#[allow(dead_code)]
pub(crate) fn escape() -> bool {
    if m::read(|s| s.approvals.drawer_open) {
        close_approvals_drawer();
        true
    } else if card_path().is_some() {
        hide_card(true);
        true
    } else if m::yes(&m::data("run-detailed")) && focus_within(&["run-view"]) {
        detail(false);
        focus(detail_switch());
        true
    } else {
        false
    }
}

/// A press anywhere but the file card and the chips puts the card away.
#[allow(dead_code)]
pub(crate) fn pointer_down(target: &web_sys::Element) {
    if card_path().is_some()
        && target
            .closest("#run-file-card, .run-file")
            .ok()
            .flatten()
            .is_none()
    {
        hide_card(false);
    }
}

/// Opens the finished turn's details on a tab; while a turn runs the strip is its own.
pub(super) fn open(tab: &str) {
    let (session, run) = (m::read(|s| s.selected.clone()), m::data("run"));
    let before = earlier(&run);
    let last = turn(&run, &before);
    let tab = shown_tab(last, tab);
    if !openers(&session, last).contains(&tab) {
        return;
    }
    m::set("run-after", last["launch"].clone());
    m::set("run-tab", json!(tab));
    m::set("run-detailed", json!(true));
    render();
    focus(&format!("run-tab-{tab}"));
}

pub(super) fn close() {
    m::take("run-after");
    render();
    focus("prompt");
}

pub(super) fn detail(on: bool) {
    m::set("run-detailed", json!(on));
    render();
}

pub fn activity(event: &V) {
    let sid = m::read(|s| s.sid.clone());
    crate::activity::observe_session(&sid, event);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OutcomeFindingCard {
    pub(super) key: String,
    pub(super) severity_class: String,
    pub(super) severity_label: String,
    pub(super) state_class: Option<String>,
    pub(super) state_label: Option<String>,
    pub(super) heading: String,
    pub(super) location: Option<String>,
    pub(super) claim: String,
    pub(super) evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ClashTurnCard {
    pub(super) key: String,
    pub(super) state_class: String,
    pub(super) state_label: String,
    pub(super) heading: String,
    pub(super) meta: Option<String>,
    pub(super) summary: String,
    pub(super) error: Option<String>,
}

pub(super) fn sanitize_token(raw: &str, fallback: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = true;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if (ch == '-' || ch == '_' || ch.is_ascii_whitespace()) && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub(super) fn format_finding_location(file: &str, line: &V) -> Option<String> {
    let file = file.trim();
    if file.is_empty() {
        return None;
    }
    if let Some(num) = line.as_u64().filter(|&n| n > 0) {
        Some(format!("{file}:{num}"))
    } else if let Some(text) = line
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "0")
    {
        Some(format!("{file}:{text}"))
    } else {
        Some(file.to_owned())
    }
}

pub(super) fn non_empty_summary_text(v: &V, keys: &[&str]) -> String {
    for key in keys {
        let text = m::text(v, key);
        if !text.trim().is_empty() {
            return text.trim().to_owned();
        }
    }
    if let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return s.to_owned();
    }
    m::summary(v)
}

pub(super) fn format_clash_history(item: &V) -> Option<String> {
    let history = m::array(&item["history"]);
    let last = history.last()?;
    let cycle = last["cycle"].as_u64().map(|n| format!("c{n}"));
    let round = last["round"].as_u64().map(|n| format!("r{n}"));
    let actor = m::label(m::text(last, "actor").trim());
    let action = m::label(m::text(last, "action").trim());
    let note = m::text(last, "note");
    let head = [
        cycle,
        round,
        (!actor.is_empty()).then_some(actor),
        (!action.is_empty()).then_some(action),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let tail = note.trim();
    let entry = match (head.is_empty(), tail.is_empty()) {
        (false, false) => format!("{head} — {tail}"),
        (false, true) => head,
        (true, false) => tail.to_owned(),
        (true, true) => String::new(),
    };
    if entry.is_empty() {
        None
    } else {
        Some(format!("History ({}): {entry}", history.len()))
    }
}

pub(super) fn build_fleet_finding_card(item: &V, idx: usize) -> OutcomeFindingCard {
    let raw_sev = m::text(item, "severity");
    let severity_class = sanitize_token(&raw_sev, "info");
    let severity_label = if raw_sev.trim().is_empty() {
        "info".to_owned()
    } else {
        m::label(raw_sev.trim())
    };
    let lens = m::label(m::text(item, "lens").trim());
    let heading = if lens.is_empty() {
        "Finding".to_owned()
    } else {
        lens
    };
    let location = format_finding_location(&m::text(item, "file"), &item["line"]);
    let claim = non_empty_summary_text(item, &["claim", "summary", "title"]);
    let evidence = m::text(item, "evidence");
    let evidence = (!evidence.trim().is_empty()).then(|| evidence.trim().to_owned());
    let key = format!(
        "{idx}:{severity_class}:{}:{claim}",
        location.as_deref().unwrap_or("")
    );
    OutcomeFindingCard {
        key,
        severity_class,
        severity_label,
        state_class: None,
        state_label: None,
        heading,
        location,
        claim,
        evidence,
    }
}

pub(super) fn build_clash_item_card(item: &V, idx: usize) -> OutcomeFindingCard {
    let raw_sev = m::text(item, "severity");
    let severity_class = sanitize_token(&raw_sev, "unrated");
    let severity_label = if raw_sev.trim().is_empty() {
        "unrated".to_owned()
    } else {
        m::label(raw_sev.trim())
    };
    let raw_status = m::text(item, "status");
    let state_class = sanitize_token(&raw_status, "open");
    let state_label = if raw_status.trim().is_empty() {
        "open".to_owned()
    } else {
        m::label(raw_status.trim())
    };
    let id = m::text(item, "id");
    let origin = m::label(m::text(item, "origin").trim());
    let kind = m::label(m::text(item, "kind").trim());
    let role_kind = [origin, kind]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let heading = match (id.trim().is_empty(), role_kind.is_empty()) {
        (false, false) => format!("{} · {role_kind}", id.trim()),
        (false, true) => id.trim().to_owned(),
        (true, false) => role_kind,
        (true, true) => "Clash item".to_owned(),
    };
    let location = format_finding_location(&m::text(item, "file"), &item["line"]);
    let claim = non_empty_summary_text(item, &["claim", "summary", "title"]);
    let raw_ev = m::text(item, "evidence");
    let evidence = match (raw_ev.trim().is_empty(), format_clash_history(item)) {
        (false, Some(hist)) => Some(format!("{}\n{hist}", raw_ev.trim())),
        (false, None) => Some(raw_ev.trim().to_owned()),
        (true, Some(hist)) => Some(hist),
        (true, None) => None,
    };
    let key = format!(
        "{idx}:{id}:{state_class}:{}:{claim}",
        location.as_deref().unwrap_or("")
    );
    OutcomeFindingCard {
        key,
        severity_class,
        severity_label,
        state_class: Some(state_class),
        state_label: Some(state_label),
        heading,
        location,
        claim,
        evidence,
    }
}

pub(super) fn build_clash_turn_card(turn: &V, idx: usize) -> ClashTurnCard {
    let (state_class, state_label) = match turn["ok"].as_bool() {
        Some(true) => ("resolved".to_owned(), "ok".to_owned()),
        Some(false) => ("disputed".to_owned(), "failed".to_owned()),
        None => {
            let status = m::text(turn, "status");
            let cls = sanitize_token(&status, "open");
            let lbl = if status.trim().is_empty() {
                "recorded".to_owned()
            } else {
                m::label(status.trim())
            };
            (cls, lbl)
        }
    };
    let cycle = turn["cycle"].as_u64().map(|n| format!("Cycle {n}"));
    let round = turn["round"].as_u64().map(|n| format!("Round {n}"));
    let role = m::label(m::text(turn, "role").trim());
    let provider = m::text(turn, "provider");
    let actor = match (role.is_empty(), provider.trim().is_empty()) {
        (false, false) => Some(format!("{role} ({})", provider.trim())),
        (false, true) => Some(role),
        (true, false) => Some(provider.trim().to_owned()),
        (true, true) => None,
    };
    let heading = [cycle, round, actor]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let heading = if heading.is_empty() {
        format!("Turn {}", idx + 1)
    } else {
        heading
    };
    let duration = turn["seconds"].as_f64().map(|s| format!("{s:.1}s"));
    let cost = turn["cost_usd"].as_f64().map(|c| format!("${c:.4}"));
    let meta = [duration, cost]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let meta = (!meta.is_empty()).then_some(meta);
    let summary_text = m::text(turn, "summary");
    let error_text = m::text(turn, "error");
    let (summary, error) = if !summary_text.trim().is_empty() {
        (
            summary_text.trim().to_owned(),
            (!error_text.trim().is_empty()).then(|| error_text.trim().to_owned()),
        )
    } else if !error_text.trim().is_empty() {
        (error_text.trim().to_owned(), None)
    } else {
        (non_empty_summary_text(turn, &["title", "status"]), None)
    };
    let key = format!("{idx}:{state_class}:{heading}");
    ClashTurnCard {
        key,
        state_class,
        state_label,
        heading,
        meta,
        summary,
        error,
    }
}

pub(super) fn fleet_summary_text(findings: &[V], cost: &V) -> String {
    let finding_count = count(findings.len() as u64, "finding");
    let cost_text = cost.as_f64().map(|c| format!("${c:.4}")).or_else(|| {
        let s = m::string(cost);
        (!s.trim().is_empty()).then(|| s.trim().to_owned())
    });
    match cost_text {
        Some(c) => format!("{finding_count} · reported cost {c}"),
        None => finding_count,
    }
}

pub(super) fn fleet_skipped_text(result: &V) -> String {
    let skipped = m::array(&result["skipped"])
        .into_iter()
        .filter_map(|item| {
            let text = item
                .as_str()
                .map(str::trim)
                .map(str::to_owned)
                .unwrap_or_else(|| m::summary(&item));
            (!text.is_empty()).then_some(text)
        })
        .collect::<Vec<_>>();
    if skipped.is_empty() {
        String::new()
    } else {
        format!("Skipped lenses: {}", skipped.join(", "))
    }
}

pub(super) fn clash_outcome_summary(r: &V, items: &[V], turns: &[V]) -> String {
    let direct = m::text(r, "summary");
    if !direct.trim().is_empty() {
        return direct.trim().to_owned();
    }
    let verdict = m::label(m::text(r, "verdict").trim());
    let status = m::label(m::text(r, "status").trim());
    let head = if !verdict.is_empty() {
        verdict
    } else if !status.is_empty() {
        status
    } else {
        String::new()
    };
    let mut parts = Vec::new();
    if !head.is_empty() {
        parts.push(head);
    }
    if let Some(cycles) = r["cycles"].as_u64() {
        parts.push(count(cycles, "cycle"));
    }
    parts.push(count(items.len() as u64, "item"));
    parts.push(count(turns.len() as u64, "turn"));
    if let Some(cost) = r["cost_usd"].as_f64() {
        parts.push(format!("${cost:.4}"));
    }
    parts.join(" · ")
}

pub(super) fn render_outcome_finding_card(card: &OutcomeFindingCard) -> web_sys::Element {
    let article = dom::el("article", "fleet-finding", "");
    let heading = dom::el("div", "fleet-finding-heading", "");
    dom::add(
        &heading,
        &dom::el(
            "span",
            &format!("fleet-severity {}", card.severity_class),
            &card.severity_label,
        ),
    );
    if let (Some(sc), Some(sl)) = (&card.state_class, &card.state_label) {
        dom::add(&heading, &dom::el("span", &format!("clash-state {sc}"), sl));
    }
    dom::add(&heading, &dom::el("strong", "", &card.heading));
    if let Some(location) = &card.location {
        dom::add(
            &heading,
            &dom::el("span", "fleet-finding-location", location),
        );
    }
    dom::add(&article, &heading);
    dom::add(&article, &dom::el("p", "fleet-finding-claim", &card.claim));
    if let Some(evidence) = &card.evidence {
        dom::add(&article, &dom::el("p", "fleet-evidence", evidence));
    }
    article
}

pub(super) fn render_clash_turn_card(card: &ClashTurnCard) -> web_sys::Element {
    let article = dom::el("article", "clash-turn", "");
    let heading = dom::el("div", "fleet-finding-heading", "");
    dom::add(
        &heading,
        &dom::el(
            "span",
            &format!("clash-state {}", card.state_class),
            &card.state_label,
        ),
    );
    dom::add(&heading, &dom::el("strong", "", &card.heading));
    if let Some(meta) = &card.meta {
        dom::add(&heading, &dom::el("span", "fleet-finding-location", meta));
    }
    dom::add(&article, &heading);
    dom::add(
        &article,
        &dom::el("p", "fleet-finding-claim", &card.summary),
    );
    if let Some(error) = &card.error {
        dom::add(&article, &dom::el("p", "fleet-evidence", error));
    }
    article
}

pub fn outcomes(session: &V) {
    let fleet = session["workflow"] == "fleet-review";
    let result = &session["fleet_result"];
    dom::hidden("fleet-progress", !fleet);
    dom::hidden("fleet-outcome", !fleet || result.is_null());
    if fleet {
        let findings = m::array(&result["findings"]);
        let cards = findings
            .iter()
            .enumerate()
            .map(|(idx, item)| json!({"idx": idx, "item": item}))
            .collect::<Vec<_>>();
        dom::text(
            "fleet-stage",
            &format!(
                "{} · {}",
                m::text(session, "status"),
                m::text(result, "stage")
            ),
        );
        dom::text(
            "fleet-result-summary",
            &fleet_summary_text(&findings, &result["cost_usd"]),
        );
        let skipped_text = fleet_skipped_text(result);
        dom::text("fleet-skipped", &skipped_text);
        dom::hidden("fleet-skipped", skipped_text.is_empty());
        dom::keyed_render_or_empty(
            "fleet-findings",
            &cards,
            |entry| {
                let idx = entry["idx"].as_u64().unwrap_or(0) as usize;
                build_fleet_finding_card(&entry["item"], idx).key
            },
            |entry| {
                let idx = entry["idx"].as_u64().unwrap_or(0) as usize;
                let card = build_fleet_finding_card(&entry["item"], idx);
                render_outcome_finding_card(&card)
            },
            "p",
            "fleet-note",
            "No findings recorded.",
        );
        let report_text = result["report"]
            .as_str()
            .or_else(|| result["preview"].as_str())
            .unwrap_or("")
            .trim();
        dom::text("fleet-report-text", report_text);
        dom::hidden("fleet-report", report_text.is_empty());
        dom::hidden(
            "fleet-decision-actions",
            session["status"] != "awaiting_approval",
        );
        dom::hidden(
            "fleet-resume-actions",
            !["interrupted", "failed", "cancelled"]
                .contains(&session["status"].as_str().unwrap_or("")),
        );
        dom::hidden("fleet-download", session["status"] != "completed");
        dom::attr(
            "fleet-download",
            "href",
            &format!("/api/sessions/{}/report", dom::enc(&m::text(session, "id"))),
        );
    }
    let clash = session["clash"].is_object();
    dom::hidden("clash-outcome", !clash || session["clash_result"].is_null());
    if clash {
        let r = &session["clash_result"];
        let items = if r["items"].is_array() {
            m::array(&r["items"])
        } else {
            m::array(&r["findings"])
        };
        let turns = m::array(&r["turns"]);
        dom::text(
            "clash-outcome-summary",
            &clash_outcome_summary(r, &items, &turns),
        );
        let verdict_note = m::text(r, "verdict_note");
        dom::text("clash-verdict-note", verdict_note.trim());
        dom::hidden("clash-verdict-note", verdict_note.trim().is_empty());
        let item_cards = items
            .iter()
            .enumerate()
            .map(|(idx, item)| json!({"idx": idx, "item": item}))
            .collect::<Vec<_>>();
        dom::keyed_render_or_empty(
            "clash-items",
            &item_cards,
            |entry| {
                let idx = entry["idx"].as_u64().unwrap_or(0) as usize;
                build_clash_item_card(&entry["item"], idx).key
            },
            |entry| {
                let idx = entry["idx"].as_u64().unwrap_or(0) as usize;
                let card = build_clash_item_card(&entry["item"], idx);
                render_outcome_finding_card(&card)
            },
            "p",
            "fleet-note",
            "No clash items recorded.",
        );
        let turn_cards = turns
            .iter()
            .enumerate()
            .map(|(idx, turn)| json!({"idx": idx, "turn": turn}))
            .collect::<Vec<_>>();
        dom::keyed_render_or_empty(
            "clash-turn-list",
            &turn_cards,
            |entry| {
                let idx = entry["idx"].as_u64().unwrap_or(0) as usize;
                build_clash_turn_card(&entry["turn"], idx).key
            },
            |entry| {
                let idx = entry["idx"].as_u64().unwrap_or(0) as usize;
                let card = build_clash_turn_card(&entry["turn"], idx);
                render_clash_turn_card(&card)
            },
            "p",
            "fleet-note",
            "No turns recorded.",
        );
        let report = m::text(r, "report");
        dom::text("clash-report-text", report.trim());
        dom::hidden("clash-report", report.trim().is_empty());
    }
    if session["merge"].is_object() {
        dom::hidden("merge-context", false);
        dom::text(
            "merge-context-title",
            &format!(
                "Merged from {} chats",
                m::array(&session["merge"]["sources"]).len()
            ),
        );
        dom::rows(
            "merge-context-sources",
            &m::array(&session["merge"]["sources"]),
        );
    } else {
        dom::hidden("merge-context", m::data("merge-draft").is_null());
    }
}
