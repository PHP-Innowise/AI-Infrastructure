//! Clone fast-forward updates with an admission hold and verified native handoff.
use crate::{
    HarnessError, Result, projects,
    services::{Domain, Payload, Reply},
    sessions::Sessions,
    store,
};
use infra_core::bank::publication::PublicationFiles;
use infra_tooling::{
    artifacts::{Artifact, native_target},
    installer::Installer,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
const HOLD: &str =
    "The Harness is updating and then restarts. Start new work once the page has reloaded.";
pub trait Restart: Send + Sync {
    fn refresh_application(&self, engine: &Path, expected_sha256: &str) -> Result<Vec<String>>;
    fn handoff(&self, engine: &Path, report: &Value) -> Result<bool>;
}
pub struct Updates {
    root: PathBuf,
    sessions: Weak<Sessions>,
    restart: Arc<dyn Restart>,
    state: Mutex<Value>,
    checked: Mutex<Option<Instant>>,
    serial: Mutex<()>,
    applying: tokio::sync::Mutex<()>,
    held: AtomicBool,
    stopping: AtomicBool,
    me: Weak<Self>,
}
impl Updates {
    pub fn new(root: PathBuf, sessions: Weak<Sessions>, restart: Arc<dyn Restart>) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            root,
            sessions,
            restart,
            state: Mutex::new(
                json!({"state":"unknown","detail":"","checked_at":null,"behind":0,"ahead":0,"commits":[],"branch":null,"upstream":null,"version":null}),
            ),
            checked: Mutex::new(None),
            serial: Mutex::new(()),
            applying: tokio::sync::Mutex::new(()),
            held: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            me: me.clone(),
        })
    }
    fn runner(&self) -> Result<Arc<Sessions>> {
        self.sessions
            .upgrade()
            .ok_or_else(|| HarnessError::unavailable("The session runner is stopping."))
    }
    fn git(&self, args: &[&str], seconds: u64) -> Result<infra_core::git::GitOutput> {
        projects::git(
            &self.root,
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            seconds,
        )
    }
    fn value(&self, args: &[&str]) -> Result<Option<String>> {
        let value = self.git(args, 30)?;
        Ok((value.code == 0).then(|| {
            infra_core::domain::rstrip(&String::from_utf8_lossy(&value.stdout)).to_owned()
        }))
    }
    fn reason(result: &infra_core::git::GitOutput) -> String {
        format!("Git exited with status {}.", result.code)
    }
    fn compute(&self, fetch: bool) -> Value {
        let mut status = json!({"state":"failed","detail":"","checked_at":store::now(),"behind":0,"ahead":0,"commits":[],"branch":null,"upstream":null,"version":null});
        let result = (|| -> Result<Value> {
            if self
                .value(&["rev-parse", "--is-inside-work-tree"])?
                .as_deref()
                != Some("true")
            {
                status["state"] = "unavailable".into();
                status["detail"] = "This copy is not a Git clone and cannot update itself.".into();
                return Ok(status.clone());
            }
            let branch = self.value(&["symbolic-ref", "--quiet", "--short", "HEAD"])?;
            status["branch"] = json!(branch);
            status["version"] = json!(self.value(&["rev-parse", "--short", "HEAD"])?);
            let Some(branch) = branch else {
                status["state"] = "unavailable".into();
                status["detail"] = "This clone is not on a branch.".into();
                return Ok(status.clone());
            };
            let remote = self.value(&["config", "--get", &format!("branch.{branch}.remote")])?;
            let merge = self.value(&["config", "--get", &format!("branch.{branch}.merge")])?;
            let (remote, merge) = match (remote, merge) {
                (Some(remote), Some(merge))
                    if remote != "." && merge.starts_with("refs/heads/") =>
                {
                    (remote, merge)
                }
                _ if branch == "main"
                    && self.value(&["remote", "get-url", "origin"])?.is_some() =>
                {
                    ("origin".into(), "refs/heads/main".into())
                }
                _ => {
                    status["state"] = "unavailable".into();
                    status["detail"] =
                        format!("The branch {branch} follows no remote branch.").into();
                    return Ok(status.clone());
                }
            };
            let upstream = format!("{remote}/{}", &merge["refs/heads/".len()..]);
            status["upstream"] = upstream.clone().into();
            if fetch {
                let result = self.git(
                    &["fetch", "--quiet", "--no-tags", "--", &remote, &merge],
                    60,
                )?;
                if result.code != 0 {
                    status["detail"] =
                        format!("Could not check {upstream}: {}", Self::reason(&result)).into();
                    return Ok(status.clone());
                }
            }
            let counts = self
                .value(&[
                    "rev-list",
                    "--left-right",
                    "--count",
                    &format!("HEAD...refs/remotes/{upstream}"),
                ])?
                .ok_or_else(|| {
                    HarnessError::unavailable("The upstream branch is not known in this clone.")
                })?;
            let values = counts
                .split_whitespace()
                .map(str::parse::<u64>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| {
                    HarnessError::unavailable("Git returned invalid divergence counts.")
                })?;
            if values.len() != 2 {
                return Err(HarnessError::unavailable(
                    "Git returned invalid divergence counts.",
                ));
            }
            let (ahead, behind) = (values[0], values[1]);
            status["ahead"] = ahead.into();
            status["behind"] = behind.into();
            let log = if behind > 0 {
                self.value(&[
                    "log",
                    "-20",
                    "--format=%h%x09%s",
                    &format!("HEAD..refs/remotes/{upstream}"),
                ])?
                .unwrap_or_default()
            } else {
                String::new()
            };
            status["commits"] = log
                .lines()
                .filter_map(|line| line.split_once('\t'))
                .map(|(hash, subject)| json!({"hash":hash,"subject":subject}))
                .collect::<Vec<_>>()
                .into();
            status["state"] = if behind == 0 {
                "current"
            } else if ahead > 0 {
                "diverged"
            } else {
                "available"
            }
            .into();
            status["detail"]=if behind==0{String::new()}else if ahead>0{format!("{behind} new changes on {upstream}; this clone has {ahead} commits of its own. Update it with Git.")}else{format!("{behind} new changes on {upstream}.")}.into();
            Ok(status.clone())
        })();
        match result {
            Ok(value) => value,
            Err(error) => {
                status["detail"] = error.message.into();
                status
            }
        }
    }
    pub fn status(&self) -> Result<Value> {
        Ok(self
            .state
            .lock()
            .map_err(|_| HarnessError::unavailable("Update state unavailable."))?
            .clone())
    }
    fn check(&self, fetch: bool) -> Result<Value> {
        let _serial = self
            .serial
            .lock()
            .map_err(|_| HarnessError::unavailable("Update state unavailable."))?;
        if self.stopping.load(Ordering::Acquire) {
            return self.status();
        }
        let value = self.compute(fetch);
        let mut current = self
            .state
            .lock()
            .map_err(|_| HarnessError::unavailable("Update state unavailable."))?;
        if current["state"] != "updating" && !self.held.load(Ordering::Acquire) {
            *current = value;
        }
        *self
            .checked
            .lock()
            .map_err(|_| HarnessError::unavailable("Update clock unavailable."))? =
            Some(Instant::now());
        Ok(current.clone())
    }
    pub fn poke(&self) -> Result<Value> {
        let updating = self.status()?["state"] == "updating" || self.held.load(Ordering::Acquire);
        let mut checked = self
            .checked
            .lock()
            .map_err(|_| HarnessError::unavailable("Update clock unavailable."))?;
        let stale = checked.is_none_or(|at| at.elapsed() > Duration::from_secs(600)) && !updating;
        if stale {
            *checked = Some(Instant::now());
            if let Some(me) = self.me.upgrade() {
                tokio::task::spawn_blocking(move || {
                    let _ = me.check(true);
                });
            }
        }
        drop(checked);
        self.status()
    }
    pub fn start(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut delay = 15u64;
            loop {
                tokio::time::sleep(Duration::from_secs(delay)).await;
                let Some(me) = weak.upgrade() else { break };
                if me.stopping.load(Ordering::Acquire) {
                    break;
                }
                let _ = tokio::task::spawn_blocking(move || me.check(true)).await;
                delay = 3600;
            }
        });
    }
    pub fn close(&self) {
        self.stopping.store(true, Ordering::Release);
    }
    pub fn resume(&self) -> Result<()> {
        if self.held.swap(false, Ordering::AcqRel) {
            self.runner()?.release()?;
        }
        Ok(())
    }
    fn native_engine(&self) -> Result<(PathBuf, String)> {
        let target = native_target()?;
        let required = Installer::new(&self.root)?.engine_capabilities()?;
        let required = required["capabilities"]
            .as_array()
            .ok_or_else(|| HarnessError::invalid("Current native capabilities are unavailable."))?;
        let mut files = PublicationFiles::new(&self.root)?;
        for edition in ["PHP Core", "Laravel", "Symfony", "Cms/wordpress"] {
            let name = format!("{edition}/memory-bank/runtime-capabilities.json");
            let Some(raw) = files.read(&name, 1024 * 1024)? else {
                continue;
            };
            let meta = crate::strict_json::parse(&raw, 1024 * 1024)?;
            if meta["engine"] != "infra-engine"
                || meta["target"] != target
                || !required.iter().all(|v| {
                    meta["capabilities"]
                        .as_array()
                        .is_some_and(|a| a.contains(v))
                })
            {
                continue;
            }
            let artifact = Artifact {
                target: target.clone(),
                path: meta["path"]
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid("Release artifact path missing."))?
                    .into(),
                sha256: meta["sha256"]
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid("Release artifact digest missing."))?
                    .into(),
                bytes: meta["bytes"]
                    .as_u64()
                    .ok_or_else(|| HarnessError::invalid("Release artifact size missing."))?,
                executable: true,
                capabilities: meta["capabilities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            };
            return Ok((
                artifact.verify(&self.root.join(edition))?,
                artifact.sha256.clone(),
            ));
        }
        Err(HarnessError::unavailable(
            "The updated clone has no verified native engine for this platform and capability set. New runs remain held until a compatible engine is installed and the server restarts.",
        ))
    }
    pub async fn apply(&self) -> Result<Value> {
        let _apply = self
            .applying
            .try_lock()
            .map_err(|_| HarnessError::conflict("The update is already running."))?;
        if self.held.load(Ordering::Acquire) {
            return Err(HarnessError::conflict(
                "An update is waiting for the Harness to restart.",
            ));
        }
        self.runner()?.hold(HOLD)?;
        self.held.store(true, Ordering::Release);
        let result = (|| -> Result<Value> {
            let _serial = self
                .serial
                .lock()
                .map_err(|_| HarnessError::unavailable("Update state unavailable."))?;
            let status = self.compute(true);
            if status["state"] == "current" {
                *self
                    .state
                    .lock()
                    .map_err(|_| HarnessError::unavailable("Update state unavailable."))? = status;
                return Ok(
                    json!({"ok":true,"current":true,"restarting":false,"detail":"This clone is already up to date."}),
                );
            }
            if status["state"] != "available" {
                *self
                    .state
                    .lock()
                    .map_err(|_| HarnessError::unavailable("Update state unavailable."))? =
                    status.clone();
                return Err(HarnessError::conflict(
                    status["detail"]
                        .as_str()
                        .unwrap_or("There is no update to apply."),
                ));
            }
            let mut updating = status.clone();
            updating["state"] = "updating".into();
            updating["detail"] = "Updating…".into();
            *self
                .state
                .lock()
                .map_err(|_| HarnessError::unavailable("Update state unavailable."))? = updating;
            let upstream = status["upstream"].as_str().unwrap_or("");
            let merged = self.git(
                &[
                    "merge",
                    "--ff-only",
                    "--quiet",
                    &format!("refs/remotes/{upstream}"),
                ],
                120,
            )?;
            if merged.code != 0 {
                return Err(HarnessError::conflict(format!(
                    "The update could not be applied: {}",
                    Self::reason(&merged)
                )));
            }
            let version = self.value(&["rev-parse", "--short", "HEAD"])?;
            let mut updated = status.clone();
            updated["state"] = "updated".into();
            updated["source_updated"] = true.into();
            updated["version"] = json!(version);
            updated["behind"] = 0.into();
            updated["commits"] = json!([]);
            updated["detail"] = "The clone updated; preparing the verified native restart.".into();
            *self
                .state
                .lock()
                .map_err(|_| HarnessError::unavailable("Update state unavailable."))? = updated;
            let (engine, digest) = self.native_engine()?;
            let mut report = json!({"ok":true,"from":status["version"],"to":version,"changes":status["behind"],"restarting":false,"engine":engine,"engine_sha256":digest});
            report["application"] = match self.restart.refresh_application(&engine, &digest) {
                Ok(paths) => json!(paths),
                Err(error) => json!([format!(
                    "The application entry was not refreshed: {}",
                    error.message
                )]),
            };
            report["restarting"] = self.restart.handoff(&engine, &report)?.into();
            if report["restarting"] != true {
                self.resume()?;
            }
            Ok(report)
        })();
        match &result {
            Ok(value) if value["current"] == true => {
                self.resume()?;
            }
            Err(error) => {
                let source_changed = self.status()?["source_updated"] == true;
                let mut failed = self.status()?;
                failed["state"] = if source_changed {
                    "restart-required"
                } else {
                    "failed"
                }
                .into();
                failed["detail"] = error.message.clone().into();
                *self
                    .state
                    .lock()
                    .map_err(|_| HarnessError::unavailable("Update state unavailable."))? = failed;
                if !source_changed {
                    self.resume()?;
                }
            }
            _ => {}
        }
        result
    }
}
impl Domain for Updates {
    fn execute<'a>(&'a self, op: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let value = match op {
                "updates.status" => {
                    if request["query"]["check"].is_null() {
                        self.status()?
                    } else if request["query"]["check"] == "1" {
                        self.poke()?
                    } else {
                        return Err(HarnessError::invalid("Invalid update request."));
                    }
                }
                "updates.apply" => {
                    if !store::exact(&request["body"], &[], &[]) {
                        return Err(HarnessError::invalid("Invalid update action."));
                    }
                    self.apply().await?
                }
                _ => return Err(HarnessError::missing("Unknown update operation.")),
            };
            Ok(Payload::Json(value))
        })
    }
}
