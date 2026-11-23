use crate::{
    catalog::System,
    common::*,
    discovery, execution, native, runner,
    system_graph::{CodeGraphBridge, ServiceGraphOffer},
};
use infra_core::{Result, RuntimeError};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
/// Raw provider bytes can cross only this in-memory boundary; this type deliberately
/// has no serialization implementation and is never a public stored event.
pub enum AgentSignal {
    Configured {
        provider: String,
        roots: BTreeMap<String, PathBuf>,
        aliases: BTreeMap<PathBuf, (String, String)>,
    },
    Started {
        agent: String,
        attempt: u64,
        cwd: PathBuf,
        details: Value,
    },
    Output {
        bytes: Vec<u8>,
    },
    Finished {
        status: String,
        details: Value,
    },
}
fn signal(agent: &mut impl FnMut(AgentSignal) -> Result<()>, value: AgentSignal) {
    let _ = agent(value);
}
pub fn request_location(path: &Path, parent: &str) -> Result<(PathBuf, String)> {
    let path = absolute(path)?;
    let directory = path
        .parent()
        .ok_or_else(|| fail("Request has no parent"))?
        .to_path_buf();
    let run = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| fail("Request filename is not UTF-8"))?;
    let nonce = name
        .strip_prefix("request-")
        .and_then(|name| name.strip_suffix(".json"))
        .filter(|value| hex(value, 32))
        .ok_or_else(|| fail("Invalid request nonce"))?;
    if !hex(run, 32)
        || directory
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some(parent)
    {
        return Err(fail("Invalid request location"));
    }
    infra_core::bank::publication::PublicationFiles::new(&directory)?;
    Ok((directory, nonce.into()))
}
fn display(event: &mut impl FnMut(Value) -> Result<()>, value: Value) {
    let _ = event(value);
}
/// Selects the [`ServiceGraphOffer`]s permitted for a dispatch step.
///
/// - `contracts` and `verify` receive offers for `__system__` and all `selected` services.
/// - `service-<sid>` receives its own service offer, or all `selected` offers when `access == "all"`.
/// - Returns empty when `tell_agents_graph == Some(false)`.
pub fn offers_for_step(
    state: &Value,
    step: &Value,
    offers: &[ServiceGraphOffer],
) -> Vec<ServiceGraphOffer> {
    if state.get("tell_agents_graph") == Some(&Value::Bool(false)) {
        return vec![];
    }
    let selected: BTreeSet<String> = texts(&state["selected"]).into_iter().collect();
    let phase = string(&step["id"]);
    let step_service = string(&step["service"]);
    let all_scope =
        phase == "contracts" || phase == "verify" || execution::access_scope(state) == "all";
    offers
        .iter()
        .filter(|offer| {
            if all_scope {
                offer.service == "__system__" || selected.contains(&offer.service)
            } else {
                offer.service == step_service
            }
        })
        .cloned()
        .collect()
}

fn collect_offer_readable_paths(offers: &[ServiceGraphOffer]) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for offer in offers {
        if !offer.graph_path.as_os_str().is_empty() && seen.insert(offer.graph_path.clone()) {
            out.push(offer.graph_path.clone());
        }
        for path in &offer.readable_paths {
            if !path.as_os_str().is_empty() && seen.insert(path.clone()) {
                out.push(path.clone());
            }
        }
    }
    out
}

/// Builds Linux `bwrap` read-only bind arguments (`--ro-bind <path> <path>`) for offered code graphs.
pub fn bwrap_ro_bind_args(offers: &[ServiceGraphOffer]) -> Result<Vec<String>> {
    let mut args = Vec::new();
    for path in collect_offer_readable_paths(offers) {
        let p = path_string(&path)?;
        args.extend(["--ro-bind".into(), p.clone(), p]);
    }
    Ok(args)
}

/// Builds macOS `sandbox-exec` read-only Seatbelt rules (`(allow file-read* ...)`) for offered code graphs.
pub fn macos_seatbelt_read_rules(offers: &[ServiceGraphOffer]) -> Result<String> {
    let mut profile = String::new();
    for path in collect_offer_readable_paths(offers) {
        let escaped = path_string(&path)?
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let matcher = if path.extension().is_some() || path.is_file() {
            "literal"
        } else {
            "subpath"
        };
        profile.push_str(&format!("(allow file-read* ({matcher} \"{escaped}\"))\n"));
    }
    Ok(profile)
}

/// Applies [`ServiceGraphOffer`] environment variables, read-only sandbox grants, Codex `--add-dir`,
/// and Claude `--settings` `permissions.allow` rules to a worker [`infra_host::provider::PreparedCommand`].
pub fn prepare_worker(
    provider: &str,
    mode: &str,
    command: infra_host::provider::PreparedCommand,
    offers: &[ServiceGraphOffer],
) -> Result<infra_host::provider::PreparedCommand> {
    prepare_worker_for_transport(
        provider,
        mode,
        infra_host::provider::Transport::Text,
        command,
        offers,
    )
}

