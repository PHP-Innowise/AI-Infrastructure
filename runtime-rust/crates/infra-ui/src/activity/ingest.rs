use crate::{
    api, classify, dom, model as m,
    run::files_map::{
        self, normalize_view, offered_filters, tidy_path, FileCardAction, FileTouchEntry,
    },
};
use serde_json::{json, Value as V};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, VecDeque},
};

pub(super) const CARD_ID: &str = "run-file-card";

#[derive(Clone, Debug)]
pub(super) struct AgentCallState {
    pub(super) open: bool,
    pub(super) ok: bool,
    pub(super) settled: bool,
    pub(super) touched: BTreeMap<String, &'static str>,
}

#[derive(Clone, Debug)]
pub(super) struct AgentFileState {
    pub(super) key: String,
    pub(super) path: String,
    pub(super) service: Option<String>,
    pub(super) reads: u64,
    pub(super) edits: u64,
    pub(super) created: bool,
    pub(super) deleted: bool,
    pub(super) failed: bool,
    pub(super) outside: bool,
    pub(super) helper: bool,
    pub(super) main: bool,
    pub(super) order: usize,
    pub(super) first_step: u64,
    pub(super) last_step: u64,
    pub(super) event_id: V,
}

#[derive(Clone, Debug)]
pub(super) struct AgentRecord {
    pub(super) meta: V,
    pub(super) events: Vec<V>,
    pub(super) calls: BTreeMap<String, usize>,
    pub(super) call_states: BTreeMap<String, AgentCallState>,
    pub(super) uncalled: BTreeMap<String, VecDeque<String>>,
    pub(super) files: BTreeMap<String, AgentFileState>,
    pub(super) searches: Vec<V>,
    pub(super) plan_items: Vec<V>,
    pub(super) commands: Vec<V>,
    pub(super) from_evidence: bool,
}

impl AgentRecord {
    fn new(key: String, name: &str, attempt: u64, default_svc: String) -> Self {
        Self {
            meta: json!({
                "id": key,
                "agent": name,
                "attempt": attempt,
                "service": if default_svc.is_empty() { V::Null } else { json!(default_svc) },
                "status": "running",
                "tools": 0,
                "messages": 0,
                "tokens": 0,
            }),
            events: Vec::new(),
            calls: BTreeMap::new(),
            call_states: BTreeMap::new(),
            uncalled: BTreeMap::new(),
            files: BTreeMap::new(),
            searches: Vec::new(),
            plan_items: Vec::new(),
            commands: Vec::new(),
            from_evidence: false,
        }
    }
}

#[derive(Clone)]
pub(super) struct Panel {
    pub(super) sid: String,
    pub(super) run_id: String,
    pub(super) cursor: u64,
    pub(super) step_seq: u64,
    pub(super) epoch: u64,
    pub(super) pending: bool,
    pub(super) plan: Vec<V>,
    pub(super) agents: BTreeMap<String, AgentRecord>,
    pub(super) order: Vec<String>,
    pub(super) selected: String,
    pub(super) follow: bool,
    pub(super) active: bool,
    pub(super) tab: String,
    pub(super) files_view: String,
    pub(super) file_filter: String,
    pub(super) cmd_filter: String,
    pub(super) card_key: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            sid: String::new(),
            run_id: String::new(),
            cursor: 0,
            step_seq: 0,
            epoch: 0,
            pending: false,
            plan: Vec::new(),
            agents: BTreeMap::new(),
            order: Vec::new(),
            selected: String::new(),
            follow: true,
            active: false,
            tab: "timeline".into(),
            files_view: "map".into(),
            file_filter: "all".into(),
            cmd_filter: "all".into(),
            card_key: None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InspectorSnapshot {
    pub items: Vec<V>,
    pub show_all_card: bool,
    pub selected: String,
    pub tab: String,
    pub files_view: String,
    pub file_filter: String,
    pub cmd_filter: String,
    pub running: bool,
    pub timeline_events: Vec<V>,
    pub files: Vec<FileTouchEntry>,
    pub plan_items: Vec<V>,
    pub commands: Vec<V>,
}

thread_local! {
    pub(super) static PANELS: RefCell<BTreeMap<String, Panel>> = const { RefCell::new(BTreeMap::new()) };
    static EVIDENCE: RefCell<(String, BTreeMap<String, V>)> =
        const { RefCell::new((String::new(), BTreeMap::new())) };
}

pub(super) fn edit<R>(root: &str, f: impl FnOnce(&mut Panel) -> R) -> R {
    PANELS.with(|all| f(all.borrow_mut().entry(root.into()).or_default()))
}

pub(super) fn panel(root: &str) -> Panel {
    edit(root, |p| p.clone())
}

pub(super) fn owner_tag(root: &str) -> String {
    format!("activity:{root}")
}

pub(super) fn default_service_for_agent(agent_name: &str) -> String {
    if let Some(svc) = agent_name.strip_prefix("service-") {
        if !svc.is_empty() {
            return svc.to_owned();
        }
    }
    if matches!(agent_name, "contracts" | "verify") {
        return "__system__".into();
    }
    String::new()
}

pub(super) fn clean_service_id(raw: Option<&str>, fallback_agent: &str) -> Option<String> {
    let candidate = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| default_service_for_agent(fallback_agent));
    if candidate.is_empty() {
        None
    } else {
        Some(candidate)
    }
}

pub(super) fn shorten_folder(raw_folder: &str, system_root: &str) -> String {
    let folder = tidy_path(raw_folder.trim())
        .trim_end_matches('/')
        .to_owned();
    if folder.is_empty() {
        return String::new();
    }
    if !folder.starts_with('/') && !folder.chars().nth(1).is_some_and(|c| c == ':') {
        return folder;
    }
    let sys = tidy_path(system_root.trim())
        .trim_end_matches('/')
        .to_owned();
    if !sys.is_empty() {
        if folder == sys {
            return ".".into();
        }
        if let Some(rel) = folder.strip_prefix(&format!("{sys}/")) {
            return rel.to_owned();
        }
        let sys_parts: Vec<&str> = sys.split('/').filter(|s| !s.is_empty()).collect();
        let fld_parts: Vec<&str> = folder.split('/').filter(|s| !s.is_empty()).collect();
        let common = sys_parts
            .iter()
            .zip(fld_parts.iter())
            .take_while(|(a, b)| a == b)
            .count();
        if common > 0 && sys_parts.len().saturating_sub(common) <= 2 && common < fld_parts.len() {
            return fld_parts[common..].join("/");
        }
    }
    let parts: Vec<&str> = folder.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        parts.join("/")
    } else {
        parts[parts.len() - 2..].join("/")
    }
}

