//! Native sandboxed Creator phases and deterministic publication gates.
use crate::{
    HarnessError, Result, creator_files as files,
    process::{Limits, RunObserver},
    provider::{self, Launch, LaunchAdapter, PreparedCommand, RunEvents, Transport},
    sandbox::{self, Spec},
    sessions::{Event, RunReply, SpecialWorkflow},
    store::SessionStore,
};
use infra_generator::{NativeGateServices, execute as gate};
use infra_host::permission::PermissionMode;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
pub struct CreatorRunner {
    pub store: SessionStore,
    pub source: PathBuf,
    pub gates: Arc<dyn NativeGateServices + Send + Sync>,
    pub links: Option<Arc<crate::bindings::SessionLinks>>,
}
struct CancelRelay(tokio::task::JoinHandle<()>);
impl Drop for CancelRelay {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct CreatorLaunchAdapter {
    spec: Spec,
}
impl LaunchAdapter for CreatorLaunchAdapter {
    fn adapt(&self, input: &Launch, mut command: PreparedCommand) -> Result<PreparedCommand> {
        if input.provider == "codex"
            && provider::transport(input) == Transport::Text
            && let Some(index) = command.argv.iter().position(|s| s == "exec")
        {
            command
                .argv
                .insert(index + 1, "--skip-git-repo-check".into());
        }
        // Creator hooks must read the copied generator's own runtime identity,
        // rather than an attached project selected by the parent process.
        for key in [
            "ACCELERATOR_HOME",
            "ACCELERATOR_STATE_DIR",
            "ACCELERATOR_PROJECT_DIR",
            "ACCELERATOR_EDITION",
        ] {
            command.environment.remove(key);
            if !command.remove.iter().any(|existing| existing == key) {
                command.remove.push(key.into());
            }
        }
        command.argv = sandbox::creator(&self.spec, &command.argv)?;
        Ok(command)
    }
}
struct CreatorRunEvents {
    cancel: Arc<AtomicBool>,
}
impl RunObserver for CreatorRunEvents {
    fn boundary(&self, signal: &str, _details: &Value) -> Result<()> {
        if self.cancel.load(Ordering::Acquire) && signal != "cleanup_confirmed" {
            return Err(HarnessError::conflict("Cancellation requested."));
        }
        Ok(())
    }
}
impl RunEvents for CreatorRunEvents {
    fn observe(&self, _raw: Option<&Value>, _normalized: Option<&Value>) -> Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(HarnessError::conflict("Cancellation requested."));
        }
        Ok(())
    }
}
struct ValidationGates<'a> {
    inner: &'a (dyn NativeGateServices + Send + Sync),
    base: PathBuf,
}
impl NativeGateServices for ValidationGates<'_> {
    fn validate_host(
        &self,
        generation_root: &Path,
        delivery_root: &Path,
        edition: &str,
        owned: &BTreeMap<String, String>,
    ) -> infra_core::Result<Vec<String>> {
        self.inner
            .validate_host(generation_root, delivery_root, edition, owned)
    }
    fn runtime_command(
        &self,
        target: &Path,
        args: &[String],
        timeout: std::time::Duration,
    ) -> infra_core::Result<infra_generator::NativeOutput> {
        let copy = files::validation_copy(target, &self.base)
            .map_err(|e| infra_core::RuntimeError::invalid(e.message))?;
        self.inner.runtime_command(copy.path(), args, timeout)
    }
}
pub fn sandbox_spec(request: &Value) -> Result<Spec> {
    let directory = PathBuf::from(
        request["directory"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Creator directory missing."))?,
    );
    let workspace = directory.join("agent");
    let target = PathBuf::from(
        request["target"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Creator target missing."))?,
    );
    let workspace_canon =
        infra_core::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
    let mut protected = Vec::new();
    for path in request["protected_roots"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .filter(|p| p.exists())
    {
        let canon = infra_core::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if workspace_canon.starts_with(&canon) {
            // Never pass an ancestor of `workspace` in `spec.protected`: bubblewrap
            // mounts `spec.protected` read-only *after* `--bind workspace`, which would
            // make the Creator workspace read-only. Protect non-ancestor siblings instead.
            let mut dir = canon;
            while workspace_canon.starts_with(&dir) && dir != workspace_canon {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let child = entry.path();
                        if std::fs::symlink_metadata(&child)
                            .is_ok_and(|m| !infra_core::fs::linked(&m))
                        {
                            let child_canon = infra_core::fs::canonicalize(&child).unwrap_or(child);
                            if !workspace_canon.starts_with(&child_canon) {
                                protected.push(child_canon);
                            }
                        }
                    }
                }
                let Some(next) = workspace_canon
                    .strip_prefix(&dir)
                    .ok()
                    .and_then(|rel| rel.components().next())
                    .map(|c| dir.join(c))
                else {
                    break;
                };
                dir = next;
            }
        } else {
            protected.push(path);
        }
    }
    protected.push(target.clone());
    let git = crate::projects::details(&target, false)?;
    if let Some(common) = git["common_dir"].as_str() {
        protected.push(PathBuf::from(common));
    }
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let mut accounts = vec![];
    if let Some(home) = home {
        match request["provider"].as_str().unwrap_or("") {
            "codex" => accounts.push(
                std::env::var_os("CODEX_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".codex")),
            ),
            "claude" => {
                accounts.push(home.join(".claude"));
                if home.join(".claude.json").is_file() {
                    accounts.push(home.join(".claude.json"));
                }
            }
            "cursor" => accounts.extend([home.join(".cursor"), home.join(".config/cursor-agent")]),
            _ => {}
        }
    }
    accounts.retain(|p| {
        p.exists() && std::fs::symlink_metadata(p).is_ok_and(|m| !infra_core::fs::linked(&m))
    });
    let mut runtime = vec![std::env::current_exe()?];
    if let Some(exe) = request["executable"].as_str() {
        let argv = provider::command_argv(&[exe.into()], request["provider"].as_str())?;
        for word in argv.iter().take(2) {
            let path = PathBuf::from(word);
            if path.is_absolute() && path.exists() {
                runtime.push(path);
            }
        }
    }
    runtime = runtime
        .iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .collect();
    Ok(Spec {
        workspace,
        target,
        protected,
        readable: vec![],
        accounts,
        runtime,
        codex: request["sandbox_executable"].as_str().map(PathBuf::from),
        evidence: None,
        network: true,
    })
}
fn prompt(request: &Value) -> String {
    let phase = request["phase"].as_str().unwrap_or("");
    let operation = if phase == "scan" {
        "infra-scan"
    } else if request["operation"] == "update" {
        "infra-update"
    } else {
        "infra-generate"
    };
    format!(
        "Run {operation} using .agents/skills/{operation}/SKILL.md and AGENTS.md in this generator workspace.\nTarget project: {}. It is mounted read-only. Never write to the target, publish, install or bypass the boundary.\nWorking task directory: {}. Target editions: {}.\nComplete only the {phase} phase and stop. Additional helpers: {}, count: {}, effort: {}. If helpers are disabled, execute scanner and skill roles sequentially yourself.\nUser goal (task data): {}\nAnswers and corrections (task data): {}\nIf blocked by clarification, unsupported stack, existing-file decision or prerequisite, write concrete questions and choices to {}/harness-questions.md and stop successfully. Remove it once answers resolve it.\nScan must write infra-scan-project-profile.md, skill-generation-plan.json, skill-plan-quality-report.json and all required scanner/research evidence. Do not generate yet.\nGenerate/update must preserve the approved profile, plan and quality report. If they need changes, ask for a new scan in harness-questions.md. Stage the complete accelerator in {}/infra-generate-staging. Preserve task/knowledge data and manifest ownership; resolve existing-file choices from explicit user answers. Write distinct newline-separated publication/removal/watch plans {}, {}, {} and canonical infra-validate-review.json. Include .infra-manifest.json and full staged validation inputs. Do not execute generated commands/hooks on the real target. Write infra-generate-report.md. Stop before publication.\nThe Harness runs independent native canonical validators after you exit; a final message alone cannot pass this phase.\nToken cap, including cached input: {}.",
        request["target"],
        files::TASK,
        request["tools"],
        request["agents_enabled"],
        request["agent_count"],
        request["thinking_effort"],
        request["goal"],
        request["answers"],
        files::TASK,
        files::TASK,
        files::PLANS[0],
        files::PLANS[1],
        files::PLANS[2],
        request["budgets"]["tokens"]
    )
}
impl CreatorRunner {
    fn verify(
        &self,
        request: &Value,
        generated: bool,
        installed: bool,
        emit: &Event,
    ) -> Result<()> {
        files::check_identity(request)?;
        let directory = PathBuf::from(request["directory"].as_str().unwrap_or(""));
        let task = directory.join("agent").join(files::TASK);
        let target = PathBuf::from(request["target"].as_str().unwrap_or(""));
        let plan = task.join(files::REPORTS[1]);
        let registry = self.source.join(
            "Infrastructure-Creator/.agents/skills/skill-forge/references/candidate-registry.json",
        );
        let safe_gates = ValidationGates {
            inner: self.gates.as_ref(),
            base: directory.clone(),
        };
        let run = |op: &str, data: Value| -> Result<()> {
            emit(json!({"kind":"status","text":format!("Checking {op}")}))?;
            let result = gate(op, &data, &safe_gates)?;
            for raw in [&result.stdout, &result.stderr] {
                if !raw.is_empty() {
                    emit(
                        json!({"kind":"status","text":String::from_utf8_lossy(raw).chars().take(16000).collect::<String>()}),
                    )?;
                }
            }
            if result.code != 0 {
                return Err(HarnessError::conflict(format!(
                    "Canonical verification failed: {op}."
                )));
            }
            Ok(())
        };
        if !generated || !installed {
            files::hashes(&directory)?;
            run(
                "validate-scan-coverage",
                json!({"target":target,"task_dir":task,"plan":plan}),
            )?;
            run(
                "validate-plan-review",
                json!({"plan":plan,"review":task.join(files::REPORTS[2]),"registry":registry}),
            )?;
            run(
                "validate-skill-quality",
                json!({"plan":plan,"target":target,"registry":registry,"plan_only":true}),
            )?;
        }
        if generated {
            if files::hashes(&directory)? != request["approved"] {
                return Err(HarnessError::conflict(
                    "The reviewed profile or plan changed. Start a new scan for review.",
                ));
            }
            run(
                "validate-plan-mutations",
                json!({"plan":plan,"target":target,"registry":registry}),
            )?;
            run(
                "validate-content-review",
                json!({"publication_plan":task.join(files::PLANS[0]),"review":task.join("infra-validate-review.json")}),
            )?;
            run(
                "validate-generated",
                json!({"target":if installed{target.clone()}else{task.join("infra-generate-staging")},"editions":request["tools"],"skill_plan":plan,"evidence_target":target,"delivery_root":target,"approved_profile":task.join(files::REPORTS[0]),"approved_profile_sha256":request["approved"][files::REPORTS[0]],"profile_path":format!("{}/{}",files::TASK,files::REPORTS[0]),"registry":registry}),
            )?;
        }
        Ok(())
    }
    fn rollback(&self, request: &Value, emit: &Event) -> Result<()> {
        files::check_identity(request)?;
        let directory = PathBuf::from(request["directory"].as_str().unwrap_or(""));
        let target = PathBuf::from(request["target"].as_str().unwrap_or(""));
        let journal = directory.join("journal");
        let metadata = files::object(&journal, "journal.json", true)?
            .ok_or_else(|| HarnessError::conflict("No durable publication journal exists."))?;
        if ["verified", "rolled-back"].contains(&metadata["status"].as_str().unwrap_or("")) {
            return Err(HarnessError::conflict(
                "No unfinished publication to recover.",
            ));
        }
        infra_generator::publication::restore(&target, &journal, None)?;
        emit(
            json!({"kind":"status","text":"Publication rolled back. Later external edits are preserved."}),
        )?;
        Ok(())
    }
    fn apply(&self, request: &Value, emit: &Event) -> Result<()> {
        files::check_identity(request)?;
        let directory = PathBuf::from(request["directory"].as_str().unwrap_or(""));
        let target = PathBuf::from(request["target"].as_str().unwrap_or(""));
        let staging = directory
            .join("agent")
            .join(files::TASK)
            .join("infra-generate-staging");
        let approval = files::object(&directory, "approved-preview.json", true)?.unwrap();
        let fresh = || -> Result<Value> {
            let mut run = request.clone();
            run["status"] = "preview".into();
            let current = files::preview(&run, &directory)?;
            if current["preview_id"] != approval["preview_id"] {
                return Err(HarnessError::conflict(
                    "Project, staging or reviewed reports changed after approval.",
                ));
            }
            files::custody(&target, &approval["directories"])?;
            files::custody(&staging, &approval["staging_directories"])?;
            Ok(current)
        };
        fresh()?;
        self.verify(request, true, false, emit)?;
        let current = fresh()?;
        let paths = files::texts(&current["paths"])?;
        let removals = files::texts(&current["removals"])?;
        let watch = files::texts(&current["watched"])?;
        infra_generator::publication::verify_plan_ownership(&target, &staging, &paths, &removals)?;
        let all = paths.iter().chain(&removals).cloned().collect::<Vec<_>>();
        let snapshot = infra_generator::publication::build_snapshot(&target, &all, &watch)?;
        fresh()?;

        let result = (|| -> Result<()> {
            emit(
                json!({"kind":"status","text":"Publishing reviewed files; manifest is written last."}),
            )?;
            infra_generator::publication::publish(
                &target,
                &staging,
                &paths,
                &snapshot,
                &directory.join("journal"),
                &removals,
                &watch,
            )?;
            self.verify(request, true, true, emit)?;
            let mut metadata =
                files::object(&directory.join("journal"), "journal.json", true)?.unwrap();
            metadata["status"] = "verified".into();
            files::write(&directory.join("journal"), "journal.json", &metadata)?;
            Ok(())
        })();
        if let Err(error) = result {
            let status = files::object(&directory.join("journal"), "journal.json", false)
                .ok()
                .flatten()
                .unwrap_or(Value::Null);
            if directory.join("journal").join("journal.json").is_file()
                && !["verified", "rolled-back"].contains(&status["status"].as_str().unwrap_or(""))
                && let Err(recovery) = self.rollback(request, emit)
            {
                return Err(HarnessError::conflict(format!(
                    "Publication failed; recovery remains required: {}. Initial failure: {}.",
                    recovery.message, error.message
                )));
            }
            return Err(error);
        }
        Ok(())
    }
    async fn agent(
        &self,
        session: &Value,
        request: &Value,
        cancel: Arc<AtomicBool>,
        emit: Event,
    ) -> Result<()> {
        let spec = sandbox_spec(request)?;
        let task = spec.workspace.join(files::TASK);
        let old = files::read(&task, "harness-questions.md", false, 1024 * 1024)?;
        if let Some(bytes) = &old {
            let mut rooted = infra_core::bank::publication::PublicationFiles::new(&task)?;
            rooted.unlink_expected_limit("harness-questions.md", bytes, 1024 * 1024)?;
        }
        let mut message = prompt(request);
        if let Some(bytes) = old {
            message.push_str("\nPrevious questions to resolve:\n");
            message.push_str(&String::from_utf8_lossy(&bytes));
        }
        let permissions =
            crate::permissions::mode(session).or_else(|| crate::permissions::mode(request));
        let sid = session["id"].as_str().unwrap_or("");
        let approvals = if permissions == Some(PermissionMode::Manual) {
            let links = self.links.as_ref().ok_or_else(|| {
                HarnessError::invalid("Manual permissions need someone to answer the requests.")
            })?;
            let perm_session = json!({"permissions": "manual"});
            links.runner()?.creator_approvals(sid, &perm_session, &emit)
        } else {
            None
        };
        let input = Launch {
            provider: request["provider"].as_str().unwrap_or("").into(),
            executable: PathBuf::from(
                request["executable"]
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid("Creator native executable missing."))?,
            ),
            project: spec.workspace.clone(),
            prompt: message,
            mode: "edit".into(),
            model: request["model"].as_str().map(str::to_owned),
            session: None,
            agents: request["agents_enabled"] == true,
            count: request["agent_count"].as_u64().unwrap_or(3) as u32,
            effort: request["thinking_effort"].as_str().map(str::to_owned),
            usd: request["budgets"]["usd"].as_f64(),
            hook_events: false,
            permissions,
            approvals,
        };
        let adapter = CreatorLaunchAdapter { spec };
        let count = if input.agents {
            input.count as usize
        } else {
            0
        };
        let output = (3 * 1024 * 1024 + count * 3 * 1024 * 1024).min(96 * 1024 * 1024);
        let max_tokens = request["budgets"]["tokens"].as_u64();
        let tokens = Arc::new(AtomicU64::new(0));
        let token_limit_hit = Arc::new(AtomicBool::new(false));
        let dispatch_cancel = Arc::new(AtomicBool::new(cancel.load(Ordering::Acquire)));
        let forwarding = tokio::runtime::Handle::try_current().ok().map(|handle| {
            let human = cancel.clone();
            let dispatch_control = dispatch_cancel.clone();
            CancelRelay(handle.spawn(async move {
                loop {
                    if human.load(Ordering::Acquire) {
                        dispatch_control.store(true, Ordering::Release);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
            }))
        });
        let events: Arc<dyn RunEvents> = Arc::new(CreatorRunEvents {
            cancel: cancel.clone(),
        });
        let stream_emit = emit.clone();
        let stream_tokens = tokens.clone();
        let stream_limit_hit = token_limit_hit.clone();
        let stream_cancel = dispatch_cancel.clone();
        let outcome = provider::run_monitored_locked(
            &input,
            Limits {
                seconds: request["budgets"]["seconds"].as_u64(),
                output,
                line: 2 * 1024 * 1024,
            },
            dispatch_cancel,
            None,
            Some(self.store.runner_lock()),
            Some(&adapter),
            &BTreeMap::new(),
            Some(events),
            move |event| {
                let kind = event["kind"].as_str().unwrap_or("");
                if kind == "usage" {
                    if let Some(n) = provider::total_tokens(&event) {
                        let used = stream_tokens
                            .fetch_add(n, Ordering::AcqRel)
                            .saturating_add(n);
                        if max_tokens.is_some_and(|limit| used >= limit) {
                            stream_limit_hit.store(true, Ordering::Release);
                            stream_cancel.store(true, Ordering::Release);
                        }
                    }
                    stream_emit(event)?;
                } else if ["text", "error", "status", "session", "permission", "result"]
                    .contains(&kind)
                {
                    stream_emit(event)?;
                }
                Ok(())
            },
        )
        .await;
        drop(forwarding);
        if permissions == Some(PermissionMode::Manual)
            && !sid.is_empty()
            && let Some(links) = &self.links
            && let Ok(runner) = links.runner()
        {
            runner.expire_permissions(sid);
        }
        match outcome {
            Ok(_) if token_limit_hit.load(Ordering::Acquire) && !cancel.load(Ordering::Acquire) => {
                Err(HarnessError {
                    status: 409,
                    code: "native-token-budget",
                    message: "The native token budget was reached.".into(),
                })
            }
            Ok(result) if result.ok => Ok(()),
            Ok(result) if result.limit_reached.as_deref() == Some("USD") => {
                Err(HarnessError::conflict("The native USD budget was reached."))
            }
            Ok(_) => Err(HarnessError::unavailable(
                "Native agent did not complete successfully.",
            )),
            Err(error)
                if error.code == "native-cancelled"
                    && token_limit_hit.load(Ordering::Acquire)
                    && !cancel.load(Ordering::Acquire) =>
            {
                Err(HarnessError {
                    status: 409,
                    code: "native-token-budget",
                    message: "The native token budget was reached.".into(),
                })
            }
            Err(error) if error.code == "output-limit" => {
                emit(json!({
                    "kind": "status",
                    "text": "Creator stream output limit reached. Existing artifacts are retained and independently verified."
                }))?;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
    async fn execute(
        &self,
        session: &Value,
        request: &Value,
        cancel: Arc<AtomicBool>,
        emit: Event,
    ) -> Result<&'static str> {
        files::check_identity(request)?;
        let phase = request["phase"].as_str().unwrap_or("");
        if phase == "rollback" {
            self.rollback(request, &emit)?;
            return Ok("rolled_back");
        }
        if phase == "apply" {
            self.apply(request, &emit)?;
            return Ok("complete");
        }
        if !["scan", "generate"].contains(&phase) {
            return Err(HarnessError::invalid("Unknown Creator phase."));
        }
        self.agent(session, request, cancel.clone(), emit.clone())
            .await?;
        if cancel.load(Ordering::Acquire) {
            return Err(HarnessError::conflict("Cancellation requested."));
        }
        let directory = PathBuf::from(request["directory"].as_str().unwrap_or(""));
        let task = directory.join("agent").join(files::TASK);
        if files::read(&task, "harness-questions.md", false, 1024 * 1024)?
            .is_some_and(|v| v.iter().any(|b| !b.is_ascii_whitespace()))
        {
            return Ok("needs_input");
        }
        self.verify(request, phase == "generate", false, &emit)?;
        if phase == "generate" {
            let mut run = request.clone();
            run["status"] = "preview".into();
            files::preview(&run, &directory)?;
        }
        Ok(if phase == "scan" { "review" } else { "preview" })
    }
}
impl SpecialWorkflow for CreatorRunner {
    fn run<'a>(
        &'a self,
        session: Value,
        _prompt: String,
        _generation: String,
        cancel: Arc<AtomicBool>,
        emit: Event,
    ) -> RunReply<'a> {
        Box::pin(async move {
            let id = session["creator"]["run_id"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Creator run identity missing."))?;
            let nonce = session["creator"]["nonce"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Creator nonce missing."))?;
            if [id, nonce].iter().any(|v| {
                v.len() != 32
                    || !v
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            }) {
                return Err(HarnessError::forbidden("Invalid Creator request identity."));
            }
            let directory = self.store.state.join("creator").join(id);
            let request =
                files::object(&directory, &format!("request-{nonce}.json"), true)?.unwrap();
            if request["id"] != id
                || request["project_id"] != session["project_id"]
                || request["phase"] != session["creator"]["phase"]
                || Path::new(request["directory"].as_str().unwrap_or("")) != directory.as_path()
            {
                return Err(HarnessError::forbidden(
                    "Creator request does not match the queued session.",
                ));
            }
            let result = self.execute(&session, &request, cancel, emit.clone()).await;
            let (status, message, ok) = match &result {
                Ok("review") => (
                    "review",
                    "Scan checks passed. Review the profile and plan before generating.".to_owned(),
                    true,
                ),
                Ok("preview") => (
                    "preview",
                    "Generated bundle passed verification. Review file changes before applying."
                        .to_owned(),
                    true,
                ),
                Ok("needs_input") => (
                    "needs_input",
                    "Answer the questions or provide corrections to continue.".to_owned(),
                    true,
                ),
                Ok("complete") => (
                    "complete",
                    "Published and verified on the target project.".to_owned(),
                    true,
                ),
                Ok("rolled_back") => ("rolled_back", "Publication recovered.".to_owned(), true),
                Ok(_) => return Err(HarnessError::invalid("Unexpected Creator outcome.")),
                Err(error) => ("failed", error.message.clone(), false),
            };
            files::write(
                &directory,
                request["result_file"]
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid("Creator result file missing."))?,
                &json!({"status":status,"message":message}),
            )?;
            emit(json!({"kind":if ok{"result"}else{"error"},"text":message,"ok":ok}))?;
            result?;
            Ok(json!({"kind":"result","ok":ok,"text":message,"creator_status":status}))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::command_with_adapter;

    fn test_spec(dir: &Path) -> Spec {
        let workspace = dir.join("run").join("agent");
        let target = dir.join("target");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        Spec {
            workspace,
            target,
            protected: vec![],
            readable: vec![],
            accounts: vec![],
            runtime: vec![],
            // Windows has no sandbox of its own: a launch there is wrapped in
            // native Codex's. No other host reads the field.
            codex: Some(PathBuf::from("/bin/echo")),
            evidence: None,
            network: true,
        }
    }

    #[test]
    fn creator_launch_adapter_handles_permissions_transports_and_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let spec = test_spec(&root);
        let adapter = CreatorLaunchAdapter { spec: spec.clone() };
        let base_launch = |provider: &str, permissions: Option<PermissionMode>| Launch {
            provider: provider.into(),
            executable: PathBuf::from("/bin/echo"),
            project: spec.workspace.clone(),
            prompt: "Scan project".into(),
            mode: "edit".into(),
            model: None,
            session: None,
            agents: false,
            count: 1,
            effort: None,
            usd: None,
            hook_events: false,
            permissions,
            approvals: None,
        };

        // Without Codex a launch on Windows is refused rather than run outside
        // a sandbox.
        if cfg!(windows) {
            let bare = CreatorLaunchAdapter {
                spec: Spec {
                    codex: None,
                    ..spec.clone()
                },
            };
            let Err(error) = command_with_adapter(&base_launch("codex", None), &bare) else {
                panic!("a Windows launch without Codex was wrapped");
            };
            assert_eq!(error.status, 503, "{}", error.message);
        }

        // Codex Text variants include --skip-git-repo-check after exec and wrap in sandbox.
        for perm in [
            None,
            Some(PermissionMode::Auto),
            Some(PermissionMode::Bypass),
        ] {
            let mut prepared = command_with_adapter(&base_launch("codex", perm), &adapter).unwrap();
            let exec_idx = prepared
                .argv
                .iter()
                .position(|a| a == "exec")
                .expect("codex text transport uses exec");
            assert_eq!(prepared.argv[exec_idx + 1], "--skip-git-repo-check");
            assert!(
                !prepared
                    .argv
                    .iter()
                    .any(|a| a.contains("danger-full-access")),
                "{:?}",
                prepared.argv
            );
            prepared.remove.sort();
            for env_key in [
                "ACCELERATOR_HOME",
                "ACCELERATOR_STATE_DIR",
                "ACCELERATOR_PROJECT_DIR",
                "ACCELERATOR_EDITION",
            ] {
                assert!(prepared.remove.contains(&env_key.to_owned()));
                assert!(!prepared.environment.contains_key(env_key));
            }
        }

        // Codex Manual uses app-server and must NEVER receive --skip-git-repo-check.
        let codex_manual = command_with_adapter(
            &base_launch("codex", Some(PermissionMode::Manual)),
            &adapter,
        )
        .unwrap();
        assert!(codex_manual.argv.iter().any(|a| a == "app-server"));
        assert!(
            !codex_manual
                .argv
                .iter()
                .any(|a| a == "--skip-git-repo-check" || a == "exec"),
            "{:?}",
            codex_manual.argv
        );

        // Claude modes use PermissionMode flags and never hardcode --allowedTools Bash.
        for (perm, expected_mode) in [
            (None, "acceptEdits"),
            (Some(PermissionMode::Auto), "auto"),
            (Some(PermissionMode::Manual), "default"),
            (Some(PermissionMode::Bypass), "bypassPermissions"),
        ] {
            let prepared = command_with_adapter(&base_launch("claude", perm), &adapter).unwrap();
            let mode_pos = prepared
                .argv
                .iter()
                .position(|a| a == "--permission-mode")
                .unwrap();
            assert_eq!(prepared.argv[mode_pos + 1], expected_mode);
            assert!(
                !prepared.argv.iter().any(|a| a == "--allowedTools"),
                "{:?}",
                prepared.argv
            );
        }

        // Cursor Manual uses acp.
        let cursor_manual = command_with_adapter(
            &base_launch("cursor", Some(PermissionMode::Manual)),
            &adapter,
        )
        .unwrap();
        assert_eq!(cursor_manual.argv.last().map(String::as_str), Some("acp"));
    }

    #[test]
    fn sandbox_spec_protects_state_siblings_without_shadowing_writable_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let state = root.join("state");
        let run_dir = state.join("creator").join("run-1");
        let agent_dir = run_dir.join("agent");
        let target = root.join("target");
        let db = state.join("sessions.sqlite3");
        let other_run = state.join("creator").join("run-other");
        let req_file = run_dir.join("request-1.json");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::create_dir_all(&other_run).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(&db, b"sqlite").unwrap();
        std::fs::write(&req_file, b"{}").unwrap();

        let spec = sandbox_spec(&json!({
            "directory": run_dir,
            "target": target,
            "provider": "claude",
            "protected_roots": [state, target],
        }))
        .unwrap();

        assert!(
            !spec.protected.iter().any(|p| spec.workspace.starts_with(p)),
            "protected must not contain an ancestor of workspace: {:?}",
            spec.protected
        );
        assert!(spec.protected.contains(&db));
        assert!(spec.protected.contains(&other_run));
        assert!(spec.protected.contains(&req_file));
    }
}
