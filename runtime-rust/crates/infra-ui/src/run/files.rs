pub(crate) mod files_map {
    use crate::{dom, model as m};
    use serde_json::{json, Value as V};
    use std::collections::BTreeMap;
    use wasm_bindgen::JsCast;

    pub(crate) const OUTSIDE: &str = ":outside";

    pub(crate) const FILTERS: [(&str, &str); 5] = [
        ("all", "All"),
        ("edited", "Edited"),
        ("searched", "Searched"),
        ("failed", "Failed"),
        ("outside", "Outside"),
    ];

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub(crate) struct FileTouchEntry {
        pub path: String,
        pub service_id: Option<String>,
        pub service_folder: Option<String>,
        pub agents: Vec<String>,
        pub reads: u64,
        pub edits: u64,
        pub created: bool,
        pub deleted: bool,
        pub failed: bool,
        pub outside: bool,
        pub helper: bool,
        pub main: bool,
        pub now: bool,
        pub order: usize,
        pub first_step: u64,
        pub last_step: u64,
        pub searched_patterns: Vec<String>,
        pub event_id: V,
        pub timeline_agent: Option<String>,
    }

    impl FileTouchEntry {
        pub(crate) fn key(&self) -> String {
            if self.outside {
                return format!("{OUTSIDE}:{}", self.path);
            }
            match self.service_id.as_deref().filter(|s| !s.is_empty()) {
                Some(sid) => format!("{sid}:{}", self.path),
                None => self.path.clone(),
            }
        }

        pub(crate) fn card_heading(&self) -> String {
            if self.outside {
                return format!("{} (outside the project, name only)", self.path);
            }
            match self.service_id.as_deref().filter(|s| !s.is_empty()) {
                Some(sid) => format!("{sid} · {}", self.path),
                None => self.path.clone(),
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct FilterPill {
        pub id: String,
        pub label: String,
        pub pressed: bool,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct FileChip {
        pub key: String,
        pub path: String,
        pub service_id: Option<String>,
        pub name: String,
        pub mark: String,
        pub times: String,
        pub agent_badge: String,
        pub agents: Vec<String>,
        pub edited: bool,
        pub failed: bool,
        pub created: bool,
        pub deleted: bool,
        pub helper: bool,
        pub dim: bool,
        pub now: bool,
        pub label: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct BlockGroup {
        pub name: String,
        pub label: String,
        pub searched: bool,
        pub files: Vec<FileChip>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct DistrictGroup {
        pub name: String,
        pub label: String,
        pub service_id: Option<String>,
        pub blocks: Vec<BlockGroup>,
    }

    #[cfg(test)]
    #[derive(Clone, Debug)]
    pub(crate) struct FilesSurfaceOptions {
        pub group_by_service: bool,
        pub show_agent_badge: bool,
        pub chip_action: String,
        pub view_action: String,
        pub filter_action: String,
        pub reveal_action: String,
        pub inspect_graph_action: String,
        pub root_id: Option<String>,
    }

    #[cfg(test)]
    impl Default for FilesSurfaceOptions {
        fn default() -> Self {
            Self {
                group_by_service: false,
                show_agent_badge: false,
                chip_action: "run-file".into(),
                view_action: "run-view".into(),
                filter_action: "run-filter".into(),
                reveal_action: "run-reveal".into(),
                inspect_graph_action: "inspect-file-graph".into(),
                root_id: None,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct FileCardAction {
        pub label: String,
        pub action: String,
        pub payload: V,
        pub data_service: Option<String>,
        pub data_path: Option<String>,
    }

    pub(crate) fn clean_bidi(s: &str) -> String {
        crate::dom::clean_bidi(s)
    }

    pub(crate) fn tidy_path(path: &str) -> String {
        clean_bidi(path).replace('\\', "/")
    }

    pub(crate) fn escape_html(s: &str) -> String {
        crate::dom::escape_html(s)
    }

    pub(crate) fn truth(value: bool) -> &'static str {
        if value {
            "true"
        } else {
            "false"
        }
    }

    pub(crate) fn count_words(n: u64, one: &str) -> String {
        format!("{n} {one}{}", if n == 1 { "" } else { "s" })
    }

    pub(crate) fn normalize_view(view: &str) -> &'static str {
        if view == "list" {
            "list"
        } else {
            "map"
        }
    }

    pub(crate) fn place_of_path(path: &str, outside: bool) -> (String, String) {
        if outside {
            return (OUTSIDE.into(), String::new());
        }
        let mut folder: Vec<&str> = path.split('/').collect();
        folder.pop();
        if folder.is_empty() {
            return (String::new(), String::new());
        }
        let wordpress = folder.len() > 2
            && folder[0] == "wp-content"
            && ["plugins", "themes"].contains(&folder[1]);
        let depth = if wordpress { 3 } else { 1 };
        let rest = &folder[depth..];
        let mut block = rest.iter().take(3).copied().collect::<Vec<_>>().join("/");
        if rest.len() > 3 {
            block.push_str("/…");
        }
        (folder[..depth].join("/"), block)
    }

    pub(crate) fn district_label(name: &str) -> String {
        match name {
            "" => "./".into(),
            OUTSIDE => "Outside the project".into(),
            _ => format!("{name}/"),
        }
    }

    pub(crate) fn service_district_label(
        service_id: Option<&str>,
        service_folder: Option<&str>,
    ) -> String {
        let sid = service_id.filter(|s| !s.is_empty()).unwrap_or("__system__");
        match service_folder.map(str::trim).filter(|f| !f.is_empty()) {
            Some(folder) if folder != sid => format!("{sid} · {folder}"),
            _ => sid.to_owned(),
        }
    }

    pub(crate) fn service_district_place(
        service_id: Option<&str>,
        service_folder: Option<&str>,
        path: &str,
        outside: bool,
    ) -> (String, String, String) {
        if outside {
            return (OUTSIDE.into(), "Outside the project".into(), String::new());
        }
        let sid = service_id.filter(|s| !s.is_empty()).unwrap_or("__system__");
        let label = service_district_label(Some(sid), service_folder);
        let mut folder: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if !folder.is_empty() {
            folder.pop();
        }
        let mut block = folder.iter().take(3).copied().collect::<Vec<_>>().join("/");
        if folder.len() > 3 {
            block.push_str("/…");
        }
        (format!("service:{sid}"), label, block)
    }

    pub(crate) fn matches_filter(entry: &FileTouchEntry, filter: &str) -> bool {
        match filter {
            "edited" => entry.edits > 0,
            "searched" => !entry.searched_patterns.is_empty(),
            "failed" => entry.failed,
            "outside" => entry.outside,
            _ => true,
        }
    }

    pub(crate) fn offered_filters(
        entries: &[FileTouchEntry],
        chosen: &str,
    ) -> (String, Vec<FilterPill>) {
        let offered: Vec<(&str, String)> = FILTERS
            .iter()
            .filter_map(|(id, label)| {
                let n = entries
                    .iter()
                    .filter(|entry| matches_filter(entry, id))
                    .count();
                match (*id, n) {
                    ("all", _) => Some((*id, (*label).to_owned())),
                    (_, 0) => None,
                    _ => Some((*id, format!("{label} {n}"))),
                }
            })
            .collect();
        let resolved = if offered.iter().any(|(id, _)| *id == chosen) {
            chosen.to_owned()
        } else {
            "all".to_owned()
        };
        let pills = offered
            .into_iter()
            .map(|(id, label)| FilterPill {
                id: id.to_owned(),
                label,
                pressed: id == resolved,
            })
            .collect();
        (resolved, pills)
    }

    pub(crate) fn files_summary(entries: &[FileTouchEntry]) -> String {
        let edited = entries.iter().filter(|e| e.edits > 0).count();
        let failed = entries.iter().filter(|e| e.failed).count();
        let mut summary = format!(
            "{} · {edited} edited",
            count_words(entries.len() as u64, "file")
        );
        if failed > 0 {
            summary.push_str(&format!(" · {failed} failed"));
        }
        summary
    }

    pub(crate) fn short_agent_name(agent: &str) -> &str {
        agent.strip_prefix("service-").unwrap_or(agent)
    }

    pub(crate) fn format_agent_badge(agents: &[String]) -> String {
        match agents.len() {
            0 => String::new(),
            1 => short_agent_name(&agents[0]).to_owned(),
            2 => format!(
                "{}+{}",
                short_agent_name(&agents[0]),
                short_agent_name(&agents[1])
            ),
            n => format!("{}+{}", short_agent_name(&agents[0]), n - 1),
        }
    }

    pub(crate) fn chip_words_for(entry: &FileTouchEntry) -> Vec<String> {
        let mut words = vec![];
        match entry.reads {
            0 => {}
            1 => words.push("opened once".into()),
            reads => words.push(format!("opened {reads} times")),
        }
        if entry.created {
            words.push("created".into());
        }
        if entry.deleted {
            words.push("deleted".into());
        }
        match entry.edits {
            0 => {}
            1 => words.push("edit reported ok".into()),
            edits => words.push(format!("edit reported ok {edits} times")),
        }
        if entry.failed {
            words.push("a tool reported an error".into());
        }
        if entry.helper {
            words.push("by a helper agent".into());
        }
        if !entry.agents.is_empty() {
            words.push(format!("touched by {}", entry.agents.join(", ")));
        }
        if entry.outside {
            words.push("outside the project, name only".into());
        }
        words
    }

    pub(crate) fn card_facts_for(entry: &FileTouchEntry) -> Vec<String> {
        let mut facts = vec![];
        if entry.reads > 0 {
            facts.push(format!("Opened ×{}", entry.reads));
        }
        if !entry.searched_patterns.is_empty() {
            let last = &entry.searched_patterns[entry.searched_patterns.len().saturating_sub(2)..];
            facts.push(format!("In a folder searched for {}", last.join(", ")));
        }
        if entry.created {
            facts.push("Added (file_change)".into());
        }
        if entry.deleted {
            facts.push("Deleted (file_change)".into());
        }
        match entry.edits {
            0 => {}
            1 => facts.push("Edit reported ok".into()),
            edits => facts.push(format!("Edit reported ok ×{edits}")),
        }
        if entry.failed {
            facts.push("A tool reported an error".into());
        }
        if entry.helper {
            facts.push("by a helper agent".into());
        }
        facts
    }

    pub(crate) fn group_districts(
        entries: &[FileTouchEntry],
        filter: &str,
        running: bool,
        group_by_service: bool,
        show_agent_badge: bool,
    ) -> Vec<DistrictGroup> {
        let mut sorted: Vec<&FileTouchEntry> = entries.iter().collect();
        sorted.sort_by_key(|e| e.order);

        type RawBlock = (String, bool, Vec<FileChip>);
        type RawDistrict = (String, String, Option<String>, Vec<RawBlock>);
        let mut districts: Vec<RawDistrict> = vec![];

        for entry in sorted {
            let (district_key, d_label, block_name, svc_id) = if group_by_service {
                let (dk, dl, blk) = service_district_place(
                    entry.service_id.as_deref(),
                    entry.service_folder.as_deref(),
                    &entry.path,
                    entry.outside,
                );
                let svc = if entry.outside {
                    None
                } else {
                    Some(
                        entry
                            .service_id
                            .clone()
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "__system__".into()),
                    )
                };
                (dk, dl, blk, svc)
            } else {
                let (dk, blk) = place_of_path(&entry.path, entry.outside);
                let dl = district_label(&dk);
                (dk, dl, blk, entry.service_id.clone())
            };

            let now = running && entry.now;
            let mut label_parts = vec![match entry.service_id.as_deref() {
                Some(sid) if group_by_service && !sid.is_empty() && !entry.outside => {
                    format!("{sid}:{}", entry.path)
                }
                _ => entry.path.clone(),
            }];
            label_parts.extend(chip_words_for(entry));
            if now {
                label_parts.push("in use now".into());
            }

            let mark = format!(
                "{}{}",
                if entry.created { "+" } else { "" },
                if entry.failed { "!" } else { "" }
            );
            let chip = FileChip {
                key: entry.key(),
                path: entry.path.clone(),
                service_id: entry.service_id.clone(),
                name: entry
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&entry.path)
                    .to_owned(),
                mark,
                times: if entry.reads > 1 {
                    format!("×{}", entry.reads)
                } else {
                    String::new()
                },
                agent_badge: if show_agent_badge {
                    format_agent_badge(&entry.agents)
                } else {
                    String::new()
                },
                agents: entry.agents.clone(),
                edited: entry.edits > 0,
                failed: entry.failed,
                created: entry.created,
                deleted: entry.deleted,
                helper: entry.helper && !entry.main,
                dim: !matches_filter(entry, filter),
                now,
                label: label_parts.join(", "),
            };

            let d_idx = match districts
                .iter()
                .position(|(name, _, _, _)| *name == district_key)
            {
                Some(idx) => idx,
                None => {
                    districts.push((district_key, d_label, svc_id, vec![]));
                    districts.len() - 1
                }
            };
            let searched = !entry.searched_patterns.is_empty();
            let blocks = &mut districts[d_idx].3;
            match blocks.iter().position(|(name, _, _)| *name == block_name) {
                Some(b_idx) => {
                    blocks[b_idx].1 |= searched;
                    blocks[b_idx].2.push(chip);
                }
                None => blocks.push((block_name, searched, vec![chip])),
            }
        }

        districts
            .into_iter()
            .map(|(name, d_label, service_id, blocks)| {
                let blocks = blocks
                    .into_iter()
                    .map(|(block, searched, files)| {
                        let folder = if group_by_service {
                            match (name.as_str(), block.as_str()) {
                                (OUTSIDE, _) => "Outside the project".to_owned(),
                                (_, "") => d_label.clone(),
                                (_, blk) => format!("{d_label}/{blk}"),
                            }
                        } else {
                            match (name.as_str(), block.as_str()) {
                                (OUTSIDE, _) => "Outside the project".to_owned(),
                                ("", _) => "Project root".to_owned(),
                                (district, "") => district.to_owned(),
                                (district, blk) => format!("{district}/{blk}"),
                            }
                        };
                        let edited = files.iter().filter(|c| c.edited).count();
                        let mut label =
                            format!("{folder}, {}", count_words(files.len() as u64, "file"));
                        if edited > 0 {
                            label.push_str(&format!(", {edited} edited"));
                        }
                        BlockGroup {
                            name: block,
                            label,
                            searched,
                            files,
                        }
                    })
                    .collect();
                DistrictGroup {
                    name,
                    label: d_label,
                    service_id,
                    blocks,
                }
            })
            .collect()
    }

    pub(crate) fn group_list_folders(
        entries: &[FileTouchEntry],
        group_by_service: bool,
    ) -> Vec<(String, Vec<FileTouchEntry>)> {
        let mut folders: BTreeMap<String, Vec<FileTouchEntry>> = BTreeMap::new();
        for entry in entries {
            let folder_name = if group_by_service {
                if entry.outside {
                    "Outside project".to_owned()
                } else {
                    service_district_label(
                        entry.service_id.as_deref(),
                        entry.service_folder.as_deref(),
                    )
                }
            } else {
                let (district, _) = place_of_path(&entry.path, entry.outside);
                match district.as_str() {
                    OUTSIDE => "Outside project".to_owned(),
                    "" => "Project root".to_owned(),
                    _ => district,
                }
            };
            folders.entry(folder_name).or_default().push(entry.clone());
        }
        folders
            .into_iter()
            .map(|(name, mut files)| {
                files.sort_by(|a, b| a.path.cmp(&b.path));
                (name, files)
            })
            .collect()
    }

    pub(crate) fn place_popover(
        chip: (f64, f64, f64),
        card: (f64, f64),
        window: (f64, f64),
    ) -> (f64, f64) {
        let (left, top, bottom) = chip;
        let x = (left - 8.).min(window.0 - card.0 - 8.).max(8.);
        let y = if bottom + 8. + card.1 > window.1 - 8. {
            (top - card.1 - 8.).max(8.)
        } else {
            bottom + 8.
        };
        (x, y)
    }

    pub(crate) fn splice_draft(value: &str, start: u32, end: u32, text: &str) -> (String, u32) {
        let at = |units: u32| {
            let mut seen = 0;
            for (index, c) in value.char_indices() {
                if seen >= units {
                    return index;
                }
                seen += c.len_utf16() as u32;
            }
            value.len()
        };
        let (from, to) = (at(start), at(end.max(start)));
        let (before, after) = (&value[..from], &value[to..]);
        let lead = if before.is_empty() || before.ends_with(char::is_whitespace) {
            ""
        } else {
            " "
        };
        let trail = if after.is_empty() || after.starts_with(char::is_whitespace) {
            ""
        } else {
            " "
        };
        let caret = before.encode_utf16().count() + lead.len() + text.encode_utf16().count();
        (format!("{before}{lead}{text}{trail}{after}"), caret as u32)
    }

    #[cfg(test)]
    pub(crate) fn render_files_surface(
        entries: &[FileTouchEntry],
        view: &str,
        filter: &str,
        running: bool,
        opts: &FilesSurfaceOptions,
    ) -> String {
        let view = normalize_view(view);
        let list = view == "list";
        let (resolved_filter, pills) = offered_filters(entries, filter);
        let summary = escape_html(&files_summary(entries));
        let root_payload = opts
            .root_id
            .as_deref()
            .map(|r| escape_html(&json!({"root": r, "view": "map"}).to_string()))
            .unwrap_or_default();
        let list_payload = opts
            .root_id
            .as_deref()
            .map(|r| escape_html(&json!({"root": r, "view": "list"}).to_string()))
            .unwrap_or_default();

        let mut html = String::new();
        html.push_str(&format!(
            "<div class=\"run-files-head\"><p class=\"run-files-summary\">{summary}</p><div class=\"run-seg\" role=\"group\" aria-label=\"File view\"><button type=\"button\" data-action=\"{}\" data-payload=\"{root_payload}\" aria-pressed=\"{}\">Map</button><button type=\"button\" data-action=\"{}\" data-payload=\"{list_payload}\" aria-pressed=\"{}\">List</button></div></div>",
            escape_html(&opts.view_action),
            truth(!list),
            escape_html(&opts.view_action),
            truth(list)
        ));

        if entries.is_empty() {
            html.push_str("<p class=\"run-empty\">Files the agent opens appear here.</p>");
            return html;
        }

        if !list {
            html.push_str("<div class=\"run-filters\" role=\"group\" aria-label=\"Show files\">");
            for pill in &pills {
                let payload = match opts.root_id.as_deref() {
                    Some(r) => json!({"root": r, "filter": pill.id}).to_string(),
                    None => json!(pill.id).to_string(),
                };
                html.push_str(&format!(
                    "<button type=\"button\" class=\"run-filter\" data-action=\"{}\" data-filter=\"{}\" data-payload=\"{}\" aria-pressed=\"{}\">{}</button>",
                    escape_html(&opts.filter_action),
                    escape_html(&pill.id),
                    escape_html(&payload),
                    truth(pill.pressed),
                    escape_html(&pill.label),
                ));
            }
            html.push_str("</div>");

            let districts = group_districts(
                entries,
                &resolved_filter,
                running,
                opts.group_by_service,
                opts.show_agent_badge,
            );
            html.push_str("<div class=\"run-map\" aria-live=\"off\"><div class=\"run-map-main\">");
            for district in &districts {
                html.push_str(&format!(
                    "<div class=\"run-district\"><div class=\"run-district-label\"><bdi dir=\"ltr\">{}</bdi></div><div class=\"run-blocks\">",
                    escape_html(&district.label)
                ));
                for block in &district.blocks {
                    html.push_str(&format!(
                        "<div class=\"run-block\" role=\"group\" aria-label=\"{}\" data-searched=\"{}\">",
                        escape_html(&block.label),
                        truth(block.searched),
                    ));
                    if !block.name.is_empty() {
                        html.push_str(&format!(
                            "<div class=\"run-block-head\"><bdi class=\"run-block-name\" dir=\"ltr\">{}</bdi></div>",
                            escape_html(&block.name)
                        ));
                    }
                    html.push_str("<div class=\"run-files\">");
                    for chip in &block.files {
                        let payload = match opts.root_id.as_deref() {
                            Some(r) => json!({"root": r, "key": chip.key}).to_string(),
                            None => json!(chip.path).to_string(),
                        };
                        let now_attr = if chip.now { " data-now=\"true\"" } else { "" };
                        let svc_attr = chip
                            .service_id
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .map(|s| format!(" data-service=\"{}\"", escape_html(s)))
                            .unwrap_or_default();
                        html.push_str(&format!(
                            "<button type=\"button\" class=\"run-file\" data-action=\"{}\" data-payload=\"{}\" data-focus-key=\"{}\" data-path=\"{}\"{svc_attr} title=\"{}\" aria-label=\"{}\" data-edited=\"{}\" data-failed=\"{}\" data-created=\"{}\" data-deleted=\"{}\" data-helper=\"{}\" data-dim=\"{}\"{now_attr}><span class=\"run-file-mark\" aria-hidden=\"true\">{}</span><bdi class=\"run-file-name\" dir=\"ltr\" aria-hidden=\"true\">{}</bdi><span class=\"run-file-count\" aria-hidden=\"true\">{}</span>",
                            escape_html(&opts.chip_action),
                            escape_html(&payload),
                            escape_html(&chip.key),
                            escape_html(&chip.path),
                            escape_html(&chip.path),
                            escape_html(&chip.label),
                            truth(chip.edited),
                            truth(chip.failed),
                            truth(chip.created),
                            truth(chip.deleted),
                            truth(chip.helper),
                            truth(chip.dim),
                            escape_html(&chip.mark),
                            escape_html(&chip.name),
                            escape_html(&chip.times),
                        ));
                        if !chip.agent_badge.is_empty() {
                            html.push_str(&format!(
                                "<span class=\"run-file-count run-file-agent\" aria-hidden=\"true\">{}</span>",
                                escape_html(&chip.agent_badge)
                            ));
                        }
                        html.push_str("</button>");
                    }
                    html.push_str("</div></div>");
                }
                html.push_str("</div></div>");
            }
            html.push_str("</div></div>");
            html.push_str("<p class=\"run-legend\" aria-hidden=\"true\"><span class=\"run-file\"><bdi class=\"run-file-name\">opened</bdi></span><span class=\"run-file\" data-edited=\"true\"><bdi class=\"run-file-name\">edited</bdi></span><span class=\"run-file\" data-edited=\"true\" data-created=\"true\"><span class=\"run-file-mark\">+</span><bdi class=\"run-file-name\">created</bdi></span><span class=\"run-file\" data-failed=\"true\"><span class=\"run-file-mark\">!</span><bdi class=\"run-file-name\">failed</bdi></span><span class=\"run-file\" data-deleted=\"true\" data-edited=\"true\"><bdi class=\"run-file-name\">deleted</bdi></span><span class=\"run-file\"><bdi class=\"run-file-name\">re-opened</bdi><span class=\"run-file-count\">×N</span></span><span class=\"run-file\" data-helper=\"true\"><bdi class=\"run-file-name\">helper</bdi></span><span class=\"run-file\" data-now=\"true\"><bdi class=\"run-file-name\">now</bdi></span><span>⌕ searched folder</span></p>");
        } else {
            let folders = group_list_folders(entries, opts.group_by_service);
            html.push_str(
                "<div class=\"run-tree\" role=\"tree\" aria-label=\"Files named in tool calls\">",
            );
            for (folder_name, files) in &folders {
                html.push_str(&format!(
                    "<details class=\"run-tree-folder\" open data-restore-key=\"{}\"><summary>{} · {}</summary>",
                    escape_html(folder_name),
                    escape_html(folder_name),
                    files.len()
                ));
                for file in files {
                    let label = format!(
                        "{} · {} · {}",
                        file.path,
                        count_words(file.reads, "read"),
                        count_words(file.edits, "edit")
                    );
                    let reveal_payload = match opts.root_id.as_deref() {
                        Some(r) => json!({
                            "root": r,
                            "event_id": file.event_id,
                            "agent": file.timeline_agent,
                            "path": file.path,
                            "service": file.service_id,
                        })
                        .to_string(),
                        None => file.event_id.to_string(),
                    };
                    html.push_str(&format!(
                        "<div class=\"run-row-actions\"><button type=\"button\" class=\"button\" data-action=\"{}\" data-payload=\"{}\" data-focus-key=\"{}\">{}</button>",
                        escape_html(&opts.reveal_action),
                        escape_html(&reveal_payload),
                        escape_html(&file.key()),
                        escape_html(&label),
                    ));
                    if !file.outside && !file.path.is_empty() {
                        let inspect_payload = match file.service_id.as_deref() {
                            Some(sid) if !sid.is_empty() => {
                                json!({"service": sid, "path": file.path}).to_string()
                            }
                            _ => json!({"path": file.path}).to_string(),
                        };
                        let svc_attr = file
                            .service_id
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .map(|s| format!(" data-service=\"{}\"", escape_html(s)))
                            .unwrap_or_default();
                        html.push_str(&format!(
                            "<button type=\"button\" class=\"button compact\" data-action=\"{}\" data-payload=\"{}\" data-path=\"{}\"{svc_attr}>Map</button>",
                            escape_html(&opts.inspect_graph_action),
                            escape_html(&inspect_payload),
                            escape_html(&file.path),
                        ));
                    }
                    html.push_str("</div>");
                }
                html.push_str("</details>");
            }
            html.push_str("</div>");
        }
        html
    }

    #[cfg(test)]
    pub(crate) fn render_file_card_html(
        entry: &FileTouchEntry,
        actions: &[FileCardAction],
    ) -> String {
        let heading = escape_html(&entry.card_heading());
        let facts = escape_html(&card_facts_for(entry).join(" · "));
        let meta = if entry.first_step > 0 || entry.last_step > 0 {
            format!(
                "First step {} · last step {}",
                entry.first_step, entry.last_step
            )
        } else {
            "Recorded in dispatch evidence".into()
        };

        let mut html = format!("<p class=\"run-card-path\"><bdi dir=\"ltr\">{heading}</bdi></p>");
        if !entry.agents.is_empty() {
            html.push_str(&format!(
                "<p class=\"run-card-facts\">Touched by {}</p>",
                escape_html(&entry.agents.join(", "))
            ));
        }
        if !facts.is_empty() {
            html.push_str(&format!("<p class=\"run-card-facts\">{facts}</p>"));
        }
        html.push_str(&format!(
            "<p class=\"run-card-meta\">{}</p><div class=\"run-card-actions\">",
            escape_html(&meta)
        ));
        for act in actions {
            let svc_attr = act
                .data_service
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| format!(" data-service=\"{}\"", escape_html(s)))
                .unwrap_or_default();
            let path_attr = act
                .data_path
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| format!(" data-path=\"{}\"", escape_html(s)))
                .unwrap_or_default();
            html.push_str(&format!(
                "<button type=\"button\" class=\"button compact\" data-action=\"{}\" data-payload=\"{}\"{svc_attr}{path_attr}>{}</button>",
                escape_html(&act.action),
                escape_html(&act.payload.to_string()),
                escape_html(&act.label),
            ));
        }
        html.push_str("</div>");
        html
    }

    pub(crate) fn bdi_el(class: &str, text: &str) -> web_sys::Element {
        let node = dom::el("bdi", class, text);
        let _ = node.set_attribute("dir", "ltr");
        node
    }

    pub(crate) fn hushed_el(node: web_sys::Element) -> web_sys::Element {
        let _ = node.set_attribute("aria-hidden", "true");
        node
    }

    pub(crate) fn focus_element_no_scroll(node: &web_sys::Element) {
        let options = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&options, &"preventScroll".into(), &true.into());
        if let Ok(method) = js_sys::Reflect::get(node, &"focus".into()) {
            if let Some(method) = method.dyn_ref::<js_sys::Function>() {
                let _ = method.call1(node, &options);
            }
        }
    }

    pub(crate) fn chip_element(
        chip: &FileChip,
        action: &str,
        payload: V,
        now_value: &str,
    ) -> web_sys::Element {
        let node = dom::el("button", "run-file", "");
        for (key, value) in [
            ("type", "button".to_owned()),
            ("data-action", action.to_owned()),
            ("data-payload", payload.to_string()),
            ("data-focus-key", chip.key.clone()),
            ("data-path", chip.path.clone()),
            ("title", chip.path.clone()),
            ("aria-label", chip.label.clone()),
        ] {
            let _ = node.set_attribute(key, &value);
        }
        if let Some(sid) = chip.service_id.as_deref().filter(|s| !s.is_empty()) {
            let _ = node.set_attribute("data-service", sid);
        }
        for (flag, val) in [
            ("edited", chip.edited),
            ("failed", chip.failed),
            ("created", chip.created),
            ("deleted", chip.deleted),
            ("helper", chip.helper),
            ("dim", chip.dim),
        ] {
            let _ = node.set_attribute(&format!("data-{flag}"), truth(val));
        }
        if chip.now {
            let _ = node.set_attribute("data-now", now_value);
        }
        dom::add(
            &node,
            &hushed_el(dom::el("span", "run-file-mark", &chip.mark)),
        );
        dom::add(&node, &hushed_el(bdi_el("run-file-name", &chip.name)));
        dom::add(
            &node,
            &hushed_el(dom::el("span", "run-file-count", &chip.times)),
        );
        if !chip.agent_badge.is_empty() {
            dom::add(
                &node,
                &hushed_el(dom::el(
                    "span",
                    "run-file-count run-file-agent",
                    &chip.agent_badge,
                )),
            );
        }
        node
    }

    pub(crate) fn district_element(
        district: &DistrictGroup,
        chip_action: &str,
        root_id: Option<&str>,
        now_value: &str,
    ) -> web_sys::Element {
        let wrap = dom::el("div", "run-district", "");
        if let Some(sid) = district.service_id.as_deref().filter(|s| !s.is_empty()) {
            let _ = wrap.set_attribute("data-service", sid);
        }
        let label = dom::el("div", "run-district-label", "");
        dom::add(&label, &bdi_el("", &district.label));
        dom::add(&wrap, &label);
        let blocks = dom::el("div", "run-blocks", "");
        for block in &district.blocks {
            let node = dom::el("div", "run-block", "");
            let _ = node.set_attribute("role", "group");
            let _ = node.set_attribute("aria-label", &block.label);
            let _ = node.set_attribute("data-searched", truth(block.searched));
            if !block.name.is_empty() {
                let head = dom::el("div", "run-block-head", "");
                dom::add(&head, &bdi_el("run-block-name", &block.name));
                dom::add(&node, &head);
            }
            let files = dom::el("div", "run-files", "");
            for chip in &block.files {
                let payload = match root_id {
                    Some(r) => json!({"root": r, "key": chip.key}),
                    None => json!(chip.path),
                };
                dom::add(&files, &chip_element(chip, chip_action, payload, now_value));
            }
            dom::add(&node, &files);
            dom::add(&blocks, &node);
        }
        dom::add(&wrap, &blocks);
        wrap
    }

    pub(crate) fn legend_element() -> web_sys::Element {
        let legend = dom::el("p", "run-legend", "");
        let _ = legend.set_attribute("aria-hidden", "true");
        let add_sample = |attrs: &[(&str, &str)], mark: &str, name: &str, count: &str| {
            let chip = dom::el("span", "run-file", "");
            for (k, v) in attrs {
                let _ = chip.set_attribute(k, v);
            }
            if !mark.is_empty() {
                dom::add(&chip, &dom::el("span", "run-file-mark", mark));
            }
            dom::add(&chip, &bdi_el("run-file-name", name));
            if !count.is_empty() {
                dom::add(&chip, &dom::el("span", "run-file-count", count));
            }
            dom::add(&legend, &chip);
        };
        add_sample(&[], "", "opened", "");
        add_sample(&[("data-edited", "true")], "", "edited", "");
        add_sample(
            &[("data-edited", "true"), ("data-created", "true")],
            "+",
            "created",
            "",
        );
        add_sample(&[("data-failed", "true")], "!", "failed", "");
        add_sample(
            &[("data-deleted", "true"), ("data-edited", "true")],
            "",
            "deleted",
            "",
        );
        add_sample(&[], "", "re-opened", "×N");
        add_sample(&[("data-helper", "true")], "", "helper", "");
        add_sample(&[("data-now", "true")], "", "now", "");
        dom::add(&legend, &dom::el("span", "", "⌕ searched folder"));
        legend
    }

    pub(crate) fn position_card_element(card_id: &str, chip: &web_sys::Element) {
        let Some(card) = dom::get(card_id).and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        else {
            return;
        };
        let rect = chip.get_bounding_client_rect();
        let window = dom::window();
        let side = |value: Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>| {
            value.ok().and_then(|value| value.as_f64()).unwrap_or(0.)
        };
        let (x, y) = place_popover(
            (rect.left(), rect.top(), rect.bottom()),
            (card.offset_width() as f64, card.offset_height() as f64),
            (side(window.inner_width()), side(window.inner_height())),
        );
        let _ = card.style().set_property("left", &format!("{x}px"));
        let _ = card.style().set_property("top", &format!("{y}px"));
    }

    pub(crate) type EventCallback = wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>;

    pub(crate) fn fill_card_element(
        card_id: &str,
        owner: &str,
        entry: &FileTouchEntry,
        actions: &[FileCardAction],
        dismiss_on_action: Option<(&str, &EventCallback)>,
    ) {
        let Some(card) = dom::get(card_id) else {
            return;
        };
        let heading = entry.card_heading();
        let facts = card_facts_for(entry).join(" · ");
        let agents_line = if entry.agents.is_empty() {
            String::new()
        } else {
            format!("Touched by {}", entry.agents.join(", "))
        };
        let meta = if entry.first_step > 0 || entry.last_step > 0 {
            format!(
                "First step {} · last step {}",
                entry.first_step, entry.last_step
            )
        } else {
            "Recorded in dispatch evidence".into()
        };
        let signature = format!(
            "{owner}|{}|{agents_line}|{facts}|{meta}|{}",
            entry.key(),
            actions.len()
        );
        if card.get_attribute("data-signature").as_deref() == Some(signature.as_str()) {
            return;
        }
        let focused = dom::document()
            .active_element()
            .filter(|node| card.contains(Some(node)))
            .and_then(|node| node.get_attribute("data-action"));

        card.set_text_content(None);
        let _ = card.set_attribute("data-owner", owner);
        let _ = card.set_attribute("data-signature", &signature);
        let _ = card.set_attribute("aria-label", &format!("File details: {heading}"));

        let line = dom::el("p", "run-card-path", "");
        dom::add(&line, &bdi_el("", &heading));
        dom::add(&card, &line);

        if !agents_line.is_empty() {
            dom::add(&card, &dom::el("p", "run-card-facts", &agents_line));
        }
        if !facts.is_empty() {
            dom::add(&card, &dom::el("p", "run-card-facts", &facts));
        }
        dom::add(&card, &dom::el("p", "run-card-meta", &meta));

        let actions_wrap = dom::el("div", "run-card-actions", "");
        for act in actions {
            let button = dom::button(&act.label, &act.action, act.payload.clone());
            button.set_class_name("button compact");
            if let Some(sid) = act.data_service.as_deref().filter(|s| !s.is_empty()) {
                let _ = button.set_attribute("data-service", sid);
            }
            if let Some(p) = act.data_path.as_deref().filter(|s| !s.is_empty()) {
                let _ = button.set_attribute("data-path", p);
            }
            if let Some((watch_action, cb)) = dismiss_on_action {
                if act.action == watch_action {
                    let _ = button
                        .add_event_listener_with_callback("click", cb.as_ref().unchecked_ref());
                }
            }
            dom::add(&actions_wrap, &button);
        }
        dom::add(&card, &actions_wrap);

        if let Some(action) = focused {
            let selector = format!("#{card_id} [data-action=\"{action}\"]");
            if let Ok(Some(node)) = dom::document().query_selector(&selector) {
                focus_element_no_scroll(&node);
            }
        }
        let _ = m::string(&json!(owner));
    }
}
