//! Native workspace evidence and checks on the session owner's queue.
use crate::{
    HarnessError, Result,
    services::{Domain, Payload, Reply},
    store::SessionStore,
};
use infra_core::{
    RuntimeContext,
    bank::publication::PublicationFiles,
    results::{PrepareResult, RecordResult, ResultService as NativeResultService},
};
use infra_host::process::{self, Limits};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
pub const OUTPUT_LIMIT: usize = 512 * 1024;
pub type CheckReply<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
pub type TargetReply<'a> = Pin<Box<dyn Future<Output = Result<Capture>> + Send + 'a>>;
/// The production implementation is a Weak<Sessions> adapter to the same Job
/// channel, admission lock, generation registry and cancellation owner as turns.
pub trait ResultsBackend: Send + Sync {
    fn workspace(&self, session: &Value) -> Result<PathBuf>;
    fn idle(&self) -> Result<()>;
    fn enqueue_check(&self, sid: &str, check_id: &str, previous_status: &str) -> Result<String>;
    fn operation_lock(&self) -> Arc<tokio::sync::Mutex<()>>;
    fn history_session_ids(&self, session: &Value) -> Result<Vec<String>>;
    fn shutdown_cancel(&self) -> Arc<AtomicBool>;
    fn result_context(&self, session: &Value) -> Result<RuntimeContext>;
}
pub trait CheckWorkflow: Send + Sync {
    fn run_check<'a>(
        &'a self,
        session: Value,
        check_id: String,
        generation: String,
        cancel: Arc<AtomicBool>,
    ) -> CheckReply<'a>;
}
pub trait TargetCheckWorkflow: Send + Sync {
    fn run_target<'a>(&'a self, check: &'a mut Value, cancel: Arc<AtomicBool>) -> TargetReply<'a>;
}
#[derive(Clone, Debug)]
pub struct Capture {
    pub output: String,
    pub exit_code: Option<i32>,
    pub reason: Option<String>,
    pub seconds: f64,
}
impl Capture {
    pub fn value(&self) -> Value {
        json!({"output":self.output,"exit_code":self.exit_code,"reason":self.reason,"seconds":self.seconds})
    }
}
pub(crate) fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub(crate) fn active(v: &Value) -> bool {
    matches!(v["status"].as_str(), Some("queued" | "running"))
}
pub(crate) fn exact(v: &Value, names: &[&str]) -> bool {
    v.as_object()
        .is_some_and(|m| m.len() == names.len() && names.iter().all(|k| m.contains_key(*k)))
}
pub(crate) fn sha(bytes: &[u8]) -> String {
    crate::store::sha(bytes)
}
pub(crate) fn safe_path(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 1024
        && !v.starts_with('/')
        && !v.contains('\\')
        && !v.contains(':')
        && !v.chars().any(|c| c < ' ' || c == '\u{7f}')
        && v.split('/')
            .all(|p| !p.is_empty() && ![".", "..", ".git"].contains(&p))
}
fn text(v: &Value, k: &str) -> Result<String> {
    v[k].as_str()
        .map(str::to_owned)
        .ok_or_else(|| HarnessError::invalid(format!("Missing {k}")))
}
fn db_json(raw: String) -> Result<Value> {
    Ok(serde_json::from_str(&raw)?)
}
fn capture_error(error: HarnessError, cancelled: bool) -> Result<&'static str> {
    // Unknown physical cleanup is never downgraded to an ordinary human or
    // shutdown cancellation, even when both happen at the same boundary.
    if error.code == "cleanup-unconfirmed" {
        return Err(error);
    }
    if cancelled {
        return Ok("cancelled");
    }
    match error.code {
        "timed_out" | "timeout" | "process_timeout" | "native-timeout" => Ok("timed_out"),
        "output_limit" | "process_output_limit" | "output-limit" => Ok("output_limit"),
        "native-cancelled" => Ok("cancelled"),
        _ => Err(error),
    }
}

