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
    #[cfg(test)]
    force_sandbox_unavailable: bool,
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
        let copy_canon = infra_core::fs::canonicalize(copy.path())?;
        if args != ["status".to_string(), "--json".to_string()] && args != ["validate".to_string()]
        {
            return Err(infra_core::RuntimeError::policy(
                "Creator validation runtime accepts only `status --json` or `validate`.",
            ));
        }
        let bin_rel = if cfg!(windows) {
            "memory-bank/bin/infra-engine.exe"
        } else {
            "memory-bank/bin/infra-engine"
        };
        let installed = copy_canon.join(bin_rel);
        match std::fs::symlink_metadata(&installed) {
            Ok(meta) => {
                if infra_core::fs::linked(&meta)
                    || !meta.is_file()
                    || !current_exe_matches(&installed)?
                {
                    return Err(infra_core::RuntimeError::policy(
                        "Creator validation runtime binary is linked or differs from the pinned executable.",
                    ));
                }
                #[cfg(not(windows))]
                {
                    #[cfg(test)]
                    let sandbox_ok =
                        !self.force_sandbox_unavailable && infra_host::sandbox::available();
                    #[cfg(not(test))]
                    let sandbox_ok = infra_host::sandbox::available();
                    if !sandbox_ok {
                        return Err(infra_core::RuntimeError::coded(
                            infra_core::ErrorKind::Unavailable,
                            "SandboxUnavailable",
                            "SandboxUnavailable: Creator runtime validation requires Profile::Gate sandbox isolation on this host.",
                        ));
                    }
                    let (_pinned_dir, pinned_bin) = stage_pinned_helper(&self.base)?;
                    let mut argv = vec![
                        pinned_bin.to_string_lossy().into_owned(),
                        "memory".into(),
                        "--root".into(),
                        copy_canon.to_string_lossy().into_owned(),
                    ];
                    argv.extend(args.iter().cloned());
                    let runtime = pinned_bin
                        .parent()
                        .map(|parent| vec![parent.to_path_buf()])
                        .unwrap_or_default();
                    let spec = Spec {
                        workspace: copy_canon.clone(),
                        target: copy_canon.clone(),
                        protected: vec![],
                        readable: vec![],
                        accounts: vec![],
                        runtime,
                        codex: None,
                        evidence: None,
                        network: false,
                    };
                    let wrapped = sandbox::gate(&spec, &argv)
                        .map_err(|e| infra_core::RuntimeError::policy(e.message))?;
                    let mut env =
                        infra_host::sandbox::scrub_env(std::env::vars(), &BTreeMap::new());
                    for key in [
                        "ACCELERATOR_HOME",
                        "ACCELERATOR_STATE_DIR",
                        "ACCELERATOR_PROJECT_DIR",
                        "ACCELERATOR_EDITION",
                    ] {
                        env.remove(key);
                    }
                    let mut command = std::process::Command::new(&wrapped[0]);
                    command
                        .args(&wrapped[1..])
                        .current_dir(&copy_canon)
                        .env_clear()
                        .envs(&env)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped());
                    let mut child = command.spawn()?;
                    let deadline = std::time::Instant::now() + timeout;
                    let status = loop {
                        match child.try_wait()? {
                            Some(status) => break status,
                            None if std::time::Instant::now() < deadline => {
                                std::thread::sleep(std::time::Duration::from_millis(20));
                            }
                            None => {
                                let _ = child.kill();
                                let _ = child.wait();
                                return Err(infra_core::RuntimeError::unavailable(
                                    "Sandboxed Creator validation runtime timed out.",
                                ));
                            }
                        }
                    };
                    use std::io::Read;
                    let cap = infra_skillcache::tool::STDOUT_BYTES;
                    let mut stdout = Vec::new();
                    let mut stderr = Vec::new();
                    if let Some(out) = child.stdout.take() {
                        out.take(cap as u64 + 1).read_to_end(&mut stdout)?;
                    }
                    if let Some(err) = child.stderr.take() {
                        err.take(cap as u64 + 1).read_to_end(&mut stderr)?;
                    }
                    stdout.truncate(cap);
                    stderr.truncate(infra_skillcache::tool::DIAGNOSTIC_BYTES);
                    Ok(infra_generator::NativeOutput {
                        code: status.code().unwrap_or(1),
                        stdout,
                        stderr,
                    })
                }
                #[cfg(windows)]
                {
                    self.inner.runtime_command(copy.path(), args, timeout)
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.inner.runtime_command(copy.path(), args, timeout)
            }
            Err(err) => Err(err.into()),
        }
    }
}

