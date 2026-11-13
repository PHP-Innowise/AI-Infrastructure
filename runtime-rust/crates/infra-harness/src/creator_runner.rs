//! Native sandboxed Creator phases and deterministic publication gates.
use crate::{
    HarnessError, Result, creator_files as files,
    process::{self, Limits},
    provider::{self, Launch},
    sandbox::{self, Spec},
    sessions::{Event, RunReply, SpecialWorkflow},
    store::SessionStore,
};
use infra_generator::{NativeGateServices, execute as gate};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
pub struct CreatorRunner {
    pub store: SessionStore,
    pub source: PathBuf,
    pub gates: Arc<dyn NativeGateServices + Send + Sync>,
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
    let mut protected = request["protected_roots"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect::<Vec<_>>();
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
    async fn agent(&self, request: &Value, cancel: Arc<AtomicBool>, emit: Event) -> Result<()> {
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
        };
        let (mut command, environment, stdin) = provider::command(&input)?;
        if input.provider == "codex" {
            if let Some(index) = command.iter().position(|s| s == "exec") {
                command.insert(index + 1, "--skip-git-repo-check".into());
            }
            if cfg!(windows)
                && let Some(index) = command.iter().position(|s| s == "--sandbox")
            {
                command.splice(
                    index..index + 2,
                    [
                        "-c".into(),
                        "default_permissions=\":danger-full-access\"".into(),
                    ],
                );
            }
        }
        if input.provider == "claude" {
            if let Some(index) = command.iter().position(|s| s == "--allowedTools") {
                command.insert(index + 1, "Bash".into());
            } else {
                command.extend(["--allowedTools".into(), "Bash".into()]);
            }
        }
        let argv = sandbox::creator(&spec, &command)?;
        let count = if input.agents {
            input.count as usize
        } else {
            0
        };
        let output = (3 * 1024 * 1024 + count * 3 * 1024 * 1024).min(96 * 1024 * 1024);
        let mut terminal = false;
        let mut failed = false;
        let mut tokens = 0u64;
        let mut native_id: Option<String> = None;
        let provider = input.provider.clone();
        // Creator hooks must read the copied generator's own runtime identity,
        // rather than an attached project selected by the parent process.
        let remove = [
            "ACCELERATOR_HOME".to_string(),
            "ACCELERATOR_STATE_DIR".to_string(),
            "ACCELERATOR_PROJECT_DIR".to_string(),
            "ACCELERATOR_EDITION".to_string(),
        ];
        let outcome = process::run_environment(
            &argv,
            &spec.workspace,
            &environment,
            &remove,
            &stdin,
            Limits {
                seconds: request["budgets"]["seconds"].as_u64(),
                output,
                line: 2 * 1024 * 1024,
            },
            cancel.clone(),
            Some(self.store.runner_lock()),
            |line| {
                let Ok(raw) = crate::strict_json::parse(line, 2 * 1024 * 1024) else {
                    return Ok(());
                };
                for event in provider::normalize(&provider, &raw) {
                    let kind = event["kind"].as_str().unwrap_or("");
                    if let Some(id) = event["native_session_id"].as_str() {
                        native_id = Some(id.into());
                    }
                    if kind == "result" {
                        terminal = true;
                        failed |= event["ok"] != true;
                    }
                    if kind == "usage" {
                        tokens = tokens.saturating_add(provider::total_tokens(&event).unwrap_or(0));
                        if request["budgets"]["tokens"]
                            .as_u64()
                            .is_some_and(|limit| tokens >= limit)
                        {
                            cancel.store(true, Ordering::Release);
                        }
                        emit(event)?;
                    } else if ["text", "error", "status", "session"].contains(&kind) {
                        emit(event)?;
                    }
                }
                Ok(())
            },
        )
        .await;
        match outcome {
            Ok(output) if output.code == 0 && terminal && !failed => Ok(()),
            Err(error) if error.code == "output-limit" => {
                emit(
                    json!({"kind":"status","text":"Creator stream output limit reached. Existing artifacts are retained and independently verified."}),
                )?;
                Ok(())
            }
            Err(error) => Err(error),
            _ => Err(HarnessError::unavailable(
                "Native agent did not complete successfully.",
            )),
        }
    }
    async fn execute(
        &self,
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
        self.agent(request, cancel.clone(), emit.clone()).await?;
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
            let result = self.execute(&request, cancel, emit.clone()).await;
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