pub struct Results {
    pub store: SessionStore,
    pub(crate) backend: Arc<dyn ResultsBackend>,
    target: RwLock<Option<Weak<dyn TargetCheckWorkflow>>>,
}
impl Results {
    pub fn new(store: SessionStore, backend: Arc<dyn ResultsBackend>) -> Result<Self> {
        let checks = {
            let db = store.connection()?;
            db.execute_batch("CREATE TABLE IF NOT EXISTS launches(id TEXT PRIMARY KEY,session_id TEXT NOT NULL,kind TEXT NOT NULL,status TEXT NOT NULL,started_at TEXT NOT NULL,finished_at TEXT,settings TEXT NOT NULL,usage TEXT,context TEXT,receipt TEXT);CREATE INDEX IF NOT EXISTS launches_session ON launches(session_id,started_at);CREATE TABLE IF NOT EXISTS checks(id TEXT PRIMARY KEY,session_id TEXT NOT NULL,data TEXT NOT NULL);")?;
            let columns = {
                let mut s = db.prepare("PRAGMA table_info(launches)")?;
                s.query_map([], |r| r.get::<_, String>(1))?
                    .collect::<std::result::Result<BTreeSet<_>, _>>()?
            };
            for c in ["context", "receipt"] {
                if !columns.contains(c) {
                    db.execute(&format!("ALTER TABLE launches ADD COLUMN {c} TEXT"), [])?;
                }
            }
            db.execute(
                "UPDATE launches SET status='interrupted',finished_at=? WHERE status='running'",
                [now()],
            )?;
            let mut s = db.prepare("SELECT data FROM checks")?;
            s.query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let this = Self {
            store,
            backend,
            target: RwLock::new(None),
        };
        for raw in checks {
            let mut c = db_json(raw)?;
            if active(&c) {
                c["status"] = "interrupted".into();
                c["finished_at"] = now().into();
                this.save_check(&c)?;
                let sid = text(&c, "session_id")?;
                if this.store.get(&sid)?["status"] == "interrupted" {
                    this.store
                        .patch(&sid, &json!({"status":c["previous_status"]}))?;
                }
            }
        }
        Ok(this)
    }
    pub fn bind_target(&self, executor: Weak<dyn TargetCheckWorkflow>) -> Result<()> {
        let mut target = self
            .target
            .write()
            .map_err(|_| HarnessError::unavailable("Check executor lock failed"))?;
        if target.is_some() {
            return Err(HarnessError::conflict(
                "Target check executor already bound",
            ));
        }
        *target = Some(executor);
        Ok(())
    }
    pub async fn capture(
        &self,
        argv: &[String],
        root: &Path,
        timeout: u64,
        cancel: Arc<AtomicBool>,
        env_extra: &BTreeMap<String, String>,
        mut update: impl FnMut(&str) -> Result<()> + Send,
    ) -> Result<Capture> {
        let stopping = self.backend.shutdown_cancel();
        if cancel.load(Ordering::Acquire) || stopping.load(Ordering::Acquire) {
            return Ok(Capture {
                output: String::new(),
                exit_code: None,
                reason: Some("cancelled".into()),
                seconds: 0.0,
            });
        }
        let mut env = std::env::vars()
            .filter(|(k, _)| !k.starts_with("GIT_"))
            .collect::<BTreeMap<_, _>>();
        env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        env.extend(env_extra.clone());
        let bytes = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = bytes.clone();
        let started = Instant::now();
        let result = process::run_raw_with_lock(
            argv,
            root,
            &env,
            &[],
            Limits {
                seconds: Some(timeout),
                output: OUTPUT_LIMIT,
                line: OUTPUT_LIMIT,
            },
            cancel.clone(),
            Some(self.store.runner_lock()),
            move |chunk| {
                let mut out = sink
                    .lock()
                    .map_err(|_| HarnessError::unavailable("Check output lock failed"))?;
                let left = OUTPUT_LIMIT.saturating_sub(out.len());
                out.extend_from_slice(&chunk[..chunk.len().min(left)]);
                update(&String::from_utf8_lossy(&out))?;
                Ok(())
            },
        )
        .await;
        self.finish_capture(result, bytes.as_ref(), cancel.as_ref(), started)
    }
    fn finish_capture(
        &self,
        result: Result<process::Output>,
        bytes: &Mutex<Vec<u8>>,
        cancel: &AtomicBool,
        started: Instant,
    ) -> Result<Capture> {
        if result
            .as_ref()
            .err()
            .is_some_and(|error| error.code == "cleanup-unconfirmed")
        {
            // This Arc is the runner's global admission stop, not the local
            // check token. Latch it before output-lock or cancellation errors
            // can obscure the first loss of physical cleanup authority.
            self.backend
                .shutdown_cancel()
                .store(true, Ordering::Release);
        }
        let result = match result {
            Ok(output) => Ok(output),
            Err(error) => Err(capture_error(error, cancel.load(Ordering::Acquire))?),
        };
        let streamed = bytes
            .lock()
            .map_err(|_| HarnessError::unavailable("Check output lock failed"))?
            .clone();
        match result {
            Ok(out) => {
                let raw = if out.bytes.is_empty() {
                    streamed
                } else {
                    out.bytes
                };
                Ok(Capture {
                    output: String::from_utf8_lossy(&raw[..raw.len().min(OUTPUT_LIMIT)])
                        .into_owned(),
                    exit_code: Some(out.code),
                    reason: if cancel.load(Ordering::Acquire) {
                        Some("cancelled".into())
                    } else {
                        None
                    },
                    seconds: (out.seconds * 1000.0).round() / 1000.0,
                })
            }
            Err(reason) => Ok(Capture {
                output: String::from_utf8_lossy(&streamed).into_owned(),
                exit_code: None,
                reason: Some(reason.into()),
                seconds: (started.elapsed().as_secs_f64() * 1000.0).round() / 1000.0,
            }),
        }
    }
    pub(crate) async fn git(
        &self,
        root: &Path,
        args: &[String],
        timeout: u64,
        env: &BTreeMap<String, String>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Capture> {
        let mut argv = vec![
            "git".into(),
            "--no-optional-locks".into(),
            "--literal-pathspecs".into(),
            "-c".into(),
            format!(
                "core.hooksPath={}",
                if cfg!(windows) { "NUL" } else { "/dev/null" }
            ),
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            "color.ui=false".into(),
            "-c".into(),
            "commit.gpgSign=false".into(),
            "-c".into(),
            "merge.autoStash=false".into(),
            "-C".into(),
            root.to_string_lossy().into_owned(),
        ];
        argv.extend_from_slice(args);
        let mut env = env.clone();
        env.insert("GIT_NO_REPLACE_OBJECTS".into(), "1".into());
        self.capture(&argv, root, timeout, cancel, &env, |_| Ok(()))
            .await
    }
    pub async fn baseline(&self, session: &Value) -> Result<()> {
        if !session["result_base"].is_null() {
            return Ok(());
        }
        let value = match self.backend.workspace(session) {
            Ok(root) => self
                .git_info(&root, true)
                .await
                .unwrap_or(json!({"is_git":false})),
            Err(_) => json!({"is_git":false}),
        };
        let base = json!({"head":value["head"],"branch":value["branch"],"dirty":value["dirty"],"is_git":value["is_git"]});
        let sid = text(session, "id")?;
        if self.store.get(&sid)?["result_base"].is_null() {
            self.store.patch(&sid, &json!({"result_base":base}))?;
        }
        Ok(())
    }
    pub async fn git_info(&self, root: &Path, status: bool) -> Result<Value> {
        let anchor = PublicationFiles::new(root)?;
        anchor.check()?;
        drop(anchor);
        self.git_info_impl(root, status).await
    }
    pub async fn git_info_impl(&self, root: &Path, status: bool) -> Result<Value> {
        let run = |args: Vec<String>| async move {
            let r = self
                .git(
                    root,
                    &args,
                    10,
                    &BTreeMap::new(),
                    self.backend.shutdown_cancel(),
                )
                .await?;
            if r.reason.is_some() {
                return Err(HarnessError::unavailable(
                    "Could not read bounded Git state",
                ));
            }
            Ok(r)
        };
        let top = run(vec!["rev-parse".into(), "--show-toplevel".into()]).await?;
        if top.exit_code != Some(0) {
            return Ok(
                json!({"is_git":false,"head":null,"branch":null,"dirty":if status{json!(false)}else{Value::Null}}),
            );
        }
        let head = run(vec!["rev-parse".into(), "--verify".into(), "HEAD".into()]).await?;
        let branch = run(vec![
            "symbolic-ref".into(),
            "--quiet".into(),
            "--short".into(),
            "HEAD".into(),
        ])
        .await?;
        if ![Some(0), Some(1)].contains(&branch.exit_code) {
            return Err(HarnessError::unavailable("Could not read project branch"));
        }
        let dirty = if status {
            let r = run(vec![
                "status".into(),
                "--porcelain".into(),
                "--untracked-files=normal".into(),
            ])
            .await?;
            if r.exit_code != Some(0) {
                return Err(HarnessError::unavailable("Could not read project status"));
            }
            json!(!r.output.is_empty())
        } else {
            Value::Null
        };
        Ok(
            json!({"is_git":true,"root":top.output.trim(),"head":if head.exit_code==Some(0){Some(head.output.trim())}else{None},"branch":if branch.output.trim().is_empty(){None}else{Some(branch.output.trim())},"dirty":dirty}),
        )
    }
    pub fn start_launch(&self, sid: &str, generation: &str, context: Option<&Value>) -> Result<()> {
        let s = self.store.get(sid)?;
        let mut settings = json!({});
        for k in [
            "provider",
            "model",
            "thinking_effort",
            "agents_enabled",
            "agent_count",
            "budgets",
            "agent_budget_plan",
            "sdd",
            "mode",
            "model_routing",
            "clash",
        ] {
            settings[k] = s[k].clone();
        }
        let kind = if s["fleet"].is_object() || s["fleet"] == true {
            "fleet".to_owned()
        } else if s["clash"].is_object() || s["clash"] == true {
            "clash".to_owned()
        } else if s["creator"].is_object() {
            format!("creator-{}", s["creator"]["phase"].as_str().unwrap_or(""))
        } else {
            "native".into()
        };
        self.store.connection()?.execute("INSERT INTO launches(id,session_id,kind,status,started_at,settings,context) VALUES (?,?,?,'running',?,?,?)",params![generation,sid,kind,now(),serde_json::to_string(&settings)?,context.map(serde_json::to_string).transpose()?])?;
        Ok(())
    }
    pub fn update_context(&self, generation: &str, context: &Value) -> Result<()> {
        self.store.connection()?.execute(
            "UPDATE launches SET context=? WHERE id=?",
            params![serde_json::to_string(context)?, generation],
        )?;
        Ok(())
    }
    pub fn save_receipt(&self, generation: &str, receipt: &Value) -> Result<()> {
        self.store.connection()?.execute(
            "UPDATE launches SET receipt=? WHERE id=?",
            params![serde_json::to_string(receipt)?, generation],
        )?;
        Ok(())
    }
    pub fn finish_launch(&self, generation: &str, status: &str) -> Result<()> {
        self.store.connection()?.execute(
            "UPDATE launches SET status=?,finished_at=? WHERE id=? AND status='running'",
            params![status, now(), generation],
        )?;
        Ok(())
    }
    pub fn last_window(&self, sid: &str, model: &str) -> Result<Option<u64>> {
        let rows = {
            let db = self.store.connection()?;
            let mut stmt=db.prepare("SELECT settings,context FROM launches WHERE session_id=? AND kind='native' AND context IS NOT NULL ORDER BY started_at DESC,rowid DESC LIMIT 20")?;
            stmt.query_map([sid], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (s, c) in rows {
            let s = db_json(s)?;
            let c = db_json(c)?;
            if s["model"] == model
                && let Some(n) = c["fill"]["window"].as_u64().filter(|v| *v > 0)
            {
                return Ok(Some(n));
            }
        }
        Ok(None)
    }
    pub fn history(&self, sid: &str) -> Result<Value> {
        let session = self.store.get(sid)?;
        let ids = self.backend.history_session_ids(&session)?;
        if ids.is_empty()
            || ids.len() > 4096
            || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
        {
            return Err(HarnessError::unavailable("Invalid launch history scope"));
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let (rows, checks) = {
            let db = self.store.connection()?;
            let sql = format!(
                "SELECT id,session_id,kind,status,started_at,finished_at,settings,usage,context,receipt FROM launches WHERE session_id IN ({placeholders}) ORDER BY started_at,rowid"
            );
            let mut stmt = db.prepare(&sql)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(ids.iter()), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, Option<String>>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, Option<String>>(9)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut stmt = db.prepare(
                "SELECT data FROM checks WHERE session_id=? ORDER BY rowid DESC LIMIT 50",
            )?;
            let checks = stmt
                .query_map([sid], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            (rows, checks)
        };
        let mut records = vec![];
        for (
            id,
            session_id,
            kind,
            status,
            started_at,
            finished_at,
            settings,
            usage,
            context,
            receipt,
        ) in rows
        {
            records.push(json!({"id":id,"session_id":session_id,"kind":kind,"status":status,"started_at":started_at,"finished_at":finished_at,"settings":db_json(settings)?,"usage":usage.map(db_json).transpose()?,"context":context.map(db_json).transpose()?,"receipt":receipt.map(db_json).transpose()?}));
        }
        let mut totals = json!({});
        for k in ["tokens", "cost_usd", "seconds"] {
            let mut sum = 0.0;
            let mut unknown = 0;
            for r in &records {
                if let Some(n) = r["usage"][k]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.0)
                {
                    sum += n;
                } else {
                    unknown += 1;
                }
            }
            totals[k] = json!({"reported":sum,"unknown_launches":unknown});
        }
        Ok(
            json!({"launches":records,"totals":totals,"checks":checks.into_iter().map(db_json).collect::<Result<Vec<_>>>()?,"notice":"History begins with launches recorded by this server version. Older turns are not reconstructed; unknown usage is not zero."}),
        )
    }
    pub async fn snapshot(&self, sid: &str) -> Result<Value> {
        let s = self.store.get(sid)?;
        let root = self.backend.workspace(&s)?;
        let info = self.git_info(&root, false).await?;
        if info["is_git"] != true || !info["head"].is_string() {
            return Ok(
                json!({"available":false,"id":null,"complete":false,"message":"A Git repository with a commit is required for a diff. Checks can still run in this workspace."}),
            );
        }
        let head = s["result_base"]["head"]
            .as_str()
            .unwrap_or(info["head"].as_str().unwrap())
            .to_owned();
        let command = |tail: &[&str]| {
            let mut v = vec![
                "diff".into(),
                "--no-ext-diff".into(),
                "--no-textconv".into(),
                "--relative".into(),
                "--no-renames".into(),
            ];
            v.extend(tail.iter().map(|s| s.to_string()));
            v.extend([head.clone(), "--".into(), ".".into()]);
            v
        };
        let mut observations = vec![];
        for args in [
            command(&[]),
            command(&["--name-status", "-z"]),
            vec![
                "ls-files".into(),
                "--others".into(),
                "--exclude-standard".into(),
                "-z".into(),
                "--".into(),
                ".".into(),
            ],
        ] {
            let r = self
                .git(
                    &root,
                    &args,
                    10,
                    &BTreeMap::new(),
                    self.backend.shutdown_cancel(),
                )
                .await?;
            if r.exit_code.is_some_and(|c| c != 0) && r.reason.is_none() {
                return Err(HarnessError::unavailable(
                    "Could not read the workspace diff; its original commit may be unavailable.",
                ));
            }
            observations.push(r);
        }
        let mut complete = observations.iter().all(|v| v.reason.is_none())
            && !observations[0].output.contains("Binary files ")
            && !observations[0].output.contains("Subproject commit ");
        let parts = observations[1].output.split('\0').collect::<Vec<_>>();
        let mut files = vec![];
        for pair in parts.as_chunks::<2>().0 {
            if !pair[0].is_empty() {
                files.push(json!({"status":pair[0],"path":pair[1]}));
            }
        }
        let mut diff = observations[0].output.clone();
        let mut source = PublicationFiles::new(&root)?;
        for path in observations[2].output.split('\0').filter(|p| !p.is_empty()) {
            files.push(json!({"status":"?","path":path}));
            if files.len() > 500 || diff.len() >= OUTPUT_LIMIT {
                complete = false;
                continue;
            }
            let read = if safe_path(path) {
                source.read_prefix(path, 65537).ok().flatten()
            } else {
                None
            };
            let body = read.filter(|(size, b)| *size <= 65536 && !b.contains(&0));
            if let Some((_, bytes)) = body {
                let body = String::from_utf8_lossy(&bytes);
                diff.push_str(&format!("--- /dev/null\n+++ {path}\n"));
                let lines = body.split_inclusive('\n').collect::<Vec<_>>();
                if !lines.is_empty() {
                    diff.push_str(&format!("@@ -0,0 +1,{} @@\n", lines.len()));
                    for line in lines {
                        diff.push('+');
                        diff.push_str(line);
                        if !line.ends_with('\n') {
                            diff.push_str("\n\\ No newline at end of file\n");
                        }
                    }
                }
            } else {
                diff.push_str(&format!(
                    "\nUntracked binary, large or linked file (content omitted): {path}\n"
                ));
                complete = false;
            }
        }
        if diff.len() > OUTPUT_LIMIT {
            let mut end = OUTPUT_LIMIT;
            while !diff.is_char_boundary(end) {
                end -= 1;
            }
            diff.truncate(end);
            complete = false;
        }
        source.check()?;
        let image = json!([
            "rust-workspace-snapshot-v2",
            root,
            head,
            info["head"],
            diff,
            files
        ]);
        let id = sha(&serde_json::to_vec(&image)?);
        Ok(
            json!({"available":true,"id":id,"base":head,"head":info["head"],"branch":info["branch"],"files":files.into_iter().take(500).collect::<Vec<_>>(),"diff":diff,"complete":complete,"preexisting_changes":s["result_base"]["dirty"],"baseline_recorded":s["result_base"]["head"].is_string(),"message":"Current workspace versus the launch baseline. Includes pre-existing and external edits; this is not proof of agent authorship."}),
        )
    }
    pub async fn get(&self, sid: &str, include: &str) -> Result<Value> {
        if !["full", "names", "none"].contains(&include) {
            return Err(HarnessError::invalid("Invalid results diff selector"));
        }
        let s = self.store.get(sid)?;
        let mut out = self.history(sid)?;
        out["session_id"] = sid.into();
        out["workspace"] = s["project_path"].clone();
        if include != "none" {
            let mut snapshot = if active(&s) {
                Value::Null
            } else {
                self.snapshot(sid).await?
            };
            if include == "names"
                && let Some(o) = snapshot.as_object_mut()
            {
                o.remove("diff");
            }
            out["snapshot"] = snapshot;
        }
        Ok(out)
    }
    pub fn save_check(&self, c: &Value) -> Result<()> {
        self.store.connection()?.execute("INSERT INTO checks(id,session_id,data) VALUES (?,?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data",params![text(c,"id")?,text(c,"session_id")?,serde_json::to_string(c)?])?;
        Ok(())
    }
    pub fn check(&self, id: &str, sid: Option<&str>) -> Result<Option<Value>> {
        let raw = self
            .store
            .connection()?
            .query_row("SELECT session_id,data FROM checks WHERE id=?", [id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .optional()?;
        match raw {
            Some((owner, raw)) if sid.is_none_or(|sid| owner == sid) => Ok(Some(db_json(raw)?)),
            _ => Ok(None),
        }
    }
    pub fn check_command(data: &Value) -> Result<Vec<String>> {
        if !exact(data, &["command", "timeout"]) {
            return Err(HarnessError::invalid(
                "Provide a check command and timeout.",
            ));
        }
        let command = data["command"]
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.len() <= 4000 && !s.chars().any(|c| c < ' '))
            .ok_or_else(|| HarnessError::invalid("Enter one command of at most 4000 bytes."))?;
        let argv = infra_host::command::split_command(command)?;
        if argv.is_empty()
            || argv.len() > 100
            || argv
                .iter()
                .any(|s| ["|", "||", "&&", ";", ">", ">>", "<"].contains(&s.as_str()))
        {
            return Err(HarnessError::invalid(
                "Run one command at a time; shell operators are not expanded.",
            ));
        }
        if data["timeout"]
            .as_u64()
            .is_none_or(|s| !(1..=3600).contains(&s))
        {
            return Err(HarnessError::invalid("Use a timeout of 1–3600 seconds."));
        }
        Ok(argv)
    }
    pub fn prepare_check(&self, sid: &str, data: &Value, metadata: &Value) -> Result<Value> {
        let argv = Self::check_command(data)?;
        let s = self.store.get(sid)?;
        if active(&s) {
            return Err(HarnessError::conflict(
                "Wait for the current launch or queue to finish.",
            ));
        }
        let mut c = json!({"id":uuid::Uuid::new_v4().to_string(),"session_id":sid,"command":data["command"],"timeout":data["timeout"],"argv":argv,"status":"queued","output":"","exit_code":null,"created_at":now(),"started_at":null,"finished_at":null,"seconds":null,"previous_status":s["status"]});
        if let Some(m) = metadata.as_object() {
            for (k, v) in m {
                if c.get(k).is_some() {
                    return Err(HarnessError::invalid(
                        "Check metadata overwrites an owned field",
                    ));
                }
                c[k] = v.clone();
            }
        }
        self.save_check(&c)?;
        Ok(c)
    }
    pub fn submit_check(&self, c: &Value) -> Result<String> {
        match self.backend.enqueue_check(
            &text(c, "session_id")?,
            &text(c, "id")?,
            c["previous_status"].as_str().unwrap_or("idle"),
        ) {
            Ok(g) => Ok(g),
            Err(e) => {
                let mut c = c.clone();
                c["status"] = "cancelled".into();
                c["finished_at"] = now().into();
                self.save_check(&c)?;
                Err(e)
            }
        }
    }
    pub async fn start_check(&self, sid: &str, data: &Value) -> Result<Value> {
        if !exact(data, &["command", "timeout", "snapshot_id"]) {
            return Err(HarnessError::invalid(
                "Provide a command, timeout and the reviewed snapshot.",
            ));
        }
        let cmd = json!({"command":data["command"],"timeout":data["timeout"]});
        Self::check_command(&cmd)?;
        let lock = self.backend.operation_lock();
        let guard = lock.lock().await;
        let s = self.store.get(sid)?;
        if !s["creator"].is_null() {
            return Err(HarnessError::conflict(
                "Creator runs its canonical checks at its own checkpoints.",
            ));
        }
        let snapshot = self.snapshot(sid).await?;
        if data["snapshot_id"] != snapshot["id"] {
            return Err(HarnessError::conflict(
                "Workspace changed. Refresh the diff before running a check.",
            ));
        }
        let c = self.prepare_check(
            sid,
            &cmd,
            &json!({"snapshot_id":snapshot["id"],"snapshot_complete":snapshot["complete"]}),
        )?;
        drop(guard);
        self.submit_check(&c)?;
        self.store.get(sid)
    }
    pub fn prepare_native_result(&self, sid: &str, data: PrepareResult) -> Result<Value> {
        let _source_frame = infra_core::git::source_frame_scope();
        let s = self.store.get(sid)?;
        let context = self.backend.result_context(&s)?;
        Ok(NativeResultService::new(context).prepare(data)?)
    }
    pub fn record_native_result(&self, sid: &str, data: RecordResult) -> Result<Value> {
        let _source_frame = infra_core::git::source_frame_scope();
        let s = self.store.get(sid)?;
        let context = self.backend.result_context(&s)?;
        Ok(NativeResultService::new(context).record(data)?)
    }
}
impl CheckWorkflow for Results {
    fn run_check<'a>(
        &'a self,
        session: Value,
        check_id: String,
        generation: String,
        cancel: Arc<AtomicBool>,
    ) -> CheckReply<'a> {
        Box::pin(async move {
            let sid = text(&session, "id")?;
            let mut c = self
                .check(&check_id, Some(&sid))?
                .ok_or_else(|| HarnessError::missing("Check receipt is missing"))?;
            if cancel.load(Ordering::Acquire) || self.store.generation(&sid)? != generation {
                c["status"] = "cancelled".into();
                c["finished_at"] = now().into();
                self.save_check(&c)?;
                return Ok(());
            }
            c["status"] = "running".into();
            c["started_at"] = now().into();
            self.save_check(&c)?;
            let outcome = if c["kind"] == "target" {
                let target = self
                    .target
                    .read()
                    .map_err(|_| HarnessError::unavailable("Target check lock failed"))?
                    .as_ref()
                    .and_then(Weak::upgrade);
                match target {
                    Some(t) => t.run_target(&mut c, cancel.clone()).await,
                    None => Err(HarnessError::unavailable(
                        "Native target check executor is not bound",
                    )),
                }
            } else {
                async {
                    let before = self.snapshot(&sid).await?;
                    if before["id"] != c["snapshot_id"] {
                        return Err(HarnessError::conflict(
                            "Workspace changed while queued. Refresh and run the check again.",
                        ));
                    }
                    let root = self.backend.workspace(&self.store.get(&sid)?)?;
                    let argv = serde_json::from_value::<Vec<String>>(c["argv"].clone())?;
                    let result = self
                        .capture(
                            &argv,
                            &root,
                            c["timeout"].as_u64().unwrap_or(1),
                            cancel.clone(),
                            &BTreeMap::new(),
                            |output| {
                                c["output"] = output.into();
                                self.save_check(&c)
                            },
                        )
                        .await?;
                    let after = self.snapshot(&sid).await?;
                    c["workspace_changed"] = (after["id"] != c["snapshot_id"]).into();
                    Ok(result)
                }
                .await
            };
            match outcome {
                Ok(result) => {
                    let status = result.reason.clone().unwrap_or_else(|| {
                        if result.exit_code == Some(0) {
                            "passed".into()
                        } else {
                            "failed".into()
                        }
                    });
                    for (k, v) in result.value().as_object().unwrap() {
                        c[k] = v.clone();
                    }
                    c["status"] = status.into();
                }
                Err(error) => {
                    if error.code == "cleanup-unconfirmed" {
                        c["status"] = "failed".into();
                        c["output"] =
                            format!("{}\n{}", c["output"].as_str().unwrap_or(""), error.message)
                                .into();
                        c["finished_at"] = now().into();
                        // Receipt persistence failure cannot hide the loss of
                        // cleanup authority from the queue/admission owner.
                        let _ = self.save_check(&c);
                        return Err(error);
                    }
                    c["status"] = if cancel.load(Ordering::Acquire) {
                        "cancelled"
                    } else {
                        "failed"
                    }
                    .into();
                    c["output"] =
                        format!("{}\n{}", c["output"].as_str().unwrap_or(""), error.message).into();
                }
            }
            c["finished_at"] = now().into();
            self.save_check(&c)?;
            if self.store.generation(&sid)? == generation {
                self.store.guarded_event(&sid,&generation,&json!({"kind":"status","text":format!("Check {}: {}. This is separate from the agent result.",c["status"].as_str().unwrap_or("failed"),c["command"].as_str().unwrap_or(""))}))?;
            }
            Ok(())
        })
    }
}
pub struct ResultService {
    pub results: Arc<Results>,
}
impl Domain for ResultService {
    fn execute<'a>(&'a self, op: &'a str, r: Value) -> Reply<'a> {
        Box::pin(async move {
            let sid = r["params"]["session_id"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Missing session ID"))?;
            let value = match op {
                "results.get" => {
                    self.results
                        .get(
                            sid,
                            r["query"]["diff"]
                                .as_str()
                                .map(|v| if v == "0" { "none" } else { v })
                                .unwrap_or("full"),
                        )
                        .await?
                }
                "results.start_check" => {
                    json!({"session":self.results.start_check(sid,&r["body"]).await?})
                }
                _ => return Err(HarnessError::missing("Unknown results operation")),
            };
            Ok(Payload::Json(value))
        })
    }
}

#[cfg(test)]
mod cleanup_error_tests {
    use super::capture_error;
    use crate::HarnessError;

    fn error(code: &'static str) -> HarnessError {
        HarnessError {
            status: 503,
            code,
            message: "isolated host failure".into(),
        }
    }
    #[test]
    fn cleanup_failure_wins_over_human_and_shutdown_cancellation() {
        for cancelled in [false, true] {
            let result = capture_error(error("cleanup-unconfirmed"), cancelled);
            assert_eq!(result.unwrap_err().code, "cleanup-unconfirmed");
        }
    }
    #[test]
    fn ordinary_capture_stop_reasons_keep_their_existing_mapping() {
        assert_eq!(
            capture_error(error("native-timeout"), false).unwrap(),
            "timed_out"
        );
        assert_eq!(
            capture_error(error("output-limit"), false).unwrap(),
            "output_limit"
        );
        assert_eq!(
            capture_error(error("native-cancelled"), false).unwrap(),
            "cancelled"
        );
        assert_eq!(
            capture_error(error("native-timeout"), true).unwrap(),
            "cancelled"
        );
    }
    #[test]
    fn fatal_capture_stops_admission_before_poisoned_output_or_cancel_mapping() {
        use crate::sessions::generation_completion_tests::Fixture;
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        use std::time::Instant;
        for cancelled in [false, true] {
            let fixture = Fixture::new();
            let results = fixture.delivery().results;
            let stopped = results.backend.shutdown_cancel();
            assert!(!stopped.load(Ordering::Acquire));
            let bytes = Arc::new(Mutex::new(Vec::<u8>::new()));
            let poisoning = bytes.clone();
            assert!(
                std::thread::spawn(move || {
                    let _guard = poisoning.lock().unwrap();
                    panic!("isolated output-lock poison");
                })
                .join()
                .is_err()
            );
            let failure = results
                .finish_capture(
                    Err(error("cleanup-unconfirmed")),
                    bytes.as_ref(),
                    &AtomicBool::new(cancelled),
                    Instant::now(),
                )
                .unwrap_err();
            assert_eq!(failure.code, "cleanup-unconfirmed");
            assert!(stopped.load(Ordering::Acquire));
            assert!(bytes.is_poisoned());
        }
    }

    #[tokio::test]
    async fn latched_global_stop_refuses_capture_with_a_fresh_local_token() {
        use crate::sessions::generation_completion_tests::Fixture;
        use std::{
            collections::BTreeMap,
            path::Path,
            sync::{
                Arc, Mutex,
                atomic::{AtomicBool, Ordering},
            },
            time::Instant,
        };
        let fixture = Fixture::new();
        let results = fixture.delivery().results;
        let stopped = results.backend.shutdown_cancel();
        let failed = results
            .finish_capture(
                Err(error("cleanup-unconfirmed")),
                &Mutex::new(Vec::new()),
                &AtomicBool::new(true),
                Instant::now(),
            )
            .unwrap_err();
        assert_eq!(failed.code, "cleanup-unconfirmed");
        assert!(stopped.load(Ordering::Acquire));
        let local = Arc::new(AtomicBool::new(false));
        // Empty argv cannot start an OS process even if this preflight guard
        // regresses: Host validates it before creating a guardian.
        let capture = results
            .capture(
                &[],
                Path::new("missing-isolated-workspace"),
                1,
                local.clone(),
                &BTreeMap::new(),
                |_| panic!("stopped capture must not stream output"),
            )
            .await
            .unwrap();
        assert!(!local.load(Ordering::Acquire));
        assert_eq!(capture.reason.as_deref(), Some("cancelled"));
        assert_eq!(capture.exit_code, None);
        assert!(capture.output.is_empty());
        assert_eq!(capture.seconds, 0.0);
    }
}
