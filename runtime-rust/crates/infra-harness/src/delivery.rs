//! Explicit, immutable local Git previews. No remote delivery authority.
use crate::{
    HarnessError, Result,
    results::{Capture, Results, TargetCheckWorkflow, TargetReply, exact, safe_path, sha},
    services::{Domain, Payload, Reply},
    store::SessionStore,
};
use infra_core::bank::publication::PublicationFiles;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};
const TTL: Duration = Duration::from_secs(900);
const MAX_FILES: usize = 500;
const IMAGE: usize = 64 * 1024 * 1024;
fn invalid(s: impl Into<String>) -> HarnessError {
    HarnessError::invalid(s)
}
fn conflict(s: impl Into<String>) -> HarnessError {
    HarnessError::conflict(s)
}
fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().ok_or_else(|| invalid(format!("Missing {k}")))
}
fn owned_hash(v: &str) -> bool {
    [40, 64].contains(&v.len())
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
#[derive(Clone)]
struct Preview {
    sid: String,
    created: Instant,
    value: Value,
}
#[derive(Clone)]
pub struct Delivery {
    pub store: SessionStore,
    pub results: Arc<Results>,
    folder: PathBuf,
    previews: Arc<Mutex<BTreeMap<String, Preview>>>,
    blocked: Arc<Mutex<BTreeMap<String, String>>>,
}
impl Delivery {
    pub fn new(store: SessionStore, results: Arc<Results>) -> Result<Self> {
        let folder = store.state.join("delivery");
        infra_core::fs::ensure_directory(&folder, true)?;
        PublicationFiles::new(&folder)?.check()?;
        Ok(Self {
            store,
            results,
            folder,
            previews: Arc::new(Mutex::new(BTreeMap::new())),
            blocked: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }
    /// Recovery must finish before HTTP dispatch/runner admission.
    pub async fn open(store: SessionStore, results: Arc<Results>) -> Result<Self> {
        let this = Self::new(store, results)?;
        this.recover().await?;
        Ok(this)
    }
    pub async fn recover(&self) -> Result<()> {
        let anchor = PublicationFiles::new(&self.folder)?;
        let mut names = std::fs::read_dir(&self.folder)?
            .map(|r| r.map(|e| e.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        names.sort();
        anchor.check()?;
        for n in names {
            let n = n
                .into_string()
                .map_err(|_| invalid("Delivery storage has an invalid name"))?;
            if n.starts_with("transaction-")
                && n.ends_with(".json")
                && let Err(error) = self.recover_one(&n).await
            {
                let mut files = PublicationFiles::new(&self.folder)?;
                let raw = files
                    .read(&n, 65536)?
                    .ok_or_else(|| invalid("An interrupted delivery journal cannot be read."))?;
                let v = infra_core::strict_json::parse(&raw, 65536, 128, 65536)?;
                let sid = text(&v, "sid")?.to_owned();
                self.blocked.lock().map_err(|_|invalid("Delivery lock failed"))?.insert(sid,format!("An interrupted Git commit needs inspection before another delivery. {}",error.message));
            }
        }
        Ok(())
    }
    async fn git(
        &self,
        root: &Path,
        args: Vec<String>,
        env: &BTreeMap<String, String>,
        allowed: &[i32],
        cleanup: bool,
    ) -> Result<Capture> {
        let result = self
            .results
            .git(
                root,
                &args,
                30,
                env,
                if cleanup {
                    cancel()
                } else {
                    self.results.backend.shutdown_cancel()
                },
            )
            .await?;
        if result.reason.is_some() {
            return Err(conflict(
                "Git stopped or exceeded the preview limit. Refresh and inspect the workspace.",
            ));
        }
        if !result.exit_code.is_some_and(|n| allowed.contains(&n)) {
            return Err(conflict(format!(
                "Git could not complete the operation: {}",
                result.output.chars().take(2000).collect::<String>()
            )));
        }
        Ok(result)
    }
    async fn string(&self, root: &Path, args: Vec<String>) -> Result<String> {
        Ok(self
            .git(root, args, &BTreeMap::new(), &[0], false)
            .await?
            .output
            .trim()
            .to_owned())
    }
    async fn source(&self, sid: &str) -> Result<(Value, PathBuf, Value)> {
        let session = self.store.get(sid)?;
        if !session["creator"].is_null() || session["workspace"] != "worktree" {
            return Err(conflict(
                "Delivery is available for agent sessions in a Git worktree.",
            ));
        }
        if !session["result_base"]["head"].is_string() {
            return Err(conflict(
                "This older session has no recorded baseline. Send a new turn before using worktree delivery.",
            ));
        }
        if let Some(reason) = self
            .blocked
            .lock()
            .map_err(|_| invalid("Delivery lock failed"))?
            .get(sid)
            .cloned()
        {
            return Err(conflict(reason));
        }
        let root = self.results.backend.workspace(&session)?;
        let mut info = self.results.git_info(&root, false).await?;
        let branch = self
            .git(
                &root,
                vec!["symbolic-ref".into(), "HEAD".into()],
                &BTreeMap::new(),
                &[0, 1],
                false,
            )
            .await?
            .output
            .trim()
            .strip_prefix("refs/heads/")
            .map(str::to_owned);
        info["branch"] = json!(branch);
        if !info["head"].is_string()
            || !info["branch"].is_string()
            || info["branch"] != session["branch"]
        {
            return Err(conflict(
                "The session branch changed. Restore its original worktree branch first.",
            ));
        }
        if self
            .git(
                &root,
                vec![
                    "config".into(),
                    "--bool".into(),
                    "core.sparseCheckout".into(),
                ],
                &BTreeMap::new(),
                &[0, 1],
                false,
            )
            .await?
            .output
            .trim()
            == "true"
        {
            return Err(conflict("Delivery does not support sparse worktrees."));
        }
        self.settled(&root).await?;
        Ok((session, root, info))
    }
    async fn git_path(&self, root: &Path, name: &str) -> Result<PathBuf> {
        let raw = self
            .string(
                root,
                vec!["rev-parse".into(), "--git-path".into(), name.into()],
            )
            .await?;
        let path = PathBuf::from(raw);
        Ok(infra_core::fs::absolute(&if path.is_absolute() {
            path
        } else {
            root.join(path)
        })?)
    }
    async fn settled(&self, root: &Path) -> Result<()> {
        for n in [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "rebase-merge",
            "rebase-apply",
            "sequencer",
        ] {
            if self.git_path(root, n).await?.try_exists()? {
                return Err(conflict(
                    "Finish the existing merge, rebase or cherry-pick before delivery.",
                ));
            }
        }
        if !self
            .string(root, vec!["ls-files".into(), "--unmerged".into()])
            .await?
            .is_empty()
        {
            return Err(conflict(
                "Resolve the existing index conflicts before delivery.",
            ));
        }
        Ok(())
    }
    async fn index(&self, root: &Path) -> Result<(PathBuf, Option<Vec<u8>>, Option<u32>)> {
        let path = self.git_path(root, "index").await?;
        let parent = path
            .parent()
            .ok_or_else(|| invalid("Git index has no parent"))?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| invalid("Git index has an unsupported name"))?;
        let mut files = PublicationFiles::new(parent)?;
        let body = files.read(name, IMAGE)?;
        let mode = files.read_mode(name)?;
        files.check()?;
        Ok((path, body, mode))
    }
    async fn names(&self, root: &Path, before: &str, after: Option<&str>) -> Result<Vec<Value>> {
        let mut args = vec![
            "diff".into(),
            "--no-ext-diff".into(),
            "--no-textconv".into(),
            "--relative".into(),
            "--no-renames".into(),
            "--name-status".into(),
            "-z".into(),
            before.into(),
        ];
        if let Some(after) = after {
            args.push(after.into());
        }
        args.extend(["--".into(), ".".into()]);
        let result = self.git(root, args, &BTreeMap::new(), &[0], false).await?;
        let parts = result.output.split('\0').collect::<Vec<_>>();
        let mut files = vec![];
        for pair in parts.as_chunks::<2>().0 {
            if !pair[0].is_empty() {
                files.push(json!({"status":pair[0],"path":pair[1]}));
            }
        }
        Ok(files)
    }
    async fn pending(&self, root: &Path, info: &Value) -> Result<Value> {
        let files = self.names(root, text(info, "head")?, None).await?;
        let mut by_path = BTreeMap::new();
        for f in files {
            by_path.insert(text(&f, "path")?.to_owned(), f);
        }
        let untracked = self
            .git(
                root,
                vec![
                    "ls-files".into(),
                    "--others".into(),
                    "--exclude-standard".into(),
                    "-z".into(),
                    "--".into(),
                    ".".into(),
                ],
                &BTreeMap::new(),
                &[0],
                false,
            )
            .await?;
        for p in untracked.output.split('\0').filter(|p| !p.is_empty()) {
            let status = if by_path.contains_key(p) { "M" } else { "?" };
            by_path.insert(p.to_owned(), json!({"status":status,"path":p}));
        }
        if by_path.len() > MAX_FILES {
            return Err(conflict(
                "Delivery supports up to 500 pending files. Narrow the worktree first.",
            ));
        }
        let mut source = PublicationFiles::new(root)?;
        let mut fingerprints = vec![];
        let mut size = 0usize;
        for f in by_path.values() {
            let p = text(f, "path")?;
            if !safe_path(p) {
                return Err(conflict("A pending file has an unsupported path."));
            }
            let body = source.read(p, 4 * 1024 * 1024)?;
            size = size
                .checked_add(body.as_ref().map_or(0, Vec::len))
                .ok_or_else(|| invalid("Pending file limit exceeded"))?;
            if size > IMAGE {
                return Err(conflict("Pending files exceed the 64 MiB delivery limit."));
            }
            let image = source.image_identity(p)?;
            let meta=body.as_ref().map(|b|json!({"identity":image,"mode":image.map(|i|i.2&0o777),"bytes":b.len(),"hash":sha(b)}));
            fingerprints.push(json!([f, meta]));
        }
        let (_, index, _) = self.index(root).await?;
        source.check()?;
        let identity = source.root_identity();
        let key = sha(&serde_json::to_vec(&json!([
            "rust-delivery-snapshot-v2",
            info["head"],
            info["branch"],
            root,
            identity,
            sha(index.as_deref().unwrap_or(&[])),
            fingerprints
        ]))?);
        Ok(json!({"snapshot_id":key,"files":by_path.into_values().collect::<Vec<_>>()}))
    }
    async fn branches(&self, root: &Path) -> Result<Vec<Value>> {
        let raw = self
            .git(
                root,
                vec![
                    "for-each-ref".into(),
                    "--format=%(refname:strip=2)%00%(objectname)".into(),
                    "refs/heads/".into(),
                ],
                &BTreeMap::new(),
                &[0],
                false,
            )
            .await?
            .output;
        raw.lines()
            .map(|line| {
                let (n, h) = line
                    .split_once('\0')
                    .ok_or_else(|| invalid("Invalid Git branch record"))?;
                Ok(json!({"name":n,"head":h}))
            })
            .collect()
    }
    async fn commits(&self, s: &Value, root: &Path, info: &Value) -> Result<Vec<Value>> {
        let Some(base) = s["result_base"]["head"].as_str() else {
            return Ok(vec![]);
        };
        let ancestor = self
            .git(
                root,
                vec![
                    "merge-base".into(),
                    "--is-ancestor".into(),
                    base.into(),
                    text(info, "head")?.into(),
                ],
                &BTreeMap::new(),
                &[0, 1],
                false,
            )
            .await?;
        if ancestor.exit_code != Some(0) {
            return Ok(vec![]);
        }
        let raw = self
            .git(
                root,
                vec![
                    "log".into(),
                    "--max-count=100".into(),
                    "--no-merges".into(),
                    "--format=%H%x00%s".into(),
                    format!("{base}..{}", text(info, "head")?),
                ],
                &BTreeMap::new(),
                &[0],
                false,
            )
            .await?
            .output;
        raw.lines()
            .map(|line| {
                let (h, s) = line
                    .split_once('\0')
                    .ok_or_else(|| invalid("Invalid Git commit record"))?;
                Ok(json!({"sha":h,"subject":s}))
            })
            .collect()
    }
    async fn checks(&self, sid: &str) -> Result<Vec<Value>> {
        let snapshot = self.results.snapshot(sid).await?;
        let history = self.results.history(sid)?;
        Ok(history["checks"].as_array().into_iter().flatten().take(10).filter(|c|c["kind"]!="target").map(|c|json!({"status":c["status"],"command":c["command"],"exit_code":c["exit_code"],"current":snapshot["complete"]==true&&c["snapshot_complete"]==true&&snapshot["id"]==c["snapshot_id"]&&c["workspace_changed"]!=true})).collect())
    }
    fn remember(&self, sid: &str, value: Value) -> Result<Value> {
        let mut previews = self
            .previews
            .lock()
            .map_err(|_| invalid("Delivery preview lock failed"))?;
        previews.retain(|_, p| p.sid != sid && p.created.elapsed() < TTL);
        while previews.len() >= 4 {
            let oldest = previews
                .iter()
                .min_by_key(|(_, p)| p.created)
                .map(|(k, _)| k.clone())
                .unwrap();
            previews.remove(&oldest);
        }
        let key = uuid::Uuid::new_v4().simple().to_string();
        let public = public_preview(&value, &key);
        previews.insert(
            key,
            Preview {
                sid: sid.into(),
                created: Instant::now(),
                value,
            },
        );
        Ok(public)
    }
    fn take(&self, sid: &str, data: &Value, kind: &str, consume: bool) -> Result<Value> {
        if !exact(data, &["preview_id"]) || !data["preview_id"].is_string() {
            return Err(invalid("Use the reviewed preview ID."));
        }
        let key = text(data, "preview_id")?;
        let mut previews = self
            .previews
            .lock()
            .map_err(|_| invalid("Delivery preview lock failed"))?;
        let p = previews
            .get(key)
            .filter(|p| p.sid == sid && p.value["kind"] == kind && p.created.elapsed() < TTL)
            .ok_or_else(|| conflict("The preview expired or was already used. Preview again."))?;
        if p.value["can_apply"] != true {
            return Err(conflict("Resolve the preview conflicts before applying."));
        }
        let value = p.value.clone();
        if consume {
            previews.remove(key);
        }
        Ok(value)
    }
    pub async fn current_preview(&self, sid: &str) -> Result<Value> {
        let found = {
            let previews = self
                .previews
                .lock()
                .map_err(|_| invalid("Delivery preview lock failed"))?;
            previews
                .iter()
                .find(|(_, p)| {
                    p.sid == sid && p.value["kind"] == "transfer" && p.created.elapsed() < TTL
                })
                .map(|(k, p)| (k.clone(), p.value.clone()))
        };
        let Some((key, p)) = found else {
            return Ok(Value::Null);
        };
        let mut out = public_preview(&p, &key);
        let check = self.target_check(&p)?;
        out["target_check"] = check.clone();
        if check["status"] == "passed"
            && let Err(e) = self.validate_transfer(sid, &p).await
        {
            out["can_apply"] = false.into();
            out["notice"] = e.message.into();
        }
        Ok(out)
    }
    fn target_check(&self, p: &Value) -> Result<Value> {
        match p["_check_id"].as_str() {
            Some(id) => Ok(self.results.check(id, None)?.unwrap_or(Value::Null)),
            None => Ok(Value::Null),
        }
    }
    async fn patch(&self, root: &Path, before: &str, after: &str) -> Result<String> {
        Ok(self
            .git(
                root,
                vec![
                    "diff".into(),
                    "--no-ext-diff".into(),
                    "--no-textconv".into(),
                    "--no-renames".into(),
                    "--full-index".into(),
                    before.into(),
                    after.into(),
                    "--".into(),
                    ".".into(),
                ],
                &BTreeMap::new(),
                &[0],
                false,
            )
            .await?
            .output)
    }
    pub async fn get(&self, sid: &str) -> Result<Value> {
        self.store.get(sid)?;
        let lock = self.results.backend.operation_lock();
        let _guard = lock.lock().await;
        let outcome=async{self.results.backend.idle()?;let(s,root,info)=self.source(sid).await?;let pending=self.pending(&root,&info).await?;let branches=self.branches(&root).await?.into_iter().filter(|b|b["name"]!=info["branch"]).collect::<Vec<_>>();Ok::<Value,HarnessError>(json!({"available":true,"preview":self.current_preview(sid).await?,"workspace":root,"branch":info["branch"],"head":info["head"],"snapshot_id":pending["snapshot_id"],"files":pending["files"],"commits":self.commits(&s,&root,&info).await?,"branches":branches,"notice":"Pending files are compared with current HEAD. Whole-file selection includes staged and unstaged content; unselected index entries and files are preserved. Local Git only."}))}.await;
        match outcome {
            Ok(v) => Ok(v),
            Err(e) => Ok(
                json!({"available":false,"reason":e.message,"preview":self.current_preview(sid).await?}),
            ),
        }
    }
    pub async fn preview_commit(&self, sid: &str, data: &Value) -> Result<Value> {
        if !exact(data, &["snapshot_id", "paths", "message"]) {
            return Err(invalid(
                "Select files, a current snapshot and a commit message.",
            ));
        }
        let paths = data["paths"]
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= MAX_FILES)
            .ok_or_else(|| invalid("Select distinct supported file paths."))?
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|p| safe_path(p))
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("Select distinct supported file paths."))
            })
            .collect::<Result<Vec<_>>>()?;
        if paths.iter().collect::<BTreeSet<_>>().len() != paths.len() {
            return Err(invalid("Select distinct supported file paths."));
        }
        let message = data["message"]
            .as_str()
            .filter(|s| {
                !s.trim().is_empty()
                    && s.len() <= 4000
                    && !s.chars().any(|c| c < ' ' && !['\n', '\t'].contains(&c))
            })
            .ok_or_else(|| invalid("Enter a commit message of 1–4000 UTF-8 bytes."))?;
        let lock = self.results.backend.operation_lock();
        let _guard = lock.lock().await;
        self.results.backend.idle()?;
        let (_, root, info) = self.source(sid).await?;
        let pending = self.pending(&root, &info).await?;
        if data["snapshot_id"] != pending["snapshot_id"] {
            return Err(conflict(
                "Workspace changed. Refresh pending files before committing.",
            ));
        }
        let available = pending["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["path"].as_str())
            .collect::<BTreeSet<_>>();
        if paths.iter().any(|p| !available.contains(p.as_str())) {
            return Err(invalid("Select only current pending files."));
        }
        let temporary = tempfile::Builder::new()
            .prefix("index-")
            .tempdir_in(&self.folder)?;
        let index = temporary.path().join("index");
        let env = BTreeMap::from([(
            "GIT_INDEX_FILE".into(),
            index.to_string_lossy().into_owned(),
        )]);
        self.git(
            &root,
            vec!["read-tree".into(), text(&info, "head")?.into()],
            &env,
            &[0],
            false,
        )
        .await?;
        let mut args = vec!["add".into(), "--".into()];
        args.extend(paths.clone());
        self.git(&root, args, &env, &[0], false).await?;
        let tree = self
            .git(&root, vec!["write-tree".into()], &env, &[0], false)
            .await?
            .output
            .trim()
            .to_owned();
        if self
            .pending(&root, &self.results.git_info(&root, false).await?)
            .await?["snapshot_id"]
            != pending["snapshot_id"]
        {
            return Err(conflict(
                "Workspace changed while preparing the commit. Preview again.",
            ));
        }
        if tree
            == self
                .string(
                    &root,
                    vec![
                        "rev-parse".into(),
                        format!("{}^{{tree}}", text(&info, "head")?),
                    ],
                )
                .await?
        {
            return Err(conflict("Selected files have no changes to commit."));
        }
        self.remember(sid,json!({"kind":"commit","can_apply":true,"source_head":info["head"],"message":message.trim(),"files":self.names(&root,text(&info,"head")?,Some(&tree)).await?,"diff":self.patch(&root,text(&info,"head")?,&tree).await?,"checks":self.checks(sid).await?,"notice":"Only the reviewed file contents will be committed. Unselected changes stay in the worktree. Git hooks and signing are disabled; use the independent checks above.","_snapshot":pending["snapshot_id"],"_tree":tree,"_paths":paths,"_branch":info["branch"]}))
    }
    async fn recover_one(&self, name: &str) -> Result<()> {
        let mut journals = PublicationFiles::new(&self.folder)?;
        let raw = journals
            .read(name, 65536)?
            .ok_or_else(|| invalid("Delivery journal disappeared"))?;
        let value = infra_core::strict_json::parse(&raw, 65536, 128, 65536)?;
        if !(exact(
            &value,
            &[
                "sid",
                "index",
                "branch",
                "old",
                "commit",
                "before",
                "before_exists",
                "before_mode",
                "after",
                "identity",
            ],
        ) || exact(
            &value,
            &[
                "sid", "index", "branch", "old", "commit", "before", "after", "identity",
            ],
        )) {
            return Err(invalid("Delivery journal has an invalid schema"));
        }
        let sid = text(&value, "sid")?;
        let s = self.store.get(sid)?;
        let root = self.results.backend.workspace(&s)?;
        let (index, body, mode) = self.index(&root).await?;
        if index.to_str() != value["index"].as_str() {
            return Err(conflict("Git index location changed during delivery."));
        }
        let parent = index
            .parent()
            .ok_or_else(|| invalid("Git index parent missing"))?;
        let index_name = index
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| invalid("Git index name invalid"))?;
        let lock_name = format!("{index_name}.lock");
        let mut files = PublicationFiles::new(parent)?;
        let lock = files.read(&lock_name, IMAGE)?;
        let head = self
            .string(
                &root,
                vec![
                    "rev-parse".into(),
                    format!("refs/heads/{}", text(&value, "branch")?),
                ],
            )
            .await?;
        let before = text(&value, "before")?;
        let after = text(&value, "after")?;
        let current = sha(body.as_deref().unwrap_or(&[]));
        let before_exists = if value.get("before_exists").is_some() {
            value["before_exists"]
                .as_bool()
                .ok_or_else(|| invalid("Invalid delivery index image"))?
        } else {
            body.is_some()
        };
        if lock.is_none() {
            if head == text(&value, "commit")? && current == after {
                journals.unlink_expected_limit(name, &raw, 65536)?;
                return Ok(());
            }
            return Err(conflict(
                "The interrupted index lock is missing; inspect Git before continuing.",
            ));
        }
        let updated = lock.unwrap();
        let identity = files
            .image_identity(&lock_name)?
            .ok_or_else(|| conflict("The owned index lock disappeared"))?;
        if value["identity"] != json!([identity.0, identity.1]) || sha(&updated) != after {
            return Err(conflict(
                "The Git index changed during recovery; inspect it before continuing.",
            ));
        }
        if head == text(&value, "commit")? {
            if current == before && body.is_some() == before_exists {
                files.write_expected_mode_checked(
                    index_name,
                    &updated,
                    body.as_deref(),
                    mode,
                    value["before_mode"]
                        .as_u64()
                        .map(|m| m as u32)
                        .or(mode)
                        .unwrap_or(0o600),
                    IMAGE,
                )?;
            } else if current != after {
                return Err(conflict(
                    "The Git index changed during recovery; inspect it before continuing.",
                ));
            }
            self.store.event(sid,&json!({"kind":"status","text":format!("Recovered the index for delivered commit {head}.")}))?;
        } else if head == text(&value, "old")? {
            if current != before || body.is_some() != before_exists {
                return Err(conflict("The original index changed during recovery."));
            }
        } else {
            return Err(conflict(
                "The branch moved during recovery; inspect the interrupted commit.",
            ));
        }
        files.unlink_expected_limit(&lock_name, &updated, IMAGE)?;
        journals.unlink_expected_limit(name, &raw, 65536)?;
        Ok(())
    }
    pub async fn commit(&self, sid: &str, data: &Value) -> Result<Value> {
        let lock = self.results.backend.operation_lock();
        let _guard = lock.lock().await;
        self.results.backend.idle()?;
        let preview = self.take(sid, data, "commit", true)?;
        let (_, root, info) = self.source(sid).await?;
        if self.pending(&root, &info).await?["snapshot_id"] != preview["_snapshot"] {
            return Err(conflict("Workspace changed. Preview the commit again."));
        }
        let (index, original, mode) = self.index(&root).await?;
        let parent = index
            .parent()
            .ok_or_else(|| invalid("Git index parent missing"))?;
        let name = index
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| invalid("Git index name invalid"))?;
        let lock_name = format!("{name}.lock");
        let mut files = PublicationFiles::new(parent)?;
        files
            .write_expected_mode_checked(&lock_name, &[], None, None, 0o600, IMAGE)
            .map_err(|_| conflict("Git is busy: index.lock already exists or is unsafe."))?;
        let mut owned = Vec::new();
        let mut journal: Option<String> = None;
        let outcome=async{
   if self.pending(&root,&self.results.git_info(&root,false).await?).await?["snapshot_id"]!=preview["_snapshot"]{return Err(conflict("Workspace changed. Preview the commit again."));}let temp=tempfile::Builder::new().prefix("commit-").tempdir_in(&self.folder)?;let mut stage=PublicationFiles::new(temp.path())?;let message=format!("{}\n",text(&preview,"message")?);stage.write_expected_limit("message",message.as_bytes(),None,8192)?;
   let commit=self.string(&root,vec!["commit-tree".into(),text(&preview,"_tree")?.into(),"-p".into(),text(&info,"head")?.into(),"-F".into(),temp.path().join("message").to_string_lossy().into_owned()]).await?;
   if let Some(original)=&original{stage.write_expected_limit("index",original,None,IMAGE)?;}let env=BTreeMap::from([("GIT_INDEX_FILE".into(),temp.path().join("index").to_string_lossy().into_owned())]);if original.is_none(){self.git(&root,vec!["read-tree".into(),"--empty".into()],&env,&[0],false).await?;}let mut args=vec!["reset".into(),"--quiet".into(),commit.clone(),"--".into()];args.extend(serde_json::from_value::<Vec<String>>(preview["_paths"].clone())?);self.git(&root,args,&env,&[0],false).await?;let updated=stage.read("index",IMAGE)?.ok_or_else(||invalid("Prepared index missing"))?;files.write_expected_mode_checked(&lock_name,&updated,Some(&owned),Some(0o600),0o600,IMAGE)?;owned=updated;let identity=files.image_identity(&lock_name)?.ok_or_else(||invalid("Prepared lock missing"))?;
   let value=json!({"sid":sid,"index":index,"branch":info["branch"],"old":info["head"],"commit":commit,"before":sha(original.as_deref().unwrap_or(&[])),"before_exists":original.is_some(),"before_mode":mode,"after":sha(&owned),"identity":[identity.0,identity.1]});let journal_name=format!("transaction-{}.json",uuid::Uuid::new_v4().simple());let mut records=PublicationFiles::new(&self.folder)?;records.write_expected_limit(&journal_name,&serde_json::to_vec(&value)?,None,65536)?;journal=Some(journal_name.clone());
   if self.string(&root,vec!["symbolic-ref".into(),"HEAD".into()]).await?!=format!("refs/heads/{}",text(&info,"branch")?){return Err(conflict("The source branch changed."));}self.git(&root,vec!["update-ref".into(),"-m".into(),format!("Harness: {}",text(&preview,"message")?.lines().next().unwrap_or("")),format!("refs/heads/{}",text(&info,"branch")?),commit.clone(),text(&info,"head")?.into()],&BTreeMap::new(),&[0],false).await?;self.recover_one(&journal_name).await?;journal=None;self.store.event(sid,&json!({"kind":"status","text":format!("Committed selected files: {commit}.")}))?;Ok(json!({"ok":true,"commit":commit,"detail":"Selected files committed; unselected edits and staging are preserved."}))
  }.await;
        if let Some(name) = journal {
            if let Err(e) = self.recover_one(&name).await {
                self.blocked.lock().map_err(|_|invalid("Delivery lock failed"))?.insert(sid.into(),format!("An interrupted Git commit needs inspection before another delivery. {}",e.message));
            }
        } else if files.read(&lock_name, IMAGE)?.as_deref() == Some(owned.as_slice()) {
            files.unlink_expected_limit(&lock_name, &owned, IMAGE)?;
        }
        outcome
    }
    async fn target(&self, root: &Path, branch: &str) -> Result<Value> {
        if !self
            .branches(root)
            .await?
            .iter()
            .any(|b| b["name"] == branch)
        {
            return Err(invalid("Select an existing local target branch."));
        }
        let head = self
            .string(
                root,
                vec!["rev-parse".into(), format!("refs/heads/{branch}")],
            )
            .await?;
        let fields = self
            .git(
                root,
                vec![
                    "worktree".into(),
                    "list".into(),
                    "--porcelain".into(),
                    "-z".into(),
                ],
                &BTreeMap::new(),
                &[0],
                false,
            )
            .await?
            .output;
        let mut entry = BTreeMap::<String, String>::new();
        let mut matches = vec![];
        for f in fields.split('\0') {
            if f.is_empty() {
                if entry
                    .get("branch")
                    .is_some_and(|b| b == &format!("refs/heads/{branch}"))
                {
                    matches.push(entry.clone());
                }
                entry.clear();
            } else {
                let (k, v) = f.split_once(' ').unwrap_or((f, ""));
                entry.insert(k.into(), v.into());
            }
        }
        if matches.len() > 1 {
            return Err(conflict("The target branch is checked out more than once."));
        }
        let workspace = matches.first().and_then(|m| m.get("worktree")).cloned();
        if let Some(work) = &workspace {
            let path = Path::new(work);
            let info = self.results.git_info(path, true).await?;
            if info["is_git"] != true
                || info["head"] != head
                || self
                    .string(path, vec!["symbolic-ref".into(), "HEAD".into()])
                    .await?
                    != format!("refs/heads/{branch}")
            {
                return Err(conflict("The target checkout changed. Preview again."));
            }
            self.settled(path).await?;
            if info["dirty"] == true {
                return Err(conflict(
                    "The target worktree has uncommitted changes. Commit or move them before delivery.",
                ));
            }
        }
        Ok(json!({"branch":branch,"head":head,"workspace":workspace}))
    }
    async fn remove_worktree(&self, root: &Path, work: &Path) -> Result<()> {
        self.git(
            root,
            vec![
                "worktree".into(),
                "remove".into(),
                "--force".into(),
                work.to_string_lossy().into_owned(),
            ],
            &BTreeMap::new(),
            &[0, 128],
            true,
        )
        .await?;
        Ok(())
    }
    pub async fn preview_transfer(&self, sid: &str, data: &Value) -> Result<Value> {
        if !(exact(data, &["commit", "target_branch"])
            || exact(data, &["commit", "target_branch", "check"]))
        {
            return Err(invalid("Select one commit and a local target branch."));
        }
        if !data["check"].is_null() {
            Results::check_command(&data["check"])?;
        }
        let commit = text(data, "commit")?;
        if !owned_hash(commit) {
            return Err(invalid("Select a full commit ID."));
        }
        let lock = self.results.backend.operation_lock();
        let _guard = lock.lock().await;
        self.results.backend.idle()?;
        let (s, root, info) = self.source(sid).await?;
        let pending = self.pending(&root, &info).await?;
        if !self
            .commits(&s, &root, &info)
            .await?
            .iter()
            .any(|c| c["sha"] == commit)
        {
            return Err(invalid(
                "Select a single-parent commit created after this session baseline.",
            ));
        }
        if data["target_branch"] == info["branch"] {
            return Err(invalid("Select a different target branch."));
        }
        let target = self.target(&root, text(data, "target_branch")?).await?;
        let temp = tempfile::Builder::new()
            .prefix("transfer-")
            .tempdir_in(&self.folder)?;
        let work = temp.path().join("worktree");
        let outcome=async{
   self.git(&root,vec!["worktree".into(),"add".into(),"--quiet".into(),"--detach".into(),work.to_string_lossy().into_owned(),text(&target,"head")?.into()],&BTreeMap::new(),&[0],false).await?;
   let picked=self.git(&work,vec!["cherry-pick".into(),"--no-edit".into(),commit.into()],&BTreeMap::new(),&[0,1,128],false).await?;let conflicts=self.string(&work,vec!["diff".into(),"--name-only".into(),"--diff-filter=U".into()]).await?.lines().map(str::to_owned).collect::<Vec<_>>();let candidate=if picked.exit_code==Some(0){Some(self.string(&work,vec!["rev-parse".into(),"HEAD".into()]).await?)}else{None};let parent=self.string(&root,vec!["rev-parse".into(),format!("{commit}^")]).await?;let display=Path::new(text(&info,"root")?);let before=candidate.as_ref().map(|_|text(&target,"head")).transpose()?.unwrap_or(&parent);let after=candidate.as_deref().unwrap_or(commit);let notice=if candidate.is_some(){"Source checks do not certify the resulting target commit. If a target check is selected, it must pass before Apply. Apply fast-forwards only the reviewed target branch.".into()}else{format!("This commit cannot be transferred cleanly. Target files are unchanged. {}",picked.output.chars().take(2000).collect::<String>())};Ok::<Value,HarnessError>(json!({"kind":"transfer","can_apply":candidate.is_some(),"commit":commit,"source_head":info["head"],"target_branch":target["branch"],"target_head":target["head"],"candidate":candidate,"check":data["check"],"files":self.names(display,before,Some(after)).await?,"diff":self.patch(display,before,after).await?,"conflicts":conflicts,"checks":self.checks(sid).await?,"notice":notice,"_candidate":candidate,"_target":target,"_snapshot":pending["snapshot_id"]}))
  }.await;
        let preview = self
            .finish_owned_worktree(
                sid,
                temp,
                &work,
                "transfer",
                outcome,
                self.remove_worktree(&root, &work),
            )
            .await?;
        self.remember(sid, preview)
    }
    async fn validate_transfer(
        &self,
        sid: &str,
        preview: &Value,
    ) -> Result<(Value, PathBuf, Value)> {
        let (s, root, info) = self.source(sid).await?;
        if self.pending(&root, &info).await?["snapshot_id"] != preview["_snapshot"] {
            return Err(conflict(
                "Source workspace changed. Preview the transfer again.",
            ));
        }
        let target = self.target(&root, text(preview, "target_branch")?).await?;
        if target != preview["_target"] {
            return Err(conflict(
                "Target branch or checkout changed. Preview the transfer again.",
            ));
        }
        if self
            .string(
                &root,
                vec![
                    "rev-parse".into(),
                    format!("{}^", text(preview, "_candidate")?),
                ],
            )
            .await?
            != text(&target, "head")?
        {
            return Err(conflict("The preview commit is unavailable or changed."));
        }
        Ok((s, root, target))
    }
    pub async fn start_check(&self, sid: &str, data: &Value) -> Result<Value> {
        let lock = self.results.backend.operation_lock();
        let guard = lock.lock().await;
        self.results.backend.idle()?;
        let p = self.take(sid, data, "transfer", false)?;
        self.validate_transfer(sid, &p).await?;
        if p["check"].is_null() {
            return Err(invalid("Select a check command and prepare a new preview."));
        }
        let c=self.results.prepare_check(sid,&p["check"],&json!({"kind":"target","preview_id":data["preview_id"],"candidate":p["_candidate"],"target_branch":p["target_branch"],"target_head":p["target_head"],"snapshot_id":null,"snapshot_complete":false}))?;
        {
            let mut previews = self
                .previews
                .lock()
                .map_err(|_| invalid("Delivery preview lock failed"))?;
            let p = previews
                .get_mut(text(data, "preview_id")?)
                .ok_or_else(|| conflict("The preview expired."))?;
            p.value["_check_id"] = c["id"].clone();
        }
        drop(guard);
        self.results.submit_check(&c)?;
        Ok(json!({"session":self.store.get(sid)?}))
    }
    pub async fn apply(&self, sid: &str, data: &Value) -> Result<Value> {
        let lock = self.results.backend.operation_lock();
        let _guard = lock.lock().await;
        self.results.backend.idle()?;
        let p = self.take(sid, data, "transfer", false)?;
        let (_, root, target) = self.validate_transfer(sid, &p).await?;
        let check = self.target_check(&p)?;
        if !p["check"].is_null()
            && (check["status"] != "passed"
                || check["workspace_changed"] == true
                || check["candidate"] != p["_candidate"])
        {
            return Err(conflict(
                "Run the selected target check successfully before applying this preview.",
            ));
        }
        self.take(sid, data, "transfer", true)?;
        let candidate = text(&p, "_candidate")?.to_owned();
        let temp = tempfile::Builder::new()
            .prefix("apply-")
            .tempdir_in(&self.folder)?;
        let created = target["workspace"].is_null();
        let work = target["workspace"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| temp.path().join("worktree"));
        let outcome=async{
   if created{self.git(&root,vec!["worktree".into(),"add".into(),"--quiet".into(),work.to_string_lossy().into_owned(),text(&target,"branch")?.into()],&BTreeMap::new(),&[0],false).await?;}let current=self.target(&root,text(&target,"branch")?).await?;if current["head"]!=target["head"]||current["workspace"].as_str()!=work.to_str(){return Err(conflict("Target changed while preparing delivery."));}self.store.event(sid,&json!({"kind":"status","text":format!("Applying reviewed commit {} to {} at {}.",text(&p,"commit")?,text(&target,"branch")?,text(&target,"head")?)}))?;
   self.git(&work,vec!["merge".into(),"--ff-only".into(),"--no-edit".into(),"--no-overwrite-ignore".into(),candidate.clone()],&BTreeMap::new(),&[0],false).await?;if self.string(&work,vec!["rev-parse".into(),"HEAD".into()]).await?!=candidate{return Err(conflict("Inspect the target: Git did not finish at the reviewed commit."));}Ok::<(),HarnessError>(())
  }.await;
        self.finish_owned_worktree(sid, temp, &work, "delivery", outcome, async {
            if created {
                self.remove_worktree(&root, &work).await
            } else {
                Ok(())
            }
        })
        .await?;
        let detail = if check.is_object() {
            "Selected target check passed for this commit."
        } else {
            "No target check was selected."
        };
        self.store.event(sid,&json!({"kind":"status","text":format!("Delivered {candidate} to {}. {detail}",text(&target,"branch")?)}))?;
        Ok(
            json!({"ok":true,"commit":candidate,"branch":target["branch"],"detail":format!("Reviewed commit applied locally. {detail}")}),
        )
    }
    async fn finish_owned_worktree<T: Send>(
        &self,
        sid: &str,
        temp: tempfile::TempDir,
        work: &Path,
        purpose: &'static str,
        outcome: Result<T>,
        cleanup: impl std::future::Future<Output = Result<()>> + Send,
    ) -> Result<T> {
        let outcome = match outcome {
            Err(error) if error.code == "cleanup-unconfirmed" => {
                // The native process may still use this directory. Do not
                // poll a cleanup future that can launch Git or remove it.
                self.results
                    .backend
                    .shutdown_cancel()
                    .store(true, std::sync::atomic::Ordering::Release);
                let _ = self.retain_owned_worktree(sid, temp, work, purpose);
                return Err(error);
            }
            outcome => outcome,
        };
        let Err(cleanup_error) = cleanup.await else {
            return outcome;
        };
        if cleanup_error.code == "cleanup-unconfirmed" {
            self.results
                .backend
                .shutdown_cancel()
                .store(true, std::sync::atomic::Ordering::Release);
        }
        let (retained, blocked_error) = self.retain_owned_worktree(sid, temp, work, purpose);
        if cleanup_error.code == "cleanup-unconfirmed" {
            return Err(cleanup_error);
        }
        if let Some(error) = blocked_error {
            return Err(error);
        }
        Err(conflict(format!(
            "Could not remove the temporary {purpose} worktree at {}: {}",
            retained.display(),
            cleanup_error.message
        )))
    }
    fn retain_owned_worktree(
        &self,
        sid: &str,
        temp: tempfile::TempDir,
        work: &Path,
        purpose: &str,
    ) -> (PathBuf, Option<HarnessError>) {
        let retained = temp.keep();
        let mut reason = format!(
            "A temporary {purpose} worktree needs inspection at {}",
            retained.display()
        );
        if !work.starts_with(&retained) {
            reason.push_str(&format!(". The affected workspace is {}.", work.display()));
        }
        let blocked_error = match self.blocked.lock() {
            Ok(mut blocked) => {
                blocked.insert(sid.to_owned(), reason);
                None
            }
            Err(poisoned) => {
                // Keep the inspection receipt without clearing poison or
                // allowing its secondary error to mask lost process custody.
                poisoned.into_inner().insert(sid.to_owned(), reason);
                Some(invalid("Delivery lock failed"))
            }
        };
        (retained, blocked_error)
    }
    pub async fn act(&self, sid: &str, data: &Value) -> Result<Value> {
        let mut body = data
            .as_object()
            .ok_or_else(|| invalid("Invalid delivery action."))?
            .clone();
        let action = body
            .remove("action")
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or_else(|| invalid("Invalid delivery action."))?;
        let body = Value::Object(body);
        match action.as_str() {
            "preview_commit" => self.preview_commit(sid, &body).await,
            "commit" => self.commit(sid, &body).await,
            "preview_transfer" => self.preview_transfer(sid, &body).await,
            "apply" => self.apply(sid, &body).await,
            "check" => self.start_check(sid, &body).await,
            _ => Err(invalid("Invalid delivery action.")),
        }
    }
}
fn public_preview(value: &Value, key: &str) -> Value {
    let mut out = json!({});
    for (k, v) in value.as_object().into_iter().flatten() {
        if !k.starts_with('_') {
            out[k] = v.clone();
        }
    }
    out["preview_id"] = key.into();
    out
}
impl TargetCheckWorkflow for Delivery {
    fn run_target<'a>(&'a self, check: &'a mut Value, cancel: Arc<AtomicBool>) -> TargetReply<'a> {
        Box::pin(async move {
            let sid = text(check, "session_id")?.to_owned();
            let preview_id = text(check, "preview_id")?.to_owned();
            let p = self.take(&sid, &json!({"preview_id":preview_id}), "transfer", false)?;
            if p["_check_id"] != check["id"] {
                return Err(conflict("This target check was superseded."));
            }
            let (_, root, _) = self.validate_transfer(&sid, &p).await?;
            let info = self.results.git_info(&root, false).await?;
            let relative = root
                .strip_prefix(Path::new(text(&info, "root")?))
                .map_err(|_| conflict("Project directory escaped its Git root."))?
                .to_owned();
            let temp = tempfile::Builder::new()
                .prefix("check-")
                .tempdir_in(&self.folder)?;
            let work = temp.path().join("worktree");
            let outcome = async {
                self.git(
                    &root,
                    vec![
                        "worktree".into(),
                        "add".into(),
                        "--quiet".into(),
                        "--detach".into(),
                        work.to_string_lossy().into_owned(),
                        text(check, "candidate")?.into(),
                    ],
                    &BTreeMap::new(),
                    &[0],
                    false,
                )
                .await?;
                let before = self
                    .pending(&work, &self.results.git_info(&work, false).await?)
                    .await?["snapshot_id"]
                    .clone();
                let check_root = work.join(relative);
                PublicationFiles::new(&check_root)?.check()?;
                let argv = serde_json::from_value::<Vec<String>>(check["argv"].clone())?;
                let timeout = check["timeout"]
                    .as_u64()
                    .ok_or_else(|| invalid("Check timeout is missing"))?;
                let result = self
                    .results
                    .capture(
                        &argv,
                        &check_root,
                        timeout,
                        cancel,
                        &BTreeMap::new(),
                        |output| {
                            check["output"] = output.into();
                            self.results.save_check(check)
                        },
                    )
                    .await?;
                if result.reason.is_some() {
                    return Ok(result);
                }
                check["workspace_changed"] = (self
                    .pending(&work, &self.results.git_info(&work, false).await?)
                    .await?["snapshot_id"]
                    != before)
                    .into();
                let mut result = result;
                if check["workspace_changed"] == true {
                    result.reason = Some("workspace_changed".into());
                }
                self.take(&sid, &json!({"preview_id":preview_id}), "transfer", false)?;
                self.validate_transfer(&sid, &p).await?;
                Ok::<Capture, HarnessError>(result)
            }
            .await;
            self.finish_owned_worktree(
                &sid,
                temp,
                &work,
                "target-check",
                outcome,
                self.remove_worktree(&root, &work),
            )
            .await
        })
    }
}
impl Domain for Delivery {
    fn execute<'a>(&'a self, op: &'a str, r: Value) -> Reply<'a> {
        Box::pin(async move {
            let sid = r["params"]["session_id"]
                .as_str()
                .ok_or_else(|| invalid("Missing session ID"))?
                .to_owned();
            let value = match op {
                "delivery.get" => self.get(&sid).await?,
                "delivery.act" => {
                    let owner = self.clone();
                    let body = r["body"].clone();
                    tokio::spawn(async move{owner.act(&sid,&body).await}).await.map_err(|_|HarnessError::unavailable("The owned local Git delivery task did not finish; inspect its recovery journal."))??
                }
                _ => return Err(HarnessError::missing("Unknown delivery operation")),
            };
            Ok(Payload::Json(value))
        })
    }
}

