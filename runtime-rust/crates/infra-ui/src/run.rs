use crate::{dom, model as m, session};
use serde_json::{Value as V, json};
use std::collections::BTreeMap;
use wasm_bindgen::JsCast;

#[allow(dead_code)]
pub(crate) mod files_map {
    use crate::{dom, model as m};
    use serde_json::{Value as V, json};
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
        s.chars()
            .filter(|c| {
                !matches!(
                    *c,
                    '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}'
                )
            })
            .collect()
    }

    pub(crate) fn tidy_path(path: &str) -> String {
        clean_bidi(path).replace('\\', "/")
    }

    pub(crate) fn escape_html(s: &str) -> String {
        let cleaned = clean_bidi(s);
        let mut out = String::with_capacity(cleaned.len());
        for ch in cleaned.chars() {
            match ch {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&#39;"),
                _ => out.push(ch),
            }
        }
        out
    }

    pub(crate) fn truth(value: bool) -> &'static str {
        if value { "true" } else { "false" }
    }

    pub(crate) fn count_words(n: u64, one: &str) -> String {
        format!("{n} {one}{}", if n == 1 { "" } else { "s" })
    }

    pub(crate) fn normalize_view(view: &str) -> &'static str {
        if view == "list" { "list" } else { "map" }
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

#[cfg(test)]
use files_map::place_popover as place;
use files_map::{
    FileCardAction, FileTouchEntry, OUTSIDE, bdi_el as bdi, count_words as count, district_label,
    focus_element_no_scroll as focus_node, hushed_el as hushed, place_of_path,
    splice_draft as spliced, truth,
};

thread_local! {
    static DISMISS_ON_GRAPH_CLICK: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)> =
        wasm_bindgen::closure::Closure::new(|_: web_sys::Event| {
            hide_card(false);
        });
}

fn clean(s: &str) -> String {
    files_map::clean_bidi(s)
}

pub fn reset() {
    dom::cancel_timer("run-clock");
    for key in [
        "run",
        "agents",
        "run-file-filter",
        "run-command-filter",
        "run-before",
        "run-after",
        "run-idle",
    ] {
        m::take(key);
    }
    hide_card(false);
    dom::clear("events");
    dom::clear("run-tree");
    dom::clear("run-pane-plan");
    dom::clear("run-pane-commands");
    dom::clear("run-pane-memory");
    dom::clear("run-pane-omnigraph");
    // The away summary belongs to the session being left.
    away(false);
    // The session being left closes its strip now, not when the next one answers.
    render();
}

fn initial() -> V {
    json!({"launch":"","calls":{},"open":[],"files":{},"commands":[],"plan":[],"searches":[],"steps":0,"messages":0,"finished":false,"outcome":null,"memory":null,"omnigraph":null,"kinds":[],"started":null})
}

fn tool_name(v: &V) -> String {
    let name = v["tool"].as_str().unwrap_or_else(|| {
        v["text"]
            .as_str()
            .unwrap_or("")
            .split(": ")
            .next()
            .unwrap_or("Tool")
    });
    let name = name.strip_suffix("ToolCall").unwrap_or(name);
    match name {
        "command_execution" => "Shell",
        "file_change" => "Edit",
        "mcp_tool_call" => "MCP",
        "web_search" => "Web search",
        _ => name,
    }
    .into()
}

fn starts(v: &V) -> bool {
    if let Some(state) = v["state"].as_str() {
        return state == "started" || state == "in_progress";
    }
    let text = m::text(v, "text");
    let tail = text.split_once(": ").map(|(_, s)| s).unwrap_or("");
    !["completed", "failed", "declined"]
        .iter()
        .any(|p| tail.starts_with(p))
}

pub fn classify(command: &str) -> V {
    crate::classify::classify(command)
}

pub fn observe(event: V) {
    let (run, replaced) = advance(m::data("run"), &event);
    if let Some(replaced) = replaced {
        m::set("run-before", replaced);
    }
    m::set("run", run);
    activity(&event);
    conversation(&event);
    // Poll controls render the derived panels once after all fresh events.
    // Completion notifications must run before a later event can change launch.
    notify_finished();
}

/// One event into the model. Returns the newest launch and, when the event began another one,
/// the launch to remember in its place.
fn advance(mut run: V, event: &V) -> (V, Option<V>) {
    let launch = m::text(event, "launch_id");
    let mut replaced = None;
    if !run.is_object() || (!launch.is_empty() && run["launch"] != launch) {
        // A Harness check comes under a launch id of its own. The launch it replaces is kept, so
        // the turn's details outlive the check; a check is never the launch to keep.
        if run.is_object() && !is_check(&run) {
            replaced = Some(run);
        }
        run = initial();
        run["launch"] = json!(launch);
        run["started"] = event["at"].clone();
    }
    let kind = m::text(event, "kind");
    let mut kinds = m::array(&run["kinds"]);
    if !kinds.iter().any(|k| k == &kind) {
        kinds.push(json!(kind));
    }
    run["kinds"] = json!(kinds);
    match kind.as_str() {
        "tool" => {
            let name = tool_name(event);
            let key = name.to_lowercase();
            let start = starts(event);
            let call = match event["call"].as_str() {
                Some(call) => call.to_owned(),
                None => paired(&mut run, &key, start, &m::string(&event["id"])),
            };
            let target = event["path"]
                .as_str()
                .or_else(|| event["detail"].as_str())
                .unwrap_or("");
            let mut calls = run["calls"].as_object().cloned().unwrap_or_default();
            let mut item = calls.get(&call).cloned().unwrap_or(
                json!({"tool":name,"key":key,"path":event["path"],"detail":event["detail"],"event_id":event["id"],"started":event["at"]}),
            );
            item["open"] = json!(start);
            item["ok"] = json!(
                event["ok"]
                    .as_bool()
                    .unwrap_or(!m::text(event, "text").contains("failed"))
            );
            item["event_id"] = event["id"].clone();
            if start {
                run["steps"] = json!(run["steps"].as_u64().unwrap_or(0) + 1);
            }
            touch(&mut run, &mut item, &named(&key, target, event), event);
            if !start {
                settle(&mut run, &mut item, event);
            }
            calls.insert(call.clone(), item.clone());
            run["calls"] = json!(calls);
            if ["grep", "glob", "ls"].contains(&key.as_str()) && start {
                let mut searches = m::array(&run["searches"]);
                searches.push(
                    json!({"pattern":event["detail"],"scope":event["path"],"event_id":event["id"]}),
                );
                run["searches"] = json!(searches);
            }
            if ["bash", "shell"].contains(&key.as_str()) {
                let mut list = m::array(&run["commands"]);
                if let Some(existing) = list.iter_mut().find(|c| c["call"] == call) {
                    existing["running"] = json!(start);
                    existing["ok"] = item["ok"].clone();
                    existing["event_id"] = event["id"].clone();
                } else {
                    let command = event["detail"].as_str().unwrap_or_else(|| {
                        event["text"]
                            .as_str()
                            .unwrap_or("")
                            .split_once(": ")
                            .map(|(_, s)| s)
                            .unwrap_or("")
                    });
                    list.push(json!({"call":call,"command":command,"class":classify(command),"running":start,"ok":item["ok"],"event_id":event["id"],"at":event["at"]}));
                }
                run["commands"] = json!(list);
            }
        }
        "plan" => {
            run["plan"] = event["items"]
                .as_array()
                .or_else(|| event["plan"].as_array())
                .cloned()
                .map(V::Array)
                .unwrap_or_else(|| event["plan"].clone());
        }
        "text" => run["messages"] = json!(run["messages"].as_u64().unwrap_or(0) + 1),
        "usage" => run["usage"] = event.clone(),
        "memory" => run["memory"] = event.clone(),
        "omnigraph" => run["omnigraph"] = event.clone(),
        "delegation" => run["delegation"] = event.clone(),
        "status" => {
            if check_status(&m::text(event, "text")) {
                run["check"] = json!(true);
            }
            let status = event["status"]
                .as_str()
                .unwrap_or_else(|| event["text"].as_str().unwrap_or(""));
            if ["completed", "failed", "cancelled", "interrupted"].contains(&status) {
                run["finished"] = json!(true);
                run["outcome"] = json!(status);
            }
        }
        "error" => {
            run["error"] = event.clone();
            if m::text(event, "text").starts_with("Run failed (") {
                run["finished"] = json!(true);
                run["outcome"] = json!("failed");
            }
        }
        "result" => run["result"] = event.clone(),
        _ => {}
    }
    (run, replaced)
}

/// The call an event without a call id belongs to. Such events are plain labels: a start waits
/// under its tool's name, and the next completion of that tool closes the oldest one waiting.
fn paired(run: &mut V, key: &str, start: bool, event_id: &str) -> String {
    let mut waiting = run["uncalled"].as_object().cloned().unwrap_or_default();
    let mut queue = m::array(&waiting.get(key).cloned().unwrap_or(V::Null));
    let call = if start || queue.is_empty() {
        format!("anonymous-{event_id}")
    } else {
        m::string(&queue.remove(0))
    };
    if start {
        queue.push(json!(call));
    }
    waiting.insert(key.into(), json!(queue));
    run["uncalled"] = V::Object(waiting);
    call
}

/// A path as the page keys it: no bidirectional controls, forward slashes.
fn tidy(path: &str) -> String {
    files_map::tidy_path(path)
}

/// The files a tool event names and what the call does to each: the files of a change, or the
/// call's own target.
fn named(key: &str, target: &str, event: &V) -> Vec<(String, &'static str, bool)> {
    let changes = m::array(&event["changes"]);
    if !changes.is_empty() {
        return changes
            .iter()
            .map(|change| {
                let how = match change["kind"].as_str() {
                    Some("add") => "add",
                    Some("delete") => "delete",
                    _ => "edit",
                };
                (
                    tidy(&m::text(change, "path")),
                    how,
                    m::yes(&change["outside"]),
                )
            })
            .filter(|(path, _, _)| !path.is_empty())
            .collect();
    }
    let how = match key {
        "read" => "read",
        "delete" => "delete",
        "edit" | "write" | "multiedit" | "notebookedit" => "edit",
        _ => return vec![],
    };
    let path = tidy(target);
    if path.is_empty() {
        vec![]
    } else {
        vec![(path, how, m::yes(&event["outside"]))]
    }
}

