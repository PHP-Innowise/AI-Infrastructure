use crate::{
    api, classify, dom, model as m,
    run::files_map::{
        self, count_words, focus_element_no_scroll, normalize_view, offered_filters, splice_draft,
        tidy_path, truth, FileCardAction, FileTouchEntry,
    },
};
use serde_json::{json, Value as V};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};
use wasm_bindgen::JsCast;

const CARD_ID: &str = "run-file-card";

#[derive(Clone)]
struct Panel {
    sid: String,
    run_id: String,
    cursor: u64,
    step_seq: u64,
    epoch: u64,
    pending: bool,
    plan: Vec<V>,
    agents: BTreeMap<String, V>,
    order: Vec<String>,
    selected: String,
    follow: bool,
    active: bool,
    tab: String,
    files_view: String,
    file_filter: String,
    cmd_filter: String,
    card_key: Option<String>,
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
    static PANELS: RefCell<BTreeMap<String, Panel>> = const { RefCell::new(BTreeMap::new()) };
    static EVIDENCE: RefCell<(String, BTreeMap<String, V>)> =
        const { RefCell::new((String::new(), BTreeMap::new())) };
    static DISMISS_ON_GRAPH_CLICK: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)> =
        wasm_bindgen::closure::Closure::new(|_: web_sys::Event| {
            hide_any_activity_card(false);
        });
}

fn edit<R>(root: &str, f: impl FnOnce(&mut Panel) -> R) -> R {
    PANELS.with(|all| f(all.borrow_mut().entry(root.into()).or_default()))
}

fn panel(root: &str) -> Panel {
    edit(root, |p| p.clone())
}

fn owner_tag(root: &str) -> String {
    format!("activity:{root}")
}

