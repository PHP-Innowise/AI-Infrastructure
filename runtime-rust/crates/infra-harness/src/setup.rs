//! One-use reviewed installer plans; apply consumes the exact native receipt.
use crate::{
    HarnessError, Result, creator_files, projects,
    services::{Domain, Payload, Reply},
    sessions::Sessions,
    store::{self, SessionStore},
};
use infra_core::bank::publication::{PublicationFiles, commit::POST_RENAME_IDENTITY_UNAVAILABLE};
use infra_tooling::installer::{InstallOptions, InstallPlan, Installer};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};
const EDITIONS: [&str; 4] = ["Laravel", "Symfony", "PHP Core", "Cms/wordpress"];
struct Preview {
    at: Instant,
    project: String,
    plan: InstallPlan,
    reply: Value,
}
pub struct Setup {
    pub store: SessionStore,
    pub source: PathBuf,
    sessions: Weak<Sessions>,
    accelerators: std::sync::Arc<crate::accelerators::Accelerators>,
    previews: Mutex<BTreeMap<String, Preview>>,
    serial: Mutex<()>,
}
struct PreviewInput<'a> {
    project: &'a str,
    edition: &'a str,
    tools: Vec<String>,
}
fn preview_input(data: &Value) -> Result<PreviewInput<'_>> {
    if !store::exact(data, &["project_id", "edition", "tools"], &[]) {
        return Err(HarnessError::invalid(
            "Select a project, edition and tools.",
        ));
    }
    let project = data["project_id"]
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Select a project."))?;
    let edition = data["edition"]
        .as_str()
        .filter(|e| EDITIONS.contains(e))
        .ok_or_else(|| HarnessError::invalid("Select a supported edition."))?;
    let tools = creator_files::texts(&data["tools"])?;
    if tools.is_empty()
        || tools.len() > 3
        || tools
            .iter()
            .any(|t| !["claude", "codex", "cursor"].contains(&t.as_str()))
        || tools.iter().collect::<BTreeSet<_>>().len() != tools.len()
    {
        return Err(HarnessError::invalid("Select distinct supported tools."));
    }
    Ok(PreviewInput {
        project,
        edition,
        tools,
    })
}
fn install_keys(data: &Value) -> Result<()> {
    if !store::exact(data, &["preview_id"], &[]) {
        return Err(HarnessError::invalid(
            "Install using the reviewed preview ID.",
        ));
    }
    Ok(())
}