/// A call reaches a file once: its start and its result name the same file, and only the first
/// of them opens it.
fn touch(run: &mut V, call: &mut V, named: &[(String, &'static str, bool)], event: &V) {
    if named.is_empty() {
        return;
    }
    let mut touched = call["touched"].as_object().cloned().unwrap_or_default();
    let mut files = run["files"].as_object().cloned().unwrap_or_default();
    let step = run["steps"].clone();
    let by = if event["parent"].as_str().is_some_and(|id| !id.is_empty()) {
        "helper"
    } else {
        "main"
    };
    for (path, how, outside) in named {
        if touched.contains_key(path) {
            continue;
        }
        touched.insert(path.clone(), json!(how));
        if !files.contains_key(path) {
            let fresh = json!({"path":path,"reads":0,"edits":0,"outside":outside,"order":files.len(),"first_step":step});
            files.insert(path.clone(), fresh);
        }
        let Some(file) = files.get_mut(path) else {
            continue;
        };
        if *how == "read" {
            file["reads"] = json!(file["reads"].as_u64().unwrap_or(0) + 1);
        }
        file[by] = json!(true);
        file["last_step"] = step.clone();
        file["event_id"] = event["id"].clone();
    }
    call["touched"] = V::Object(touched);
    run["files"] = V::Object(files);
}

/// The call's result reaches the files it touched, once: an edit counts when the tool reports it
/// done, and a failure marks the file.
fn settle(run: &mut V, call: &mut V, event: &V) {
    if m::yes(&call["settled"]) {
        return;
    }
    call["settled"] = json!(true);
    let Some(touched) = call["touched"].as_object() else {
        return;
    };
    let mut files = run["files"].as_object().cloned().unwrap_or_default();
    let ok = m::yes(&call["ok"]);
    for (path, how) in touched {
        let Some(file) = files.get_mut(path) else {
            continue;
        };
        if !ok {
            // A command that was never run reported nothing about the file.
            if event["outcome"] != "not_run" {
                file["failed"] = json!(true);
            }
            continue;
        }
        match how.as_str() {
            Some("read") => continue,
            Some("add") => file["created"] = json!(true),
            Some("delete") => file["deleted"] = json!(true),
            _ => {}
        }
        file["edits"] = json!(file["edits"].as_u64().unwrap_or(0) + 1);
        file["event_id"] = event["id"].clone();
    }
    run["files"] = V::Object(files);
}

fn conversation(v: &V) {
    let kind = m::text(v, "kind");
    let text = clean(&m::text(v, "text"));
    let id = m::string(&v["id"]);
    if ![
        "text",
        "thinking",
        "tool",
        "status",
        "error",
        "memory",
        "omnigraph",
        "delegation",
        "clash_turn",
        "clash_state",
    ]
    .contains(&kind.as_str())
    {
        return;
    }
    if kind == "text" && text.is_empty() {
        return;
    }
    let block = dom::el(
        if kind == "tool" { "details" } else { "div" },
        &format!("conversation-{kind}"),
        "",
    );
    let _ = block.set_attribute("data-event-id", &id);
    if kind == "tool" {
        dom::add(
            &block,
            &dom::el(
                "summary",
                "",
                &format!(
                    "{} · {}",
                    tool_name(v),
                    if starts(v) {
                        "Running"
                    } else if v["ok"] == false {
                        "Failed"
                    } else {
                        "Completed"
                    }
                ),
            ),
        );
        dom::add(&block, &dom::el("pre", "", &text));
    } else {
        dom::add(&block, &dom::el("p", "", &text));
    }
    if kind == "error" {
        let _ = block.set_attribute("role", "alert");
    }
    dom::append("events", &block);
}

fn native_run(run: &V) -> bool {
    !run["kinds"].as_array().into_iter().flatten().any(|kind| {
        matches!(
            kind.as_str(),
            Some("fleet_stage" | "fleet_reviewer" | "clash_turn" | "agent" | "agent_activity")
        )
    })
}

#[allow(dead_code)]
fn run_has_activity(run: &V) -> bool {
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
enum Mode {
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
fn plain(session: &V) -> bool {
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

fn kinds(run: &V) -> impl Iterator<Item = &str> {
    run["kinds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(V::as_str)
}

/// The provider has written something for this launch; until then the session is only preparing.
fn cli_started(run: &V) -> bool {
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
fn over(run: &V, idle: &V) -> bool {
    let launch = run["launch"].as_str().unwrap_or("");
    m::yes(&run["finished"])
        || idle.as_str() == Some(launch)
        || (!launch.is_empty() && kinds(run).any(|kind| kind == "user"))
}

/// The two lines a Harness check writes: its result and its end.
fn check_status(text: &str) -> bool {
    text == "Check finished."
        || text.strip_prefix("Check ").is_some_and(|rest| {
            rest.split_once(": ").is_some_and(|(word, _)| {
                !word.is_empty() && word.chars().all(|c| c.is_alphanumeric() || c == '_')
            })
        })
}

/// A Harness check runs under a launch id of its own and writes only its status lines and, when
/// it could not run, an error: it is not a turn. A launch the provider wrote to is never one.
fn is_check(run: &V) -> bool {
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
fn turn<'a>(run: &'a V, before: &'a V) -> &'a V {
    if is_check(run) { before } else { run }
}

/// A launch the Run view describes: a native one, in a session that keeps no progress of its own.
fn described(session: &V, run: &V) -> bool {
    run.is_object() && native_run(run) && !plain(session)
}

/// Whether the Run view is open, and as what. `before` is the launch the newest one replaced,
/// `after` the launch whose finished details the person opened, and `idle` the launch the page
/// last saw at rest.
fn mode(session: &V, run: &V, before: &V, after: &V, idle: &V) -> Mode {
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

const TABS: [&str; 5] = ["files", "plan", "commands", "memory", "omnigraph"];

/// The launch recorded something for this tab.
fn has(run: &V, tab: &str) -> bool {
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
fn openers(session: &V, run: &V) -> Vec<&'static str> {
    if !session.is_object() || m::active(session) || !described(session, run) {
        return vec![];
    }
    TABS.into_iter().filter(|tab| has(run, tab)).collect()
}

/// The tab the panel shows: the chosen one while the launch has something for it, else Files.
fn shown_tab<'a>(run: &V, chosen: &'a str) -> &'a str {
    if TABS.contains(&chosen) && has(run, chosen) {
        chosen
    } else {
        "files"
    }
}

fn counts(run: &V) -> String {
    format!(
        "{} · {}",
        count(run["steps"].as_u64().unwrap_or(0), "step"),
        count(run["messages"].as_u64().unwrap_or(0), "message")
    )
}

fn number(file: &V, key: &str) -> u64 {
    file[key].as_u64().unwrap_or(0)
}

/// The launch's files in the order they were first touched.
fn files_of(run: &V) -> Vec<V> {
    let mut files: Vec<V> = run["files"]
        .as_object()
        .map(|map| map.values().cloned().collect())
        .unwrap_or_default();
    files.sort_by_key(|file| number(file, "order"));
    files
}

/// Map layout: the top folder is a district (a WordPress plugin or theme is one), and up to
/// three more folders make a block.
fn place_of(file: &V) -> (String, String) {
    place_of_path(&m::text(file, "path"), m::yes(&file["outside"]))
}

/// The patterns of the searches whose scope holds the file; a search without a scope looked
/// everywhere in the project.
fn searched_in(run: &V, file: &V) -> Vec<String> {
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

fn entry_from_run_file(
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

fn entries_of_run(run: &V, running: bool) -> Vec<FileTouchEntry> {
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
fn matches(run: &V, file: &V, filter: &str) -> bool {
    let empty = std::collections::BTreeSet::new();
    files_map::matches_filter(&entry_from_run_file(run, file, &empty), filter)
}

/// The filters worth offering: All, and each kind that has a file. A choice whose kind has no
/// file any more reads as All.
fn filters(run: &V, chosen: &str) -> Vec<V> {
    let entries = entries_of_run(run, false);
    let (_, pills) = files_map::offered_filters(&entries, chosen);
    pills
        .into_iter()
        .map(|pill| json!({"id": pill.id, "label": pill.label, "pressed": pill.pressed}))
        .collect()
}

/// What a chip says after its path to someone who cannot see its marks.
#[cfg(test)]
fn chip_words(file: &V) -> Vec<String> {
    let empty = std::collections::BTreeSet::new();
    files_map::chip_words_for(&entry_from_run_file(&V::Null, file, &empty))
}

/// The facts on a file's card.
#[cfg(test)]
fn card_facts(run: &V, file: &V) -> Vec<String> {
    let empty = std::collections::BTreeSet::new();
    files_map::card_facts_for(&entry_from_run_file(run, file, &empty))
}

/// The map as data: districts, their blocks and the chips in them, in the order the files were
/// first touched. The filter dims the chips it leaves out, and while the launch runs a ring
/// marks the files of the calls that are open.
fn map(run: &V, filter: &str, running: bool) -> Vec<V> {
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
struct RunClock {
    started: Option<String>,
    running: bool,
}

fn clock_state(run: &V, session: &V) -> Option<RunClock> {
    if !run.is_object() || !native_run(run) {
        return None;
    }
    Some(RunClock {
        started: run["started"].as_str().map(str::to_owned),
        running: !m::yes(&run["finished"]) && m::active(session),
    })
}

fn update_clock() {
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

fn set_inert(id: &str, value: bool) {
    if let Some(e) = dom::get(id) {
        if value {
            let _ = e.set_attribute("inert", "");
        } else {
            let _ = e.remove_attribute("inert");
        }
    }
}

fn focus_within(ids: &[&str]) -> bool {
    let Some(focused) = dom::document().active_element() else {
        return false;
    };
    ids.iter()
        .any(|id| dom::get(id).is_some_and(|e| e.contains(Some(&focused))))
}

/// Focus without scrolling: the view clips its content while it opens, and a scrolling focus
/// would shift what it clips.
fn focus(id: &str) {
    if let Some(node) = dom::get(id) {
        focus_node(&node);
    }
}

/// The Calm/Detailed switch that is on screen: a narrow window shows only the compact chip.
fn detail_switch() -> &'static str {
    let narrow = dom::window()
        .match_media("(max-width:760px)")
        .ok()
        .flatten()
        .is_some_and(|media| media.matches());
    if narrow { "run-compact" } else { "run-calm" }
}

/// Opens or collapses the view and its panel. The markup ships both collapsed and inert, and the
/// stylesheet shows the view only as `data-open="true"` and the panel only as
/// `data-detailed="true"` on the card.
fn reveal(mode: Mode, detailed: bool) {
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
    dom::hidden("run-close", mode != Mode::After);
    dom::attr(
        "run-dot",
        "data-state",
        if mode == Mode::Now { "running" } else { "done" },
    );
    dom::attr("run-dot", "data-pulse", truth(mode == Mode::Now));
}

const TURN: &str = "run-turn";

/// The line under the conversation that opens a finished turn's details. The page keeps the
/// newest launch only, so there is one line, and it goes when the next turn starts.
fn turn_line(session: &V, run: &V) {
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
fn away(shown: bool) {
    dom::hidden("run-away", !shown);
    dom::hidden("run-chips", shown);
}

fn chip(text: &str) {
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
fn earlier(run: &V) -> V {
    if is_check(run) {
        m::data("run-before")
    } else {
        V::Null
    }
}

fn chip_node(chip: &V) -> web_sys::Element {
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

fn district_node(district: &V) -> web_sys::Element {
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
fn filter_buttons(items: &[V]) {
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

const CARD: &str = "run-file-card";

/// The file whose card is open in Sessions Run view.
fn card_path() -> Option<String> {
    m::data("run-card").as_str().map(str::to_owned)
}

fn chip_of(path: &str) -> Option<web_sys::Element> {
    dom::all("#run-map-main .run-file")
        .into_iter()
        .find(|chip| chip.get_attribute("data-focus-key").as_deref() == Some(path))
}

fn run_card_actions(file: &V) -> Vec<FileCardAction> {
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
    actions
}

/// Writes the card for a file. An unchanged card is left alone, and a rewritten one gives the
/// focus back to the action that had it.
fn fill_card(run: &V, file: &V) {
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

fn place_card(chip: &web_sys::Element) {
    files_map::position_card_element(CARD, chip);
}

/// Opens the card of a file beside its chip; the first of its actions takes the focus.
fn show_card(run: &V, path: &str) {
    let Some(file) = run["files"].get(path) else {
        return;
    };
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
    }
}

/// Puts the card away; `restore` gives the focus back to the chip it belongs to.
fn hide_card(restore: bool) {
    let path = m::take("run-card");
    if path.is_string()
        || dom::get(CARD)
            .and_then(|c| c.get_attribute("data-owner"))
            .as_deref()
            == Some("run")
    {
        dom::hidden(CARD, true);
    }
    if restore {
        if let Some(chip) = path.as_str().and_then(chip_of) {
            focus_node(&chip);
        }
    }
}

/// The launch the strip and the panel show: the one that runs, or at rest the last turn's.
fn shown() -> V {
    let (session, run) = (m::read(|s| s.selected.clone()), m::data("run"));
    if m::active(&session) {
        run
    } else {
        let before = earlier(&run);
        turn(&run, &before).clone()
    }
}

/// The Files tab: the summary, the filters, the map of touched files and the same set as a list.
fn files_pane(run: &V, running: bool) {
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
            dom::add(
                &d,
                &dom::el(
                    "summary",
                    "",
                    &format!("{} · {}", m::text(v, "name"), files.len()),
                ),
            );
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
                dom::add(&row, &b);
                if !outside && !path.is_empty() {
                    let map_btn = dom::button("Map", "inspect-file-graph", json!({"path": path}));
                    map_btn.set_class_name("button compact");
                    let _ = map_btn.set_attribute("data-path", &path);
                    let _ = map_btn
                        .set_attribute("aria-label", &format!("Show {path} on dependency map"));
                    dom::add(&row, &map_btn);
                }
                dom::add(&d, &row);
            }
            d
        },
    );
    // An open card follows its file; a file that is gone, or a map that is, takes it along.
    if let Some(path) = card_path() {
        match run["files"].get(&path) {
            Some(file) if !list => fill_card(run, file),
            _ => hide_card(false),
        }
    }
}

/// Puts a path into the message being written, at the caret; Send stays the person's.
fn insert_draft(text: &str) {
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

/// Escape puts away the nearest thing the person opened: the file card, then the details.
#[allow(dead_code)]
pub(crate) fn escape() -> bool {
    if card_path().is_some() {
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

pub fn render() {
    let run = m::data("run");
    let session = m::read(|s| s.selected.clone());
    if m::active(&session) {
        // A turn that starts takes the strip back from an earlier turn's details.
        m::take("run-after");
    } else if run.is_object() {
        // The page has seen this launch at rest: whatever runs next is not it.
        m::set("run-idle", run["launch"].clone());
    }
    let before = earlier(&run);
    let mode = mode(
        &session,
        &run,
        &before,
        &m::data("run-after"),
        &m::data("run-idle"),
    );
    reveal(mode, m::yes(&m::data("run-detailed")));
    turn_line(&session, turn(&run, &before));
    // At rest the strip and the panel hold the last turn, ready to be opened; a Harness check
    // is not one.
    let run = if m::active(&session) {
        run
    } else {
        turn(&run, &before).clone()
    };
    if !run.is_object() || !native_run(&run) {
        dom::cancel_timer("run-clock");
        return;
    }
    let calls = run["calls"].as_object().cloned().unwrap_or_default();
    let open: Vec<_> = calls
        .values()
        .filter(|c| m::yes(&c["open"]))
        .cloned()
        .collect();
    let done = m::yes(&run["finished"]) || !m::active(&session);
    let now = if done {
        format!(
            "Run {}",
            run["outcome"]
                .as_str()
                .unwrap_or_else(|| session["status"].as_str().unwrap_or("finished"))
        )
    } else if open.len() == 1 {
        format!(
            "{} · {}",
            m::text(&open[0], "tool"),
            open[0]["path"]
                .as_str()
                .or_else(|| open[0]["detail"].as_str())
                .unwrap_or("")
        )
    } else if !open.is_empty() {
        format!("{} steps running", open.len())
    } else {
        "Model’s turn".into()
    };
    if mode == Mode::Now {
        update_clock();
    } else {
        // Only a running launch has a clock: a finished turn would show the age of its first event.
        dom::cancel_timer("run-clock");
        dom::text("run-clock", "");
    }
    dom::text("run-now-text", &now);
    dom::text("run-verb", &now);
    // The counts go into the chips' own box: the second line also holds the Calm/Detailed switch,
    // Close and the away summary, which a text written over the whole line would delete.
    let counts = counts(&run);
    chip(&counts);
    dom::text(
        "run-compact",
        &count(run["steps"].as_u64().unwrap_or(0), "step"),
    );
    dom::attr(
        "run-compact",
        "aria-label",
        &format!("Run details. {counts}"),
    );
    files_pane(&run, mode == Mode::Now);
    let plan = m::array(&run["plan"]);
    dom::text("run-badge-plan", &plan.len().to_string());
    dom::keyed_render(
        "run-pane-plan",
        &plan,
        |v| {
            v["id"]
                .as_str()
                .or_else(|| v["content"].as_str())
                .or_else(|| v["text"].as_str())
                .or_else(|| v["step"].as_str())
                .unwrap_or("")
                .into()
        },
        |p| {
            let label = format!(
                "{} · {}",
                m::text(p, "status"),
                p["text"]
                    .as_str()
                    .or_else(|| p["content"].as_str())
                    .or_else(|| p["step"].as_str())
                    .unwrap_or("")
            );
            dom::el("p", "run-plan-item", &label)
        },
    );
    for (tab, pane) in [
        ("memory", render_run_memory_pane_html(&run["memory"])),
        (
            "omnigraph",
            render_run_omnigraph_pane_html(&run["omnigraph"]),
        ),
    ] {
        if let Some(element) = dom::get(&format!("run-pane-{tab}")) {
            element.set_inner_html(&pane);
        }
    }
    let commands = m::array(&run["commands"]);
    dom::text("run-badge-commands", &commands.len().to_string());
    dom::keyed_render(
        "run-pane-commands",
        &commands,
        |v| m::text(v, "call"),
        |cmd| {
            let class = &cmd["class"];
            let glyph = if m::yes(&cmd["running"]) {
                "●"
            } else if !class["masked"].is_null() {
                "?"
            } else if m::yes(&cmd["ok"]) {
                "✓"
            } else {
                "✗"
            };
            let block = dom::details(
                &format!("{glyph} {}", m::text(cmd, "command")),
                &serde_json::to_string_pretty(cmd).unwrap_or_default(),
            );
            if m::yes(&class["check"]) && m::yes(&class["runnable"]) {
                dom::add(
                    &block,
                    &dom::button(
                        "Prefill verification command",
                        "run-prefill",
                        class["bare"].clone(),
                    ),
                );
            }
            block
        },
    );
    let chosen = m::data("run-tab");
    let tab = shown_tab(&run, chosen.as_str().unwrap_or("files"));
    if chosen.as_str().is_some_and(|chosen| chosen != tab) {
        // The fallback becomes the choice: the panel does not jump to Plan when one arrives later.
        m::set("run-tab", json!(tab));
    }
    for name in TABS {
        let selected = name == tab;
        let button = format!("run-tab-{name}");
        // Plan, Commands, Memory and Omnigraph ship hidden: each joins the tab list once the
        // launch has one.
        dom::hidden(&button, name != "files" && !has(&run, name));
        dom::attr(&button, "aria-selected", truth(selected));
        // Every shown tab is a tab stop: the page has no arrow-key handling for the tab list.
        dom::attr(&button, "tabindex", "0");
        dom::hidden(&format!("run-pane-{name}"), !selected);
    }
}

/// Opens the finished turn's details on a tab; while a turn runs the strip is its own.
fn open(tab: &str) {
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

fn close() {
    m::take("run-after");
    render();
    focus("prompt");
}

fn detail(on: bool) {
    m::set("run-detailed", json!(on));
    render();
}

pub fn activity(event: &V) {
    let sid = m::read(|s| s.sid.clone());
    crate::activity::observe_session(&sid, event);
}

pub fn outcomes(session: &V) {
    let fleet = session["workflow"] == "fleet-review";
    let result = &session["fleet_result"];
    dom::hidden("fleet-progress", !fleet);
    dom::hidden("fleet-outcome", !fleet || result.is_null());
    if fleet {
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
            &format!(
                "{} findings · reported cost {}",
                m::array(&result["findings"]).len(),
                m::string(&result["cost_usd"])
            ),
        );
        dom::rows("fleet-findings", &m::array(&result["findings"]));
        dom::text(
            "fleet-report-text",
            result["report"]
                .as_str()
                .or_else(|| result["preview"].as_str())
                .unwrap_or(""),
        );
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
        dom::text("clash-outcome-summary", &m::summary(r));
        dom::rows("clash-items", &m::array(&r["findings"]));
        dom::rows("clash-turn-list", &m::array(&r["turns"]));
        dom::text("clash-report-text", &m::text(r, "report"));
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

pub async fn event(kind: &str, id: &str, action: &str, payload: &V, _value: &str) -> bool {
    if kind != "click" {
        return false;
    }
    match action {
        "run-calm" => detail(false),
        "run-detailed" => detail(true),
        "run-compact" | "run-now" => detail(!m::yes(&m::data("run-detailed"))),
        "run-close" => close(),
        "run-open" => open(payload.as_str().unwrap_or("files")),
        "run-tab-files" | "run-tab-plan" | "run-tab-commands" | "run-tab-memory"
        | "run-tab-omnigraph" => {
            m::set("run-tab", json!(action.trim_start_matches("run-tab-")));
            // The file card belongs to the Files tab.
            hide_card(false);
            render();
        }
        "run-view-map" | "run-view-list" => {
            let view = if action == "run-view-map" {
                "map"
            } else {
                "list"
            };
            m::set("run-files-view", json!(view));
            hide_card(false);
            render();
        }
        "run-prefill" => {
            dom::set_value("check-command", payload.as_str().unwrap_or(""));
            session::open_view("checks", true).await;
            dom::focus("check-command");
        }
        "run-away-close" => {
            away(false);
            focus("run-now");
        }
        "run-away-jump" => {
            if let Some(e) =
                dom::get("conversation").and_then(|v| v.dyn_into::<web_sys::HtmlElement>().ok())
            {
                e.set_scroll_top(e.scroll_height());
            }
            away(false);
            focus("run-now");
        }
        "run-reveal" => {
            hide_card(false);
            let selector = format!("[data-event-id=\"{}\"]", m::string(payload));
            if let Ok(Some(e)) = dom::document().query_selector(&selector) {
                e.scroll_into_view();
            }
        }
        "run-file" => {
            // A second press on the chip puts its card away again.
            let path = payload.as_str().unwrap_or("");
            if card_path().as_deref() == Some(path) {
                hide_card(true);
            } else {
                show_card(&shown(), path);
            }
        }
        "run-add-path" => {
            hide_card(false);
            insert_draft(payload.as_str().unwrap_or(""));
        }
        "run-copy-path" => {
            let copying = dom::window()
                .navigator()
                .clipboard()
                .write_text(payload.as_str().unwrap_or(""));
            let copied = wasm_bindgen_futures::JsFuture::from(copying).await.is_ok();
            let selector = format!("#{CARD} [data-action=\"run-copy-path\"]");
            if let Ok(Some(button)) = dom::document().query_selector(&selector) {
                button.set_text_content(Some(if copied { "Copied" } else { "Copy failed" }));
            }
        }
        "run-notify" => {
            if let Ok(p) = web_sys::Notification::request_permission() {
                let _ = wasm_bindgen_futures::JsFuture::from(p).await;
            }
        }
        _ => {
            if id.starts_with("run-filter-") {
                m::set(
                    "run-file-filter",
                    json!(id.trim_start_matches("run-filter-")),
                );
                render();
            } else {
                return false;
            }
        }
    }
    true
}

fn snapshot() -> V {
    let run = m::data("run");
    let edits = run["files"]
        .as_object()
        .map(|m| m.values().filter_map(|v| v["edits"].as_u64()).sum::<u64>())
        .unwrap_or(0);
    let files = run["files"]
        .as_object()
        .map(|m| {
            m.values()
                .filter(|v| v["edits"].as_u64().unwrap_or(0) > 0)
                .map(|v| m::text(v, "path"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let failed = m::array(&run["commands"])
        .iter()
        .filter(|v| v["class"]["check"] == true && v["running"] != true && v["ok"] == false)
        .count();
    json!({"sid":m::read(|s|s.sid.clone()),"launch":run["launch"],"steps":run["steps"],"edits":edits,"files":files,"messages":run["messages"],"failed":failed,"plan":run["plan"],"finished":run["finished"],"outcome":run["outcome"]})
}

pub fn visibility() {
    if dom::document().hidden() {
        if m::read(|s| m::active(&s.selected)) {
            m::set("run-away-snapshot", snapshot());
            m::set("run-hidden-at", json!(js_sys::Date::now()));
        }
        return;
    }
    let before = m::take("run-away-snapshot");
    let hidden = m::take("run-hidden-at").as_f64();
    if !before.is_object() {
        return;
    }
    let after = snapshot();
    if before["sid"] != after["sid"] || before["launch"] != after["launch"] {
        return;
    }
    let steps = after["steps"]
        .as_u64()
        .unwrap_or(0)
        .saturating_sub(before["steps"].as_u64().unwrap_or(0));
    let edits = after["edits"]
        .as_u64()
        .unwrap_or(0)
        .saturating_sub(before["edits"].as_u64().unwrap_or(0));
    let messages = after["messages"]
        .as_u64()
        .unwrap_or(0)
        .saturating_sub(before["messages"].as_u64().unwrap_or(0));
    let failed = after["failed"]
        .as_u64()
        .unwrap_or(0)
        .saturating_sub(before["failed"].as_u64().unwrap_or(0));
    if steps == 0 && edits == 0 && messages == 0 && after["finished"] != true {
        return;
    }
    let minutes = hidden
        .map(|at| ((js_sys::Date::now() - at) / 60000.).floor() as u64)
        .unwrap_or(0);
    let text = format!(
        "While you were away{}: {steps} steps · {edits} edits · {messages} messages · {failed} failed checks{}",
        if minutes > 0 {
            format!(" ({minutes} min)")
        } else {
            String::new()
        },
        if after["finished"] == true {
            format!(" · run {}", m::text(&after, "outcome"))
        } else {
            String::new()
        }
    );
    dom::text("run-away-text", &text);
    away(true);
    dom::text("run-announcer", &text);
}

fn notify_finished() {
    if !dom::document().hidden()
        || web_sys::Notification::permission() != web_sys::NotificationPermission::Granted
    {
        return;
    }
    let Some(hidden) = m::data("run-hidden-at").as_f64() else {
        return;
    };
    if js_sys::Date::now() - hidden < 20000. {
        return;
    }
    let run = m::data("run");
    let before = m::data("run-away-snapshot");
    let sid = m::read(|s| s.sid.clone());
    if before["sid"] != sid || before["launch"] != run["launch"] || !m::yes(&run["finished"]) {
        return;
    }
    let key = format!("{}:{}", sid, m::string(&run["launch"]));
    if m::data("run-notified") == key {
        return;
    }
    m::set("run-notified", json!(key));
    let title = if run["outcome"] == "completed" {
        "Harness run finished"
    } else {
        "Harness run needs attention"
    };
    let body = format!(
        "{} · {} steps",
        m::text(&run, "outcome"),
        m::string(&run["steps"])
    );
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"body".into(), &body.into());
    let _ = js_sys::Reflect::set(
        &options,
        &"tag".into(),
        &format!("harness-run-{sid}").into(),
    );
    let _ = js_sys::Reflect::set(&options, &"silent".into(), &true.into());
    let options: web_sys::NotificationOptions = options.unchecked_into();
    if let Ok(note) = web_sys::Notification::new_with_options(title, &options) {
        let n = note.clone();
        let cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            let _ = dom::window().focus();
            n.close();
        });
        note.set_onclick(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    fn launch(id: &str, kinds: &[&str]) -> V {
        let mut run = initial();
        run["launch"] = json!(id);
        run["kinds"] = json!(kinds);
        run
    }
    fn session(status: &str) -> V {
        json!({"id":"s","status":status,"workflow":"native"})
    }
    const NONE: V = V::Null;
    /// The mode when the newest launch replaced none.
    fn mode(session: &V, run: &V, after: &V, idle: &V) -> Mode {
        super::mode(session, run, &NONE, after, idle)
    }

    #[test]
    fn the_view_stays_closed_while_nothing_runs() {
        let run = launch("a", &["status", "tool", "text"]);
        assert_eq!(mode(&NONE, &NONE, &NONE, &NONE), Mode::Closed);
        assert_eq!(mode(&NONE, &run, &NONE, &NONE), Mode::Closed);
        for status in [
            "completed",
            "failed",
            "cancelled",
            "interrupted",
            "awaiting_context",
            "awaiting_approval",
        ] {
            assert_eq!(
                mode(&session(status), &run, &NONE, &json!("a")),
                Mode::Closed,
                "{status}"
            );
        }
    }

    #[test]
    fn a_running_native_launch_opens_the_now_line() {
        let running = session("running");
        for kind in [
            "session", "text", "tool", "plan", "usage", "result", "error",
        ] {
            assert_eq!(
                mode(&running, &launch("a", &["status", kind]), &NONE, &NONE),
                Mode::Now,
                "{kind}"
            );
        }
        // A reload during the run replays the record with the session still running.
        assert_eq!(
            mode(&running, &launch("b", &["tool"]), &NONE, &json!("a")),
            Mode::Now
        );
    }

    #[test]
    fn the_strip_waits_until_the_provider_writes() {
        let running = session("running");
        assert_eq!(mode(&running, &NONE, &NONE, &NONE), Mode::Waiting);
        assert_eq!(
            mode(&running, &launch("", &["user"]), &NONE, &NONE),
            Mode::Waiting
        );
        assert_eq!(
            mode(&running, &launch("a", &["status", "memory"]), &NONE, &NONE),
            Mode::Waiting
        );
        assert_eq!(
            mode(&session("queued"), &launch("a", &["tool"]), &NONE, &NONE),
            Mode::Waiting
        );
    }

    #[test]
    fn an_earlier_launch_never_stands_in_for_the_one_that_starts() {
        let running = session("running");
        // The person's next message arrived after this launch began.
        let answered = launch("a", &["tool", "text", "user"]);
        assert_eq!(mode(&running, &answered, &NONE, &NONE), Mode::Waiting);
        // The page saw the session at rest with this launch: a check or a retry runs now.
        let rested = launch("a", &["tool", "text"]);
        assert_eq!(mode(&running, &rested, &NONE, &json!("a")), Mode::Waiting);
        // The launch closed itself while the session still settles.
        let mut closed = launch("a", &["tool", "status"]);
        closed["finished"] = json!(true);
        assert_eq!(mode(&running, &closed, &NONE, &NONE), Mode::Waiting);
        // Its successor's first provider event takes the strip.
        assert_eq!(
            mode(&running, &launch("b", &["tool"]), &NONE, &json!("a")),
            Mode::Now
        );
        // A record without launch ids holds the messages too; they end nothing.
        assert!(!over(&launch("", &["user", "tool"]), &NONE));
        assert!(over(&launch("", &["user", "tool"]), &json!("")));
    }

    #[test]
    fn sessions_with_their_own_progress_only_wait() {
        let run = launch("a", &["status", "tool"]);
        let mut sessions = vec![json!({"id":"s","status":"running","workflow":"fleet-review"})];
        for key in [
            "fleet",
            "clash",
            "creator",
            "system_run",
            "system_discovery",
        ] {
            let mut session = session("running");
            session[key] = json!({});
            sessions.push(session);
        }
        for mut session in sessions {
            assert_eq!(mode(&session, &run, &NONE, &NONE), Mode::Waiting);
            session["status"] = json!("completed");
            assert_eq!(mode(&session, &run, &json!("a"), &NONE), Mode::Closed);
            assert!(openers(&session, &run).is_empty());
        }
        for kind in [
            "fleet_stage",
            "fleet_reviewer",
            "clash_turn",
            "agent",
            "agent_activity",
        ] {
            let mut run = launch("a", &["tool", kind]);
            run["files"] = json!({"a.php":{"path":"a.php"}});
            assert_eq!(mode(&session("running"), &run, &NONE, &NONE), Mode::Waiting);
            assert_eq!(
                mode(&session("completed"), &run, &json!("a"), &NONE),
                Mode::Closed
            );
            assert!(openers(&session("completed"), &run).is_empty());
        }
    }

    #[test]
    fn a_finished_turn_opens_for_its_own_launch_and_closes_again() {
        let done = session("completed");
        let run = launch("a", &["tool", "text"]);
        assert_eq!(mode(&done, &run, &json!("a"), &json!("a")), Mode::After);
        // Close forgets the launch; another launch's request opens nothing.
        assert_eq!(mode(&done, &run, &NONE, &json!("a")), Mode::Closed);
        assert_eq!(mode(&done, &run, &json!("b"), &json!("a")), Mode::Closed);
        assert_eq!(mode(&done, &NONE, &json!("a"), &NONE), Mode::Closed);
        // No session is selected: a launch left in the model opens nothing.
        assert_eq!(mode(&NONE, &run, &json!("a"), &NONE), Mode::Closed);
        // A record without launch ids is one launch with an empty id.
        assert_eq!(
            mode(
                &done,
                &launch("", &["user", "tool"]),
                &json!(""),
                &json!("")
            ),
            Mode::After
        );
        // The next turn takes the strip back.
        assert_eq!(
            mode(&session("queued"), &run, &json!("a"), &json!("a")),
            Mode::Waiting
        );
        assert_eq!(
            mode(&session("running"), &run, &json!("a"), &json!("a")),
            Mode::Waiting
        );
    }

    #[test]
    fn a_finished_turn_offers_only_the_tabs_it_filled() {
        let done = session("completed");
        let mut run = launch("a", &["tool", "text"]);
        assert!(openers(&done, &run).is_empty());
        run["files"] = json!({"a.php":{"path":"a.php","reads":1,"edits":0}});
        assert_eq!(openers(&done, &run), ["files"]);
        run["plan"] = json!([{"content":"Write the test","status":"pending"}]);
        run["commands"] = json!([{"call":"c1","command":"composer test"}]);
        assert_eq!(openers(&done, &run), ["files", "plan", "commands"]);
        run["files"] = json!({});
        assert_eq!(openers(&done, &run), ["plan", "commands"]);
        // While a turn runs, the strip is its own; without a session there is nothing to open.
        assert!(openers(&session("running"), &run).is_empty());
        assert!(openers(&session("queued"), &run).is_empty());
        assert!(openers(&NONE, &run).is_empty());
        assert!(openers(&done, &NONE).is_empty());
    }

    #[test]
    fn the_panel_shows_files_when_the_chosen_tab_is_empty() {
        let mut run = launch("a", &["tool"]);
        assert_eq!(shown_tab(&run, "files"), "files");
        assert_eq!(shown_tab(&run, "plan"), "files");
        assert_eq!(shown_tab(&run, "commands"), "files");
        assert_eq!(shown_tab(&run, "elsewhere"), "files");
        run["plan"] = json!([{"content":"Write the test","status":"pending"}]);
        assert_eq!(shown_tab(&run, "plan"), "plan");
        assert_eq!(shown_tab(&run, "commands"), "files");
        // A plan event without a list leaves the tab out, as the pane would be empty.
        run["plan"] = json!({"note":"no items"});
        assert_eq!(shown_tab(&run, "plan"), "files");
    }

    #[test]
    fn openers_and_shown_tab_include_memory_and_omnigraph_when_present() {
        let done = session("completed");
        let mut run = launch("a", &["tool", "text"]);
        run["files"] = json!({"a.php":{"path":"a.php","reads":1,"edits":0}});
        assert_eq!(openers(&done, &run), ["files"]);
        assert_eq!(shown_tab(&run, "memory"), "files");
        run["memory"] = json!({"kind":"memory","ok":true,"text":"Saved 2 learnings.","delivered_sources":3,"opened_delivered_sources":1});
        run["omnigraph"] =
            json!({"kind":"omnigraph","phase":"inbox_notify","ok":false,"text":"Not told <now>."});
        assert_eq!(openers(&done, &run), ["files", "memory", "omnigraph"]);
        assert_eq!(shown_tab(&run, "memory"), "memory");
        assert_eq!(shown_tab(&run, "omnigraph"), "omnigraph");
        let memory = render_run_memory_pane_html(&run["memory"]);
        assert!(memory.contains("data-ok=\"true\">Saved</p>"), "{memory}");
        assert!(memory.contains("<li>3 sources delivered</li>"), "{memory}");
        let omnigraph = render_run_omnigraph_pane_html(&run["omnigraph"]);
        assert!(
            omnigraph.contains("data-ok=\"false\">inbox notify</p>"),
            "{omnigraph}"
        );
        assert!(omnigraph.contains("Not told &lt;now&gt;."), "{omnigraph}");
        assert_eq!(render_run_memory_pane_html(&json!(null)), "");
    }
}

#[cfg(test)]
mod check_tests {
    use super::*;

    const NONE: V = V::Null;
    /// The model after these events: the newest launch and the one it replaced.
    fn play(events: &[V]) -> (V, V) {
        let (mut run, mut before) = (V::Null, V::Null);
        for event in events {
            let (next, replaced) = advance(run, event);
            run = next;
            if let Some(replaced) = replaced {
                before = replaced;
            }
        }
        (run, before)
    }
    /// A turn that opened one file, under the launch id.
    fn a_turn(launch: &str) -> Vec<V> {
        vec![
            json!({"id":1,"kind":"user","text":"Add the index."}),
            json!({"id":2,"kind":"status","launch_id":launch,"text":"Starting Claude Code."}),
            json!({"id":3,"kind":"text","launch_id":launch,"text":"Reading the model."}),
            json!({"id":4,"kind":"tool","launch_id":launch,"tool":"Read","call":"r1","state":"started","path":"app/Order.php"}),
            json!({"id":5,"kind":"tool","launch_id":launch,"tool":"Read","call":"r1","state":"completed","ok":true,"path":"app/Order.php"}),
        ]
    }
    /// What the server writes for a check that ran: its result, then its end.
    fn a_check(launch: &str, result: &str) -> Vec<V> {
        vec![
            json!({"id":10,"kind":"status","launch_id":launch,"text":format!("Check {result}: php artisan test. This is separate from the agent result.")}),
            json!({"id":11,"kind":"status","launch_id":launch,"text":"Check finished.","status":"completed"}),
        ]
    }
    fn done() -> V {
        json!({"id":"s","status":"completed","workflow":"native"})
    }

    #[test]
    fn the_model_follows_the_newest_launch() {
        let (run, before) = play(&a_turn("a"));
        assert_eq!(run["launch"], "a");
        assert_eq!(run["steps"], 1);
        assert_eq!(run["messages"], 1);
        assert!(has(&run, "files"));
        assert!(!is_check(&run));
        // The first launch replaced only the record of the message that asked for it.
        assert_eq!(before["launch"], "");
        assert!(!has(&before, "files"));
    }

    #[test]
    fn a_harness_check_leaves_the_turn_it_follows() {
        let mut events = a_turn("a");
        events.extend(a_check("c", "passed"));
        let (run, before) = play(&events);
        assert_eq!(run["launch"], "c");
        assert!(is_check(&run));
        let last = turn(&run, &before);
        assert_eq!(last["launch"], "a");
        assert_eq!(openers(&done(), last), ["files"]);
        // The line opens the turn's details, not the check's, and Close puts them away.
        assert_eq!(
            mode(&done(), &run, &before, &json!("a"), &json!("c")),
            Mode::After
        );
        assert_eq!(
            mode(&done(), &run, &before, &json!("c"), &json!("c")),
            Mode::Closed
        );
        assert_eq!(
            mode(&done(), &run, &before, &NONE, &json!("c")),
            Mode::Closed
        );
        // The same holds for a check that failed.
        let mut events = a_turn("a");
        events.extend(a_check("c", "failed"));
        let (run, before) = play(&events);
        assert_eq!(turn(&run, &before)["launch"], "a");
    }

    #[test]
    fn a_check_that_could_not_run_is_still_a_check() {
        let mut events = a_turn("a");
        events.push(
            json!({"id":10,"kind":"error","launch_id":"c","text":"The check worker is busy."}),
        );
        events.push(json!({"id":11,"kind":"status","launch_id":"c","text":"Check finished.","status":"completed"}));
        let (run, before) = play(&events);
        assert!(is_check(&run));
        assert_eq!(turn(&run, &before)["launch"], "a");
    }

    #[test]
    fn checks_in_a_row_keep_the_turn_before_them() {
        let mut events = a_turn("a");
        events.extend(a_check("c", "failed"));
        events.extend(a_check("d", "passed"));
        let (run, before) = play(&events);
        assert_eq!(run["launch"], "d");
        assert_eq!(before["launch"], "a");
        assert_eq!(openers(&done(), turn(&run, &before)), ["files"]);
    }

    #[test]
    fn a_running_check_only_waits() {
        let running = json!({"id":"s","status":"running","workflow":"native"});
        // Before the check writes, the model still holds the turn the page saw at rest.
        let (run, before) = play(&a_turn("a"));
        assert_eq!(
            mode(&running, &run, &before, &NONE, &json!("a")),
            Mode::Waiting
        );
        // Its result arrives while the session still settles.
        let mut events = a_turn("a");
        events.push(a_check("c", "passed").remove(0));
        let (run, before) = play(&events);
        assert_eq!(
            mode(&running, &run, &before, &NONE, &json!("a")),
            Mode::Waiting
        );
        // An error on its record does not make a check what the Now line describes.
        events.push(
            json!({"id":12,"kind":"error","launch_id":"c","text":"The command could not start."}),
        );
        let (run, before) = play(&events);
        assert!(cli_started(&run));
        assert_eq!(
            mode(&running, &run, &before, &NONE, &json!("a")),
            Mode::Waiting
        );
        // Nothing opens under a running check, and there is no line to open it from.
        assert_eq!(
            mode(&running, &run, &before, &json!("a"), &json!("a")),
            Mode::Waiting
        );
        assert!(openers(&running, turn(&run, &before)).is_empty());
    }

    #[test]
    fn the_turn_after_a_check_takes_over() {
        let running = json!({"id":"s","status":"running","workflow":"native"});
        let mut events = a_turn("a");
        events.extend(a_check("c", "passed"));
        events.push(json!({"id":20,"kind":"user","text":"Now the migration."}));
        // The message is in, the new launch has written nothing: the strip waits.
        let (run, before) = play(&events);
        assert_eq!(
            mode(&running, &run, &before, &NONE, &json!("c")),
            Mode::Waiting
        );
        events.push(json!({"id":21,"kind":"text","launch_id":"b","text":"Writing it."}));
        let (run, before) = play(&events);
        assert_eq!(run["launch"], "b");
        assert_eq!(mode(&running, &run, &before, &NONE, &json!("c")), Mode::Now);
        // The check is not kept: behind the new turn stands the turn before the check.
        assert_eq!(before["launch"], "a");
        // At rest the new turn is the last one, and it opened no file.
        assert_eq!(turn(&run, &before)["launch"], "b");
        assert!(openers(&done(), turn(&run, &before)).is_empty());
    }

    #[test]
    fn a_turn_without_details_does_not_borrow_the_earlier_one() {
        let mut events = a_turn("a");
        events.push(json!({"id":20,"kind":"user","text":"What did you change?"}));
        events.push(json!({"id":21,"kind":"text","launch_id":"b","text":"Only the model."}));
        let (run, before) = play(&events);
        assert_eq!(before["launch"], "a");
        assert_eq!(turn(&run, &before)["launch"], "b");
        assert!(openers(&done(), turn(&run, &before)).is_empty());
        assert_eq!(
            mode(&done(), &run, &before, &json!("a"), &json!("b")),
            Mode::Closed
        );
    }

    #[test]
    fn only_a_launch_the_provider_never_wrote_to_is_a_check() {
        for kind in ["session", "text", "tool", "plan", "usage", "result"] {
            let (run, _) = play(&[
                json!({"id":1,"kind":kind,"launch_id":"a","text":"x"}),
                json!({"id":2,"kind":"status","launch_id":"a","text":"Check passed: make test. Fine."}),
            ]);
            assert!(m::yes(&run["check"]));
            assert!(!is_check(&run), "{kind}");
        }
        // A launch that wrote no such line is no check, however little it wrote.
        let (run, _) = play(&[
            json!({"id":1,"kind":"status","launch_id":"a","text":"Context prepared. Review it before starting the provider."}),
            json!({"id":2,"kind":"error","launch_id":"a","text":"Provider is not installed."}),
        ]);
        assert!(!is_check(&run));
    }

    #[test]
    fn the_check_lines_are_told_from_other_status_text() {
        for text in [
            "Check passed: php artisan test. This is separate from the agent result.",
            "Check failed: vendor/bin/phpstan analyse. This is separate from the agent result.",
            "Check cancelled: make test. This is separate from the agent result.",
            "Check finished.",
        ] {
            assert!(check_status(text), "{text}");
        }
        for text in [
            "",
            "Check",
            "Check finished",
            "Checking composer.json",
            "Checked 3 files: ok",
            "Check : nothing",
            "Check the plan: step one",
            "Cancellation requested.",
        ] {
            assert!(!check_status(text), "{text}");
        }
    }
}

#[cfg(test)]
mod files_tests {
    use super::*;

    const L: &str = "a";
    /// The launch after these events.
    fn play(events: &[V]) -> V {
        let mut run = V::Null;
        for event in events {
            run = advance(run, event).0;
        }
        run
    }
    fn tool(id: u64, name: &str, call: &str, state: &str, more: V) -> V {
        let mut event = json!({"id":id,"kind":"tool","launch_id":L,"tool":name,"call":call,"state":state,"text":format!("{name}: {state}")});
        for (key, value) in more.as_object().into_iter().flatten() {
            event[key] = value.clone();
        }
        event
    }
    /// A call that starts and reports its result, as two events.
    fn done(id: u64, name: &str, call: &str, more: V, ok: bool) -> Vec<V> {
        let mut end = more.clone();
        end["ok"] = json!(ok);
        vec![
            tool(id, name, call, "started", more),
            tool(id + 1, name, call, "completed", end),
        ]
    }
    fn file<'a>(run: &'a V, path: &str) -> &'a V {
        &run["files"][path]
    }

    #[test]
    fn a_call_counts_once_for_the_file_it_names() {
        let mut events = done(1, "Read", "r1", json!({"path":"app/Order.php"}), true);
        events.extend(done(3, "Edit", "e1", json!({"path":"app/Order.php"}), true));
        events.extend(done(5, "Read", "r2", json!({"path":"app/Order.php"}), true));
        let run = play(&events);
        assert_eq!(file(&run, "app/Order.php")["reads"], 2);
        assert_eq!(file(&run, "app/Order.php")["edits"], 1);
        assert_eq!(run["steps"], 3);
        // A result that arrives twice changes nothing.
        events.push(tool(
            9,
            "Edit",
            "e1",
            "completed",
            json!({"path":"app/Order.php","ok":true}),
        ));
        assert_eq!(file(&play(&events), "app/Order.php")["edits"], 1);
    }

    #[test]
    fn an_edit_counts_when_the_tool_reports_it_done() {
        let started = [tool(1, "Edit", "e1", "started", json!({"path":"a.php"}))];
        let run = play(&started);
        assert_eq!(file(&run, "a.php")["edits"], 0);
        assert!(!m::yes(&file(&run, "a.php")["failed"]));
        let run = play(&done(1, "Edit", "e1", json!({"path":"a.php"}), false));
        assert_eq!(file(&run, "a.php")["edits"], 0);
        assert!(m::yes(&file(&run, "a.php")["failed"]));
        // A step that was never run reported nothing about its file.
        let run = play(&[
            tool(1, "Edit", "e1", "started", json!({"path":"a.php"})),
            tool(
                2,
                "Edit",
                "e1",
                "completed",
                json!({"path":"a.php","ok":false,"outcome":"not_run"}),
            ),
        ]);
        assert_eq!(file(&run, "a.php")["edits"], 0);
        assert!(!m::yes(&file(&run, "a.php")["failed"]));
        // A failed read marks the file too.
        let run = play(&done(1, "Read", "r1", json!({"path":"a.php"}), false));
        assert_eq!(file(&run, "a.php")["reads"], 1);
        assert!(m::yes(&file(&run, "a.php")["failed"]));
    }

    #[test]
    fn a_result_without_a_start_is_the_whole_call() {
        let run = play(&[tool(
            1,
            "Edit",
            "e1",
            "completed",
            json!({"path":"a.php","ok":true}),
        )]);
        assert_eq!(file(&run, "a.php")["edits"], 1);
        let run = play(&[tool(
            1,
            "Read",
            "r1",
            "completed",
            json!({"path":"a.php","ok":true}),
        )]);
        assert_eq!(file(&run, "a.php")["reads"], 1);
    }

    #[test]
    fn a_change_names_its_own_files() {
        let changes = json!({"changes":[
            {"path":"app/New.php","kind":"add"},
            {"path":"app/Old.php","kind":"delete"},
            {"path":"app\\Kept.php","kind":"update"},
            {"path":"notes.txt","kind":"update","outside":true},
        ],"ok":true});
        // Codex reports a change when it is done: the start names nothing.
        let run = play(&[
            tool(1, "file_change", "c1", "started", json!({})),
            tool(2, "file_change", "c1", "completed", changes),
        ]);
        assert!(m::yes(&file(&run, "app/New.php")["created"]));
        assert!(m::yes(&file(&run, "app/Old.php")["deleted"]));
        assert_eq!(file(&run, "app/Kept.php")["edits"], 1);
        assert!(m::yes(&file(&run, "notes.txt")["outside"]));
        for path in ["app/New.php", "app/Old.php", "app/Kept.php", "notes.txt"] {
            assert_eq!(file(&run, path)["edits"], 1, "{path}");
            assert_eq!(file(&run, path)["reads"], 0, "{path}");
        }
    }

    #[test]
    fn labels_without_call_ids_pair_in_order() {
        let label = |id: u64, text: &str| json!({"id":id,"kind":"tool","launch_id":L,"text":text});
        let run = play(&[
            label(1, "Read: started"),
            label(2, "Read: started"),
            label(3, "Bash: started"),
            label(4, "Read: completed"),
        ]);
        let open = |run: &V| {
            run["calls"]
                .as_object()
                .map(|calls| calls.values().filter(|call| m::yes(&call["open"])).count())
        };
        assert_eq!(run["steps"], 3);
        assert_eq!(open(&run), Some(2));
        let run = play(&[
            label(1, "Read: started"),
            label(2, "Read: completed"),
            label(3, "Read: completed"),
        ]);
        assert_eq!(open(&run), Some(0));
    }

    #[test]
    fn the_map_keeps_the_order_files_were_touched() {
        let mut events = done(1, "Read", "r1", json!({"path":"tests/OrderTest.php"}), true);
        events.extend(done(
            3,
            "Read",
            "r2",
            json!({"path":"app/Models/Order.php"}),
            true,
        ));
        events.extend(done(5, "Read", "r3", json!({"path":"composer.json"}), true));
        events.extend(done(
            7,
            "Edit",
            "e1",
            json!({"path":"app/Http/Controllers/Api/V1/Admin/OrderController.php"}),
            true,
        ));
        events.extend(done(
            9,
            "Read",
            "r4",
            json!({"path":"app/Models/User.php"}),
            true,
        ));
        events.extend(done(
            11,
            "Read",
            "r5",
            json!({"path":"secrets.env","outside":true}),
            true,
        ));
        let districts = map(&play(&events), "all", false);
        let names: Vec<_> = districts.iter().map(|d| m::text(d, "label")).collect();
        assert_eq!(names, ["tests/", "app/", "./", "Outside the project"]);
        let app = &districts[1];
        let blocks: Vec<_> = m::array(&app["blocks"])
            .iter()
            .map(|b| m::text(b, "name"))
            .collect();
        assert_eq!(blocks, ["Models", "Http/Controllers/Api/…"]);
        let models = &app["blocks"][0];
        assert_eq!(models["label"], "app/Models, 2 files");
        assert_eq!(
            app["blocks"][1]["label"],
            "app/Http/Controllers/Api/…, 1 file, 1 edited"
        );
        let chips: Vec<_> = m::array(&models["files"])
            .iter()
            .map(|c| m::text(c, "name"))
            .collect();
        assert_eq!(chips, ["Order.php", "User.php"]);
        assert_eq!(districts[2]["blocks"][0]["label"], "Project root, 1 file");
        assert_eq!(
            districts[3]["blocks"][0]["label"],
            "Outside the project, 1 file"
        );
    }

    #[test]
    fn a_wordpress_plugin_is_a_district_of_its_own() {
        let at = |path: &str| place_of(&json!({"path":path}));
        assert_eq!(
            at("wp-content/plugins/shop/includes/class-order.php"),
            ("wp-content/plugins/shop".into(), "includes".into())
        );
        assert_eq!(
            at("wp-content/themes/store/functions.php"),
            ("wp-content/themes/store".into(), String::new())
        );
        assert_eq!(
            at("wp-content/uploads/a.png"),
            ("wp-content".into(), "uploads".into())
        );
        assert_eq!(at("composer.json"), (String::new(), String::new()));
        assert_eq!(
            place_of(&json!({"path":"outside","outside":true})),
            (OUTSIDE.into(), String::new())
        );
    }

    #[test]
    fn a_chip_carries_what_happened_to_its_file() {
        let mut events = done(1, "Read", "r1", json!({"path":"app/Order.php"}), true);
        events.extend(done(3, "Read", "r2", json!({"path":"app/Order.php"}), true));
        events.extend(done(5, "Edit", "e1", json!({"path":"app/Order.php"}), true));
        events.extend(done(
            7,
            "Edit",
            "e2",
            json!({"path":"app/Broken.php"}),
            false,
        ));
        events.extend(done(
            9,
            "Read",
            "h1",
            json!({"path":"app/Helper.php","parent":"task-1"}),
            true,
        ));
        events.push(tool(
            11,
            "file_change",
            "c1",
            "completed",
            json!({"changes":[{"path":"app/New.php","kind":"add"},{"path":"app/Old.php","kind":"delete"}],"ok":true}),
        ));
        events.push(tool(
            12,
            "Read",
            "r9",
            "started",
            json!({"path":"app/Open.php"}),
        ));
        let run = play(&events);
        let chip = |running: bool, name: &str| {
            map(&run, "all", running)[0]["blocks"][0]["files"]
                .as_array()
                .and_then(|chips| chips.iter().find(|chip| chip["name"] == name).cloned())
                .unwrap_or(V::Null)
        };
        let order = chip(true, "Order.php");
        assert_eq!(order["times"], "×2");
        assert_eq!(order["edited"], true);
        assert_eq!(order["mark"], "");
        assert_eq!(
            order["label"],
            "app/Order.php, opened 2 times, edit reported ok"
        );
        let broken = chip(true, "Broken.php");
        assert_eq!(broken["mark"], "!");
        assert_eq!(broken["edited"], false);
        assert_eq!(broken["failed"], true);
        assert_eq!(chip(true, "New.php")["mark"], "+");
        assert_eq!(chip(true, "New.php")["created"], true);
        assert_eq!(chip(true, "Old.php")["deleted"], true);
        assert_eq!(chip(true, "Helper.php")["helper"], true);
        assert_eq!(order["helper"], false);
        assert_eq!(
            chip_words(file(&run, "app/Order.php")),
            ["opened 2 times", "edit reported ok"]
        );
        // The ring follows the call that is open, and only while the launch runs.
        assert_eq!(chip(true, "Open.php")["now"], true);
        assert_eq!(
            chip(true, "Open.php")["label"],
            "app/Open.php, opened once, in use now"
        );
        assert_eq!(chip(false, "Open.php")["now"], false);
        assert_eq!(order["now"], false);
    }

    #[test]
    fn filters_offer_what_the_launch_has_and_dim_the_rest() {
        let mut events = done(1, "Read", "r1", json!({"path":"app/Order.php"}), true);
        events.extend(done(3, "Edit", "e1", json!({"path":"app/Order.php"}), true));
        events.extend(done(
            5,
            "Read",
            "r2",
            json!({"path":"tests/OrderTest.php"}),
            true,
        ));
        let run = play(&events);
        let labels = |run: &V, chosen: &str| -> Vec<(String, bool)> {
            filters(run, chosen)
                .iter()
                .map(|f| (m::text(f, "label"), m::yes(&f["pressed"])))
                .collect()
        };
        assert_eq!(
            labels(&run, "all"),
            [("All".to_owned(), true), ("Edited 1".to_owned(), false)]
        );
        assert_eq!(
            labels(&run, "edited"),
            [("All".to_owned(), false), ("Edited 1".to_owned(), true)]
        );
        // A choice the launch has nothing for reads as All.
        assert_eq!(labels(&run, "failed")[0], ("All".to_owned(), true));
        let dim = |filter: &str| -> Vec<bool> {
            map(&run, filter, false)
                .iter()
                .flat_map(|d| m::array(&d["blocks"]))
                .flat_map(|b| m::array(&b["files"]))
                .map(|chip| m::yes(&chip["dim"]))
                .collect()
        };
        assert_eq!(dim("all"), [false, false]);
        assert_eq!(dim("edited"), [false, true]);
        assert!(!matches(&run, file(&run, "tests/OrderTest.php"), "edited"));
        // A search in a folder offers Searched for the files under it; one without a folder
        // looked everywhere.
        events.push(tool(
            7,
            "Grep",
            "g1",
            "started",
            json!({"detail":"rules(","path":"app"}),
        ));
        let run = play(&events);
        assert!(labels(&run, "all").contains(&("Searched 1".to_owned(), false)));
        assert_eq!(searched_in(&run, file(&run, "app/Order.php")), ["rules("]);
        assert!(searched_in(&run, file(&run, "tests/OrderTest.php")).is_empty());
        events.push(tool(
            8,
            "Glob",
            "g2",
            "started",
            json!({"detail":"**/*.php"}),
        ));
        let run = play(&events);
        assert!(labels(&run, "all").contains(&("Searched 2".to_owned(), false)));
    }

    #[test]
    fn the_card_says_what_the_page_knows_about_the_file() {
        let mut events = done(1, "Read", "r1", json!({"path":"app/Order.php"}), true);
        events.extend(done(3, "Edit", "e1", json!({"path":"app/Order.php"}), true));
        events.extend(done(5, "Edit", "e2", json!({"path":"app/Order.php"}), true));
        events.push(tool(
            7,
            "Grep",
            "g1",
            "started",
            json!({"detail":"rules(","path":"app"}),
        ));
        let run = play(&events);
        assert_eq!(
            card_facts(&run, file(&run, "app/Order.php")),
            [
                "Opened ×1",
                "In a folder searched for rules(",
                "Edit reported ok ×2"
            ]
        );
        assert_eq!(file(&run, "app/Order.php")["first_step"], 1);
        assert_eq!(file(&run, "app/Order.php")["last_step"], 3);
        let run = play(&[tool(
            1,
            "file_change",
            "c1",
            "completed",
            json!({"changes":[{"path":"a.php","kind":"add"}],"ok":true}),
        )]);
        assert_eq!(
            card_facts(&run, file(&run, "a.php")),
            ["Added (file_change)", "Edit reported ok"]
        );
    }

    #[test]
    fn the_card_stays_inside_the_window() {
        let (card, window) = ((360., 140.), (1280., 800.));
        // Under the chip, a little to its left.
        assert_eq!(place((400., 300., 324.), card, window), (392., 332.));
        // Above it when there is no room below.
        assert_eq!(place((400., 700., 724.), card, window), (392., 552.));
        // Never past the right or the left edge, nor above the top.
        assert_eq!(place((1200., 300., 324.), card, window).0, 912.);
        assert_eq!(place((2., 300., 324.), card, window).0, 8.);
        assert_eq!(place((400., 60., 84.), (360., 760.), window).1, 8.);
        // A window narrower than the card keeps the card's left edge on screen.
        assert_eq!(place((100., 300., 324.), card, (320., 800.)).0, 8.);
    }

    #[test]
    fn a_path_goes_into_the_draft_at_the_caret() {
        assert_eq!(spliced("", 0, 0, "app/a.php"), ("app/a.php".into(), 9));
        assert_eq!(
            spliced("Look at", 7, 7, "app/a.php"),
            ("Look at app/a.php".into(), 17)
        );
        assert_eq!(
            spliced("Look at ", 8, 8, "a.php"),
            ("Look at a.php".into(), 13)
        );
        assert_eq!(
            spliced("see here", 3, 3, "a.php"),
            ("see a.php here".into(), 9)
        );
        assert_eq!(
            spliced("see  here", 4, 4, "a.php"),
            ("see a.php here".into(), 9)
        );
        // A selection is replaced.
        assert_eq!(
            spliced("see THIS here", 4, 8, "a.php"),
            ("see a.php here".into(), 9)
        );
        // Offsets count UTF-16 units: Cyrillic is one each, an emoji two.
        assert_eq!(
            spliced("Посмотри тут", 8, 8, "a.php"),
            ("Посмотри a.php тут".into(), 14)
        );
        assert_eq!(spliced("😀 x", 2, 2, "a.php"), ("😀 a.php x".into(), 8));
        // A caret past the end, or an end before the start, still lands in the text.
        assert_eq!(spliced("ab", 9, 9, "c"), ("ab c".into(), 4));
        assert_eq!(spliced("ab", 2, 0, "c"), ("ab c".into(), 4));
    }

    #[test]
    fn counts_read_as_english() {
        assert_eq!(count(0, "step"), "0 steps");
        assert_eq!(count(1, "step"), "1 step");
        assert_eq!(count(2, "file"), "2 files");
        assert_eq!(
            counts(&json!({"steps":1,"messages":1})),
            "1 step · 1 message"
        );
        assert_eq!(
            counts(&json!({"steps":9,"messages":2})),
            "9 steps · 2 messages"
        );
    }

    #[test]
    fn inspect_file_graph_action_appears_in_list_rows_and_file_card_for_project_files() {
        let mut events = done(1, "Edit", "e1", json!({"path":"app/Order.php"}), true);
        events.extend(done(
            3,
            "Read",
            "r1",
            json!({"path":"secrets.env","outside":true}),
            true,
        ));
        let run = play(&events);
        let project_actions = run_card_actions(file(&run, "app/Order.php"));
        assert!(project_actions.iter().any(|a| {
            a.action == "inspect-file-graph"
                && a.label == "Show on dependency map"
                && a.data_path.as_deref() == Some("app/Order.php")
                && a.payload["path"] == "app/Order.php"
        }));
        let card_html = files_map::render_file_card_html(
            &entry_from_run_file(
                &run,
                file(&run, "app/Order.php"),
                &std::collections::BTreeSet::new(),
            ),
            &project_actions,
        );
        assert!(card_html.contains("data-action=\"inspect-file-graph\""));
        assert!(card_html.contains("data-path=\"app/Order.php\""));
        assert!(card_html.contains(">Show on dependency map</button>"));

        // Outside files have no project code graph node, so they omit inspect-file-graph.
        let outside_actions = run_card_actions(file(&run, "secrets.env"));
        assert!(
            !outside_actions
                .iter()
                .any(|a| a.action == "inspect-file-graph")
        );

        let entries = entries_of_run(&run, false);
        let list_html = files_map::render_files_surface(
            &entries,
            "list",
            "all",
            false,
            &files_map::FilesSurfaceOptions::default(),
        );
        assert!(list_html.contains("data-action=\"inspect-file-graph\""));
        assert!(list_html.contains("data-path=\"app/Order.php\">Map</button>"));
        assert!(!list_html.contains("data-path=\"secrets.env\">Map</button>"));
    }

    #[test]
    fn files_map_groups_multi_service_entries_by_service_district_and_subfolder_blocks() {
        let entries = vec![
            FileTouchEntry {
                path: "src/Domain/Order.php".into(),
                service_id: Some("orders".into()),
                service_folder: Some("services/orders".into()),
                agents: vec!["service-orders".into(), "verify".into()],
                reads: 2,
                edits: 1,
                created: true,
                now: true,
                order: 0,
                first_step: 1,
                last_step: 4,
                ..FileTouchEntry::default()
            },
            FileTouchEntry {
                path: "contracts/order-placed.json".into(),
                service_id: Some("__system__".into()),
                service_folder: Some(".".into()),
                agents: vec!["contracts".into()],
                reads: 1,
                order: 1,
                first_step: 2,
                last_step: 2,
                ..FileTouchEntry::default()
            },
            FileTouchEntry {
                path: "internal/charge/handler.go".into(),
                service_id: Some("payments".into()),
                service_folder: Some("../harbor-payments".into()),
                agents: vec!["service-payments".into()],
                reads: 1,
                failed: true,
                order: 2,
                first_step: 3,
                last_step: 3,
                ..FileTouchEntry::default()
            },
        ];

        let districts = files_map::group_districts(&entries, "edited", true, true, true);
        let labels: Vec<_> = districts.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "orders · services/orders",
                "__system__ · .",
                "payments · ../harbor-payments"
            ]
        );
        assert_eq!(districts[0].blocks[0].name, "src/Domain");
        let order_chip = &districts[0].blocks[0].files[0];
        assert_eq!(order_chip.mark, "+");
        assert_eq!(order_chip.times, "×2");
        assert_eq!(order_chip.agent_badge, "orders+verify");
        assert!(order_chip.edited);
        assert!(order_chip.now);
        assert!(!order_chip.dim);
        // Unedited files are dimmed under the "edited" filter while remaining visible in place.
        assert!(districts[1].blocks[0].files[0].dim);
        assert_eq!(districts[2].blocks[0].files[0].mark, "!");

        // Single-agent mode (group_by_service = false) groups by top folders 1:1 with Run Files map.
        let single_districts = files_map::group_districts(&entries[..1], "all", true, false, false);
        assert_eq!(single_districts[0].label, "src/");
        assert_eq!(single_districts[0].blocks[0].name, "Domain");

        let html = files_map::render_files_surface(
            &entries,
            "map",
            "all",
            true,
            &files_map::FilesSurfaceOptions {
                group_by_service: true,
                show_agent_badge: true,
                chip_action: "activity-file".into(),
                view_action: "activity-files-view".into(),
                filter_action: "activity-file-filter".into(),
                reveal_action: "activity-reveal-event".into(),
                inspect_graph_action: "inspect-system-file-graph".into(),
                root_id: Some("system-agents".into()),
            },
        );
        assert!(html.contains("orders · services/orders"));
        assert!(html.contains("data-now=\"true\""));
        assert!(html.contains("orders+verify"));
        assert!(html.contains("Edited 1"));
        assert!(html.contains("Failed 1"));
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;

    #[test]
    fn the_clock_tracks_only_an_active_unfinished_native_run() {
        let mut run =
            json!({"started":"2026-11-03T12:00:00Z","finished":false,"kinds":["tool","text"]});
        let active = json!({"status":"running"});
        assert_eq!(
            clock_state(&run, &active),
            Some(RunClock {
                started: Some("2026-11-03T12:00:00Z".into()),
                running: true,
            })
        );
        for status in ["completed", "failed", "cancelled", "interrupted"] {
            assert!(
                !clock_state(&run, &json!({"status":status}))
                    .unwrap()
                    .running
            );
        }
        run["finished"] = json!(true);
        assert!(!clock_state(&run, &active).unwrap().running);
        run["started"] = V::Null;
        assert!(clock_state(&run, &active).unwrap().started.is_none());
    }

    #[test]
    fn missing_and_managed_runs_do_not_keep_a_clock_timer() {
        let active = json!({"status":"running"});
        assert!(clock_state(&V::Null, &active).is_none());
        for kind in [
            "fleet_stage",
            "fleet_reviewer",
            "clash_turn",
            "agent",
            "agent_activity",
        ] {
            assert!(clock_state(&json!({"kinds":["tool",kind]}), &active).is_none());
        }
    }

    #[test]
    fn run_view_opens_for_active_sessions_or_nonempty_native_runs() {
        assert!(!run_view_open(&V::Null, &json!({"status": "completed"})));
        assert!(run_view_open(&V::Null, &json!({"status": "running"})));
        let empty = initial();
        assert!(!run_view_open(&empty, &json!({"status": "completed"})));
        assert!(run_view_open(&empty, &json!({"status": "running"})));
        let mut with_step = empty.clone();
        with_step["steps"] = json!(1);
        with_step["kinds"] = json!(["tool"]);
        assert!(run_view_open(&with_step, &json!({"status": "completed"})));
        let mut managed = with_step;
        managed["kinds"] = json!(["fleet_stage"]);
        assert!(!run_view_open(&managed, &json!({"status": "completed"})));
    }
}
