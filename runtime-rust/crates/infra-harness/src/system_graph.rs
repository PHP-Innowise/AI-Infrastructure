//! Multi-service code graph target resolution, HTTP domain handlers, live run bridge, and
//! multi-root read-only `system_explain` session adaptation.
use crate::{
    HarnessError, Result,
    graph::{Graphs, RunChange, Trigger, prepare, service_offer, shell_reads},
    graph_changes::{self, Change, Resolved, Target},
    graph_live::{CreatedEntity, GraphFocus, LiveFocusTracker, predict_targets},
    graph_map::{self, Index},
    run_activity::{Enricher, redact_command, relative},
    services::{Domain, Payload, Reply},
    store::{self, SessionStore},
};
use infra_host::{
    activity::normalize,
    provider::{Launch, PreparedCommand},
};
use infra_system::{
    ChangedEntityEntry, ChangedFileEntry, CodeGraphBridge, CodeReference, CrossServiceImpactQuery,
    DispatchEvidence, DispatchGraphContext, EvidenceFileChange, LayerViolationEntry,
    ServiceCodeSummary, ServiceGraphOffer, SystemGraph, SystemManager, SystemMapQuery,
    catalog::System, collect_git_service_summaries, collect_run_service_summaries,
    git_changes_since, git_worktree_changes, load_run_evidence, summarize_run_evidence,
};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

const SYSTEM_FOCUS_PREFIX: &str = "system_graph_focus.";
const MAX_UNDECLARED_SCAN_FILES: usize = 64;
const MAX_UNDECLARED_SCAN_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug)]
pub struct ResolvedServiceTarget {
    pub project_id: String,
    pub config_path: String,
    pub service_id: String,
    pub graph_id: String,
    pub root: PathBuf,
    pub name: String,
    pub reused_project: bool,
}

fn validate_config_rel(raw: Option<&str>) -> Result<String> {
    let trimmed = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("system.json");
    if trimmed.len() > 512 || trimmed.chars().any(char::is_control) {
        return Err(HarnessError::invalid("Invalid system configuration path."));
    }
    let normalized = trimmed.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') || !normalized.ends_with(".json") {
        return Err(HarnessError::invalid(
            "Choose a relative system JSON configuration file.",
        ));
    }
    for comp in Path::new(&normalized).components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            _ => {
                return Err(HarnessError::forbidden(
                    "System configuration path escapes registered project root.",
                ));
            }
        }
    }
    Ok(normalized)
}

pub fn registered_project_roots(
    store: &SessionStore,
) -> Result<(BTreeMap<PathBuf, String>, Vec<PathBuf>)> {
    let mut by_root = BTreeMap::new();
    let mut allowed = Vec::new();
    for proj in store.projects()? {
        let Some(id) = proj["id"].as_str() else {
            continue;
        };
        let Some(path_str) = proj["path"].as_str() else {
            continue;
        };
        if let Ok(canon) = infra_core::fs::canonicalize(Path::new(path_str)) {
            by_root
                .entry(canon.clone())
                .or_insert_with(|| id.to_owned());
            if !allowed.contains(&canon) {
                allowed.push(canon);
            }
        }
    }
    Ok((by_root, allowed))
}

fn is_under_allowed_roots(candidate: &Path, allowed: &[PathBuf]) -> bool {
    allowed.iter().any(|root| candidate.starts_with(root))
}