fn stage_pinned_helper(base: &Path) -> infra_core::Result<(tempfile::TempDir, PathBuf)> {
    use std::io::{Read, Write};
    std::fs::create_dir_all(base)?;
    let dir = tempfile::Builder::new()
        .prefix("creator-pinned-")
        .tempdir_in(base)?;
    let current = infra_core::fs::canonicalize(&std::env::current_exe()?)?;
    let mut reader = std::fs::File::open(&current)?;
    let meta = reader.metadata()?;
    if infra_core::fs::linked(&meta) || !meta.is_file() {
        return Err(infra_core::RuntimeError::policy(
            "Current executable is not a regular file for pinned helper staging.",
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    reader.read_to_end(&mut bytes)?;
    let expected = infra_core::query::sha256(&bytes);

    let bin = dir.path().join(if cfg!(windows) {
        "infra-engine.exe"
    } else {
        "infra-engine"
    });
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o700)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut out = options.open(&bin)?;
    out.write_all(&bytes)?;
    out.sync_all()?;
    drop(out);
    if infra_core::query::sha256(&std::fs::read(&bin)?) != expected {
        return Err(infra_core::RuntimeError::policy(
            "Pinned helper staging failed integrity check.",
        ));
    }
    Ok((dir, bin))
}

fn current_exe_matches(path: &Path) -> infra_core::Result<bool> {
    let meta = std::fs::symlink_metadata(path)?;
    if infra_core::fs::linked(&meta) || !meta.is_file() {
        return Ok(false);
    }
    let current = infra_core::fs::canonicalize(&std::env::current_exe()?)?;
    let current_meta = std::fs::metadata(&current)?;
    if meta.len() != current_meta.len() {
        return Ok(false);
    }
    let expected = infra_core::query::sha256(&std::fs::read(&current)?);
    let actual = infra_core::query::sha256(&std::fs::read(path)?);
    Ok(actual == expected)
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
const VERIFY_REPAIR_ROUNDS: usize = 3;
const DIAGNOSTIC_CAP_BYTES: usize = infra_skillcache::tool::DIAGNOSTIC_BYTES;

struct AgentLoop<'a> {
    repair: Option<&'a RepairContext>,
    used_tokens: &'a AtomicU64,
    started: std::time::Instant,
}

type ExecuteReportResult =
    std::result::Result<(&'static str, usize), Box<(HarnessError, usize, Value)>>;

#[derive(Clone, Debug)]
struct VerifyFailure {
    op: String,
    repairable: bool,
    error_count: usize,
    summary: String,
    diagnostics: Vec<Value>,
}
impl VerifyFailure {
    fn into_error(self) -> HarnessError {
        if self.repairable {
            HarnessError::conflict(format!(
                "Canonical verification failed: {}. {}",
                self.op, self.summary
            ))
        } else {
            HarnessError::conflict(self.summary)
        }
    }
}

#[derive(Clone, Debug)]
struct RepairContext {
    round: usize,
    max_rounds: usize,
    op: String,
    diagnostics_summary: String,
}

fn mask_bounded(raw: &str, cap: usize) -> String {
    let masked = match infra_core::domain::mask_secrets(raw) {
        Ok((text, _)) => text,
        Err(_) => "[diagnostics withheld: a possible secret could not be masked]".into(),
    };
    let mut end = masked.len().min(cap);
    while end > 0 && !masked.is_char_boundary(end) {
        end -= 1;
    }
    masked[..end].to_owned()
}

fn parse_gate_diagnostics(op: &str, stdout: &[u8], stderr: &[u8]) -> (usize, String, Vec<Value>) {
    for raw in [stdout, stderr] {
        if raw.is_empty() {
            continue;
        }
        if let Ok(parsed) = serde_json::from_slice::<Value>(raw)
            && parsed.is_object()
        {
            if let Some(items) = parsed["diagnostics"].as_array() {
                let mut errors = Vec::new();
                let mut lines = Vec::new();
                for item in items.iter().take(25) {
                    let sev = item["severity"].as_str().unwrap_or("error");
                    let code = item["code"].as_str().unwrap_or(op);
                    let path = item["path"].as_str().filter(|p| !p.is_empty());
                    let msg = item["message"].as_str().unwrap_or("");
                    if sev == "error" {
                        errors.push(item.clone());
                    }
                    match path {
                        Some(p) => lines.push(format!("- [{sev}/{code}] ({p}) {msg}")),
                        None => lines.push(format!("- [{sev}/{code}] {msg}")),
                    }
                }
                let error_count = parsed["error_count"]
                    .as_u64()
                    .map(|n| n as usize)
                    .unwrap_or_else(|| {
                        items
                            .iter()
                            .filter(|i| i["severity"].as_str() == Some("error"))
                            .count()
                            .max(usize::from(!items.is_empty()))
                    });
                let summary = mask_bounded(&lines.join("\n"), DIAGNOSTIC_CAP_BYTES);
                return (
                    error_count.max(1),
                    summary,
                    if errors.is_empty() {
                        items.iter().take(25).cloned().collect()
                    } else {
                        errors
                    },
                );
            }
            if let Some(items) = parsed["errors"].as_array() {
                let errors: Vec<Value> = items
                    .iter()
                    .filter_map(Value::as_str)
                    .take(25)
                    .map(|msg| json!({"severity": "error", "code": op, "message": msg}))
                    .collect();
                let lines: Vec<String> = errors
                    .iter()
                    .map(|d| format!("- [error/{}] {}", op, d["message"].as_str().unwrap_or("")))
                    .collect();
                let summary = mask_bounded(&lines.join("\n"), DIAGNOSTIC_CAP_BYTES);
                return (items.len().max(1), summary, errors);
            }
        }
    }
    let fallback = [&stderr, &stdout]
        .into_iter()
        .find(|raw| !raw.is_empty())
        .map(|raw| String::from_utf8_lossy(raw).into_owned())
        .unwrap_or_default();
    let summary = mask_bounded(fallback.trim(), DIAGNOSTIC_CAP_BYTES);
    let diag = if summary.is_empty() {
        vec![]
    } else {
        vec![json!({"severity": "error", "code": op, "message": summary})]
    };
    (1, summary, diag)
}

fn format_repair_prompt(base_prompt: String, repair: Option<&RepairContext>) -> String {
    let Some(repair) = repair else {
        return base_prompt;
    };
    format!(
        "{base_prompt}\n\n## Bounded Diagnostic Repair (round {}/{})\nCanonical gate `{}` failed with the following structured diagnostics:\n{}\n\nFix ONLY the reported canonical gate violations in the workspace and ensure all required output artifacts are valid. Do not alter the approved scan profile or plan during generate, and do not modify the real target.",
        repair.round, repair.max_rounds, repair.op, repair.diagnostics_summary
    )
}

#[derive(Default)]
struct VerifiedGateCache {
    passed: BTreeMap<String, String>,
    reused_count: usize,
}

fn hash_path_entry(path: &Path) -> Value {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return json!({"exists": false});
    };
    if infra_core::fs::linked(&meta) {
        return json!({"linked": true});
    }
    if meta.is_file() {
        let sha = std::fs::read(path)
            .map(|b| infra_core::query::sha256(&b))
            .unwrap_or_default();
        return json!({"file": true, "len": meta.len(), "sha256": sha});
    }
    if meta.is_dir() {
        let mut children = BTreeMap::new();
        let mut stack = vec![path.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten().take(1024) {
                let p = entry.path();
                let Ok(m) = std::fs::symlink_metadata(&p) else {
                    continue;
                };
                if infra_core::fs::linked(&m) {
                    continue;
                }
                if m.is_dir() {
                    if children.len() < 1024 {
                        stack.push(p);
                    }
                } else if m.is_file() && children.len() < 1024 {
                    let rel = p
                        .strip_prefix(path)
                        .map(|r| r.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    let sha = std::fs::read(&p)
                        .map(|b| infra_core::query::sha256(&b))
                        .unwrap_or_default();
                    children.insert(rel, json!([m.len(), sha]));
                }
            }
        }
        return json!({"dir": children});
    }
    json!({"other": true})
}

fn gate_input_digest(op: &str, data: &Value, task: &Path, request: &Value) -> String {
    let mut parts = BTreeMap::<String, Value>::new();
    for key in [
        "plan",
        "review",
        "registry",
        "publication_plan",
        "task_dir",
        "target",
        "editions",
        "skill_plan",
        "approved_profile",
        "approved_profile_sha256",
        "profile_path",
    ] {
        let val = &data[key];
        if val.is_null() {
            continue;
        }
        if let Some(p) = val.as_str() {
            let path = Path::new(p);
            if matches!(
                key,
                "plan"
                    | "review"
                    | "registry"
                    | "publication_plan"
                    | "task_dir"
                    | "skill_plan"
                    | "approved_profile"
            ) || (key == "target" && op == "validate-generated")
            {
                parts.insert(key.into(), hash_path_entry(path));
            } else {
                parts.insert(key.into(), json!(p));
            }
        } else {
            parts.insert(key.into(), val.clone());
        }
    }
    if op == "validate-plan-mutations" {
        parts.insert(
            "approved_profile".into(),
            hash_path_entry(&task.join(files::REPORTS[0])),
        );
    }
    parts.insert("approved_hashes".into(), request["approved"].clone());
    infra_core::query::sha256(json!({"op": op, "parts": parts}).to_string().as_bytes())
}

impl CreatorRunner {
    fn verify_structured(
        &self,
        request: &Value,
        generated: bool,
        installed: bool,
        emit: &Event,
    ) -> Result<std::result::Result<(), VerifyFailure>> {
        self.verify_structured_with_cache(request, generated, installed, emit, None)
    }

    fn verify_structured_with_cache(
        &self,
        request: &Value,
        generated: bool,
        installed: bool,
        emit: &Event,
        mut cache: Option<&mut VerifiedGateCache>,
    ) -> Result<std::result::Result<(), VerifyFailure>> {
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
            #[cfg(test)]
            force_sandbox_unavailable: false,
        };
        let mut run = |op: &str,
                       mut data: Value|
         -> Result<std::result::Result<(), VerifyFailure>> {
            let input_digest = gate_input_digest(op, &data, &task, request);
            if let Some(c) = cache.as_deref_mut()
                && c.passed.get(op) == Some(&input_digest)
            {
                c.reused_count += 1;
                emit(
                    json!({"kind":"status","text":format!("Reusing verified {op} (artifacts unchanged)")}),
                )?;
                return Ok(Ok(()));
            }
            emit(json!({"kind":"status","text":format!("Checking {op}")}))?;
            if let Some(obj) = data.as_object_mut() {
                obj.insert("json".into(), Value::Bool(true));
            }
            let result = gate(op, &data, &safe_gates)?;
            for raw in [&result.stdout, &result.stderr] {
                if !raw.is_empty() {
                    emit(
                        json!({"kind":"status","text":String::from_utf8_lossy(raw).chars().take(16000).collect::<String>()}),
                    )?;
                }
            }
            if result.code != 0 {
                if let Some(c) = cache.as_deref_mut() {
                    c.passed.remove(op);
                }
                let (error_count, summary, diagnostics) =
                    parse_gate_diagnostics(op, &result.stdout, &result.stderr);
                return Ok(Err(VerifyFailure {
                    op: op.to_owned(),
                    repairable: true,
                    error_count,
                    summary,
                    diagnostics,
                }));
            }
            if let Some(c) = cache.as_deref_mut() {
                c.passed.insert(op.to_owned(), input_digest);
            }
            Ok(Ok(()))
        };
        if !generated || !installed {
            if let Err(err) = files::hashes(&directory) {
                return Ok(Err(VerifyFailure {
                    op: "required-reports".into(),
                    repairable: false,
                    error_count: 1,
                    summary: err.message.clone(),
                    diagnostics: vec![
                        json!({"severity": "error", "code": "MISSING_REQUIRED_REPORT", "message": err.message}),
                    ],
                }));
            }
            if let Err(failure) = run(
                "validate-scan-coverage",
                json!({"target":target,"task_dir":task,"plan":plan}),
            )? {
                return Ok(Err(failure));
            }
            if let Err(failure) = run(
                "validate-plan-review",
                json!({"plan":plan,"review":task.join(files::REPORTS[2]),"registry":registry}),
            )? {
                return Ok(Err(failure));
            }
            if let Err(failure) = run(
                "validate-skill-quality",
                json!({"plan":plan,"target":target,"registry":registry,"plan_only":true}),
            )? {
                return Ok(Err(failure));
            }
        }
        if generated {
            match files::hashes(&directory) {
                Ok(current) if current == request["approved"] => {}
                Ok(_) => {
                    return Ok(Err(VerifyFailure {
                        op: "approved-hashes".into(),
                        repairable: false,
                        error_count: 1,
                        summary:
                            "The reviewed profile or plan changed. Start a new scan for review."
                                .into(),
                        diagnostics: vec![
                            json!({"severity": "error", "code": "APPROVED_PROFILE_CHANGED", "message": "The reviewed profile or plan changed. Start a new scan for review."}),
                        ],
                    }));
                }
                Err(err) => {
                    return Ok(Err(VerifyFailure {
                        op: "approved-hashes".into(),
                        repairable: false,
                        error_count: 1,
                        summary: err.message.clone(),
                        diagnostics: vec![
                            json!({"severity": "error", "code": "MISSING_APPROVED_REPORT", "message": err.message}),
                        ],
                    }));
                }
            }
            if let Err(failure) = run(
                "validate-plan-mutations",
                json!({"plan":plan,"target":target,"registry":registry}),
            )? {
                return Ok(Err(failure));
            }
            if let Err(failure) = run(
                "validate-content-review",
                json!({"publication_plan":task.join(files::PLANS[0]),"review":task.join("infra-validate-review.json")}),
            )? {
                return Ok(Err(failure));
            }
            if let Err(failure) = run(
                "validate-generated",
                json!({"target":if installed{target.clone()}else{task.join("infra-generate-staging")},"editions":request["tools"],"skill_plan":plan,"evidence_target":target,"delivery_root":target,"approved_profile":task.join(files::REPORTS[0]),"approved_profile_sha256":request["approved"][files::REPORTS[0]],"profile_path":format!("{}/{}",files::TASK,files::REPORTS[0]),"registry":registry}),
            )? {
                return Ok(Err(failure));
            }
        }
        Ok(Ok(()))
    }
    fn verify(
        &self,
        request: &Value,
        generated: bool,
        installed: bool,
        emit: &Event,
    ) -> Result<()> {
        match self.verify_structured(request, generated, installed, emit)? {
            Ok(()) => Ok(()),
            Err(failure) => Err(failure.into_error()),
        }
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
        loop_state: AgentLoop<'_>,
    ) -> Result<bool> {
        let AgentLoop {
            repair,
            used_tokens,
            started,
        } = loop_state;
        let spec = sandbox_spec(request)?;
        let task = spec.workspace.join(files::TASK);
        let old = files::read(&task, "harness-questions.md", false, 1024 * 1024)?;
        if let Some(bytes) = &old {
            let mut rooted = infra_core::bank::publication::PublicationFiles::new(&task)?;
            rooted.unlink_expected_limit("harness-questions.md", bytes, 1024 * 1024)?;
        }
        let mut message = format_repair_prompt(prompt(request), repair);
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
        let max_tokens = request["budgets"]["tokens"].as_u64();
        let remaining_seconds = request["budgets"]["seconds"]
            .as_u64()
            .map(|limit| limit.saturating_sub(started.elapsed().as_secs()).max(1));
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
        let prior_tokens = used_tokens.load(Ordering::Acquire);
        let tokens = Arc::new(AtomicU64::new(prior_tokens));
        let token_limit_hit = Arc::new(AtomicBool::new(
            max_tokens.is_some_and(|limit| prior_tokens >= limit),
        ));
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
                seconds: remaining_seconds,
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
        used_tokens.store(tokens.load(Ordering::Acquire), Ordering::Release);
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
            Ok(result) if result.ok => Ok(false),
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
                Ok(true)
            }
            Err(error) => Err(error),
        }
    }
    async fn execute_with_report(
        &self,
        session: &Value,
        request: &Value,
        cancel: Arc<AtomicBool>,
        emit: Event,
    ) -> ExecuteReportResult {
        if let Err(err) = files::check_identity(request) {
            return Err(Box::new((err, 0, Value::Null)));
        }
        let phase = request["phase"].as_str().unwrap_or("");
        if phase == "rollback" {
            return match self.rollback(request, &emit) {
                Ok(()) => Ok(("rolled_back", 0)),
                Err(err) => Err(Box::new((err, 0, Value::Null))),
            };
        }
        if phase == "apply" {
            return match self.apply(request, &emit) {
                Ok(()) => Ok(("complete", 0)),
                Err(err) => Err(Box::new((err, 0, Value::Null))),
            };
        }
        if !["scan", "generate"].contains(&phase) {
            return Err(Box::new((
                HarnessError::invalid("Unknown Creator phase."),
                0,
                Value::Null,
            )));
        }
        let max_repair_rounds = request["verify_repair_rounds"]
            .as_u64()
            .map(|n| (n as usize).min(VERIFY_REPAIR_ROUNDS))
            .unwrap_or(VERIFY_REPAIR_ROUNDS);
        let directory = PathBuf::from(request["directory"].as_str().unwrap_or(""));
        let task = directory.join("agent").join(files::TASK);
        let started = std::time::Instant::now();
        let used_tokens = AtomicU64::new(0);
        let max_tokens = request["budgets"]["tokens"].as_u64();
        let max_seconds = request["budgets"]["seconds"].as_u64();
        let mut repair: Option<RepairContext> = None;
        let mut repair_rounds = 0usize;
        let mut gate_cache = VerifiedGateCache::default();
        loop {
            let output_limited = match self
                .agent(
                    session,
                    request,
                    cancel.clone(),
                    emit.clone(),
                    AgentLoop {
                        repair: repair.as_ref(),
                        used_tokens: &used_tokens,
                        started,
                    },
                )
                .await
            {
                Ok(hit) => hit,
                Err(err) => return Err(Box::new((err, repair_rounds, Value::Null))),
            };
            if cancel.load(Ordering::Acquire) {
                return Err(Box::new((
                    HarnessError::conflict("Cancellation requested."),
                    repair_rounds,
                    Value::Null,
                )));
            }
            match files::read(&task, "harness-questions.md", false, 1024 * 1024) {
                Ok(Some(v)) if v.iter().any(|b| !b.is_ascii_whitespace()) => {
                    return Ok(("needs_input", repair_rounds));
                }
                Ok(_) => {}
                Err(err) => return Err(Box::new((err, repair_rounds, Value::Null))),
            }
            match self.verify_structured_with_cache(
                request,
                phase == "generate",
                false,
                &emit,
                Some(&mut gate_cache),
            ) {
                Ok(Ok(())) => break,
                Ok(Err(failure)) => {
                    let budget_exhausted = max_tokens
                        .is_some_and(|limit| used_tokens.load(Ordering::Acquire) >= limit)
                        || max_seconds.is_some_and(|limit| started.elapsed().as_secs() >= limit);
                    if !failure.repairable
                        || output_limited
                        || budget_exhausted
                        || repair_rounds >= max_repair_rounds
                    {
                        let diagnostics = json!(failure.diagnostics);
                        return Err(Box::new((failure.into_error(), repair_rounds, diagnostics)));
                    }
                    repair_rounds += 1;
                    let _ = emit(json!({
                        "kind": "status",
                        "text": format!(
                            "Canonical gate `{}` reported {} diagnostic error(s); starting bounded repair round {}/{}.",
                            failure.op, failure.error_count, repair_rounds, max_repair_rounds
                        )
                    }));
                    repair = Some(RepairContext {
                        round: repair_rounds,
                        max_rounds: max_repair_rounds,
                        op: failure.op,
                        diagnostics_summary: failure.summary,
                    });
                }
                Err(err) => return Err(Box::new((err, repair_rounds, Value::Null))),
            }
        }
        if phase == "generate" {
            let mut run = request.clone();
            run["status"] = "preview".into();
            if let Err(err) = files::preview(&run, &directory) {
                return Err(Box::new((err, repair_rounds, Value::Null)));
            }
        }
        Ok((
            if phase == "scan" { "review" } else { "preview" },
            repair_rounds,
        ))
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
            let outcome = self
                .execute_with_report(&session, &request, cancel, emit.clone())
                .await;
            let (status, message, ok, repair_rounds, diagnostics) = match &outcome {
                Ok(("review", rounds)) => (
                    "review",
                    "Scan checks passed. Review the profile and plan before generating.".to_owned(),
                    true,
                    *rounds,
                    Value::Null,
                ),
                Ok(("preview", rounds)) => (
                    "preview",
                    "Generated bundle passed verification. Review file changes before applying."
                        .to_owned(),
                    true,
                    *rounds,
                    Value::Null,
                ),
                Ok(("needs_input", rounds)) => (
                    "needs_input",
                    "Answer the questions or provide corrections to continue.".to_owned(),
                    true,
                    *rounds,
                    Value::Null,
                ),
                Ok(("complete", rounds)) => (
                    "complete",
                    "Published and verified on the target project.".to_owned(),
                    true,
                    *rounds,
                    Value::Null,
                ),
                Ok(("rolled_back", rounds)) => (
                    "rolled_back",
                    "Publication recovered.".to_owned(),
                    true,
                    *rounds,
                    Value::Null,
                ),
                Ok(_) => return Err(HarnessError::invalid("Unexpected Creator outcome.")),
                Err(boxed) => {
                    let (error, rounds, diagnostics) = &**boxed;
                    (
                        "failed",
                        error.message.clone(),
                        false,
                        *rounds,
                        diagnostics.clone(),
                    )
                }
            };
            let mut written = json!({
                "status": status,
                "message": message,
                "repair_rounds": repair_rounds,
            });
            if !diagnostics.is_null() {
                written["diagnostics"] = diagnostics;
            }
            files::write(
                &directory,
                request["result_file"]
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid("Creator result file missing."))?,
                &written,
            )?;
            emit(json!({"kind":if ok{"result"}else{"error"},"text":message,"ok":ok}))?;
            outcome.map_err(|boxed| boxed.0)?;
            Ok(
                json!({"kind":"result","ok":ok,"text":message,"creator_status":status,"repair_rounds":repair_rounds}),
            )
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

    #[test]
    fn parse_gate_diagnostics_extracts_coded_errors_and_caps_output() {
        let json_out = json!({
            "valid": false,
            "error_count": 2,
            "diagnostics": [
                {"severity": "error", "code": "MISSING_PLAN", "message": "skill-generation-plan.json missing"},
                {"severity": "error", "code": "LEAK", "message": "bad token sk-abcdefghijklmnopqrstuvwxyz123456"}
            ]
        });
        let (count, summary, items) = parse_gate_diagnostics(
            "validate-plan-review",
            &serde_json::to_vec(&json_out).unwrap(),
            b"",
        );
        assert_eq!(count, 2);
        assert_eq!(items.len(), 2);
        assert!(summary.contains("[error/MISSING_PLAN]"));
        assert!(!summary.contains("sk-abcdefghijklmnopqrstuvwxyz123456"));
    }

    #[test]
    fn validation_gates_refuses_unlisted_runtime_arguments() {
        struct Stub;
        impl NativeGateServices for Stub {
            fn validate_host(
                &self,
                _: &Path,
                _: &Path,
                _: &str,
                _: &BTreeMap<String, String>,
            ) -> infra_core::Result<Vec<String>> {
                Ok(vec![])
            }
            fn runtime_command(
                &self,
                _: &Path,
                _: &[String],
                _: std::time::Duration,
            ) -> infra_core::Result<infra_generator::NativeOutput> {
                Ok(infra_generator::NativeOutput {
                    code: 0,
                    stdout: vec![],
                    stderr: vec![],
                })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let base = dir.path().join("base");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&base).unwrap();
        let gates = ValidationGates {
            inner: &Stub,
            base: base.clone(),
            force_sandbox_unavailable: false,
        };
        let err = gates
            .runtime_command(
                &target,
                &["--arbitrary".into()],
                std::time::Duration::from_secs(1),
            )
            .unwrap_err();
        assert!(err.to_string().contains("status --json"));
        let ok = gates
            .runtime_command(
                &target,
                &["validate".into()],
                std::time::Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(ok.code, 0);

        // Untrusted staged binary MUST be refused rather than silently falling back to `Stub`.
        let bin_rel = if cfg!(windows) {
            "memory-bank/bin/infra-engine.exe"
        } else {
            "memory-bank/bin/infra-engine"
        };
        let staged_bin = target.join(bin_rel);
        std::fs::create_dir_all(staged_bin.parent().unwrap()).unwrap();
        std::fs::write(&staged_bin, b"untrusted-binary").unwrap();
        let untrusted = gates
            .runtime_command(
                &target,
                &["validate".into()],
                std::time::Duration::from_secs(1),
            )
            .unwrap_err();
        assert!(
            untrusted.to_string().contains("pinned executable"),
            "{untrusted}"
        );

        // When staged binary matches `current_exe()` and sandbox is unavailable on Unix,
        // `ValidationGates` returns `SandboxUnavailable` instead of silently running unsandboxed.
        #[cfg(not(windows))]
        {
            std::fs::copy(std::env::current_exe().unwrap(), &staged_bin).unwrap();
            let no_sandbox_gates = ValidationGates {
                inner: &Stub,
                base,
                force_sandbox_unavailable: true,
            };
            let unavailable = no_sandbox_gates
                .runtime_command(
                    &target,
                    &["validate".into()],
                    std::time::Duration::from_secs(1),
                )
                .unwrap_err();
            assert_eq!(unavailable.code.as_deref(), Some("SandboxUnavailable"));
            assert!(unavailable.message.contains("SandboxUnavailable"));
        }
    }

    #[test]
    fn verified_gate_cache_fingerprints_artifacts_and_reuses_when_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let task = dir.path().join("task");
        let target = dir.path().join("target");
        std::fs::create_dir_all(&task).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        let plan = task.join("skill-generation-plan.json");
        let review = task.join("skill-plan-quality-report.json");
        let registry = task.join("registry.json");
        std::fs::write(&plan, b"{\"skills\":[]}").unwrap();
        std::fs::write(&review, b"{\"ok\":true}").unwrap();
        std::fs::write(&registry, b"{}").unwrap();
        let data = json!({"plan": &plan, "review": &review, "registry": &registry});
        let req = json!({"approved": {}});
        let d1 = gate_input_digest("validate-plan-review", &data, &task, &req);
        let d2 = gate_input_digest("validate-plan-review", &data, &task, &req);
        assert_eq!(d1, d2);
        std::fs::write(&review, b"{\"ok\":false}").unwrap();
        let d3 = gate_input_digest("validate-plan-review", &data, &task, &req);
        assert_ne!(d1, d3);
    }

    #[test]
    fn bounded_repair_context_is_formatted_only_when_present() {
        assert_eq!(format_repair_prompt("base".into(), None), "base");
        let repaired = format_repair_prompt(
            "base".into(),
            Some(&RepairContext {
                round: 1,
                max_rounds: 3,
                op: "validate-plan-review".into(),
                diagnostics_summary: "- [error/CODE] fix me".into(),
            }),
        );
        assert!(repaired.contains("round 1/3"));
        assert!(repaired.contains("validate-plan-review"));
        assert!(repaired.contains("- [error/CODE] fix me"));
    }
}
