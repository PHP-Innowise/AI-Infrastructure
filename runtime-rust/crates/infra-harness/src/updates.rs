//! Clone fast-forward updates with an admission hold and verified native handoff.
//!
//! The restart runs only an engine and a browser interface compiled from the
//! sources the clone has after the fast-forward. Both record the digest of
//! their sources; when either differs, the server keeps running, new runs stay
//! held, and the update says what to rebuild before Update is pressed again.
use crate::{
    HarnessError, Result, projects,
    services::{Domain, Payload, Reply},
    sessions::Sessions,
    store,
};
use infra_core::bank::publication::PublicationFiles;
use infra_tooling::{
    artifacts::{Artifact, native_target},
    source_identity, ui_build, wasm_assets,
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
const HELD: &str = "New runs stay held until the Harness restarts.";
/// Rebuild steps, in the order they run from the clone.
const BUILD_ENGINE: &str =
    "cargo build --release --locked --manifest-path runtime-rust/Cargo.toml --bin infra-engine";
const BUILD_INTERFACE: &str = "./infra-engine repo build-ui";
const ASSEMBLE_ENGINE: &str = "./infra-engine repo artifacts --write --include-generator";
pub trait Restart: Send + Sync {
    fn refresh_application(&self, engine: &Path, expected_sha256: &str) -> Result<Vec<String>>;
    fn handoff(&self, engine: &Path, report: &Value) -> Result<bool>;
}
/// Admission of new runs, held from the update's start until the restart.
pub trait Admission: Send + Sync {
    fn hold(&self, reason: &str) -> Result<()>;
    fn release(&self) -> Result<()>;
}
impl Admission for Sessions {
    fn hold(&self, reason: &str) -> Result<()> {
        Sessions::hold(self, reason)
    }
    fn release(&self) -> Result<()> {
        Sessions::release(self)
    }
}
/// Why the engine or the browser interface cannot serve the clone's sources.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stale {
    /// Nothing usable was built for this platform.
    Missing,
    /// The build does not record the sources it came from.
    Unrecorded,
    /// The build came from other sources.
    Other,
}
pub struct Updates {
    root: PathBuf,
    sessions: Weak<dyn Admission>,
    restart: Arc<dyn Restart>,
    /// What the running engine can do; a restart needs at least as much.
    capabilities: Vec<Value>,
    state: Mutex<Value>,
    checked: Mutex<Option<Instant>>,
    serial: Mutex<()>,
    applying: tokio::sync::Mutex<()>,
    held: AtomicBool,
    stopping: AtomicBool,
    me: Weak<Self>,
}
impl Updates {
    pub fn new(
        root: PathBuf,
        sessions: Weak<dyn Admission>,
        restart: Arc<dyn Restart>,
        capabilities: Vec<Value>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            root,
            sessions,
            restart,
            capabilities,
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
    fn runner(&self) -> Result<Arc<dyn Admission>> {
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
    fn set(&self, value: Value) -> Result<()> {
        *self
            .state
            .lock()
            .map_err(|_| HarnessError::unavailable("Update state unavailable."))? = value;
        Ok(())
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
    /// The assembled engine compiled from the sources the clone has now.
    fn native_engine(&self) -> Result<std::result::Result<(PathBuf, String), Stale>> {
        let target = native_target()?;
        let sources = source_identity::engine(&self.root).map_err(|error| {
            HarnessError::unavailable(format!(
                "The clone's engine sources could not be read: {error}"
            ))
        })?;
        let mut files = PublicationFiles::new(&self.root)?;
        let mut stale = Stale::Missing;
        for edition in ["PHP Core", "Laravel", "Symfony", "Cms/wordpress"] {
            let name = format!("{edition}/memory-bank/runtime-capabilities.json");
            let Some(raw) = files.read(&name, 1024 * 1024)? else {
                continue;
            };
            let meta = crate::strict_json::parse(&raw, 1024 * 1024)?;
            if meta["engine"] != "infra-engine"
                || meta["target"] != target
                || !self.capabilities.iter().all(|v| {
                    meta["capabilities"]
                        .as_array()
                        .is_some_and(|a| a.contains(v))
                })
            {
                continue;
            }
            if meta["source"]["scheme"] != source_identity::SCHEME
                || !meta["source"]["sha256"].is_string()
            {
                if stale == Stale::Missing {
                    stale = Stale::Unrecorded;
                }
                continue;
            }
            if meta["source"]["sha256"] != sources.sha256.as_str() {
                stale = Stale::Other;
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
            return Ok(Ok((
                artifact.verify(&self.root.join(edition))?,
                artifact.sha256.clone(),
            )));
        }
        Ok(Err(stale))
    }
    /// Whether the browser interface the restarted server would load was
    /// compiled from the clone's interface sources.
    fn interface(&self) -> Result<std::result::Result<(), Stale>> {
        let sources = ui_build::source_sha256(&self.root)?;
        let Ok(assets) = wasm_assets::read_verified(&mut PublicationFiles::new(&self.root)?) else {
            return Ok(Err(Stale::Missing));
        };
        Ok(match assets.source_sha256 {
            Some(built) if built == sources => Ok(()),
            Some(_) => Err(Stale::Other),
            None => Err(Stale::Unrecorded),
        })
    }
    /// Hands the server over to the engine and interface built from the
    /// clone's sources, or refuses and says what to rebuild.
    fn restart(&self, from: &Value, changes: &Value) -> Result<Value> {
        let version = self.value(&["rev-parse", "--short", "HEAD"])?;
        let (engine, digest) = match (self.native_engine()?, self.interface()?) {
            (Ok(engine), Ok(())) => engine,
            (engine, interface) => {
                return Err(HarnessError {
                    status: 409,
                    code: "restart-required",
                    message: rebuild(
                        version.as_deref().unwrap_or("a new version"),
                        engine.err(),
                        interface.err(),
                    ),
                });
            }
        };
        let mut report = json!({"ok":true,"from":from,"to":version,"changes":changes,"restarting":false,"engine":engine,"engine_sha256":digest});
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
    }
    /// Records a failed update. Before the fast-forward nothing changed and
    /// runs resume. After it the clone is ahead of the running server, so runs
    /// stay held until a restart, and the error says so.
    fn settle(&self, mut error: HarnessError) -> HarnessError {
        let Ok(mut state) = self.status() else {
            return error;
        };
        if state["source_updated"] == true {
            if error.code != "restart-required" {
                error.message = format!(
                    "The clone was updated, but the Harness could not restart on it ({}). {HELD}",
                    error.message.trim_end_matches('.')
                );
            }
            state["state"] = "restart-required".into();
            if let Ok(runner) = self.runner() {
                let _ = runner.hold(&error.message);
            }
        } else {
            state["state"] = "failed".into();
            let _ = self.resume();
        }
        state["detail"] = error.message.clone().into();
        let _ = self.set(state);
        error
    }
    pub async fn apply(&self) -> Result<Value> {
        let _apply = self
            .applying
            .try_lock()
            .map_err(|_| HarnessError::conflict("The update is already running."))?;
        if self.held.load(Ordering::Acquire) {
            let state = self.status()?;
            if state["state"] != "restart-required" {
                return Err(HarnessError::conflict(
                    "An update is waiting for the Harness to restart.",
                ));
            }
            // The clone has already moved; a rebuild since may let the restart through.
            return self
                .restart(&state["from"], &state["changes"])
                .map_err(|error| self.settle(error));
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
                self.set(status)?;
                return Ok(
                    json!({"ok":true,"current":true,"restarting":false,"detail":"This clone is already up to date."}),
                );
            }
            if status["state"] != "available" {
                self.set(status.clone())?;
                return Err(HarnessError::conflict(
                    status["detail"]
                        .as_str()
                        .unwrap_or("There is no update to apply."),
                ));
            }
            let mut updating = status.clone();
            updating["state"] = "updating".into();
            updating["detail"] = "Updating…".into();
            self.set(updating)?;
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
            let mut updated = status.clone();
            updated["state"] = "updated".into();
            updated["source_updated"] = true.into();
            updated["from"] = status["version"].clone();
            updated["changes"] = status["behind"].clone();
            updated["version"] = json!(self.value(&["rev-parse", "--short", "HEAD"])?);
            updated["behind"] = 0.into();
            updated["commits"] = json!([]);
            updated["detail"] = "The clone updated; preparing the verified native restart.".into();
            self.set(updated)?;
            self.restart(&status["version"], &status["behind"])
        })();
        match result {
            Ok(value) => {
                if value["current"] == true {
                    self.resume()?;
                }
                Ok(value)
            }
            Err(error) => Err(self.settle(error)),
        }
    }
}
/// Explains what the clone lacks for a restart and how to provide it.
fn rebuild(version: &str, engine: Option<Stale>, interface: Option<Stale>) -> String {
    let mut problems = Vec::new();
    let mut commands = vec![BUILD_ENGINE];
    if let Some(stale) = engine {
        problems.push(match stale {
            Stale::Missing => "it has no assembled engine for this platform",
            Stale::Unrecorded => {
                "its assembled engine does not record which sources it was built from"
            }
            Stale::Other => "its assembled engine was built from other sources",
        });
    }
    if let Some(stale) = interface {
        problems.push(match stale {
            Stale::Missing => "its browser interface is not built",
            Stale::Unrecorded => {
                "its browser interface does not record which sources it was built from"
            }
            Stale::Other => "its browser interface was built from other sources",
        });
        commands.push(BUILD_INTERFACE);
    }
    if engine.is_some() {
        commands.push(ASSEMBLE_ENGINE);
    }
    format!(
        "The clone is now at {version}, but {}, so the Harness kept running its previous version. In the clone, run `{}`, then press Update again or run `./accelerator-app update`. Stopping the Harness with `./accelerator-app stop` and starting it with `./accelerator-app open` also starts the rebuilt version. {HELD}",
        problems.join(" and "),
        commands.join("`, then `"),
    )
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
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};

    struct Gate(Mutex<Option<String>>);
    impl Admission for Gate {
        fn hold(&self, reason: &str) -> Result<()> {
            *self.0.lock().unwrap() = Some(reason.into());
            Ok(())
        }
        fn release(&self) -> Result<()> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }
    /// Records the engine of each handoff instead of starting a server.
    #[derive(Default)]
    struct Handoffs(Mutex<Vec<PathBuf>>);
    impl Restart for Handoffs {
        fn refresh_application(&self, _: &Path, _: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
        fn handoff(&self, engine: &Path, _: &Value) -> Result<bool> {
            self.0.lock().unwrap().push(engine.into());
            Ok(true)
        }
    }
    fn git(directory: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=Harness",
                "-c",
                "user.email=harness@example.invalid",
            ])
            .args(["-c", "commit.gpgsign=false", "-c"])
            .arg(format!(
                "core.hooksPath={}",
                if cfg!(windows) { "NUL" } else { "/dev/null" }
            ))
            .arg("-C")
            .arg(directory)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    fn write(root: &Path, name: &str, bytes: impl AsRef<[u8]>) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    /// An upstream repository, its bare origin and the clone the Harness runs from.
    struct Fixture {
        _temporary: tempfile::TempDir,
        upstream: PathBuf,
        clone: PathBuf,
        gate: Arc<Gate>,
        handoffs: Arc<Handoffs>,
        updates: Arc<Updates>,
    }
    impl Fixture {
        fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let upstream = temporary.path().join("upstream");
            let clone = temporary.path().join("clone");
            fs::create_dir(&upstream).unwrap();
            git(&upstream, &["init", "--quiet", "--initial-branch=main"]);
            for (name, text) in [
                (
                    ".gitignore",
                    "/runtime-rust/target/\n**/memory-bank/bin/\n**/memory-bank/runtime-capabilities.json\n",
                ),
                ("README.md", "# Accelerators\n"),
                (
                    "runtime-rust/Cargo.toml",
                    "[workspace]\nmembers = [\"crates/*\"]\n",
                ),
                ("runtime-rust/Cargo.lock", "version = 4\n"),
                ("runtime-rust/crates/infra-cli/Cargo.toml", "[package]\n"),
                (
                    "runtime-rust/crates/infra-cli/src/lib.rs",
                    "pub fn run() {}\n",
                ),
                (
                    "runtime-rust/crates/infra-cli/tests/cli.rs",
                    "#[test]\nfn runs() {}\n",
                ),
                ("runtime-rust/crates/infra-ui/Cargo.toml", "[package]\n"),
                (
                    "runtime-rust/crates/infra-ui/src/lib.rs",
                    "pub fn render() {}\n",
                ),
            ] {
                write(&upstream, name, text);
            }
            git(&upstream, &["add", "--all"]);
            git(&upstream, &["commit", "--quiet", "--message", "base"]);
            git(
                temporary.path(),
                &["clone", "--quiet", "--bare", "upstream", "origin.git"],
            );
            git(&upstream, &["remote", "add", "origin", "../origin.git"]);
            git(
                temporary.path(),
                &["clone", "--quiet", "origin.git", "clone"],
            );
            let gate = Arc::new(Gate(Mutex::new(None)));
            let handoffs = Arc::new(Handoffs::default());
            let admission: Weak<dyn Admission> = Arc::<Gate>::downgrade(&gate);
            let updates = Updates::new(
                clone.clone(),
                admission,
                handoffs.clone(),
                vec![json!("harness")],
            );
            Self {
                _temporary: temporary,
                upstream,
                clone,
                gate,
                handoffs,
                updates,
            }
        }
        /// Commits upstream changes for the clone to update to.
        fn publish(&self, changes: &[(&str, &str)]) {
            for (name, text) in changes {
                write(&self.upstream, name, text);
            }
            git(
                &self.upstream,
                &["commit", "--quiet", "--all", "--message", "change"],
            );
            git(&self.upstream, &["push", "--quiet", "origin", "main"]);
        }
        /// The digests builds record of the clone's engine and interface sources.
        fn sources(&self) -> (String, String) {
            (
                source_identity::engine(&self.clone).unwrap().sha256,
                ui_build::source_sha256(&self.clone).unwrap(),
            )
        }
        /// Places an engine as `repo artifacts --write` does.
        fn assemble(&self, sources: Option<String>) {
            let engine = format!("infra-engine built from {sources:?}").into_bytes();
            write(
                &self.clone,
                "PHP Core/memory-bank/bin/infra-engine",
                &engine,
            );
            let descriptor = json!({"schema_version":1,"engine":"infra-engine","capability_profile":"infra-engine-v1","version":"0.1.0","target":native_target().unwrap(),"path":"memory-bank/bin/infra-engine","sha256":store::sha(&engine),"bytes":engine.len(),"capabilities":["harness","memory"],"source":sources.map(|sha256|json!({"scheme":source_identity::SCHEME,"sha256":sha256}))});
            write(
                &self.clone,
                "PHP Core/memory-bank/runtime-capabilities.json",
                serde_json::to_vec(&descriptor).unwrap(),
            );
        }
        /// Publishes an interface as `repo build-ui` does.
        fn build_interface(&self, sources: Option<String>) {
            let javascript = b"export default async function init() {}\n".as_slice();
            let wasm = b"\0asm\x01\0\0\0".as_slice();
            let mut manifest = json!({"schema_version":1,"js_sha256":store::sha(javascript),"wasm_sha256":store::sha(wasm)});
            if let Some(sources) = sources {
                manifest["source_sha256"] = sources.into();
            }
            let directory = "runtime-rust/target/wasm-ui";
            write(&self.clone, &format!("{directory}/infra_ui.js"), javascript);
            write(&self.clone, &format!("{directory}/infra_ui_bg.wasm"), wasm);
            write(
                &self.clone,
                &format!("{directory}/abi-manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            );
        }
        fn build_both(&self) {
            let (engine, interface) = self.sources();
            self.assemble(Some(engine));
            self.build_interface(Some(interface));
        }
        fn held(&self) -> Option<String> {
            self.gate.0.lock().unwrap().clone()
        }
        fn handoffs(&self) -> Vec<PathBuf> {
            self.handoffs.0.lock().unwrap().clone()
        }
    }

    #[tokio::test]
    async fn new_engine_sources_hold_the_restart_until_a_rebuilt_engine_is_assembled() {
        let fixture = Fixture::new();
        fixture.build_both();
        fixture.publish(&[(
            "runtime-rust/crates/infra-cli/src/lib.rs",
            "pub fn run() { println!(\"new\"); }\n",
        )]);
        let error = fixture.updates.apply().await.unwrap_err();
        assert_eq!(error.code, "restart-required");
        assert!(
            error
                .message
                .contains("but its assembled engine was built from other sources, so"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains(&format!(
                "run `{BUILD_ENGINE}`, then `{ASSEMBLE_ENGINE}`, then press Update again"
            )),
            "{}",
            error.message
        );
        // The clone moved; the server stays as it was and new runs say why.
        assert_eq!(
            git(&fixture.clone, &["rev-parse", "HEAD"]),
            git(&fixture.upstream, &["rev-parse", "HEAD"])
        );
        assert!(fixture.handoffs().is_empty());
        let status = fixture.updates.status().unwrap();
        assert_eq!(status["state"], "restart-required");
        assert_eq!(status["detail"], error.message.as_str());
        assert_eq!(fixture.held().as_deref(), Some(error.message.as_str()));
        // Update again after the rebuild: the restart runs the rebuilt engine.
        fixture.build_both();
        let report = fixture.updates.apply().await.unwrap();
        assert_eq!(report["restarting"], true);
        assert_eq!(report["changes"], 1);
        let handoffs = fixture.handoffs();
        assert_eq!(handoffs.len(), 1);
        assert!(handoffs[0].ends_with("PHP Core/memory-bank/bin/infra-engine"));
        assert_eq!(
            report["engine_sha256"],
            store::sha(&fs::read(&handoffs[0]).unwrap())
        );
    }

    #[tokio::test]
    async fn an_update_outside_the_compiled_sources_restarts_on_the_assembled_engine() {
        let fixture = Fixture::new();
        fixture.build_both();
        fixture.publish(&[
            ("README.md", "# Accelerators, revised\n"),
            (
                "runtime-rust/crates/infra-cli/tests/cli.rs",
                "#[test]\nfn still_runs() {}\n",
            ),
        ]);
        let report = fixture.updates.apply().await.unwrap();
        assert_eq!(report["restarting"], true);
        assert_eq!(
            report["to"],
            git(&fixture.clone, &["rev-parse", "--short", "HEAD"])
        );
        assert_eq!(fixture.handoffs().len(), 1);
        assert_eq!(fixture.held().as_deref(), Some(HOLD));
    }

    #[tokio::test]
    async fn an_interface_from_other_sources_holds_the_restart_until_it_is_rebuilt() {
        let fixture = Fixture::new();
        fixture.build_both();
        // The engine's workspace holds the interface crate, so both go stale.
        fixture.publish(&[(
            "runtime-rust/crates/infra-ui/src/lib.rs",
            "pub fn render() { /* new */ }\n",
        )]);
        let error = fixture.updates.apply().await.unwrap_err();
        assert!(
            error.message.contains(&format!(
                "but its assembled engine was built from other sources and its browser interface was built from other sources, so the Harness kept running its previous version. In the clone, run `{BUILD_ENGINE}`, then `{BUILD_INTERFACE}`, then `{ASSEMBLE_ENGINE}`, then press Update again"
            )),
            "{}",
            error.message
        );
        fixture.assemble(Some(fixture.sources().0));
        let error = fixture.updates.apply().await.unwrap_err();
        assert!(
            error.message.contains(&format!(
                "but its browser interface was built from other sources, so the Harness kept running its previous version. In the clone, run `{BUILD_ENGINE}`, then `{BUILD_INTERFACE}`, then press"
            )),
            "{}",
            error.message
        );
        assert!(fixture.handoffs().is_empty());
        fixture.build_interface(Some(fixture.sources().1));
        assert_eq!(fixture.updates.apply().await.unwrap()["restarting"], true);
        assert_eq!(fixture.handoffs().len(), 1);
    }

    #[tokio::test]
    async fn builds_that_do_not_record_their_sources_are_never_restarted() {
        let fixture = Fixture::new();
        fixture.assemble(None);
        fixture.build_interface(None);
        fixture.publish(&[("README.md", "# Accelerators, revised\n")]);
        let error = fixture.updates.apply().await.unwrap_err();
        assert!(
            error.message.contains("but its assembled engine does not record which sources it was built from and its browser interface does not record which sources it was built from,"),
            "{}",
            error.message
        );
        assert!(fixture.handoffs().is_empty());
        let fresh = Fixture::new();
        fresh.publish(&[("README.md", "# Accelerators, revised\n")]);
        let error = fresh.updates.apply().await.unwrap_err();
        assert!(
            error.message.contains(
                "but it has no assembled engine for this platform and its browser interface is not built,"
            ),
            "{}",
            error.message
        );
    }

    #[tokio::test]
    async fn a_current_clone_releases_the_hold() {
        let fixture = Fixture::new();
        let report = fixture.updates.apply().await.unwrap();
        assert_eq!(report["current"], true);
        assert_eq!(fixture.held(), None);
        assert_eq!(fixture.updates.status().unwrap()["state"], "current");
    }
}