fn default_service_for_agent(agent_name: &str) -> String {
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

fn clean_service_id(raw: Option<&str>, fallback_agent: &str) -> Option<String> {
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

fn shorten_folder(raw_folder: &str, system_root: &str) -> String {
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

fn service_folders_map(p: &Panel, evidence: Option<&V>) -> BTreeMap<String, String> {
    let catalog_wrap = m::data("system-catalog");
    let catalog = if catalog_wrap["catalog"].is_object() {
        &catalog_wrap["catalog"]
    } else {
        &catalog_wrap
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
    for a in p.agents.values() {
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

fn active_evidence_for(root: &str, p: &Panel) -> Option<V> {
    if root != "system-agents" {
        return None;
    }
    let sys_run = m::data("system-run");
    let sys_run_id = m::text(&sys_run, "id");
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
        return Some(sys_run);
    }
    None
}

pub fn reset(root: &str) {
    hide_card_for_root(root, false);
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
    render(root);
}

pub fn set_plan(root: &str, steps: Vec<V>, active: bool) {
    edit(root, |p| {
        p.plan = steps;
        p.active = active;
    });
    render(root);
}

#[allow(dead_code)]
pub(crate) fn set_run_evidence(run_id: &str, evidence: &V) {
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
    render("system-agents");
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

fn paired_call_id(agent: &mut V, key: &str, start: bool, event_id: &str) -> String {
    let mut waiting = agent["uncalled"].as_object().cloned().unwrap_or_default();
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
    agent["uncalled"] = V::Object(waiting);
    call
}

#[derive(Clone, Debug)]
struct NamedTarget {
    service_id: Option<String>,
    path: String,
    how: &'static str,
    outside: bool,
}

fn parse_shown_location(
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

fn touch_agent_files(
    agent: &mut V,
    call: &mut V,
    targets: &[NamedTarget],
    step: u64,
    event_id: &V,
    helper: bool,
) {
    if targets.is_empty() {
        return;
    }
    let mut touched = call["touched"].as_object().cloned().unwrap_or_default();
    let mut files = agent["files"].as_object().cloned().unwrap_or_default();
    let role_key = if helper { "helper" } else { "main" };

    for target in targets {
        let fkey = file_map_key(target.service_id.as_deref(), &target.path, target.outside);
        if touched.contains_key(&fkey) {
            continue;
        }
        touched.insert(fkey.clone(), json!(target.how));
        if !files.contains_key(&fkey) {
            files.insert(
                fkey.clone(),
                json!({
                    "key": fkey,
                    "path": target.path,
                    "service": target.service_id,
                    "reads": 0,
                    "edits": 0,
                    "created": false,
                    "deleted": false,
                    "failed": false,
                    "outside": target.outside,
                    "helper": false,
                    "main": false,
                    "order": files.len(),
                    "first_step": step,
                    "last_step": step,
                    "event_id": event_id,
                }),
            );
        }
        if let Some(file) = files.get_mut(&fkey) {
            if target.how == "read" {
                file["reads"] = json!(file["reads"].as_u64().unwrap_or(0) + 1);
            }
            file[role_key] = json!(true);
            file["last_step"] = json!(step);
            if !event_id.is_null() {
                file["event_id"] = event_id.clone();
            }
        }
    }
    call["touched"] = V::Object(touched);
    agent["files"] = V::Object(files);
}

fn settle_agent_files(agent: &mut V, call: &mut V, event: &V) {
    if m::yes(&call["settled"]) {
        return;
    }
    call["settled"] = json!(true);
    let Some(touched) = call["touched"].as_object() else {
        return;
    };
    let mut files = agent["files"].as_object().cloned().unwrap_or_default();
    let ok = m::yes(&call["ok"]);
    for (fkey, how) in touched {
        let Some(file) = files.get_mut(fkey) else {
            continue;
        };
        if !ok {
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
        if !event["id"].is_null() {
            file["event_id"] = event["id"].clone();
        }
    }
    agent["files"] = V::Object(files);
}

fn observe_panel(p: &mut Panel, event: &V) {
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
            json!({
                "id": key,
                "agent": name,
                "attempt": attempt,
                "service": if default_svc.is_empty() { V::Null } else { json!(default_svc) },
                "status": "running",
                "events": [],
                "calls": {},
                "call_states": {},
                "files": {},
                "searches": [],
                "plan_items": [],
                "commands": [],
                "tools": 0,
                "messages": 0,
                "tokens": 0,
            }),
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
                    agent[*field] = event[*field].clone();
                }
            }
            if event["status"] == "running" {
                agent["started"] = event["at"].clone();
            } else {
                agent["finished"] = event["at"].clone();
                if event
                    .as_object()
                    .map(|o| o.contains_key("tokens"))
                    .unwrap_or(false)
                {
                    agent["tokens"] = event["tokens"].clone();
                }
                if let Some(states) = agent["call_states"].as_object_mut() {
                    for val in states.values_mut() {
                        val["open"] = json!(false);
                    }
                }
                let default_svc = clean_service_id(agent["service"].as_str(), name);
                let mut files = agent["files"].as_object().cloned().unwrap_or_default();
                for changed in m::array(&agent["changed_files"]) {
                    let raw = changed
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| m::text(&changed, "path"));
                    let (svc, path, outside) = parse_shown_location(&raw, default_svc.as_deref());
                    if path.is_empty() || path == "." {
                        continue;
                    }
                    let fkey = file_map_key(svc.as_deref(), &path, outside);
                    let entry = files.entry(fkey.clone()).or_insert_with(|| {
                        json!({
                            "key": fkey,
                            "path": path,
                            "service": svc,
                            "reads": 0,
                            "edits": 1,
                            "created": false,
                            "deleted": false,
                            "failed": false,
                            "outside": outside,
                            "helper": false,
                            "main": true,
                            "order": 10_000,
                            "first_step": p.step_seq,
                            "last_step": p.step_seq,
                            "event_id": event_id,
                        })
                    });
                    if entry["edits"].as_u64().unwrap_or(0) == 0 {
                        entry["edits"] = json!(1);
                    }
                }
                agent["files"] = V::Object(files);
            }
        }
        Some("usage") => {
            if agent["finished"].is_null() {
                let count = [
                    "input_tokens",
                    "output_tokens",
                    "cache_read_input_tokens",
                    "cache_creation_input_tokens",
                ]
                .iter()
                .filter_map(|k| event[*k].as_f64())
                .sum::<f64>();
                agent["tokens"] = json!(agent["tokens"].as_f64().unwrap_or(0.) + count);
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
                let agent_svc = clean_service_id(agent["service"].as_str(), name);
                let targets =
                    extract_named_targets(raw_tool, &norm_key, event, agent_svc.as_deref());
                let helper = event["parent"].as_str().is_some_and(|s| !s.is_empty());

                let mut call_states = agent["call_states"]
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                let mut call_obj = call_states.get(&call_id).cloned().unwrap_or_else(|| {
                    json!({
                        "tool": display_tool,
                        "key": norm_key,
                        "path": event["path"],
                        "detail": event["detail"],
                        "event_id": event_id,
                        "started": event["at"],
                    })
                });
                call_obj["open"] = json!(start);
                call_obj["ok"] = json!(event["ok"]
                    .as_bool()
                    .unwrap_or(!m::text(event, "text").contains("failed")));
                call_obj["event_id"] = event_id.clone();

                touch_agent_files(agent, &mut call_obj, &targets, step, &event_id, helper);
                if !start {
                    settle_agent_files(agent, &mut call_obj, event);
                }
                call_states.insert(call_id.clone(), call_obj.clone());
                agent["call_states"] = V::Object(call_states);

                if ["grep", "glob", "ls"].contains(&norm_key.as_str()) && start {
                    let mut searches = m::array(&agent["searches"]);
                    let scope_svc = event["service"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .or_else(|| agent_svc.clone());
                    searches.push(json!({
                        "service": scope_svc,
                        "pattern": event["detail"],
                        "scope": event["path"],
                        "event_id": event_id,
                    }));
                    agent["searches"] = json!(searches);
                }

                if ["bash", "shell"].contains(&norm_key.as_str()) {
                    let mut commands = m::array(&agent["commands"]);
                    if let Some(existing) = commands.iter_mut().find(|c| c["call"] == call_id) {
                        existing["running"] = json!(start);
                        existing["ok"] = call_obj["ok"].clone();
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
                        commands.push(json!({
                            "call": call_id,
                            "agent": name,
                            "agent_key": key,
                            "service": event["service"].as_str().filter(|s| !s.is_empty()).or(agent_svc.as_deref()),
                            "command": cmd_text,
                            "class": classify::classify(cmd_text),
                            "running": start,
                            "ok": call_obj["ok"],
                            "event_id": event_id,
                            "at": event["at"],
                        }));
                    }
                    agent["commands"] = json!(commands);
                }
            } else if kind == "plan" {
                p.step_seq += 1;
                let items = event["items"]
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
                agent["plan_items"] = json!(items);
            }

            let mut events = m::array(&agent["events"]);
            if kind == "tool" && !raw_call.is_empty() && event["state"] == "completed" {
                if let Some(index) = agent["calls"][&raw_call].as_u64() {
                    if let Some(item) = events.get_mut(index as usize) {
                        item["state"] = json!("completed");
                        item["ok"] = event["ok"].clone();
                        item["done"] = event["at"].clone();
                        agent["events"] = json!(events);
                        return;
                    }
                }
            }
            if kind == "tool" {
                agent["tools"] = json!(agent["tools"].as_u64().unwrap_or(0) + 1);
                if !raw_call.is_empty() {
                    agent["calls"][&raw_call] = json!(events.len());
                }
            }
            if kind == "text" {
                agent["messages"] = json!(agent["messages"].as_u64().unwrap_or(0) + 1);
            }
            let mut stored = event.clone();
            stored["event_id"] = event_id;
            stored["event_seq"] = json!(p.cursor.max(p.step_seq));
            stored["agent_key"] = json!(key);
            events.push(stored);
            agent["events"] = json!(events);
            agent["last"] = event.clone();
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
            render(&root);
        }
    }
}

pub async fn load(root: &str, sid: &str) {
    if sid.is_empty() {
        let has_plan_or_evidence = edit(root, |p| {
            let has_ev = active_evidence_for(root, p).is_some();
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
            render(root);
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
    render(root);
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
                    }
                }
                if events.len() < 250 {
                    break;
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
        edit(root, |p| p.pending = false);
        render(root);
    }
}

fn dispatch_phase(d: &V) -> String {
    d["phase"]
        .as_str()
        .or_else(|| d["agent"].as_str())
        .or_else(|| d["id"].as_str())
        .unwrap_or("")
        .to_owned()
}

fn dispatch_key(d: &V) -> Option<String> {
    let phase = dispatch_phase(d);
    if phase.is_empty() {
        return None;
    }
    let attempt = d["attempt"].as_u64().unwrap_or(1);
    Some(format!("{phase}#{attempt}"))
}

fn extract_dispatch_evidences(raw: &V) -> (Vec<V>, Vec<V>) {
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

fn filter_dispatches_for_launch(dispatches: Vec<V>, sid: &str) -> Vec<V> {
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

fn parse_evidence_path_item(
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

fn synthetic_agent_from_evidence(d: &V, receipts: &[V]) -> Option<(String, V)> {
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

    Some((
        key.clone(),
        json!({
            "id": key,
            "agent": phase,
            "attempt": attempt,
            "service": sid,
            "root": d["root"],
            "mode": d["mode"],
            "provider": d["provider"],
            "status": status,
            "ok": ok,
            "summary": summary,
            "changed_files": changed_files,
            "checks": checks,
            "tools": tools,
            "messages": 0,
            "tokens": d["tokens"].as_f64().unwrap_or(0.),
            "cost_usd": d["cost_usd"],
            "duration_seconds": d["duration_seconds"],
            "events": [],
            "files": {},
            "searches": [],
            "plan_items": [],
            "commands": [],
            "from_evidence": true,
        }),
    ))
}

fn effective_agents(
    p: &Panel,
    evidence: Option<&V>,
) -> (BTreeMap<String, V>, Vec<String>, Vec<V>, Vec<V>) {
    let mut agents = p.agents.clone();
    let mut order = p.order.clone();
    let (raw_dispatches, receipts) = evidence.map(extract_dispatch_evidences).unwrap_or_default();
    let dispatches = filter_dispatches_for_launch(raw_dispatches, &p.sid);

    for d in &dispatches {
        if let Some((key, synth)) = synthetic_agent_from_evidence(d, &receipts) {
            if let Some(existing) = agents.get_mut(&key) {
                for field in ["service", "root", "summary", "checks"] {
                    if existing[field].is_null() && !synth[field].is_null() {
                        existing[field] = synth[field].clone();
                    }
                }
            } else {
                order.push(key.clone());
                agents.insert(key, synth);
            }
        }
    }
    (agents, order, dispatches, receipts)
}

fn ordered_items(p: &Panel, agents: &BTreeMap<String, V>, order: &[String]) -> Vec<V> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for step in &p.plan {
        let name = m::text(step, "id");
        let latest = order
            .iter()
            .rev()
            .filter_map(|id| agents.get(id))
            .find(|a| a["agent"] == name);
        let mut item = step.clone();
        if !item.is_object() {
            continue;
        }
        if let Some(a) = latest {
            for (k, v) in a.as_object().into_iter().flatten() {
                item[k] = v.clone();
            }
            item["key"] = a["id"].clone();
            seen.insert(m::text(a, "id"));
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
        let a = &agents[key];
        if !seen.contains(key) && !p.plan.iter().any(|s| s["id"] == a["agent"]) {
            let mut item = a.clone();
            item["key"] = json!(key);
            item["step"] = a["agent"].clone();
            item["label"] = a["agent"].clone();
            result.push(item);
        }
    }
    result
}

fn selection(a: &V) -> String {
    a["key"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("step:{}", m::text(a, "step")))
}

fn resolve_selection(root: &str, p: &Panel, items: &[V]) -> (bool, String) {
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

fn searched_patterns_for(
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

fn build_agent_files(
    agent_key: &str,
    agent: &V,
    dispatch: Option<&V>,
    receipt: Option<&V>,
    folders: &BTreeMap<String, String>,
) -> Vec<FileTouchEntry> {
    let agent_name = m::text(agent, "agent");
    let default_svc = clean_service_id(agent["service"].as_str(), &agent_name);
    let agent_running = agent["status"] == "running";

    let mut open_keys = BTreeSet::new();
    if agent_running {
        if let Some(states) = agent["call_states"].as_object() {
            for state in states.values() {
                if m::yes(&state["open"]) {
                    if let Some(touched) = state["touched"].as_object() {
                        open_keys.extend(touched.keys().cloned());
                    }
                }
            }
        }
    }

    let mut searches = m::array(&agent["searches"]);
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
    let mut live_files: Vec<V> = agent["files"]
        .as_object()
        .map(|map| map.values().cloned().collect())
        .unwrap_or_default();
    live_files.sort_by_key(|f| f["order"].as_u64().unwrap_or(0));

    for (idx, f) in live_files.iter().enumerate() {
        let path = tidy_path(&m::text(f, "path"));
        if path.is_empty() || path == "." {
            continue;
        }
        let outside = m::yes(&f["outside"]);
        let svc = if outside {
            None
        } else {
            f["service"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| default_svc.clone())
        };
        let folder = svc.as_deref().and_then(|s| folders.get(s)).cloned();
        let raw_fkey = m::text(f, "key");
        let searched_patterns = searched_patterns_for(&searches, svc.as_deref(), &path, outside);
        let entry = FileTouchEntry {
            path,
            service_id: svc,
            service_folder: folder,
            agents: vec![agent_name.clone()],
            reads: f["reads"].as_u64().unwrap_or(0),
            edits: f["edits"].as_u64().unwrap_or(0),
            created: m::yes(&f["created"]),
            deleted: m::yes(&f["deleted"]),
            failed: m::yes(&f["failed"]),
            outside,
            helper: m::yes(&f["helper"]),
            main: m::yes(&f["main"]),
            now: agent_running && open_keys.contains(&raw_fkey),
            order: idx,
            first_step: f["first_step"].as_u64().unwrap_or(0),
            last_step: f["last_step"].as_u64().unwrap_or(0),
            searched_patterns,
            event_id: f["event_id"].clone(),
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

fn merge_multi_agent_files(per_agent: Vec<Vec<FileTouchEntry>>) -> Vec<FileTouchEntry> {
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

fn build_agent_commands(
    agent_key: &str,
    agent: &V,
    dispatch: Option<&V>,
    receipt: Option<&V>,
) -> Vec<V> {
    let agent_name = m::text(agent, "agent");
    let default_svc = clean_service_id(agent["service"].as_str(), &agent_name);
    let mut list = m::array(&agent["commands"]);
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

    let mut checks = m::array(&agent["checks"]);
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

fn build_agent_plan_items(
    agent_key: &str,
    agent: &V,
    dispatch: Option<&V>,
    step_item: Option<&V>,
    include_step_fallback: bool,
) -> Vec<V> {
    let agent_name = m::text(agent, "agent");
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    for raw in m::array(&agent["plan_items"]) {
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
                .unwrap_or_else(|| agent["status"].as_str().unwrap_or("completed"));
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

fn build_synthetic_timeline(
    agent_key: &str,
    agent: &V,
    files: &[FileTouchEntry],
    commands: &[V],
) -> Vec<V> {
    let agent_name = m::text(agent, "agent");
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
    let mut p = panel(root);
    let evidence = active_evidence_for(root, &p);
    let folders = service_folders_map(&p, evidence.as_ref());
    let (agents, order, dispatches, receipts) = effective_agents(&p, evidence.as_ref());
    let items = ordered_items(&p, &agents, &order);
    let (show_all_card, selected) = resolve_selection(root, &p, &items);
    if p.selected != selected {
        p.selected = selected.clone();
        edit(root, |state| state.selected = selected.clone());
    }

    let dispatch_for = |key: &str| -> Option<&V> {
        dispatches
            .iter()
            .find(|d| dispatch_key(d).as_deref() == Some(key))
    };
    let receipt_for = |agent: &V| -> Option<&V> {
        let phase = m::text(agent, "agent");
        let attempt = agent["attempt"].as_u64().unwrap_or(1);
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
            if agent["status"] == "running" {
                running = true;
            }
            let d = dispatch_for(key);
            let r = receipt_for(agent);
            let ag_files = build_agent_files(key, agent, d, r, &folders);
            let ag_cmds = build_agent_commands(key, agent, d, r);
            let ag_plan = build_agent_plan_items(key, agent, d, None, false);
            plan_items.extend(ag_plan);

            let live_evs = m::array(&agent["events"]);
            if live_evs.is_empty() && m::yes(&agent["from_evidence"]) {
                timeline_events.extend(build_synthetic_timeline(key, agent, &ag_files, &ag_cmds));
            } else {
                timeline_events.extend(live_evs);
            }
            per_agent_files.push(ag_files);
            commands.extend(ag_cmds);
        }
        timeline_events.sort_by_key(|ev| ev["event_seq"].as_u64().unwrap_or(0));
        files = merge_multi_agent_files(per_agent_files);
    } else if let Some(agent) = agents.get(&selected) {
        running = agent["status"] == "running";
        let d = dispatch_for(&selected);
        let r = receipt_for(agent);
        let step_item = items.iter().find(|i| selection(i) == selected);
        files = build_agent_files(&selected, agent, d, r, &folders);
        commands = build_agent_commands(&selected, agent, d, r);
        plan_items = build_agent_plan_items(&selected, agent, d, step_item, true);
        let live_evs = m::array(&agent["events"]);
        timeline_events = if live_evs.is_empty() && m::yes(&agent["from_evidence"]) {
            build_synthetic_timeline(&selected, agent, &files, &commands)
        } else {
            live_evs
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

    InspectorSnapshot {
        items,
        show_all_card,
        selected,
        tab,
        files_view: normalize_view(&p.files_view).to_owned(),
        file_filter: resolved_filter,
        cmd_filter: p.cmd_filter,
        running,
        timeline_events,
        files,
        plan_items,
        commands,
    }
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

fn status(s: &str) -> String {
    match s {
        "pending" => "Waiting".into(),
        "running" => "Working".into(),
        "completed" => "Completed".into(),
        "blocked" => "Blocked".into(),
        "interrupted" => "Stopped".into(),
        "reported" => "Reported complete".into(),
        _ => m::label(s),
    }
}

fn describe(v: &V) -> String {
    if v["type"] == "tool" {
        ["service", "path", "detail"]
            .iter()
            .map(|k| m::text(v, k))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    } else {
        m::text(v, "text")
    }
}

fn clock(value: &V) -> String {
    if !cfg!(target_arch = "wasm32") {
        return String::new();
    }
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(
        value.as_str().unwrap_or(""),
    ));
    if date.get_time().is_finite() {
        format!(
            "{:02}:{:02}:{:02}",
            date.get_hours(),
            date.get_minutes(),
            date.get_seconds()
        )
    } else {
        String::new()
    }
}

fn duration(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0. {
        return String::new();
    }
    let n = seconds.round() as u64;
    if n >= 60 {
        format!("{}m {:02}s", n / 60, n % 60)
    } else {
        format!("{n}s")
    }
}

fn chip_for_root_key(root: &str, key: &str) -> Option<web_sys::Element> {
    if !cfg!(target_arch = "wasm32") {
        return None;
    }
    dom::all(&format!("#{root} .run-map-main .run-file"))
        .into_iter()
        .find(|chip| chip.get_attribute("data-focus-key").as_deref() == Some(key))
}

fn open_card_root() -> Option<(String, String)> {
    PANELS.with(|all| {
        all.borrow()
            .iter()
            .find_map(|(root, p)| p.card_key.as_ref().map(|key| (root.clone(), key.clone())))
    })
}

fn hide_card_for_root(root: &str, restore_focus: bool) {
    let prev = edit(root, |p| p.card_key.take());
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let owner = owner_tag(root);
    let owned_by_root = dom::get(CARD_ID)
        .and_then(|c| c.get_attribute("data-owner"))
        .as_deref()
        == Some(owner.as_str());
    if prev.is_some() || owned_by_root {
        dom::hidden(CARD_ID, true);
    }
    if restore_focus {
        if let Some(chip) = prev.as_deref().and_then(|k| chip_for_root_key(root, k)) {
            focus_element_no_scroll(&chip);
        }
    }
}

fn hide_any_activity_card(restore_focus: bool) -> bool {
    if let Some((root, _)) = open_card_root() {
        hide_card_for_root(&root, restore_focus);
        true
    } else if cfg!(target_arch = "wasm32")
        && dom::get(CARD_ID)
            .and_then(|c| c.get_attribute("data-owner"))
            .is_some_and(|o| o.starts_with("activity:"))
    {
        dom::hidden(CARD_ID, true);
        true
    } else {
        false
    }
}

fn open_activity_file_card(root: &str, key: &str) {
    let snap = snapshot_for_root(root);
    let Some(entry) = snap.files.iter().find(|e| e.key() == key) else {
        return;
    };
    edit(root, |p| p.card_key = Some(key.to_owned()));
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let _ = m::take("run-card");
    if let Some(card) = dom::get(CARD_ID) {
        let _ = card.remove_attribute("data-signature");
    }
    let actions = system_card_actions(root, entry);
    let owner = owner_tag(root);
    DISMISS_ON_GRAPH_CLICK.with(|cb| {
        files_map::fill_card_element(
            CARD_ID,
            &owner,
            entry,
            &actions,
            Some(("inspect-system-file-graph", cb)),
        );
    });
    dom::hidden(CARD_ID, false);
    if let Some(chip) = chip_for_root_key(root, key) {
        files_map::position_card_element(CARD_ID, &chip);
    }
    let selector = format!("#{CARD_ID} button");
    if let Ok(Some(first)) = dom::document().query_selector(&selector) {
        focus_element_no_scroll(&first);
    }
}

fn insert_system_task_path(path: &str) {
    if !cfg!(target_arch = "wasm32") || path.is_empty() {
        return;
    }
    dom::hidden("system-plan-form", false);
    dom::hidden("system-runs-empty", true);
    dom::attr("system-new-change", "aria-expanded", "true");
    let Some(task) =
        dom::get("system-task").and_then(|e| e.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
    else {
        return;
    };
    if task.disabled() {
        return;
    }
    let value = task.value();
    let units = value.encode_utf16().count() as u32;
    let start = task
        .selection_start()
        .ok()
        .flatten()
        .unwrap_or(units)
        .min(units);
    let end = task
        .selection_end()
        .ok()
        .flatten()
        .unwrap_or(start)
        .clamp(start, units);
    let (draft, caret) = splice_draft(&value, start, end, path);
    task.set_value(&draft);
    let _ = task.set_selection_range(caret, caret);
    focus_element_no_scroll(task.as_ref());
}

fn copy_path_to_clipboard(path: &str) {
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let text = path.to_owned();
    wasm_bindgen_futures::spawn_local(async move {
        let promise = dom::window().navigator().clipboard().write_text(&text);
        let ok = wasm_bindgen_futures::JsFuture::from(promise).await.is_ok();
        let selector = format!("#{CARD_ID} [data-action=\"activity-copy-path\"]");
        if let Ok(Some(btn)) = dom::document().query_selector(&selector) {
            btn.set_text_content(Some(if ok { "Copied" } else { "Copy failed" }));
        }
    });
}

fn render_all_agents_card(
    root: &str,
    items: &[V],
    files_count: usize,
    selected: bool,
) -> web_sys::Element {
    let card = dom::button("", "agent-select", json!({"root": root, "key": "all"}));
    let agg_status = if items.iter().any(|a| a["status"] == "running") {
        "running"
    } else if items.iter().any(|a| a["status"] == "blocked") {
        "blocked"
    } else if items.iter().any(|a| a["status"] == "interrupted") {
        "interrupted"
    } else if items
        .iter()
        .any(|a| matches!(a["status"].as_str(), Some("completed" | "reported")))
    {
        "completed"
    } else {
        "pending"
    };
    card.set_class_name(&format!("agent-card {agg_status}"));
    let _ = card.set_attribute("data-focus-key", "card:all");
    let _ = card.set_attribute("aria-pressed", truth(selected));

    let top = dom::el("div", "agent-card-top", "");
    dom::add(&top, &dom::el("span", "agent-dot", ""));
    dom::add(&top, &dom::el("strong", "", "All agents"));

    let total_secs: f64 = items
        .iter()
        .filter_map(|a| a["duration_seconds"].as_f64())
        .sum();
    let elapsed = if total_secs > 0. {
        duration(total_secs)
    } else {
        String::new()
    };
    dom::add(
        &top,
        &dom::el(
            "span",
            "agent-card-status",
            &[status(agg_status), elapsed]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
        ),
    );
    dom::add(&card, &top);

    let services: BTreeSet<String> = items
        .iter()
        .map(|a| m::text(a, "service"))
        .filter(|s| !s.is_empty() && s != "__system__")
        .collect();
    let scope = format!(
        "{} · {}",
        count_words(items.len() as u64, "agent"),
        count_words(services.len().max(1) as u64, "service")
    );
    dom::add(&card, &dom::el("p", "agent-card-scope", &scope));

    if let Some(active_agent) = items
        .iter()
        .rev()
        .find(|a| a["status"] == "running" && !a["last"].is_null())
    {
        let last = &active_agent["last"];
        let short = files_map::short_agent_name(active_agent["agent"].as_str().unwrap_or("agent"));
        let line = format!(
            "{short} · {}{}",
            if last["type"] == "tool" {
                format!("{} · ", last["tool"].as_str().unwrap_or("Tool"))
            } else {
                String::new()
            },
            describe(last)
        );
        dom::add(
            &card,
            &dom::el(
                "p",
                "agent-card-now",
                &line.chars().take(180).collect::<String>(),
            ),
        );
    }

    let total_tools: u64 = items.iter().filter_map(|a| a["tools"].as_u64()).sum();
    let total_tokens: f64 = items.iter().filter_map(|a| a["tokens"].as_f64()).sum();
    let total_cost: f64 = items.iter().filter_map(|a| a["cost_usd"].as_f64()).sum();
    let mut stats = Vec::new();
    if total_tools > 0 {
        stats.push(count_words(total_tools, "tool call"));
    }
    if total_tokens > 0. {
        stats.push(if total_tokens >= 1000. {
            format!("{:.1}k tokens", total_tokens / 1000.)
        } else {
            format!("{total_tokens:.0} tokens")
        });
    }
    if total_cost > 0. {
        stats.push(format!("${total_cost:.4}"));
    }
    if files_count > 0 {
        stats.push(count_words(files_count as u64, "touched file"));
    }
    if !stats.is_empty() {
        dom::add(&card, &dom::el("p", "agent-card-stats", &stats.join(" · ")));
    }
    card
}

fn render_agent_card(root: &str, a: &V, is_selected: bool) -> web_sys::Element {
    let sel_key = selection(a);
    let card = dom::button("", "agent-select", json!({"root": root, "key": sel_key}));
    card.set_class_name(&format!("agent-card {}", m::text(a, "status")));
    let _ = card.set_attribute("data-focus-key", &format!("card:{sel_key}"));
    let _ = card.set_attribute("aria-pressed", truth(is_selected));

    let top = dom::el("div", "agent-card-top", "");
    dom::add(&top, &dom::el("span", "agent-dot", ""));
    dom::add(
        &top,
        &dom::el(
            "strong",
            "",
            a["label"]
                .as_str()
                .unwrap_or_else(|| a["agent"].as_str().unwrap_or("Agent")),
        ),
    );
    let elapsed = if let Some(secs) = a["duration_seconds"].as_f64() {
        duration(secs)
    } else if let Some(start) = a["started"].as_str() {
        let end = a["finished"]
            .as_str()
            .or_else(|| {
                (a["status"] != "running")
                    .then(|| a["last"]["at"].as_str())
                    .flatten()
            })
            .map(|s| js_sys::Date::new(&s.into()).get_time())
            .unwrap_or_else(js_sys::Date::now);
        duration((end - js_sys::Date::new(&start.into()).get_time()) / 1000.)
    } else {
        String::new()
    };
    dom::add(
        &top,
        &dom::el(
            "span",
            "agent-card-status",
            &[status(a["status"].as_str().unwrap_or("pending")), elapsed]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
        ),
    );
    dom::add(&card, &top);

    let mut scope = vec![m::text(a, "mode")];
    if a["attempt"].as_u64().unwrap_or(1) > 1 {
        scope.push(format!("attempt {}", a["attempt"]));
    }
    for (field, count_field, prefix) in [
        ("readable", "readable_count", "reads"),
        ("writable", "writable_count", "writes"),
    ] {
        let names = m::array(&a[field])
            .iter()
            .filter_map(V::as_str)
            .map(|name| {
                if name == "__system__" {
                    "system folder"
                } else {
                    name
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        if !names.is_empty() {
            let extra = a[count_field]
                .as_u64()
                .unwrap_or(0)
                .saturating_sub(m::array(&a[field]).len() as u64);
            scope.push(format!(
                "{prefix} {names}{}",
                if extra > 0 {
                    format!(" +{extra} more")
                } else {
                    String::new()
                }
            ));
        }
    }
    let scope = scope
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if !scope.is_empty() {
        dom::add(&card, &dom::el("p", "agent-card-scope", &scope));
    }

    let now = if a["status"] == "running" && !a["last"].is_null() {
        let last = &a["last"];
        format!(
            "{}{}",
            if last["type"] == "tool" {
                format!("{} · ", last["tool"].as_str().unwrap_or("Tool"))
            } else {
                String::new()
            },
            describe(last)
        )
    } else if a["earlier"] == true {
        "Completed in an earlier launch.".into()
    } else {
        ["summary", "reason", "error", "goal"]
            .iter()
            .map(|k| m::text(a, k))
            .find(|s| !s.is_empty())
            .unwrap_or_default()
    };
    if !now.is_empty() {
        dom::add(
            &card,
            &dom::el(
                "p",
                if a["status"] == "running" {
                    "agent-card-now"
                } else {
                    "agent-card-summary"
                },
                &now.chars().take(180).collect::<String>(),
            ),
        );
    }

    let mut stats = Vec::new();
    for (field, noun) in [("tools", "tool call"), ("messages", "message")] {
        if let Some(n) = a[field].as_u64().filter(|n| *n > 0) {
            stats.push(count_words(n, noun));
        }
    }
    if let Some(n) = a["tokens"].as_f64().filter(|n| *n > 0.) {
        stats.push(if n >= 1000. {
            format!("{:.1}k tokens", n / 1000.)
        } else {
            format!("{n:.0} tokens")
        });
    }
    if let Some(n) = a["cost_usd"].as_f64().filter(|n| *n > 0.) {
        stats.push(format!("${n:.4}"));
    }
    let changed = a["changed_files_count"]
        .as_u64()
        .unwrap_or(m::array(&a["changed_files"]).len() as u64);
    if changed > 0 {
        stats.push(format!("{changed} changed files"));
    }
    if !stats.is_empty() {
        dom::add(&card, &dom::el("p", "agent-card-stats", &stats.join(" · ")));
    }
    card
}

fn folder_restore_key(folder_name: &str) -> String {
    format!("folder:{folder_name}")
}

fn build_folder_render_items(
    folders: &[(String, Vec<FileTouchEntry>)],
    show_agent_badge: bool,
) -> Vec<V> {
    folders
        .iter()
        .map(|(folder_name, entries)| {
            let files = entries
                .iter()
                .map(|file| {
                    let agent_note = if show_agent_badge && !file.agents.is_empty() {
                        format!(" · {}", files_map::format_agent_badge(&file.agents))
                    } else {
                        String::new()
                    };
                    json!({
                        "key": file.key(),
                        "path": file.path,
                        "reads": file.reads,
                        "edits": file.edits,
                        "outside": file.outside,
                        "agent_note": agent_note,
                        "timeline_agent": file.timeline_agent,
                        "event_id": file.event_id,
                        "service_id": file.service_id,
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "key": folder_restore_key(folder_name),
                "folder_name": folder_name,
                "files": files,
            })
        })
        .collect()
}

fn build_folder_details_element(root: &str, item: &V) -> web_sys::Element {
    let folder_name = m::text(item, "folder_name");
    let files = m::array(&item["files"]);
    let restore_key = m::text(item, "key");
    let details = dom::el("details", "run-tree-folder", "");
    let _ = details.set_attribute("open", "");
    let _ = details.set_attribute("data-restore-key", &restore_key);
    let summary = dom::el("summary", "", &format!("{folder_name} · {}", files.len()));
    let _ = summary.set_attribute("data-focus-key", &format!("summary:{restore_key}"));
    dom::add(&details, &summary);
    for file in &files {
        let row = dom::el("div", "run-row-actions", "");
        let path = m::text(file, "path");
        let reads = file["reads"].as_u64().unwrap_or(0);
        let edits = file["edits"].as_u64().unwrap_or(0);
        let agent_note = m::text(file, "agent_note");
        let label = format!(
            "{path} · {} · {}{agent_note}",
            count_words(reads, "read"),
            count_words(edits, "edit")
        );
        let reveal = dom::button(
            &label,
            "activity-reveal",
            json!({
                "root": root,
                "agent": file["timeline_agent"],
                "event_id": file["event_id"],
                "path": path,
                "service": file["service_id"],
            }),
        );
        let _ = reveal.set_attribute("data-focus-key", &format!("list:{}", m::text(file, "key")));
        dom::add(&row, &reveal);
        if !m::yes(&file["outside"]) && !path.is_empty() {
            let svc = file["service_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("__system__")
                .to_owned();
            let map_btn = dom::button(
                "Map",
                "inspect-system-file-graph",
                json!({"service": svc, "path": path}),
            );
            map_btn.set_class_name("button compact");
            let _ = map_btn.set_attribute("data-service", &svc);
            let _ = map_btn.set_attribute("data-path", &path);
            let _ = map_btn.set_attribute("aria-label", &format!("Show {path} on dependency map"));
            dom::add(&row, &map_btn);
        }
        dom::add(&details, &row);
    }
    details
}

fn command_restore_keys(commands: &[V]) -> Vec<String> {
    let mut seen = BTreeMap::<String, usize>::new();
    commands
        .iter()
        .enumerate()
        .map(|(index, cmd)| {
            let call = m::text(cmd, "call");
            let base = if !call.is_empty() {
                format!("cmd:{call}")
            } else {
                let agent = m::text(cmd, "agent_key");
                let command = m::text(cmd, "command");
                if !command.is_empty() {
                    format!("cmd:{agent}:{command}")
                } else {
                    format!("cmd:idx:{index}")
                }
            };
            let count = seen.entry(base.clone()).or_insert(0);
            let key = if *count == 0 {
                base
            } else {
                format!("{base}#{}", *count)
            };
            *count += 1;
            key
        })
        .collect()
}

fn build_command_render_items(snap: &InspectorSnapshot) -> (String, usize, usize, Vec<V>) {
    let checks_count = snap
        .commands
        .iter()
        .filter(|c| m::yes(&c["class"]["check"]))
        .count();
    let failed_count = snap
        .commands
        .iter()
        .filter(|c| !m::yes(&c["running"]) && !m::yes(&c["ok"]) && !m::yes(&c["not_run"]))
        .count();
    let resolved_cmd_filter = match snap.cmd_filter.as_str() {
        "checks" if checks_count > 0 => "checks",
        "failed" if failed_count > 0 => "failed",
        _ => "all",
    }
    .to_owned();
    let keys = command_restore_keys(&snap.commands);
    let mut items = Vec::new();
    for (cmd, key) in snap.commands.iter().zip(keys) {
        let keep = match resolved_cmd_filter.as_str() {
            "checks" => m::yes(&cmd["class"]["check"]),
            "failed" => !m::yes(&cmd["running"]) && !m::yes(&cmd["ok"]) && !m::yes(&cmd["not_run"]),
            _ => true,
        };
        if keep {
            items.push(json!({
                "key": key,
                "selected": snap.selected,
                "cmd": cmd,
            }));
        }
    }
    (resolved_cmd_filter, checks_count, failed_count, items)
}

fn build_command_details_element(item: &V) -> web_sys::Element {
    let key = m::text(item, "key");
    let selected = m::text(item, "selected");
    let cmd = &item["cmd"];
    let class = &cmd["class"];
    let glyph = if m::yes(&cmd["running"]) {
        "●"
    } else if m::yes(&cmd["not_run"]) || !class["masked"].is_null() {
        "?"
    } else if m::yes(&cmd["ok"]) {
        "✓"
    } else {
        "✗"
    };
    let prefix = if selected == "all" {
        let ag = m::text(cmd, "agent");
        if !ag.is_empty() {
            format!("{} · ", files_map::short_agent_name(&ag))
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    let family = class["family"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|f| format!(" [{f}]"))
        .unwrap_or_default();
    let block = dom::details(
        &format!("{glyph} {prefix}{}{family}", m::text(cmd, "command")),
        &serde_json::to_string_pretty(cmd).unwrap_or_default(),
    );
    let _ = block.set_attribute("data-restore-key", &key);
    if let Some(summary) = block.first_element_child() {
        let _ = summary.set_attribute("data-focus-key", &format!("summary:{key}"));
    }
    block
}

fn render_files_pane_dom(root: &str, snap: &InspectorSnapshot) -> web_sys::Element {
    let pane = dom::el("div", "run-pane", "");
    let _ = pane.set_attribute("role", "tabpanel");
    let group_by_service = snap.selected == "all";
    let show_agent_badge = snap.selected == "all";
    let list_mode = snap.files_view == "list";

    let head = dom::el("div", "run-files-head", "");
    let summary = dom::el(
        "p",
        "run-files-summary",
        &files_map::files_summary(&snap.files),
    );
    let _ = summary.set_attribute("id", &format!("{root}-files-summary"));
    dom::add(&head, &summary);
    let seg = dom::el("div", "run-seg", "");
    let _ = seg.set_attribute("id", &format!("{root}-files-seg"));
    let _ = seg.set_attribute("role", "group");
    let _ = seg.set_attribute("aria-label", "File view");
    for (view_id, label, pressed) in [("map", "Map", !list_mode), ("list", "List", list_mode)] {
        let btn = dom::button(
            label,
            "activity-files-view",
            json!({"root": root, "view": view_id}),
        );
        btn.set_class_name("");
        let _ = btn.set_attribute("aria-pressed", truth(pressed));
        let _ = btn.set_attribute("data-focus-key", &format!("view:{view_id}"));
        dom::add(&seg, &btn);
    }
    dom::add(&head, &seg);
    dom::add(&pane, &head);

    if snap.files.is_empty() {
        dom::add(
            &pane,
            &dom::el("p", "run-empty", "Files the agent opens appear here."),
        );
        return pane;
    }

    if !list_mode {
        let (resolved_filter, pills) = offered_filters(&snap.files, &snap.file_filter);
        let filters = dom::el("div", "run-filters", "");
        let _ = filters.set_attribute("role", "group");
        let _ = filters.set_attribute("aria-label", "Show files");
        for pill in &pills {
            let btn = dom::button(
                &pill.label,
                "activity-file-filter",
                json!({"root": root, "filter": pill.id}),
            );
            btn.set_class_name("run-filter");
            let _ = btn.set_attribute("aria-pressed", truth(pill.pressed));
            let _ = btn.set_attribute("data-focus-key", &format!("filter:{}", pill.id));
            dom::add(&filters, &btn);
        }
        dom::add(&pane, &filters);

        let districts = files_map::group_districts(
            &snap.files,
            &resolved_filter,
            snap.running,
            group_by_service,
            show_agent_badge,
        );
        let map = dom::el("div", "run-map", "");
        let _ = map.set_attribute("aria-live", "off");
        let main = dom::el("div", "run-map-main", "");
        for d in &districts {
            dom::add(
                &main,
                &files_map::district_element(d, "activity-file", Some(root), "true"),
            );
        }
        dom::add(&map, &main);
        dom::add(&pane, &map);
        dom::add(&pane, &files_map::legend_element());
    } else {
        let folders = files_map::group_list_folders(&snap.files, group_by_service);
        let tree = dom::el("div", "run-tree", "");
        let _ = tree.set_attribute("id", &format!("{root}-tree"));
        let _ = tree.set_attribute("role", "tree");
        let _ = tree.set_attribute("aria-label", "Files named in tool calls");
        for item in build_folder_render_items(&folders, show_agent_badge) {
            let details = build_folder_details_element(root, &item);
            let _ = details.set_attribute("data-key", &m::text(&item, "key"));
            let _ = details.set_attribute("data-signature", &item.to_string());
            dom::add(&tree, &details);
        }
        dom::add(&pane, &tree);
    }
    pane
}

fn render_plan_pane_dom(snap: &InspectorSnapshot) -> web_sys::Element {
    let pane = dom::el("div", "run-pane", "");
    let _ = pane.set_attribute("role", "tabpanel");
    if snap.plan_items.is_empty() {
        dom::add(
            &pane,
            &dom::el("p", "run-empty", "No plan items recorded for this scope."),
        );
        return pane;
    }
    for item in &snap.plan_items {
        let ag = m::text(item, "agent");
        let st = status(item["status"].as_str().unwrap_or("pending"));
        let txt = m::text(item, "text");
        let line = if snap.selected == "all" && !ag.is_empty() && !m::yes(&item["wave_step"]) {
            format!("{} · {st} · {txt}", files_map::short_agent_name(&ag))
        } else {
            format!("{st} · {txt}")
        };
        dom::add(&pane, &dom::el("p", "run-plan-item", &line));
    }
    pane
}

fn populate_command_filters(
    filters: &web_sys::Element,
    root: &str,
    resolved_cmd_filter: &str,
    checks_count: usize,
    failed_count: usize,
) {
    let sig = json!([resolved_cmd_filter, checks_count, failed_count]).to_string();
    if filters.get_attribute("data-signature").as_deref() == Some(sig.as_str()) {
        return;
    }
    let _ = filters.set_attribute("data-signature", &sig);
    filters.set_text_content(None);
    for (fid, label) in [
        ("all", Some("All".to_owned())),
        (
            "checks",
            (checks_count > 0).then(|| format!("Checks {checks_count}")),
        ),
        (
            "failed",
            (failed_count > 0).then(|| format!("Failed {failed_count}")),
        ),
    ] {
        let Some(label) = label else { continue };
        let btn = dom::button(
            &label,
            "activity-cmd-filter",
            json!({"root": root, "filter": fid}),
        );
        btn.set_class_name("run-filter");
        let _ = btn.set_attribute("aria-pressed", truth(fid == resolved_cmd_filter));
        let _ = btn.set_attribute("data-focus-key", &format!("cmdfilter:{fid}"));
        dom::add(filters, &btn);
    }
}

fn render_commands_pane_dom(root: &str, snap: &InspectorSnapshot) -> web_sys::Element {
    let pane = dom::el("div", "run-pane", "");
    let _ = pane.set_attribute("role", "tabpanel");
    if snap.commands.is_empty() {
        dom::add(
            &pane,
            &dom::el(
                "p",
                "run-empty",
                "No shell commands or checks recorded yet.",
            ),
        );
        return pane;
    }

    let (resolved_cmd_filter, checks_count, failed_count, items) = build_command_render_items(snap);

    let filters = dom::el("div", "run-filters", "");
    let _ = filters.set_attribute("id", &format!("{root}-commands-filters"));
    let _ = filters.set_attribute("role", "group");
    let _ = filters.set_attribute("aria-label", "Filter commands");
    populate_command_filters(
        &filters,
        root,
        &resolved_cmd_filter,
        checks_count,
        failed_count,
    );
    dom::add(&pane, &filters);

    let list = dom::el("div", "run-commands-list", "");
    let _ = list.set_attribute("id", &format!("{root}-commands-list"));
    for item in &items {
        let block = build_command_details_element(item);
        let _ = block.set_attribute("data-key", &m::text(item, "key"));
        let _ = block.set_attribute("data-signature", &item.to_string());
        dom::add(&list, &block);
    }
    dom::add(&pane, &list);
    pane
}

#[allow(dead_code)]
pub(crate) fn render_into(root: &str) {
    render(root);
}

fn capture_details_open_state(root_el: &web_sys::Element) -> BTreeMap<String, bool> {
    let mut states = BTreeMap::new();
    if let Ok(nodes) = root_el.query_selector_all("details[data-restore-key]") {
        for i in 0..nodes.length() {
            if let Some(el) = nodes
                .item(i)
                .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
            {
                if let Some(key) = el.get_attribute("data-restore-key") {
                    states.insert(key, el.has_attribute("open"));
                }
            }
        }
    }
    states
}

fn apply_details_open_state(root_el: &web_sys::Element, states: &BTreeMap<String, bool>) {
    if states.is_empty() {
        return;
    }
    if let Ok(nodes) = root_el.query_selector_all("details[data-restore-key]") {
        for i in 0..nodes.length() {
            if let Some(el) = nodes
                .item(i)
                .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
            {
                if let Some(was_open) = el
                    .get_attribute("data-restore-key")
                    .and_then(|key| states.get(&key).copied())
                {
                    if was_open {
                        let _ = el.set_attribute("open", "");
                    } else {
                        let _ = el.remove_attribute("open");
                    }
                }
            }
        }
    }
}

pub(crate) fn render(root: &str) {
    let snap = snapshot_for_root(root);
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let Some(host) = dom::get(root) else { return };
    let p = panel(root);

    let focused_key = dom::document()
        .active_element()
        .filter(|el| host.contains(Some(el)))
        .and_then(|el| el.get_attribute("data-focus-key"));

    let previous_details = capture_details_open_state(&host);

    let scroll = host
        .query_selector(".agent-events")
        .ok()
        .flatten()
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        .map(|e| {
            (
                e.scroll_top(),
                e.scroll_height() - e.scroll_top() - e.client_height() < 40,
            )
        });

    let commands_list_id = format!("{root}-commands-list");
    let tree_id = format!("{root}-tree");
    let pane_id = format!("{root}-timeline-pane");
    let timeline_id = format!("{root}-timeline");
    let cards_id = format!("{root}-cards");
    let heading_id = format!("{root}-timeline-heading");
    let tabs_id = format!("{root}-timeline-tabs");
    let follow_id = format!("{root}-follow-input");

    let reuse_shell = dom::get(&timeline_id).is_some()
        && dom::get(&cards_id).is_some()
        && dom::get(&pane_id).is_some();

    if !reuse_shell {
        host.set_text_content(None);

        let label = dom::el("label", "checkbox agent-follow", "");
        let input = dom::el("input", "", "");
        let _ = input.set_attribute("id", &follow_id);
        let _ = input.set_attribute("type", "checkbox");
        let _ = input.set_attribute("data-action", "agent-follow");
        let _ = input.set_attribute("data-payload", &json!(root).to_string());
        if let Some(e) = input.dyn_ref::<web_sys::HtmlInputElement>() {
            e.set_checked(p.follow);
        }
        dom::add(&label, &input);
        dom::add(&label, &dom::el("span", "", "Follow the active agent"));
        dom::add(&host, &label);

        let cards = dom::el("div", "agent-cards", "");
        let _ = cards.set_attribute("id", &cards_id);
        dom::add(&host, &cards);

        let timeline = dom::el("div", "agent-timeline", "");
        let _ = timeline.set_attribute("id", &timeline_id);
        let _ = timeline.set_attribute("aria-live", "polite");
        let heading = dom::el("div", "agent-timeline-heading", "");
        let _ = heading.set_attribute("id", &heading_id);
        dom::add(&timeline, &heading);
        let tabs_bar = dom::el("div", "run-tabs", "");
        let _ = tabs_bar.set_attribute("id", &tabs_id);
        let _ = tabs_bar.set_attribute("role", "tablist");
        let _ = tabs_bar.set_attribute("aria-label", "Agent inspector views");
        dom::add(&timeline, &tabs_bar);
        let pane_host = dom::el("div", "", "");
        let _ = pane_host.set_attribute("id", &pane_id);
        dom::add(&timeline, &pane_host);
        dom::add(&host, &timeline);
    } else if let Some(input) =
        dom::get(&follow_id).and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        if input.checked() != p.follow {
            input.set_checked(p.follow);
        }
    }

    let mut card_items = Vec::new();
    if snap.show_all_card {
        card_items.push(json!({
            "key": "all",
            "selected": snap.selected == "all",
            "files_count": snap.files.len(),
            "items": snap.items,
        }));
    }
    for a in &snap.items {
        let sel_key = selection(a);
        card_items.push(json!({
            "key": sel_key,
            "selected": sel_key == snap.selected,
            "agent": a,
        }));
    }
    dom::keyed_render(
        &cards_id,
        &card_items,
        |item| m::text(item, "key"),
        |item| {
            if item["key"] == "all" {
                render_all_agents_card(
                    root,
                    &m::array(&item["items"]),
                    item["files_count"].as_u64().unwrap_or(0) as usize,
                    m::yes(&item["selected"]),
                )
            } else {
                render_agent_card(root, &item["agent"], m::yes(&item["selected"]))
            }
        },
    );

    if let Some(heading) = dom::get(&heading_id) {
        let heading_sig = if snap.selected == "all" {
            let ran_count = snap.items.iter().filter(|a| a["key"].is_string()).count();
            json!(["all", ran_count.max(snap.items.len()), snap.files.len()])
        } else if let Some(a) = snap.items.iter().find(|i| selection(i) == snap.selected) {
            let title = a["label"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| a["agent"].as_str())
                .unwrap_or("Agent");
            let meta = [
                m::summary(&m::provider(&m::text(a, "provider"))),
                m::text(a, "root"),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            json!(["one", title, meta])
        } else {
            json!(["empty", root])
        }
        .to_string();
        if heading.get_attribute("data-signature").as_deref() != Some(heading_sig.as_str()) {
            let _ = heading.set_attribute("data-signature", &heading_sig);
            heading.set_text_content(None);
            if snap.selected == "all" {
                dom::add(&heading, &dom::el("strong", "", "All agents"));
                let ran_count = snap.items.iter().filter(|a| a["key"].is_string()).count();
                dom::add(
                    &heading,
                    &dom::el(
                        "span",
                        "",
                        &format!(
                            "{} · {}",
                            count_words(ran_count.max(snap.items.len()) as u64, "agent"),
                            count_words(snap.files.len() as u64, "file")
                        ),
                    ),
                );
            } else if let Some(a) = snap.items.iter().find(|i| selection(i) == snap.selected) {
                let title = a["label"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| a["agent"].as_str())
                    .unwrap_or("Agent");
                dom::add(&heading, &dom::el("strong", "", title));
                let meta = [
                    m::summary(&m::provider(&m::text(a, "provider"))),
                    m::text(a, "root"),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
                if !meta.is_empty() {
                    dom::add(&heading, &dom::el("span", "", &meta));
                }
            } else {
                dom::add(
                    &heading,
                    &dom::el(
                        "span",
                        "",
                        if root == "system-discovery-agent" {
                            "The discovery agent has not started yet."
                        } else {
                            "Select an agent card to follow its work."
                        },
                    ),
                );
            }
        }
    }

    if let Some(tabs_bar) = dom::get(&tabs_id) {
        let tabs_sig = json!([
            snap.tab,
            snap.files.len(),
            snap.plan_items.len(),
            snap.commands.len()
        ])
        .to_string();
        if tabs_bar.get_attribute("data-signature").as_deref() != Some(tabs_sig.as_str()) {
            let _ = tabs_bar.set_attribute("data-signature", &tabs_sig);
            tabs_bar.set_text_content(None);
            for (tab_id, title, badge_count) in [
                ("timeline", "Timeline", None),
                ("files", "Files", Some(snap.files.len())),
                ("plan", "Plan", Some(snap.plan_items.len())),
                ("commands", "Commands", Some(snap.commands.len())),
            ] {
                let active_tab = snap.tab == tab_id;
                let btn = dom::button("", "activity-tab", json!({"root": root, "tab": tab_id}));
                btn.set_class_name("run-tab");
                let _ = btn.set_attribute("role", "tab");
                let _ = btn.set_attribute("aria-selected", truth(active_tab));
                let _ = btn.set_attribute("data-focus-key", &format!("tab:{tab_id}"));
                if let Some(n) = badge_count {
                    let _ = btn.set_attribute("aria-label", &format!("{title} ({n})"));
                    btn.set_text_content(Some(&format!("{title} ")));
                    dom::add(&btn, &dom::el("span", "run-badge", &n.to_string()));
                } else {
                    btn.set_text_content(Some(title));
                }
                dom::add(&tabs_bar, &btn);
            }
        }
    }

    let Some(pane_host) = dom::get(&pane_id) else {
        return;
    };

    if snap.tab == "commands" && !snap.commands.is_empty() && dom::get(&commands_list_id).is_some()
    {
        let (resolved_cmd_filter, checks_count, failed_count, items) =
            build_command_render_items(&snap);
        if let Some(filters) = dom::get(&format!("{root}-commands-filters")) {
            populate_command_filters(
                &filters,
                root,
                &resolved_cmd_filter,
                checks_count,
                failed_count,
            );
        }
        dom::keyed_render(
            &commands_list_id,
            &items,
            |item| m::text(item, "key"),
            build_command_details_element,
        );
    } else if snap.tab == "files"
        && snap.files_view == "list"
        && !snap.files.is_empty()
        && dom::get(&tree_id).is_some()
    {
        let summary_text = files_map::files_summary(&snap.files);
        let summary_id = format!("{root}-files-summary");
        if dom::get(&summary_id)
            .and_then(|e| e.text_content())
            .as_deref()
            != Some(summary_text.as_str())
        {
            dom::text(&summary_id, &summary_text);
        }
        let group_by_service = snap.selected == "all";
        let show_agent_badge = snap.selected == "all";
        let folders = files_map::group_list_folders(&snap.files, group_by_service);
        let items = build_folder_render_items(&folders, show_agent_badge);
        let root_owned = root.to_owned();
        dom::keyed_render(
            &tree_id,
            &items,
            |item| m::text(item, "key"),
            move |item| build_folder_details_element(&root_owned, item),
        );
        if let Some(tree) = dom::get(&tree_id) {
            apply_details_open_state(&tree, &previous_details);
        }
    } else {
        match snap.tab.as_str() {
            "files" => {
                pane_host.set_text_content(None);
                let pane = render_files_pane_dom(root, &snap);
                apply_details_open_state(&pane, &previous_details);
                dom::add(&pane_host, &pane);
            }
            "plan" => {
                pane_host.set_text_content(None);
                dom::add(&pane_host, &render_plan_pane_dom(&snap));
            }
            "commands" => {
                pane_host.set_text_content(None);
                let pane = render_commands_pane_dom(root, &snap);
                apply_details_open_state(&pane, &previous_details);
                dom::add(&pane_host, &pane);
            }
            _ => {
                let timeline_sig = json!([
                    snap.selected,
                    snap.running,
                    snap.timeline_events,
                    snap.items
                ])
                .to_string();
                let current_timeline_sig = pane_host
                    .query_selector(".agent-events")
                    .ok()
                    .flatten()
                    .and_then(|el| el.get_attribute("data-signature"));
                if current_timeline_sig.as_deref() == Some(timeline_sig.as_str()) {
                    // Nothing in the timeline changed; preserve DOM and scroll untouched.
                } else {
                    pane_host.set_text_content(None);
                    let list = dom::el("ol", "agent-events", "");
                    let _ = list.set_attribute("data-signature", &timeline_sig);
                    if snap.timeline_events.is_empty() {
                        let msg = if snap.selected == "all" {
                            if snap.running {
                                "Waiting for the first agent action…"
                            } else {
                                "No timeline events recorded in this launch."
                            }
                        } else if let Some(a) =
                            snap.items.iter().find(|i| selection(i) == snap.selected)
                        {
                            if a["status"] == "running" {
                                "Waiting for the first action…"
                            } else if a["earlier"] == true {
                                "This agent completed in an earlier launch; choose that launch above."
                            } else if a["key"].is_null() {
                                "This agent has not run in this launch."
                            } else {
                                "No activity was recorded for this agent."
                            }
                        } else {
                            "Select an agent card to follow its work."
                        };
                        dom::add(&list, &dom::el("li", "agent-placeholder", msg));
                    } else {
                        for item in &snap.timeline_events {
                            let kind = m::text(item, "type");
                            let row = dom::el(
                                "li",
                                &format!(
                                    "agent-event {kind}{}",
                                    if item["ok"] == false { " failed" } else { "" }
                                ),
                                "",
                            );
                            let ev_id = m::string(&item["event_id"]);
                            if !ev_id.is_empty() {
                                let _ = row.set_attribute("data-event-id", &ev_id);
                            }
                            let ev_path = m::text(item, "path");
                            if !ev_path.is_empty() {
                                let _ = row.set_attribute("data-file-path", &ev_path);
                            }
                            dom::add(
                                &row,
                                &dom::el("time", "agent-event-time", &clock(&item["at"])),
                            );
                            let base_title = if kind == "tool" {
                                item["tool"].as_str().unwrap_or("Tool").to_owned()
                            } else {
                                match kind.as_str() {
                                    "text" => "Says".into(),
                                    "thinking" => "Reasons".into(),
                                    "plan" => "Plan".into(),
                                    "error" => "Error".into(),
                                    "status" => "Note".into(),
                                    _ => m::label(&kind),
                                }
                            };
                            let title = if snap.selected == "all" {
                                let ag = m::text(item, "agent");
                                if !ag.is_empty() {
                                    format!("{} · {base_title}", files_map::short_agent_name(&ag))
                                } else {
                                    base_title
                                }
                            } else {
                                base_title
                            };
                            dom::add(&row, &dom::el("span", "agent-event-kind", &title));
                            dom::add(
                                &row,
                                &dom::el(
                                    if ["text", "thinking", "plan"].contains(&kind.as_str()) {
                                        "pre"
                                    } else {
                                        "span"
                                    },
                                    "agent-event-body",
                                    &describe(item),
                                ),
                            );
                            if kind == "tool" {
                                dom::add(
                                    &row,
                                    &dom::el(
                                        "span",
                                        "agent-event-state",
                                        if item["state"] == "completed" {
                                            if item["ok"] == false {
                                                "failed"
                                            } else {
                                                "done"
                                            }
                                        } else {
                                            "running"
                                        },
                                    ),
                                );
                            }
                            dom::add(&list, &row);
                        }
                    }

                    if snap.selected != "all" {
                        if let Some(a) = snap
                            .items
                            .iter()
                            .find(|i| selection(i) == snap.selected && i["key"].is_string())
                        {
                            let outcome = [m::text(a, "summary"), m::text(a, "reason")]
                                .into_iter()
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                                .join("\n");
                            let outcome = if outcome.is_empty() {
                                m::text(a, "error")
                            } else {
                                outcome
                            };
                            if !outcome.is_empty() {
                                dom::add(
                                    &list,
                                    &dom::el(
                                        "li",
                                        if a["ok"] == true {
                                            "agent-outcome ok"
                                        } else {
                                            "agent-outcome failed"
                                        },
                                        &outcome,
                                    ),
                                );
                            }
                        }
                    }

                    dom::add(&pane_host, &list);
                    if let Ok(e) = list.dyn_into::<web_sys::HtmlElement>() {
                        if let Some((top, stick)) = scroll {
                            e.set_scroll_top(if stick { e.scroll_height() } else { top });
                        }
                    }
                }
            }
        }
    }

    if let Some(card_key) = p.card_key.as_deref() {
        if snap.tab == "files" && snap.files_view == "map" {
            if let Some(entry) = snap.files.iter().find(|e| e.key() == card_key) {
                let actions = system_card_actions(root, entry);
                let owner = owner_tag(root);
                DISMISS_ON_GRAPH_CLICK.with(|cb| {
                    files_map::fill_card_element(
                        CARD_ID,
                        &owner,
                        entry,
                        &actions,
                        Some(("inspect-system-file-graph", cb)),
                    );
                });
            } else {
                hide_card_for_root(root, false);
            }
        } else {
            hide_card_for_root(root, false);
        }
    }

    if let Some(fk) = focused_key {
        let focus_still_in_host = dom::document()
            .active_element()
            .is_some_and(|el| host.contains(Some(&el)));
        if !focus_still_in_host {
            let restored = dom::all(&format!("#{root} [data-focus-key]"))
                .into_iter()
                .find(|el| el.get_attribute("data-focus-key").as_deref() == Some(fk.as_str()));
            if let Some(node) = restored {
                focus_element_no_scroll(&node);
            }
        }
    }
}

fn scroll_to_timeline_event(root: &str, event_id: &V, path: &str) {
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let ev_str = m::string(event_id);
    if !ev_str.is_empty() {
        let found = dom::all(&format!("#{root} .agent-event[data-event-id]"))
            .into_iter()
            .find(|el| el.get_attribute("data-event-id").as_deref() == Some(ev_str.as_str()));
        if let Some(node) = found {
            node.scroll_into_view();
            return;
        }
    }
    if !path.is_empty() {
        let found = dom::all(&format!("#{root} .agent-event[data-file-path]"))
            .into_iter()
            .find(|el| el.get_attribute("data-file-path").as_deref() == Some(path));
        if let Some(node) = found {
            node.scroll_into_view();
        }
    }
}

#[allow(dead_code)]
pub(crate) fn handle_click(target: &web_sys::Element) -> bool {
    let Some(el) = target.closest("[data-action]").ok().flatten() else {
        return false;
    };
    let action = el.get_attribute("data-action").unwrap_or_default();
    let payload = el
        .get_attribute("data-payload")
        .and_then(|raw| serde_json::from_str::<V>(&raw).ok())
        .unwrap_or(V::Null);
    event("click", &action, &payload, false)
}

#[allow(dead_code)]
pub(crate) fn escape() -> bool {
    hide_any_activity_card(true)
}

#[allow(dead_code)]
pub(crate) fn pointer_down(target: &web_sys::Element) {
    if open_card_root().is_some()
        && target
            .closest("#run-file-card, .run-file")
            .ok()
            .flatten()
            .is_none()
    {
        hide_any_activity_card(false);
    }
}

pub fn event(kind: &str, action: &str, payload: &V, checked: bool) -> bool {
    match (kind, action) {
        ("click", "inspect-system-file-graph") => {
            hide_any_activity_card(false);
            false
        }
        ("click", "agent-select") => {
            let root = m::text(payload, "root");
            hide_card_for_root(&root, false);
            edit(&root, |p| {
                p.selected = m::text(payload, "key");
                p.follow = false;
            });
            render(&root);
            true
        }
        ("change", "agent-follow") => {
            let root = payload.as_str().unwrap_or("");
            edit(root, |p| p.follow = checked);
            render(root);
            true
        }
        ("click", "activity-tab") => {
            let root = m::text(payload, "root");
            let tab = m::text(payload, "tab");
            hide_card_for_root(&root, false);
            edit(&root, |p| p.tab = tab);
            render(&root);
            true
        }
        ("click", "activity-files-view") => {
            let root = m::text(payload, "root");
            let view = normalize_view(&m::text(payload, "view")).to_owned();
            hide_card_for_root(&root, false);
            edit(&root, |p| p.files_view = view);
            render(&root);
            true
        }
        ("click", "activity-file-filter") => {
            let root = m::text(payload, "root");
            let filter = m::text(payload, "filter");
            edit(&root, |p| p.file_filter = filter);
            render(&root);
            true
        }
        ("click", "activity-cmd-filter") => {
            let root = m::text(payload, "root");
            let filter = m::text(payload, "filter");
            edit(&root, |p| p.cmd_filter = filter);
            render(&root);
            true
        }
        ("click", "activity-file") => {
            let root = m::text(payload, "root");
            let key = m::text(payload, "key");
            let current = panel(&root).card_key;
            if current.as_deref() == Some(key.as_str()) {
                hide_card_for_root(&root, true);
            } else {
                open_activity_file_card(&root, &key);
            }
            true
        }
        ("click", "activity-reveal") => {
            let root = m::text(payload, "root");
            let target_agent = m::text(payload, "agent");
            let path = m::text(payload, "path");
            hide_card_for_root(&root, false);
            edit(&root, |p| {
                p.tab = "timeline".into();
                if p.selected != "all" && !target_agent.is_empty() {
                    p.selected = target_agent;
                    p.follow = false;
                }
            });
            render(&root);
            scroll_to_timeline_event(&root, &payload["event_id"], &path);
            true
        }
        ("click", "activity-add-path") => {
            hide_any_activity_card(false);
            insert_system_task_path(payload.as_str().unwrap_or(""));
            true
        }
        ("click", "activity-copy-path") => {
            copy_path_to_clipboard(payload.as_str().unwrap_or(""));
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::files_map::{group_districts, render_file_card_html};

    fn setup_sample_plan() -> Vec<V> {
        vec![
            json!({
                "id": "contracts",
                "service": "__system__",
                "mode": "read-only",
                "label": "Contracts agent",
                "goal": "Reads contracts and shared context before dispatch.",
            }),
            json!({
                "id": "service-orders",
                "service": "orders",
                "mode": "edit",
                "label": "orders agent",
                "goal": "Updates OrderPlaced emitter and tests.",
            }),
            json!({
                "id": "service-payments",
                "service": "payments",
                "mode": "edit",
                "label": "payments agent",
                "goal": "Consumes updated OrderPlaced payload.",
            }),
            json!({
                "id": "verify",
                "service": "__system__",
                "mode": "read-only",
                "label": "Verification agent",
                "goal": "Checks service receipts and cross-service contracts.",
            }),
        ]
    }

    #[test]
    fn multi_agent_live_stream_builds_timeline_files_plan_commands_and_service_districts() {
        reset("system-agents");
        m::set(
            "system-catalog",
            json!({
                "catalog": {
                    "root": "/workspace/harbor-system",
                    "services": [
                        {"id": "orders", "root": "/workspace/harbor-core/services/orders"},
                        {"id": "payments", "root": "/workspace/harbor-payments"}
                    ]
                }
            }),
        );
        set_plan("system-agents", setup_sample_plan(), true);
        edit("system-agents", |p| p.sid = "sess-live-1".into());

        // 1. Contracts agent reads shared contract
        observe_session(
            "sess-live-1",
            &json!({
                "id": 1,
                "kind": "agent",
                "agent": "contracts",
                "attempt": 1,
                "status": "running",
                "service": "__system__",
                "root": "/workspace/harbor-system",
            }),
        );
        observe_session(
            "sess-live-1",
            &json!({
                "id": 2,
                "kind": "agent_activity",
                "agent": "contracts",
                "attempt": 1,
                "type": "tool",
                "tool": "Read",
                "call": "c-1",
                "state": "started",
                "service": "__system__",
                "path": "contracts/order-placed.yaml",
            }),
        );
        observe_session(
            "sess-live-1",
            &json!({
                "id": 3,
                "kind": "agent_activity",
                "agent": "contracts",
                "attempt": 1,
                "type": "tool",
                "tool": "Read",
                "call": "c-1",
                "state": "completed",
                "ok": true,
                "service": "__system__",
                "path": "contracts/order-placed.yaml",
            }),
        );
        observe_session(
            "sess-live-1",
            &json!({
                "id": 4,
                "kind": "agent",
                "agent": "contracts",
                "attempt": 1,
                "status": "completed",
                "ok": true,
                "summary": "Reviewed order-placed contract.",
            }),
        );

        // 2. Orders agent searches, plans, edits, creates via file_change, and runs PHPUnit
        observe_session(
            "sess-live-1",
            &json!({
                "id": 5,
                "kind": "agent",
                "agent": "service-orders",
                "attempt": 1,
                "status": "running",
                "service": "orders",
                "root": "/workspace/harbor-core/services/orders",
            }),
        );
        observe_session(
            "sess-live-1",
            &json!({
                "id": 6,
                "kind": "agent_activity",
                "agent": "service-orders",
                "attempt": 1,
                "type": "plan",
                "items": [
                    {"content": "Add currency field to OrderPlaced", "status": "in_progress"},
                    {"content": "Run orders test suite", "status": "pending"}
                ]
            }),
        );
        observe_session(
            "sess-live-1",
            &json!({
                "id": 7,
                "kind": "agent_activity",
                "agent": "service-orders",
                "attempt": 1,
                "type": "tool",
                "tool": "Grep",
                "call": "o-grep",
                "state": "started",
                "service": "orders",
                "path": "src/Events",
                "detail": "OrderPlaced",
            }),
        );
        observe_session(
            "sess-live-1",
            &json!({
                "id": 8,
                "kind": "agent_activity",
                "agent": "service-orders",
                "attempt": 1,
                "type": "tool",
                "tool": "Edit",
                "call": "o-edit",
                "state": "started",
                "service": "orders",
                "path": "src/Events/OrderPlaced.php",
            }),
        );

        // While o-edit is still open and orders is running, follow selects service-orders#1
        let live_snap = snapshot_for_root("system-agents");
        assert_eq!(live_snap.selected, "service-orders#1");
        assert!(live_snap.running);
        assert_eq!(live_snap.files.len(), 1);
        assert!(live_snap.files[0].now);

        observe_session(
            "sess-live-1",
            &json!({
                "id": 9,
                "kind": "agent_activity",
                "agent": "service-orders",
                "attempt": 1,
                "type": "tool",
                "tool": "Edit",
                "call": "o-edit",
                "state": "completed",
                "ok": true,
                "service": "orders",
                "path": "src/Events/OrderPlaced.php",
            }),
        );
        observe_session(
            "sess-live-1",
            &json!({
                "id": 10,
                "kind": "agent_activity",
                "agent": "service-orders",
                "attempt": 1,
                "type": "tool",
                "tool": "file_change",
                "call": "o-fc",
                "state": "completed",
                "ok": true,
                "detail": "add orders:tests/OrderPlacedTest.php",
            }),
        );
        observe_session(
            "sess-live-1",
            &json!({
                "id": 11,
                "kind": "agent_activity",
                "agent": "service-orders",
                "attempt": 1,
                "type": "tool",
                "tool": "Bash",
                "call": "o-bash",
                "state": "completed",
                "ok": true,
                "service": "orders",
                "detail": "vendor/bin/phpunit --filter OrderPlacedTest",
            }),
        );
        observe_session(
            "sess-live-1",
            &json!({
                "id": 12,
                "kind": "agent",
                "agent": "service-orders",
                "attempt": 1,
                "status": "completed",
                "ok": true,
            }),
        );

        // 3. Verify agent also reads OrderPlaced.php across service boundary!
        observe_session(
            "sess-live-1",
            &json!({
                "id": 13,
                "kind": "agent",
                "agent": "verify",
                "attempt": 1,
                "status": "running",
                "service": "__system__",
                "root": "/workspace/harbor-system",
            }),
        );
        observe_session(
            "sess-live-1",
            &json!({
                "id": 14,
                "kind": "agent_activity",
                "agent": "verify",
                "attempt": 1,
                "type": "tool",
                "tool": "Read",
                "call": "v-1",
                "state": "completed",
                "ok": true,
                "service": "orders",
                "path": "src/Events/OrderPlaced.php",
            }),
        );
        observe_session(
            "sess-live-1",
            &json!({
                "id": 15,
                "kind": "agent",
                "agent": "verify",
                "attempt": 1,
                "status": "completed",
                "ok": true,
            }),
        );

        // Once all agents finish, follow defaults to "all" (All agents)
        let all_snap = snapshot_for_root("system-agents");
        assert_eq!(all_snap.selected, "all");
        assert!(all_snap.show_all_card);
        assert_eq!(all_snap.files.len(), 3);
        assert_eq!(all_snap.commands.len(), 1);
        assert!(all_snap.plan_items.len() >= 6); // 4 wave steps + 2 orders plan items

        // In All agents view, districts group by Service (`{service_id} · {folder}`)
        let svc_districts = group_districts(&all_snap.files, "all", false, true, true);
        assert_eq!(svc_districts.len(), 2);
        assert_eq!(svc_districts[0].label, "__system__ · .");
        assert_eq!(
            svc_districts[1].label,
            "orders · harbor-core/services/orders"
        );
        // OrderPlaced.php was touched by both service-orders and verify!
        let order_event_chip = &svc_districts[1].blocks[0].files[0];
        assert_eq!(order_event_chip.agent_badge, "orders+verify");
        assert!(order_event_chip.edited);

        // Selecting single agent `service-orders#1` switches districts to top folders (1:1 with single-project Run Files map)
        assert!(event(
            "click",
            "agent-select",
            &json!({"root": "system-agents", "key": "service-orders#1"}),
            false
        ));
        assert!(event(
            "click",
            "activity-tab",
            &json!({"root": "system-agents", "tab": "files"}),
            false
        ));
        let orders_snap = snapshot_for_root("system-agents");
        assert_eq!(orders_snap.selected, "service-orders#1");
        assert_eq!(orders_snap.files.len(), 2);
        let folder_districts = group_districts(&orders_snap.files, "all", false, false, false);
        assert_eq!(folder_districts[0].label, "src/");
        assert_eq!(folder_districts[1].label, "tests/");

        // File card HTML & 4 required actions (including inspect-system-file-graph with data-service and data-path)
        let shared_entry = all_snap
            .files
            .iter()
            .find(|e| e.path == "src/Events/OrderPlaced.php")
            .expect("OrderPlaced entry");
        let actions = system_card_actions("system-agents", shared_entry);
        assert_eq!(actions.len(), 4);
        assert_eq!(actions[0].label, "Show in timeline");
        assert_eq!(actions[1].label, "Show on dependency map");
        assert_eq!(actions[1].action, "inspect-system-file-graph");
        assert_eq!(actions[1].data_service.as_deref(), Some("orders"));
        assert_eq!(
            actions[1].data_path.as_deref(),
            Some("src/Events/OrderPlaced.php")
        );
        assert_eq!(actions[2].label, "Add path to task");
        assert_eq!(actions[3].label, "Copy path");

        let card_html = render_file_card_html(shared_entry, &actions);
        assert!(card_html.contains("orders · src/Events/OrderPlaced.php"));
        assert!(card_html.contains("Touched by service-orders, verify"));
        assert!(card_html.contains("data-action=\"inspect-system-file-graph\""));
        assert!(card_html.contains("data-service=\"orders\""));
        assert!(card_html.contains("data-path=\"src/Events/OrderPlaced.php\""));

        // inspect-system-file-graph hides activity card and returns false so systems.rs handles it
        assert!(!event(
            "click",
            "inspect-system-file-graph",
            &json!({"service": "orders", "path": "src/Events/OrderPlaced.php"}),
            false
        ));

        assert_eq!(all_snap.tab, "timeline");
        assert_eq!(all_snap.files.len(), 3);
        assert_eq!(all_snap.commands.len(), 1);
    }

    fn render_files_surface_for_test(root: &str, snap: &InspectorSnapshot) -> String {
        let opts = crate::run::files_map::FilesSurfaceOptions {
            group_by_service: snap.selected == "all",
            show_agent_badge: snap.selected == "all",
            chip_action: "activity-file".into(),
            view_action: "activity-files-view".into(),
            filter_action: "activity-file-filter".into(),
            reveal_action: "activity-reveal".into(),
            inspect_graph_action: "inspect-system-file-graph".into(),
            root_id: Some(root.to_owned()),
        };
        files_map::render_files_surface(
            &snap.files,
            &snap.files_view,
            &snap.file_filter,
            snap.running,
            &opts,
        )
    }

    #[test]
    fn persisted_evidence_hydrates_cli_runs_and_filters_by_launch_without_duplicate_counts() {
        reset("system-agents");
        set_plan("system-agents", setup_sample_plan(), false);

        // Simulate CLI-executed or historical run via set_run_evidence (no session events at all!)
        set_run_evidence(
            "sysrun-42",
            &json!({
                "dispatches": [
                    {
                        "phase": "contracts",
                        "attempt": 1,
                        "launch_id": "launch-a",
                        "service": "__system__",
                        "root": ".",
                        "opened": ["contracts/orders.yaml"],
                        "steps": 2
                    },
                    {
                        "phase": "service-orders",
                        "attempt": 1,
                        "launch_id": "launch-a",
                        "service": "orders",
                        "root": "services/orders",
                        "opened": ["src/OrderService.php"],
                        "shell_reads": {"src/LegacyOrderMapper.php": 17},
                        "git_changed": [
                            {"path": "src/OrderService.php", "status": "M"},
                            {"path": "src/OrderDto.php", "status": "A"}
                        ],
                        "searched": [{"pattern": "OrderDto", "scope": "src"}],
                        "plan_items": [{"text": "Emit OrderDto from OrderService", "status": "completed"}],
                        "command_runs": [
                            {"command": "vendor/bin/phpunit", "ok": true},
                            {"command": "vendor/bin/phpstan analyse", "ok": true}
                        ],
                        "commands": 2,
                        "steps": 6
                    },
                    {
                        "phase": "service-payments",
                        "attempt": 1,
                        "launch_id": "launch-b",
                        "service": "payments",
                        "root": "../harbor-payments",
                        "opened": ["internal/consumer.go"],
                        "git_changed": ["internal/consumer.go"],
                        "commands": ["go test ./..."],
                        "steps": 4
                    }
                ]
            }),
        );

        // With empty sid (CLI mode), all dispatches are shown
        let cli_snap = snapshot_for_root("system-agents");
        assert_eq!(cli_snap.selected, "all");
        assert_eq!(cli_snap.files.len(), 5); // orders.yaml, OrderService.php, LegacyOrderMapper.php (shell_read!), OrderDto.php, consumer.go
        assert_eq!(cli_snap.commands.len(), 3);
        assert!(cli_snap
            .files
            .iter()
            .any(|f| f.path == "src/OrderDto.php" && f.created && f.edits == 1));
        assert!(cli_snap
            .files
            .iter()
            .any(|f| f.path == "src/LegacyOrderMapper.php" && f.reads == 1));
        assert!(cli_snap
            .files
            .iter()
            .any(|f| f.path == "src/OrderService.php"
                && f.searched_patterns == vec!["OrderDto".to_owned()]));
        // Synthetic timeline events exist even without live session stream!
        assert!(!cli_snap.timeline_events.is_empty());

        // Scoping to launch-b shows only launch-b's dispatches (`service-payments`)
        edit("system-agents", |p| p.sid = "launch-b".into());
        let launch_b_snap = snapshot_for_root("system-agents");
        assert_eq!(launch_b_snap.files.len(), 1);
        assert_eq!(
            launch_b_snap.files[0].service_id.as_deref(),
            Some("payments")
        );
        assert_eq!(launch_b_snap.files[0].path, "internal/consumer.go");
        assert_eq!(launch_b_snap.commands.len(), 1);

        // When BOTH live session events AND persisted evidence report the same edit on consumer.go,
        // union-max prevents double counting `edits` (stays 1, not 2!)
        observe_session(
            "launch-b",
            &json!({
                "id": 1,
                "kind": "agent",
                "agent": "service-payments",
                "attempt": 1,
                "status": "completed",
                "service": "payments",
                "root": "../harbor-payments",
                "ok": true,
            }),
        );
        observe_session(
            "launch-b",
            &json!({
                "id": 2,
                "kind": "agent_activity",
                "agent": "service-payments",
                "attempt": 1,
                "type": "tool",
                "tool": "Edit",
                "call": "p-edit",
                "state": "completed",
                "ok": true,
                "service": "payments",
                "path": "internal/consumer.go",
            }),
        );
        let merged_snap = snapshot_for_root("system-agents");
        assert_eq!(merged_snap.files.len(), 1);
        assert_eq!(merged_snap.files[0].edits, 1);
    }

    #[test]
    fn list_view_filters_reveal_and_outside_or_failed_files_work_end_to_end() {
        reset("system-agents");
        set_plan("system-agents", setup_sample_plan(), false);

        set_run_evidence(
            "sysrun-99",
            &json!({
                "evidence": {
                    "by_phase": {
                        "service-orders": {
                            "attempt": 1,
                            "service": "orders",
                            "root": "/home/dev/lab/harbor-core/services/orders",
                            "file_touches": [
                                {
                                    "path": "src/OrderController.php",
                                    "reads": 3,
                                    "edits": 1,
                                    "first_step": 1,
                                    "last_step": 4
                                },
                                {
                                    "path": "src/BrokenHandler.php",
                                    "reads": 1,
                                    "edits": 0,
                                    "failed": true,
                                    "first_step": 2,
                                    "last_step": 2
                                },
                                {
                                    "path": "secrets.env",
                                    "reads": 1,
                                    "edits": 0,
                                    "outside": true,
                                    "first_step": 3,
                                    "last_step": 3
                                }
                            ],
                            "commands": [
                                {"command": "vendor/bin/phpunit", "ok": false, "exit_code": 1}
                            ]
                        }
                    }
                },
                "receipts": [
                    {
                        "phase": "verify",
                        "attempt": 1,
                        "ok": true,
                        "report": {
                            "status": "completed",
                            "summary": "All cross-service contracts verified.",
                            "changed_files": [],
                            "checks": [
                                {
                                    "name": "contract-suite",
                                    "command": "cargo test --workspace",
                                    "status": "passed"
                                }
                            ]
                        }
                    }
                ]
            }),
        );

        assert!(event(
            "click",
            "activity-tab",
            &json!({"root": "system-agents", "tab": "files"}),
            false
        ));
        assert!(event(
            "click",
            "activity-file-filter",
            &json!({"root": "system-agents", "filter": "failed"}),
            false
        ));
        let failed_snap = snapshot_for_root("system-agents");
        assert_eq!(failed_snap.file_filter, "failed");
        let map_html = render_files_surface_for_test("system-agents", &failed_snap);
        assert!(map_html.contains("Edited 1"));
        assert!(map_html.contains("Failed 1"));
        assert!(map_html.contains("Outside 1"));
        assert!(map_html.contains("×3"));

        // Outside files omit `Show on dependency map` action on the file card
        let outside_entry = failed_snap
            .files
            .iter()
            .find(|e| e.outside)
            .expect("outside entry");
        let outside_actions = system_card_actions("system-agents", outside_entry);
        assert_eq!(outside_actions.len(), 3);
        assert!(outside_actions
            .iter()
            .all(|a| a.action != "inspect-system-file-graph"));

        // Toggle to List view and verify Map button carries inspect-system-file-graph
        assert!(event(
            "click",
            "activity-files-view",
            &json!({"root": "system-agents", "view": "list"}),
            false
        ));
        let list_snap = snapshot_for_root("system-agents");
        assert_eq!(list_snap.files_view, "list");
        let list_html = render_files_surface_for_test("system-agents", &list_snap);
        assert!(list_html.contains("class=\"run-tree\""));
        assert!(list_html.contains("data-action=\"inspect-system-file-graph\""));

        // Clicking Show in timeline (`activity-reveal`) switches back to `timeline` tab
        // and selects the target agent when not in `all` mode
        assert!(event(
            "click",
            "agent-select",
            &json!({"root": "system-agents", "key": "step:contracts"}),
            false
        ));
        assert!(event(
            "click",
            "activity-reveal",
            &json!({
                "root": "system-agents",
                "agent": "service-orders#1",
                "event_id": "ev:service-orders#1:src/OrderController.php",
                "path": "src/OrderController.php",
                "service": "orders"
            }),
            false
        ));
        let revealed = snapshot_for_root("system-agents");
        assert_eq!(revealed.tab, "timeline");
        assert_eq!(revealed.selected, "service-orders#1");
    }

    #[test]
    fn command_and_folder_restore_keys_stay_stable_across_status_updates_and_dedup_collisions() {
        let cmds = vec![
            json!({
                "call": "c-1",
                "agent_key": "service-orders#1",
                "command": "cargo test",
                "running": true,
                "ok": true,
                "class": { "check": true }
            }),
            json!({
                "agent_key": "service-orders#1",
                "command": "git status",
                "running": false,
                "ok": true,
                "class": { "check": false }
            }),
            json!({
                "agent_key": "service-orders#1",
                "command": "git status",
                "running": false,
                "ok": false,
                "class": { "check": false }
            }),
        ];
        assert_eq!(
            command_restore_keys(&cmds),
            vec![
                "cmd:c-1",
                "cmd:service-orders#1:git status",
                "cmd:service-orders#1:git status#1",
            ]
        );
        // Key stays identical when running -> completed so keyed_render preserves open <details>
        let mut finished = cmds.clone();
        finished[0]["running"] = json!(false);
        assert_eq!(command_restore_keys(&finished)[0], "cmd:c-1");

        let snap = InspectorSnapshot {
            items: Vec::new(),
            show_all_card: false,
            selected: "all".into(),
            tab: "commands".into(),
            files_view: "map".into(),
            file_filter: "all".into(),
            cmd_filter: "checks".into(),
            running: false,
            timeline_events: Vec::new(),
            files: Vec::new(),
            plan_items: Vec::new(),
            commands: finished,
        };
        let (filter, checks_count, failed_count, items) = build_command_render_items(&snap);
        assert_eq!(filter, "checks");
        assert_eq!(checks_count, 1);
        assert_eq!(failed_count, 1);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["key"], "cmd:c-1");

        let folders = vec![(
            "orders · src/".to_owned(),
            vec![FileTouchEntry {
                path: "src/Order.php".to_owned(),
                service_id: Some("orders".to_owned()),
                reads: 2,
                edits: 1,
                ..FileTouchEntry::default()
            }],
        )];
        let folder_items = build_folder_render_items(&folders, true);
        assert_eq!(folder_items.len(), 1);
        assert_eq!(folder_items[0]["key"], "folder:orders · src/");
    }
}