/// Applies [`ServiceGraphOffer`] configuration appropriate for the active provider [`infra_host::provider::Transport`].
pub fn prepare_worker_for_transport(
    provider: &str,
    _mode: &str,
    transport: infra_host::provider::Transport,
    mut command: infra_host::provider::PreparedCommand,
    offers: &[ServiceGraphOffer],
) -> Result<infra_host::provider::PreparedCommand> {
    if offers.is_empty() {
        return Ok(command);
    }
    command
        .remove
        .retain(|key| key != "GRAPHIFY_NO_AUTO_REFRESH");
    command
        .environment
        .insert("GRAPHIFY_NO_AUTO_REFRESH".into(), "1".into());
    for offer in offers {
        for (key, value) in &offer.env {
            command.remove.retain(|removed| removed != key);
            command.environment.insert(key.clone(), value.clone());
        }
    }

    let readable_paths = collect_offer_readable_paths(offers);
    for path in &readable_paths {
        if !command.readable.contains(path) {
            command.readable.push(path.clone());
        }
    }

    let readable_dirs = {
        let mut dirs = Vec::new();
        let mut seen = BTreeSet::new();
        for path in &readable_paths {
            let dir = if path.is_dir() {
                path.clone()
            } else if path.extension().is_some() || path.is_file() {
                path.parent()
                    .map_or_else(|| path.clone(), Path::to_path_buf)
            } else {
                path.clone()
            };
            if seen.insert(dir.clone()) {
                dirs.push(dir);
            }
        }
        dirs
    };

    if provider == "codex" && transport == infra_host::provider::Transport::Text {
        let mut existing_dirs = BTreeSet::new();
        for pair in command.argv.windows(2) {
            if (pair[0] == "--add-dir" || pair[0] == "-C") && !pair[1].is_empty() {
                existing_dirs.insert(pair[1].clone());
            }
        }
        let mut additions = Vec::new();
        // Codex `--add-dir` grants write access in workspace-write mode, so
        // it names only each published graph's own directory; program and
        // virtualenv paths stay read-only sandbox grants.
        let mut graph_dirs = Vec::new();
        for offer in offers {
            if let Some(dir) = offer
                .graph_path
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                && !graph_dirs.contains(&dir.to_path_buf())
            {
                graph_dirs.push(dir.to_path_buf());
            }
        }
        for dir in &graph_dirs {
            let dir_str = path_string(dir)?;
            if existing_dirs.insert(dir_str.clone()) {
                additions.extend(["--add-dir".into(), dir_str]);
            }
        }
        if !additions.is_empty() {
            let insert_at = if command.argv.last().map(String::as_str) == Some("-") {
                command.argv.len() - 1
            } else {
                command.argv.len()
            };
            command.argv.splice(insert_at..insert_at, additions);
        }
    } else if provider == "claude" {
        let mut rules = Vec::new();
        let mut seen_rules = BTreeSet::new();
        for dir in &readable_dirs {
            let read_rule = native::claude_read_rule(dir)?;
            if seen_rules.insert(read_rule.clone()) {
                rules.push(read_rule);
            }
        }
        for offer in offers {
            for rule in &offer.claude_allow_rules {
                if !rule.trim().is_empty() && seen_rules.insert(rule.clone()) {
                    rules.push(rule.clone());
                }
            }
        }
        if !rules.is_empty() {
            merge_claude_settings_allow(&mut command.argv, &rules)?;
        }
    }

    Ok(command)
}