fn system_display_name(system: &System) -> &str {
    system.config_data["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("system")
}

pub fn resolve_system(
    store: &SessionStore,
    manager: Option<&SystemManager>,
    project_id: &str,
    config_path: Option<&str>,
) -> Result<(System, String, BTreeMap<PathBuf, String>)> {
    let pid = project_id.trim();
    if pid.is_empty() || pid.len() > 128 || pid.chars().any(char::is_control) {
        return Err(HarnessError::invalid("Project identity is required."));
    }
    let config_rel = validate_config_rel(config_path)?;
    let (by_root, allowed) = registered_project_roots(store)?;
    if let Some(mgr) = manager {
        let sys = mgr.load_system(pid, &config_rel)?;
        return Ok((sys, config_rel, by_root));
    }
    let project = store.project(pid)?;
    let proj_path = project["path"]
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Registered project path missing."))?;
    let canon_proj = infra_core::fs::canonicalize(Path::new(proj_path))?;
    if !is_under_allowed_roots(&canon_proj, &allowed) {
        return Err(HarnessError::forbidden(
            "Project root is outside registered allowed roots.",
        ));
    }
    let config_file = canon_proj.join(&config_rel);
    if !config_file.is_file() {
        return Err(HarnessError::missing(
            "System configuration file not found.",
        ));
    }
    let canon_cfg = infra_core::fs::canonicalize(&config_file)?;
    if !is_under_allowed_roots(&canon_cfg, &allowed) {
        return Err(HarnessError::forbidden(
            "System configuration file escapes registered allowed roots.",
        ));
    }
    let sys = System::new(&canon_cfg, &allowed)?;
    Ok((sys, config_rel, by_root))
}

pub fn compute_graph_id_for_root(
    canonical_projects: &BTreeMap<PathBuf, String>,
    canonical_root: &Path,
) -> (String, bool) {
    if let Some(pid) = canonical_projects.get(canonical_root) {
        (pid.clone(), true)
    } else {
        let digest = store::sha(canonical_root.to_string_lossy().as_bytes());
        (format!("sys-{}", &digest[..16]), false)
    }
}

fn build_target(
    graphs: Option<&Graphs>,
    canonical_projects: &BTreeMap<PathBuf, String>,
    project_id: &str,
    config_path: &str,
    service_id: &str,
    raw_root: &Path,
    display_name: String,
) -> Result<ResolvedServiceTarget> {
    let canon_root = infra_core::fs::canonicalize(raw_root).map_err(|_| {
        HarnessError::missing(format!("Service root for '{service_id}' not found."))
    })?;
    let allowed: Vec<PathBuf> = canonical_projects.keys().cloned().collect();
    if !is_under_allowed_roots(&canon_root, &allowed) {
        return Err(HarnessError::forbidden(format!(
            "Service '{service_id}' root lies outside registered allowed project roots."
        )));
    }
    let (graph_id, reused_project) = compute_graph_id_for_root(canonical_projects, &canon_root);
    if !reused_project && let Some(g) = graphs {
        g.register_target(&graph_id, canon_root.clone(), display_name.clone())?;
    }
    Ok(ResolvedServiceTarget {
        project_id: project_id.to_owned(),
        config_path: config_path.to_owned(),
        service_id: service_id.to_owned(),
        graph_id,
        root: canon_root,
        name: display_name,
        reused_project,
    })
}

fn targets_from_loaded_system(
    graphs: Option<&Graphs>,
    by_root: &BTreeMap<PathBuf, String>,
    project_id: &str,
    cfg_rel: &str,
    system: &System,
) -> Vec<ResolvedServiceTarget> {
    let sys_name = system_display_name(system);
    let mut out = Vec::new();
    if let Ok(sys_target) = build_target(
        graphs,
        by_root,
        project_id,
        cfg_rel,
        "__system__",
        &system.root,
        format!("{sys_name} · system"),
    ) {
        out.push(sys_target);
    }
    for (sid, svc) in &system.services {
        if svc.access != "available" {
            continue;
        }
        let label = svc
            .manifest
            .as_ref()
            .and_then(|m| m["name"].as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(sid.as_str())
            .to_owned();
        if let Ok(target) =
            build_target(graphs, by_root, project_id, cfg_rel, sid, &svc.root, label)
        {
            out.push(target);
        }
    }
    out
}

pub fn resolve_service_target(
    store: &SessionStore,
    graphs: Option<&Graphs>,
    manager: Option<&SystemManager>,
    project_id: &str,
    config_path: Option<&str>,
    service_id: &str,
) -> Result<(System, ResolvedServiceTarget)> {
    let sid = service_id.trim();
    if sid.is_empty() || sid.len() > 128 || sid.chars().any(char::is_control) {
        return Err(HarnessError::invalid("Service identity is required."));
    }
    let (system, cfg_rel, by_root) = resolve_system(store, manager, project_id, config_path)?;
    let sys_name = system_display_name(&system);
    if sid == "__system__" {
        let target = build_target(
            graphs,
            &by_root,
            project_id,
            &cfg_rel,
            "__system__",
            &system.root,
            format!("{sys_name} · system"),
        )?;
        return Ok((system, target));
    }
    let Some(svc) = system.services.get(sid) else {
        return Err(HarnessError::missing(format!(
            "Unknown service '{sid}' in system configuration."
        )));
    };
    if svc.access != "available" {
        return Err(HarnessError::forbidden(format!(
            "Service '{sid}' is not accessible ({}).",
            svc.access
        )));
    }
    let label = svc
        .manifest
        .as_ref()
        .and_then(|m| m["name"].as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(sid)
        .to_owned();
    let target = build_target(
        graphs, &by_root, project_id, &cfg_rel, sid, &svc.root, label,
    )?;
    Ok((system, target))
}

pub fn resolve_all_targets(
    store: &SessionStore,
    graphs: Option<&Graphs>,
    manager: Option<&SystemManager>,
    project_id: &str,
    config_path: Option<&str>,
) -> Result<(System, String, Vec<ResolvedServiceTarget>)> {
    let (system, cfg_rel, by_root) = resolve_system(store, manager, project_id, config_path)?;
    let out = targets_from_loaded_system(graphs, &by_root, project_id, &cfg_rel, &system);
    Ok((system, cfg_rel, out))
}

pub(crate) fn resolve_targets_from_system_file(
    store: &SessionStore,
    graphs: Option<&Graphs>,
    system_file: &Path,
) -> Option<(System, Vec<ResolvedServiceTarget>)> {
    let (by_root, allowed) = registered_project_roots(store).ok()?;
    let canon_cfg = infra_core::fs::canonicalize(system_file).ok()?;
    if !is_under_allowed_roots(&canon_cfg, &allowed) {
        return None;
    }
    let system = System::new(&canon_cfg, &allowed).ok()?;
    let canon_sys_root =
        infra_core::fs::canonicalize(&system.root).unwrap_or_else(|_| system.root.clone());
    let matched = by_root
        .iter()
        .filter(|(r, _)| {
            canon_cfg.starts_with(r.as_path()) || canon_sys_root.starts_with(r.as_path())
        })
        .max_by_key(|(r, _)| r.as_os_str().len());
    let project_id = matched.map(|(_, pid)| pid.clone()).unwrap_or_default();
    let cfg_rel = matched
        .and_then(|(matched_root, _)| canon_cfg.strip_prefix(matched_root).ok())
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .filter(|s| !s.is_empty())
        .or_else(|| {
            canon_cfg
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "system.json".to_owned());
    let targets = targets_from_loaded_system(graphs, &by_root, &project_id, &cfg_rel, &system);
    Some((system, targets))
}

// ---------------------------------------------------------------------------
// Multi-Root Read-Only `system_explain` Session Validation & Launch Adapter
// ---------------------------------------------------------------------------

pub fn validate_system_explain(
    store: &SessionStore,
    session_project: &Value,
    spec: &Value,
) -> Result<Value> {
    if !store::exact(spec, &["services"], &["project_id", "config_path"]) {
        return Err(HarnessError::invalid(
            "Invalid system_explain specification.",
        ));
    }
    let session_pid = session_project["id"]
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Session project identity missing."))?;
    let requested_pid = spec["project_id"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(session_pid);
    if requested_pid != session_pid {
        return Err(HarnessError::invalid(
            "system_explain project_id must match the session project_id.",
        ));
    }
    let services_arr = spec["services"]
        .as_array()
        .ok_or_else(|| HarnessError::invalid("system_explain.services must be an array."))?;
    if services_arr.is_empty() || services_arr.len() > 32 {
        return Err(HarnessError::invalid(
            "system_explain.services must name between 1 and 32 services.",
        ));
    }
    let config_raw = spec.get("config_path").and_then(Value::as_str);
    let (system, cfg_rel, by_root) = resolve_system(store, None, requested_pid, config_raw)?;
    let sys_name = system_display_name(&system);

    let sys_target = build_target(
        None,
        &by_root,
        requested_pid,
        &cfg_rel,
        "__system__",
        &system.root,
        format!("{sys_name} · system"),
    )?;
    store_synthetic_target_setting(store, &sys_target)?;

    let mut seen = BTreeSet::new();
    let mut service_ids = Vec::new();
    let mut roots = vec![json!({
        "id": "__system__",
        "path": sys_target.root.to_string_lossy(),
        "graph_id": sys_target.graph_id,
    })];
    let mut primary_root: Option<String> = None;

    for raw_sid in services_arr {
        let sid = raw_sid
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
            .ok_or_else(|| HarnessError::invalid("Invalid service name in system_explain."))?;
        if !seen.insert(sid.to_owned()) {
            continue;
        }
        service_ids.push(sid.to_owned());
        if sid == "__system__" {
            if primary_root.is_none() {
                primary_root = Some(sys_target.root.to_string_lossy().into_owned());
            }
            continue;
        }
        let Some(svc) = system.services.get(sid) else {
            return Err(HarnessError::missing(format!(
                "Unknown service '{sid}' in system_explain."
            )));
        };
        if svc.access != "available" {
            return Err(HarnessError::forbidden(format!(
                "Service '{sid}' is outside registered allowed project roots."
            )));
        }
        let target = build_target(
            None,
            &by_root,
            requested_pid,
            &cfg_rel,
            sid,
            &svc.root,
            sid.to_owned(),
        )?;
        store_synthetic_target_setting(store, &target)?;
        if primary_root.is_none() {
            primary_root = Some(target.root.to_string_lossy().into_owned());
        }
        roots.push(json!({
            "id": sid,
            "path": target.root.to_string_lossy(),
            "graph_id": target.graph_id,
        }));
    }

    let primary = primary_root.unwrap_or_else(|| sys_target.root.to_string_lossy().into_owned());
    Ok(json!({
        "project_id": requested_pid,
        "config_path": cfg_rel,
        "system_name": sys_name,
        "system_root": sys_target.root.to_string_lossy(),
        "primary_root": primary,
        "services": service_ids,
        "roots": roots,
    }))
}

fn store_synthetic_target_setting(
    store: &SessionStore,
    target: &ResolvedServiceTarget,
) -> Result<()> {
    if target.reused_project {
        return Ok(());
    }
    let key = format!("graph.{}", target.graph_id);
    let mut setting = store.setting(&key)?.unwrap_or_else(|| json!({}));
    if !setting.is_object() {
        setting = json!({});
    }
    setting["target_root"] = json!(target.root.to_string_lossy());
    setting["target_name"] = json!(target.name);
    store.set_setting(&key, &setting)
}

pub fn system_explain_prompt_prefix(store: &SessionStore, spec: &Value) -> String {
    let sys_name = spec["system_name"].as_str().unwrap_or("system");
    let cfg = spec["config_path"].as_str().unwrap_or("system.json");
    let mut lines = vec![format!(
        "Read-only multi-service architecture explanation for system `{sys_name}` (`{cfg}`). Do not edit, create, or delete any files."
    )];
    lines.push("Accessible read-only directories:".to_owned());
    let mut offers = Vec::new();
    if let Some(roots) = spec["roots"].as_array() {
        for entry in roots {
            let id = entry["id"].as_str().unwrap_or("");
            let path = entry["path"].as_str().unwrap_or("");
            let gid = entry["graph_id"].as_str().unwrap_or("");
            if !id.is_empty() && !path.is_empty() {
                lines.push(format!("- `{id}`: `{path}`"));
            }
            if !gid.is_empty()
                && let Ok(Some(offer)) = service_offer(store, gid, id, false)
            {
                offers.push(offer.prompt_block);
            }
        }
    }
    if !offers.is_empty() {
        lines.push(String::new());
        lines.extend(offers);
    }
    lines.push(String::new());
    lines.push(String::new());
    lines.join("\n")
}

pub fn adapt_system_explain_launch(
    store: &SessionStore,
    session: &Value,
    input: &Launch,
    mut command: PreparedCommand,
) -> Result<PreparedCommand> {
    let spec = &session["system_explain"];
    command
        .environment
        .insert("GRAPHIFY_NO_AUTO_REFRESH".to_owned(), "1".to_owned());

    let mut dirs = Vec::new();
    let mut graph_ids = Vec::new();
    if let Some(roots) = spec["roots"].as_array() {
        for entry in roots {
            if let Some(path_str) = entry["path"].as_str()
                && !path_str.is_empty()
            {
                let pb = PathBuf::from(path_str);
                if !dirs.contains(&pb) {
                    dirs.push(pb.clone());
                }
                if !command.readable.contains(&pb) {
                    command.readable.push(pb);
                }
            }
            if let Some(gid) = entry["graph_id"].as_str()
                && !gid.is_empty()
                && !graph_ids.iter().any(|g: &String| g == gid)
            {
                graph_ids.push(gid.to_owned());
            }
        }
    }

    for gid in &graph_ids {
        prepare(store, gid, &input.provider, &mut command)?;
    }

    let extra_dirs: Vec<PathBuf> = dirs.into_iter().filter(|d| *d != input.project).collect();

    match input.provider.as_str() {
        "codex" => {
            let insert_at = command
                .argv
                .iter()
                .rposition(|arg| arg == "-")
                .unwrap_or(command.argv.len());
            let mut additions = Vec::new();
            if !command.argv.iter().any(|a| a == "--skip-git-repo-check") {
                additions.push("--skip-git-repo-check".to_owned());
            }
            for dir in &extra_dirs {
                let text = dir.to_string_lossy().into_owned();
                additions.push("--add-dir".to_owned());
                additions.push(text);
            }
            if !additions.is_empty() {
                command.argv.splice(insert_at..insert_at, additions);
            }
        }
        "claude" => {
            let mut read_rules = Vec::new();
            for dir in std::iter::once(&input.project).chain(extra_dirs.iter()) {
                let clean = dir
                    .to_string_lossy()
                    .replace('\\', "/")
                    .trim_start_matches('/')
                    .to_owned();
                for tool in ["Read", "Glob", "Grep"] {
                    read_rules.push(format!("{tool}(//{clean}/**)"));
                }
            }
            merge_claude_allow_rules(&mut command.argv, &read_rules)?;
            for dir in &extra_dirs {
                let text = dir.to_string_lossy().into_owned();
                command.argv.push("--add-dir".to_owned());
                command.argv.push(text);
            }
        }
        _ => {}
    }

    Ok(command)
}

fn merge_claude_allow_rules(argv: &mut Vec<String>, rules: &[String]) -> Result<()> {
    if rules.is_empty() {
        return Ok(());
    }
    let at = argv.iter().enumerate().find_map(|(idx, arg)| {
        if arg == "--settings" {
            argv.get(idx + 1).map(|val| (idx + 1, false, val.as_str()))
        } else {
            arg.strip_prefix("--settings=").map(|val| (idx, true, val))
        }
    });
    let mut settings = match at {
        Some((_, _, text)) => serde_json::from_str::<Value>(text)
            .map_err(|_| HarnessError::invalid("Invalid Claude settings JSON."))?,
        None => json!({}),
    };
    if !settings["permissions"].is_object() {
        settings["permissions"] = json!({});
    }
    if !settings["permissions"]["allow"].is_array() {
        settings["permissions"]["allow"] = json!([]);
    }
    if let Some(allow) = settings["permissions"]["allow"].as_array_mut() {
        for rule in rules {
            let val = json!(rule);
            if !allow.contains(&val) {
                allow.push(val);
            }
        }
    }
    let encoded = serde_json::to_string(&settings)?;
    match at {
        Some((idx, false, _)) => argv[idx] = encoded,
        Some((idx, true, _)) => argv[idx] = format!("--settings={encoded}"),
        None => {
            let pos = usize::from(!argv.is_empty());
            argv.splice(pos..pos, ["--settings".to_owned(), encoded]);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `sysrun:<run_id>` Resolution, Launch Comparison & Focus Helpers
// ---------------------------------------------------------------------------

pub fn load_system_run_record(store: &SessionStore, run_id: &str) -> Result<Value> {
    let rid = run_id.trim();
    if rid.is_empty()
        || rid.len() > 64
        || !rid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(HarnessError::invalid("Invalid system run ID."));
    }
    let conn = store.connection()?;
    let mut stmt = conn
        .prepare("SELECT data FROM system_runs WHERE id = ?1")
        .map_err(|_| HarnessError::missing("This project has no such system run."))?;
    let mut rows = stmt.query([rid])?;
    let Some(row) = rows.next()? else {
        return Err(HarnessError::missing(
            "This project has no such system run.",
        ));
    };
    let raw: String = row.get(0)?;
    crate::strict_json::parse(raw.as_bytes(), 8 * 1024 * 1024)
}

fn agent_matches(filter: Option<&str>, phase: &str, service: &str) -> bool {
    let Some(f) = filter
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
    else {
        return true;
    };
    if f == "contracts" {
        return phase == "contracts";
    }
    if f == "verify" {
        return phase == "verify";
    }
    if let Some(sid) = f.strip_prefix("service-") {
        return phase == f || service == sid;
    }
    phase == f || service == f
}

fn resolve_launch_selector(
    selector: Option<&str>,
    launches: &[(String, usize, Value)],
) -> Result<Option<String>> {
    let Some(sel) = selector.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    for (sid, num, _) in launches {
        if sid == sel
            || sel == num.to_string()
            || sel.strip_prefix("launch-") == Some(num.to_string().as_str())
        {
            return Ok(Some(sid.clone()));
        }
    }
    Err(HarnessError::missing(
        "This system run has no such recorded launch.",
    ))
}

pub(crate) fn sysrun_launches(
    store: &SessionStore,
    project: &str,
    root: &Path,
    run_id: &str,
    agent: Option<&str>,
    launch: Option<&str>,
    compare: Option<&str>,
) -> Result<(RunChange, Option<RunChange>)> {
    let record = load_system_run_record(store, run_id)?;
    let run_project_id = record["project_id"].as_str().unwrap_or("");
    let run_config = record["config_path"].as_str();
    let (system, _cfg_rel, by_root) = resolve_system(store, None, run_project_id, run_config)?;

    let canon_root = infra_core::fs::canonicalize(root)?;
    let canon_sys_root =
        infra_core::fs::canonicalize(&system.root).unwrap_or_else(|_| system.root.clone());

    let mut matched_service: Option<String> = None;
    for (sid, svc) in &system.services {
        if svc.access == "available"
            && let Ok(c) = infra_core::fs::canonicalize(&svc.root)
            && c == canon_root
        {
            matched_service = Some(sid.clone());
            break;
        }
    }
    if matched_service.is_none() && canon_sys_root == canon_root {
        matched_service = Some("__system__".to_owned());
    }
    let Some(service_id) = matched_service else {
        let (expected_gid, _) = compute_graph_id_for_root(&by_root, &canon_root);
        if expected_gid != project && run_project_id != project {
            return Err(HarnessError::missing(
                "This project has no such system run.",
            ));
        }
        return Err(HarnessError::missing(
            "This service does not belong to the requested system run.",
        ));
    };

    let exec_dir = store.state.join("ai-system").join(run_id).join("execution");
    let run_state = fs::read(exec_dir.join("run.json"))
        .ok()
        .and_then(|bytes| crate::strict_json::parse(&bytes, 16 * 1024 * 1024).ok())
        .unwrap_or_else(|| json!({}));

    let mut dispatches = load_run_evidence(&exec_dir, &run_state).unwrap_or_default();
    if let Some(live) = active_bridge_dispatches(run_id) {
        for ev in live {
            if !dispatches
                .iter()
                .any(|d| d.phase == ev.phase && d.attempt == ev.attempt)
            {
                dispatches.push(ev);
            }
        }
    }

    let mut launch_ids: Vec<String> = record["launches"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    if let Some(cur_sid) = record["session_id"].as_str().filter(|s| !s.is_empty())
        && !launch_ids.iter().any(|s| s == cur_sid)
    {
        launch_ids.push(cur_sid.to_owned());
    }
    let mut launches_meta = Vec::new();
    let mut existing_sessions = BTreeSet::new();
    let mut launches_json = Vec::new();
    for (idx, sid) in launch_ids.iter().enumerate() {
        let session_row = store.get(sid).ok();
        if session_row.is_some() {
            existing_sessions.insert(sid.clone());
        }
        let at = session_row
            .map(|s| s["created_at"].clone())
            .unwrap_or(Value::Null);
        let num = idx + 1;
        launches_meta.push((sid.clone(), num, at.clone()));
        launches_json.push(json!({
            "id": sid,
            "number": num,
            "at": at,
        }));
    }

    let primary_launch_sid = resolve_launch_selector(launch, &launches_meta)?;
    let compare_launch_sid = resolve_launch_selector(compare, &launches_meta)?;

    let mut agents_list = Vec::new();
    let mut seen_agents = BTreeSet::new();
    if let Some(steps) = run_state["steps"].as_array() {
        for step in steps {
            if let Some(id) = step["id"].as_str().filter(|s| !s.is_empty())
                && seen_agents.insert(id.to_owned())
            {
                agents_list.push(json!({
                    "id": id,
                    "service": step["service"],
                    "role": step["role"],
                    "status": step["status"],
                    "attempt": step["attempt"],
                }));
            }
        }
    }
    for ev in &dispatches {
        if seen_agents.insert(ev.phase.clone()) {
            let role = if ev.phase == "contracts" || ev.phase == "verify" {
                "read"
            } else {
                "edit"
            };
            agents_list.push(json!({
                "id": ev.phase,
                "service": ev.service,
                "role": role,
                "status": "completed",
                "attempt": ev.attempt,
            }));
        }
    }

    let collect_session_shell_events = |sess_id: &str| -> (BTreeSet<u64>, BTreeMap<String, u64>) {
        let mut valid_ids = BTreeSet::new();
        let mut resolved_reads = BTreeMap::new();
        let svc_prefix = format!("{service_id}/");
        let mut after = 0i64;
        while let Ok(events) = store.events(sess_id, after) {
            let Some(last_id) = events.last().and_then(|e| e["id"].as_i64()) else {
                break;
            };
            for ev_row in &events {
                let Some(eid) = ev_row["id"].as_u64().filter(|id| *id > 0) else {
                    continue;
                };
                if ev_row["tool"] == "Shell"
                    && ev_row["state"] == "completed"
                    && let Some(detail) = ev_row["detail"].as_str().filter(|s| !s.is_empty())
                {
                    valid_ids.insert(eid);
                    if ev_row["ok"] == false {
                        continue;
                    }
                    let ev_agent = ev_row["agent"].as_str();
                    if ev_agent.is_some_and(|ag| !agent_matches(agent, ag, &service_id)) {
                        continue;
                    }
                    let targets_this_service = ev_agent.is_none()
                        || service_id == "__system__"
                        || ev_agent.is_some_and(|ag| {
                            ag == format!("service-{service_id}")
                                || ag == service_id
                                || ag == "verify"
                                || ag == "contracts"
                        });
                    if targets_this_service
                        && let Some((_, files)) =
                            shell_reads::completed_command(eid, detail, &canon_root)
                    {
                        for f in files {
                            resolved_reads.insert(f, eid);
                        }
                    }
                    if canon_root != canon_sys_root
                        && let Some((_, sys_files)) =
                            shell_reads::completed_command(eid, detail, &canon_sys_root)
                    {
                        for sf in sys_files {
                            if let Some(rel) = sf.strip_prefix(&svc_prefix)
                                && !rel.is_empty()
                            {
                                resolved_reads.insert(rel.to_owned(), eid);
                            }
                        }
                    }
                }
            }
            if last_id <= after {
                break;
            }
            after = last_id;
        }
        (valid_ids, resolved_reads)
    };

    let build_slice = |selected_sid: Option<&str>| -> RunChange {
        let mut opened = BTreeSet::new();
        let mut shell_map: BTreeMap<String, u64> = BTreeMap::new();
        let mut files_map: BTreeMap<String, Change> = BTreeMap::new();
        let mut steps = 0usize;
        let mut searched = 0usize;
        let mut commands = 0usize;
        let mut helpers = 0usize;
        let mut graph_asked = false;

        let launch_index = selected_sid.and_then(|sid| {
            launches_meta
                .iter()
                .find(|(id, _, _)| id == sid)
                .map(|(_, num, _)| *num as u64)
        });

        let mut matched_dispatches: Vec<&DispatchEvidence> = Vec::new();
        for ev in &dispatches {
            if let Some(sel) = selected_sid {
                let matches_launch = match ev.launch_id.as_deref() {
                    Some(lid) => lid == sel,
                    None => {
                        launch_index.is_none_or(|idx| ev.attempt == idx || launches_meta.len() == 1)
                    }
                };
                if !matches_launch {
                    continue;
                }
            }
            if !agent_matches(agent, &ev.phase, &ev.service) {
                continue;
            }
            matched_dispatches.push(ev);
        }

        let resolved_session_sid: Option<String> = if let Some(sel) = selected_sid {
            existing_sessions.contains(sel).then(|| sel.to_owned())
        } else {
            let svc_prefix = format!("{service_id}/");
            let shell_owner = matched_dispatches.iter().rev().find_map(|ev| {
                let direct_service = ev.service == service_id
                    || (service_id == "__system__"
                        && (ev.phase == "contracts" || ev.phase == "verify"));
                let has_scoped_shell = ev.shell_reads.keys().any(|raw_p| {
                    scope_evidence_path(raw_p, &service_id, direct_service, &svc_prefix).is_some()
                });
                if has_scoped_shell
                    && let Some(lid) = ev.launch_id.as_deref()
                    && existing_sessions.contains(lid)
                {
                    Some(lid.to_owned())
                } else {
                    None
                }
            });
            shell_owner.or_else(|| {
                launches_meta
                    .iter()
                    .rev()
                    .find(|(id, _, _)| existing_sessions.contains(id))
                    .map(|(id, _, _)| id.clone())
            })
        };

        let session_shell_info = resolved_session_sid
            .as_deref()
            .map(collect_session_shell_events);

        for ev in &matched_dispatches {
            steps += ev.steps;
            searched += ev.searched;
            commands += ev.commands;
            helpers += ev.helpers;
            graph_asked |= ev.graph_asked;

            let direct_service = ev.service == service_id
                || (service_id == "__system__"
                    && (ev.phase == "contracts" || ev.phase == "verify"));
            let svc_prefix = format!("{service_id}/");

            for raw_path in &ev.opened {
                if let Some(scoped) =
                    scope_evidence_path(raw_path, &service_id, direct_service, &svc_prefix)
                {
                    opened.insert(scoped);
                }
            }
            for (raw_path, eid) in &ev.shell_reads {
                if let Some(scoped) =
                    scope_evidence_path(raw_path, &service_id, direct_service, &svc_prefix)
                {
                    opened.insert(scoped.clone());
                    if let Some((ref valid_ids, ref resolved_reads)) = session_shell_info {
                        let same_session = resolved_session_sid
                            .as_deref()
                            .is_none_or(|sid| ev.launch_id.as_deref().is_none_or(|lid| lid == sid));
                        if same_session && *eid > 0 && valid_ids.contains(&(*eid as u64)) {
                            shell_map.insert(scoped, *eid as u64);
                        } else if let Some(&sql_eid) = resolved_reads.get(&scoped) {
                            shell_map.insert(scoped, sql_eid);
                        }
                    } else if launches_meta.is_empty() && *eid > 0 {
                        shell_map.insert(scoped, *eid as u64);
                    }
                }
            }
            for ch in ev.git_changed.iter().chain(ev.ledger_edited.iter()) {
                if let Some(scoped) =
                    scope_evidence_path(&ch.path, &service_id, direct_service, &svc_prefix)
                {
                    let status = ch.status.chars().next().unwrap_or('M');
                    files_map.entry(scoped.clone()).or_insert(Change {
                        path: scoped,
                        status,
                        from: ch.from.clone(),
                    });
                }
            }
        }

        if let Some((_, ref resolved_reads)) = session_shell_info {
            for (scoped, sql_eid) in resolved_reads {
                opened.insert(scoped.clone());
                shell_map.entry(scoped.clone()).or_insert(*sql_eid);
            }
        }

        if files_map.is_empty()
            && selected_sid.is_none()
            && agent.map(str::trim).filter(|s| !s.is_empty()).is_none()
            && let Some(base_sha) = record["baseline_commits"][&service_id].as_str()
        {
            for ch in git_changes_since(&canon_root, Some(base_sha)) {
                let clean = ch
                    .path
                    .replace('\\', "/")
                    .trim_start_matches("./")
                    .to_owned();
                if !clean.is_empty() && !is_runtime_artifact_path(&clean) {
                    files_map.insert(
                        clean.clone(),
                        Change {
                            path: clean,
                            status: ch.status.chars().next().unwrap_or('M'),
                            from: ch.from,
                        },
                    );
                }
            }
        }

        let files: Vec<Change> = files_map.into_values().collect();
        let title = record["goal"]
            .as_str()
            .or_else(|| record["title"].as_str())
            .unwrap_or("System change");
        let status = record["status"]
            .as_str()
            .or_else(|| run_state["status"].as_str())
            .unwrap_or("completed");
        let told = json!({
            "session": resolved_session_sid,
            "run_id": run_id,
            "service": service_id,
            "title": title,
            "provider": record["provider"],
            "status": status,
            "steps": steps,
            "opened": opened.len(),
            "edited": files.len(),
            "shell_opened": shell_map.len(),
            "outside": 0,
            "searched": searched,
            "commands": commands,
            "helpers": helpers,
            "graph_asked": graph_asked,
            "limited": false,
            "elsewhere": false,
            "agent": agent.filter(|a| !a.is_empty()),
            "agents": agents_list,
            "launches": launches_json,
            "launch": selected_sid,
            "compare_launch": compare_launch_sid,
        });

        RunChange {
            resolved: Arc::new(Resolved {
                target: Target::SysRun(run_id.to_owned()),
                kind: "run",
                name: title.chars().take(80).collect(),
                title: None,
                pull: None,
                head: None,
                base: None,
                revision: None,
                prefix: String::new(),
                files,
                notes: Vec::new(),
            }),
            opened,
            shell_reads: shell_map,
            told,
        }
    };

    let mut primary = build_slice(primary_launch_sid.as_deref());
    primary.told["launch"] = json!(primary_launch_sid);
    primary.told["compare_launch"] = json!(compare_launch_sid);
    let comparison = compare_launch_sid
        .as_deref()
        .map(|cid| build_slice(Some(cid)));
    Ok((primary, comparison))
}

fn scope_evidence_path(
    raw: &str,
    service_id: &str,
    direct_service: bool,
    svc_prefix: &str,
) -> Option<String> {
    let norm = raw.replace('\\', "/");
    let trimmed = norm.trim_start_matches("./");
    if trimmed.is_empty() || trimmed.starts_with("outside:") {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix(svc_prefix) {
        let clean = rest.trim_start_matches("./");
        return (!clean.is_empty()).then(|| clean.to_owned());
    }
    if direct_service {
        if service_id == "__system__" && trimmed.contains(':') {
            return None;
        }
        return Some(trimmed.to_owned());
    }
    None
}

pub fn load_system_focus_value(
    store: &SessionStore,
    run_id: &str,
    project: &str,
) -> Result<Option<Value>> {
    let rid = run_id
        .strip_prefix("sysrun:")
        .or_else(|| run_id.strip_prefix("system-run:"))
        .unwrap_or(run_id)
        .trim();
    if rid.is_empty() {
        return Ok(None);
    }
    if let Ok(record) = load_system_run_record(store, rid) {
        let pid = record["project_id"].as_str().unwrap_or("");
        let cfg = record["config_path"].as_str();
        if let Ok((_, _, targets)) = resolve_all_targets(store, None, None, pid, cfg) {
            for t in targets {
                if t.graph_id == project {
                    let key = format!("{SYSTEM_FOCUS_PREFIX}{rid}.{}", t.service_id);
                    if let Some(val) = store.setting(&key)?
                        && !val.is_null()
                    {
                        return Ok(Some(val));
                    }
                }
            }
        }
    }
    let direct_key = format!("{SYSTEM_FOCUS_PREFIX}{rid}.{project}");
    if let Some(val) = store.setting(&direct_key)?
        && !val.is_null()
    {
        return Ok(Some(val));
    }
    let agg_key = format!("{SYSTEM_FOCUS_PREFIX}{rid}.__aggregate__");
    if let Some(val) = store.setting(&agg_key)?
        && !val.is_null()
    {
        return Ok(Some(val));
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Global Weak Registry for Live In-Progress Dispatches
// ---------------------------------------------------------------------------

static ACTIVE_BRIDGES: Mutex<Vec<Weak<HarnessCodeGraphBridge>>> = Mutex::new(Vec::new());

fn register_global_bridge(bridge: &Arc<HarnessCodeGraphBridge>) {
    if let Ok(mut list) = ACTIVE_BRIDGES.lock() {
        list.retain(|w| w.strong_count() > 0);
        list.push(Arc::downgrade(bridge));
    }
}

fn active_bridge_dispatches(run_id: &str) -> Option<Vec<DispatchEvidence>> {
    let bridges: Vec<Arc<HarnessCodeGraphBridge>> = ACTIVE_BRIDGES
        .lock()
        .ok()?
        .iter()
        .filter_map(Weak::upgrade)
        .collect();
    let mut out = Vec::new();
    for bridge in bridges {
        out.extend(bridge.snapshot_active_dispatches(run_id));
    }
    (!out.is_empty()).then_some(out)
}

// ---------------------------------------------------------------------------
// `HarnessCodeGraphBridge` (`impl infra_system::CodeGraphBridge`)
// ---------------------------------------------------------------------------

struct ActiveDispatch {
    ctx: DispatchGraphContext,
    enricher: Enricher,
    service_trackers: BTreeMap<String, LiveFocusTracker>,
    service_roots: BTreeMap<String, PathBuf>,
    shell_opened: BTreeSet<String>,
    shell_reads: BTreeMap<String, i64>,
    shell_events: Vec<Value>,
    file_touches: Vec<Value>,
    plan_items: Vec<Value>,
    command_runs: Vec<Value>,
    pending_commands: BTreeMap<String, String>,
    pending_paths: BTreeMap<String, (String, String)>,
    fallback_event_seq: i64,
    graph_asked: bool,
}

impl ActiveDispatch {
    fn snapshot_evidence(&self, git_changed: Vec<EvidenceFileChange>) -> DispatchEvidence {
        let (raw_opened, raw_edited) = self.enricher.ledger.files();
        let mut opened: BTreeSet<String> = raw_opened
            .into_iter()
            .map(|p| p.replace('\\', "/").trim_start_matches("./").to_owned())
            .filter(|p| !p.is_empty() && !p.starts_with("outside:"))
            .collect();
        for p in &self.shell_opened {
            opened.insert(p.clone());
        }
        for k in self.shell_reads.keys() {
            opened.insert(k.clone());
        }
        if self.ctx.service == "__system__" {
            for (sid, tracker) in &self.service_trackers {
                if sid != "__system__" {
                    for p in &tracker.focus().target_paths {
                        opened.insert(format!("{sid}/{p}"));
                    }
                }
            }
        }
        let ledger_edited = raw_edited
            .into_iter()
            .filter(|(p, _)| !p.starts_with("outside:"))
            .map(|(p, st)| EvidenceFileChange {
                path: p.replace('\\', "/").trim_start_matches("./").to_owned(),
                status: st.to_string(),
                from: None,
            })
            .collect();
        let receipt = self.enricher.ledger.receipt();
        let primary_focus = self.enricher.ledger.graph_focus();
        let mut created_entities: Vec<Value> = primary_focus
            .created_entities
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .collect();
        for (sid, tracker) in &self.service_trackers {
            if *sid != self.ctx.service {
                for entity in &tracker.focus().created_entities {
                    if let Ok(mut val) = serde_json::to_value(entity) {
                        val["service"] = json!(sid);
                        if !created_entities.contains(&val) {
                            created_entities.push(val);
                        }
                    }
                }
            }
        }

        DispatchEvidence {
            phase: self.ctx.phase.clone(),
            attempt: self.ctx.attempt,
            launch_id: self.ctx.launch_id.clone(),
            service: self.ctx.service.clone(),
            root: self.ctx.root.to_string_lossy().into_owned(),
            base_head: self.ctx.base_head.clone(),
            head_after: self.ctx.head_after.clone(),
            git_changed,
            ledger_edited,
            opened: opened.into_iter().collect(),
            shell_reads: self.shell_reads.clone(),
            shell_events: self.shell_events.clone(),
            graph_asked: self.graph_asked || receipt["graph"].as_u64().unwrap_or(0) > 0,
            steps: receipt["steps"].as_u64().unwrap_or(0) as usize,
            searched: receipt["searched"].as_u64().unwrap_or(0) as usize,
            commands: receipt["commands"].as_u64().unwrap_or(0) as usize,
            helpers: receipt["helpers"].as_u64().unwrap_or(0) as usize,
            limited: receipt["limited"] == true,
            created_entities,
            target_folders: primary_focus.target_folders.clone(),
            target_paths: primary_focus.target_paths.clone(),
            target_nodes: primary_focus.target_nodes.clone(),
            file_touches: self.file_touches.clone(),
            plan_items: self.plan_items.clone(),
            command_runs: self.command_runs.clone(),
        }
    }
}

#[derive(Default)]
struct BridgeState {
    sessions: BTreeMap<String, (String, String)>,
    dispatches: BTreeMap<(String, String, u64), ActiveDispatch>,
    /// How many events of each run passed the bridge: what a run's
    /// evidence reaches is worked out anew once another one did.
    events: BTreeMap<String, u64>,
}

pub struct HarnessCodeGraphBridge {
    store: SessionStore,
    graphs: Arc<Graphs>,
    state: Mutex<BridgeState>,
    /// What the evidence of each run was last found to reach.
    reach: Mutex<BTreeMap<String, Reach>>,
    /// How often that was worked out, not taken from what was kept.
    reached: std::sync::atomic::AtomicU64,
}

impl HarnessCodeGraphBridge {
    pub fn new(store: SessionStore, graphs: Arc<Graphs>) -> Arc<Self> {
        let me = Arc::new(Self {
            store,
            graphs,
            state: Mutex::new(BridgeState::default()),
            reach: Mutex::new(BTreeMap::new()),
            reached: std::sync::atomic::AtomicU64::new(0),
        });
        register_global_bridge(&me);
        me
    }

    pub fn register_session(&self, run_id: &str, sid: &str, generation: &str) {
        if let Ok(mut guard) = self.state.lock() {
            guard
                .sessions
                .insert(run_id.to_owned(), (sid.to_owned(), generation.to_owned()));
        }
    }

    pub fn unregister_session(&self, run_id: &str) {
        if let Ok(mut guard) = self.state.lock() {
            guard.sessions.remove(run_id);
        }
    }

    /// Receives every emitted session event (along with its inserted SQLite `events.id`) from
    /// `SystemWorkflow`'s `ActivityStream::new_with_ids` so `shell_reads` stores exact SQLite row IDs.
    pub fn on_emitted_session_event(&self, run_id: &str, event_id: Option<u64>, event: &Value) {
        if let Ok(mut guard) = self.state.lock() {
            *guard.events.entry(run_id.to_owned()).or_default() += 1;
        }
        let Some(eid) = event_id else {
            return;
        };
        if event["kind"] != "agent_activity"
            || event["type"] != "tool"
            || event["state"] != "completed"
        {
            return;
        }
        let Some(detail) = event["detail"].as_str().filter(|s| !s.is_empty()) else {
            return;
        };
        let phase_opt = event["phase_id"]
            .as_str()
            .or_else(|| event["agent"].as_str());
        let attempt_opt = event["attempt"].as_u64();

        if let Ok(mut guard) = self.state.lock() {
            for ((rid, phase, attempt), active) in guard.dispatches.iter_mut() {
                if rid != run_id {
                    continue;
                }
                if let Some(p) = phase_opt
                    && p != phase
                {
                    continue;
                }
                if let Some(a) = attempt_opt
                    && a != *attempt
                {
                    continue;
                }
                if let Some((_, files)) =
                    shell_reads::completed_command(eid, detail, &active.ctx.root)
                {
                    for file in files {
                        active.shell_reads.insert(file.clone(), eid as i64);
                        active.shell_events.push(json!({
                            "id": eid,
                            "path": file,
                            "detail": detail,
                        }));
                    }
                }
                if active.ctx.service == "__system__" {
                    for (sid, sroot) in &active.service_roots {
                        if sid == "__system__" {
                            continue;
                        }
                        if let Some((_, files)) = shell_reads::completed_command(eid, detail, sroot)
                        {
                            for file in files {
                                let qual = format!("{sid}/{file}");
                                active.shell_reads.insert(qual.clone(), eid as i64);
                                active.shell_events.push(json!({
                                    "id": eid,
                                    "path": qual,
                                    "detail": detail,
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    fn snapshot_active_dispatches(&self, run_id: &str) -> Vec<DispatchEvidence> {
        let Ok(guard) = self.state.lock() else {
            return Vec::new();
        };
        guard
            .dispatches
            .iter()
            .filter(|((rid, _, _), _)| rid == run_id)
            .map(|(_, d)| d.snapshot_evidence(Vec::new()))
            .collect()
    }

    fn emit_focus_event(
        &self,
        run_id: &str,
        session_override: Option<&str>,
        service: &str,
        phase_id: &str,
        attempt: u64,
        focus: &GraphFocus,
    ) {
        let _ = self.persist_service_focus(run_id, service, focus);
        let session_pair = self
            .state
            .lock()
            .ok()
            .and_then(|g| g.sessions.get(run_id).cloned());
        let event = json!({
            "kind": "graph_focus",
            "run_id": run_id,
            "service": service,
            "phase_id": phase_id,
            "attempt": attempt,
            "phase": focus.phase,
            "summary": focus.summary,
            "target_folders": focus.target_folders,
            "target_paths": focus.target_paths,
            "target_nodes": focus.target_nodes,
            "created_entities": focus.created_entities,
            "focus": focus,
        });
        if let Some((sid, generation)) = session_pair {
            let _ = self.store.guarded_event(&sid, &generation, &event);
        } else if let Some(sid) = session_override.filter(|s| !s.is_empty())
            && let Ok(sess) = self.store.get(sid)
            && let Some(generation) = sess["generation"].as_str()
        {
            let _ = self.store.guarded_event(sid, generation, &event);
        }
    }

    fn persist_service_focus(&self, run_id: &str, service: &str, focus: &GraphFocus) -> Result<()> {
        let key = format!("{SYSTEM_FOCUS_PREFIX}{run_id}.{service}");
        let val = serde_json::to_value(focus)?;
        self.store.set_setting(&key, &val)?;

        let agg_key = format!("{SYSTEM_FOCUS_PREFIX}{run_id}.__aggregate__");
        let mut agg: GraphFocus = self
            .store
            .setting(&agg_key)?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        agg.phase = focus.phase.clone();
        if !focus.summary.is_empty() {
            agg.summary = focus.summary.clone();
        }
        let prefix = if service == "__system__" {
            String::new()
        } else {
            format!("{service}/")
        };
        for f in &focus.target_folders {
            let q = if prefix.is_empty() {
                f.clone()
            } else {
                format!("{prefix}{f}")
            };
            if !agg.target_folders.contains(&q) && agg.target_folders.len() < 24 {
                agg.target_folders.push(q);
            }
        }
        for p in &focus.target_paths {
            let q = if prefix.is_empty() {
                p.clone()
            } else {
                format!("{prefix}{p}")
            };
            if !agg.target_paths.contains(&q) && agg.target_paths.len() < 48 {
                agg.target_paths.push(q);
            }
        }
        for n in &focus.target_nodes {
            if !agg.target_nodes.contains(n) && agg.target_nodes.len() < 48 {
                agg.target_nodes.push(n.clone());
            }
        }
        for ent in &focus.created_entities {
            let mut copy: CreatedEntity = ent.clone();
            if !prefix.is_empty() && !copy.path.starts_with(&prefix) {
                copy.path = format!("{prefix}{}", copy.path);
            }
            if !agg
                .created_entities
                .iter()
                .any(|e| e.id == copy.id && e.path == copy.path)
                && agg.created_entities.len() < 64
            {
                agg.created_entities.push(copy);
            }
        }
        self.store
            .set_setting(&agg_key, &serde_json::to_value(&agg)?)?;
        Ok(())
    }
}

fn prompt_seed_from_ctx(ctx: &DispatchGraphContext) -> String {
    let mut parts = Vec::new();
    if let Some(goal) = ctx.state["goal"]
        .as_str()
        .or_else(|| ctx.plan["goal"].as_str())
        .filter(|s| !s.is_empty())
    {
        parts.push(goal.to_owned());
    }
    if let Some(summary) = ctx.plan["summary"].as_str().filter(|s| !s.is_empty()) {
        parts.push(summary.to_owned());
    }
    if let Some(steps) = ctx.plan["steps"].as_array() {
        for step in steps {
            if step["service"].as_str() == Some(ctx.service.as_str())
                && let Some(text) = step["task"]
                    .as_str()
                    .or_else(|| step["summary"].as_str())
                    .filter(|s| !s.is_empty())
            {
                parts.push(text.to_owned());
            }
        }
    }
    parts.join("\n")
}

impl CodeGraphBridge for HarnessCodeGraphBridge {
    fn on_run_start(
        &self,
        _run_id: &str,
        state: &Value,
        system: &System,
    ) -> infra_core::Result<()> {
        if let Ok((by_root, _)) = registered_project_roots(&self.store) {
            let selected: Vec<String> = state["selected"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            for target in
                targets_from_loaded_system(Some(&self.graphs), &by_root, "", "system.json", system)
            {
                if target.service_id == "__system__"
                    || selected.is_empty()
                    || selected.contains(&target.service_id)
                {
                    let _ = self.graphs.refresh(&target.graph_id, Trigger::Launch);
                }
            }
        }
        Ok(())
    }

    fn offers_for_dispatch(
        &self,
        ctx: &DispatchGraphContext,
    ) -> infra_core::Result<Vec<ServiceGraphOffer>> {
        if ctx.tell_agents_graph == Some(false) {
            return Ok(Vec::new());
        }
        let Some((_, targets)) =
            resolve_targets_from_system_file(&self.store, Some(&self.graphs), &ctx.system_file)
        else {
            return Ok(Vec::new());
        };
        let force_offer = ctx.tell_agents_graph == Some(true);
        let mut wanted = BTreeSet::new();
        wanted.insert(ctx.service.clone());
        if ctx.phase == "contracts" || ctx.phase == "verify" || ctx.access == "all" {
            wanted.insert("__system__".to_owned());
            for sid in &ctx.selected {
                wanted.insert(sid.clone());
            }
        }
        for sid in &ctx.readable {
            wanted.insert(sid.clone());
        }

        let mut offers = Vec::new();
        for target in targets {
            if !wanted.contains(&target.service_id) {
                continue;
            }
            let Ok(Some(data)) = service_offer(
                &self.store,
                &target.graph_id,
                &target.service_id,
                force_offer,
            ) else {
                continue;
            };
            let edges = self
                .graphs
                .status(&target.graph_id, false)
                .ok()
                .and_then(|st| st["graph"]["edges"].as_u64())
                .unwrap_or(0) as usize;
            offers.push(ServiceGraphOffer {
                service: target.service_id,
                command: data.program_path.to_string_lossy().into_owned(),
                graph_path: data.graph_path,
                nodes: data.nodes.unwrap_or(0) as usize,
                edges,
                commit: data.commit,
                readable_paths: data.readable_paths,
                claude_allow_rules: data.claude_allow_rules,
                env: data.environment.into_iter().collect(),
                prompt_instructions: Some(data.prompt_block),
            });
        }
        Ok(offers)
    }

    fn on_dispatch_start(&self, ctx: &DispatchGraphContext) -> infra_core::Result<()> {
        let mut service_roots = ctx.roots.clone();
        service_roots
            .entry(ctx.service.clone())
            .or_insert_with(|| ctx.root.clone());

        let prompt_seed = prompt_seed_from_ctx(ctx);
        let mut service_trackers = BTreeMap::new();
        if let Some((_, targets)) =
            resolve_targets_from_system_file(&self.store, Some(&self.graphs), &ctx.system_file)
        {
            for target in &targets {
                service_roots
                    .entry(target.service_id.clone())
                    .or_insert_with(|| target.root.clone());
                if target.service_id == ctx.service
                    || ctx.phase == "contracts"
                    || ctx.phase == "verify"
                {
                    let _ = self.graphs.refresh(&target.graph_id, Trigger::Launch);
                }
                if target.service_id == ctx.service
                    || (ctx.phase == "verify" && ctx.selected.contains(&target.service_id))
                {
                    let idx = self.graphs.live_index(&target.graph_id);
                    let predicted =
                        predict_targets(&target.root, idx.as_deref(), &prompt_seed, &[], &[]);
                    let mut tracker = LiveFocusTracker::new(target.root.clone());
                    tracker.seed_focus(predicted);
                    self.emit_focus_event(
                        &ctx.run_id,
                        ctx.launch_id.as_deref(),
                        &target.service_id,
                        &ctx.phase,
                        ctx.attempt,
                        tracker.focus(),
                    );
                    service_trackers.insert(target.service_id.clone(), tracker);
                } else {
                    service_trackers.insert(
                        target.service_id.clone(),
                        LiveFocusTracker::new(target.root.clone()),
                    );
                }
            }
        }

        let ws_str = ctx.root.to_string_lossy().into_owned();
        let home = crate::run_activity::user_home();
        let mut enricher = Enricher::new(
            &ctx.provider,
            &ws_str,
            None,
            &[],
            home.as_deref(),
            None,
            false,
        );
        if let Some(primary_tracker) = service_trackers.get(&ctx.service) {
            enricher
                .ledger
                .seed_graph_focus(primary_tracker.focus().clone());
        } else {
            let predicted = predict_targets(&ctx.root, None, &prompt_seed, &[], &[]);
            enricher.ledger.seed_graph_focus(predicted.clone());
            self.emit_focus_event(
                &ctx.run_id,
                ctx.launch_id.as_deref(),
                &ctx.service,
                &ctx.phase,
                ctx.attempt,
                &predicted,
            );
        }

        if let Ok(mut guard) = self.state.lock() {
            guard.dispatches.insert(
                (ctx.run_id.clone(), ctx.phase.clone(), ctx.attempt),
                ActiveDispatch {
                    ctx: ctx.clone(),
                    enricher,
                    service_trackers,
                    service_roots,
                    shell_opened: BTreeSet::new(),
                    shell_reads: BTreeMap::new(),
                    shell_events: Vec::new(),
                    file_touches: Vec::new(),
                    plan_items: Vec::new(),
                    command_runs: Vec::new(),
                    pending_commands: BTreeMap::new(),
                    pending_paths: BTreeMap::new(),
                    fallback_event_seq: 1,
                    graph_asked: false,
                },
            );
        }
        Ok(())
    }

    fn on_dispatch_signal(
        &self,
        ctx: &DispatchGraphContext,
        raw_line: &[u8],
    ) -> infra_core::Result<()> {
        let Ok(parsed) = crate::strict_json::parse(raw_line, 2 * 1024 * 1024) else {
            return Ok(());
        };
        let items = normalize(&ctx.provider, &parsed, true);
        if items.is_empty() {
            return Ok(());
        }
        let raw_codex_cmd = parsed["item"]["command"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let mut claude_raw_cmds: BTreeMap<String, String> = BTreeMap::new();
        if let Some(blocks) = parsed["message"]["content"].as_array() {
            for b in blocks {
                if b["type"] == "tool_use"
                    && let (Some(cid), Some(cmd)) =
                        (b["id"].as_str(), b["input"]["command"].as_str())
                {
                    claude_raw_cmds.insert(cid.to_owned(), cmd.to_owned());
                }
            }
        }

        let mut focus_emissions: Vec<(String, GraphFocus)> = Vec::new();
        if let Ok(mut guard) = self.state.lock() {
            let has_live_session =
                guard.sessions.contains_key(&ctx.run_id) || ctx.launch_id.is_some();
            let key = (ctx.run_id.clone(), ctx.phase.clone(), ctx.attempt);
            let Some(active) = guard.dispatches.get_mut(&key) else {
                return Ok(());
            };
            for item in items {
                let kind = item["kind"]
                    .as_str()
                    .or_else(|| item["type"].as_str())
                    .unwrap_or("");
                if kind == "plan" {
                    let mut plan_ev = item.clone();
                    plan_ev["kind"] = json!("plan");
                    if let Some(cleaned) = active.enricher.plan(&plan_ev) {
                        active.plan_items.push(json!({
                            "phase": ctx.phase,
                            "service": ctx.service,
                            "attempt": ctx.attempt,
                            "text": cleaned["text"],
                            "items": cleaned["items"],
                        }));
                    }
                    continue;
                }
                if kind != "tool" {
                    continue;
                }

                let raw_targets = item.get("targets").cloned().unwrap_or(Value::Null);
                let mut clean = item;
                clean["kind"] = json!("tool");
                let display = active.enricher.take(&mut clean, 0, 0);
                if let Some(fields) = display.as_ref().and_then(Value::as_object) {
                    for (k, v) in fields {
                        clean[k] = v.clone();
                    }
                }

                let state_str = clean["state"]
                    .as_str()
                    .or_else(|| raw_targets["state"].as_str())
                    .unwrap_or("started");
                let call_id = raw_targets["call"]
                    .as_str()
                    .or_else(|| clean["call"].as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let mut tool_name = clean["tool"]
                    .as_str()
                    .or_else(|| raw_targets["tool"].as_str())
                    .unwrap_or("")
                    .to_owned();
                if state_str == "started"
                    && let Some(ref cid) = call_id
                    && let Some(raw_p) = raw_targets["path"].as_str()
                {
                    active
                        .pending_paths
                        .insert(cid.clone(), (tool_name.clone(), raw_p.to_owned()));
                }
                let completed_pending_path = if state_str == "completed" {
                    call_id
                        .as_ref()
                        .and_then(|cid| active.pending_paths.remove(cid))
                } else {
                    None
                };
                if tool_name.is_empty()
                    && let Some((ref saved_tool, _)) = completed_pending_path
                {
                    tool_name = saved_tool.clone();
                }

                if let Some(path_str) = clean["path"].as_str()
                    && active.file_touches.len() < 200
                {
                    active.file_touches.push(json!({
                        "phase": ctx.phase,
                        "service": ctx.service,
                        "attempt": ctx.attempt,
                        "tool": tool_name,
                        "state": state_str,
                        "path": path_str,
                        "outside": clean["outside"] == true,
                    }));
                }
                if let Some(changes) = clean["changes"].as_array() {
                    for ch in changes {
                        if let Some(path_str) = ch["path"].as_str()
                            && active.file_touches.len() < 200
                        {
                            active.file_touches.push(json!({
                                "phase": ctx.phase,
                                "service": ctx.service,
                                "attempt": ctx.attempt,
                                "tool": tool_name,
                                "state": state_str,
                                "path": path_str,
                                "kind": ch["kind"],
                                "outside": ch["outside"] == true,
                            }));
                        }
                    }
                }

                let explicit_cmd = call_id
                    .as_ref()
                    .and_then(|cid| claude_raw_cmds.get(cid).map(String::as_str))
                    .or(raw_codex_cmd.as_deref())
                    .or_else(|| {
                        if matches!(
                            tool_name.to_ascii_lowercase().as_str(),
                            "bash" | "shell" | "command" | "exec_command"
                        ) {
                            raw_targets["detail"]
                                .as_str()
                                .or_else(|| clean["detail"].as_str())
                        } else {
                            None
                        }
                    });
                if let Some(cmd_str) = explicit_cmd {
                    if crate::graph::asked(cmd_str) {
                        active.graph_asked = true;
                    }
                    if state_str == "started"
                        && let Some(ref cid) = call_id
                    {
                        active
                            .pending_commands
                            .insert(cid.clone(), cmd_str.to_owned());
                    }
                }
                let completed_cmd = if state_str == "completed" {
                    let popped = call_id
                        .as_ref()
                        .and_then(|cid| active.pending_commands.remove(cid));
                    explicit_cmd.map(str::to_owned).or(popped)
                } else {
                    None
                };

                if let Some(detail_raw) = completed_cmd.as_deref().filter(|s| !s.is_empty()) {
                    if crate::graph::asked(detail_raw) {
                        active.graph_asked = true;
                    }
                    if active.command_runs.len() < 200 {
                        active.command_runs.push(json!({
                            "phase": ctx.phase,
                            "service": ctx.service,
                            "attempt": ctx.attempt,
                            "command": redact_command(&detail_raw.chars().take(300).collect::<String>()),
                            "ok": clean["ok"] != false && raw_targets["ok"] != false,
                            "exit_code": clean["exit_code"],
                        }));
                    }
                    if clean["ok"] != false && raw_targets["ok"] != false {
                        let seq_id = active.fallback_event_seq;
                        active.fallback_event_seq = active.fallback_event_seq.saturating_add(1);
                        if let Some((_, files)) = shell_reads::completed_command(
                            seq_id as u64,
                            detail_raw,
                            &active.ctx.root,
                        ) {
                            for file in files {
                                active.shell_opened.insert(file.clone());
                                if !has_live_session {
                                    active.shell_reads.entry(file).or_insert(seq_id);
                                }
                            }
                        }
                        if active.ctx.service == "__system__" {
                            for (sid, sroot) in &active.service_roots {
                                if sid == "__system__" {
                                    continue;
                                }
                                if let Some((_, files)) =
                                    shell_reads::completed_command(seq_id as u64, detail_raw, sroot)
                                {
                                    for file in files {
                                        let qual = format!("{sid}/{file}");
                                        active.shell_opened.insert(qual.clone());
                                        if !has_live_session {
                                            active.shell_reads.entry(qual).or_insert(seq_id);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if !active.enricher.take_focus_events().is_empty() {
                    focus_emissions.push((
                        ctx.service.clone(),
                        active.enricher.ledger.graph_focus().clone(),
                    ));
                }

                if raw_targets.is_object() {
                    let effective_raw_path = raw_targets["path"]
                        .as_str()
                        .or_else(|| completed_pending_path.as_ref().map(|(_, p)| p.as_str()));
                    for (sid, sroot) in &active.service_roots {
                        if *sid == ctx.service {
                            continue;
                        }
                        let Some(tracker) = active.service_trackers.get_mut(sid) else {
                            continue;
                        };
                        let sroot_str = sroot.to_string_lossy();
                        let mut routed = json!({
                            "kind": "tool",
                            "state": state_str,
                            "tool": tool_name,
                            "ok": raw_targets["ok"] != false,
                        });
                        let mut matched_any = false;
                        if let Some(raw_path) = effective_raw_path
                            && let (Some(rel), false) = relative(raw_path, &sroot_str, None)
                        {
                            routed["path"] = json!(rel);
                            matched_any = true;
                        }
                        if let Some(arr) = raw_targets["changes"].as_array() {
                            let mut mapped_changes = Vec::new();
                            for ch in arr {
                                if let Some(raw_path) = ch["path"].as_str()
                                    && let (Some(rel), false) = relative(raw_path, &sroot_str, None)
                                {
                                    mapped_changes.push(json!({
                                        "path": rel,
                                        "kind": ch["kind"],
                                    }));
                                    matched_any = true;
                                }
                            }
                            if !mapped_changes.is_empty() {
                                routed["changes"] = json!(mapped_changes);
                            }
                        }
                        if matched_any && tracker.observe_tool_event(&routed).is_some() {
                            focus_emissions.push((sid.clone(), tracker.focus().clone()));
                        }
                    }
                }
            }
        }

        for (service, focus) in focus_emissions {
            self.emit_focus_event(
                &ctx.run_id,
                ctx.launch_id.as_deref(),
                &service,
                &ctx.phase,
                ctx.attempt,
                &focus,
            );
        }
        Ok(())
    }

    fn on_dispatch_finish(
        &self,
        ctx: &DispatchGraphContext,
        _receipt: &Value,
    ) -> infra_core::Result<Option<DispatchEvidence>> {
        let removed = self.state.lock().ok().and_then(|mut g| {
            g.dispatches
                .remove(&(ctx.run_id.clone(), ctx.phase.clone(), ctx.attempt))
        });

        let git_changed = if ctx.mode == "edit" {
            let mut changes = git_changes_since(&ctx.root, ctx.base_head.as_deref());
            for wc in git_worktree_changes(&ctx.root) {
                if !changes.iter().any(|c| c.path == wc.path) {
                    changes.push(wc);
                }
            }
            changes
                .into_iter()
                .filter(|c| !is_runtime_artifact_path(&c.path))
                .collect()
        } else {
            Vec::new()
        };

        let evidence = if let Some(mut active) = removed {
            active.ctx.head_after = ctx.head_after.clone();
            let mut completed_focus = active.enricher.ledger.graph_focus().clone();
            completed_focus.phase = "completed".to_owned();
            self.emit_focus_event(
                &ctx.run_id,
                ctx.launch_id.as_deref(),
                &ctx.service,
                &ctx.phase,
                ctx.attempt,
                &completed_focus,
            );
            for (sid, tracker) in &mut active.service_trackers {
                if *sid != ctx.service && !tracker.focus().is_empty() {
                    tracker.complete();
                    self.emit_focus_event(
                        &ctx.run_id,
                        ctx.launch_id.as_deref(),
                        sid,
                        &ctx.phase,
                        ctx.attempt,
                        tracker.focus(),
                    );
                }
            }
            Some(active.snapshot_evidence(git_changed))
        } else {
            Some(DispatchEvidence {
                phase: ctx.phase.clone(),
                attempt: ctx.attempt,
                launch_id: ctx.launch_id.clone(),
                service: ctx.service.clone(),
                root: ctx.root.to_string_lossy().into_owned(),
                base_head: ctx.base_head.clone(),
                head_after: ctx.head_after.clone(),
                git_changed,
                ..Default::default()
            })
        };

        if ctx.mode == "edit"
            && let Some((_, targets)) =
                resolve_targets_from_system_file(&self.store, Some(&self.graphs), &ctx.system_file)
        {
            for target in targets {
                if target.service_id == ctx.service || ctx.writable.contains(&target.service_id) {
                    let _ = self.graphs.refresh(&target.graph_id, Trigger::Finish);
                }
            }
        }

        Ok(evidence)
    }

    fn on_run_finish(
        &self,
        run_id: &str,
        _state: &Value,
        system: &System,
    ) -> infra_core::Result<()> {
        self.unregister_session(run_id);
        if let Ok((by_root, _)) = registered_project_roots(&self.store) {
            for target in
                targets_from_loaded_system(Some(&self.graphs), &by_root, "", "system.json", system)
            {
                let _ = self.graphs.refresh(&target.graph_id, Trigger::Finish);
            }
        }
        Ok(())
    }

    fn enrich_run_view(
        &self,
        run_id: &str,
        system: Option<&System>,
        state: Option<&Value>,
        dispatches: &[DispatchEvidence],
        view: &mut Value,
    ) -> infra_core::Result<()> {
        if run_id.is_empty() {
            return Ok(());
        }
        let pid = view["project_id"].as_str().unwrap_or("");
        let cfg = view["config_path"].as_str();

        let mut all_dispatches = dispatches.to_vec();
        for live in self.snapshot_active_dispatches(run_id) {
            if !all_dispatches
                .iter()
                .any(|d| d.phase == live.phase && d.attempt == live.attempt)
            {
                all_dispatches.push(live);
            }
        }

        let targets = if let Some(sys) = system
            && let Ok((by_root, _)) = registered_project_roots(&self.store)
        {
            targets_from_loaded_system(
                Some(&self.graphs),
                &by_root,
                pid,
                cfg.unwrap_or("system.json"),
                sys,
            )
        } else {
            resolve_all_targets(&self.store, Some(&self.graphs), None, pid, cfg)
                .map(|(_, _, t)| t)
                .unwrap_or_default()
        };

        // 1. Attach `graph_focus` (`services` + `aggregate`).
        let mut by_service = serde_json::Map::new();
        for target in &targets {
            let key = format!("{SYSTEM_FOCUS_PREFIX}{run_id}.{}", target.service_id);
            if let Ok(Some(val)) = self.store.setting(&key)
                && !val.is_null()
            {
                by_service.insert(target.service_id.clone(), val);
            }
        }
        let agg = self
            .store
            .setting(&format!("{SYSTEM_FOCUS_PREFIX}{run_id}.__aggregate__"))
            .ok()
            .flatten()
            .unwrap_or(Value::Null);
        view["graph_focus"] = json!({
            "services": by_service,
            "aggregate": agg,
        });

        // 2. Enrich `evidence.unopened_callers` and `cross_service_unopened` from ready member-service Graphify indexes.
        if let (Some(sys), Some(st)) = (system, state) {
            // The evidence of a run is summed up from its dispatches, with a
            // look at the checkout of every member, and what it reaches is
            // read from the graph of every edited service. A run that is
            // going on is asked for all of it about once a second. The same
            // evidence over the same graphs gives the same answer, so the
            // answer is kept until either of them changes.
            let key = self.reach_key(
                run_id,
                sys,
                st,
                &all_dispatches,
                &targets,
                &view["evidence"],
            );
            if let Some((evidence, cross)) = self.kept_reach(run_id, key) {
                view["evidence"] = evidence;
                view["cross_service_unopened"] = cross;
                return Ok(());
            }
            let clean_dispatches: Vec<DispatchEvidence> = all_dispatches
                .iter()
                .cloned()
                .map(|mut ev| {
                    ev.git_changed
                        .retain(|c| !is_runtime_artifact_path(&c.path));
                    ev.ledger_edited
                        .retain(|c| !is_runtime_artifact_path(&c.path));
                    ev
                })
                .collect();
            if !clean_dispatches.is_empty() || !view["evidence"].is_object() {
                view["evidence"] = summarize_run_evidence(Some(sys), st, &clean_dispatches);
            }
            let existing = view["evidence"]["cross_service_unopened"].clone();
            let (unopened_callers, merged_cross) = self.reach(
                run_id,
                sys,
                &targets,
                &clean_dispatches,
                &all_dispatches,
                &existing,
            );
            if view["evidence"].is_object() {
                view["evidence"]["unopened_callers"] = json!(unopened_callers);
                view["evidence"]["cross_service_unopened"] = json!(merged_cross);
                view["cross_service_unopened"] = view["evidence"]["cross_service_unopened"].clone();
            }
            self.keep_reach(
                run_id,
                key,
                &view["evidence"],
                &view["cross_service_unopened"],
            );
        }
        Ok(())
    }
}

/// How long what a run's evidence reaches is kept while the run goes on,
/// whatever the evidence says: a guard for an event that was stored without
/// passing the bridge.
const REACH_KEPT: std::time::Duration = std::time::Duration::from_secs(15);
/// Runs whose reach is kept at once; the least recently worked out gives way.
const REACH_RUNS: usize = 32;

/// The evidence of a run as it was last summed up, with what it was found
/// to reach, and what that was worked out from.
struct Reach {
    key: u64,
    at: std::time::Instant,
    evidence: Value,
    cross: Value,
}

impl HarnessCodeGraphBridge {
    /// What the evidence of a run and its reach are worked out from, as
    /// one number: the evidence of every dispatch, the summary the view
    /// came with, the system and its state, the published graph of every
    /// member, and how many events of the run passed the bridge.
    fn reach_key(
        &self,
        run_id: &str,
        sys: &System,
        state: &Value,
        dispatches: &[DispatchEvidence],
        targets: &[ResolvedServiceTarget],
        existing: &Value,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(dispatches)
            .unwrap_or_default()
            .hash(&mut hasher);
        existing.to_string().hash(&mut hasher);
        state["status"].to_string().hash(&mut hasher);
        sys.config.hash(&mut hasher);
        sys.config_data.to_string().hash(&mut hasher);
        for edge in &sys.edges {
            edge.to_string().hash(&mut hasher);
        }
        sys.snapshot.hash(&mut hasher);
        for target in targets {
            target.service_id.hash(&mut hasher);
            target.graph_id.hash(&mut hasher);
            self.graphs.live_stamp(&target.graph_id).hash(&mut hasher);
        }
        let events = self
            .state
            .lock()
            .ok()
            .and_then(|guard| guard.events.get(run_id).copied())
            .unwrap_or(0);
        events.hash(&mut hasher);
        hasher.finish()
    }
    /// The evidence kept for a run, where it was worked out from what
    /// `key` stands for and not long ago.
    fn kept_reach(&self, run_id: &str, key: u64) -> Option<(Value, Value)> {
        let guard = self.reach.lock().ok()?;
        let kept = guard.get(run_id)?;
        (kept.key == key && kept.at.elapsed() < REACH_KEPT)
            .then(|| (kept.evidence.clone(), kept.cross.clone()))
    }
    fn keep_reach(&self, run_id: &str, key: u64, evidence: &Value, cross: &Value) {
        let Ok(mut guard) = self.reach.lock() else {
            return;
        };
        if guard.len() >= REACH_RUNS && !guard.contains_key(run_id) {
            let oldest = guard
                .iter()
                .min_by_key(|(_, kept)| kept.at)
                .map(|(run, _)| run.clone());
            if let Some(oldest) = oldest {
                guard.remove(&oldest);
            }
        }
        guard.insert(
            run_id.to_owned(),
            Reach {
                key,
                at: std::time::Instant::now(),
                evidence: evidence.clone(),
                cross: cross.clone(),
            },
        );
    }
    /// What the evidence of a run reaches beyond what the run opened: in
    /// each edited service the files that use what was changed and were
    /// not opened, and across services the consumers of its contracts.
    fn reach(
        &self,
        run_id: &str,
        sys: &System,
        targets: &[ResolvedServiceTarget],
        clean_dispatches: &[DispatchEvidence],
        all_dispatches: &[DispatchEvidence],
        existing: &Value,
    ) -> (Vec<Value>, Vec<Value>) {
        self.reached
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let mut impact_summaries = build_run_scoped_summaries(sys, clean_dispatches);
        let edited_services: BTreeSet<String> = impact_summaries
            .iter()
            .filter(|(sid, s)| *sid != "__system__" && !s.changed_files.is_empty())
            .map(|(sid, _)| sid.clone())
            .collect();

        let mut unopened_callers = Vec::new();
        let target_sysrun = Target::SysRun(run_id.to_owned());
        for target in targets {
            if target.service_id == "__system__" || !edited_services.contains(&target.service_id) {
                continue;
            }
            let Some(idx) = self.graphs.live_index(&target.graph_id) else {
                continue;
            };
            let mut added_for_service = false;
            if let Ok(map_val) = self.graphs.map(
                &target.graph_id,
                Some(&target_sysrun),
                &crate::graph::Shown {
                    scope: "",
                    focus: None,
                    focus_session: None,
                    certain: false,
                    filter: "",
                    followed: None,
                    notests: false,
                    spread: &[],
                    link: None,
                    path: None,
                    longer: 0,
                    failing: &[],
                    cycle: None,
                    base: None,
                    shared: None,
                    patterns: false,
                    history: false,
                    modules: false,
                    no_incoming: false,
                    launch: None,
                    compare_launch: None,
                    agent: None,
                    scenario: None,
                    pattern: None,
                    whole: false,
                    ..Default::default()
                },
            ) && let Some(items) = map_val["run"]["reach"]["rows"].as_array()
                && !items.is_empty()
            {
                for item in items.iter().take(16) {
                    let mut entry = item.clone();
                    entry["service"] = json!(target.service_id);
                    unopened_callers.push(entry);
                    added_for_service = true;
                }
            }
            if !added_for_service
                && let Some(svc_summary) = impact_summaries.get(&target.service_id)
            {
                let edited_ids: BTreeSet<u32> = svc_summary
                    .changed_files
                    .iter()
                    .filter_map(|ch| idx.file_id(&ch.path))
                    .collect();
                let opened_paths = &svc_summary.opened_files;
                let files = idx.files();
                let mut seen_callers = BTreeSet::new();
                for (from, to, pair) in idx.pairs() {
                    if edited_ids.contains(to)
                        && !edited_ids.contains(from)
                        && pair.weight(false) > 0
                        && seen_callers.insert(*from)
                        && let Some(caller_path) = files.get(*from as usize)
                        && !opened_paths.contains(caller_path.as_str())
                    {
                        let callee_path = files.get(*to as usize).cloned().unwrap_or_default();
                        unopened_callers.push(json!({
                            "service": target.service_id,
                            "path": caller_path,
                            "links": pair.weight(false),
                            "to": callee_path,
                            "test": graph_map::is_test_path(caller_path),
                        }));
                        if unopened_callers.len() >= 32 {
                            break;
                        }
                    }
                }
            }
        }

        augment_summaries_with_intra_callers(&self.graphs, targets, &mut impact_summaries, false);
        let opened_by_service = collect_opened_by_service_for_run(all_dispatches, None);
        let sys_graph = SystemGraph::from_system(sys, impact_summaries);
        let mut merged_cross = Vec::new();
        let mut seen_cross = BTreeSet::new();
        if let Some(existing) = existing.as_array() {
            for raw_item in existing {
                let item = normalize_cross_unopened_entry(raw_item);
                let key = (
                    item["consumer"].as_str().unwrap_or("").to_owned(),
                    item["contract"].as_str().unwrap_or("").to_owned(),
                    item["path"].as_str().unwrap_or("").to_owned(),
                );
                if seen_cross.insert(key) {
                    merged_cross.push(item);
                }
            }
        }
        for target in targets {
            if target.service_id == "__system__" || !edited_services.contains(&target.service_id) {
                continue;
            }
            let imp = sys_graph.impact_across_services(&CrossServiceImpactQuery {
                service: Some(target.service_id.clone()),
                path: None,
                contract: None,
                certain: false,
                opened_by_service: opened_by_service.clone(),
            });
            if let Some(arr) = imp["cross_service_unopened"].as_array() {
                for raw_item in arr {
                    let item = normalize_cross_unopened_entry(raw_item);
                    let key = (
                        item["consumer"].as_str().unwrap_or("").to_owned(),
                        item["contract"].as_str().unwrap_or("").to_owned(),
                        item["path"].as_str().unwrap_or("").to_owned(),
                    );
                    if seen_cross.insert(key) {
                        merged_cross.push(item);
                    }
                }
            }
        }
        (unopened_callers, merged_cross)
    }
}

// ---------------------------------------------------------------------------
// `SystemGraphDomain` (`impl Domain`) — Member-Service & System-Wide Endpoints
// ---------------------------------------------------------------------------

pub struct SystemGraphDomain {
    pub store: SessionStore,
    pub graphs: Arc<Graphs>,
    pub bridge: Arc<HarnessCodeGraphBridge>,
    pub manager: Option<Arc<SystemManager>>,
}

impl SystemGraphDomain {
    pub fn new(
        store: SessionStore,
        graphs: Arc<Graphs>,
        bridge: Arc<HarnessCodeGraphBridge>,
        manager: Option<Arc<SystemManager>>,
    ) -> Self {
        Self {
            store,
            graphs,
            bridge,
            manager,
        }
    }

    fn parse_check_flag(query: &Value) -> Result<bool> {
        match query["check"].as_str() {
            None | Some("") | Some("0") => Ok(false),
            Some("1") => Ok(true),
            Some(_) => Err(HarnessError::invalid("Invalid graph check flag.")),
        }
    }

    fn parse_bool_flag(query: &Value, name: &str) -> Result<bool> {
        match query[name].as_str() {
            None | Some("") | Some("0") | Some("false") => Ok(false),
            Some("1") | Some("true") => Ok(true),
            Some(_) => Err(HarnessError::invalid(format!("Invalid flag '{name}'."))),
        }
    }

    fn normalize_target_and_run_id(
        raw_target: Option<&str>,
        raw_run_id: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let explicit_rid = raw_run_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|rid| {
                rid.strip_prefix("sysrun:")
                    .or_else(|| rid.strip_prefix("system-run:"))
                    .unwrap_or(rid)
                    .trim()
                    .to_owned()
            })
            .filter(|s| !s.is_empty());

        let trimmed_target = raw_target.map(str::trim).filter(|s| !s.is_empty());
        let target_rid = trimmed_target
            .and_then(|t| {
                t.strip_prefix("sysrun:")
                    .or_else(|| t.strip_prefix("system-run:"))
            })
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        let run_id = explicit_rid.or(target_rid);
        let is_bare_sysrun = matches!(trimmed_target, Some("sysrun" | "system-run"));

        let normalized_target = match (trimmed_target, &run_id) {
            (None, Some(rid)) => Some(format!("sysrun:{rid}")),
            (Some(_), Some(rid)) if is_bare_sysrun => Some(format!("sysrun:{rid}")),
            (Some(_), None) if is_bare_sysrun => None,
            (Some(t), _) => Some(t.to_owned()),
            (None, None) => None,
        };
        (normalized_target, run_id)
    }

    fn normalize_target_query(query: &Value) -> Result<(Option<String>, Option<String>)> {
        let raw_target = query["target"].as_str().map(str::trim).unwrap_or("");
        let raw_from = query["from"].as_str().map(str::trim).unwrap_or("");
        let raw_to = query["to"].as_str().map(str::trim).unwrap_or("");
        let effective_target = if !raw_target.is_empty() {
            if !raw_from.is_empty() || !raw_to.is_empty() {
                return Err(HarnessError::invalid(
                    "Specify either target or from/to range parameters, not both.",
                ));
            }
            Some(raw_target.to_owned())
        } else if !raw_from.is_empty() || !raw_to.is_empty() {
            if raw_from.is_empty() || raw_to.is_empty() {
                return Err(HarnessError::invalid(
                    "Both 'from' and 'to' are required for a range.",
                ));
            }
            let range = format!("range:{raw_from}..{raw_to}");
            Target::parse(&range)?;
            Some(range)
        } else {
            None
        };
        Ok(Self::normalize_target_and_run_id(
            effective_target.as_deref(),
            query["run_id"].as_str(),
        ))
    }

    fn resolve_service_summaries_for_target(
        &self,
        system: &System,
        targets: &[ResolvedServiceTarget],
        target_spec: Option<&str>,
        run_id: Option<&str>,
        agent_filter: Option<&str>,
        dispatches: &[DispatchEvidence],
    ) -> Result<BTreeMap<String, ServiceCodeSummary>> {
        let explicit_nonwork_target = if run_id.is_none() {
            match target_spec.filter(|t| !t.is_empty() && *t != "work" && *t != "uncommitted") {
                Some(tspec) => Some(Target::parse(tspec)?),
                None => None,
            }
        } else {
            None
        };
        let mut summaries = if let Some(rid) = run_id {
            let mut run_summaries = build_run_scoped_summaries(system, dispatches);
            if dispatches.is_empty()
                && agent_filter.is_none()
                && let Ok(record) = load_system_run_record(&self.store, rid)
            {
                for target in targets {
                    if let Some(base_sha) = record["baseline_commits"][&target.service_id].as_str()
                    {
                        let entry = run_summaries.entry(target.service_id.clone()).or_default();
                        for ch in git_changes_since(&target.root, Some(base_sha)) {
                            let clean = ch
                                .path
                                .replace('\\', "/")
                                .trim_start_matches("./")
                                .to_owned();
                            if !clean.is_empty()
                                && !is_runtime_artifact_path(&clean)
                                && !entry.changed_files.iter().any(|f| f.path == clean)
                            {
                                entry.changed_files.push(ChangedFileEntry {
                                    is_test: graph_map::is_test_path(&clean),
                                    path: clean,
                                    status: ch.status,
                                    from: ch.from,
                                    additions: None,
                                    deletions: None,
                                });
                            }
                        }
                    }
                }
            }
            run_summaries
        } else if explicit_nonwork_target.is_some() {
            collect_run_service_summaries(system, &[])
        } else {
            collect_git_service_summaries(system, dispatches)
        };

        if let Some(ref parsed_target) = explicit_nonwork_target {
            for target in targets {
                if let Ok(resolved) = graph_changes::resolve(&target.root, parsed_target, &[]) {
                    let entry = summaries.entry(target.service_id.clone()).or_default();
                    entry.changed_files = resolved
                        .files
                        .into_iter()
                        .map(|ch| ChangedFileEntry {
                            is_test: graph_map::is_test_path(&ch.path),
                            path: ch.path,
                            status: ch.status.to_string(),
                            from: ch.from,
                            additions: None,
                            deletions: None,
                        })
                        .collect();
                }
            }
        }
        Ok(summaries)
    }

    fn load_dispatches_for_run(&self, run_id: &str) -> Vec<DispatchEvidence> {
        let exec_dir = self
            .store
            .state
            .join("ai-system")
            .join(run_id)
            .join("execution");
        let run_state = fs::read(exec_dir.join("run.json"))
            .ok()
            .and_then(|b| crate::strict_json::parse(&b, 16 * 1024 * 1024).ok())
            .unwrap_or_else(|| json!({}));
        let mut dispatches = load_run_evidence(&exec_dir, &run_state).unwrap_or_default();
        for live in self.bridge.snapshot_active_dispatches(run_id) {
            if !dispatches
                .iter()
                .any(|d| d.phase == live.phase && d.attempt == live.attempt)
            {
                dispatches.push(live);
            }
        }
        dispatches
    }

    fn opened_by_service_for_run(
        &self,
        dispatches: &[DispatchEvidence],
        agent_filter: Option<&str>,
    ) -> BTreeMap<String, BTreeSet<String>> {
        collect_opened_by_service_for_run(dispatches, agent_filter)
    }

    fn build_enriched_system_graph_summaries(
        &self,
        system: &System,
        targets: &[ResolvedServiceTarget],
        target_spec: Option<&str>,
        run_id: Option<&str>,
        agent_filter: Option<&str>,
    ) -> Result<BTreeMap<String, ServiceCodeSummary>> {
        let dispatches: Vec<DispatchEvidence> = match run_id {
            Some(rid) => self
                .load_dispatches_for_run(rid)
                .into_iter()
                .filter(|ev| agent_matches(agent_filter, &ev.phase, &ev.service))
                .collect(),
            None => Vec::new(),
        };
        let mut summaries = self.resolve_service_summaries_for_target(
            system,
            targets,
            target_spec,
            run_id,
            agent_filter,
            &dispatches,
        )?;

        let provided_tokens: Vec<(String, String, String, Vec<String>)> = system
            .services
            .iter()
            .flat_map(|(sid, svc)| {
                svc.manifest
                    .as_ref()
                    .and_then(|m| m["provides"].as_array())
                    .into_iter()
                    .flatten()
                    .map(move |prov| {
                        let cid = prov["id"].as_str().unwrap_or("").to_owned();
                        let ckind = prov["kind"].as_str().unwrap_or("other").to_owned();
                        (sid.clone(), cid, ckind, contract_probes(prov))
                    })
            })
            .collect();

        for target in targets {
            let entry = summaries.entry(target.service_id.clone()).or_default();
            if let Ok(st) = self.graphs.status(&target.graph_id, false) {
                entry.state = st["state"].as_str().unwrap_or("off").to_owned();
                entry.stale = st["state"] == "stale";
                if let Some(gobj) = st["graph"].as_object() {
                    entry.nodes = gobj.get("nodes").and_then(Value::as_u64).unwrap_or(0) as usize;
                    entry.edges = gobj.get("edges").and_then(Value::as_u64).unwrap_or(0) as usize;
                    if let Some(c) = gobj.get("commit").and_then(Value::as_str) {
                        entry.commit = Some(c.to_owned());
                    }
                }
            }

            let live_idx = self
                .graphs
                .index_for_target_or_live(&target.graph_id, target_spec)
                .map(|(idx, _)| idx);
            if let Some(idx) = &live_idx {
                let changed_set: BTreeSet<&str> = entry
                    .changed_files
                    .iter()
                    .map(|f| f.path.as_str())
                    .collect();
                for (sym_idx, sym) in idx.symbol_entries().iter().enumerate() {
                    if let Some(fpath) = idx.files().get(sym.file as usize)
                        && changed_set.contains(fpath.as_str())
                        && entry.changed_entities.len() < 48
                    {
                        entry.changed_entities.push(ChangedEntityEntry {
                            name: idx.symbol_label(sym_idx as u32).to_owned(),
                            kind: sym.kind.word().to_owned(),
                            path: fpath.clone(),
                            change: "modified".to_owned(),
                            visibility: None,
                            visibility_narrowed: false,
                        });
                    }
                }
                if let Ok(Some(rules)) = graph_map::layer_rules::Rules::read(&target.root) {
                    for (from_f, to_f, pair) in idx.pairs() {
                        if pair.weight(false) == 0 {
                            continue;
                        }
                        let (Some(fp), Some(tp)) = (
                            idx.files().get(*from_f as usize),
                            idx.files().get(*to_f as usize),
                        ) else {
                            continue;
                        };
                        if let Some((_, from_layer, _, to_layer)) =
                            graph_map::check_layer_rule(&rules, fp, tp)
                            && entry.layer_violations.len() < 32
                        {
                            entry.layer_violations.push(LayerViolationEntry {
                                from_path: fp.clone(),
                                to_path: tp.clone(),
                                from_layer,
                                to_layer,
                                reason: Some(format!("{fp} depends upward on {tp}")),
                            });
                        }
                    }
                }
            }

            if target.service_id != "__system__" && !provided_tokens.is_empty() {
                let ir_data = self
                    .graphs
                    .get_ir_store_readonly(&target.graph_id)
                    .and_then(|ir| ir.semantic_edges_for_system().ok());
                if let Some((_, ref sem_edges)) = ir_data {
                    for (rel_path, _kind, sem_target, from_sym, _line, _ev) in sem_edges {
                        let stripped = sem_target
                            .strip_prefix("route:")
                            .or_else(|| sem_target.strip_prefix("event:"))
                            .or_else(|| sem_target.strip_prefix("service:"))
                            .or_else(|| sem_target.strip_prefix("table:"))
                            .unwrap_or(sem_target);
                        for (provider_id, cid, ckind, probes) in &provided_tokens {
                            if provider_id == &target.service_id {
                                continue;
                            }
                            let matched_probe = probes.iter().find(|probe| {
                                !probe.is_empty()
                                    && (stripped.contains(probe.as_str())
                                        || sem_target.contains(probe.as_str()))
                            });
                            let service_url_probe = format!("http://{provider_id}");
                            let matched = matched_probe.cloned().or_else(|| {
                                (stripped.contains(&service_url_probe)
                                    || sem_target.contains(&service_url_probe))
                                .then_some(service_url_probe)
                            });
                            if let Some(token) = matched {
                                let contract_opt = (!cid.is_empty()).then(|| cid.clone());
                                let already = entry.code_refs.iter().any(|r| {
                                    r.from_path == *rel_path
                                        && r.target_service == *provider_id
                                        && r.contract == contract_opt
                                });
                                if !already {
                                    entry.code_refs.push(CodeReference {
                                        from_path: rel_path.clone(),
                                        from_symbol: from_sym.clone(),
                                        target_service: provider_id.clone(),
                                        contract: contract_opt,
                                        kind: ckind.clone(),
                                        certainty: "EXTRACTED".to_owned(),
                                        detail: Some(format!("IR edge references `{token}`")),
                                    });
                                }
                            }
                        }
                    }
                }
                let ir_populated = ir_data
                    .as_ref()
                    .is_some_and(|(ir_files, _)| !ir_files.is_empty());
                let candidate_files: Vec<String> = if ir_populated && !entry.stale {
                    // Clean populated IR already provided semantic edges; skip disk walks.
                    Vec::new()
                } else if ir_populated {
                    entry
                        .changed_files
                        .iter()
                        .map(|f| f.path.as_str())
                        .filter(|p| !graph_map::is_test_path(p))
                        .take(MAX_UNDECLARED_SCAN_FILES)
                        .map(str::to_owned)
                        .collect()
                } else if let Some(idx) = &live_idx {
                    idx.files()
                        .iter()
                        .filter(|p| !graph_map::is_test_path(p))
                        .take(MAX_UNDECLARED_SCAN_FILES)
                        .cloned()
                        .collect()
                } else {
                    collect_service_code_files(&target.root, MAX_UNDECLARED_SCAN_FILES)
                };
                for rel_path in candidate_files {
                    if rel_path == "ai-service.json" || rel_path.ends_with(".lock") {
                        continue;
                    }
                    let full_path = target.root.join(&rel_path);
                    let Ok(meta) = fs::symlink_metadata(&full_path) else {
                        continue;
                    };
                    if !meta.is_file() || meta.len() > MAX_UNDECLARED_SCAN_BYTES {
                        continue;
                    }
                    let Ok(content) = fs::read_to_string(&full_path) else {
                        continue;
                    };
                    for (provider_id, cid, ckind, probes) in &provided_tokens {
                        if provider_id == &target.service_id {
                            continue;
                        }
                        let matched_probe = probes
                            .iter()
                            .find(|probe| !probe.is_empty() && content.contains(probe.as_str()));
                        let service_url_probe = format!("http://{provider_id}");
                        let matched = matched_probe.cloned().or_else(|| {
                            content
                                .contains(&service_url_probe)
                                .then_some(service_url_probe)
                        });
                        if let Some(token) = matched {
                            let contract_opt = (!cid.is_empty()).then(|| cid.clone());
                            let already = entry.code_refs.iter().any(|r| {
                                r.from_path == rel_path
                                    && r.target_service == *provider_id
                                    && r.contract == contract_opt
                            });
                            if !already {
                                entry.code_refs.push(CodeReference {
                                    from_path: rel_path.clone(),
                                    from_symbol: None,
                                    target_service: provider_id.clone(),
                                    contract: contract_opt,
                                    kind: ckind.clone(),
                                    certainty: "EXTRACTED".to_owned(),
                                    detail: Some(format!("code references `{token}`")),
                                });
                            }
                        }
                    }
                }
            }
        }

        Ok(summaries)
    }

    fn build_enriched_system_graph(
        &self,
        system: &System,
        targets: &[ResolvedServiceTarget],
        target_spec: Option<&str>,
        run_id: Option<&str>,
        agent_filter: Option<&str>,
    ) -> Result<SystemGraph> {
        let summaries = self.build_enriched_system_graph_summaries(
            system,
            targets,
            target_spec,
            run_id,
            agent_filter,
        )?;
        Ok(SystemGraph::from_system(system, summaries))
    }

    fn compute_system_status(
        &self,
        project_id: &str,
        config_path: Option<&str>,
        check: bool,
        service_filter: Option<&str>,
    ) -> Result<Value> {
        if let Some(sid) = service_filter.map(str::trim).filter(|s| !s.is_empty()) {
            return self.compute_service_status(project_id, config_path, sid, check);
        }
        let (system, cfg_rel, targets) = resolve_all_targets(
            &self.store,
            Some(&self.graphs),
            self.manager.as_deref(),
            project_id,
            config_path,
        )?;
        if check {
            for target in &targets {
                let _ = self.graphs.auto_start(&target.graph_id);
            }
        }
        let mut services_map = serde_json::Map::new();
        let mut system_root_status = Value::Null;
        let mut ready_count = 0usize;
        let mut building_count = 0usize;
        let mut stale_count = 0usize;
        let mut failed_count = 0usize;
        let mut missing_count = 0usize;
        let mut enabled_count = 0usize;
        let mut agents_count = 0usize;
        let mut total_count = 0usize;
        let mut first_build = Value::Null;

        for target in &targets {
            let mut st = self.graphs.status(&target.graph_id, false)?;
            st["service"] = json!(target.service_id);
            st["graph_id"] = json!(target.graph_id);
            st["reused_project"] = json!(target.reused_project);
            st["root"] = json!(target.root.to_string_lossy());
            if target.service_id == "__system__" {
                system_root_status = st;
                continue;
            }
            total_count += 1;
            if st["enabled"] == true {
                enabled_count += 1;
            }
            if st["agents"] == true {
                agents_count += 1;
            }
            match st["state"].as_str().unwrap_or("off") {
                "building" => {
                    building_count += 1;
                    if first_build.is_null() {
                        first_build = st["build"].clone();
                    }
                }
                "ready" => ready_count += 1,
                "stale" => {
                    ready_count += 1;
                    stale_count += 1;
                }
                "failed" => failed_count += 1,
                "missing" => missing_count += 1,
                _ => {}
            }
            services_map.insert(target.service_id.clone(), st);
        }

        for (sid, svc) in &system.services {
            if svc.access != "available" {
                total_count += 1;
                services_map.insert(
                    sid.clone(),
                    json!({
                        "service": sid,
                        "enabled": false,
                        "agents": false,
                        "state": "off",
                        "access": svc.access,
                        "message": format!("Service root unavailable ({}).", svc.access),
                    }),
                );
            }
        }

        let enabled = enabled_count > 0 || system_root_status["enabled"] == true;
        let tell_agents = agents_count > 0 || system_root_status["agents"] == true;
        let state = if !enabled && total_count > 0 {
            "off"
        } else if building_count > 0 || system_root_status["state"] == "building" {
            "building"
        } else if missing_count > 0 && ready_count == 0 {
            "missing"
        } else if failed_count > 0 && ready_count == 0 {
            "failed"
        } else if total_count == 0 {
            system_root_status["state"].as_str().unwrap_or("ready")
        } else if ready_count == 0 {
            "empty"
        } else if stale_count > 0 {
            "stale"
        } else {
            "ready"
        };

        let message = match state {
            "building" => format!(
                "Building code graphs ({building_count} building, {ready_count}/{total_count} ready)."
            ),
            "ready" => format!(
                "System code graph ready ({ready_count}/{total_count} member services indexed)."
            ),
            "stale" => format!(
                "System code graph available ({ready_count}/{total_count} indexed, {stale_count} stale)."
            ),
            "off" => "Code graphs are turned off for this system.".to_owned(),
            "missing" => {
                "Graphify is not installed; architecture map uses passport & Git metadata."
                    .to_owned()
            }
            "failed" => "Code graph build failed for one or more member services.".to_owned(),
            _ => format!("{ready_count}/{total_count} member service graphs ready."),
        };

        Ok(json!({
            "project_id": project_id,
            "config_path": cfg_rel,
            "state": state,
            "enabled": enabled,
            "agents": tell_agents,
            "tell_agents": tell_agents,
            "ready_count": ready_count,
            "total_count": total_count,
            "building_count": building_count,
            "message": message,
            "build": first_build,
            "system": system_root_status,
            "services": services_map,
        }))
    }

    fn compute_service_status(
        &self,
        project_id: &str,
        config_path: Option<&str>,
        service_id: &str,
        check: bool,
    ) -> Result<Value> {
        let (_, target) = resolve_service_target(
            &self.store,
            Some(&self.graphs),
            self.manager.as_deref(),
            project_id,
            config_path,
            service_id,
        )?;
        if check {
            self.graphs.auto_start(&target.graph_id)?;
        }
        let mut st = self.graphs.status(&target.graph_id, check)?;
        st["project_id"] = json!(project_id);
        st["config_path"] = json!(target.config_path);
        st["service"] = json!(target.service_id);
        st["graph_id"] = json!(target.graph_id);
        st["reused_project"] = json!(target.reused_project);
        st["root"] = json!(target.root.to_string_lossy());
        Ok(st)
    }

    fn execute_service_action(
        &self,
        project_id: &str,
        config_path: Option<&str>,
        service_id: &str,
        raw_action: &str,
        target_arg: Option<&str>,
    ) -> Result<Value> {
        let action = normalize_graph_action(raw_action);
        let (_, target) = resolve_service_target(
            &self.store,
            Some(&self.graphs),
            self.manager.as_deref(),
            project_id,
            config_path,
            service_id,
        )?;
        let mut out = if action == "changes" {
            let raw_t = target_arg
                .filter(|s| !s.is_empty())
                .ok_or_else(|| HarnessError::invalid("Target is required for 'changes' action."))?;
            let parsed = Target::parse(raw_t)?;
            self.graphs.choose(&target.graph_id, &parsed)?
        } else {
            self.graphs.act(&target.graph_id, action)?
        };
        out["project_id"] = json!(project_id);
        out["config_path"] = json!(target.config_path);
        out["service"] = json!(target.service_id);
        out["graph_id"] = json!(target.graph_id);
        out["reused_project"] = json!(target.reused_project);
        Ok(out)
    }

    fn execute_system_action(&self, request: &Value) -> Result<Value> {
        let body = &request["body"];
        let query = &request["query"];
        if !body.is_object()
            || !store::exact(
                body,
                &["action"],
                &["project_id", "config_path", "service", "target"],
            )
        {
            return Err(HarnessError::invalid("Invalid system code graph action."));
        }
        let project_id = body["project_id"]
            .as_str()
            .or_else(|| query["project_id"].as_str())
            .unwrap_or("");
        let config_path = body["config_path"]
            .as_str()
            .or_else(|| query["config_path"].as_str());
        let service_opt = body["service"]
            .as_str()
            .or_else(|| query["service"].as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let action = normalize_graph_action(body["action"].as_str().unwrap_or(""));
        let target_arg = body["target"].as_str();

        if let Some(sid) = service_opt {
            return self.execute_service_action(project_id, config_path, sid, action, target_arg);
        }

        let (_, cfg_rel, targets) = resolve_all_targets(
            &self.store,
            Some(&self.graphs),
            self.manager.as_deref(),
            project_id,
            config_path,
        )?;
        if targets.is_empty() {
            return self.compute_system_status(project_id, Some(&cfg_rel), false, None);
        }

        match action {
            "enable" | "disable" | "withhold" | "delete" => {
                for t in &targets {
                    let _ = self.graphs.act(&t.graph_id, action)?;
                }
            }
            "offer" => {
                let mut offered_any = false;
                for t in &targets {
                    if self.graphs.status(&t.graph_id, false)?["enabled"] == true {
                        self.graphs.act(&t.graph_id, "offer")?;
                        offered_any = true;
                    }
                }
                if !offered_any {
                    return Err(HarnessError::invalid(
                        "Turn the code graph on before telling agents about it.",
                    ));
                }
            }
            "rebuild" => {
                let mut rebuilt_any = false;
                let mut last_err = None;
                for t in &targets {
                    if self.graphs.status(&t.graph_id, false)?["enabled"] == true {
                        match self.graphs.act(&t.graph_id, "rebuild") {
                            Ok(_) => rebuilt_any = true,
                            Err(err) => last_err = Some(err),
                        }
                    }
                }
                if !rebuilt_any {
                    if let Some(err) = last_err {
                        return Err(err);
                    }
                    return Err(HarnessError::invalid(
                        "Turn the code graph on before rebuilding it.",
                    ));
                }
            }
            "changes" => {
                let raw_t = target_arg.filter(|s| !s.is_empty()).ok_or_else(|| {
                    HarnessError::invalid("Target is required for 'changes' action.")
                })?;
                let parsed = Target::parse(raw_t)?;
                for t in &targets {
                    let _ = self.graphs.choose(&t.graph_id, &parsed);
                }
            }
            _ => {
                return Err(HarnessError::invalid(
                    "Choose enable, disable, offer, withhold, rebuild or delete.",
                ));
            }
        }
        self.compute_system_status(project_id, Some(&cfg_rel), false, None)
    }

    async fn delegate_to_graphs(
        &self,
        op: &'static str,
        target: &ResolvedServiceTarget,
        mut query: Value,
        body: Value,
    ) -> Result<Value> {
        if let Some(qobj) = query.as_object_mut() {
            qobj.remove("project_id");
            qobj.remove("config_path");
            qobj.remove("service");
            let raw_target = qobj
                .get("target")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let raw_run_id = qobj
                .remove("run_id")
                .and_then(|v| v.as_str().map(str::to_owned));
            let (norm_target, _) =
                Self::normalize_target_and_run_id(raw_target.as_deref(), raw_run_id.as_deref());
            match norm_target {
                Some(nt) => {
                    qobj.insert("target".to_owned(), json!(nt));
                }
                None => {
                    qobj.remove("target");
                }
            }
            if op == "graph.find"
                && !qobj.contains_key("query")
                && let Some(qval) = qobj.remove("q")
            {
                qobj.insert("query".to_owned(), qval);
            }
        }
        let delegated = json!({
            "params": { "project_id": target.graph_id },
            "query": query,
            "body": body,
        });
        match self.graphs.execute(op, delegated).await? {
            Payload::Json(mut val) => {
                if val.is_object() {
                    val["project_id"] = json!(target.project_id);
                    val["config_path"] = json!(target.config_path);
                    val["service"] = json!(target.service_id);
                    val["graph_id"] = json!(target.graph_id);
                    val["reused_project"] = json!(target.reused_project);
                }
                Ok(val)
            }
            _ => Err(HarnessError::unavailable(
                "Unexpected code graph response payload.",
            )),
        }
    }

    fn execute_blocking(&self, operation: &str, request: &Value) -> Result<Value> {
        let query = &request["query"];
        match operation {
            "system_graph.service_status" => {
                let check = Self::parse_check_flag(query)?;
                self.compute_service_status(
                    query["project_id"].as_str().unwrap_or(""),
                    query["config_path"].as_str(),
                    query["service"].as_str().unwrap_or(""),
                    check,
                )
            }
            "system_graph.service_act" => {
                let body = &request["body"];
                if !body.is_object()
                    || !store::exact(
                        body,
                        &["action"],
                        &["project_id", "config_path", "service", "target"],
                    )
                {
                    return Err(HarnessError::invalid(
                        "Invalid member-service code graph action.",
                    ));
                }
                let pid = body["project_id"]
                    .as_str()
                    .or_else(|| query["project_id"].as_str())
                    .unwrap_or("");
                let cfg = body["config_path"]
                    .as_str()
                    .or_else(|| query["config_path"].as_str());
                let sid = body["service"]
                    .as_str()
                    .or_else(|| query["service"].as_str())
                    .unwrap_or("");
                self.execute_service_action(
                    pid,
                    cfg,
                    sid,
                    body["action"].as_str().unwrap_or(""),
                    body["target"].as_str(),
                )
            }
            "system_graph.status" => {
                let check = Self::parse_check_flag(query)?;
                self.compute_system_status(
                    query["project_id"].as_str().unwrap_or(""),
                    query["config_path"].as_str(),
                    check,
                    query["service"].as_str(),
                )
            }
            "system_graph.act" => self.execute_system_action(request),
            "system_graph.map" => {
                let project_id = query["project_id"].as_str().unwrap_or("");
                let config_path = query["config_path"].as_str();
                let (target_spec, run_id) = Self::normalize_target_and_run_id(
                    query["target"].as_str(),
                    query["run_id"].as_str(),
                );
                let agent_filter = query["agent"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let (system, cfg_rel, targets) = resolve_all_targets(
                    &self.store,
                    Some(&self.graphs),
                    self.manager.as_deref(),
                    project_id,
                    config_path,
                )?;
                let sys_graph = self.build_enriched_system_graph(
                    &system,
                    &targets,
                    target_spec.as_deref(),
                    run_id.as_deref(),
                    agent_filter,
                )?;
                let map_query = parse_system_map_query(query)?;
                let mut view = sys_graph.query(&map_query);
                view["project_id"] = json!(project_id);
                view["config_path"] = json!(cfg_rel);
                if let Some(ts) = &target_spec {
                    view["target"] = json!(ts);
                }
                if let Some(af) = agent_filter {
                    view["agent"] = json!(af);
                }
                if let Some(rid) = &run_id {
                    let dispatches: Vec<DispatchEvidence> = self
                        .load_dispatches_for_run(rid)
                        .into_iter()
                        .filter(|ev| agent_matches(agent_filter, &ev.phase, &ev.service))
                        .map(|mut ev| {
                            ev.git_changed
                                .retain(|c| !is_runtime_artifact_path(&c.path));
                            ev.ledger_edited
                                .retain(|c| !is_runtime_artifact_path(&c.path));
                            ev
                        })
                        .collect();
                    let exec_dir = self
                        .store
                        .state
                        .join("ai-system")
                        .join(rid)
                        .join("execution");
                    let run_state = fs::read(exec_dir.join("run.json"))
                        .ok()
                        .and_then(|b| crate::strict_json::parse(&b, 16 * 1024 * 1024).ok())
                        .unwrap_or_else(|| json!({}));
                    view["run"] = summarize_run_evidence(Some(&system), &run_state, &dispatches);
                    if let Ok(Some(focus)) = load_system_focus_value(&self.store, rid, project_id) {
                        view["graph_focus"] = focus;
                    }
                }
                Ok(view)
            }
            "system_graph.find" => {
                let project_id = query["project_id"].as_str().unwrap_or("");
                let config_path = query["config_path"].as_str();
                let wanted = query["query"]
                    .as_str()
                    .or_else(|| query["q"].as_str())
                    .unwrap_or("");
                if wanted.len() > 400 || wanted.chars().any(char::is_control) {
                    return Err(HarnessError::invalid("Invalid search query."));
                }
                let notests = Self::parse_bool_flag(query, "notests")?;
                let service_filter = query["service"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let (system, cfg_rel, targets) = resolve_all_targets(
                    &self.store,
                    Some(&self.graphs),
                    self.manager.as_deref(),
                    project_id,
                    config_path,
                )?;
                let sys_graph =
                    self.build_enriched_system_graph(&system, &targets, None, None, None)?;
                let mut out = sys_graph.find(wanted);
                out["project_id"] = json!(project_id);
                out["config_path"] = json!(cfg_rel);

                if let Some(sf) = service_filter
                    && let Some(results) = out["results"].as_array_mut()
                {
                    results.retain(|item| {
                        item["service"].as_str() == Some(sf) || item["id"].as_str() == Some(sf)
                    });
                    out["total"] = json!(results.len());
                }

                if !wanted.trim().is_empty()
                    && let Some(results) = out["results"].as_array_mut()
                {
                    for target in &targets {
                        if let Some(sf) = service_filter
                            && target.service_id != sf
                        {
                            continue;
                        }
                        let Some(raw_idx) = self.graphs.live_index(&target.graph_id) else {
                            continue;
                        };
                        let idx: Arc<Index> = if notests {
                            raw_idx.without_tests()
                        } else {
                            raw_idx
                        };
                        let found = idx.find(wanted);
                        if let Some(rows) = found["rows"].as_array() {
                            for row in rows.iter().take(25) {
                                if results.len() >= 120 {
                                    break;
                                }
                                let mut item = row.clone();
                                item["kind"] =
                                    item.get("what").cloned().unwrap_or_else(|| json!("symbol"));
                                item["service"] = json!(target.service_id);
                                item["graph_id"] = json!(target.graph_id);
                                results.push(item);
                            }
                        }
                    }
                    out["total"] = json!(results.len());
                }
                Ok(out)
            }
            "system_graph.changes" => {
                let project_id = query["project_id"].as_str().unwrap_or("");
                let config_path = query["config_path"].as_str();
                let notests = Self::parse_bool_flag(query, "notests")?;
                let service_filter = query["service"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let (target_spec, run_id) = Self::normalize_target_query(query)?;
                let agent_filter = query["agent"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let (system, cfg_rel, targets) = resolve_all_targets(
                    &self.store,
                    Some(&self.graphs),
                    self.manager.as_deref(),
                    project_id,
                    config_path,
                )?;
                let sys_graph = self.build_enriched_system_graph(
                    &system,
                    &targets,
                    target_spec.as_deref(),
                    run_id.as_deref(),
                    agent_filter,
                )?;
                let mut out = sys_graph.changes_across_services(service_filter, notests);
                out["project_id"] = json!(project_id);
                out["config_path"] = json!(cfg_rel);
                out["target"] = json!(target_spec.as_deref().unwrap_or("work"));
                if let Some(af) = agent_filter {
                    out["agent"] = json!(af);
                }
                Ok(out)
            }
            "system_graph.impact" => {
                let project_id = query["project_id"].as_str().unwrap_or("");
                let config_path = query["config_path"].as_str();
                let certain = Self::parse_bool_flag(query, "certain")?;
                let (target_spec, run_id) = Self::normalize_target_query(query)?;
                let agent_filter = query["agent"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let focus_session = query["focus_session"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let parsed_target = match target_spec.as_deref() {
                    None => None,
                    Some(ts) => Some(Target::parse(ts)?),
                };
                let service_opt = query["service"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let path_opt = query["path"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let contract_opt = query["contract"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let (system, cfg_rel, targets) = resolve_all_targets(
                    &self.store,
                    Some(&self.graphs),
                    self.manager.as_deref(),
                    project_id,
                    config_path,
                )?;
                let dispatches = run_id
                    .as_deref()
                    .map(|rid| self.load_dispatches_for_run(rid))
                    .unwrap_or_default();
                let opened_by_service = self.opened_by_service_for_run(&dispatches, agent_filter);
                let mut summaries = self.build_enriched_system_graph_summaries(
                    &system,
                    &targets,
                    target_spec.as_deref(),
                    run_id.as_deref(),
                    agent_filter,
                )?;
                let query_path_for_sys = if let (Some(sid), Some(p)) = (&service_opt, &path_opt)
                    && let Some(svc_target) = targets.iter().find(|t| t.service_id == *sid)
                {
                    let clean_p = p.replace('\\', "/").trim_start_matches("./").to_owned();
                    let mut expanded = vec![clean_p.clone()];
                    if let Some((idx, _)) = self
                        .graphs
                        .index_for_target_or_live(&svc_target.graph_id, target_spec.as_deref())
                    {
                        expanded.extend(intra_service_caller_paths(
                            &idx,
                            std::slice::from_ref(&clean_p),
                            certain,
                        ));
                    }
                    let entry = summaries.entry(sid.clone()).or_default();
                    entry.changed_files = expanded
                        .into_iter()
                        .map(|path| ChangedFileEntry {
                            is_test: graph_map::is_test_path(&path),
                            path,
                            status: "M".to_owned(),
                            from: None,
                            additions: None,
                            deletions: None,
                        })
                        .collect();
                    None
                } else {
                    augment_summaries_with_intra_callers(
                        &self.graphs,
                        &targets,
                        &mut summaries,
                        certain,
                    );
                    path_opt.clone()
                };
                let sys_graph = SystemGraph::from_system(&system, summaries);
                let mut out = sys_graph.impact_across_services(&CrossServiceImpactQuery {
                    service: service_opt.clone(),
                    path: query_path_for_sys,
                    contract: contract_opt,
                    certain,
                    opened_by_service,
                });
                out["project_id"] = json!(project_id);
                out["config_path"] = json!(cfg_rel);
                if let Some(ts) = &target_spec {
                    out["target"] = json!(ts);
                }
                if let Some(af) = agent_filter {
                    out["agent"] = json!(af);
                }
                if let (Some(sid), Some(path)) = (&service_opt, &path_opt)
                    && let Some(target) = targets.iter().find(|t| t.service_id == *sid)
                    && let Ok(intra) = self.graphs.file_impact_with_agent(
                        &target.graph_id,
                        path,
                        parsed_target.as_ref(),
                        focus_session,
                        agent_filter,
                        certain,
                    )
                {
                    out["intra_service"] = intra;
                }
                Ok(out)
            }
            "system_graph.bridge" => {
                let project_id = query["project_id"].as_str().unwrap_or("");
                let config_path = query["config_path"].as_str();
                let certain = Self::parse_bool_flag(query, "certain")?;
                let (target_spec, run_id) = Self::normalize_target_and_run_id(
                    query["target"].as_str(),
                    query["run_id"].as_str(),
                );
                let agent_filter = query["agent"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let named = |key: &str| -> Result<String> {
                    let value = query[key].as_str().map(str::trim).unwrap_or("");
                    if value.len() > 256 || value.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid service link."));
                    }
                    Ok(value.to_owned())
                };
                let (from, to, contract) = (named("from")?, named("to")?, named("contract")?);
                if from.is_empty() || to.is_empty() || from == to {
                    return Err(HarnessError::invalid("Invalid service link."));
                }
                let (system, cfg_rel, targets) = resolve_all_targets(
                    &self.store,
                    Some(&self.graphs),
                    self.manager.as_deref(),
                    project_id,
                    config_path,
                )?;
                let dispatches = run_id
                    .as_deref()
                    .map(|rid| self.load_dispatches_for_run(rid))
                    .unwrap_or_default();
                let opened_by_service = self.opened_by_service_for_run(&dispatches, agent_filter);
                let sys_graph = self.build_enriched_system_graph(
                    &system,
                    &targets,
                    target_spec.as_deref(),
                    run_id.as_deref(),
                    agent_filter,
                )?;
                let link = sys_graph.link(&from, &to, certain);
                let ctx = BridgeViewContext {
                    target_spec: target_spec.as_deref(),
                    run_id: run_id.as_deref(),
                    opened_by_service: run_id.is_some().then_some(&opened_by_service),
                };
                let mut view = bridge_view(
                    &self.graphs,
                    &system,
                    &targets,
                    &link,
                    (!contract.is_empty()).then_some(contract.as_str()),
                    certain,
                    &ctx,
                );
                view["project_id"] = json!(project_id);
                view["config_path"] = json!(cfg_rel);
                Ok(view)
            }
            _ => Err(HarnessError::missing(
                "Unknown system code graph operation.",
            )),
        }
    }
}

impl Domain for SystemGraphDomain {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let query = &request["query"];
            match operation {
                "system_graph.service_map"
                | "system_graph.service_find"
                | "system_graph.service_changes"
                | "system_graph.service_file_impact"
                | "system_graph.service_pulls"
                | "system_graph.service_commits"
                | "system_graph.service_branches" => {
                    let project_id = query["project_id"].as_str().unwrap_or("");
                    let config_path = query["config_path"].as_str();
                    let service_id = query["service"].as_str().unwrap_or("");
                    let (system, target) = resolve_service_target(
                        &self.store,
                        Some(&self.graphs),
                        self.manager.as_deref(),
                        project_id,
                        config_path,
                        service_id,
                    )?;
                    let graph_op = match operation {
                        "system_graph.service_map" => "graph.map",
                        "system_graph.service_find" => "graph.find",
                        "system_graph.service_changes" => "graph.changes",
                        "system_graph.service_file_impact" => "graph.file_impact",
                        "system_graph.service_pulls" => "graph.pulls",
                        "system_graph.service_commits" => "graph.commits",
                        _ => "graph.branches",
                    };
                    let mut out = self
                        .delegate_to_graphs(
                            graph_op,
                            &target,
                            query.clone(),
                            request["body"].clone(),
                        )
                        .await?;

                    if operation == "system_graph.service_map" && target.service_id != "__system__"
                    {
                        let (target_spec, run_id) = Self::normalize_target_and_run_id(
                            query["target"].as_str(),
                            query["run_id"].as_str(),
                        );
                        let target_str = out["target"].as_str().unwrap_or("");
                        let effective_target = target_spec
                            .as_deref()
                            .or_else(|| (!target_str.is_empty()).then_some(target_str));
                        if run_id.is_some() || effective_target.is_some() {
                            let agent_filter = query["agent"]
                                .as_str()
                                .map(str::trim)
                                .filter(|s| !s.is_empty());
                            let selected_launch =
                                out["run"]["launch"].as_str().filter(|s| !s.is_empty());
                            let dispatches: Vec<DispatchEvidence> = run_id
                                .as_deref()
                                .map(|rid| self.load_dispatches_for_run(rid))
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|ev| {
                                    selected_launch.is_none_or(|sel| {
                                        ev.launch_id.as_deref().is_none_or(|lid| lid == sel)
                                    })
                                })
                                .collect();
                            let opened_by_service =
                                self.opened_by_service_for_run(&dispatches, agent_filter);
                            let certain = Self::parse_bool_flag(query, "certain").unwrap_or(false);
                            let filtered_for_edits: Vec<DispatchEvidence> = dispatches
                                .iter()
                                .filter(|ev| agent_matches(agent_filter, &ev.phase, &ev.service))
                                .cloned()
                                .collect();
                            let mut summaries = self.resolve_service_summaries_for_target(
                                &system,
                                std::slice::from_ref(&target),
                                effective_target,
                                run_id.as_deref(),
                                agent_filter,
                                &filtered_for_edits,
                            )?;
                            let has_edits = summaries
                                .get(&target.service_id)
                                .is_some_and(|s| !s.changed_files.is_empty());
                            let (cross_arr, consumers_val) = if has_edits {
                                augment_summaries_with_intra_callers(
                                    &self.graphs,
                                    std::slice::from_ref(&target),
                                    &mut summaries,
                                    certain,
                                );
                                let sys_graph = SystemGraph::from_system(&system, summaries);
                                let impact =
                                    sys_graph.impact_across_services(&CrossServiceImpactQuery {
                                        service: Some(target.service_id.clone()),
                                        path: None,
                                        contract: None,
                                        certain,
                                        opened_by_service,
                                    });
                                (
                                    normalize_cross_unopened_list(
                                        &impact["cross_service_unopened"],
                                    ),
                                    impact["contract_consumers"].clone(),
                                )
                            } else {
                                (json!([]), json!([]))
                            };
                            out["cross_service_unopened"] = cross_arr.clone();
                            out["contract_consumers"] = consumers_val;
                            if out["run"]["reach"].is_object() {
                                out["run"]["reach"]["cross_service_unopened"] = cross_arr.clone();
                            }
                            if out["reach"].is_object() {
                                out["reach"]["cross_service_unopened"] = cross_arr;
                            }
                        }
                    }

                    if operation == "system_graph.service_file_impact" {
                        let (target_spec, run_id) = Self::normalize_target_query(query)?;
                        let agent_filter = query["agent"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty());
                        let dispatches = run_id
                            .as_deref()
                            .map(|rid| self.load_dispatches_for_run(rid))
                            .unwrap_or_default();
                        let opened_by_service =
                            self.opened_by_service_for_run(&dispatches, agent_filter);
                        let certain = Self::parse_bool_flag(query, "certain").unwrap_or(false);
                        let path_opt = query["path"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned);
                        let mut summaries = self.resolve_service_summaries_for_target(
                            &system,
                            std::slice::from_ref(&target),
                            target_spec.as_deref(),
                            run_id.as_deref(),
                            agent_filter,
                            &dispatches,
                        )?;
                        let query_path_for_sys = if let Some(ref p) = path_opt {
                            let clean_p = p.replace('\\', "/").trim_start_matches("./").to_owned();
                            let mut expanded = vec![clean_p.clone()];
                            if let Some((idx, _)) = self
                                .graphs
                                .index_for_target_or_live(&target.graph_id, target_spec.as_deref())
                            {
                                expanded.extend(intra_service_caller_paths(
                                    &idx,
                                    std::slice::from_ref(&clean_p),
                                    certain,
                                ));
                            }
                            let entry = summaries.entry(target.service_id.clone()).or_default();
                            entry.changed_files = expanded
                                .into_iter()
                                .map(|path| ChangedFileEntry {
                                    is_test: graph_map::is_test_path(&path),
                                    path,
                                    status: "M".to_owned(),
                                    from: None,
                                    additions: None,
                                    deletions: None,
                                })
                                .collect();
                            None
                        } else {
                            augment_summaries_with_intra_callers(
                                &self.graphs,
                                std::slice::from_ref(&target),
                                &mut summaries,
                                certain,
                            );
                            None
                        };
                        let sys_graph = SystemGraph::from_system(&system, summaries);
                        let impact = sys_graph.impact_across_services(&CrossServiceImpactQuery {
                            service: Some(target.service_id.clone()),
                            path: query_path_for_sys,
                            contract: None,
                            certain,
                            opened_by_service,
                        });
                        out["contract_consumers"] = impact["contract_consumers"].clone();
                        out["impacted_contracts"] = impact["impacted_contracts"].clone();
                        out["impacted_capabilities"] = impact["impacted_capabilities"].clone();
                        out["downstream_services"] = impact["downstream_services"].clone();
                        out["cross_service_unopened"] =
                            normalize_cross_unopened_list(&impact["cross_service_unopened"]);
                    }

                    Ok(Payload::Json(out))
                }
                _ => {
                    if tokio::runtime::Handle::try_current().is_ok() {
                        let store = self.store.clone();
                        let graphs = self.graphs.clone();
                        let bridge = self.bridge.clone();
                        let manager = self.manager.clone();
                        let op = operation.to_owned();
                        let val = tokio::task::spawn_blocking(move || {
                            let domain = SystemGraphDomain::new(store, graphs, bridge, manager);
                            domain.execute_blocking(&op, &request)
                        })
                        .await
                        .map_err(|_| {
                            HarnessError::unavailable("System code graph worker stopped.")
                        })??;
                        Ok(Payload::Json(val))
                    } else {
                        Ok(Payload::Json(self.execute_blocking(operation, &request)?))
                    }
                }
            }
        })
    }
}

fn parse_csv_or_json_list(raw: Option<&str>, max_items: usize) -> Result<Vec<String>> {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    if text.starts_with('[') {
        let parsed: Vec<String> = serde_json::from_str(text)
            .map_err(|_| HarnessError::invalid("Invalid list parameter in system graph query."))?;
        if parsed.len() > max_items {
            return Err(HarnessError::invalid(
                "Too many items in system graph query parameter.",
            ));
        }
        return Ok(parsed
            .into_iter()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect());
    }
    let items: Vec<String> = text
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if items.len() > max_items {
        return Err(HarnessError::invalid(
            "Too many items in system graph query parameter.",
        ));
    }
    Ok(items)
}

fn parse_hop_count(raw: Option<&str>) -> Result<usize> {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(0);
    };
    if let Ok(n) = text.parse::<usize>() {
        if n > 64 {
            return Err(HarnessError::invalid("Hop count exceeds bound."));
        }
        return Ok(n);
    }
    let list = parse_csv_or_json_list(Some(text), 64)?;
    Ok(list.len().min(64))
}

fn parse_system_map_query(query: &Value) -> Result<SystemMapQuery> {
    let text_opt = |name: &str| -> Result<Option<String>> {
        let Some(v) = query[name]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        if v.len() > 1024 || v.chars().any(char::is_control) {
            return Err(HarnessError::invalid(format!(
                "Invalid system graph parameter '{name}'."
            )));
        }
        Ok(Some(v.to_owned()))
    };
    let bool_flag = |name: &str| SystemGraphDomain::parse_bool_flag(query, name);
    let depth = match query["depth"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(d) => Some(
            d.parse::<usize>()
                .ok()
                .filter(|n| *n <= 64)
                .ok_or_else(|| HarnessError::invalid("Invalid blast-radius depth."))?,
        ),
    };
    let skip = match query["skip"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => 0,
        Some(s) => s
            .parse::<usize>()
            .ok()
            .filter(|n| *n <= 10_000)
            .ok_or_else(|| HarnessError::invalid("Invalid skip parameter."))?,
    };
    let mut from = text_opt("from")?;
    let mut to = text_opt("to")?;
    let mut is_path = false;
    let mut is_shared = false;

    match query["path"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None | Some("0") | Some("false") => {}
        Some("1") | Some("true") => {
            is_path = true;
        }
        Some(raw_path) => {
            let path_items = parse_csv_or_json_list(Some(raw_path), 2)?;
            let [a, b] = path_items.as_slice() else {
                return Err(HarnessError::invalid(
                    "Invalid system graph path endpoints.",
                ));
            };
            from = Some(a.clone());
            to = Some(b.clone());
            is_path = true;
        }
    }

    match query["shared"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None | Some("0") | Some("false") => {}
        Some("1") | Some("true") => {
            if is_path {
                return Err(HarnessError::invalid(
                    "Cannot combine 'path' and 'shared' queries.",
                ));
            }
            is_shared = true;
        }
        Some(raw_shared) => {
            if is_path {
                return Err(HarnessError::invalid(
                    "Cannot combine 'path' and 'shared' queries.",
                ));
            }
            let shared_items = parse_csv_or_json_list(Some(raw_shared), 2)?;
            let [a, b] = shared_items.as_slice() else {
                return Err(HarnessError::invalid(
                    "Invalid system graph shared endpoints.",
                ));
            };
            from = Some(a.clone());
            to = Some(b.clone());
            is_shared = true;
        }
    }

    if (is_path || is_shared) && (from.is_none() || to.is_none()) {
        return Err(HarnessError::invalid(
            "Both 'from' and 'to' are required for 'path' or 'shared' system graph queries.",
        ));
    }
    if from.is_some() != to.is_some() {
        return Err(HarnessError::invalid(
            "Both 'from' and 'to' are required when inspecting a system graph link.",
        ));
    }

    let cycle_raw = text_opt("cycle")?;
    let is_cycle = match cycle_raw.as_deref() {
        None | Some("0") | Some("false") => false,
        Some(_) => true,
    };

    let mut scope = text_opt("scope")?;
    if scope.is_none()
        && is_cycle
        && let Some(c) = &cycle_raw
        && c != "1"
        && c != "true"
    {
        scope = Some(c.clone());
    }

    Ok(SystemMapQuery {
        scope,
        focus: text_opt("focus")?,
        filter: text_opt("filter")?,
        spread: parse_csv_or_json_list(query["spread"].as_str(), 64)?,
        whole: bool_flag("whole")?,
        certain: bool_flag("certain")?,
        follow: text_opt("follow")?,
        out: parse_hop_count(query["out"].as_str())?,
        into: parse_hop_count(query["into"].as_str())?,
        from,
        to,
        path: is_path,
        shared: is_shared,
        side: text_opt("side")?,
        skip,
        cycle: is_cycle,
        layers: bool_flag("layers")?,
        seed_services: parse_csv_or_json_list(query["seed_services"].as_str(), 64)?,
        contracts: parse_csv_or_json_list(query["contracts"].as_str(), 32)?,
        depth,
    })
}

/// What code that uses a contract would hold of it: its id, the path,
/// route, endpoint, topic, event or queue it is reached by, and the paths
/// and dotted names its summary gives.
fn contract_probes(prov: &Value) -> Vec<String> {
    let mut probes = Vec::new();
    if let Some(cid) = prov["id"].as_str().filter(|cid| cid.len() >= 4) {
        probes.push(cid.to_owned());
    }
    for key in ["path", "route", "endpoint", "topic", "event", "queue"] {
        if let Some(v) = prov[key].as_str().filter(|s| s.len() >= 4) {
            probes.push(v.to_owned());
        }
    }
    if let Some(summary) = prov["summary"].as_str() {
        for token in summary.split_whitespace() {
            if (token.starts_with('/') && token.len() >= 5)
                || (token.contains('.') && token.len() >= 6)
            {
                probes.push(
                    token
                        .trim_matches(|c: char| c == ',' || c == ';')
                        .to_owned(),
                );
            }
        }
    }
    probes
}

/// How many files of a provider are read for what names a contract, how
/// many bytes in all, and how many files of each side are listed.
const BRIDGE_FILES: usize = 400;
const BRIDGE_BYTES: u64 = 8 * 1024 * 1024;
const BRIDGE_LISTED: usize = 24;

struct SemanticBridgeRegexes {
    php_route: Regex,
    php_api_resource: Regex,
    php_attr_route: Regex,
    js_route: Regex,
    decorator_route: Regex,
    py_route: Regex,
    rust_route: Regex,
    rust_attr_route: Regex,
    go_route: Regex,
    http_call_fn: Regex,
    url_literal: Regex,
    path_literal: Regex,
    php_event_new: Regex,
    php_event_dispatch_static: Regex,
    php_event_dispatch_str: Regex,
    generic_publish_str: Regex,
    generic_publish_new: Regex,
    php_event_listen: Regex,
    php_as_message_handler: Regex,
    php_handle_typed_event: Regex,
    decorator_event_listen: Regex,
    broker_listener: Regex,
    generic_subscribe_str: Regex,
    spring_event_listener: Regex,
    py_receiver: Regex,
    laravel_di_bind: Regex,
    symfony_as_alias: Regex,
    js_inject_provide: Regex,
    spring_di_qualifier: Regex,
    rust_di_type: Regex,
    sql_delete: Regex,
    sql_insert: Regex,
    sql_update: Regex,
    sql_create_alter: Regex,
    sql_select_from: Regex,
    sql_join: Regex,
    laravel_db_table: Regex,
    knex_table: Regex,
    prisma_model: Regex,
    orm_table_decl: Regex,
}

fn semantic_regexes() -> &'static SemanticBridgeRegexes {
    static RE: OnceLock<SemanticBridgeRegexes> = OnceLock::new();
    RE.get_or_init(|| SemanticBridgeRegexes {
        php_route: Regex::new(
            r#"(?i)\bRoute::(get|post|put|patch|delete|any|options)\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("php_route"),
        php_api_resource: Regex::new(r#"\bRoute::apiResource\s*\(\s*['"]([^'"]+)['"]"#)
            .expect("php_api_resource"),
        php_attr_route: Regex::new(
            r#"#\[\s*Route\s*\(\s*['"]([^'"]+)['"](?:[^)\]]*methods:\s*\[?\s*['"]([A-Za-z]+)['"])?"#,
        )
        .expect("php_attr_route"),
        js_route: Regex::new(
            r#"\b(?:app|router|fastify)\.(get|post|put|patch|delete|all)\s*\(\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("js_route"),
        decorator_route: Regex::new(
            r#"@(Get|Post|Put|Patch|Delete|RequestMapping|GetMapping|PostMapping|PutMapping|PatchMapping|DeleteMapping)\s*\(\s*(?:value\s*=\s*|path\s*=\s*)?['"]([^'"]+)['"]"#,
        )
        .expect("decorator_route"),
        py_route: Regex::new(
            r#"@(?:app|router|bp|blueprint)\.(get|post|put|patch|delete|route)\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("py_route"),
        rust_route: Regex::new(
            r#"\.route\s*\(\s*"([^"]+)"\s*,\s*(?:[a-z_]+::)?(get|post|put|patch|delete)?"#,
        )
        .expect("rust_route"),
        rust_attr_route: Regex::new(r#"#\[\s*(get|post|put|patch|delete)\s*\(\s*"([^"]+)""#)
            .expect("rust_attr_route"),
        go_route: Regex::new(
            r#"\.(HandleFunc|GET|POST|PUT|PATCH|DELETE|Get|Post|Put|Patch|Delete)\s*\(\s*"([^"]+)""#,
        )
        .expect("go_route"),
        http_call_fn: Regex::new(
            r#"(?i)(?:\bfetch|\baxios(?:\.(?:get|post|put|patch|delete|request))?|\bHttp::(?:get|post|put|patch|delete)|\.?(?:get|post|put|patch|delete|request))\s*\(\s*['"`]((?:https?://[^'"`?\s)]+)?/[^'"`?\s)]+)['"`]"#,
        )
        .expect("http_call_fn"),
        url_literal: Regex::new(r#"['"`](https?://[A-Za-z0-9_.:-]+(/[^'"`?\s)]+))['"`]"#)
            .expect("url_literal"),
        path_literal: Regex::new(
            r#"['"`](/[A-Za-z0-9_{}:.-]+(?:/[A-Za-z0-9_{}:.-]+)+)['"`]"#,
        )
        .expect("path_literal"),
        php_event_new: Regex::new(
            r#"\bevent\s*\(\s*new\s+(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)"#,
        )
        .expect("php_event_new"),
        php_event_dispatch_static: Regex::new(r#"\b([A-Z][A-Za-z0-9_]+)::dispatch\s*\("#)
            .expect("php_event_dispatch_static"),
        php_event_dispatch_str: Regex::new(
            r#"\b(?:Event::dispatch|dispatch)\s*\(\s*['"]([A-Za-z0-9_.:/-]{3,120})['"]"#,
        )
        .expect("php_event_dispatch_str"),
        generic_publish_str: Regex::new(
            r#"(?:\b|\.)(?:publish|emit|dispatch|produce|publishEvent|sendEvent|enqueue|broadcast)\s*\(\s*['"`]([A-Za-z0-9_.:/-]{3,120})['"`]"#,
        )
        .expect("generic_publish_str"),
        generic_publish_new: Regex::new(
            r#"(?:\b|\.)(?:publish|emit|dispatch|publishEvent)\s*\(\s*new\s+(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)"#,
        )
        .expect("generic_publish_new"),
        php_event_listen: Regex::new(
            r#"\bEvent::listen\s*\(\s*(?:['"]([A-Za-z0-9_\\.:/-]{3,120})['"]|(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)::class)"#,
        )
        .expect("php_event_listen"),
        php_as_message_handler: Regex::new(
            r#"#\[\s*AsMessageHandler[^\]]*\][\s\S]{0,240}?function\s+\w+\s*\(\s*(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)"#,
        )
        .expect("php_as_message_handler"),
        php_handle_typed_event: Regex::new(
            r#"function\s+(?:handle|__invoke)\s*\(\s*(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]*(?:Event|Message|Job|Notification))\b"#,
        )
        .expect("php_handle_typed_event"),
        decorator_event_listen: Regex::new(
            r#"@(?:OnEvent|EventPattern|MessagePattern)\s*\(\s*['"`]([A-Za-z0-9_.:/-]{3,120})['"`]"#,
        )
        .expect("decorator_event_listen"),
        broker_listener: Regex::new(
            r#"@(?:KafkaListener|RabbitListener)\s*\([^)]*(?:topics|queues)\s*=\s*['"]([A-Za-z0-9_.:/-]{3,120})['"]"#,
        )
        .expect("broker_listener"),
        generic_subscribe_str: Regex::new(
            r#"(?:\b|\.)(?:subscribe|consume|listen|on)\s*\(\s*['"`]([A-Za-z0-9_.:/-]{3,120})['"`]"#,
        )
        .expect("generic_subscribe_str"),
        spring_event_listener: Regex::new(
            r#"@EventListener\b[\s\S]{0,160}?\(\s*([A-Z][A-Za-z0-9_]+)\b"#,
        )
        .expect("spring_event_listener"),
        py_receiver: Regex::new(r#"@receiver\s*\(\s*([A-Za-z0-9_.]+)"#).expect("py_receiver"),
        laravel_di_bind: Regex::new(
            r#"->\s*(?:bind|singleton|scoped)\s*\(\s*(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)::class\s*,\s*(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)::class"#,
        )
        .expect("laravel_di_bind"),
        symfony_as_alias: Regex::new(
            r#"#\[\s*AsAlias\s*\(\s*(?:id:\s*)?(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)::class"#,
        )
        .expect("symfony_as_alias"),
        js_inject_provide: Regex::new(
            r#"(?:@Inject\s*\(\s*['"`]([A-Za-z0-9_.:/-]{4,120})['"`]|\bprovide\s*:\s*['"`]([A-Za-z0-9_.:/-]{4,120})['"`])"#,
        )
        .expect("js_inject_provide"),
        spring_di_qualifier: Regex::new(
            r#"@(?:Qualifier|Service)\s*\(\s*["']([A-Za-z0-9_.:/-]{4,120})["']"#,
        )
        .expect("spring_di_qualifier"),
        rust_di_type: Regex::new(
            r#"(?:(?:Extension|State)\s*<\s*(?:[A-Za-z0-9_]+::)*([A-Z][A-Za-z0-9_]+)\s*>|Arc\s*<\s*dyn\s+(?:[A-Za-z0-9_]+::)*([A-Z][A-Za-z0-9_]+))"#,
        )
        .expect("rust_di_type"),
        sql_delete: Regex::new(r#"(?i)\bDELETE\s+FROM\s+[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?"#)
            .expect("sql_delete"),
        sql_insert: Regex::new(
            r#"(?i)\bINSERT\s+(?:IGNORE\s+)?INTO\s+[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?"#,
        )
        .expect("sql_insert"),
        sql_update: Regex::new(
            r#"(?i)\bUPDATE\s+[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?\s+SET\b"#,
        )
        .expect("sql_update"),
        sql_create_alter: Regex::new(
            r#"(?i)\b(?:CREATE|ALTER)\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?"#,
        )
        .expect("sql_create_alter"),
        sql_select_from: Regex::new(
            r#"(?is)\bSELECT\b.{1,240}?\bFROM\s+[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?"#,
        )
        .expect("sql_select_from"),
        sql_join: Regex::new(r#"(?i)\bJOIN\s+[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?"#)
            .expect("sql_join"),
        laravel_db_table: Regex::new(
            r#"\bDB::table\s*\(\s*['"]([a-zA-Z_][a-zA-Z0-9_]{1,63})['"]\s*\)([^;\n]{0,200})"#,
        )
        .expect("laravel_db_table"),
        knex_table: Regex::new(
            r#"\bknex\s*\(\s*['"]([a-zA-Z_][a-zA-Z0-9_]{1,63})['"]\s*\)([^;\n]{0,200})"#,
        )
        .expect("knex_table"),
        prisma_model: Regex::new(
            r#"\bprisma\.([a-zA-Z_][a-zA-Z0-9_]{1,63})\.(findMany|findUnique|findFirst|count|aggregate|create|createMany|update|updateMany|delete|deleteMany|upsert)\b"#,
        )
        .expect("prisma_model"),
        orm_table_decl: Regex::new(
            r#"(?:\$table\s*=\s*['"]([a-zA-Z_][a-zA-Z0-9_]{1,63})['"]|#\[\s*ORM\\Table\s*\(\s*(?:name:\s*)?['"]([a-zA-Z_][a-zA-Z0-9_]{1,63})['"]|@Table\s*\(\s*(?:name\s*=\s*)?["']([a-zA-Z_][a-zA-Z0-9_]{1,63})["']|__tablename__\s*=\s*['"]([a-zA-Z_][a-zA-Z0-9_]{1,63})['"]|table!\s*\{\s*([a-zA-Z_][a-zA-Z0-9_]{1,63}))"#,
        )
        .expect("orm_table_decl"),
    })
}

#[derive(Debug, Default)]
struct RouteSite {
    display: String,
    methods: BTreeSet<String>,
    files: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct TopicSite {
    display: String,
    files: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct DiSite {
    display: String,
    kinds: BTreeSet<String>,
    files: BTreeSet<String>,
}

#[derive(Default)]
struct ServiceSemanticSignals {
    exposed_routes: BTreeMap<String, RouteSite>,
    called_routes: BTreeMap<String, RouteSite>,
    published_events: BTreeMap<String, TopicSite>,
    handled_events: BTreeMap<String, TopicSite>,
    di_bindings: BTreeMap<String, DiSite>,
    table_reads: BTreeMap<String, BTreeSet<String>>,
    table_writes: BTreeMap<String, BTreeSet<String>>,
}

fn normalize_route_path(raw: &str) -> Option<(String, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path_part = if let Some(rest) = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
    {
        let (_, path) = rest.split_once('/')?;
        format!("/{path}")
    } else if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    };
    let without_query = path_part
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    if without_query.is_empty()
        || without_query == "/"
        || matches!(
            without_query.to_ascii_lowercase().as_str(),
            "/health" | "/healthz" | "/ready" | "/metrics" | "/favicon.ico"
        )
        || [
            "/etc/", "/usr/", "/var/", "/tmp/", "/dev/", "/proc/", "/bin/", "/opt/",
        ]
        .iter()
        .any(|prefix| without_query.starts_with(prefix))
    {
        return None;
    }
    let norm_segments = without_query
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|seg| {
            if (seg.starts_with('{') && seg.ends_with('}'))
                || (seg.starts_with("${") && seg.ends_with('}'))
                || (seg.starts_with('<') && seg.ends_with('>'))
                || (seg.starts_with(':') && seg.len() > 1)
            {
                "{*}".to_owned()
            } else {
                seg.to_ascii_lowercase()
            }
        })
        .collect::<Vec<_>>();
    if norm_segments.is_empty() {
        return None;
    }
    Some((
        format!("/{}", norm_segments.join("/")),
        without_query.to_owned(),
    ))
}

fn routes_match(exposed_norm: &str, called_norm: &str) -> bool {
    if exposed_norm == called_norm {
        return true;
    }
    let exp: Vec<&str> = exposed_norm.trim_start_matches('/').split('/').collect();
    let cal: Vec<&str> = called_norm.trim_start_matches('/').split('/').collect();
    if exp.len() < 2 || exp.len() != cal.len() {
        return false;
    }
    exp.iter()
        .zip(cal.iter())
        .all(|(e, c)| e == c || *e == "{*}" || *c == "{*}")
}

fn normalize_event_topic(raw: &str) -> Option<(String, String)> {
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '\'' | '"' | '`' | ' '));
    let short = trimmed.rsplit('\\').next().unwrap_or(trimmed).trim();
    if short.len() < 3 {
        return None;
    }
    let lower = short.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "error"
            | "data"
            | "end"
            | "close"
            | "open"
            | "exit"
            | "click"
            | "change"
            | "input"
            | "focus"
            | "blur"
            | "connect"
            | "disconnect"
            | "listening"
            | "message"
            | "ready"
            | "drain"
            | "finish"
            | "pipe"
            | "unpipe"
            | "warning"
            | "timeout"
    ) {
        return None;
    }
    Some((lower, short.to_owned()))
}

fn normalize_di_symbol(raw: &str) -> Option<(String, String)> {
    let trimmed = raw
        .trim()
        .rsplit(['\\', ':'])
        .next()
        .unwrap_or(raw.trim())
        .trim();
    if trimmed.len() < 4 {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "service"
            | "client"
            | "config"
            | "logger"
            | "context"
            | "state"
            | "appstate"
            | "request"
            | "response"
            | "string"
            | "array"
            | "object"
    ) {
        return None;
    }
    Some((lower, trimmed.to_owned()))
}

fn normalize_table_name(raw: &str) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.len() < 2 {
        return None;
    }
    if matches!(
        lower.as_str(),
        "select"
            | "from"
            | "where"
            | "set"
            | "values"
            | "null"
            | "true"
            | "false"
            | "dual"
            | "inner"
            | "left"
            | "right"
            | "outer"
            | "join"
            | "as"
            | "on"
            | "and"
            | "or"
            | "table"
            | "into"
            | "exists"
            | "case"
            | "when"
            | "then"
            | "else"
            | "end"
            | "order"
            | "group"
            | "by"
            | "limit"
            | "offset"
            | "having"
            | "distinct"
            | "count"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "not"
            | "is"
            | "in"
            | "like"
            | "between"
            | "union"
            | "all"
            | "any"
    ) {
        return None;
    }
    Some(lower)
}

impl ServiceSemanticSignals {
    fn add_exposed_route(&mut self, raw_path: &str, method: Option<&str>, file: &str) {
        if let Some((norm, display)) = normalize_route_path(raw_path) {
            let entry = self
                .exposed_routes
                .entry(norm)
                .or_insert_with(|| RouteSite {
                    display,
                    methods: BTreeSet::new(),
                    files: BTreeSet::new(),
                });
            if let Some(m) = method.map(str::trim).filter(|m| !m.is_empty()) {
                entry.methods.insert(m.to_ascii_uppercase());
            }
            entry.files.insert(file.to_owned());
        }
    }

    fn add_called_route(&mut self, raw_path: &str, method: Option<&str>, file: &str) {
        if let Some((norm, display)) = normalize_route_path(raw_path) {
            let entry = self.called_routes.entry(norm).or_insert_with(|| RouteSite {
                display,
                methods: BTreeSet::new(),
                files: BTreeSet::new(),
            });
            if let Some(m) = method.map(str::trim).filter(|m| !m.is_empty()) {
                entry.methods.insert(m.to_ascii_uppercase());
            }
            entry.files.insert(file.to_owned());
        }
    }

    fn add_published_event(&mut self, raw: &str, file: &str) {
        if let Some((key, display)) = normalize_event_topic(raw) {
            let entry = self
                .published_events
                .entry(key)
                .or_insert_with(|| TopicSite {
                    display,
                    files: BTreeSet::new(),
                });
            entry.files.insert(file.to_owned());
        }
    }

    fn add_handled_event(&mut self, raw: &str, file: &str) {
        if let Some((key, display)) = normalize_event_topic(raw) {
            let entry = self.handled_events.entry(key).or_insert_with(|| TopicSite {
                display,
                files: BTreeSet::new(),
            });
            entry.files.insert(file.to_owned());
        }
    }

    fn add_di_binding(&mut self, raw: &str, kind: &str, file: &str) {
        if let Some((key, display)) = normalize_di_symbol(raw) {
            let entry = self.di_bindings.entry(key).or_insert_with(|| DiSite {
                display,
                kinds: BTreeSet::new(),
                files: BTreeSet::new(),
            });
            entry.kinds.insert(kind.to_owned());
            entry.files.insert(file.to_owned());
        }
    }

    fn add_table_read(&mut self, raw: &str, file: &str) {
        if let Some(tbl) = normalize_table_name(raw) {
            self.table_reads
                .entry(tbl)
                .or_default()
                .insert(file.to_owned());
        }
    }

    fn add_table_write(&mut self, raw: &str, file: &str) {
        if let Some(tbl) = normalize_table_name(raw) {
            self.table_writes
                .entry(tbl)
                .or_default()
                .insert(file.to_owned());
        }
    }

    fn scan_source(&mut self, rel_path: &str, content: &str) {
        let re = semantic_regexes();
        // 1. Route definitions
        for cap in re.php_route.captures_iter(content) {
            self.add_exposed_route(&cap[2], Some(&cap[1]), rel_path);
        }
        for cap in re.php_api_resource.captures_iter(content) {
            self.add_exposed_route(&cap[1], Some("RESOURCE"), rel_path);
        }
        for cap in re.php_attr_route.captures_iter(content) {
            self.add_exposed_route(&cap[1], cap.get(2).map(|m| m.as_str()), rel_path);
        }
        for cap in re.js_route.captures_iter(content) {
            self.add_exposed_route(&cap[2], Some(&cap[1]), rel_path);
        }
        for cap in re.decorator_route.captures_iter(content) {
            let method = match &cap[1] {
                "Get" | "GetMapping" => Some("GET"),
                "Post" | "PostMapping" => Some("POST"),
                "Put" | "PutMapping" => Some("PUT"),
                "Patch" | "PatchMapping" => Some("PATCH"),
                "Delete" | "DeleteMapping" => Some("DELETE"),
                _ => None,
            };
            self.add_exposed_route(&cap[2], method, rel_path);
        }
        for cap in re.py_route.captures_iter(content) {
            let method = match &cap[1] {
                "route" => None,
                other => Some(other),
            };
            self.add_exposed_route(&cap[2], method, rel_path);
        }
        for cap in re.rust_route.captures_iter(content) {
            self.add_exposed_route(&cap[1], cap.get(2).map(|m| m.as_str()), rel_path);
        }
        for cap in re.rust_attr_route.captures_iter(content) {
            self.add_exposed_route(&cap[2], Some(&cap[1]), rel_path);
        }
        for cap in re.go_route.captures_iter(content) {
            let method = match &cap[1] {
                "HandleFunc" => None,
                other => Some(other),
            };
            self.add_exposed_route(&cap[2], method, rel_path);
        }

        // 2. HTTP / route calls
        for cap in re.http_call_fn.captures_iter(content) {
            self.add_called_route(&cap[1], None, rel_path);
        }
        for cap in re.url_literal.captures_iter(content) {
            self.add_called_route(&cap[1], None, rel_path);
        }
        for cap in re.path_literal.captures_iter(content) {
            if let Some((norm, _)) = normalize_route_path(&cap[1])
                && !self.exposed_routes.contains_key(&norm)
            {
                self.add_called_route(&cap[1], None, rel_path);
            }
        }

        // 3. Event publishers & handlers
        for cap in re.php_event_new.captures_iter(content) {
            self.add_published_event(&cap[1], rel_path);
        }
        for cap in re.php_event_dispatch_static.captures_iter(content) {
            self.add_published_event(&cap[1], rel_path);
        }
        for cap in re.php_event_dispatch_str.captures_iter(content) {
            self.add_published_event(&cap[1], rel_path);
        }
        for cap in re.generic_publish_str.captures_iter(content) {
            self.add_published_event(&cap[1], rel_path);
        }
        for cap in re.generic_publish_new.captures_iter(content) {
            self.add_published_event(&cap[1], rel_path);
        }
        for cap in re.php_event_listen.captures_iter(content) {
            if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
                self.add_handled_event(m.as_str(), rel_path);
            }
        }
        for cap in re.php_as_message_handler.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.php_handle_typed_event.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.decorator_event_listen.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.broker_listener.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.generic_subscribe_str.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.spring_event_listener.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.py_receiver.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }

        // 4. DI / service bindings
        for cap in re.laravel_di_bind.captures_iter(content) {
            self.add_di_binding(&cap[1], "binds", rel_path);
            self.add_di_binding(&cap[2], "binds", rel_path);
        }
        for cap in re.symfony_as_alias.captures_iter(content) {
            self.add_di_binding(&cap[1], "binds", rel_path);
        }
        for cap in re.js_inject_provide.captures_iter(content) {
            if let Some(m) = cap.get(1) {
                self.add_di_binding(m.as_str(), "injects", rel_path);
            }
            if let Some(m) = cap.get(2) {
                self.add_di_binding(m.as_str(), "binds", rel_path);
            }
        }
        for cap in re.spring_di_qualifier.captures_iter(content) {
            self.add_di_binding(&cap[1], "binds", rel_path);
        }
        for cap in re.rust_di_type.captures_iter(content) {
            if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
                self.add_di_binding(m.as_str(), "injects", rel_path);
            }
        }

        // 5. Database table reads & writes
        for cap in re.sql_delete.captures_iter(content) {
            self.add_table_write(&cap[1], rel_path);
        }
        for cap in re.sql_insert.captures_iter(content) {
            self.add_table_write(&cap[1], rel_path);
        }
        for cap in re.sql_update.captures_iter(content) {
            self.add_table_write(&cap[1], rel_path);
        }
        for cap in re.sql_create_alter.captures_iter(content) {
            self.add_table_write(&cap[1], rel_path);
        }
        for cap in re.sql_select_from.captures_iter(content) {
            self.add_table_read(&cap[1], rel_path);
        }
        for cap in re.sql_join.captures_iter(content) {
            self.add_table_read(&cap[1], rel_path);
        }
        for cap in re.laravel_db_table.captures_iter(content) {
            let chain = cap[2].to_ascii_lowercase();
            if [
                "insert",
                "update",
                "delete",
                "upsert",
                "increment",
                "decrement",
                "truncate",
            ]
            .iter()
            .any(|kw| chain.contains(kw))
            {
                self.add_table_write(&cap[1], rel_path);
            } else {
                self.add_table_read(&cap[1], rel_path);
            }
        }
        for cap in re.knex_table.captures_iter(content) {
            let chain = cap[2].to_ascii_lowercase();
            if [".insert", ".update", ".del", ".delete", ".truncate"]
                .iter()
                .any(|kw| chain.contains(kw))
            {
                self.add_table_write(&cap[1], rel_path);
            } else {
                self.add_table_read(&cap[1], rel_path);
            }
        }
        for cap in re.prisma_model.captures_iter(content) {
            match &cap[2] {
                "create" | "createMany" | "update" | "updateMany" | "delete" | "deleteMany"
                | "upsert" => self.add_table_write(&cap[1], rel_path),
                _ => self.add_table_read(&cap[1], rel_path),
            }
        }
        let lower_content = content.to_ascii_lowercase();
        let has_write_hint = ["insert", "update", "delete", "save(", "persist(", "create("]
            .iter()
            .any(|kw| lower_content.contains(kw));
        let has_read_hint = ["select", "find", "get(", "fetch", "query", "where("]
            .iter()
            .any(|kw| lower_content.contains(kw));
        for cap in re.orm_table_decl.captures_iter(content) {
            if let Some(m) = cap
                .get(1)
                .or_else(|| cap.get(2))
                .or_else(|| cap.get(3))
                .or_else(|| cap.get(4))
                .or_else(|| cap.get(5))
            {
                if has_write_hint {
                    self.add_table_write(m.as_str(), rel_path);
                }
                if has_read_hint || !has_write_hint {
                    self.add_table_read(m.as_str(), rel_path);
                }
            }
        }
    }

    fn ingest_semantic_relation(&mut self, relation: &str, target_label: &str, from_file: &str) {
        match relation {
            "exposes_route" | "route" => {
                let raw = target_label.strip_prefix("route:").unwrap_or(target_label);
                let (method, path) = match raw.trim().split_once(' ') {
                    Some((m, p)) if p.starts_with('/') => (Some(m), p),
                    _ => (None, raw),
                };
                self.add_exposed_route(path, method, from_file);
            }
            "calls_route" => {
                let raw = target_label.strip_prefix("route:").unwrap_or(target_label);
                let (method, path) = match raw.trim().split_once(' ') {
                    Some((m, p)) if p.starts_with('/') => (Some(m), p),
                    _ => (None, raw),
                };
                self.add_called_route(path, method, from_file);
            }
            "di_binds" | "di_binding" => {
                let raw = target_label
                    .strip_prefix("service:")
                    .unwrap_or(target_label);
                self.add_di_binding(raw, "binds", from_file);
            }
            "publishes_event" => {
                let raw = target_label.strip_prefix("event:").unwrap_or(target_label);
                self.add_published_event(raw, from_file);
            }
            "handles_event" => {
                let raw = target_label.strip_prefix("event:").unwrap_or(target_label);
                self.add_handled_event(raw, from_file);
            }
            "reads_table" | "table_read" => {
                let raw = target_label.strip_prefix("table:").unwrap_or(target_label);
                self.add_table_read(raw, from_file);
            }
            "writes_table" | "table_write" => {
                let raw = target_label.strip_prefix("table:").unwrap_or(target_label);
                self.add_table_write(raw, from_file);
            }
            _ => {}
        }
    }

    fn ingest_ir_store(
        &mut self,
        graphs: Option<&Graphs>,
        state_dir: &Path,
        graph_id: &str,
        certain: bool,
    ) -> Option<Vec<String>> {
        let store = graphs
            .and_then(|g| g.get_ir_store_readonly(graph_id))
            .or_else(|| {
                crate::graph_ir::GraphIrStore::open_readonly(state_dir, graph_id)
                    .ok()
                    .flatten()
                    .map(Arc::new)
            })?;
        let (files, edges) = store.semantic_edges_for_system().ok()?;
        if files.is_empty() && edges.is_empty() {
            return None;
        }
        for (file_path, kind, target, _source_symbol, _line, evidence) in &edges {
            let guessed = evidence.eq_ignore_ascii_case("AMBIGUOUS")
                || evidence.eq_ignore_ascii_case("INFERRED");
            if certain && guessed {
                continue;
            }
            self.ingest_semantic_relation(kind, target, file_path);
        }
        Some(files)
    }

    fn ingest_index(&mut self, index: &Index, certain: bool) {
        let files = index.files();
        let symbols = index.symbol_entries();
        for &(from, to, kind_id, guessed) in index.uses() {
            if certain && guessed {
                continue;
            }
            let relation = index.kind_name(kind_id);
            let from_file = files
                .get(symbols[from as usize].file as usize)
                .map(String::as_str)
                .unwrap_or("");
            let target_label = index.symbol_label(to);
            self.ingest_semantic_relation(relation, target_label, from_file);
        }
    }
}

fn read_bounded_service_file(
    root: &Path,
    canon_root: Option<&Path>,
    rel_path: &str,
) -> Option<(String, u64)> {
    if rel_path.is_empty()
        || rel_path.starts_with('/')
        || rel_path.contains('\\')
        || rel_path
            .split('/')
            .any(|part| part.is_empty() || part == "..")
    {
        return None;
    }
    let full_path = root.join(rel_path);
    if let Some(canon_root) = canon_root {
        let canon_file = infra_core::fs::canonicalize(&full_path).ok()?;
        if !canon_file.starts_with(canon_root) {
            return None;
        }
    }
    let meta = fs::symlink_metadata(&full_path).ok()?;
    if !meta.is_file() || meta.len() > MAX_UNDECLARED_SCAN_BYTES {
        return None;
    }
    let file = fs::File::open(&full_path).ok()?;
    let mut buf = Vec::with_capacity(meta.len().min(MAX_UNDECLARED_SCAN_BYTES) as usize);
    file.take(MAX_UNDECLARED_SCAN_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() as u64 > MAX_UNDECLARED_SCAN_BYTES {
        return None;
    }
    let len = buf.len() as u64;
    let content = String::from_utf8(buf).ok()?;
    Some((content, len))
}

#[cfg(test)]
fn collect_service_semantics_with(
    state_dir: Option<&Path>,
    target: Option<&ResolvedServiceTarget>,
    index: Option<&Arc<Index>>,
    certain: bool,
    need_content_probes: bool,
    on_file: impl FnMut(&str, &str),
) -> (ServiceSemanticSignals, usize, bool) {
    collect_service_semantics_with_graphs(
        None,
        state_dir,
        target,
        index,
        certain,
        need_content_probes,
        on_file,
    )
}

fn collect_service_semantics_with_graphs(
    graphs: Option<&Graphs>,
    state_dir: Option<&Path>,
    target: Option<&ResolvedServiceTarget>,
    index: Option<&Arc<Index>>,
    certain: bool,
    need_content_probes: bool,
    mut on_file: impl FnMut(&str, &str),
) -> (ServiceSemanticSignals, usize, bool) {
    let mut signals = ServiceSemanticSignals::default();
    let ir_files = state_dir.zip(target).and_then(|(state, target)| {
        signals.ingest_ir_store(graphs, state, &target.graph_id, certain)
    });
    let ir_loaded = ir_files.is_some();
    if !ir_loaded && let Some(idx) = index {
        signals.ingest_index(idx, certain);
    }
    let Some(target) = target else {
        return (signals, 0, false);
    };
    if ir_loaded && !need_content_probes {
        return (signals, 0, false);
    }
    let canon_root = infra_core::fs::canonicalize(&target.root).ok();
    let files = match (ir_files, index) {
        (Some(files), _) => files
            .into_iter()
            .filter(|path| !graph_map::is_test_path(path))
            .collect::<Vec<_>>(),
        (None, Some(idx)) => idx
            .files()
            .iter()
            .filter(|path| !graph_map::is_test_path(path))
            .cloned()
            .collect::<Vec<_>>(),
        (None, None) => collect_service_code_files(&target.root, BRIDGE_FILES),
    };
    let mut files_read = 0usize;
    let mut bytes_read = 0u64;
    let mut cut = false;
    for rel_path in files {
        if rel_path == "ai-service.json" || rel_path.ends_with(".lock") {
            continue;
        }
        if files_read >= BRIDGE_FILES || bytes_read > BRIDGE_BYTES {
            cut = true;
            break;
        }
        let Some((content, len)) =
            read_bounded_service_file(&target.root, canon_root.as_deref(), &rel_path)
        else {
            continue;
        };
        files_read += 1;
        bytes_read += len;
        if !ir_loaded {
            signals.scan_source(&rel_path, &content);
        }
        on_file(&rel_path, &content);
    }
    (signals, files_read, cut)
}

fn build_semantic_bridges(
    from: &str,
    to: &str,
    from_sig: &ServiceSemanticSignals,
    to_sig: &ServiceSemanticSignals,
) -> Value {
    let mut routes = Vec::new();
    let mut push_routes = |caller: &str,
                           provider: &str,
                           direction: &str,
                           calls: &BTreeMap<String, RouteSite>,
                           exposed: &BTreeMap<String, RouteSite>| {
        for (cal_norm, cal_site) in calls {
            for (exp_norm, exp_site) in exposed {
                if routes_match(exp_norm, cal_norm) {
                    routes.push(json!({
                        "route": exp_site.display,
                        "method": exp_site.methods.iter().next().map(String::as_str).unwrap_or("ANY"),
                        "methods": exp_site.methods.iter().cloned().collect::<Vec<_>>(),
                        "direction": direction,
                        "caller": caller,
                        "provider": provider,
                        "caller_files": cal_site.files.iter().cloned().collect::<Vec<_>>(),
                        "provider_files": exp_site.files.iter().cloned().collect::<Vec<_>>(),
                    }));
                }
            }
        }
    };
    push_routes(
        from,
        to,
        "from->to",
        &from_sig.called_routes,
        &to_sig.exposed_routes,
    );
    push_routes(
        to,
        from,
        "to->from",
        &to_sig.called_routes,
        &from_sig.exposed_routes,
    );
    routes.truncate(BRIDGE_LISTED);

    let mut events = Vec::new();
    let mut push_events = |producer: &str,
                           consumer: &str,
                           direction: &str,
                           pubs: &BTreeMap<String, TopicSite>,
                           handlers: &BTreeMap<String, TopicSite>| {
        for (topic_key, pub_site) in pubs {
            if let Some(sub_site) = handlers.get(topic_key) {
                events.push(json!({
                    "event": pub_site.display,
                    "direction": direction,
                    "producer": producer,
                    "consumer": consumer,
                    "producer_files": pub_site.files.iter().cloned().collect::<Vec<_>>(),
                    "consumer_files": sub_site.files.iter().cloned().collect::<Vec<_>>(),
                }));
            }
        }
    };
    push_events(
        from,
        to,
        "from->to",
        &from_sig.published_events,
        &to_sig.handled_events,
    );
    push_events(
        to,
        from,
        "to->from",
        &to_sig.published_events,
        &from_sig.handled_events,
    );
    events.truncate(BRIDGE_LISTED);

    let mut di_bindings = Vec::new();
    for (key, left) in &from_sig.di_bindings {
        if let Some(right) = to_sig.di_bindings.get(key) {
            di_bindings.push(json!({
                "service": left.display,
                "from_kinds": left.kinds.iter().cloned().collect::<Vec<_>>(),
                "to_kinds": right.kinds.iter().cloned().collect::<Vec<_>>(),
                "from_files": left.files.iter().cloned().collect::<Vec<_>>(),
                "to_files": right.files.iter().cloned().collect::<Vec<_>>(),
            }));
        }
    }
    di_bindings.truncate(BRIDGE_LISTED);

    let mut all_tables = BTreeSet::new();
    for tbl in from_sig
        .table_reads
        .keys()
        .chain(from_sig.table_writes.keys())
    {
        if to_sig.table_reads.contains_key(tbl) || to_sig.table_writes.contains_key(tbl) {
            all_tables.insert(tbl.clone());
        }
    }
    let mut tables = Vec::new();
    for tbl in all_tables.into_iter().take(BRIDGE_LISTED) {
        let from_r = from_sig.table_reads.get(&tbl);
        let from_w = from_sig.table_writes.get(&tbl);
        let to_r = to_sig.table_reads.get(&tbl);
        let to_w = to_sig.table_writes.get(&tbl);
        let mut from_access = Vec::new();
        if from_r.is_some() {
            from_access.push("read");
        }
        if from_w.is_some() {
            from_access.push("write");
        }
        let mut to_access = Vec::new();
        if to_r.is_some() {
            to_access.push("read");
        }
        if to_w.is_some() {
            to_access.push("write");
        }
        let coupling = if from_w.is_some() && to_w.is_some() {
            "shared_write"
        } else if from_w.is_some() || to_w.is_some() {
            "cross_read_write"
        } else {
            "shared_read"
        };
        let mut from_files = BTreeSet::new();
        for f in from_r
            .into_iter()
            .flatten()
            .chain(from_w.into_iter().flatten())
        {
            from_files.insert(f.clone());
        }
        let mut to_files = BTreeSet::new();
        for f in to_r.into_iter().flatten().chain(to_w.into_iter().flatten()) {
            to_files.insert(f.clone());
        }
        tables.push(json!({
            "table": tbl,
            "coupling": coupling,
            "from_access": from_access,
            "to_access": to_access,
            "from_files": from_files.into_iter().collect::<Vec<_>>(),
            "to_files": to_files.into_iter().collect::<Vec<_>>(),
        }));
    }

    let total = routes.len() + events.len() + di_bindings.len() + tables.len();
    json!({
        "routes": routes,
        "events": events,
        "di_bindings": di_bindings,
        "tables": tables,
        "summary": {
            "routes": routes.len(),
            "events": events.len(),
            "di_bindings": di_bindings.len(),
            "tables": tables.len(),
            "total": total,
        },
    })
}

struct BridgeViewContext<'a> {
    target_spec: Option<&'a str>,
    run_id: Option<&'a str>,
    opened_by_service: Option<&'a BTreeMap<String, BTreeSet<String>>>,
}

/// The code on the two sides of a link between two services. Of the
/// consumer, the files that name what it uses of the provider, as the code
/// references of the system graph found them. Of the provider, the files
/// its passport says a contract is made in, and the files that name the
/// contract. Each file is told how many files of its own service a change
/// of it reaches, where the service has a code graph: that is how far the
/// contract goes into each side.
fn bridge_view(
    graphs: &Graphs,
    system: &System,
    targets: &[ResolvedServiceTarget],
    link: &Value,
    contract: Option<&str>,
    certain: bool,
    ctx: &BridgeViewContext<'_>,
) -> Value {
    let (from, to) = (
        link["from"].as_str().unwrap_or(""),
        link["to"].as_str().unwrap_or(""),
    );
    let ids = match contract {
        Some(contract) => vec![contract.to_owned()],
        None => link["contracts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>(),
    };
    let index_of = |service: &str| {
        targets
            .iter()
            .find(|target| target.service_id == service)
            .and_then(|target| graphs.index_for_target_or_live(&target.graph_id, ctx.target_spec))
    };
    let reach = |index: &Option<Arc<Index>>, path: &str| match index {
        Some(index) => index
            .blast(path, certain)
            .map(|blast| json!({"files": blast["files"], "tests": blast["tests"]}))
            .unwrap_or(Value::Null),
        None => Value::Null,
    };
    // What the passport names first, then code, then the documents that
    // name the contract as well.
    let document = |path: &str| {
        let path = path.to_ascii_lowercase();
        [".md", ".markdown", ".rst", ".txt", ".adoc"]
            .iter()
            .any(|end| path.ends_with(end))
    };
    let service_opened = |service: &str, path: &str| -> bool {
        let clean = path.replace('\\', "/").trim_start_matches("./").to_owned();
        ctx.opened_by_service
            .and_then(|map| map.get(service))
            .is_some_and(|set| set.contains(&clean))
    };
    let listed = |service: &str,
                  sites: BTreeMap<String, (BTreeSet<String>, Value)>,
                  index: &Option<Arc<Index>>| {
        let total = sites.len();
        let more = total.saturating_sub(BRIDGE_LISTED);
        let opened_total = ctx.opened_by_service.map_or(0usize, |_| {
            sites
                .keys()
                .filter(|path| service_opened(service, path))
                .count()
        });
        let mut sites = sites.into_iter().collect::<Vec<_>>();
        sites.sort_by_key(|(path, (_, row))| {
            (row["declared"] != true, document(path), path.clone())
        });
        let rows = sites
            .into_iter()
            .take(BRIDGE_LISTED)
            .map(|(path, (contracts, mut row))| {
                row["document"] = json!(document(&path));
                row["path"] = json!(path);
                row["contracts"] = json!(contracts);
                row["in_graph"] = json!(
                    index
                        .as_ref()
                        .is_some_and(|index| index.file_id(&path).is_some())
                );
                row["reach"] = reach(index, &path);
                if ctx.run_id.is_some() {
                    row["opened_by_run"] = json!(service_opened(service, &path));
                }
                row
            })
            .collect::<Vec<_>>();
        (rows, more, opened_total, total)
    };
    // The consumer: what its code was found to name of the provider.
    let consumer_target = targets.iter().find(|target| target.service_id == from);
    let consumer_loaded = index_of(from);
    let consumer_is_snapshot = consumer_loaded.as_ref().is_some_and(|(_, snap)| *snap);
    let consumer_index = consumer_loaded.map(|(idx, _)| idx);
    let mut certain_refs = 0usize;
    let mut guessed_refs = 0usize;
    let mut consumer: BTreeMap<String, (BTreeSet<String>, Value)> = BTreeMap::new();
    for found in link["code_refs"].as_array().into_iter().flatten() {
        let named = found["contract"].as_str();
        if let (Some(asked), Some(named)) = (contract, named)
            && asked != named
        {
            continue;
        }
        let Some(path) = found["from_path"].as_str() else {
            continue;
        };
        if matches!(found["certainty"].as_str(), Some("INFERRED" | "guessed")) {
            guessed_refs += 1;
        } else {
            certain_refs += 1;
        }
        let entry = consumer.entry(path.to_owned()).or_insert_with(|| {
            (
                BTreeSet::new(),
                json!({"detail": found["detail"], "certainty": found["certainty"]}),
            )
        });
        if let Some(sym) = found["from_symbol"].as_str().filter(|s| !s.is_empty()) {
            let list = entry
                .1
                .as_object_mut()
                .expect("consumer row object")
                .entry("from_symbols")
                .or_insert_with(|| json!([]));
            if let Some(arr) = list.as_array_mut()
                && !arr.iter().any(|v| v.as_str() == Some(sym))
            {
                arr.push(json!(sym));
            }
        }
        if let Some(named) = named {
            entry.0.insert(named.to_owned());
        }
    }
    // The provider: where its passport says each contract is made, and
    // what of its code names the contract.
    let provider_target = targets.iter().find(|target| target.service_id == to);
    let provider_loaded = index_of(to);
    let provider_is_snapshot = provider_loaded.as_ref().is_some_and(|(_, snap)| *snap);
    let provider_index = provider_loaded.map(|(idx, _)| idx);
    let provides = system
        .services
        .get(to)
        .and_then(|service| service.manifest.as_ref())
        .and_then(|manifest| manifest["provides"].as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|prov| {
            prov["id"]
                .as_str()
                .is_some_and(|id| ids.iter().any(|asked| asked == id))
        })
        .collect::<Vec<_>>();
    let mut provider: BTreeMap<String, (BTreeSet<String>, Value)> = BTreeMap::new();
    for prov in &provides {
        let id = prov["id"].as_str().unwrap_or("");
        for source in prov["sources"].as_array().into_iter().flatten() {
            if let Some(path) = source.as_str().filter(|path| !path.is_empty()) {
                let entry = provider.entry(path.to_owned()).or_insert_with(|| {
                    (BTreeSet::new(), json!({"declared": true, "found": false}))
                });
                entry.0.insert(id.to_owned());
                entry.1["declared"] = json!(true);
            }
        }
    }
    let probes = provides
        .iter()
        .map(|prov| {
            (
                prov["id"].as_str().unwrap_or("").to_owned(),
                contract_probes(prov),
            )
        })
        .filter(|(_, probes)| !probes.is_empty())
        .collect::<Vec<_>>();
    let provider_state_dir = (!provider_is_snapshot).then(|| graphs.state_dir());
    let consumer_state_dir = (!consumer_is_snapshot).then(|| graphs.state_dir());
    let (provider_semantics, files_read, raw_cut) = collect_service_semantics_with_graphs(
        Some(graphs),
        provider_state_dir,
        provider_target,
        provider_index.as_ref(),
        certain,
        !probes.is_empty(),
        |rel_path, content| {
            if !probes.is_empty() {
                for (id, probes) in &probes {
                    if probes.iter().any(|probe| content.contains(probe.as_str())) {
                        let entry = provider.entry(rel_path.to_owned()).or_insert_with(|| {
                            (BTreeSet::new(), json!({"declared": false, "found": true}))
                        });
                        entry.0.insert(id.clone());
                        entry.1["found"] = json!(true);
                    }
                }
            }
        },
    );
    let scanned = if probes.is_empty() { 0 } else { files_read };
    let cut = !probes.is_empty() && raw_cut;
    let (consumer_semantics, _, _) = collect_service_semantics_with_graphs(
        Some(graphs),
        consumer_state_dir,
        consumer_target,
        consumer_index.as_ref(),
        certain,
        false,
        |_, _| {},
    );
    let semantic_bridges =
        build_semantic_bridges(from, to, &consumer_semantics, &provider_semantics);
    let declared_sites = provider
        .values()
        .filter(|(_, row)| row["declared"] == true)
        .count();
    let discovered_sites = provider
        .values()
        .filter(|(_, row)| row["found"] == true && row["declared"] != true)
        .count();
    let certain_total = certain_refs + declared_sites;
    let total_confidence = certain_total + guessed_refs + discovered_sites;
    let confidence_ratio = if total_confidence == 0 {
        1.0
    } else {
        ((certain_total as f64 / total_confidence as f64) * 1000.0).round() / 1000.0
    };
    let (consumer_sites, consumer_more, consumer_opened, consumer_total) =
        listed(from, consumer, &consumer_index);
    let (provider_sites, provider_more, provider_opened, provider_total) =
        listed(to, provider, &provider_index);
    let mut out = json!({
        "kind": "system_bridge",
        "from": from,
        "to": to,
        "direction": link["direction"],
        "contracts": ids,
        "contract": contract,
        "certain": certain,
        "confidence": {
            "certain_refs": certain_refs,
            "guessed_refs": guessed_refs,
            "declared_sites": declared_sites,
            "discovered_sites": discovered_sites,
            "ratio": confidence_ratio,
        },
        "consumer": {
            "service": from,
            "graph": consumer_index.is_some(),
            "sites": consumer_sites,
            "more": consumer_more,
        },
        "provider": {
            "service": to,
            "graph": provider_index.is_some(),
            "sites": provider_sites,
            "more": provider_more,
            "scanned": scanned,
            "cut": cut,
        },
        "semantic_bridges": semantic_bridges,
    });
    if let Some(ts) = ctx.target_spec {
        out["target"] = json!(ts);
    }
    if let Some(rid) = ctx.run_id {
        out["run_id"] = json!(rid);
        out["run_coverage"] = json!({
            "run_id": rid,
            "consumer_opened": consumer_opened,
            "consumer_total": consumer_total,
            "provider_opened": provider_opened,
            "provider_total": provider_total,
        });
    }
    out
}

fn collect_service_code_files(root: &Path, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited_dirs = 0usize;
    while let Some(dir) = stack.pop() {
        if out.len() >= limit || visited_dirs >= 120 {
            break;
        }
        visited_dirs += 1;
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut sorted: Vec<_> = entries.flatten().collect();
        sorted.sort_by_key(|e| e.file_name());
        for entry in sorted {
            if out.len() >= limit {
                break;
            }
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.')
                || matches!(
                    name_str.as_ref(),
                    "vendor" | "node_modules" | "target" | "dist" | "build" | "storage" | "var"
                )
            {
                continue;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(
                    ext.as_str(),
                    "php"
                        | "rs"
                        | "ts"
                        | "tsx"
                        | "js"
                        | "jsx"
                        | "py"
                        | "go"
                        | "java"
                        | "kt"
                        | "rb"
                        | "cs"
                        | "swift"
                        | "scala"
                ) && let Ok(rel) = path.strip_prefix(root)
                {
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    if !graph_map::is_test_path(&rel_str) {
                        out.push(rel_str);
                    }
                }
            }
        }
    }
    out
}

fn intra_service_caller_paths(idx: &Index, seed_paths: &[String], certain: bool) -> Vec<String> {
    let mut visited: BTreeSet<u32> = seed_paths.iter().filter_map(|p| idx.file_id(p)).collect();
    if visited.is_empty() {
        return Vec::new();
    }
    let mut frontier = visited.clone();
    let files = idx.files();
    for _ in 0..2 {
        let mut next = BTreeSet::new();
        for (from, to, pair) in idx.pairs() {
            if frontier.contains(to)
                && !visited.contains(from)
                && pair.weight(certain) > 0
                && let Some(p) = files.get(*from as usize)
                && !graph_map::is_test_path(p)
            {
                visited.insert(*from);
                next.insert(*from);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    let seed_set: BTreeSet<&str> = seed_paths.iter().map(String::as_str).collect();
    visited
        .into_iter()
        .filter_map(|fid| files.get(fid as usize))
        .filter(|p| !seed_set.contains(p.as_str()))
        .cloned()
        .collect()
}

fn augment_summaries_with_intra_callers(
    graphs: &Graphs,
    targets: &[ResolvedServiceTarget],
    summaries: &mut BTreeMap<String, ServiceCodeSummary>,
    certain: bool,
) {
    for target in targets {
        if target.service_id == "__system__" {
            continue;
        }
        let Some(summary) = summaries.get_mut(&target.service_id) else {
            continue;
        };
        if summary.changed_files.is_empty() {
            continue;
        }
        let Some(idx) = graphs.live_index(&target.graph_id) else {
            continue;
        };
        let seeds: Vec<String> = summary
            .changed_files
            .iter()
            .map(|c| c.path.clone())
            .collect();
        let callers = intra_service_caller_paths(&idx, &seeds, certain);
        for caller in callers {
            if !summary.changed_files.iter().any(|c| c.path == caller) {
                summary.changed_files.push(ChangedFileEntry {
                    is_test: graph_map::is_test_path(&caller),
                    path: caller,
                    status: "M".to_owned(),
                    from: None,
                    additions: None,
                    deletions: None,
                });
            }
        }
    }
}

fn normalize_graph_action(raw: &str) -> &str {
    match raw.trim() {
        "agents_on" => "offer",
        "agents_off" => "withhold",
        other => other,
    }
}

fn is_runtime_artifact_path(raw: &str) -> bool {
    let norm = raw.replace('\\', "/");
    let trimmed = norm.trim_start_matches("./");
    let check = |p: &str| {
        p.starts_with("project-brain/")
            || p.starts_with("memory-bank/")
            || p.starts_with(".agentic-security/")
    };
    if check(trimmed) {
        return true;
    }
    if let Some((_, rest)) = trimmed.split_once('/')
        && check(rest)
    {
        return true;
    }
    false
}

fn collect_opened_by_service_for_run(
    dispatches: &[DispatchEvidence],
    agent_filter: Option<&str>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut opened_by_service: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for ev in dispatches {
        if !agent_matches(agent_filter, &ev.phase, &ev.service) {
            continue;
        }
        let bucket = if ev.phase == "verify" {
            "verify".to_owned()
        } else {
            ev.service.clone()
        };
        for p in ev.opened.iter().chain(ev.shell_reads.keys()) {
            let norm = p.replace('\\', "/");
            let trimmed = norm.trim_start_matches("./");
            if ev.service == "__system__"
                && let Some((svc, rest)) = trimmed.split_once('/')
            {
                opened_by_service
                    .entry(svc.to_owned())
                    .or_default()
                    .insert(rest.to_owned());
            }
            opened_by_service
                .entry(bucket.clone())
                .or_default()
                .insert(trimmed.to_owned());
        }
    }
    opened_by_service
}

fn build_run_scoped_summaries(
    system: &System,
    dispatches: &[DispatchEvidence],
) -> BTreeMap<String, ServiceCodeSummary> {
    let mut summaries = collect_run_service_summaries(system, dispatches);
    for (sid, svc) in &system.services {
        if svc.access != "available" {
            continue;
        }
        let svc_prefix = format!("{sid}/");
        for ev in dispatches {
            if ev.service == *sid {
                continue;
            }
            for (p, eid) in &ev.shell_reads {
                let norm = p.replace('\\', "/");
                if let Some(rel) = norm.trim_start_matches("./").strip_prefix(&svc_prefix)
                    && !rel.is_empty()
                {
                    let entry = summaries.entry(sid.clone()).or_default();
                    entry.opened_files.insert(rel.to_owned());
                    entry.shell_reads.insert(rel.to_owned(), *eid);
                }
            }
            for ch in ev.git_changed.iter().chain(ev.ledger_edited.iter()) {
                let norm = ch.path.replace('\\', "/");
                if let Some(rel) = norm.trim_start_matches("./").strip_prefix(&svc_prefix)
                    && !rel.is_empty()
                    && !is_runtime_artifact_path(rel)
                {
                    let entry = summaries.entry(sid.clone()).or_default();
                    if !entry.changed_files.iter().any(|f| f.path == rel) {
                        entry.changed_files.push(ChangedFileEntry {
                            is_test: graph_map::is_test_path(rel),
                            path: rel.to_owned(),
                            status: ch.status.clone(),
                            from: ch.from.clone(),
                            additions: None,
                            deletions: None,
                        });
                    }
                }
            }
        }
    }
    summaries
}

fn normalize_cross_unopened_entry(raw: &Value) -> Value {
    let mut item = raw.clone();
    if item.is_object()
        && item.get("service").is_none()
        && let Some(consumer) = item["consumer"].as_str().map(str::to_owned)
    {
        item["service"] = json!(consumer);
    }
    item
}

fn normalize_cross_unopened_list(raw: &Value) -> Value {
    match raw.as_array() {
        Some(arr) => Value::Array(arr.iter().map(normalize_cross_unopened_entry).collect()),
        None => json!([]),
    }
}

#[cfg(test)]
#[path = "system_graph_tests.rs"]
mod tests;

#[cfg(test)]
mod semantic_bridge_tests {
    use super::*;
    use crate::graph::publish_for_tests;
    use tempfile::TempDir;

    fn make_service(root: &Path, id: &str, provides: Value, consumes: Value, sources: Value) {
        fs::create_dir_all(root).unwrap();
        let passport = json!({
            "schema_version": 1,
            "id": id,
            "description": format!("{id} service"),
            "owner": "platform",
            "relationships_complete": true,
            "capabilities": [],
            "provides": provides,
            "consumes": consumes,
            "sources": sources,
        });
        fs::write(
            root.join("ai-service.json"),
            serde_json::to_vec_pretty(&passport).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn bridge_view_discovers_cross_service_routes_events_di_and_table_couplings() {
        let tmp = TempDir::new().unwrap();
        let state_dir = tmp.path().join("state");
        let store = SessionStore::open(&state_dir).unwrap();
        let mono_root = tmp.path().join("mono");
        let web_root = mono_root.join("services/web");
        let orders_root = mono_root.join("services/orders");
        fs::create_dir_all(web_root.join("src")).unwrap();
        fs::create_dir_all(orders_root.join("src")).unwrap();

        // Consumer (`web`):
        // - Calls GET/fetch on `/api/v1/orders/42` (matches provider's `/api/v1/orders/{id}`)
        // - Handles `OrderPaid` event & publishes `checkout.requested`
        // - Injects `BillingToken` DI binding
        // - Reads `orders` table (cross_read_write), writes `payments` (shared_write), reads `currencies` (shared_read)
        fs::write(
            web_root.join("src/OrderClient.ts"),
            r#"
import { Inject } from '@nestjs/common';

export class OrderClient {
  constructor(@Inject('BillingToken') private readonly token: string) {}
  async fetchOrder() {
    return fetch('http://orders/api/v1/orders/42');
  }
  async startCheckout() {
    emit('checkout.requested', { id: 1 });
  }
}
"#,
        )
        .unwrap();
        fs::write(
            web_root.join("src/OrderMirrorWorker.php"),
            r#"<?php
class OrderMirrorWorker {
    #[AsMessageHandler]
    public function onPaid(OrderPaid $event): void {
        $rows = DB::table('orders')->where('id', 1)->get();
        DB::table('payments')->insert(['order_id' => 1]);
        $cur = "SELECT code FROM currencies WHERE active = 1";
    }
}
"#,
        )
        .unwrap();

        // Provider (`orders`):
        // - Exposes Laravel/Symfony `/api/v1/orders/{id}` and Express `/api/v1/checkout`
        // - Publishes `OrderPaid` and subscribes to `checkout.requested`
        // - Binds `BillingToken` via `provide: 'BillingToken'`
        // - Writes `orders` and `payments`, reads `currencies`
        fs::write(
            orders_root.join("src/OrderController.php"),
            r#"<?php
Route::get('/api/v1/orders/{id}', [OrderController::class, 'show']);

class OrderController {
    public function store() {
        $sql = "INSERT INTO orders (total) VALUES (100)";
        $pay = "UPDATE payments SET status = 'paid' WHERE id = 1";
        $cur = "SELECT code FROM currencies";
        event(new \App\Events\OrderPaid());
    }
}
"#,
        )
        .unwrap();
        fs::write(
            orders_root.join("src/CheckoutConsumer.ts"),
            r#"
export const billingProvider = { provide: 'BillingToken', useValue: 'x' };

@OnEvent('checkout.requested')
export class CheckoutConsumer {
  handle() {}
}
"#,
        )
        .unwrap();

        make_service(
            &orders_root,
            "orders",
            json!([{
                "id": "orders-http-v1",
                "kind": "http",
                "version": "1.0.0",
                "sources": ["src/OrderController.php"]
            }]),
            json!([]),
            json!([
                {"path": "src/OrderController.php", "kind": "contract"},
                {"path": "src/CheckoutConsumer.ts", "kind": "code"}
            ]),
        );
        make_service(
            &web_root,
            "web",
            json!([]),
            json!([{
                "service": "orders",
                "contract": "orders-http-v1",
                "version": "1.0.0"
            }]),
            json!([
                {"path": "src/OrderClient.ts", "kind": "code"},
                {"path": "src/OrderMirrorWorker.php", "kind": "code"}
            ]),
        );

        let sys_cfg = json!({
            "schema_version": 1,
            "name": "semantic-shop",
            "shared_sources": [],
            "services": [
                {"id": "web", "root": "services/web", "manifest": "ai-service.json"},
                {"id": "orders", "root": "services/orders", "manifest": "ai-service.json"}
            ]
        });
        fs::write(
            mono_root.join("system.json"),
            serde_json::to_vec_pretty(&sys_cfg).unwrap(),
        )
        .unwrap();

        let pid = store.register(&mono_root).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let fake_graphify = tmp.path().join("fake-graphify");
        fs::write(&fake_graphify, "#!/bin/sh\nexit 0\n").unwrap();
        let graphs = Graphs::with(
            store.clone(),
            Some(fake_graphify.clone()),
            std::time::Duration::from_secs(30),
        );
        let bridge = HarnessCodeGraphBridge::new(store.clone(), graphs.clone());
        let domain = SystemGraphDomain::new(store.clone(), graphs.clone(), bridge, None);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let payload = rt
            .block_on(domain.execute(
                "system_graph.bridge",
                json!({"query": {"project_id": pid, "from": "web", "to": "orders"}, "body": {}}),
            ))
            .unwrap();
        let Payload::Json(view) = payload else {
            panic!("expected json payload");
        };

        // Legacy fields preserved.
        assert_eq!(view["kind"], "system_bridge");
        assert_eq!(view["from"], "web");
        assert_eq!(view["to"], "orders");
        assert_eq!(view["contracts"], json!(["orders-http-v1"]));
        assert_eq!(
            view["provider"]["sites"][0]["path"],
            "src/OrderController.php"
        );
        assert!(view["consumer"].is_object());

        let sem = &view["semantic_bridges"];
        assert_eq!(sem["summary"]["routes"], 1, "{sem}");
        assert_eq!(sem["routes"][0]["route"], "/api/v1/orders/{id}");
        assert_eq!(sem["routes"][0]["method"], "GET");
        assert_eq!(sem["routes"][0]["direction"], "from->to");
        assert_eq!(
            sem["routes"][0]["caller_files"],
            json!(["src/OrderClient.ts"])
        );
        assert_eq!(
            sem["routes"][0]["provider_files"],
            json!(["src/OrderController.php"])
        );

        let events = sem["events"].as_array().unwrap();
        assert_eq!(events.len(), 2, "{sem}");
        assert!(events.iter().any(|e| {
            e["event"] == "checkout.requested"
                && e["direction"] == "from->to"
                && e["producer"] == "web"
                && e["consumer"] == "orders"
        }));
        assert!(events.iter().any(|e| {
            e["event"] == "OrderPaid"
                && e["direction"] == "to->from"
                && e["producer"] == "orders"
                && e["consumer"] == "web"
        }));

        let di = sem["di_bindings"].as_array().unwrap();
        assert_eq!(di.len(), 1, "{sem}");
        assert_eq!(di[0]["service"], "BillingToken");
        assert_eq!(di[0]["from_kinds"], json!(["injects"]));
        assert_eq!(di[0]["to_kinds"], json!(["binds"]));

        let tables = sem["tables"].as_array().unwrap();
        assert_eq!(tables.len(), 3, "{sem}");
        let by_table = |name: &str| {
            tables
                .iter()
                .find(|t| t["table"] == name)
                .unwrap_or_else(|| panic!("missing table {name}: {sem}"))
        };
        assert_eq!(by_table("orders")["coupling"], "cross_read_write");
        assert_eq!(by_table("orders")["from_access"], json!(["read"]));
        assert_eq!(by_table("orders")["to_access"], json!(["write"]));
        assert_eq!(by_table("payments")["coupling"], "shared_write");
        assert_eq!(by_table("currencies")["coupling"], "shared_read");

        // Also verify graph-level semantic edges integrate alongside source scans.
        let (_, orders_t) =
            resolve_service_target(&store, Some(&graphs), None, &pid, None, "orders").unwrap();
        publish_for_tests(&store, &orders_t.graph_id, &fake_graphify);
        let graph_file = store
            .state
            .join("graphs")
            .join(&orders_t.graph_id)
            .join("live")
            .join("graph.json");
        fs::create_dir_all(graph_file.parent().unwrap()).unwrap();
        let doc = json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id": "ctrl", "label": "OrderController", "source_file": "src/OrderController.php", "source_location": "L1"},
                {"id": "rt", "label": "route:POST /api/v1/checkout", "source_file": "src/OrderController.php", "source_location": "L2"}
            ],
            "links": [
                {"source": "ctrl", "target": "rt", "relation": "exposes_route", "confidence": "EXTRACTED"}
            ]
        });
        fs::write(&graph_file, serde_json::to_vec(&doc).unwrap()).unwrap();
        fs::write(
            web_root.join("src/CheckoutCaller.ts"),
            "export const go = () => axios.post('/api/v1/checkout');\n",
        )
        .unwrap();

        let payload2 = rt
            .block_on(domain.execute(
                "system_graph.bridge",
                json!({"query": {"project_id": pid, "from": "web", "to": "orders"}, "body": {}}),
            ))
            .unwrap();
        let Payload::Json(view2) = payload2 else {
            panic!("expected json payload");
        };
        let routes2 = view2["semantic_bridges"]["routes"].as_array().unwrap();
        assert!(
            routes2
                .iter()
                .any(|r| r["route"] == "/api/v1/checkout" && r["method"] == "POST"),
            "{routes2:?}"
        );
    }

    #[test]
    fn read_bounded_service_file_rejects_traversal_and_outside_symlinks() {
        let tmp = TempDir::new().unwrap();
        let svc = tmp.path().join("svc");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(svc.join("src")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(svc.join("src/ok.php"), "<?php echo 1;\n").unwrap();
        let secret = outside.join("secret.php");
        fs::write(&secret, "<?php // secret\n").unwrap();
        let canon = infra_core::fs::canonicalize(&svc).unwrap();

        assert!(read_bounded_service_file(&svc, Some(&canon), "../outside/secret.php").is_none());
        assert!(read_bounded_service_file(&svc, Some(&canon), "/etc/passwd").is_none());
        assert!(read_bounded_service_file(&svc, Some(&canon), "src/ok.php").is_some());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&secret, svc.join("src/escape.php")).unwrap();
            assert!(read_bounded_service_file(&svc, Some(&canon), "src/escape.php").is_none());
        }
    }

    #[test]
    fn collect_service_semantics_reads_ir_store_first_without_disk_scan() {
        let tmp = TempDir::new().unwrap();
        let state_dir = tmp.path().join("state");
        let svc_root = tmp.path().join("svc");
        fs::create_dir_all(svc_root.join("src")).unwrap();
        fs::write(
            svc_root.join("src/Client.ts"),
            "export function run() {\n  emit('checkout.requested', { id: 1 });\n  return fetch('/api/v1/orders/42');\n}\n",
        )
        .unwrap();

        let ir = crate::graph_ir::GraphIrStore::open(&state_dir, "svc-graph").unwrap();
        ir.sync_workspace(&svc_root).unwrap();
        // Replace on-disk file with an empty stub without re-syncing IR so only GraphIrStore has the signals.
        fs::write(svc_root.join("src/Client.ts"), "// stub on disk\n").unwrap();

        let target = ResolvedServiceTarget {
            project_id: "mono".to_owned(),
            config_path: "system.json".to_owned(),
            service_id: "web".to_owned(),
            graph_id: "svc-graph".to_owned(),
            root: svc_root,
            name: "web".to_owned(),
            reused_project: false,
        };

        let (signals, files_read, cut) = collect_service_semantics_with(
            Some(&state_dir),
            Some(&target),
            None,
            true,
            false,
            |_, _| {},
        );
        assert_eq!(files_read, 0);
        assert!(!cut);
        assert!(
            signals
                .called_routes
                .values()
                .any(|site| site.methods.contains("GET")),
            "expected GET route from IR: {:?}",
            signals.called_routes
        );
        assert!(
            signals.published_events.contains_key("checkout.requested"),
            "expected event from IR: {:?}",
            signals.published_events
        );
    }
}