/// Bounded native offload; public synchronous Setup callers retain their locks.
pub struct SetupDomain {
    setup: Arc<Setup>,
    lifecycle: Arc<tokio::sync::Mutex<bool>>,
    mutation: Arc<tokio::sync::Mutex<()>>,
}
impl SetupDomain {
    pub fn new(setup: Arc<Setup>) -> Self {
        Self {
            setup,
            lifecycle: Arc::new(tokio::sync::Mutex::new(false)),
            mutation: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
    pub async fn close(&self) {
        // FIFO admission drains previously queued work, including detached
        // blocking closures, before rejecting later callers.
        *self.lifecycle.lock().await = true;
    }
}
impl Domain for SetupDomain {
    fn execute<'a>(&'a self, op: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let mutating = match op {
                "setup.status" => false,
                "setup.preview" => {
                    preview_input(&request["body"])?;
                    true
                }
                "setup.install" => {
                    // The ID's type and one-use receipt are deliberately
                    // checked only after the original serial lock is acquired.
                    install_keys(&request["body"])?;
                    true
                }
                _ => return Err(HarnessError::missing("Unknown setup operation.")),
            };
            let reservation = if mutating {
                Some(
                    self.mutation
                        .clone()
                        .try_lock_owned()
                        .map_err(|_| HarnessError::conflict("Accelerator setup is busy."))?,
                )
            } else {
                None
            };
            let runtime = match tokio::runtime::Handle::try_current() {
                Ok(runtime) => runtime,
                Err(_) => {
                    let closed = self.lifecycle.blocking_lock();
                    if *closed {
                        drop(reservation);
                        return Err(HarnessError::conflict("Accelerator setup is closed."));
                    }
                    let reply = self.setup.execute_value(op, &request).map(Payload::Json);
                    drop(reservation);
                    drop(closed);
                    return reply;
                }
            };
            // One blocking job at a time. Status also queues here, so an active
            // installation cannot accumulate blocking status workers.
            let closed = self.lifecycle.clone().lock_owned().await;
            if *closed {
                drop(reservation);
                return Err(HarnessError::conflict("Accelerator setup is closed."));
            }
            let setup = self.setup.clone();
            let op = op.to_owned();
            runtime
                .spawn_blocking(move || {
                    // Request cancellation must not release custody while the
                    // installer is still publishing or rolling back files.
                    let _closed = closed;
                    let _reservation = reservation;
                    setup.execute_value(&op, &request).map(Payload::Json)
                })
                .await
                .map_err(|_| HarnessError::unavailable("Setup operation worker stopped."))?
        })
    }
}
fn installation_failure(
    error: &infra_core::RuntimeError,
    has_confirmed_effects: bool,
) -> Result<Option<Value>> {
    if error.code.as_deref() == Some(POST_RENAME_IDENTITY_UNAVAILABLE) {
        return Ok(Some(json!({
            "code": POST_RENAME_IDENTITY_UNAVAILABLE,
            "os_code": error.os_code,
            "recovery_required": true,
            "error": error.message,
            "summary": "Installation stopped after a committed publication whose final identity could not be confirmed. Preserve the target for recovery; do not retry until it has been inspected."
        })));
    }
    if !has_confirmed_effects {
        return Err(HarnessError::from(infra_core::RuntimeError::conflict(
            format!(
                "Installation did not complete; reviewed effects were rolled back or no files were written. Preview again. {error}"
            ),
        )));
    }
    Ok(None)
}
impl Setup {
    pub fn new(
        store: SessionStore,
        source: PathBuf,
        sessions: Weak<Sessions>,
        accelerators: std::sync::Arc<crate::accelerators::Accelerators>,
    ) -> Self {
        Self {
            store,
            source,
            sessions,
            accelerators,
            previews: Mutex::new(BTreeMap::new()),
            serial: Mutex::new(()),
        }
    }
    fn runner(&self) -> Result<std::sync::Arc<Sessions>> {
        self.sessions
            .upgrade()
            .ok_or_else(|| HarnessError::unavailable("The runner is stopping."))
    }
    fn target(&self, project: &str) -> Result<PathBuf> {
        let value = self.store.project(project)?;
        let root = std::fs::canonicalize(
            value["path"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Project path missing."))?,
        )?;
        PublicationFiles::new(&root)?;
        for boundary in [&self.source, &self.store.state] {
            if root.starts_with(boundary) || boundary.starts_with(&root) {
                return Err(HarnessError::forbidden(
                    "Install into a separate project outside the accelerator source and Harness state.",
                ));
            }
        }
        Ok(root)
    }
    pub fn preview(&self, data: &Value) -> Result<Value> {
        let PreviewInput {
            project,
            edition,
            tools,
        } = preview_input(data)?;
        let _serial = self
            .serial
            .try_lock()
            .map_err(|_| HarnessError::conflict("Accelerator setup is busy."))?;
        let target = self.target(project)?;
        let installer = Installer::new(&self.source)?;
        let options = InstallOptions {
            merge_existing: true,
            ..Default::default()
        };
        let plan = installer.plan(&target, edition, &tools, &options)?;
        let images = installer.preview_images(&plan, 16384)?;
        if images.len() > 5000 {
            return Err(HarnessError::invalid(
                "The selected accelerator exceeds 5000 files.",
            ));
        }
        let mut total = 0usize;
        let mut files = vec![];
        let mut diff_budget = 256000usize;
        for image in images {
            total = total
                .saturating_add(image.before_bytes as usize)
                .saturating_add(image.after_bytes as usize);
            if total > 64 * 1024 * 1024 {
                return Err(HarnessError::invalid(
                    "The selected source and project images exceed 64 MiB.",
                ));
            }
            let (change, truncated) = if image.before_text.is_some() || image.after_text.is_some() {
                creator_files::diff(
                    image.before_text.as_deref().unwrap_or("").as_bytes(),
                    image.after_text.as_deref().unwrap_or("").as_bytes(),
                    &image.path,
                    diff_budget,
                )
            } else {
                (image.reason.clone().unwrap_or_else(||"Binary or oversized image; exact hashes are retained in the reviewed receipt.".into()),true)
            };
            let truncated = truncated || image.diff_truncated;
            diff_budget = diff_budget.saturating_sub(change.len());
            files.push(json!({"path":image.path,"source_path":image.source_path,"action":image.action,"before_bytes":image.before_bytes,"after_bytes":image.after_bytes,"reason":image.reason,"diff":change,"diff_truncated":truncated}));
        }
        let report = plan.report("planned", true);
        let collisions = report["collisions"].as_array().cloned().unwrap_or_default();
        let changed = files
            .iter()
            .filter(|f| !["unchanged", "kept"].contains(&f["action"].as_str().unwrap_or("")))
            .count();
        let id = store::hexid();
        let reply = json!({"preview_id":id,"project_id":project,"edition":edition,"tools":tools,"expires_at":chrono::Utc::now().timestamp()+900,"files":files,"file_count":files.len(),"changed_count":changed,"collisions":collisions,"can_install":collisions.is_empty(),"summary":format!("{} files; {changed} changes; {} collisions. {}",files.len(),collisions.len(),if collisions.is_empty(){"Review the changes before installing."}else{"Resolve collisions and preview again."})});
        let mut cache = self
            .previews
            .lock()
            .map_err(|_| HarnessError::unavailable("Setup preview cache unavailable."))?;
        cache.retain(|_, v| v.at.elapsed() < Duration::from_secs(900));
        while cache.len() >= 8 {
            let oldest = cache
                .iter()
                .min_by_key(|(_, v)| v.at)
                .map(|(k, _)| k.clone())
                .unwrap();
            cache.remove(&oldest);
        }
        cache.insert(
            id,
            Preview {
                at: Instant::now(),
                project: project.into(),
                plan,
                reply: reply.clone(),
            },
        );
        Ok(reply)
    }
    pub fn install(&self, data: &Value) -> Result<Value> {
        install_keys(data)?;
        let _serial = self
            .serial
            .try_lock()
            .map_err(|_| HarnessError::conflict("Accelerator setup is busy."))?;
        let id = data["preview_id"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Invalid setup preview."))?;
        let preview = self
            .previews
            .lock()
            .map_err(|_| HarnessError::unavailable("Setup preview cache unavailable."))?
            .remove(id)
            .ok_or_else(|| {
                HarnessError::conflict("The preview expired or was already used. Preview again.")
            })?;
        if preview.at.elapsed() >= Duration::from_secs(900) || preview.reply["can_install"] != true
        {
            return Err(HarnessError::conflict(
                "Resolve collisions or preview again.",
            ));
        }
        let runner = self.runner()?;
        let operation = runner.operation_lock();
        let _operation = operation
            .try_lock()
            .map_err(|_| HarnessError::conflict("Another workspace operation is completing."))?;
        runner.idle()?;
        let target = self.target(&preview.project)?;
        let serialized = serde_json::to_value(&preview.plan)?;
        if serialized["target"].as_str() != target.to_str() {
            return Err(HarnessError::conflict(
                "The registered project changed. Preview again.",
            ));
        }
        let mut confirmed = BTreeMap::<String, String>::new();
        let mut rollback_failed = BTreeSet::<String>::new();
        let shutdown = runner.shutdown_cancel();
        let result =
            Installer::new(&self.source)?.apply_plan_observed(&preview.plan, |path, action| {
                match action {
                    "rolled-back" => {
                        confirmed.remove(path);
                        rollback_failed.remove(path);
                    }
                    "rollback-failed" => {
                        rollback_failed.insert(path.to_owned());
                    }
                    _ => {
                        confirmed.insert(path.to_owned(), action.to_owned());
                        if shutdown.load(std::sync::atomic::Ordering::Acquire) {
                            return Err(infra_core::RuntimeError::conflict(
                                "Accelerator setup stopped during installation",
                            ));
                        }
                    }
                }
                Ok(())
            });
        let recovery = match &result {
            Err(error) => {
                installation_failure(error, !confirmed.is_empty() || !rollback_failed.is_empty())?
            }
            Ok(_) => None,
        };
        let mut installed = Vec::new();
        let mut merged = Vec::new();
        let mut repaired = Vec::new();
        let mut unchanged = Vec::new();
        for entry in preview.reply["files"].as_array().into_iter().flatten() {
            let Some(path) = entry["path"].as_str() else {
                continue;
            };
            let action = entry["action"].as_str().unwrap_or("");
            if ["unchanged", "kept"].contains(&action) {
                unchanged.push(path.to_owned());
            }
        }
        for (path, action) in &confirmed {
            match action.as_str() {
                "merge" => merged.push(path.clone()),
                "fix-mode" => repaired.push(path.clone()),
                _ => installed.push(path.clone()),
            }
        }
        let verification = (|| -> Result<()> {
            let output = result
                .as_ref()
                .map_err(|_| HarnessError::conflict("Installation stopped during application."))?;
            if output["status"] != "complete" {
                return Err(HarnessError::conflict(
                    "The reviewed installation did not complete.",
                ));
            }
            let mut rooted = PublicationFiles::new(&target)?;
            let mut images = json!({});
            for item in &preview.plan.files {
                let expected = match &item.payload {
                    infra_tooling::installer::PlannedPayload::Inline { sha256, .. } => sha256,
                    infra_tooling::installer::PlannedPayload::Source { image } => &image.sha256,
                };
                let bytes = rooted.read(&item.path, 64 * 1024 * 1024)?.ok_or_else(|| {
                    HarnessError::conflict("A reviewed installed file is missing.")
                })?;
                let mode = rooted.read_mode(&item.path)?;
                if store::sha(&bytes) != *expected || mode != Some(item.mode) {
                    return Err(HarnessError::conflict(
                        "Installed bytes or modes changed before Setup verification.",
                    ));
                }
                images[&item.path] = json!({"hash":expected,"mode":mode,"bytes":bytes.len()});
            }
            rooted.check()?;
            if shutdown.load(std::sync::atomic::Ordering::Acquire) {
                return Err(HarnessError::conflict(
                    "Accelerator setup stopped before verification completed.",
                ));
            }
            let receipt = json!({"root":target,"identity":rooted.root_identity(),"edition":preview.reply["edition"],"tools":preview.reply["tools"],"files":images,"installed_at":store::now(),"plan":serialized["sha256"]});
            self.store
                .set_setting(&format!("setup.{}", preview.project), &receipt)?;
            Ok(())
        })();
        let ok = verification.is_ok();
        let mut out = json!({"ok":ok,"project_id":preview.project,"installed":installed,"merged":merged,"unchanged":unchanged,"repaired":repaired,"summary":if ok{format!("{} files installed, {} merged, {} made executable, {} unchanged. Reviewed payload hashes and modes verified.",installed.len(),merged.len(),repaired.len(),unchanged.len())}else{format!("{} confirmed files added or replaced and {} merged; installation requires a fresh review.",installed.len(),merged.len())}});
        if !ok {
            out["error"] =
                "Installation stopped and may be partial. Refresh the project and preview again."
                    .into();
            out["rollback_failed"] = json!(rollback_failed);
        }
        if let Some(recovery) = recovery {
            for field in ["code", "os_code", "recovery_required", "error", "summary"] {
                out[field] = recovery[field].clone();
            }
        }
        Ok(out)
    }
    pub fn status(&self, project_id: &str) -> Result<Value> {
        let project = self.store.project(project_id)?;
        let mut editions = vec![];
        let installer = Installer::new(&self.source)?;
        for edition in EDITIONS {
            let release = installer
                .inventory(edition, &["claude".into(), "codex".into(), "cursor".into()])
                .ok()
                .map(|v| v.release);
            editions.push(json!({"id":edition,"name":edition,"release":release}));
        }
        let mut output = json!({"project_id":project_id,"path":project["path"],"available":false,"editions":editions,"tools":[{"id":"claude","name":"Claude Code","installed":false},{"id":"codex","name":"Codex","installed":false},{"id":"cursor","name":"Cursor","installed":false}],"installed_edition":null,"readiness":{"policy":false,"memory_runtime":false,"brain_runtime":false},"payload_verified":false,"git":{"is_git":false},"providers":self.runner()?.providers.listing()?,"scope":["A found CLI may not be signed in; authentication is not checked."],"diagnostics":[],"accelerator":self.accelerators.get(project_id)?});
        let root = PathBuf::from(project["path"].as_str().unwrap_or(""));
        let inspect = (|| -> Result<()> {
            let mut files = PublicationFiles::new(&root)?;
            output["available"] = true.into();
            let present = |files: &mut PublicationFiles, name: &str| -> Result<bool> {
                Ok(files.read_prefix(name, 0)?.is_some_and(|(n, _)| n > 0))
            };
            output["readiness"]["policy"] = present(&mut files, "AGENTS.md")?.into();
            let engine = if cfg!(windows) {
                "memory-bank/bin/infra-engine.exe"
            } else {
                "memory-bank/bin/infra-engine"
            };
            output["readiness"]["memory_runtime"] = present(&mut files, engine)?.into();
            let config = files
                .read("project-brain/config/runtime.json", 1024 * 1024)?
                .map(|b| crate::strict_json::parse(&b, 1024 * 1024))
                .transpose()?;
            output["readiness"]["brain_runtime"] = (output["readiness"]["memory_runtime"] == true
                && config.is_some_and(|v| {
                    ["governed", "legacy"].contains(&v["mode"].as_str().unwrap_or("governed"))
                })
                && present(&mut files, "project-brain/PROTOCOL.md")?)
            .into();
            for (tool, names) in [
                (0, vec![".claude/settings.json"]),
                (
                    1,
                    vec![".codex/config.toml", ".agents/skills/SKILL FLOW.md"],
                ),
                (
                    2,
                    vec![".cursor/hooks.json", ".cursor/rules/accelerator.mdc"],
                ),
            ] {
                let mut exists = false;
                for name in names {
                    exists |= present(&mut files, name)?;
                }
                output["tools"][tool]["installed"] = exists.into();
            }
            if let Some(receipt) = self.store.setting(&format!("setup.{project_id}"))?
                && receipt["root"] == project["path"]
                && receipt["identity"] == serde_json::to_value(files.root_identity())?
            {
                output["installed_edition"] = receipt["edition"].clone();
                let mut verified = receipt["files"]
                    .as_object()
                    .is_some_and(|m| !m.is_empty() && m.len() <= 5000);
                if let Some(images) = receipt["files"].as_object() {
                    for (name, expected) in images {
                        let actual = files.read(name, 64 * 1024 * 1024)?;
                        verified &= actual.is_some_and(|b| expected["hash"] == store::sha(&b))
                            && files.read_mode(name)?.map(|m| json!(m))
                                == Some(expected["mode"].clone());
                    }
                }
                output["payload_verified"] = verified.into();
                if !verified {
                    output["diagnostics"].as_array_mut().unwrap().push(json!(
                        "Installed files differ from the last successful Setup receipt."
                    ));
                }
            }
            output["git"] = projects::details(&root, false)?;
            output["scope"].as_array_mut().unwrap().extend([
                json!("Git branch only; the working tree was not inspected."),
                json!("Static file check; no project scripts or hooks ran."),
            ]);
            Ok(())
        })();
        if let Err(error) = inspect {
            output["diagnostics"]
                .as_array_mut()
                .unwrap()
                .push(json!(format!(
                    "Project setup metadata is unavailable: {}",
                    error.message
                )));
        }
        Ok(output)
    }
    fn execute_value(&self, op: &str, request: &Value) -> Result<Value> {
        match op {
            "setup.status" => self.status(request["params"]["project_id"].as_str().unwrap_or("")),
            "setup.preview" => self.preview(&request["body"]),
            "setup.install" => self.install(&request["body"]),
            _ => Err(HarnessError::missing("Unknown setup operation.")),
        }
    }
}
impl Domain for Setup {
    fn execute<'a>(&'a self, op: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move { self.execute_value(op, &request).map(Payload::Json) })
    }
}