fn merge_claude_settings_allow(argv: &mut Vec<String>, rules: &[String]) -> Result<()> {
    let existing_idx = argv
        .iter()
        .enumerate()
        .find_map(|(idx, arg)| (arg == "--settings" && idx + 1 < argv.len()).then_some(idx + 1));

    let mut settings = if let Some(val_idx) = existing_idx {
        let raw = &argv[val_idx];
        parse_json(raw.as_bytes(), MAX_BYTES).unwrap_or_else(|_| object!({}))
    } else {
        object!({})
    };
    if !settings.is_object() {
        settings = object!({});
    }
    if !settings["permissions"].is_object() {
        settings["permissions"] = object!({});
    }
    let mut allow_list = settings["permissions"]["allow"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for rule in rules {
        let val = Value::String(rule.clone());
        if !allow_list.contains(&val) {
            allow_list.push(val);
        }
    }
    settings["permissions"]["allow"] = Value::Array(allow_list);
    let encoded_settings = encoded(&settings)?;
    if let Some(val_idx) = existing_idx {
        argv[val_idx] = encoded_settings;
    } else {
        argv.extend(["--settings".into(), encoded_settings]);
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Internal request runner threads custody, observer, approvals, and optional graph bridge."
)]
async fn run_system_context(
    path: &Path,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<File>>,
    adapter: Option<Arc<dyn infra_host::provider::LaunchAdapter>>,
    observer: Option<Arc<dyn infra_host::process::RunObserver>>,
    bridge: Option<Arc<dyn CodeGraphBridge>>,
    launch_id: Option<String>,
    approvals: Option<Arc<dyn infra_host::permission::Approvals>>,
    mut event: impl FnMut(Value) -> Result<()> + Send,
    mut agent: impl FnMut(AgentSignal) -> Result<()> + Send,
) -> Result<Value> {
    let (directory, nonce) = request_location(path, "ai-system")?;
    let system_run_id = directory
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string);
    let request = read_json(
        &directory,
        &format!("request-{nonce}.json"),
        execution::MAX_STATE,
    )?;
    fields(
        &request,
        &[
            "directory",
            "system_file",
            "allowed_roots",
            "provider",
            "executable",
            "mode",
            "timeout",
            "access",
            "retry_step",
            "accept_source_changes",
        ],
        &[
            "accept_unrun_checks",
            "tell_agents_graph",
            "launch_id",
            "permissions",
        ],
    )?;
    if request
        .get("tell_agents_graph")
        .is_some_and(|v| !v.is_null() && !v.is_boolean())
    {
        return Err(fail("tell_agents_graph must be boolean"));
    }
    let request_permissions = match request.get("permissions") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(raw)) => {
            let mode = infra_host::permission::PermissionMode::parse(raw)
                .ok_or_else(|| fail("Choose Auto, Manual or Bypass permissions."))?;
            Some(Some(mode.as_str().to_string()))
        }
        Some(_) => return Err(fail("Choose Auto, Manual or Bypass permissions.")),
    };
    let resolved_launch_id = launch_id.or_else(|| {
        request
            .get("launch_id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    });
    if absolute(Path::new(string(&request["directory"])))? != directory {
        return Err(fail("System request directory changed"));
    }
    let provider = choice(
        &request["provider"],
        &["codex", "claude", "cursor"],
        "system worker provider",
    )?;
    let allow = texts(&request["allowed_roots"])
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let mut system = System::new(Path::new(string(&request["system_file"])), &allow)?;
    let journal = directory.join("execution");
    let state = if journal.join("run.json").exists() {
        let mut state = native::validate_state_sync(&mut system, &journal, true)?;
        let selected = native::executable(&provider, Some(string(&request["executable"]))).await?;
        if state["provider"] != provider || Path::new(string(&state["executable"])) != selected {
            return Err(fail("Configured provider changed; prepare a new plan"));
        }
        let mut updated = false;
        if let Some(tell) = request.get("tell_agents_graph").and_then(Value::as_bool)
            && state.get("tell_agents_graph").and_then(Value::as_bool) != Some(tell)
        {
            state["tell_agents_graph"] = object!(tell);
            updated = true;
        }
        if let Some(requested_perm) = &request_permissions {
            let next = match requested_perm {
                Some(p) => Value::String(p.clone()),
                None => Value::Null,
            };
            let current = state.get("permissions").cloned().unwrap_or(Value::Null);
            if current != next {
                if next.is_null() {
                    if let Some(obj) = state.as_object_mut() {
                        obj.remove("permissions");
                    }
                } else {
                    state["permissions"] = next;
                }
                updated = true;
            }
        }
        if updated {
            save(&journal, "run.json", &state, false)?;
        }
        state
    } else {
        let plan = read_json(&directory, "approved-plan.json", execution::MAX_STATE)?;
        native::create_run_context_with_permissions(
            &mut system,
            &plan,
            &journal,
            &provider,
            Some(string(&request["executable"])),
            string(&request["mode"]),
            crate::manager::integer(request.get("timeout"), 900, 1, 86400)?,
            string(&request["access"]),
            request.get("tell_agents_graph").and_then(Value::as_bool),
            request_permissions.flatten().as_deref(),
            adapter.as_deref(),
        )
        .await?
    };
    signal(
        &mut agent,
        AgentSignal::Configured {
            provider: provider.clone(),
            roots: state["roots"]
                .as_object()
                .ok_or_else(|| fail("System dispatch roots missing"))?
                .iter()
                .map(|(sid, path)| {
                    Ok((
                        sid.clone(),
                        PathBuf::from(
                            path.as_str()
                                .ok_or_else(|| fail("Invalid system dispatch root"))?,
                        ),
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
            aliases: BTreeMap::new(),
        },
    );
    display(
        &mut event,
        object!({"kind":"status","text":"Sequential system dispatch started. Progress and receipts are saved after each dispatch."}),
    );
    let hooks = runner::RunnerGraphHooks {
        bridge,
        system_run_id,
        launch_id: resolved_launch_id,
        approvals,
    };
    let result=Box::pin(runner::drive_run_context_with_hooks(&mut system,&journal,request["retry_step"].as_str(),runner::Acceptance{source_changes:request["accept_source_changes"]==true,unrun_checks:request["accept_unrun_checks"]==true},cancel.clone(),runner_lock,adapter.clone(),observer.clone(),None,hooks,&mut|value:Value|{
 let kind=string(&value["type"]);match kind{
 "output"=>{if let Some(line)=value["line"].as_str(){signal(&mut agent,AgentSignal::Output{bytes:line.as_bytes().to_vec()});}},
 "dispatch_started"=>signal(&mut agent,AgentSignal::Started{agent:string(&value["phase"]).into(),attempt:value["attempt"].as_u64().unwrap_or(0),cwd:PathBuf::from(string(&value["root"])),details:object!({"service":value["service"],"label":match string(&value["phase"]){"contracts"=>"Contract agent".to_string(),"verify"=>"Verification agent".to_string(),_=>format!("{} agent",string(&value["service"]))},"mode":value["mode"],"provider":provider,"root":value["root"],"dispatch":value["dispatch_id"],"readable":value["readable"],"writable":value["writable"]})}),
 "dispatch_finished"=>{let report=&value["report"];let checks=report["checks"].as_array().cloned().unwrap_or_default();signal(&mut agent,AgentSignal::Finished{status:if value["interrupted"]==true{"interrupted"}else if value["ok"]==true{"completed"}else{"blocked"}.into(),details:object!({"ok":value["ok"],"error":value["error"],"duration_seconds":value["duration_seconds"],"summary":report["summary"],"changed_files":report["changed_files"].as_array().into_iter().flatten().take(100).cloned().collect::<Vec<_>>(),"checks":(["passed","failed","not_run"].iter().map(|status|((*status).to_string(),checks.iter().filter(|check|check["status"]==*status).count())).collect::<BTreeMap<_,_>>())})});},
 "permission"|"status" if value.get("event").is_some_and(Value::is_object)=>{display(&mut event,value["event"].clone());},
 _=>{}}Ok(())})).await;
    match result {
        Ok(state) => {
            let ok = state["status"] == "completed";
            display(
                &mut event,
                object!({"kind":"text","text":state["error"].as_str().unwrap_or("Worker-reported checks completed; native tasks closed and handoff saved.")}),
            );
            display(
                &mut event,
                object!({"kind":"result","ok":ok,"text":format!("System run {}",string(&state["status"]))}),
            );
            // A saved but unfinished run (blocked, interrupted) is an incomplete
            // launch: its Harness session reads failed, not completed.
            let mut summary = execution::run_summary(&state, &journal);
            summary["outcome"] = if ok { "completed" } else { "incomplete" }.into();
            Ok(summary)
        }
        Err(error) => {
            display(
                &mut event,
                object!({"kind":"error","text":error.to_string()}),
            );
            display(
                &mut event,
                object!({"kind":"result","ok":false,"text":"System run blocked. Inspect the plan and execution journal."}),
            );
            Err(error)
        }
    }
}
/// Names why the in-sandbox boundary check refused the scan. The probe writes its
/// refusal as one JSON line on stdout; no line at all means the sandbox itself could
/// not start the probe.
pub fn probe_failure(stdout: &[u8], code: i32) -> String {
    let reason = stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter(|value| value["ok"] == false)
        .filter_map(|value| value["error"].as_str().map(str::to_string))
        .next_back();
    match reason {
        Some(reason) if reason.starts_with("The sandbox could read an original") => format!(
            "The AI discovery sandbox did not keep the original folders unreadable. {reason}"
        ),
        Some(reason) => format!("The AI discovery sandbox check failed: {reason}"),
        None => format!(
            "The AI discovery sandbox could not start its boundary check (exit code {code}). On Linux, bubblewrap must be able to create user namespaces."
        ),
    }
}
/// The sandbox of an isolated agent: its CLI and the runtimes it needs
/// readable, its account state writable, `original` denied, the workspace
/// writable and the workspace's `evidence/` read-only.
pub fn runtime_profile(
    executable: &Path,
    original: &[PathBuf],
    workspace: &Path,
    codex: Option<PathBuf>,
    readable: &[PathBuf],
) -> Result<infra_host::sandbox::Spec> {
    let mut runtime = BTreeSet::new();
    runtime.insert(infra_host::sandbox::trusted_helper(workspace).map_err(host)?);
    runtime.insert(executable.to_path_buf());
    if let Some(parent) = executable.parent() {
        runtime.insert(parent.to_path_buf());
    }
    for parent in executable.ancestors().skip(1).take(6) {
        if let Ok(mut custody) = infra_core::bank::publication::PublicationFiles::new(parent)
            && let Ok(Some(raw)) = custody.read("package.json", 64 * 1024)
            && let Ok(package) = parse_json(&raw, 64 * 1024)
            && [
                "@openai/codex",
                "@anthropic-ai/claude-code",
                "@cursor/agent",
                "cursor-agent",
            ]
            .contains(&string(&package["name"]))
        {
            runtime.insert(parent.to_path_buf());
            break;
        }
    }
    for name in ["node", "bun"] {
        if let Some(path) = infra_host::provider::find_executable(name) {
            let path = infra_core::fs::canonicalize(path)?;
            runtime.insert(path.clone());
            if let Some(parent) = path.parent() {
                runtime.insert(parent.to_path_buf());
            }
        }
    }
    if let Some(codex) = &codex {
        runtime.insert(codex.clone());
        if let Some(parent) = codex.parent() {
            runtime.insert(parent.to_path_buf());
        }
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or_else(|| fail("Provider account home unavailable"))?;
    let mut accounts = vec![];
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    if codex_home.exists() {
        accounts.push(codex_home);
    }
    for name in [".claude", ".claude.json", ".config/cursor-agent", ".cursor"] {
        let path = home.join(name);
        if path.exists() {
            accounts.push(path);
        }
    }
    // Never let an account or executable read grant expose an original source root.
    for grant in accounts.iter().chain(runtime.iter()).chain(readable.iter()) {
        for root in original {
            if grant.starts_with(root) || root.starts_with(grant) {
                return Err(fail(
                    "Discovery source overlaps provider account or runtime; use standalone CLI outside source folders",
                ));
            }
        }
    }
    Ok(infra_host::sandbox::Spec {
        workspace: workspace.to_path_buf(),
        target: workspace.to_path_buf(),
        protected: original.to_vec(),
        readable: readable.to_vec(),
        accounts,
        runtime: runtime.into_iter().collect(),
        codex,
        evidence: Some(workspace.join("evidence")),
        network: true,
    })
}
/// Hashes the normalized sandbox profile and discovery launch identity so any boundary,
/// account, runtime, protected root, or selected service change triggers a fail-closed full rescan.
pub(crate) fn sandbox_boundary_sha256(
    spec: &infra_host::sandbox::Spec,
    provider: &str,
    executable: &Path,
    ids: &[String],
) -> Result<String> {
    let canonical = infra_core::domain::json::canonical(&object!({
        "provider": provider,
        "executable": executable,
        "workspace": spec.workspace,
        "protected": spec.protected,
        "readable": spec.readable,
        "accounts": spec.accounts,
        "runtime": spec.runtime,
        "codex": spec.codex,
        "evidence": spec.evidence,
        "network": spec.network,
        "ids": ids,
    }))?;
    Ok(digest(canonical.as_bytes()))
}

pub(crate) fn build_cached_discovery_report(
    request: &Value,
    ids: &[String],
    plan: &discovery::IncrementalDiscoveryPlan,
) -> Result<Value> {
    if plan.fallback_reason.is_some()
        || !plan.pending_ids.is_empty()
        || plan.reused_reports.len() != ids.len()
    {
        return Err(fail(
            "Incremental discovery plan is not a complete cache hit",
        ));
    }
    let mut services = Vec::with_capacity(ids.len());
    for sid in ids {
        let report = plan
            .reused_reports
            .get(sid)
            .ok_or_else(|| fail("Missing cached passport report for selected service"))?;
        services.push(report.clone());
    }
    Ok(object!({
        "name": plan
            .cached_system_name
            .clone()
            .unwrap_or_else(|| string(&request["editor"]["name"]).to_string()),
        "shared_sources": plan
            .cached_shared_sources
            .clone()
            .unwrap_or_else(|| object!([])),
        "services": services,
        "warnings": plan.cached_warnings.clone(),
    }))
}

pub(crate) fn verify_and_commit_discovery_report(
    directory: &Path,
    ledger_dir: &Path,
    request: &Value,
    report: &Value,
    manifest_index: &discovery::ManifestIndex,
    provider: &str,
    boundary_sha: &str,
) -> Result<Value> {
    discovery::gate1_verify_discovery_sources(report, request, manifest_index)?;
    let proposal = discovery::validate_proposal(report, request)?;
    discovery::check_fresh(request)?;
    discovery::save_verified_passports(
        ledger_dir,
        request,
        report,
        manifest_index,
        provider,
        boundary_sha,
    )?;
    create_new(directory, "proposal.json", report)?;
    Ok(proposal)
}

fn prepare_discovery_argv(
    provider: &str,
    mut argv: Vec<String>,
    codex: &Option<PathBuf>,
    spec: &infra_host::sandbox::Spec,
) -> Result<Vec<String>> {
    let _ = (&provider, codex, &mut argv);
    #[cfg(windows)]
    {
        if codex.is_none() {
            return Err(fail(
                "Windows discovery requires available Codex elevated sandbox",
            ));
        }
        if provider == "codex" {
            let position = argv
                .iter()
                .position(|value| value == "--sandbox")
                .ok_or_else(|| fail("Codex sandbox option missing"))?;
            argv.splice(
                position..position + 2,
                [
                    "-c".into(),
                    "default_permissions=\":danger-full-access\"".into(),
                ],
            );
        }
    }
    infra_host::sandbox::discovery(spec, &argv).map_err(host)
}

async fn run_discovery_context(
    path: &Path,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<File>>,
    adapter: Option<Arc<dyn infra_host::provider::LaunchAdapter>>,
    observer: Option<Arc<dyn infra_host::process::RunObserver>>,
    mut event: impl FnMut(Value) -> Result<()> + Send,
    mut agent: impl FnMut(AgentSignal) -> Result<()> + Send,
) -> Result<Value> {
    let (directory, nonce) = request_location(path, "system-discovery")?;
    let request = read_json(&directory, &format!("request-{nonce}.json"), 8 * MAX_BYTES)?;
    if request["nonce"] != nonce {
        return Err(fail("Invalid discovery nonce"));
    }
    discovery::check_fresh(&request)?;
    let workspace = directory.join("agent");
    let provider = choice(
        &request["provider"],
        &["codex", "claude", "cursor"],
        "discovery provider",
    )?;
    let executable = native::executable(&provider, Some(string(&request["executable"]))).await?;
    let ids = items(&request["editor"]["services"], 50)?
        .iter()
        .map(|service| identifier(string(&service["passport"]["id"])))
        .collect::<Result<Vec<_>>>()?;
    let mut aliases = BTreeMap::new();
    for (sid, records) in request["inventory"]
        .as_object()
        .ok_or_else(|| fail("Discovery inventory missing"))?
    {
        for (name, record) in records
            .as_object()
            .ok_or_else(|| fail("Discovery source inventory invalid"))?
        {
            let copy = relative(string(&record["copy"]))?;
            if !copy.starts_with("evidence/") {
                return Err(fail("Discovery copy outside evidence"));
            }
            aliases.insert(workspace.join(copy), (sid.clone(), name.clone()));
        }
    }
    signal(
        &mut agent,
        AgentSignal::Configured {
            provider: provider.clone(),
            roots: BTreeMap::from([("scan".to_string(), workspace.clone())]),
            aliases,
        },
    );
    let manifest_index = discovery::ManifestIndex::from_request(&request)?;
    let shared_manifest_sha = discovery::shared_manifest_digest(&request, &manifest_index)?;
    let schema = discovery::result_schema(&ids);
    save(&directory, "schema.json", &schema, false)?;
    save(
        &workspace.join("evidence"),
        "inventory.json",
        &request["inventory"],
        false,
    )?;
    save(
        &workspace.join("evidence"),
        "draft.json",
        &request["editor"],
        false,
    )?;
    let prompt = discovery::build_discovery_prompt_with_prefix_memo(
        &request,
        &ids,
        &BTreeMap::new(),
        &shared_manifest_sha,
    )?;
    let prepared_full = native::prepare_launch(
        &provider,
        &executable,
        &workspace,
        &prompt,
        &schema,
        &directory.join("schema.json"),
        "read-only",
        &[],
        &[],
        adapter.as_deref(),
    )?;
    let original = request["roots"]
        .as_object()
        .ok_or_else(|| fail("Missing discovery roots"))?
        .values()
        .map(|record| absolute(Path::new(string(&record["path"]))))
        .collect::<Result<Vec<_>>>()?;
    let codex = request["sandbox_executable"].as_str().map(PathBuf::from);
    let spec = runtime_profile(
        &executable,
        &original,
        &workspace,
        codex.clone(),
        &prepared_full.readable,
    )?;
    let boundary_sha = sandbox_boundary_sha256(&spec, &provider, &executable, &ids)?;
    // Every original path and an outside sentinel must be denied in the same native sandbox
    // before provider code can inspect the immutable numbered evidence copies.
    display(
        &mut event,
        object!({"kind":"status","text":"Checking that the sandbox keeps original service folders unreadable."}),
    );
    let boundary = infra_host::sandbox::boundary_request(&spec).map_err(host)?;
    let probe = native::run_raw_monitored(
        &boundary.command,
        &workspace,
        &prepared_full.environment,
        &prepared_full.remove,
        &[],
        30,
        cancel.clone(),
        runner_lock.clone(),
        observer.clone(),
        |_| Ok(()),
    )
    .await?;
    if probe.code != 0 {
        return Err(fail(probe_failure(&probe.bytes, probe.code)));
    }
    let attestation = parse_json(&probe.bytes, MAX_BYTES)?;
    fields(
        &attestation,
        &[
            "ok",
            "denied",
            "allowed",
            "evidence_read_only",
            "workspace_writable",
        ],
        &[],
    )?;
    if attestation["ok"] != true
        || attestation["denied"].as_u64() != Some(boundary.denied as u64)
        || attestation["allowed"].as_u64() != Some(boundary.allowed as u64)
        || attestation["evidence_read_only"] != true
        || attestation["workspace_writable"] != true
    {
        return Err(fail(
            "Discovery sandbox did not attest every boundary condition",
        ));
    }
    drop(boundary);

    let ledger_dir = directory.parent().unwrap_or(&directory);
    let bypass_cache = request.get("no_cache") == Some(&Value::Bool(true))
        || request.get("strict") == Some(&Value::Bool(true));
    let plan = if bypass_cache {
        discovery::IncrementalDiscoveryPlan {
            reused_reports: BTreeMap::new(),
            pending_ids: ids.clone(),
            fallback_reason: Some("cache-bypassed".into()),
            cached_system_name: None,
            cached_shared_sources: None,
            cached_warnings: Vec::new(),
        }
    } else {
        discovery::plan_incremental_discovery(
            ledger_dir,
            &request,
            &ids,
            &manifest_index,
            &provider,
            &boundary_sha,
        )?
    };

    let started = std::time::Instant::now();
    let timeout = crate::manager::integer(request.get("timeout"), 900, 1, 86400)?;

    // Fast path: every selected service matched verified prior passports under unchanged manifests,
    // provider, and sandbox boundary. Still re-run Gate-1 + proposal + freshness before committing.
    if plan.fallback_reason.is_none()
        && plan.pending_ids.is_empty()
        && plan.reused_reports.len() == ids.len()
        && let Ok(cached_report) = build_cached_discovery_report(&request, &ids, &plan)
        && discovery::gate1_verify_discovery_sources(&cached_report, &request, &manifest_index)
            .is_ok()
        && let Ok(proposal) = discovery::validate_proposal(&cached_report, &request)
    {
        discovery::check_fresh(&request)?;
        discovery::save_verified_passports(
            ledger_dir,
            &request,
            &cached_report,
            &manifest_index,
            &provider,
            &boundary_sha,
        )?;
        create_new(&directory, "proposal.json", &cached_report)?;
        signal(
            &mut agent,
            AgentSignal::Finished {
                status: "completed".into(),
                details: object!({"ok":true,"cached":true,"duration_seconds":started.elapsed().as_secs_f64(),"summary":format!("{} verified service passports reused",ids.len())}),
            },
        );
        display(
            &mut event,
            object!({"kind":"text","text":"Service metadata proposal verified from unchanged service passports. No source metadata was written."}),
        );
        display(&mut event, object!({"kind":"result","ok":true}));
        return Ok(proposal);
    }

    // Incremental subset path when at least one unchanged service passport can be reused and at
    // least one changed service remains pending; falls back fail-closed to full discovery on drift.
    let mut attempt = 0u64;
    if plan.fallback_reason.is_none()
        && !plan.reused_reports.is_empty()
        && !plan.pending_ids.is_empty()
        && plan.pending_ids.len() < ids.len()
    {
        let subset_schema = discovery::result_schema_for_subset(&ids, &plan.pending_ids);
        let subset_prompt = discovery::build_discovery_prompt_with_prefix_memo(
            &request,
            &plan.pending_ids,
            &plan.reused_reports,
            &shared_manifest_sha,
        )?;
        save(&directory, "schema.json", &subset_schema, true)?;
        let subset_prepared = native::prepare_launch(
            &provider,
            &executable,
            &workspace,
            &subset_prompt,
            &subset_schema,
            &directory.join("schema.json"),
            "read-only",
            &[],
            &[],
            adapter.as_deref(),
        )?;
        if subset_prepared.readable == prepared_full.readable {
            let subset_argv =
                prepare_discovery_argv(&provider, subset_prepared.argv, &codex, &spec)?;
            attempt += 1;
            signal(
                &mut agent,
                AgentSignal::Started {
                    agent: "discovery".into(),
                    attempt,
                    cwd: workspace.clone(),
                    details: object!({"label":"Discovery agent","service":"__system__","mode":"read-only","provider":provider,"root":workspace,"readable":plan.pending_ids,"reused":plan.reused_reports.keys().collect::<Vec<_>>(),"writable":[]}),
                },
            );
            display(
                &mut event,
                object!({"kind":"status","text":format!("AI is inspecting {} changed services ({} unchanged reused).",plan.pending_ids.len(),plan.reused_reports.len())}),
            );
            let output = native::run_raw_monitored(
                &subset_argv,
                &workspace,
                &subset_prepared.environment,
                &subset_prepared.remove,
                &subset_prepared.input,
                timeout,
                cancel.clone(),
                runner_lock.clone(),
                observer.clone(),
                |line| {
                    signal(
                        &mut agent,
                        AgentSignal::Output {
                            bytes: line.to_vec(),
                        },
                    );
                    Ok(())
                },
            )
            .await;
            if cancel.load(Ordering::SeqCst) {
                let err = RuntimeError::unavailable("Discovery interrupted");
                signal(
                    &mut agent,
                    AgentSignal::Finished {
                        status: "interrupted".into(),
                        details: object!({"ok":false,"error":err.to_string(),"duration_seconds":started.elapsed().as_secs_f64()}),
                    },
                );
                display(
                    &mut event,
                    object!({"kind":"error","text":format!("AI discovery failed: {err}")}),
                );
                display(&mut event, object!({"kind":"result","ok":false}));
                return Err(err);
            }
            if let Ok(output) = output
                && output.code == 0
                && let Ok(partial_report) = native::worker_result(&output.bytes, &provider)
                && let Ok(merged) =
                    discovery::merge_discovery_reports(&ids, &plan.reused_reports, &partial_report)
                && let Ok(proposal) = verify_and_commit_discovery_report(
                    &directory,
                    ledger_dir,
                    &request,
                    &merged,
                    &manifest_index,
                    &provider,
                    &boundary_sha,
                )
            {
                signal(
                    &mut agent,
                    AgentSignal::Finished {
                        status: "completed".into(),
                        details: object!({"ok":true,"incremental":true,"reused":plan.reused_reports.len(),"discovered":plan.pending_ids.len(),"duration_seconds":started.elapsed().as_secs_f64(),"summary":format!("{} service passports proposed for review",ids.len())}),
                    },
                );
                display(
                    &mut event,
                    object!({"kind":"text","text":"Service metadata proposal is ready for review. No source metadata was written."}),
                );
                display(&mut event, object!({"kind":"result","ok":true}));
                return Ok(proposal);
            }
            display(
                &mut event,
                object!({"kind":"status","text":"Incremental discovery did not verify cleanly; falling back fail-closed to full discovery."}),
            );
        }
        // Restore full schema before executing fail-closed full discovery.
        save(&directory, "schema.json", &schema, true)?;
    }

    let argv = prepare_discovery_argv(&provider, prepared_full.argv, &codex, &spec)?;
    attempt += 1;
    signal(
        &mut agent,
        AgentSignal::Started {
            agent: "discovery".into(),
            attempt,
            cwd: workspace.clone(),
            details: object!({"label":"Discovery agent","service":"__system__","mode":"read-only","provider":provider,"root":workspace,"readable":ids,"writable":[]}),
        },
    );
    display(
        &mut event,
        object!({"kind":"status","text":"AI is inspecting captured service evidence."}),
    );
    let output = native::run_raw_monitored(
        &argv,
        &workspace,
        &prepared_full.environment,
        &prepared_full.remove,
        &prepared_full.input,
        timeout,
        cancel.clone(),
        runner_lock,
        observer,
        |line| {
            signal(
                &mut agent,
                AgentSignal::Output {
                    bytes: line.to_vec(),
                },
            );
            Ok(())
        },
    )
    .await;
    let result = (|| -> Result<Value> {
        let output = output?;
        if cancel.load(Ordering::SeqCst) {
            return Err(RuntimeError::unavailable("Discovery interrupted"));
        }
        if output.code != 0 {
            return Err(fail(
                match native::worker_failure(&output.bytes, &provider) {
                    Some(cause) => format!(
                        "The {provider} CLI exited with code {}: {cause}",
                        output.code
                    ),
                    None => format!("Discovery provider exited with code {}", output.code),
                },
            ));
        }
        let report = native::worker_result(&output.bytes, &provider)?;
        verify_and_commit_discovery_report(
            &directory,
            ledger_dir,
            &request,
            &report,
            &manifest_index,
            &provider,
            &boundary_sha,
        )
    })();
    match result {
        Ok(proposal) => {
            signal(
                &mut agent,
                AgentSignal::Finished {
                    status: "completed".into(),
                    details: object!({"ok":true,"duration_seconds":started.elapsed().as_secs_f64(),"summary":format!("{} service passports proposed for review",ids.len())}),
                },
            );
            display(
                &mut event,
                object!({"kind":"text","text":"Service metadata proposal is ready for review. No source metadata was written."}),
            );
            display(&mut event, object!({"kind":"result","ok":true}));
            Ok(proposal)
        }
        Err(error) => {
            signal(
                &mut agent,
                AgentSignal::Finished {
                    status: if cancel.load(Ordering::SeqCst) {
                        "interrupted"
                    } else {
                        "blocked"
                    }
                    .into(),
                    details: object!({"ok":false,"error":error.to_string(),"duration_seconds":started.elapsed().as_secs_f64()}),
                },
            );
            display(
                &mut event,
                object!({"kind":"error","text":format!("AI discovery failed: {error}")}),
            );
            display(&mut event, object!({"kind":"result","ok":false}));
            Err(error)
        }
    }
}

pub async fn run_system_request(
    path: &Path,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<File>>,
    event: impl FnMut(Value) -> Result<()> + Send,
) -> Result<Value> {
    Box::pin(run_system_context(
        path,
        cancel,
        runner_lock,
        None,
        None,
        None,
        None,
        None,
        event,
        |_: AgentSignal| Ok(()),
    ))
    .await
}
pub async fn run_system_request_with_adapter(
    path: &Path,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<File>>,
    adapter: Arc<dyn infra_host::provider::LaunchAdapter>,
    observer: Arc<dyn infra_host::process::RunObserver>,
    event: impl FnMut(Value) -> Result<()> + Send,
    agent: impl FnMut(AgentSignal) -> Result<()> + Send,
) -> Result<Value> {
    run_system_request_with_adapter_and_approvals(
        path,
        cancel,
        runner_lock,
        adapter,
        observer,
        None,
        event,
        agent,
    )
    .await
}
#[expect(
    clippy::too_many_arguments,
    reason = "Adapter entry point carries explicit custody, observer, approvals, and callbacks."
)]
pub async fn run_system_request_with_adapter_and_approvals(
    path: &Path,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<File>>,
    adapter: Arc<dyn infra_host::provider::LaunchAdapter>,
    observer: Arc<dyn infra_host::process::RunObserver>,
    approvals: Option<Arc<dyn infra_host::permission::Approvals>>,
    event: impl FnMut(Value) -> Result<()> + Send,
    agent: impl FnMut(AgentSignal) -> Result<()> + Send,
) -> Result<Value> {
    Box::pin(run_system_context(
        path,
        cancel,
        runner_lock,
        Some(adapter),
        Some(observer),
        None,
        None,
        approvals,
        event,
        agent,
    ))
    .await
}
#[expect(
    clippy::too_many_arguments,
    reason = "Bridge entry point carries explicit custody, observer, launch identity, and graph callbacks."
)]
pub async fn run_system_request_with_bridge(
    path: &Path,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<File>>,
    adapter: Arc<dyn infra_host::provider::LaunchAdapter>,
    observer: Arc<dyn infra_host::process::RunObserver>,
    bridge: Arc<dyn CodeGraphBridge>,
    launch_id: Option<String>,
    event: impl FnMut(Value) -> Result<()> + Send,
    agent: impl FnMut(AgentSignal) -> Result<()> + Send,
) -> Result<Value> {
    run_system_request_with_bridge_and_approvals(
        path,
        cancel,
        runner_lock,
        adapter,
        observer,
        bridge,
        launch_id,
        None,
        event,
        agent,
    )
    .await
}
#[expect(
    clippy::too_many_arguments,
    reason = "Bridge entry point carries explicit custody, observer, launch identity, approvals, and graph callbacks."
)]
pub async fn run_system_request_with_bridge_and_approvals(
    path: &Path,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<File>>,
    adapter: Arc<dyn infra_host::provider::LaunchAdapter>,
    observer: Arc<dyn infra_host::process::RunObserver>,
    bridge: Arc<dyn CodeGraphBridge>,
    session_id: Option<String>,
    approvals: Option<Arc<dyn infra_host::permission::Approvals>>,
    event: impl FnMut(Value) -> Result<()> + Send,
    agent: impl FnMut(AgentSignal) -> Result<()> + Send,
) -> Result<Value> {
    Box::pin(run_system_context(
        path,
        cancel,
        runner_lock,
        Some(adapter),
        Some(observer),
        Some(bridge),
        session_id,
        approvals,
        event,
        agent,
    ))
    .await
}
pub async fn run_discovery_request(
    path: &Path,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<File>>,
    event: impl FnMut(Value) -> Result<()> + Send,
) -> Result<Value> {
    run_discovery_context(
        path,
        cancel,
        runner_lock,
        None,
        None,
        event,
        |_: AgentSignal| Ok(()),
    )
    .await
}
pub async fn run_discovery_request_with_adapter(
    path: &Path,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<File>>,
    adapter: Arc<dyn infra_host::provider::LaunchAdapter>,
    observer: Arc<dyn infra_host::process::RunObserver>,
    event: impl FnMut(Value) -> Result<()> + Send,
    agent: impl FnMut(AgentSignal) -> Result<()> + Send,
) -> Result<Value> {
    run_discovery_context(
        path,
        cancel,
        runner_lock,
        Some(adapter),
        Some(observer),
        event,
        agent,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_worker_verifies_gate1_before_commit_and_detects_sandbox_boundary_drift() {
        let base =
            std::env::temp_dir().join(format!("infra-sys-workers-test-{}", uuid::Uuid::new_v4()));
        let sys_root = base.join("sys");
        let svc_a = base.join("svc-a");
        let run1_dir = base.join("system-discovery/run1");
        let run2_dir = base.join("system-discovery/run2");
        let ledger_dir = base.join("system-discovery");
        std::fs::create_dir_all(&sys_root).unwrap();
        std::fs::create_dir_all(&svc_a).unwrap();
        std::fs::create_dir_all(&run1_dir).unwrap();
        std::fs::create_dir_all(&run2_dir).unwrap();

        std::fs::write(sys_root.join("ai-system.json"), "{}\n").unwrap();
        std::fs::write(svc_a.join("CODEOWNERS"), "* @orders-team\n").unwrap();
        std::fs::write(svc_a.join("README.md"), "# Service A\n").unwrap();

        let (_, sig_sys) = discovery::collect(&sys_root, &[]).unwrap();
        let (_, sig_a) = discovery::collect(&svc_a, &[]).unwrap();
        let sha_sys = digest(b"{}\n");
        let sha_co = digest(b"* @orders-team\n");
        let sha_a = digest(b"# Service A\n");

        let spec1 = infra_host::sandbox::Spec {
            workspace: run1_dir.join("agent"),
            target: run1_dir.join("agent"),
            protected: vec![sys_root.clone(), svc_a.clone()],
            readable: vec![],
            accounts: vec![base.join(".codex")],
            runtime: vec![base.join("bin")],
            codex: None,
            evidence: Some(run1_dir.join("agent/evidence")),
            network: true,
        };
        let mut spec_changed = spec1.clone();
        spec_changed.accounts.push(base.join(".claude"));
        let b1 = sandbox_boundary_sha256(
            &spec1,
            "codex",
            Path::new("/usr/bin/codex"),
            &["svc-a".into()],
        )
        .unwrap();
        let b1_same = sandbox_boundary_sha256(
            &spec1,
            "codex",
            Path::new("/usr/bin/codex"),
            &["svc-a".into()],
        )
        .unwrap();
        let b2 = sandbox_boundary_sha256(
            &spec_changed,
            "codex",
            Path::new("/usr/bin/codex"),
            &["svc-a".into()],
        )
        .unwrap();
        let b3 = sandbox_boundary_sha256(
            &spec1,
            "claude",
            Path::new("/usr/bin/codex"),
            &["svc-a".into()],
        )
        .unwrap();
        assert_eq!(b1, b1_same);
        assert_ne!(b1, b2);
        assert_ne!(b1, b3);

        let request = object!({
            "editor": {
                "project_id": "proj1",
                "config_path": sys_root.join("ai-system.json"),
                "revision": 1,
                "name": "Shop",
                "shared_sources": [],
                "services": [
                    {
                        "path": svc_a,
                        "passport": {
                            "schema_version": 1,
                            "id": "svc-a",
                            "description": "Draft",
                            "owner": "unknown",
                            "relationships_complete": false,
                            "capabilities": [],
                            "provides": [],
                            "consumes": [],
                            "sources": []
                        }
                    }
                ]
            },
            "roots": {
                "__system__": {"path": sys_root, "excluded": [], "metadata": [], "signature": sig_sys},
                "svc-a": {"path": svc_a, "excluded": [], "metadata": [], "signature": sig_a}
            },
            "inventory": {
                "__system__": {"ai-system.json": {"kind": "code", "sha256": sha_sys, "copy": "evidence/000000.txt"}},
                "svc-a": {
                    "CODEOWNERS": {"kind": "policy", "sha256": sha_co, "copy": "evidence/000001.txt"},
                    "README.md": {"kind": "spec", "sha256": sha_a, "copy": "evidence/000002.txt"}
                }
            },
            "warnings": []
        });

        let report = object!({
            "name": "Shop",
            "shared_sources": [{"path": "ai-system.json", "kind": "code"}],
            "warnings": [],
            "services": [
                {
                    "id": "svc-a",
                    "passport": {
                        "schema_version": 1,
                        "id": "svc-a",
                        "description": "Orders",
                        "owner": "@orders-team",
                        "relationships_complete": false,
                        "capabilities": [],
                        "provides": [],
                        "consumes": [],
                        "sources": [{"path": "README.md", "kind": "spec"}]
                    },
                    "owner_sources": ["CODEOWNERS"],
                    "description_sources": ["README.md"],
                    "consumption_sources": [],
                    "uncertainties": []
                }
            ]
        });

        let index = discovery::ManifestIndex::from_request(&request).unwrap();
        let mut unverified_report = report.clone();
        unverified_report["services"][0]["description_sources"] = object!(["bogus.md"]);
        assert!(
            verify_and_commit_discovery_report(
                &run1_dir,
                &ledger_dir,
                &request,
                &unverified_report,
                &index,
                "codex",
                &b1,
            )
            .is_err()
        );
        assert!(!run1_dir.join("proposal.json").exists());

        let proposal = verify_and_commit_discovery_report(
            &run1_dir,
            &ledger_dir,
            &request,
            &report,
            &index,
            "codex",
            &b1,
        )
        .unwrap();
        assert_eq!(proposal["editor"]["name"], "Shop");
        assert!(run1_dir.join("proposal.json").is_file());

        let ids = vec!["svc-a".to_string()];
        let future_now = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i128)
            + discovery::DISCOVERY_RACY_NS
            + 10_000;
        let plan_hit = discovery::plan_incremental_discovery_at(
            &ledger_dir,
            &request,
            &ids,
            &index,
            "codex",
            &b1,
            future_now,
        )
        .unwrap();
        assert!(plan_hit.pending_ids.is_empty());
        assert!(plan_hit.fallback_reason.is_none());

        let cached = build_cached_discovery_report(&request, &ids, &plan_hit).unwrap();
        verify_and_commit_discovery_report(
            &run2_dir,
            &ledger_dir,
            &request,
            &cached,
            &index,
            "codex",
            &b1,
        )
        .unwrap();
        assert!(run2_dir.join("proposal.json").is_file());

        let _ = std::fs::remove_dir_all(&base);
    }
}