#[cfg(test)]
mod target_cleanup_boundary_tests {
    use super::*;
    use crate::sessions::generation_completion_tests::{
        Fixture as SessionFixture, assert_cleanup_failure_stops,
    };
    use std::sync::atomic::Ordering;

    struct Fixture {
        delivery: Delivery,
        _sessions: SessionFixture,
    }
    impl Fixture {
        fn new() -> Self {
            let sessions = SessionFixture::new();
            let delivery = sessions.delivery();
            Self {
                delivery,
                _sessions: sessions,
            }
        }
        fn owned_temp(&self) -> (tempfile::TempDir, PathBuf) {
            let temp = tempfile::Builder::new()
                .prefix("check-")
                .tempdir_in(&self.delivery.folder)
                .unwrap();
            let path = temp.path().to_owned();
            std::fs::create_dir(path.join("worktree")).unwrap();
            std::fs::write(
                path.join("worktree/owned-receipt"),
                b"owned target-check fixture",
            )
            .unwrap();
            (temp, path)
        }
        fn assert_retained(&self, sid: &str, path: &Path) {
            assert_eq!(
                std::fs::read(path.join("worktree/owned-receipt")).unwrap(),
                b"owned target-check fixture"
            );
            let blocked = self
                .delivery
                .blocked
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let retained = path.display().to_string();
            assert!(blocked.get(sid).unwrap().contains(retained.as_str()));
        }
        fn stopped(&self) -> bool {
            self.delivery
                .results
                .backend
                .shutdown_cancel()
                .load(Ordering::Acquire)
        }
    }
    fn fatal(message: &str) -> HarnessError {
        HarnessError {
            status: 503,
            code: "cleanup-unconfirmed",
            message: message.into(),
        }
    }
    fn completed() -> Result<Capture> {
        Ok(Capture {
            output: String::new(),
            exit_code: Some(0),
            reason: None,
            seconds: 0.0,
        })
    }
    async fn cleanup(result: Result<()>, polled: Arc<AtomicBool>) -> Result<()> {
        polled.store(true, Ordering::Release);
        result
    }
    fn poison(delivery: &Delivery) {
        let blocked = delivery.blocked.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = blocked.lock().unwrap();
                panic!("isolated delivery blocked-map poison");
            })
            .join()
            .is_err()
        );
        assert!(delivery.blocked.is_poisoned());
    }

    #[tokio::test]
    async fn original_target_fatal_never_polls_cleanup_and_keeps_owned_files() {
        for cleanup_failed in [false, true] {
            let fixture = Fixture::new();
            let (temp, path) = fixture.owned_temp();
            let work = path.join("worktree");
            let polled = Arc::new(AtomicBool::new(false));
            let result = if cleanup_failed {
                Err(conflict("ordinary removal refusal"))
            } else {
                Ok(())
            };
            let error = fixture
                .delivery
                .finish_owned_worktree(
                    "sid",
                    temp,
                    &work,
                    "target-check",
                    Err::<Capture, _>(fatal("original target fatal")),
                    cleanup(result, polled.clone()),
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, "cleanup-unconfirmed");
            assert_eq!(error.message, "original target fatal");
            assert!(!polled.load(Ordering::Acquire));
            assert!(fixture.stopped());
            fixture.assert_retained("sid", &path);
        }
    }

    #[tokio::test]
    async fn worktree_cleanup_fatal_preserves_original_error_and_owned_path() {
        let fixture = Fixture::new();
        let (temp, path) = fixture.owned_temp();
        let work = path.join("worktree");
        let polled = Arc::new(AtomicBool::new(false));
        let error = fixture
            .delivery
            .finish_owned_worktree(
                "sid",
                temp,
                &work,
                "target-check",
                completed(),
                cleanup(Err(fatal("original worktree fatal")), polled.clone()),
            )
            .await
            .unwrap_err();
        assert!(polled.load(Ordering::Acquire));
        assert!(fixture.stopped());
        assert_eq!(error.code, "cleanup-unconfirmed");
        assert_eq!(error.message, "original worktree fatal");
        fixture.assert_retained("sid", &path);
    }

    #[tokio::test]
    async fn target_fatal_prevents_a_failing_cleanup_future_from_starting() {
        let fixture = Fixture::new();
        let (temp, path) = fixture.owned_temp();
        let work = path.join("worktree");
        let polled = Arc::new(AtomicBool::new(false));
        let error = fixture
            .delivery
            .finish_owned_worktree(
                "sid",
                temp,
                &work,
                "target-check",
                Err::<Capture, _>(fatal("first target fatal")),
                cleanup(Err(fatal("unobserved cleanup failure")), polled.clone()),
            )
            .await
            .unwrap_err();
        assert!(!polled.load(Ordering::Acquire));
        assert!(fixture.stopped());
        assert_eq!(error.code, "cleanup-unconfirmed");
        assert_eq!(error.message, "first target fatal");
        fixture.assert_retained("sid", &path);
    }

    #[tokio::test]
    async fn ordinary_worktree_failure_is_polled_and_keeps_the_conflict_contract() {
        let fixture = Fixture::new();
        let (temp, path) = fixture.owned_temp();
        let work = path.join("worktree");
        let polled = Arc::new(AtomicBool::new(false));
        let error = fixture
            .delivery
            .finish_owned_worktree(
                "sid",
                temp,
                &work,
                "target-check",
                completed(),
                cleanup(Err(conflict("ordinary removal refusal")), polled.clone()),
            )
            .await
            .unwrap_err();
        assert!(polled.load(Ordering::Acquire));
        assert!(!fixture.stopped());
        assert_eq!(error.code, "conflict");
        assert!(error.message.contains("ordinary removal refusal"));
        let retained = path.display().to_string();
        assert!(error.message.contains(retained.as_str()));
        fixture.assert_retained("sid", &path);
    }

    #[tokio::test]
    async fn poisoned_blocked_map_cannot_hide_fatal_errors_or_start_their_cleanup() {
        for (target_fatal, cleanup_fatal) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let fixture = Fixture::new();
            poison(&fixture.delivery);
            let (temp, path) = fixture.owned_temp();
            let work = path.join("worktree");
            let polled = Arc::new(AtomicBool::new(false));
            let outcome = if target_fatal {
                Err(fatal("target fatal despite poison"))
            } else {
                completed()
            };
            let removal = if cleanup_fatal {
                Err(fatal("cleanup fatal despite poison"))
            } else {
                Err(conflict("ordinary removal failure"))
            };
            let error = fixture
                .delivery
                .finish_owned_worktree(
                    "sid",
                    temp,
                    &work,
                    "target-check",
                    outcome,
                    cleanup(removal, polled.clone()),
                )
                .await
                .unwrap_err();
            assert_eq!(polled.load(Ordering::Acquire), !target_fatal);
            assert_eq!(fixture.stopped(), target_fatal || cleanup_fatal);
            if target_fatal || cleanup_fatal {
                assert_eq!(error.code, "cleanup-unconfirmed");
                assert_eq!(
                    error.message,
                    if target_fatal {
                        "target fatal despite poison"
                    } else {
                        "cleanup fatal despite poison"
                    }
                );
            } else {
                assert_eq!(error.code, "invalid-request");
            }
            assert!(fixture.delivery.blocked.is_poisoned());
            fixture.assert_retained("sid", &path);
        }
    }

    #[tokio::test]
    async fn retained_delivery_fatal_reaches_stale_cancelled_check_admission_owner() {
        let fixture = Fixture::new();
        poison(&fixture.delivery);
        let (temp, path) = fixture.owned_temp();
        let work = path.join("worktree");
        let polled = Arc::new(AtomicBool::new(false));
        let error = fixture
            .delivery
            .finish_owned_worktree(
                "sid",
                temp,
                &work,
                "target-check",
                Err::<Capture, _>(fatal("delivery boundary unknown cleanup")),
                cleanup(Err(conflict("ordinary removal failure")), polled.clone()),
            )
            .await
            .unwrap_err();
        assert!(!polled.load(Ordering::Acquire));
        fixture.assert_retained("sid", &path);
        assert_cleanup_failure_stops(error).await;
        assert!(!polled.load(Ordering::Acquire));
        fixture.assert_retained("sid", &path);
    }

    #[tokio::test]
    async fn ordinary_cleanup_preserves_capture_preview_and_apply_return_contracts() {
        let fixture = Fixture::new();
        let (temp, path) = fixture.owned_temp();
        let work = path.join("worktree");
        let polled = Arc::new(AtomicBool::new(false));
        let capture = fixture
            .delivery
            .finish_owned_worktree(
                "sid",
                temp,
                &work,
                "target-check",
                completed(),
                cleanup(Ok(()), polled.clone()),
            )
            .await
            .unwrap();
        assert!(polled.load(Ordering::Acquire));
        assert_eq!(capture.exit_code, Some(0));
        assert!(!path.exists());

        let (temp, path) = fixture.owned_temp();
        let work = path.join("worktree");
        let polled = Arc::new(AtomicBool::new(false));
        let preview = json!({"kind":"transfer","candidate":"owned"});
        let value = fixture
            .delivery
            .finish_owned_worktree(
                "sid",
                temp,
                &work,
                "transfer",
                Ok(preview.clone()),
                cleanup(Ok(()), polled.clone()),
            )
            .await
            .unwrap();
        assert_eq!(value, preview);
        assert!(polled.load(Ordering::Acquire));
        assert!(!path.exists());

        let (temp, path) = fixture.owned_temp();
        let work = path.join("worktree");
        let polled = Arc::new(AtomicBool::new(false));
        fixture
            .delivery
            .finish_owned_worktree(
                "sid",
                temp,
                &work,
                "delivery",
                Ok(()),
                cleanup(Ok(()), polled.clone()),
            )
            .await
            .unwrap();
        assert!(polled.load(Ordering::Acquire));
        assert!(!path.exists());

        let (temp, path) = fixture.owned_temp();
        let work = path.join("worktree");
        let polled = Arc::new(AtomicBool::new(false));
        let error = fixture
            .delivery
            .finish_owned_worktree(
                "sid",
                temp,
                &work,
                "transfer",
                Err::<Value, _>(conflict("original operation refusal")),
                cleanup(Ok(()), polled.clone()),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "conflict");
        assert_eq!(error.message, "original operation refusal");
        assert!(polled.load(Ordering::Acquire));
        assert!(!path.exists());
        assert!(!fixture.stopped());
        assert!(fixture.delivery.blocked.lock().unwrap().is_empty());
    }
}
