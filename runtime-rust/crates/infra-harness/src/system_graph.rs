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
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, Weak},
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
}

pub struct HarnessCodeGraphBridge {
    store: SessionStore,
    graphs: Arc<Graphs>,
    state: Mutex<BridgeState>,
}

impl HarnessCodeGraphBridge {
    pub fn new(store: SessionStore, graphs: Arc<Graphs>) -> Arc<Self> {
        let me = Arc::new(Self {
            store,
            graphs,
            state: Mutex::new(BridgeState::default()),
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
            let mut impact_summaries = build_run_scoped_summaries(sys, &clean_dispatches);
            let edited_services: BTreeSet<String> = impact_summaries
                .iter()
                .filter(|(sid, s)| *sid != "__system__" && !s.changed_files.is_empty())
                .map(|(sid, _)| sid.clone())
                .collect();

            let mut unopened_callers = Vec::new();
            let target_sysrun = Target::SysRun(run_id.to_owned());
            for target in &targets {
                if target.service_id == "__system__"
                    || !edited_services.contains(&target.service_id)
                {
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

            augment_summaries_with_intra_callers(
                &self.graphs,
                &targets,
                &mut impact_summaries,
                false,
            );
            let opened_by_service = collect_opened_by_service_for_run(&all_dispatches, None);
            let sys_graph = SystemGraph::from_system(sys, impact_summaries);
            let mut merged_cross = Vec::new();
            let mut seen_cross = BTreeSet::new();
            if let Some(existing) = view["evidence"]["cross_service_unopened"].as_array() {
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
            for target in &targets {
                if target.service_id == "__system__"
                    || !edited_services.contains(&target.service_id)
                {
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

            if view["evidence"].is_object() {
                view["evidence"]["unopened_callers"] = json!(unopened_callers);
                view["evidence"]["cross_service_unopened"] = json!(merged_cross);
                view["cross_service_unopened"] = view["evidence"]["cross_service_unopened"].clone();
            }
        }
        Ok(())
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

    fn extract_run_id(query: &Value) -> Option<String> {
        Self::normalize_target_and_run_id(query["target"].as_str(), query["run_id"].as_str()).1
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
        let explicit_nonwork_target = if run_id.is_none() {
            match target_spec.filter(|t| !t.is_empty() && *t != "work" && *t != "uncommitted") {
                Some(tspec) => Some(Target::parse(tspec)?),
                None => None,
            }
        } else {
            None
        };
        let mut summaries = if let Some(rid) = run_id {
            let mut run_summaries = build_run_scoped_summaries(system, &dispatches);
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
            collect_git_service_summaries(system, &[])
        };

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
                        let mut probes = Vec::new();
                        if cid.len() >= 4 {
                            probes.push(cid.clone());
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
                        (sid.clone(), cid, ckind, probes)
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

            if let Some(ref parsed_target) = explicit_nonwork_target
                && let Ok(resolved) = graph_changes::resolve(&target.root, parsed_target, &[])
            {
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

            let live_idx = self.graphs.live_index(&target.graph_id);
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
                let candidate_files: Vec<String> = match &live_idx {
                    Some(idx) => idx
                        .files()
                        .iter()
                        .filter(|p| !graph_map::is_test_path(p))
                        .take(MAX_UNDECLARED_SCAN_FILES)
                        .cloned()
                        .collect(),
                    None => collect_service_code_files(&target.root, MAX_UNDECLARED_SCAN_FILES),
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
            qobj.remove("phase");
            qobj.remove("attempt");
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
                let (target_spec, run_id) = Self::normalize_target_and_run_id(
                    query["target"].as_str(),
                    query["run_id"].as_str(),
                );
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
                    if let Some(idx) = self.graphs.live_index(&svc_target.graph_id) {
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
                        let run_id = Self::extract_run_id(query);
                        let target_str = out["target"].as_str().unwrap_or("");
                        if run_id.is_some() || !target_str.is_empty() {
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
                            let mut summaries = if run_id.is_some() {
                                build_run_scoped_summaries(&system, &filtered_for_edits)
                            } else {
                                collect_git_service_summaries(&system, &filtered_for_edits)
                            };
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
                        let run_id = Self::extract_run_id(query);
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
                        let mut summaries = if run_id.is_some() {
                            build_run_scoped_summaries(&system, &dispatches)
                        } else {
                            collect_git_service_summaries(&system, &dispatches)
                        };
                        let query_path_for_sys = if let Some(ref p) = path_opt {
                            let clean_p = p.replace('\\', "/").trim_start_matches("./").to_owned();
                            let mut expanded = vec![clean_p.clone()];
                            if let Some(idx) = self.graphs.live_index(&target.graph_id) {
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
                    "php" | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java" | "kt" | "rb"
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