pub(super) fn service_folders_map_with_catalog(
    p: &Panel,
    evidence: Option<&V>,
    catalog_wrap: &V,
) -> BTreeMap<String, String> {
    let catalog = if catalog_wrap["catalog"].is_object() {
        &catalog_wrap["catalog"]
    } else {
        catalog_wrap
    };
    let mut system_root = m::text(catalog, "root");
    if system_root.is_empty() {
        system_root = m::text(&catalog_wrap["editor"], "root");
    }
    let mut out = BTreeMap::new();
    out.insert("__system__".into(), ".".into());

    for svc in m::array(&catalog["services"]) {
        let id = m::text(&svc, "id");
        if id.is_empty() {
            continue;
        }
        let raw = svc["declared"]["path"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| svc["path"].as_str().filter(|s| !s.is_empty()))
            .or_else(|| svc["root"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("");
        let short = shorten_folder(raw, &system_root);
        if !short.is_empty() {
            out.insert(id, short);
        }
    }
    for svc in m::array(&catalog_wrap["editor"]["services"]) {
        let id = m::text(&svc, "id");
        let path = m::text(&svc, "path");
        if !id.is_empty() && !path.is_empty() {
            out.entry(id)
                .or_insert_with(|| shorten_folder(&path, &system_root));
        }
    }
    for record in p.agents.values() {
        let a = &record.meta;
        let sid = m::text(a, "service");
        let root = m::text(a, "root");
        if sid == "__system__" && system_root.is_empty() && !root.is_empty() {
            system_root = root.clone();
        }
        if !sid.is_empty() && !root.is_empty() {
            let short = shorten_folder(&root, &system_root);
            if !short.is_empty() {
                out.entry(sid).or_insert(short);
            }
        }
    }
    if let Some(ev) = evidence {
        let (dispatches, _) = extract_dispatch_evidences(ev);
        for d in &dispatches {
            let phase = dispatch_phase(d);
            let sid = clean_service_id(d["service"].as_str(), &phase).unwrap_or_default();
            let root = m::text(d, "root");
            if sid == "__system__" && system_root.is_empty() && !root.is_empty() {
                system_root = root.clone();
            }
            if !sid.is_empty() && !root.is_empty() {
                let short = shorten_folder(&root, &system_root);
                if !short.is_empty() {
                    out.entry(sid).or_insert(short);
                }
            }
        }
    }
    out
}

pub(super) fn active_evidence_for_with_run(root: &str, p: &Panel, sys_run: &V) -> Option<V> {
    if root != "system-agents" {
        return None;
    }
    let sys_run_id = m::text(sys_run, "id");
    let from_store = EVIDENCE.with(|cell| {
        let (latest_id, map) = &*cell.borrow();
        if !p.run_id.is_empty() {
            if let Some(v) = map.get(&p.run_id) {
                return Some(v.clone());
            }
        }
        if !sys_run_id.is_empty() {
            if let Some(v) = map.get(&sys_run_id) {
                return Some(v.clone());
            }
        }
        if !latest_id.is_empty() {
            return map.get(latest_id).cloned();
        }
        None
    });
    if from_store.is_some() {
        return from_store;
    }
    if sys_run.is_object() && (!sys_run["evidence"].is_null() || sys_run["receipts"].is_array()) {
        return Some(sys_run.clone());
    }
    None
}

pub(super) fn render_frame_key(root: &str) -> String {
    format!("activity-render:{root}")
}

pub fn reset(root: &str) {
    dom::cancel_frame(&render_frame_key(root));
    super::render::hide_card_for_root(root, false);
    if root == "system-agents" {
        EVIDENCE.with(|cell| {
            let mut guard = cell.borrow_mut();
            guard.0.clear();
            guard.1.clear();
        });
    }
    edit(root, |p| {
        let epoch = p.epoch + 1;
        *p = Panel {
            follow: true,
            epoch,
            ..Panel::default()
        };
    });
    super::render::render(root);
}

pub fn set_plan(root: &str, steps: Vec<V>, active: bool) {
    dom::cancel_frame(&render_frame_key(root));
    edit(root, |p| {
        p.plan = steps;
        p.active = active;
    });
    super::render::render(root);
}

#[allow(dead_code)]
pub(crate) fn set_run_evidence(run_id: &str, evidence: &V) {
    dom::cancel_frame(&render_frame_key("system-agents"));
    let rid = run_id.trim().to_owned();
    EVIDENCE.with(|cell| {
        let mut guard = cell.borrow_mut();
        if evidence.is_null() {
            guard.1.remove(&rid);
            if guard.0 == rid {
                guard.0.clear();
            }
        } else {
            guard.0 = rid.clone();
            guard.1.insert(rid.clone(), evidence.clone());
        }
    });
    edit("system-agents", |p| {
        if p.run_id != rid {
            p.run_id = rid;
        }
    });
    super::render::render("system-agents");
}

fn tool_display_name(v: &V) -> String {
    let raw = v["tool"].as_str().unwrap_or_else(|| {
        v["text"]
            .as_str()
            .unwrap_or("")
            .split(": ")
            .next()
            .unwrap_or("Tool")
    });
    let trimmed = raw.strip_suffix("ToolCall").unwrap_or(raw);
    match trimmed {
        "command_execution" => "Shell",
        "file_change" => "Edit",
        "mcp_tool_call" => "MCP",
        "web_search" => "Web search",
        _ => trimmed,
    }
    .into()
}

fn tool_starts(v: &V) -> bool {
    if let Some(state) = v["state"].as_str() {
        return state == "started" || state == "in_progress";
    }
    let text = m::text(v, "text");
    let tail = text.split_once(": ").map(|(_, s)| s).unwrap_or("");
    !["completed", "failed", "declined"]
        .iter()
        .any(|p| tail.starts_with(p))
}

fn paired_call_id(agent: &mut AgentRecord, key: &str, start: bool, event_id: &str) -> String {
    let queue = agent.uncalled.entry(key.to_owned()).or_default();
    let call = if start {
        let generated = format!("anonymous-{event_id}");
        queue.push_back(generated.clone());
        generated
    } else {
        queue
            .pop_front()
            .unwrap_or_else(|| format!("anonymous-{event_id}"))
    };
    call
}

#[derive(Clone, Debug)]
pub(super) struct NamedTarget {
    service_id: Option<String>,
    path: String,
    how: &'static str,
    outside: bool,
}

pub(super) fn parse_shown_location(
    raw: &str,
    default_service: Option<&str>,
) -> (Option<String>, String, bool) {
    let trimmed = raw.trim();
    let (core, outside) = match trimmed.strip_suffix(" (outside the project)") {
        Some(prefix) => (prefix.trim(), true),
        None => (trimmed, false),
    };
    if outside {
        return (None, tidy_path(core), true);
    }
    if let Some((svc, rest)) = core.split_once(':') {
        let valid_svc =
            !svc.is_empty() && !svc.contains('/') && !svc.contains('\\') && svc.len() > 1;
        if valid_svc {
            return (Some(svc.to_owned()), tidy_path(rest), false);
        }
    }
    (
        default_service.filter(|s| !s.is_empty()).map(str::to_owned),
        tidy_path(core),
        false,
    )
}

fn extract_named_targets(
    raw_tool: &str,
    normalized_key: &str,
    event: &V,
    agent_service: Option<&str>,
) -> Vec<NamedTarget> {
    let event_service = event["service"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(agent_service);
    let mut targets = Vec::new();

    let changes = m::array(&event["changes"]);
    if !changes.is_empty() {
        for change in &changes {
            let how = match change["kind"].as_str() {
                Some("add" | "create" | "created") => "add",
                Some("delete" | "remove" | "deleted") => "delete",
                _ => "edit",
            };
            let outside = m::yes(&change["outside"]);
            let svc = if outside {
                None
            } else {
                change["service"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or(event_service)
                    .map(str::to_owned)
            };
            let path = tidy_path(&m::text(change, "path"));
            if !path.is_empty() && path != "." {
                targets.push(NamedTarget {
                    service_id: svc,
                    path,
                    how,
                    outside,
                });
            }
        }
        return targets;
    }

    if raw_tool == "file_change" {
        let detail = m::text(event, "detail");
        if !detail.is_empty() {
            for part in detail.split(", ") {
                let part = part.trim();
                let Some((verb, loc)) = part.split_once(' ') else {
                    continue;
                };
                let how = match verb {
                    "add" | "create" => "add",
                    "delete" | "remove" => "delete",
                    "update" | "change" | "edit" | "modify" => "edit",
                    _ => continue,
                };
                let (svc, path, outside) = parse_shown_location(loc, event_service);
                if !path.is_empty() && path != "." {
                    targets.push(NamedTarget {
                        service_id: svc,
                        path,
                        how,
                        outside,
                    });
                }
            }
            if !targets.is_empty() {
                return targets;
            }
        }
    }

    for extra_field in ["opened", "shell_reads"] {
        for opened in m::array(&event[extra_field]) {
            let (svc, path, outside) = if let Some(s) = opened.as_str() {
                parse_shown_location(s, event_service)
            } else {
                let outside = m::yes(&opened["outside"]);
                let svc = if outside {
                    None
                } else {
                    opened["service"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .or(event_service)
                        .map(str::to_owned)
                };
                (svc, tidy_path(&m::text(&opened, "path")), outside)
            };
            if !path.is_empty() && path != "." {
                targets.push(NamedTarget {
                    service_id: svc,
                    path,
                    how: "read",
                    outside,
                });
            }
        }
    }

    let how = match normalized_key {
        "read" | "read_file" | "notebookread" | "view" => "read",
        "delete" => "delete",
        "edit" | "write" | "multiedit" | "notebookedit" | "apply_patch" | "applypatch" => "edit",
        _ => return targets,
    };

    let raw_path = event["path"]
        .as_str()
        .or_else(|| event["detail"].as_str())
        .unwrap_or("");
    let path = tidy_path(raw_path);
    if !path.is_empty() && path != "." {
        let outside = m::yes(&event["outside"]);
        targets.push(NamedTarget {
            service_id: if outside {
                None
            } else {
                event_service.map(str::to_owned)
            },
            path,
            how,
            outside,
        });
    }
    targets
}

fn file_map_key(service_id: Option<&str>, path: &str, outside: bool) -> String {
    if outside {
        format!("{}:{path}", files_map::OUTSIDE)
    } else {
        match service_id.filter(|s| !s.is_empty()) {
            Some(sid) => format!("{sid}:{path}"),
            None => path.to_owned(),
        }
    }
}

pub(super) fn touch_agent_files(
    agent: &mut AgentRecord,
    call: &mut AgentCallState,
    targets: &[NamedTarget],
    step: u64,
    event_id: &V,
    helper: bool,
) {
    if targets.is_empty() {
        return;
    }

    for target in targets {
        let fkey = file_map_key(target.service_id.as_deref(), &target.path, target.outside);
        if call.touched.contains_key(&fkey) {
            continue;
        }
        call.touched.insert(fkey.clone(), target.how);
        let next_order = agent.files.len();
        let file = agent
            .files
            .entry(fkey.clone())
            .or_insert_with(|| AgentFileState {
                key: fkey,
                path: target.path.clone(),
                service: target.service_id.clone(),
                reads: 0,
                edits: 0,
                created: false,
                deleted: false,
                failed: false,
                outside: target.outside,
                helper: false,
                main: false,
                order: next_order,
                first_step: step,
                last_step: step,
                event_id: event_id.clone(),
            });
        if target.how == "read" {
            file.reads += 1;
        }
        if helper {
            file.helper = true;
        } else {
            file.main = true;
        }
        file.last_step = step;
        if !event_id.is_null() {
            file.event_id = event_id.clone();
        }
    }
}

pub(super) fn settle_agent_files(agent: &mut AgentRecord, call: &mut AgentCallState, event: &V) {
    if call.settled {
        return;
    }
    call.settled = true;
    let ok = call.ok;
    for (fkey, how) in &call.touched {
        let Some(file) = agent.files.get_mut(fkey) else {
            continue;
        };
        if !ok {
            if event["outcome"] != "not_run" {
                file.failed = true;
            }
            continue;
        }
        match *how {
            "read" => continue,
            "add" => file.created = true,
            "delete" => file.deleted = true,
            _ => {}
        }
        file.edits += 1;
        if !event["id"].is_null() {
            file.event_id = event["id"].clone();
        }
    }
}

pub(super) fn observe_panel(p: &mut Panel, event: &V) {
    let Some(name) = event["agent"].as_str() else {
        return;
    };
    let Some(attempt) = event["attempt"].as_u64() else {
        return;
    };
    let key = format!("{name}#{attempt}");
    if !p.agents.contains_key(&key) {
        p.order.push(key.clone());
        let default_svc = default_service_for_agent(name);
        p.agents.insert(
            key.clone(),
            AgentRecord::new(key.clone(), name, attempt, default_svc),
        );
    }

    let next_step_seq = p.step_seq + 1;
    let event_id = if event["id"].is_null() {
        json!(format!("seq-{next_step_seq}"))
    } else {
        event["id"].clone()
    };

    let agent = p.agents.get_mut(&key).expect("inserted agent");
    match event["kind"].as_str() {
        Some("agent") => {
            let fields = if event["status"] == "running" {
                &[
                    "status",
                    "service",
                    "mode",
                    "provider",
                    "root",
                    "readable",
                    "writable",
                    "readable_count",
                    "writable_count",
                ][..]
            } else {
                &[
                    "status",
                    "ok",
                    "error",
                    "duration_seconds",
                    "summary",
                    "changed_files",
                    "changed_files_count",
                    "checks",
                    "reason",
                    "cost_usd",
                ][..]
            };
            for field in fields {
                if !event[*field].is_null() {
                    agent.meta[*field] = event[*field].clone();
                }
            }
            if event["status"] == "running" {
                agent.meta["started"] = event["at"].clone();
            } else {
                agent.meta["finished"] = event["at"].clone();
                if event
                    .as_object()
                    .map(|o| o.contains_key("tokens"))
                    .unwrap_or(false)
                {
                    agent.meta["tokens"] = event["tokens"].clone();
                }
                for val in agent.call_states.values_mut() {
                    val.open = false;
                }
                let default_svc = clean_service_id(agent.meta["service"].as_str(), name);
                for changed in m::array(&agent.meta["changed_files"]) {
                    let raw = changed
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| m::text(&changed, "path"));
                    let (svc, path, outside) = parse_shown_location(&raw, default_svc.as_deref());
                    if path.is_empty() || path == "." {
                        continue;
                    }
                    let fkey = file_map_key(svc.as_deref(), &path, outside);
                    let entry = agent
                        .files
                        .entry(fkey.clone())
                        .or_insert_with(|| AgentFileState {
                            key: fkey,
                            path,
                            service: svc,
                            reads: 0,
                            edits: 1,
                            created: false,
                            deleted: false,
                            failed: false,
                            outside,
                            helper: false,
                            main: true,
                            order: 10_000,
                            first_step: p.step_seq,
                            last_step: p.step_seq,
                            event_id: event_id.clone(),
                        });
                    if entry.edits == 0 {
                        entry.edits = 1;
                    }
                }
            }
        }
        Some("usage") => {
            if agent.meta["finished"].is_null() {
                let count = [
                    "input_tokens",
                    "output_tokens",
                    "cache_read_input_tokens",
                    "cache_creation_input_tokens",
                ]
                .iter()
                .filter_map(|k| event[*k].as_f64())
                .sum::<f64>();
                agent.meta["tokens"] = json!(agent.meta["tokens"].as_f64().unwrap_or(0.) + count);
            }
        }
        Some("agent_activity") => {
            let kind = m::text(event, "type");
            let raw_call = m::text(event, "call");

            if kind == "tool" {
                let raw_tool = event["tool"].as_str().unwrap_or("");
                let display_tool = tool_display_name(event);
                let norm_key = display_tool.to_lowercase();
                let start = tool_starts(event);
                let call_id = if !raw_call.is_empty() {
                    raw_call.clone()
                } else {
                    paired_call_id(agent, &norm_key, start, &m::string(&event_id))
                };
                if start {
                    p.step_seq += 1;
                } else if p.step_seq == 0 {
                    p.step_seq = 1;
                }
                let step = p.step_seq;
                let agent_svc = clean_service_id(agent.meta["service"].as_str(), name);
                let targets =
                    extract_named_targets(raw_tool, &norm_key, event, agent_svc.as_deref());
                let helper = event["parent"].as_str().is_some_and(|s| !s.is_empty());

                let mut call_obj =
                    agent
                        .call_states
                        .remove(&call_id)
                        .unwrap_or_else(|| AgentCallState {
                            open: start,
                            ok: true,
                            settled: false,
                            touched: BTreeMap::new(),
                        });
                call_obj.open = start;
                call_obj.ok = event["ok"]
                    .as_bool()
                    .unwrap_or(!m::text(event, "text").contains("failed"));

                touch_agent_files(agent, &mut call_obj, &targets, step, &event_id, helper);
                if !start {
                    settle_agent_files(agent, &mut call_obj, event);
                }
                let call_ok = call_obj.ok;
                agent.call_states.insert(call_id.clone(), call_obj);

                if ["grep", "glob", "ls"].contains(&norm_key.as_str()) && start {
                    let scope_svc = event["service"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .or_else(|| agent_svc.clone());
                    agent.searches.push(json!({
                        "service": scope_svc,
                        "pattern": event["detail"],
                        "scope": event["path"],
                        "event_id": event_id,
                    }));
                }

                if ["bash", "shell"].contains(&norm_key.as_str()) {
                    if let Some(existing) = agent.commands.iter_mut().find(|c| c["call"] == call_id)
                    {
                        existing["running"] = json!(start);
                        existing["ok"] = json!(call_ok);
                        existing["event_id"] = event_id.clone();
                    } else {
                        let cmd_text = event["detail"].as_str().unwrap_or_else(|| {
                            event["text"]
                                .as_str()
                                .unwrap_or("")
                                .split_once(": ")
                                .map(|(_, s)| s)
                                .unwrap_or("")
                        });
                        agent.commands.push(json!({
                            "call": call_id,
                            "agent": name,
                            "agent_key": key,
                            "service": event["service"].as_str().filter(|s| !s.is_empty()).or(agent_svc.as_deref()),
                            "command": cmd_text,
                            "class": classify::classify(cmd_text),
                            "running": start,
                            "ok": call_ok,
                            "event_id": event_id,
                            "at": event["at"],
                        }));
                    }
                }
            } else if kind == "plan" {
                p.step_seq += 1;
                agent.plan_items = event["items"]
                    .as_array()
                    .or_else(|| event["plan"].as_array())
                    .cloned()
                    .unwrap_or_else(|| {
                        let text = m::text(event, "text");
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            text.lines()
                                .map(str::trim)
                                .filter(|l| !l.is_empty())
                                .map(|l| json!({"text": l, "status": "in_progress"}))
                                .collect()
                        }
                    });
            }

            if kind == "tool" && !raw_call.is_empty() && event["state"] == "completed" {
                if let Some(&index) = agent.calls.get(&raw_call) {
                    if let Some(item) = agent.events.get_mut(index) {
                        item["state"] = json!("completed");
                        item["ok"] = event["ok"].clone();
                        item["done"] = event["at"].clone();
                        return;
                    }
                }
            }
            if kind == "tool" {
                agent.meta["tools"] = json!(agent.meta["tools"].as_u64().unwrap_or(0) + 1);
                if !raw_call.is_empty() {
                    agent.calls.insert(raw_call, agent.events.len());
                }
            }
            if kind == "text" {
                agent.meta["messages"] = json!(agent.meta["messages"].as_u64().unwrap_or(0) + 1);
            }
            let mut stored = event.clone();
            stored["event_id"] = event_id;
            stored["event_seq"] = json!(p.cursor.max(p.step_seq));
            stored["agent_key"] = json!(key);
            agent.events.push(stored);
            agent.meta["last"] = event.clone();
        }
        _ => {}
    }
}

fn observe(root: &str, event: &V) {
    edit(root, |p| observe_panel(p, event));
}

pub fn observe_session(sid: &str, event: &V) {
    let roots = PANELS.with(|all| {
        all.borrow()
            .iter()
            .filter(|(_, p)| p.sid == sid)
            .map(|(r, _)| r.clone())
            .collect::<Vec<_>>()
    });
    for root in roots {
        let id = event["id"].as_u64().unwrap_or(0);
        let accept = edit(&root, |p| {
            if id == 0 || id <= p.cursor {
                false
            } else {
                p.cursor = id;
                true
            }
        });
        if accept {
            observe(&root, event);
            let root_for_frame = root.clone();
            dom::schedule_frame(&render_frame_key(&root), move || {
                super::render::render(&root_for_frame);
            });
        }
    }
}

pub async fn load(root: &str, sid: &str) {
    if sid.is_empty() {
        let sys_run = if root == "system-agents" {
            m::data("system-run")
        } else {
            V::Null
        };
        let has_plan_or_evidence = edit(root, |p| {
            let has_ev = active_evidence_for_with_run(root, p, &sys_run).is_some();
            if has_ev || !p.plan.is_empty() {
                p.sid.clear();
                p.cursor = 0;
                p.step_seq = 0;
                p.pending = false;
                p.agents.clear();
                p.order.clear();
                true
            } else {
                false
            }
        });
        if has_plan_or_evidence {
            super::render::render(root);
        } else {
            reset(root);
        }
        return;
    }
    let epoch = edit(root, |p| {
        if p.sid != sid {
            let epoch = p.epoch + 1;
            let plan = p.plan.clone();
            let active = p.active;
            let run_id = p.run_id.clone();
            *p = Panel {
                sid: sid.into(),
                run_id,
                plan,
                active,
                follow: true,
                epoch,
                ..Panel::default()
            };
        }
        if p.pending {
            None
        } else {
            p.pending = true;
            Some(p.epoch)
        }
    });
    let Some(epoch) = epoch else { return };
    super::render::render(root);
    for _ in 0..40 {
        let p = panel(root);
        if p.epoch != epoch || p.sid != sid {
            break;
        }
        match api::get(&format!(
            "/api/sessions/{}?after={}",
            dom::enc(sid),
            p.cursor
        ))
        .await
        {
            Ok(data) => {
                if panel(root).epoch != epoch {
                    break;
                }
                let events = m::array(&data["events"]);
                let mut page_updated = false;
                for event in &events {
                    let id = event["id"].as_u64().unwrap_or(0);
                    let accept = edit(root, |p| {
                        if id <= p.cursor {
                            false
                        } else {
                            p.cursor = id;
                            true
                        }
                    });
                    if accept {
                        observe(root, event);
                        page_updated = true;
                    }
                }
                if events.len() < 250 {
                    break;
                }
                if page_updated {
                    let root_for_frame = root.to_owned();
                    dom::schedule_frame(&render_frame_key(root), move || {
                        super::render::render(&root_for_frame);
                    });
                }
            }
            Err(error) => {
                if panel(root).epoch == epoch {
                    if let Some(host) = dom::get(root) {
                        dom::add(
                            &host,
                            &dom::el(
                                "p",
                                "agent-error",
                                &format!("Agent activity is unavailable: {}", error.message),
                            ),
                        );
                    }
                }
                break;
            }
        }
    }
    if panel(root).epoch == epoch {
        dom::cancel_frame(&render_frame_key(root));
        edit(root, |p| p.pending = false);
        super::render::render(root);
    }
}

pub(super) fn dispatch_phase(d: &V) -> String {
    d["phase"]
        .as_str()
        .or_else(|| d["agent"].as_str())
        .or_else(|| d["id"].as_str())
        .unwrap_or("")
        .to_owned()
}

pub(super) fn dispatch_key(d: &V) -> Option<String> {
    let phase = dispatch_phase(d);
    if phase.is_empty() {
        return None;
    }
    let attempt = d["attempt"].as_u64().unwrap_or(1);
    Some(format!("{phase}#{attempt}"))
}

pub(super) fn extract_dispatch_evidences(raw: &V) -> (Vec<V>, Vec<V>) {
    if raw.is_null() {
        return (Vec::new(), Vec::new());
    }
    let receipts = m::array(&raw["receipts"]);
    let ev = if raw["evidence"].is_object() || raw["evidence"].is_array() {
        &raw["evidence"]
    } else {
        raw
    };

    let mut dispatches = Vec::new();
    if let Some(arr) = ev.as_array() {
        dispatches.extend(arr.iter().cloned());
    } else if ev.is_object() {
        for key in ["dispatches", "steps", "items", "entries"] {
            if let Some(arr) = ev[key].as_array() {
                dispatches.extend(arr.iter().cloned());
                break;
            }
        }
        if dispatches.is_empty() {
            for key in ["by_step", "by_phase", "phases", "agents"] {
                if let Some(map) = ev[key].as_object() {
                    for (k, v) in map {
                        let mut item = v.clone();
                        if item.is_object() && dispatch_phase(&item).is_empty() {
                            let phase = k.split('#').next().unwrap_or(k);
                            item["phase"] = json!(phase);
                        }
                        dispatches.push(item);
                    }
                    break;
                }
            }
        }
        if dispatches.is_empty() && !dispatch_phase(ev).is_empty() {
            dispatches.push(ev.clone());
        } else if dispatches.is_empty() {
            if let Some(map) = ev.as_object() {
                for (k, v) in map {
                    if v.is_object()
                        && (!dispatch_phase(v).is_empty()
                            || k == "contracts"
                            || k == "verify"
                            || k.starts_with("service-"))
                    {
                        let mut item = v.clone();
                        if dispatch_phase(&item).is_empty() {
                            let phase = k.split('#').next().unwrap_or(k);
                            item["phase"] = json!(phase);
                        }
                        dispatches.push(item);
                    }
                }
            }
        }
    }
    (dispatches, receipts)
}

pub(super) fn filter_dispatches_for_launch(dispatches: Vec<V>, sid: &str) -> Vec<V> {
    if sid.is_empty() {
        return dispatches;
    }
    let any_tagged = dispatches.iter().any(|d| {
        d["launch_id"]
            .as_str()
            .or_else(|| d["session_id"].as_str())
            .is_some_and(|s| !s.is_empty())
    });
    if !any_tagged {
        return dispatches;
    }
    dispatches
        .into_iter()
        .filter(|d| {
            let launch = d["launch_id"]
                .as_str()
                .or_else(|| d["session_id"].as_str())
                .unwrap_or("");
            launch.is_empty() || launch == sid
        })
        .collect()
}

pub(super) fn parse_evidence_path_item(
    item: &V,
    default_service: Option<&str>,
) -> Option<(Option<String>, String, bool, &'static str, u64)> {
    if let Some(s) = item.as_str() {
        let (svc, path, outside) = parse_shown_location(s, default_service);
        if path.is_empty() || path == "." {
            return None;
        }
        return Some((svc, path, outside, "touch", 1));
    }
    if let Some(arr) = item.as_array() {
        let raw_path = arr.first().and_then(V::as_str)?;
        let status = arr.get(1).and_then(V::as_str).unwrap_or("M");
        let how = match status {
            "A" | "??" | "add" | "added" | "created" => "add",
            "D" | "delete" | "deleted" => "delete",
            _ => "edit",
        };
        let (svc, path, outside) = parse_shown_location(raw_path, default_service);
        if path.is_empty() || path == "." {
            return None;
        }
        return Some((svc, path, outside, how, 1));
    }
    if item.is_object() {
        let raw_path = item["path"]
            .as_str()
            .or_else(|| item["file"].as_str())
            .unwrap_or("");
        if raw_path.is_empty() {
            return None;
        }
        let outside = m::yes(&item["outside"]);
        let explicit_svc = item["service"]
            .as_str()
            .or_else(|| item["service_id"].as_str())
            .filter(|s| !s.is_empty());
        let (svc, path, parsed_outside) =
            parse_shown_location(raw_path, explicit_svc.or(default_service));
        if path.is_empty() || path == "." {
            return None;
        }
        let how = match item["status"]
            .as_str()
            .or_else(|| item["kind"].as_str())
            .unwrap_or("")
        {
            "A" | "??" | "add" | "added" | "create" | "created" => "add",
            "D" | "delete" | "deleted" | "remove" => "delete",
            _ => "touch",
        };
        let count = item["reads"]
            .as_u64()
            .or_else(|| item["count"].as_u64())
            .or_else(|| item["edits"].as_u64())
            .unwrap_or(1)
            .max(1);
        return Some((
            if outside || parsed_outside { None } else { svc },
            path,
            outside || parsed_outside,
            how,
            count,
        ));
    }
    None
}

pub(super) enum AgentView<'a> {
    Live { record: &'a AgentRecord, meta: V },
    Synth(AgentRecord),
}

impl<'a> AgentView<'a> {
    pub(super) fn meta(&self) -> &V {
        match self {
            Self::Live { meta, .. } => meta,
            Self::Synth(record) => &record.meta,
        }
    }

    pub(super) fn events(&self) -> &[V] {
        match self {
            Self::Live { record, .. } => &record.events,
            Self::Synth(record) => &record.events,
        }
    }

    pub(super) fn files(&self) -> &BTreeMap<String, AgentFileState> {
        match self {
            Self::Live { record, .. } => &record.files,
            Self::Synth(record) => &record.files,
        }
    }

    pub(super) fn call_states(&self) -> &BTreeMap<String, AgentCallState> {
        match self {
            Self::Live { record, .. } => &record.call_states,
            Self::Synth(record) => &record.call_states,
        }
    }

    pub(super) fn searches(&self) -> &[V] {
        match self {
            Self::Live { record, .. } => &record.searches,
            Self::Synth(record) => &record.searches,
        }
    }

    pub(super) fn commands(&self) -> &[V] {
        match self {
            Self::Live { record, .. } => &record.commands,
            Self::Synth(record) => &record.commands,
        }
    }

    pub(super) fn plan_items(&self) -> &[V] {
        match self {
            Self::Live { record, .. } => &record.plan_items,
            Self::Synth(record) => &record.plan_items,
        }
    }

    pub(super) fn is_from_evidence(&self) -> bool {
        match self {
            Self::Live { record, .. } => record.from_evidence,
            Self::Synth(record) => record.from_evidence,
        }
    }
}

pub(super) fn synthetic_agent_from_evidence(
    d: &V,
    receipts: &[V],
) -> Option<(String, AgentRecord)> {
    let key = dispatch_key(d)?;
    let phase = dispatch_phase(d);
    let attempt = d["attempt"].as_u64().unwrap_or(1);
    let sid = clean_service_id(d["service"].as_str(), &phase);
    let receipt = receipts
        .iter()
        .find(|r| r["phase"] == phase && r["attempt"].as_u64().unwrap_or(1) == attempt);

    let status = d["status"]
        .as_str()
        .or_else(|| {
            receipt.and_then(|r| {
                if m::yes(&r["ok"]) {
                    Some("completed")
                } else {
                    r["report"]["status"].as_str().or(Some("blocked"))
                }
            })
        })
        .unwrap_or("completed");
    let ok = d["ok"]
        .as_bool()
        .or_else(|| receipt.and_then(|r| r["ok"].as_bool()))
        .unwrap_or(status == "completed");
    let summary = d["summary"]
        .as_str()
        .or_else(|| receipt.and_then(|r| r["report"]["summary"].as_str()))
        .unwrap_or("");
    let changed_files = if d["git_changed"].is_array() {
        d["git_changed"].clone()
    } else if let Some(r) = receipt {
        r["report"]["changed_files"].clone()
    } else {
        json!([])
    };
    let checks = if d["checks"].is_array() {
        d["checks"].clone()
    } else if let Some(r) = receipt {
        r["report"]["checks"].clone()
    } else {
        json!([])
    };
    let tools = d["steps"]
        .as_u64()
        .unwrap_or_else(|| m::array(&d["commands"]).len() as u64);

    let mut rec = AgentRecord::new(key.clone(), &phase, attempt, sid.unwrap_or_default());
    rec.meta["root"] = d["root"].clone();
    rec.meta["mode"] = d["mode"].clone();
    rec.meta["provider"] = d["provider"].clone();
    rec.meta["status"] = json!(status);
    rec.meta["ok"] = json!(ok);
    rec.meta["summary"] = json!(summary);
    rec.meta["changed_files"] = changed_files;
    rec.meta["checks"] = checks;
    rec.meta["tools"] = json!(tools);
    rec.meta["tokens"] = json!(d["tokens"].as_f64().unwrap_or(0.));
    rec.meta["cost_usd"] = d["cost_usd"].clone();
    rec.meta["duration_seconds"] = d["duration_seconds"].clone();
    rec.meta["from_evidence"] = json!(true);
    rec.from_evidence = true;
    Some((key, rec))
}

pub(super) fn effective_agents<'a>(
    p: &'a Panel,
    evidence: Option<&V>,
) -> (BTreeMap<String, AgentView<'a>>, Vec<String>, Vec<V>, Vec<V>) {
    let mut agents: BTreeMap<String, AgentView<'a>> = p
        .agents
        .iter()
        .map(|(k, record)| {
            (
                k.clone(),
                AgentView::Live {
                    record,
                    meta: record.meta.clone(),
                },
            )
        })
        .collect();
    let mut order = p.order.clone();
    let (raw_dispatches, receipts) = evidence.map(extract_dispatch_evidences).unwrap_or_default();
    let dispatches = filter_dispatches_for_launch(raw_dispatches, &p.sid);

    for d in &dispatches {
        if let Some((key, synth)) = synthetic_agent_from_evidence(d, &receipts) {
            if let Some(existing) = agents.get_mut(&key) {
                if let AgentView::Live { meta, .. } = existing {
                    for field in ["service", "root", "summary", "checks"] {
                        if meta[field].is_null() && !synth.meta[field].is_null() {
                            meta[field] = synth.meta[field].clone();
                        }
                    }
                }
            } else {
                order.push(key.clone());
                agents.insert(key, AgentView::Synth(synth));
            }
        }
    }
    (agents, order, dispatches, receipts)
}

pub(super) fn ordered_items(
    p: &Panel,
    agents: &BTreeMap<String, AgentView<'_>>,
    order: &[String],
) -> Vec<V> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for step in &p.plan {
        let name = m::text(step, "id");
        let latest = order
            .iter()
            .rev()
            .filter_map(|id| agents.get(id))
            .find(|a| a.meta()["agent"] == name);
        let mut item = step.clone();
        if !item.is_object() {
            continue;
        }
        if let Some(a) = latest {
            let meta = a.meta();
            for (k, v) in meta.as_object().into_iter().flatten() {
                item[k] = v.clone();
            }
            item["key"] = meta["id"].clone();
            seen.insert(m::text(meta, "id"));
        } else {
            item["key"] = V::Null;
            item["status"] = step["journal"]
                .as_str()
                .map(|s| json!(s))
                .unwrap_or(json!("pending"));
            item["earlier"] = json!(step["journal"] == "completed");
        }
        item["step"] = json!(name);
        if !p.active && step["journal"].is_string() {
            item["status"] = step["journal"].clone();
        }
        result.push(item);
    }
    for key in order {
        let Some(a) = agents.get(key) else {
            continue;
        };
        let meta = a.meta();
        if !seen.contains(key) && !p.plan.iter().any(|s| s["id"] == meta["agent"]) {
            let mut item = meta.clone();
            item["key"] = json!(key);
            item["step"] = meta["agent"].clone();
            item["label"] = meta["agent"].clone();
            result.push(item);
        }
    }
    result
}

pub(super) fn selection(a: &V) -> String {
    a["key"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("step:{}", m::text(a, "step")))
}

pub(super) fn resolve_selection(root: &str, p: &Panel, items: &[V]) -> (bool, String) {
    let show_all_card = root == "system-agents" || items.len() > 1;
    let ran_count = items.iter().filter(|a| a["key"].is_string()).count();
    let running = items
        .iter()
        .rev()
        .find(|a| a["status"] == "running" && a["key"].is_string())
        .map(selection);

    if p.follow {
        let chosen = if let Some(run_sel) = running {
            run_sel
        } else if show_all_card && ran_count > 1 {
            "all".into()
        } else {
            items
                .iter()
                .rev()
                .find(|a| a["key"].is_string())
                .or_else(|| items.first())
                .map(selection)
                .unwrap_or_else(|| {
                    if show_all_card {
                        "all".into()
                    } else {
                        String::new()
                    }
                })
        };
        return (show_all_card, chosen);
    }

    if p.selected == "all" && show_all_card {
        return (show_all_card, "all".into());
    }
    if let Some(step) = p.selected.strip_prefix("step:") {
        if let Some(a) = items
            .iter()
            .find(|a| a["step"] == step && a["key"].is_string())
        {
            return (show_all_card, selection(a));
        }
    }
    if items.iter().any(|a| selection(a) == p.selected) {
        return (show_all_card, p.selected.clone());
    }
    let fallback = if show_all_card {
        "all".into()
    } else {
        items.first().map(selection).unwrap_or_default()
    };
    (show_all_card, fallback)
}

pub(super) fn searched_patterns_for(
    searches: &[V],
    service_id: Option<&str>,
    path: &str,
    outside: bool,
) -> Vec<String> {
    if outside {
        return Vec::new();
    }
    let mut out = Vec::new();
    for s in searches {
        let s_svc = s["service"].as_str().filter(|v| !v.is_empty());
        if let (Some(a), Some(b)) = (s_svc, service_id.filter(|v| !v.is_empty())) {
            if a != b {
                continue;
            }
        }
        let scope = m::text(s, "scope");
        let scope = scope.trim_end_matches('/');
        let matched = scope.is_empty()
            || scope == "."
            || path == scope
            || path.starts_with(&format!("{scope}/"));
        if matched {
            let pat = m::text(s, "pattern");
            if !pat.is_empty() && !out.contains(&pat) {
                out.push(pat);
            }
        }
    }
    out
}

pub(super) fn build_agent_files(
    agent_key: &str,
    agent: &AgentView<'_>,
    dispatch: Option<&V>,
    receipt: Option<&V>,
    folders: &BTreeMap<String, String>,
) -> Vec<FileTouchEntry> {
    let meta = agent.meta();
    let agent_name = m::text(meta, "agent");
    let default_svc = clean_service_id(meta["service"].as_str(), &agent_name);
    let agent_running = meta["status"] == "running";

    let mut open_keys = BTreeSet::new();
    if agent_running {
        for state in agent.call_states().values() {
            if state.open {
                open_keys.extend(state.touched.keys().cloned());
            }
        }
    }

    let mut searches = agent.searches().to_vec();
    if let Some(d) = dispatch {
        for item in m::array(&d["searched"]) {
            if let Some(pat) = item.as_str() {
                searches.push(json!({
                    "service": default_svc,
                    "pattern": pat,
                    "scope": "",
                }));
            } else if item.is_object() {
                let pat = item["pattern"]
                    .as_str()
                    .or_else(|| item["query"].as_str())
                    .or_else(|| item["detail"].as_str())
                    .unwrap_or("");
                let scope = item["scope"]
                    .as_str()
                    .or_else(|| item["path"].as_str())
                    .or_else(|| item["folder"].as_str())
                    .unwrap_or("");
                let svc = item["service"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or(default_svc.as_deref());
                searches.push(json!({
                    "service": svc,
                    "pattern": pat,
                    "scope": scope,
                }));
            }
        }
    }

    let mut by_key: BTreeMap<String, FileTouchEntry> = BTreeMap::new();
    let mut live_files: Vec<&AgentFileState> = agent.files().values().collect();
    live_files.sort_by_key(|f| f.order);

    for (idx, f) in live_files.iter().enumerate() {
        let path = tidy_path(&f.path);
        if path.is_empty() || path == "." {
            continue;
        }
        let outside = f.outside;
        let svc = if outside {
            None
        } else {
            f.service
                .clone()
                .filter(|s| !s.is_empty())
                .or_else(|| default_svc.clone())
        };
        let folder = svc.as_deref().and_then(|s| folders.get(s)).cloned();
        let searched_patterns = searched_patterns_for(&searches, svc.as_deref(), &path, outside);
        let entry = FileTouchEntry {
            path,
            service_id: svc,
            service_folder: folder,
            agents: vec![agent_name.clone()],
            reads: f.reads,
            edits: f.edits,
            created: f.created,
            deleted: f.deleted,
            failed: f.failed,
            outside,
            helper: f.helper,
            main: f.main,
            now: agent_running && open_keys.contains(&f.key),
            order: idx,
            first_step: f.first_step,
            last_step: f.last_step,
            searched_patterns,
            event_id: f.event_id.clone(),
            timeline_agent: Some(agent_key.to_owned()),
        };
        by_key.insert(entry.key(), entry);
    }

    if let Some(d) = dispatch {
        let helper_dispatch = m::yes(&d["helpers"])
            || d["helpers"].as_u64().is_some_and(|n| n > 0)
            || d["helpers"].as_array().is_some_and(|a| !a.is_empty());

        let mut touch_items = m::array(&d["file_touches"]);
        if touch_items.is_empty() {
            if let Some(map) = d["file_touches"].as_object() {
                touch_items.extend(map.values().cloned());
            }
        }
        for ft in &touch_items {
            let raw_path = ft["path"].as_str().unwrap_or("");
            let outside = m::yes(&ft["outside"]);
            let explicit_svc = ft["service"]
                .as_str()
                .or_else(|| ft["service_id"].as_str())
                .filter(|s| !s.is_empty());
            let (svc, path, parsed_outside) =
                parse_shown_location(raw_path, explicit_svc.or(default_svc.as_deref()));
            if path.is_empty() || path == "." {
                continue;
            }
            let outside = outside || parsed_outside;
            let svc = if outside { None } else { svc };
            let folder = svc.as_deref().and_then(|s| folders.get(s)).cloned();
            let next_order = by_key.len();
            let key = file_map_key(svc.as_deref(), &path, outside);
            let searched = searched_patterns_for(&searches, svc.as_deref(), &path, outside);
            let ev_reads = ft["reads"].as_u64().unwrap_or(0);
            let ev_edits = ft["edits"].as_u64().unwrap_or(0);
            let entry = by_key.entry(key).or_insert_with(|| FileTouchEntry {
                path: path.clone(),
                service_id: svc.clone(),
                service_folder: folder,
                agents: vec![agent_name.clone()],
                reads: 0,
                edits: 0,
                created: false,
                deleted: false,
                failed: false,
                outside,
                helper: false,
                main: true,
                now: false,
                order: next_order,
                first_step: ft["first_step"].as_u64().unwrap_or(0),
                last_step: ft["last_step"].as_u64().unwrap_or(0),
                searched_patterns: Vec::new(),
                event_id: json!(format!("ev:{agent_key}:{path}")),
                timeline_agent: Some(agent_key.to_owned()),
            });
            entry.reads = entry.reads.max(ev_reads);
            entry.edits = entry.edits.max(ev_edits);
            entry.created |= m::yes(&ft["created"]);
            entry.deleted |= m::yes(&ft["deleted"]);
            entry.failed |= m::yes(&ft["failed"]);
            entry.helper |= m::yes(&ft["helper"]);
            if ft["main"] == true {
                entry.main = true;
            }
            if entry.first_step == 0 {
                entry.first_step = ft["first_step"].as_u64().unwrap_or(0);
            }
            entry.last_step = entry.last_step.max(ft["last_step"].as_u64().unwrap_or(0));
            for pat in ft["searched_patterns"]
                .as_array()
                .or_else(|| ft["searched"].as_array())
                .into_iter()
                .flatten()
                .filter_map(V::as_str)
            {
                if !pat.is_empty() && !entry.searched_patterns.iter().any(|p| p == pat) {
                    entry.searched_patterns.push(pat.to_owned());
                }
            }
            for pat in searched {
                if !entry.searched_patterns.contains(&pat) {
                    entry.searched_patterns.push(pat);
                }
            }
        }

        type EvReadSlot = (Option<String>, String, bool, u64, Option<V>);
        let mut ev_read_counts: BTreeMap<String, EvReadSlot> = BTreeMap::new();
        for field in ["opened", "shell_reads"] {
            if let Some(obj) = d[field].as_object() {
                for (raw_path, ev_id) in obj {
                    if let Some((svc, path, outside, _, cnt)) =
                        parse_evidence_path_item(&json!(raw_path), default_svc.as_deref())
                    {
                        let key = file_map_key(svc.as_deref(), &path, outside);
                        let slot = ev_read_counts
                            .entry(key)
                            .or_insert((svc, path, outside, 0, None));
                        slot.3 = slot.3.max(cnt);
                        if !ev_id.is_null() {
                            slot.4 = Some(ev_id.clone());
                        }
                    }
                }
            } else {
                for raw_item in m::array(&d[field]) {
                    if let Some((svc, path, outside, _, cnt)) =
                        parse_evidence_path_item(&raw_item, default_svc.as_deref())
                    {
                        let key = file_map_key(svc.as_deref(), &path, outside);
                        let slot = ev_read_counts
                            .entry(key)
                            .or_insert((svc, path, outside, 0, None));
                        slot.3 = slot.3.max(cnt);
                    }
                }
            }
        }
        for (key, (svc, path, outside, min_reads, custom_event_id)) in ev_read_counts {
            let folder = svc.as_deref().and_then(|s| folders.get(s)).cloned();
            let searched = searched_patterns_for(&searches, svc.as_deref(), &path, outside);
            let next_order = by_key.len();
            let entry = by_key.entry(key).or_insert_with(|| FileTouchEntry {
                path: path.clone(),
                service_id: svc,
                service_folder: folder,
                agents: vec![agent_name.clone()],
                reads: 0,
                edits: 0,
                created: false,
                deleted: false,
                failed: false,
                outside,
                helper: helper_dispatch,
                main: !helper_dispatch,
                now: false,
                order: next_order,
                first_step: 0,
                last_step: d["steps"].as_u64().unwrap_or(0),
                searched_patterns: Vec::new(),
                event_id: custom_event_id
                    .clone()
                    .unwrap_or_else(|| json!(format!("ev:{agent_key}:{path}"))),
                timeline_agent: Some(agent_key.to_owned()),
            });
            entry.reads = entry.reads.max(min_reads);
            if let Some(cid) = custom_event_id {
                if entry
                    .event_id
                    .as_str()
                    .is_some_and(|s| s.starts_with("ev:"))
                {
                    entry.event_id = cid;
                }
            }
            for pat in searched {
                if !entry.searched_patterns.contains(&pat) {
                    entry.searched_patterns.push(pat);
                }
            }
        }

        for field in ["ledger_edited", "git_changed"] {
            for raw_item in m::array(&d[field]) {
                if let Some((svc, path, outside, how, cnt)) =
                    parse_evidence_path_item(&raw_item, default_svc.as_deref())
                {
                    let key = file_map_key(svc.as_deref(), &path, outside);
                    let folder = svc.as_deref().and_then(|s| folders.get(s)).cloned();
                    let searched = searched_patterns_for(&searches, svc.as_deref(), &path, outside);
                    let next_order = by_key.len();
                    let entry = by_key.entry(key).or_insert_with(|| FileTouchEntry {
                        path: path.clone(),
                        service_id: svc,
                        service_folder: folder,
                        agents: vec![agent_name.clone()],
                        reads: 0,
                        edits: 0,
                        created: false,
                        deleted: false,
                        failed: false,
                        outside,
                        helper: helper_dispatch,
                        main: !helper_dispatch,
                        now: false,
                        order: next_order,
                        first_step: 0,
                        last_step: d["steps"].as_u64().unwrap_or(0),
                        searched_patterns: Vec::new(),
                        event_id: json!(format!("ev:{agent_key}:{path}")),
                        timeline_agent: Some(agent_key.to_owned()),
                    });
                    entry.edits = entry.edits.max(cnt);
                    if how == "add" {
                        entry.created = true;
                    } else if how == "delete" {
                        entry.deleted = true;
                    }
                    if raw_item.is_object() && m::yes(&raw_item["failed"]) {
                        entry.failed = true;
                    }
                    for pat in searched {
                        if !entry.searched_patterns.contains(&pat) {
                            entry.searched_patterns.push(pat);
                        }
                    }
                }
            }
        }
    }

    if let Some(r) = receipt {
        for raw_item in m::array(&r["report"]["changed_files"]) {
            if let Some((svc, path, outside, how, cnt)) =
                parse_evidence_path_item(&raw_item, default_svc.as_deref())
            {
                let key = file_map_key(svc.as_deref(), &path, outside);
                let folder = svc.as_deref().and_then(|s| folders.get(s)).cloned();
                let next_order = by_key.len();
                let entry = by_key.entry(key).or_insert_with(|| FileTouchEntry {
                    path: path.clone(),
                    service_id: svc,
                    service_folder: folder,
                    agents: vec![agent_name.clone()],
                    reads: 0,
                    edits: 0,
                    created: false,
                    deleted: false,
                    failed: false,
                    outside,
                    helper: false,
                    main: true,
                    now: false,
                    order: next_order,
                    first_step: 0,
                    last_step: 0,
                    searched_patterns: Vec::new(),
                    event_id: json!(format!("ev:{agent_key}:{path}")),
                    timeline_agent: Some(agent_key.to_owned()),
                });
                entry.edits = entry.edits.max(cnt);
                if how == "add" {
                    entry.created = true;
                } else if how == "delete" {
                    entry.deleted = true;
                }
            }
        }
    }

    let mut out: Vec<FileTouchEntry> = by_key.into_values().collect();
    out.sort_by_key(|e| e.order);
    out
}

pub(super) fn merge_multi_agent_files(per_agent: Vec<Vec<FileTouchEntry>>) -> Vec<FileTouchEntry> {
    let mut merged: BTreeMap<String, FileTouchEntry> = BTreeMap::new();
    let mut next_order = 0usize;
    for entries in per_agent {
        for entry in entries {
            let key = entry.key();
            if let Some(existing) = merged.get_mut(&key) {
                for ag in entry.agents {
                    if !existing.agents.contains(&ag) {
                        existing.agents.push(ag);
                    }
                }
                existing.reads += entry.reads;
                existing.edits += entry.edits;
                existing.created |= entry.created;
                existing.deleted |= entry.deleted;
                existing.failed |= entry.failed;
                existing.helper |= entry.helper;
                existing.main |= entry.main;
                existing.now |= entry.now;
                if existing.first_step == 0
                    || (entry.first_step > 0 && entry.first_step < existing.first_step)
                {
                    existing.first_step = entry.first_step;
                }
                existing.last_step = existing.last_step.max(entry.last_step);
                for pat in entry.searched_patterns {
                    if !existing.searched_patterns.contains(&pat) {
                        existing.searched_patterns.push(pat);
                    }
                }
                if existing.service_folder.is_none() && entry.service_folder.is_some() {
                    existing.service_folder = entry.service_folder;
                }
                if !entry.event_id.is_null() {
                    existing.event_id = entry.event_id;
                    existing.timeline_agent = entry.timeline_agent;
                }
            } else {
                let mut fresh = entry;
                fresh.order = next_order;
                next_order += 1;
                merged.insert(key, fresh);
            }
        }
    }
    let mut out: Vec<FileTouchEntry> = merged.into_values().collect();
    out.sort_by_key(|e| e.order);
    out
}

pub(super) fn build_agent_commands(
    agent_key: &str,
    agent: &AgentView<'_>,
    dispatch: Option<&V>,
    receipt: Option<&V>,
) -> Vec<V> {
    let meta = agent.meta();
    let agent_name = m::text(meta, "agent");
    let default_svc = clean_service_id(meta["service"].as_str(), &agent_name);
    let mut list = agent.commands().to_vec();
    let mut seen_cmds: BTreeSet<String> = list
        .iter()
        .map(|c| m::text(c, "command"))
        .filter(|s| !s.is_empty())
        .collect();

    if let Some(d) = dispatch {
        for field in ["command_runs", "shell_events", "commands"] {
            for (idx, item) in m::array(&d[field]).iter().enumerate() {
                let (cmd_str, ok, running) = if let Some(s) = item.as_str() {
                    (s.to_owned(), true, false)
                } else {
                    let s = item["command"]
                        .as_str()
                        .or_else(|| item["detail"].as_str())
                        .or_else(|| item["cmd"].as_str())
                        .unwrap_or("");
                    let state_str = item["status"]
                        .as_str()
                        .or_else(|| item["state"].as_str())
                        .unwrap_or("completed");
                    let running = state_str == "running" || state_str == "started";
                    let ok = item["ok"]
                        .as_bool()
                        .or_else(|| item["exit_code"].as_i64().map(|code| code == 0))
                        .unwrap_or(state_str != "failed");
                    (s.to_owned(), ok, running)
                };
                if cmd_str.is_empty() || seen_cmds.contains(&cmd_str) {
                    continue;
                }
                seen_cmds.insert(cmd_str.clone());
                let class = if item["class"].is_object() {
                    item["class"].clone()
                } else {
                    classify::classify(&cmd_str)
                };
                list.push(json!({
                    "call": format!("ev-cmd-{agent_key}-{idx}"),
                    "agent": agent_name,
                    "agent_key": agent_key,
                    "service": default_svc,
                    "command": cmd_str,
                    "class": class,
                    "running": running,
                    "ok": ok,
                    "source": "evidence",
                }));
            }
        }
    }

    let mut checks = m::array(&meta["checks"]);
    if checks.is_empty() {
        if let Some(r) = receipt {
            checks = m::array(&r["report"]["checks"]);
        }
    }
    for (idx, chk) in checks.iter().enumerate() {
        let cmd_str = chk["command"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| chk["name"].as_str())
            .unwrap_or("");
        if cmd_str.is_empty() || seen_cmds.contains(cmd_str) {
            continue;
        }
        seen_cmds.insert(cmd_str.to_owned());
        let st = chk["status"].as_str().unwrap_or("completed");
        let ok = st == "passed" || st == "completed" || chk["ok"] == true;
        let mut class = classify::classify(cmd_str);
        if class["check"] != true {
            class["check"] = json!(true);
        }
        list.push(json!({
            "call": format!("chk-{agent_key}-{idx}"),
            "agent": agent_name,
            "agent_key": agent_key,
            "service": default_svc,
            "name": chk["name"],
            "command": cmd_str,
            "status": st,
            "summary": chk["summary"],
            "class": class,
            "running": false,
            "ok": ok,
            "not_run": st == "not_run",
            "source": "check",
        }));
    }

    list
}

pub(super) fn build_agent_plan_items(
    agent_key: &str,
    agent: &AgentView<'_>,
    dispatch: Option<&V>,
    step_item: Option<&V>,
    include_step_fallback: bool,
) -> Vec<V> {
    let meta = agent.meta();
    let agent_name = m::text(meta, "agent");
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    for raw in agent.plan_items() {
        let text = raw["text"]
            .as_str()
            .or_else(|| raw["content"].as_str())
            .or_else(|| raw["step"].as_str())
            .or_else(|| raw.as_str())
            .unwrap_or("")
            .trim()
            .to_owned();
        if text.is_empty() || !seen.insert(text.clone()) {
            continue;
        }
        let st = raw["status"].as_str().unwrap_or("in_progress");
        out.push(json!({
            "id": format!("{agent_key}:{text}"),
            "agent": agent_name,
            "status": st,
            "text": text,
        }));
    }

    if let Some(d) = dispatch {
        for raw in d["plan_items"]
            .as_array()
            .or_else(|| d["plan"].as_array())
            .into_iter()
            .flatten()
        {
            let text = raw["text"]
                .as_str()
                .or_else(|| raw["content"].as_str())
                .or_else(|| raw["step"].as_str())
                .or_else(|| raw["label"].as_str())
                .or_else(|| raw.as_str())
                .unwrap_or("")
                .trim()
                .to_owned();
            if text.is_empty() || !seen.insert(text.clone()) {
                continue;
            }
            let st = raw["status"]
                .as_str()
                .unwrap_or_else(|| meta["status"].as_str().unwrap_or("completed"));
            out.push(json!({
                "id": format!("ev:{agent_key}:{text}"),
                "agent": agent_name,
                "status": st,
                "text": text,
            }));
        }
    }

    if out.is_empty() && include_step_fallback {
        if let Some(step) = step_item {
            let goal = m::text(step, "goal");
            let label = m::text(step, "label");
            let text = if !goal.is_empty() {
                format!("{label}: {goal}")
            } else {
                label
            };
            if !text.is_empty() {
                out.push(json!({
                    "id": format!("step:{agent_key}"),
                    "agent": agent_name,
                    "status": step["status"].as_str().unwrap_or("pending"),
                    "text": text,
                }));
            }
        }
    }

    out
}

pub(super) fn build_synthetic_timeline(
    agent_key: &str,
    agent: &AgentView<'_>,
    files: &[FileTouchEntry],
    commands: &[V],
) -> Vec<V> {
    let agent_name = m::text(agent.meta(), "agent");
    let mut out = Vec::new();
    for file in files {
        let tool = if file.created {
            "Create"
        } else if file.deleted {
            "Delete"
        } else if file.edits > 0 {
            "Edit"
        } else {
            "Read"
        };
        out.push(json!({
            "type": "tool",
            "tool": tool,
            "agent": agent_name,
            "agent_key": agent_key,
            "service": file.service_id,
            "path": file.path,
            "state": "completed",
            "ok": !file.failed,
            "event_id": file.event_id,
            "event_seq": file.order as u64,
        }));
    }
    for (idx, cmd) in commands.iter().enumerate() {
        out.push(json!({
            "type": "tool",
            "tool": "Shell",
            "agent": agent_name,
            "agent_key": agent_key,
            "service": cmd["service"],
            "detail": cmd["command"],
            "state": if m::yes(&cmd["running"]) { "started" } else { "completed" },
            "ok": cmd["ok"],
            "event_id": cmd["call"],
            "event_seq": (files.len() + idx) as u64,
        }));
    }
    out
}

pub(crate) fn snapshot_for_root(root: &str) -> InspectorSnapshot {
    let sys_run = if root == "system-agents" {
        m::data("system-run")
    } else {
        V::Null
    };
    let catalog = m::data("system-catalog");
    let (snapshot, updated_selected) = PANELS.with(|all| {
        let mut map = all.borrow_mut();
        let p = map.entry(root.into()).or_default();
        let evidence = active_evidence_for_with_run(root, p, &sys_run);
        let folders = service_folders_map_with_catalog(p, evidence.as_ref(), &catalog);
        let (agents, order, dispatches, receipts) = effective_agents(p, evidence.as_ref());
        let items = ordered_items(p, &agents, &order);
        let (show_all_card, selected) = resolve_selection(root, p, &items);
        let updated_selected = (p.selected != selected).then(|| selected.clone());

        let dispatch_for = |key: &str| -> Option<&V> {
            dispatches
                .iter()
                .find(|d| dispatch_key(d).as_deref() == Some(key))
        };
        let receipt_for = |agent: &AgentView<'_>| -> Option<&V> {
            let meta = agent.meta();
            let phase = m::text(meta, "agent");
            let attempt = meta["attempt"].as_u64().unwrap_or(1);
            receipts
                .iter()
                .find(|r| r["phase"] == phase && r["attempt"].as_u64().unwrap_or(1) == attempt)
        };

        let mut running = false;
        let mut timeline_events = Vec::new();
        let mut files = Vec::new();
        let mut plan_items = Vec::new();
        let mut commands = Vec::new();

        if selected == "all" {
            let mut per_agent_files = Vec::new();
            for step in &items {
                let step_id = m::text(step, "step");
                let label = m::text(step, "label");
                let goal = m::text(step, "goal");
                let summary = if !goal.is_empty() {
                    format!("{label} — {goal}")
                } else {
                    label
                };
                if !summary.is_empty() {
                    plan_items.push(json!({
                        "id": format!("wave:{step_id}"),
                        "agent": step_id,
                        "status": step["status"].as_str().unwrap_or("pending"),
                        "text": summary,
                        "wave_step": true,
                    }));
                }
            }

            for key in &order {
                let Some(agent) = agents.get(key) else {
                    continue;
                };
                if agent.meta()["status"] == "running" {
                    running = true;
                }
                let d = dispatch_for(key);
                let r = receipt_for(agent);
                let ag_files = build_agent_files(key, agent, d, r, &folders);
                let ag_cmds = build_agent_commands(key, agent, d, r);
                let ag_plan = build_agent_plan_items(key, agent, d, None, false);
                plan_items.extend(ag_plan);

                let live_evs = agent.events();
                if live_evs.is_empty() && agent.is_from_evidence() {
                    timeline_events
                        .extend(build_synthetic_timeline(key, agent, &ag_files, &ag_cmds));
                } else {
                    timeline_events.extend(live_evs.iter().cloned());
                }
                per_agent_files.push(ag_files);
                commands.extend(ag_cmds);
            }
            timeline_events.sort_by_key(|ev| ev["event_seq"].as_u64().unwrap_or(0));
            files = merge_multi_agent_files(per_agent_files);
        } else if let Some(agent) = agents.get(&selected) {
            running = agent.meta()["status"] == "running";
            let d = dispatch_for(&selected);
            let r = receipt_for(agent);
            let step_item = items.iter().find(|i| selection(i) == selected);
            files = build_agent_files(&selected, agent, d, r, &folders);
            commands = build_agent_commands(&selected, agent, d, r);
            plan_items = build_agent_plan_items(&selected, agent, d, step_item, true);
            let live_evs = agent.events();
            timeline_events = if live_evs.is_empty() && agent.is_from_evidence() {
                build_synthetic_timeline(&selected, agent, &files, &commands)
            } else {
                live_evs.to_vec()
            };
        } else if let Some(step_item) = items.iter().find(|i| selection(i) == selected) {
            let goal = m::text(step_item, "goal");
            let label = m::text(step_item, "label");
            if !label.is_empty() {
                plan_items.push(json!({
                    "id": format!("step:{selected}"),
                    "agent": m::text(step_item, "step"),
                    "status": step_item["status"].as_str().unwrap_or("pending"),
                    "text": if goal.is_empty() { label } else { format!("{label}: {goal}") },
                }));
            }
        }

        let tab = match p.tab.as_str() {
            "files" | "plan" | "commands" => p.tab.clone(),
            _ => "timeline".into(),
        };
        let (resolved_filter, _) = offered_filters(&files, &p.file_filter);

        let snap = InspectorSnapshot {
            items,
            show_all_card,
            selected,
            tab,
            files_view: normalize_view(&p.files_view).to_owned(),
            file_filter: resolved_filter,
            cmd_filter: p.cmd_filter.clone(),
            running,
            timeline_events,
            files,
            plan_items,
            commands,
        };
        (snap, updated_selected)
    });
    if let Some(selected) = updated_selected {
        edit(root, |state| state.selected = selected);
    }
    snapshot
}

pub(crate) fn system_card_actions(root: &str, entry: &FileTouchEntry) -> Vec<FileCardAction> {
    let mut actions = vec![FileCardAction {
        label: "Show in timeline".into(),
        action: "activity-reveal".into(),
        payload: json!({
            "root": root,
            "agent": entry.timeline_agent,
            "event_id": entry.event_id,
            "path": entry.path,
            "service": entry.service_id,
        }),
        data_service: None,
        data_path: None,
    }];
    if !entry.outside && !entry.path.is_empty() {
        let svc = entry
            .service_id
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "__system__".into());
        actions.push(FileCardAction {
            label: "Show on dependency map".into(),
            action: "inspect-system-file-graph".into(),
            payload: json!({
                "service": svc,
                "path": entry.path,
            }),
            data_service: Some(svc),
            data_path: Some(entry.path.clone()),
        });
    }
    actions.push(FileCardAction {
        label: "Add path to task".into(),
        action: "activity-add-path".into(),
        payload: json!(entry.path),
        data_service: None,
        data_path: None,
    });
    actions.push(FileCardAction {
        label: "Copy path".into(),
        action: "activity-copy-path".into(),
        payload: json!(entry.path),
        data_service: None,
        data_path: None,
    });
    actions
}