#[cfg(test)]
mod installation_failure_tests {
    use super::*;

    #[test]
    fn unreceipted_publication_keeps_recovery_evidence_even_without_observed_effects() {
        let original = "A publication was committed but has no final identity receipt; preserve the target for recovery.";
        for has_confirmed_effects in [false, true] {
            for os_code in [None, Some(5)] {
                let mut error = infra_core::RuntimeError::coded(
                    infra_core::ErrorKind::Conflict,
                    POST_RENAME_IDENTITY_UNAVAILABLE,
                    original,
                );
                error.os_code = os_code;
                let reply = installation_failure(&error, has_confirmed_effects)
                    .expect("recovery response")
                    .expect("explicit recovery evidence");
                assert_eq!(reply["code"], POST_RENAME_IDENTITY_UNAVAILABLE);
                assert_eq!(reply["os_code"], json!(os_code));
                assert_eq!(reply["error"], original);
                assert_eq!(reply["recovery_required"], true);
                let summary = reply["summary"].as_str().unwrap();
                assert!(summary.contains("committed publication"));
                assert!(summary.contains("Preserve the target for recovery"));
                assert!(!summary.contains("rolled back"));
                assert!(!summary.contains("no files were written"));
                assert!(
                    reply.get("path").is_none(),
                    "no unconfirmed path is invented"
                );
                let wire = serde_json::to_string(&reply).unwrap();
                let decoded: Value = serde_json::from_str(&wire).unwrap();
                assert_eq!(decoded, reply, "typed evidence survives the JSON boundary");
            }
        }
    }

    #[test]
    fn ordinary_no_effect_failure_keeps_existing_conflict_diagnostic() {
        let error = infra_core::RuntimeError::conflict("Reviewed source changed");
        let reply = installation_failure(&error, false).expect_err("ordinary failure");
        assert_eq!(reply.status, 409);
        assert_eq!(reply.code, "runtime-refused");
        assert_eq!(
            reply.message,
            "Installation did not complete; reviewed effects were rolled back or no files were written. Preview again. Reviewed source changed"
        );
    }

    #[test]
    fn ordinary_partial_failure_keeps_existing_partial_response_path() {
        let error = infra_core::RuntimeError::conflict("A confirmed rollback requires inspection");
        assert!(installation_failure(&error, true).unwrap().is_none());
    }
}
