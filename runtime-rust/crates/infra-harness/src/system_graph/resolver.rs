use super::helpers::is_runtime_artifact_path;
use super::live_bridge::active_bridge_dispatches;
use crate::{
    HarnessError, Result,
    graph::{Graphs, RunChange, prepare, service_offer, shell_reads},
    graph_changes::{Change, Resolved, Target},
    store::{self, SessionStore},
};
use infra_host::provider::{Launch, PreparedCommand};
use infra_system::{
    DispatchEvidence, SystemManager, catalog::System, git_changes_since, load_run_evidence,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub(super) const SYSTEM_FOCUS_PREFIX: &str = "system_graph_focus.";
pub(super) const MAX_UNDECLARED_SCAN_FILES: usize = 64;
pub(super) const MAX_UNDECLARED_SCAN_BYTES: u64 = 256 * 1024;

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

pub(super) fn validate_config_rel(raw: Option<&str>) -> Result<String> {
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

pub(super) fn targets_from_loaded_system(
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

pub(super) fn store_synthetic_target_setting(
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

pub(super) fn agent_matches(filter: Option<&str>, phase: &str, service: &str) -> bool {
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

pub(super) fn resolve_launch_selector(
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
