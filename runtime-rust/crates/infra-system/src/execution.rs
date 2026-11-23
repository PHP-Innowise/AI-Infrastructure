use crate::{catalog::System, common::*};
use infra_core::bank::publication::PublicationFiles;
use infra_core::{
    BankService, BrainRepository, CreateRecord, ErrorKind, Result, Roots, RuntimeContext,
    RuntimeError, Store, UpdateRecord,
};
use serde_json::{Value, json as object};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};
pub const MAX_STATE: usize = 8 * MAX_BYTES;
pub struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
pub fn lock(root: &Path, name: &str) -> Result<Lock> {
    let mut custody = PublicationFiles::new(root)?;
    let before = custody.read(name, 1024)?;
    if before.is_none() {
        match custody.write_expected(name, b"", None) {
            Ok(()) => {}
            Err(error) if custody.read(name, 1024)?.is_some() => {
                let _ = error;
            }
            Err(error) => return Err(error),
        }
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options.open(root.join(name))?;
    if !file.metadata()?.is_file() {
        return Err(fail("Refused execution lock"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        if metadata.nlink() != 1 || metadata.uid() != unsafe { libc::getuid() } {
            return Err(fail("Refused execution lock"));
        }
    }
    file.try_lock()
        .map_err(|_| RuntimeError::conflict("Another executor is using this run or workspace"))?;
    custody.check()?;
    Ok(Lock(file))
}
/// Whether an executor holds this run now. A free lock is taken for an instant
/// to find out, so an executor starting at that very moment can see it busy
/// and report a conflict; starting it again succeeds.
pub fn executing(directory: &Path) -> bool {
    directory.join(".run.lock").is_file() && lock(directory, ".run.lock").is_err()
}
pub fn workspace_locks(system: &System, selected: &[String]) -> Result<Vec<Lock>> {
    #[cfg(unix)]
    let user = unsafe { libc::getuid() }.to_string();
    #[cfg(not(unix))]
    let user = std::env::var("USERNAME")
        .unwrap_or_else(|_| "user".into())
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || "._-".contains(character) {
                character
            } else {
                '_'
            }
        })
        .take(64)
        .collect::<String>();
    let root = std::env::temp_dir().join(format!("ai-system-locks-{user}"));
    crate::native::private_directory(&root)?;
    let mut identities = BTreeSet::new();
    for root in std::iter::once(&system.root).chain(
        selected
            .iter()
            .filter_map(|sid| system.services.get(sid).map(|service| &service.root)),
    ) {
        let output = infra_core::git::run(
            root,
            &infra_core::git::strings(&["rev-parse", "--show-toplevel"]),
            None,
            Duration::from_secs(10),
            16384,
        );
        let identity = match output {
            Ok(output) if output.code == 0 => output.stdout,
            _ => path_string(root)?.into_bytes(),
        };
        identities.insert(digest(&identity));
    }
    identities
        .into_iter()
        .map(|identity| lock(&root, &format!("{identity}.lock")))
        .collect()
}
pub fn guard_brain(root: &Path) -> Result<()> {
    let mut custody = PublicationFiles::new(root)?;
    let mut count = 0usize;
    for start in ["project-brain", "memory-bank"] {
        match std::fs::symlink_metadata(root.join(start)) {
            Ok(metadata) if infra_core::fs::linked(&metadata) => {
                return Err(fail("Refused linked native runtime storage"));
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(fail("Native storage root is not a directory")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
        let (_, nodes) = custody.audit_tree(start, 128, 20000usize.saturating_sub(count))?;
        count += nodes;
        if count > 20000 {
            return Err(fail("Native runtime storage exceeds inspection limit"));
        }
    }
    custody.check()
}
/// A rule that ignores its own folder, itself included (`*` matches it too).
const LOCAL_IGNORE: &[u8] = b"# Local runtime state of this clone: never committed.\n*\n";
/// The lock and the context database a run's native storage keeps exist only
/// in this clone. An installed edition ignores them through its own rules; a
/// service folder without one would offer them to `git add -A`, so each local
/// folder gets a rule that hides it from Git.
fn ignore_local_storage(root: &Path) -> Result<()> {
    let mut files = PublicationFiles::new(root)?;
    for directory in ["project-brain/local", "memory-bank/local"] {
        if !std::fs::symlink_metadata(root.join(directory)).is_ok_and(|metadata| metadata.is_dir())
        {
            continue;
        }
        let rule = format!("{directory}/.gitignore");
        if files.read(&rule, 4096)?.is_none() {
            match files.write_expected_limit(&rule, LOCAL_IGNORE, None, 4096) {
                Ok(()) => {}
                // Another process wrote it first.
                Err(_) if files.read(&rule, 4096)?.is_some() => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}
pub struct Brain {
    pub repository: BrainRepository,
}
impl Brain {
    pub fn new(root: &Path, owner: &str) -> Result<Self> {
        guard_brain(root)?;
        Ok(Self {
            repository: BrainRepository::new(RuntimeContext::new(
                Roots::installed(root.to_path_buf())?,
                owner,
                "governed",
            )?)?,
        })
    }
    pub fn ensure(&self, reference: &mut Value) -> Result<Value> {
        guard_brain(&self.repository.context.roots.state)?;
        let id = string(&reference["external_id"]).to_string();
        let record = match self.repository.get_task(&id) {
            Ok(record) => record,
            Err(error) if error.kind == ErrorKind::NotFound => self.repository.create_task(
                &id,
                string(&reference["goal"]),
                vec![],
                vec![],
                "team",
            )?,
            Err(error) => return Err(error),
        };
        if record["external_id"] != reference["external_id"]
            || record["goal"] != reference["goal"]
            || record["owner"] != self.repository.context.actor
            || (!reference["uuid"].is_null() && reference["uuid"] != record["id"])
        {
            return Err(fail("Native task identity or ownership conflict"));
        }
        reference["uuid"] = record["id"].clone();
        if record["status"] != "completed" {
            let path = infra_core::storage::default_database(&self.repository.context.roots)?;
            let mut store = Store::open(
                path,
                infra_core::OpenOptions {
                    startup_wait: Duration::from_secs(5),
                    busy_wait: Duration::from_secs(5),
                    require_write: true,
                },
            )?;
            let revision = record["revision"]
                .as_u64()
                .ok_or_else(|| fail("Native task missing revision"))?;
            // Every dispatch re-ensures its task: an existing binding to this same
            // record follows its revision; one to another record is never repointed.
            match store.binding(&id)? {
                Some(binding) if binding.task_uuid == string(&record["id"]) => {
                    store.refresh_binding(&id, revision)?
                }
                Some(_) => {
                    return Err(RuntimeError::conflict(
                        "Native task binding points at another record; repair it with rebind",
                    ));
                }
                None => store.bind(&id, string(&record["id"]), revision)?,
            }
        }
        ignore_local_storage(&self.repository.context.roots.state)?;
        Ok(record)
    }
    pub fn progress(&self, reference: &mut Value, note: &str) -> Result<Value> {
        let mut last = None;
        for _ in 0..3 {
            let record = self.ensure(reference)?;
            if record["status"] == "completed" {
                return Err(fail("Native task completed outside dispatch"));
            }
            if record["progress"] == note {
                return Ok(record);
            }
            match self.repository.update(
                string(&reference["external_id"]),
                UpdateRecord {
                    expected_revision: record["revision"].as_u64(),
                    progress: Some(note.into()),
                    ..Default::default()
                },
            ) {
                Ok(record) => return Ok(record),
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| fail("Native progress update unavailable")))
    }
    pub fn complete(&self, reference: &mut Value, run_id: &str) -> Result<()> {
        let outcome = format!(
            "AI system run {run_id}: workers and cross-service verification reported complete."
        );
        let verification =
            "Structured worker reports; inspect execution receipts for evidence and limitations";
        let expected = format!("{outcome}\nVerification: {verification}");
        let mut last = None;
        for _ in 0..3 {
            let record = self.ensure(reference)?;
            if record["status"] == "completed" {
                if record["progress"] != expected {
                    return Err(fail("Terminal native task does not match run"));
                }
                return Ok(());
            }
            match self.repository.close_task(
                string(&reference["external_id"]),
                &outcome,
                &[verification.into()],
                record["revision"].as_u64(),
            ) {
                Ok(_) => return Ok(()),
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| fail("Native task completion unavailable")))
    }
    /// Makes what a completed run established the system's memory, by itself:
    /// a resolved finding in this (the system's) Brain, verified by the agent
    /// that independently checked the run, promoted to its Memory Bank
    /// without a person's review (tagged `auto-promoted`, as Harness sessions
    /// save memory). Later plans share it with every agent of a change. A
    /// system folder whose runtime settings turn `automatic_promotion` off
    /// keeps a proposal for a person instead (`memory promote-review`, then
    /// `promote-apply`). Running it again for the same run reuses the finding
    /// and its promotion.
    pub fn share_outcome(&self, state: &Value, plan: &Value, reports: &[Value]) -> Result<Value> {
        guard_brain(&self.repository.context.roots.state)?;
        let run = string(&state["run_id"]);
        let external = format!("ai-system/{run}/outcome");
        let task = string(&plan["context"]["task"]);
        // The Memory Bank names the chunk after its title, and its publication
        // writes `.{name}.{uuid}.tmp` beside it: a real 200-character title
        // overran the 255-byte name limit. The whole task stays in the text.
        let title = clipped(
            &format!("System change {}: {task}", string(&plan["change_id"])),
            120,
        );
        let body = format!("Task: {task} {}", outcome_text(state, reports));
        let root = self.repository.context.roots.state.clone();
        // What the agreement rested on: the system's own documents in the
        // shared context. A later change to them makes the memory stale.
        let sources = plan["context"]["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|source| source["service"] == "__system__" && source["kind"] != "memory")
            .map(|source| string(&source["path"]).to_string())
            .filter(|path| root.join(path).is_file())
            .take(10)
            .collect::<Vec<_>>();
        let record = match self.repository.find(&external, None, false) {
            Ok(image) => image.record,
            Err(error) if error.kind == ErrorKind::NotFound => {
                self.repository.create(CreateRecord {
                    record_type: "finding".into(),
                    external_id: external.clone(),
                    title: title.clone(),
                    goal: Some(task.into()),
                    sources,
                    privacy: "team".into(),
                    authority: "verified".into(),
                    attestation: Some("agent".into()),
                    ..CreateRecord::default()
                })?
            }
            Err(error) => return Err(error),
        };
        let id = string(&record["id"]).to_string();
        if record["status"] != "resolved" {
            self.repository.update(
                &id,
                UpdateRecord {
                    expected_revision: record["revision"].as_u64(),
                    progress: Some(body.clone()),
                    transition_to: Some("resolved".into()),
                    reason: format!("System run {run} completed and its verification passed"),
                    attestation: Some("agent".into()),
                    ..Default::default()
                },
            )?;
        }
        let bank = BankService::new(self.repository.context.clone());
        let existing = infra_core::bank::promotion::list(&bank, false)?
            .into_iter()
            .find(|proposal| {
                proposal["status"] != "rejected"
                    && proposal["source_records"]
                        .as_array()
                        .is_some_and(|rows| rows.iter().any(|row| row["id"] == id.as_str()))
            });
        // Only an explicit `false` in the folder's own settings holds the
        // outcome for a person; the loaded default is off everywhere.
        let held = std::fs::read(root.join("project-brain/config/runtime.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
            .is_some_and(|config| config["automatic_promotion"] == false);
        let mut proposal = match existing {
            Some(proposal) => proposal,
            None => bank.propose(std::slice::from_ref(&id), &title, &body, !held)?,
        };
        let mut note = Value::Null;
        if proposal["review_mode"] == "automatic" && proposal["status"] != "applied" {
            let promotion = string(&proposal["id"]).to_string();
            // The handoff reports the promotion as far as it got.
            let mut attempt = Ok(());
            if proposal["status"] == "proposed" {
                match bank.review(&promotion, None, true) {
                    Ok(reviewed) => proposal = reviewed,
                    Err(error) => attempt = Err(error),
                }
            }
            if attempt.is_ok() {
                match bank.apply(&promotion) {
                    Ok(applied) => proposal = applied,
                    Err(error) => attempt = Err(error),
                }
            }
            if let Err(error) = attempt {
                note = format!(
                    "it stays finding {id} in the system's Brain: promoting it failed: {}",
                    error.message
                )
                .into();
            }
        }
        Ok(
            object!({"record_id":id,"proposal_id":proposal["id"],"status":proposal["status"],"review_mode":proposal["review_mode"],"memory_id":proposal["destination_memory_id"],"title":title,"note":note}),
        )
    }
}
/// Text of a run's outcome for its memory proposal: the mode and services,
/// then each report's summary in run order, bounded.
fn outcome_text(state: &Value, reports: &[Value]) -> String {
    let mode = if state["mode"] == "read-only" {
        "Investigation"
    } else {
        "Change"
    };
    let mut parts = vec![format!(
        "{mode} across {}.",
        texts(&state["selected"]).join(", ")
    )];
    for report in reports {
        let label = match string(&report["phase"]) {
            "contracts" => "Agreement",
            "verify" => "Verification",
            phase => phase.strip_prefix("service-").unwrap_or(phase),
        };
        parts.push(format!(
            "{label}: {}",
            clipped(&without_repetition(string(&report["summary"])), 900)
        ));
    }
    clipped(&parts.join(" "), 6000)
}
/// A summary without a degenerate tail: a unit of at most eight characters
/// repeated ten times or more at its end. A real verification summary ended
/// in 3848 characters of `");`, and the run's memory kept 750 of them.
fn without_repetition(text: &str) -> String {
    let mut characters = text.chars().collect::<Vec<_>>();
    loop {
        let cut = (1..=8).find_map(|unit| {
            if characters.len() < unit * 10 {
                return None;
            }
            let tail = characters[characters.len() - unit..].to_vec();
            let mut end = characters.len();
            let mut count = 0;
            while end >= unit && characters[end - unit..end] == tail[..] {
                end -= unit;
                count += 1;
            }
            (count >= 10).then_some(end)
        });
        match cut {
            Some(end) => characters.truncate(end),
            None => break,
        }
    }
    characters
        .into_iter()
        .collect::<String>()
        .trim_end()
        .to_string()
}
fn clipped(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut short = text.chars().take(limit - 1).collect::<String>();
    short.push('…');
    short
}
pub fn access_scope(state: &Value) -> &str {
    state
        .get("access")
        .and_then(Value::as_str)
        .unwrap_or("service")
}
/// What a dispatch may read and write. Every agent of a run shares one
/// context, so each reads the system folder and every service of the change
/// whatever the scope: a service agent that saw only its own folder could not
/// check how its neighbours use what it changes. The scope decides only where
/// a service agent writes; coordination and verification never write, and no
/// agent writes the system folder.
pub fn dispatch_access(state: &Value, step: &Value) -> Value {
    let selected = texts(&state["selected"]);
    let readable = std::iter::once("__system__".to_string())
        .chain(selected.iter().cloned())
        .collect::<Vec<_>>();
    let writable = if step["service"] == "__system__" || step["mode"] != "edit" {
        vec![]
    } else if access_scope(state) == "all" {
        selected
    } else {
        vec![string(&step["service"]).to_string()]
    };
    object!({"scope":access_scope(state),"readable":readable,"writable":writable})
}
/// Folders added to a dispatch's working folder (`--add-dir`). Codex reads the
/// whole disk already and is granted only the roots it writes. Claude may
/// write in an added folder under `acceptEdits`, so an edit dispatch adds only
/// the roots it writes and reads the others through [`read_directories`]; a
/// read-only one (plan mode keeps them read-only) adds every root it reads and
/// the run's own folder, so the receipts its prompt names can be read: a real
/// verification agent was given their paths and could not open them.
pub fn extra_directories(state: &Value, step: &Value, run: &Path) -> Vec<PathBuf> {
    let access = dispatch_access(state, step);
    let edit = step["mode"] == "edit";
    let wanted = match string(&state["provider"]) {
        "codex" => texts(&access["writable"]),
        "claude" if edit => texts(&access["writable"]),
        "claude" => texts(&access["readable"]),
        _ => vec![],
    };
    let mut result = outside_working_folder(state, step, wanted);
    if string(&state["provider"]) == "claude" && !edit && !result.iter().any(|path| path == run) {
        result.push(run.to_path_buf());
    }
    result
}
/// Folders a Claude edit dispatch may read but not write: the roots of the
/// run's shared context it does not write, and the run's own folder with the
/// receipts its prompt names. They are granted as read rules, since an added
/// folder would be writable. Other CLIs need none: Codex reads the whole disk,
/// and Cursor Agent has no verified option to grant a folder.
pub fn read_directories(state: &Value, step: &Value, run: &Path) -> Vec<PathBuf> {
    if string(&state["provider"]) != "claude" || step["mode"] != "edit" {
        return vec![];
    }
    let access = dispatch_access(state, step);
    let writable = texts(&access["writable"]);
    let wanted = texts(&access["readable"])
        .into_iter()
        .filter(|sid| !writable.contains(sid))
        .collect();
    let mut result = outside_working_folder(state, step, wanted);
    if !result.iter().any(|path| path == run) {
        result.push(run.to_path_buf());
    }
    result
}
fn outside_working_folder(state: &Value, step: &Value, wanted: Vec<String>) -> Vec<PathBuf> {
    let cwd = Path::new(string(&state["roots"][string(&step["service"])]));
    let mut result = vec![];
    for sid in wanted {
        let path = PathBuf::from(string(&state["roots"][&sid]));
        if !path.starts_with(cwd) && !result.contains(&path) {
            result.push(path);
        }
    }
    result
}
pub fn changed_path(state: &Value, step: &Value, value: &str) -> Result<PathBuf> {
    let (sid, path) = if access_scope(state) == "all" {
        let (sid, path) = value
            .split_once('/')
            .ok_or_else(|| fail("Shared-scope changes must start with selected service ID"))?;
        if !texts(&state["selected"])
            .iter()
            .any(|selected| selected == sid)
        {
            return Err(fail("Unknown changed service"));
        }
        (sid, path)
    } else {
        (string(&step["service"]), value)
    };
    Ok(Path::new(string(&state["roots"][sid])).join(relative(path)?))
}
pub fn validate_report(
    value: &Value,
    step: &Value,
    selected: &[String],
    scope: &str,
) -> Result<()> {
    fields(
        value,
        &[
            "status",
            "summary",
            "checks",
            "changed_files",
            "service_order",
        ],
        &[],
    )?;
    choice(&value["status"], &["completed", "blocked"], "worker status")?;
    text(&value["summary"], "worker summary", 4000)?;
    for check in items(&value["checks"], MAX_ITEMS)? {
        fields(check, &["name", "status", "detail"], &[])?;
        text(&check["name"], "check name", 200)?;
        text(&check["detail"], "check detail", 1000)?;
        choice(
            &check["status"],
            &["passed", "failed", "not_run"],
            "worker check status",
        )?;
    }
    for entry in items(&value["changed_files"], MAX_ITEMS)? {
        let value = text(entry, "changed file", 1200)?;
        let path = if scope == "all" {
            let (sid, path) = value
                .split_once('/')
                .ok_or_else(|| fail("Shared-scope change requires service ID"))?;
            if !selected.iter().any(|value| value == sid) {
                return Err(fail("Shared change outside selected services"));
            }
            path
        } else {
            &value
        };
        relative(path)?;
    }
    let order = texts(&value["service_order"]);
    items(&value["service_order"], MAX_SERVICES)?;
    if step["id"] == "contracts" && value["status"] == "completed" {
        if order.len() != selected.len()
            || order.iter().collect::<BTreeSet<_>>() != selected.iter().collect()
        {
            return Err(fail(
                "Contract worker must order every selected service exactly once",
            ));
        }
    } else if order.len() != order.iter().collect::<BTreeSet<_>>().len()
        || order.iter().any(|sid| !selected.contains(sid))
    {
        // Only the completed agreement's order is ever applied. Any other
        // dispatch's order (a blocked agreement's proposal, a service worker
        // echoing it) is kept as evidence; it must still name selected services
        // once each, but never costs the dispatch its work.
        return Err(fail("A service order may name each selected service once"));
    }
    Ok(())
}
pub fn result_schema(selected: &[String]) -> Value {
    let line = |limit| object!({"type":"string","minLength":1,"maxLength":limit,"pattern":r"^[^\x00-\x1f\x7f]+$"});
    object!({"type":"object","additionalProperties":false,"required":["status","summary","checks","changed_files","service_order"],"properties":{"status":{"type":"string","enum":["completed","blocked"]},"summary":line(4000),"checks":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["name","status","detail"],"properties":{"name":line(200),"status":{"type":"string","enum":["passed","failed","not_run"]},"detail":line(1000)}}},"changed_files":{"type":"array","items":{"type":"string"}},"service_order":{"type":"array","items":{"type":"string","enum":selected}}}})
}
/// `investigating` is true in a read-only run. Its dispatches can fix
/// nothing, so a check that failed (a reproduced defect) or could not run is
/// a finding of the report, not a reason to stop: a real payments
/// investigation reproduced the defect it was asked about and stopped the
/// run before verification. Verification still needs one passed check.
pub fn success(receipt: &Value, step: &Value, investigating: bool) -> bool {
    let report = &receipt["report"];
    if receipt["returncode"] != 0
        || !receipt["error"].is_null()
        || report["status"] != "completed"
        || report["checks"].as_array().is_none()
    {
        return false;
    }
    // A read-only dispatch that reports writes stops the run; its report is
    // kept so the files it names can be inspected before a retry.
    if step["mode"] == "read-only" && !texts(&report["changed_files"]).is_empty() {
        return false;
    }
    let checks = report["checks"].as_array().cloned().unwrap_or_default();
    if investigating {
        return step["id"] != "verify" || checks.iter().any(|check| check["status"] == "passed");
    }
    // An agreement decides by its status: blocked when it cannot be reached.
    // Its checks are what it rests on; a real contract agent agreed a fix and
    // reported the defect the fix is for as a failed check, stopping the run.
    if step["id"] == "contracts" {
        return true;
    }
    if checks.iter().any(|check| check["status"] == "failed") {
        return false;
    }
    if step["id"] == "verify" {
        !checks.is_empty() && checks.iter().all(|check| check["status"] == "passed")
    } else {
        true
    }
}
/// The checks a verification left `not_run` when nothing else stopped it: it
/// completed, ran and passed at least one check, failed none and wrote
/// nothing. Verification needs every check passed, so an honest report of a
/// check this environment cannot run (a container, a live broker) stops the
/// run until a person accepts those gaps on resume. `None` otherwise.
pub fn unrun_checks(receipt: &Value, step: &Value) -> Option<Vec<String>> {
    let report = &receipt["report"];
    let checks = report["checks"].as_array()?;
    let unrun = checks
        .iter()
        .filter(|check| check["status"] == "not_run")
        .map(|check| string(&check["name"]).to_string())
        .collect::<Vec<_>>();
    (step["id"] == "verify"
        && receipt["ok"] != true
        && receipt["returncode"] == 0
        && receipt["error"] == "worker_blocked_or_verification_incomplete"
        && receipt["rejection"].is_null()
        && report["status"] == "completed"
        && texts(&report["changed_files"]).is_empty()
        && checks.iter().any(|check| check["status"] == "passed")
        && checks
            .iter()
            .all(|check| matches!(string(&check["status"]), "passed" | "not_run"))
        && !unrun.is_empty())
    .then_some(unrun)
}
/// A completed step's receipt succeeded, or it is a verification whose unrun
/// checks a person accepted, exactly as the step records them.
pub fn completed_receipt(receipt: &Value, step: &Value) -> bool {
    receipt["ok"] == true
        || (step["accepted_unrun_checks"].is_array()
            && unrun_checks(receipt, step)
                .is_some_and(|unrun| unrun == texts(&step["accepted_unrun_checks"])))
}
pub fn dispatch_receipt(directory: &Path, state: &Value, step: &Value) -> Result<Value> {
    let receipt = read_json(directory, &receipt_name(step), MAX_STATE)?;
    fields(
        &receipt,
        &[
            "dispatch_id",
            "phase",
            "input_sha256",
            "mode",
            "returncode",
            "error",
            "duration_seconds",
            "ok",
            "report",
        ],
        &["rejection", "failure", "usage_components", "deterministic"],
    )?;
    if ["rejection", "failure"].iter().any(|key| {
        receipt
            .get(*key)
            .is_some_and(|value| !value.is_string() || receipt["ok"] == true)
    }) {
        return Err(fail("Invalid receipt rejection"));
    }
    if receipt["dispatch_id"] != step["dispatch_id"]
        || receipt["input_sha256"] != step["input_sha256"]
        || receipt["phase"] != step["id"]
        || receipt["mode"] != step["mode"]
        || !receipt["ok"].is_boolean()
    {
        return Err(fail("Mismatched dispatch receipt"));
    }
    if !receipt["report"].is_null() {
        validate_report(
            &receipt["report"],
            step,
            &texts(&state["selected"]),
            access_scope(state),
        )?;
        // Only an unjustified success is refused: a recorded failure is the
        // conservative claim, and older receipts recorded stricter rules.
        if !receipt["ok"].is_boolean()
            || (receipt["ok"] == true && !success(&receipt, step, state["mode"] == "read-only"))
        {
            return Err(fail("Invalid receipt success claim"));
        }
    } else if receipt["ok"] == true {
        return Err(fail("Success receipt has no worker report"));
    }
    Ok(receipt)
}
/// The complete reports of the run's completed dispatches, in run order, each
/// naming its receipt. Every later agent receives them in its shared context,
/// and the handoff keeps them: a later agent used to see only the first 1000
/// characters of a summary and ten checks of each earlier one.
pub fn reports(directory: &Path, state: &Value) -> Result<Vec<Value>> {
    let mut result = vec![];
    for step in items(&state["steps"], MAX_SERVICES + 2)? {
        if step["status"] == "completed" {
            let receipt = dispatch_receipt(directory, state, step)?;
            if !completed_receipt(&receipt, step) {
                return Err(fail("Completed dispatch lacks successful receipt"));
            }
            let report = &receipt["report"];
            let mut entry = object!({"phase":step["id"],"service":step["service"],"attempt":step["attempt"],"status":report["status"],"summary":report["summary"],"checks":report["checks"],"changed_files":report["changed_files"],"receipt":path_string(&directory.join(receipt_name(step)))?});
            if !texts(&report["service_order"]).is_empty() {
                entry["service_order"] = report["service_order"].clone();
            }
            if step["accepted_unrun_checks"].is_array() {
                entry["accepted_unrun_checks"] = step["accepted_unrun_checks"].clone();
            }
            result.push(entry);
        }
    }
    Ok(result)
}
pub fn task_id(run: &str, change: &str, sid: &str) -> String {
    format!(
        "ai-system/{run}/{}",
        &digest(format!("{change}/{sid}").as_bytes())[..24]
    )
}
pub fn run_summary(state: &Value, directory: &Path) -> Value {
    object!({"kind":state["kind"],"run_id":state["run_id"],"change_id":state["change_id"],"run_dir":directory,"status":state["status"],"error":state["error"],"provider":state["provider"],"mode":state["mode"],"access":access_scope(state),"steps":state["steps"],"tasks":state["tasks"].as_object().map(|tasks|tasks.iter().map(|(sid,task)|(sid.clone(),object!({"external_id":task["external_id"],"uuid":task["uuid"],"closed":texts(&state["closed_tasks"]).contains(sid)}))).collect::<serde_json::Map<_,_>>()).unwrap_or_default(),"meaning":"Historical dispatch state; checks and completion are worker-reported"})
}

/// Output cap matching `infra_skillcache::tool::STDOUT_BYTES`.
pub const PROGRAM_STDOUT_BYTES: usize = 256 * 1024;
/// Failure diagnostic summary cap matching `infra_skillcache::tool::DIAGNOSTIC_BYTES`.
pub const PROGRAM_DIAGNOSTIC_BYTES: usize = 4 * 1024;

fn add_opt_u64(target: &mut Option<u64>, add: Option<u64>) {
    if let Some(value) = add {
        *target = Some(target.unwrap_or(0).saturating_add(value));
    }
}

fn opt_u64(value: &Value) -> Option<u64> {
    value.as_u64()
}

#[derive(Clone, Debug, Default)]
struct ComponentCounts {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_write_5m_input_tokens: Option<u64>,
    cache_write_1h_input_tokens: Option<u64>,
    model_calls: Option<u64>,
}

impl ComponentCounts {
    fn add(&mut self, other: &Self) {
        add_opt_u64(&mut self.input_tokens, other.input_tokens);
        add_opt_u64(&mut self.output_tokens, other.output_tokens);
        add_opt_u64(
            &mut self.cache_read_input_tokens,
            other.cache_read_input_tokens,
        );
        add_opt_u64(
            &mut self.cache_write_5m_input_tokens,
            other.cache_write_5m_input_tokens,
        );
        add_opt_u64(
            &mut self.cache_write_1h_input_tokens,
            other.cache_write_1h_input_tokens,
        );
        add_opt_u64(&mut self.model_calls, other.model_calls);
    }

    fn to_json(&self, source: Option<&str>) -> Value {
        object!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cache_read_input_tokens": self.cache_read_input_tokens,
            "cache_write_5m_input_tokens": self.cache_write_5m_input_tokens,
            "cache_write_1h_input_tokens": self.cache_write_1h_input_tokens,
            "model_calls": self.model_calls,
            "source": source,
        })
    }
}

fn claude_message_counts(usage: &Value) -> ComponentCounts {
    let cache = &usage["cache_creation"];
    let mut write_5m = opt_u64(&cache["ephemeral_5m_input_tokens"]);
    let write_1h = opt_u64(&cache["ephemeral_1h_input_tokens"]);
    if write_5m.is_none() && write_1h.is_none() {
        write_5m = opt_u64(&usage["cache_creation_input_tokens"]);
    }
    ComponentCounts {
        input_tokens: opt_u64(&usage["input_tokens"]),
        output_tokens: opt_u64(&usage["output_tokens"]),
        cache_read_input_tokens: opt_u64(&usage["cache_read_input_tokens"]),
        cache_write_5m_input_tokens: write_5m,
        cache_write_1h_input_tokens: write_1h,
        model_calls: Some(1),
    }
}

fn codex_turn_counts(usage: &Value) -> ComponentCounts {
    let cached = opt_u64(&usage["cached_input_tokens"]);
    let input = opt_u64(&usage["input_tokens"]).map(|raw| match cached {
        Some(read) => raw.saturating_sub(read),
        None => raw,
    });
    ComponentCounts {
        input_tokens: input,
        output_tokens: opt_u64(&usage["output_tokens"]),
        cache_read_input_tokens: cached,
        cache_write_5m_input_tokens: None,
        cache_write_1h_input_tokens: None,
        model_calls: Some(1),
    }
}

/// Normalizes provider token telemetry into `UsageComponents` per system step receipt,
/// matching `infra_harness::cost_observation::UsageComponents`.
pub struct StepUsageTracker {
    provider: String,
    claude_messages: std::collections::BTreeMap<String, ComponentCounts>,
    claude_result_outputs: Vec<Option<u64>>,
    codex_totals: ComponentCounts,
    cursor_calls: std::collections::BTreeSet<String>,
}

impl StepUsageTracker {
    pub fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_owned(),
            claude_messages: std::collections::BTreeMap::new(),
            claude_result_outputs: Vec::new(),
            codex_totals: ComponentCounts::default(),
            cursor_calls: std::collections::BTreeSet::new(),
        }
    }

    pub fn observe(&mut self, event: &Value) {
        let kind = event["type"].as_str().unwrap_or("");
        match self.provider.as_str() {
            "claude" if kind == "assistant" => {
                let msg = &event["message"];
                let usage = &msg["usage"];
                if !usage.is_object() {
                    return;
                }
                let id = msg["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("msg-{}", self.claude_messages.len()));
                let mut counts = claude_message_counts(usage);
                counts.output_tokens = None;
                self.claude_messages.insert(id, counts);
            }
            "claude" if kind == "result" => {
                self.claude_result_outputs
                    .push(opt_u64(&event["usage"]["output_tokens"]));
            }
            "codex" if kind == "turn.completed" && event["usage"].is_object() => {
                self.codex_totals.add(&codex_turn_counts(&event["usage"]));
            }
            "cursor" if kind == "assistant" => {
                if let Some(call_id) = event["model_call_id"].as_str().filter(|id| !id.is_empty()) {
                    self.cursor_calls.insert(call_id.to_owned());
                }
            }
            _ => {}
        }
    }

    pub fn snapshot(&self) -> Value {
        let mut total = ComponentCounts::default();
        match self.provider.as_str() {
            "claude" => {
                for counts in self.claude_messages.values() {
                    total.add(counts);
                }
                total.output_tokens = match self.claude_result_outputs.as_slice() {
                    [Some(output)] => Some(*output),
                    _ => None,
                };
            }
            "codex" => total.add(&self.codex_totals),
            "cursor" => {
                let calls = self.cursor_calls.len() as u64;
                if calls > 0 {
                    total.model_calls = Some(calls);
                }
            }
            _ => {}
        }
        let source = ["claude", "codex", "cursor"]
            .contains(&self.provider.as_str())
            .then_some(self.provider.as_str());
        total.to_json(source)
    }
}

/// Returns zero-token / zero-call `usage_components` for deterministic steps,
/// or null-valued components for unknown/unobserved LLM steps.
pub fn empty_usage_components(source: Option<&str>, model_calls: u64) -> Value {
    if source == Some("deterministic") && model_calls == 0 {
        ComponentCounts {
            input_tokens: Some(0),
            output_tokens: Some(0),
            cache_read_input_tokens: Some(0),
            cache_write_5m_input_tokens: Some(0),
            cache_write_1h_input_tokens: Some(0),
            model_calls: Some(0),
        }
        .to_json(Some("deterministic"))
    } else {
        ComponentCounts::default().to_json(source)
    }
}

/// Extracts normalized `usage_components` from a step's raw worker NDJSON stream.
pub fn usage_components_from_raw(raw: &[u8], provider: &str) -> Value {
    let mut tracker = StepUsageTracker::new(provider);
    for line in String::from_utf8_lossy(raw).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<Value>(trimmed) {
            tracker.observe(&event);
        }
    }
    tracker.snapshot()
}

pub struct DeterministicVerifyOutcome {
    pub all_passed: bool,
    pub checks: Vec<Value>,
    pub diagnostics_summary: String,
    pub duration_seconds: f64,
}

fn bound_diagnostic(raw: &str, cap: usize) -> String {
    let masked = match infra_core::domain::mask_secrets(raw) {
        Ok((text, _)) => text,
        Err(_) => "[diagnostics withheld: a possible secret could not be masked]".into(),
    };
    let trimmed = masked.trim();
    if trimmed.len() <= cap {
        return trimmed.to_owned();
    }
    let mut end = cap;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    trimmed[..end].to_owned()
}

/// Executes deterministic verification checks declared on the `verify` plan step
/// inside a network-off `Profile::Gate` sandbox with scrubbed environment and bounded output.
pub async fn run_deterministic_verify_checks(
    specs: &[Value],
    state: &Value,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    runner_lock: Option<std::sync::Arc<std::fs::File>>,
) -> Result<DeterministicVerifyOutcome> {
    run_deterministic_verify_checks_cached(specs, state, cancel, runner_lock, None).await
}

const VERIFY_RACY_NS: i128 = 2_000_000_000;

fn time_to_unix_ns(time: std::time::SystemTime) -> Option<i128> {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(delta) => i128::try_from(delta.as_nanos()).ok(),
        Err(err) => i128::try_from(err.duration().as_nanos())
            .ok()
            .and_then(|ns| ns.checked_neg()),
    }
}

fn now_unix_ns() -> i128 {
    time_to_unix_ns(std::time::SystemTime::now()).unwrap_or(0)
}

#[cfg(test)]
fn verify_tree_stamp(root: &Path) -> Option<(String, i128)> {
    verify_tree_stamp_tier(root, &[], None).map(|(_, stamp, mtime)| (stamp, mtime))
}

fn resolve_state_dir_from_health_db(health_db: &Path) -> Option<PathBuf> {
    let mut cur = health_db.parent();
    for _ in 0..4 {
        let candidate = cur?;
        if candidate.join("graphs").is_dir() || candidate.join("sessions.sqlite3").is_file() {
            return Some(candidate.to_path_buf());
        }
        cur = candidate.parent();
    }
    None
}

fn root_command_basename(cmd: &str) -> String {
    cmd.rsplit(['\\', '/'])
        .next()
        .unwrap_or(cmd)
        .to_ascii_lowercase()
}

fn stat_regular_non_symlink(path: &Path) -> Option<(u64, i128)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let mtime = meta.modified().ok().and_then(time_to_unix_ns)?;
    Some((meta.len(), mtime))
}

fn scoped_closure_files(root: &Path, argv: &[String]) -> Option<Vec<PathBuf>> {
    let exe = root_command_basename(argv.first()?);
    let args = &argv[1..];
    match exe.as_str() {
        "composer" => {
            let subcmd = args.iter().find(|a| !a.starts_with('-'))?.as_str();
            if subcmd != "validate" {
                return None;
            }
            let mut closure = Vec::new();
            let composer = root.join("composer.json");
            if !composer.is_file() {
                return None;
            }
            closure.push(composer);
            let lock = root.join("composer.lock");
            if lock.exists() {
                closure.push(lock);
            }
            Some(closure)
        }
        "php" => {
            if !args.iter().any(|a| a == "-l" || a == "--syntax-check")
                || args
                    .iter()
                    .any(|a| matches!(a.as_str(), "-r" | "-f" | "-B" | "-R" | "-E" | "-F"))
            {
                return None;
            }
            let mut targets = Vec::new();
            let mut i = 0;
            while i < args.len() {
                let arg = &args[i];
                if matches!(arg.as_str(), "-c" | "-d" | "-z") {
                    i += 2;
                    continue;
                }
                if arg.starts_with('-') {
                    i += 1;
                    continue;
                }
                let p = root.join(arg);
                let canon = infra_core::fs::canonicalize(&p).ok()?;
                if !canon.starts_with(root) || !canon.is_file() {
                    return None;
                }
                targets.push(p);
                i += 1;
            }
            (!targets.is_empty()).then_some(targets)
        }
        _ => None,
    }
}

fn find_graph_ir_db(root: &Path, state_dir: &Path) -> Option<PathBuf> {
    let canon = infra_core::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let sessions_db = state_dir.join("sessions.sqlite3");
    let mut candidate_ids: Vec<String> = Vec::new();
    if sessions_db.is_file()
        && let Ok(conn) = rusqlite::Connection::open_with_flags(
            &sessions_db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        && let Ok(mut stmt) = conn.prepare("SELECT id, path FROM registered_projects")
        && let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
    {
        for (pid, raw_path) in rows.flatten() {
            let reg_canon = infra_core::fs::canonicalize(Path::new(&raw_path))
                .unwrap_or_else(|_| PathBuf::from(&raw_path));
            if reg_canon == canon {
                candidate_ids.push(pid);
            }
        }
    }
    let root_hash16 = &digest(canon.to_string_lossy().as_bytes())[..16];
    candidate_ids.push(format!("sys-{root_hash16}"));
    candidate_ids.push(root_hash16.to_string());

    for pid in candidate_ids {
        let db = state_dir.join("graphs").join(&pid).join("graph_ir.sqlite3");
        if db.is_file() {
            return Some(db);
        }
    }
    None
}

fn git_dirty_entries(root: &Path) -> Option<Vec<String>> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "-z", "-uall"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut entries = Vec::new();
    for raw in out.stdout.split(|b| *b == 0) {
        if raw.is_empty() {
            continue;
        }
        let s = String::from_utf8_lossy(raw);
        if s.len() > 3 {
            entries.push(s[3..].to_string());
        } else {
            entries.push(s.into_owned());
        }
    }
    entries.sort();
    Some(entries)
}

fn try_ir_tree_stamp(root: &Path, state_dir: &Path) -> Option<(&'static str, String, i128)> {
    let db_path = find_graph_ir_db(root, state_dir)?;
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let write_epoch: String = conn
        .query_row(
            "SELECT value FROM ir_meta WHERE key = 'write_epoch'",
            [],
            |r| r.get(0),
        )
        .ok()?;
    let mut stmt = conn
        .prepare("SELECT path, size, mtime_ns FROM ir_files ORDER BY path ASC")
        .ok()?;
    let rows: Vec<(String, i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .ok()?
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    if rows.is_empty() {
        return None;
    }

    let mut max_mtime: i128 = root
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(time_to_unix_ns)
        .unwrap_or(0);
    let ir_max_mtime: i128 = rows
        .iter()
        .map(|(_, _, m)| i128::from(*m))
        .max()
        .unwrap_or(0);
    max_mtime = max_mtime.max(ir_max_mtime);

    if let Some(head) = crate::catalog::git_head(root)
        && let Some(dirty) = git_dirty_entries(root)
    {
        let mut parts = vec![
            format!("ir_epoch:{write_epoch}"),
            format!("ir_count:{}", rows.len()),
            format!("git_head:{head}"),
        ];
        for rel in dirty {
            let full = root.join(&rel);
            if let Ok(meta) = std::fs::symlink_metadata(&full) {
                if meta.file_type().is_symlink() {
                    return None;
                }
                if meta.is_file() {
                    let mtime = meta.modified().ok().and_then(time_to_unix_ns)?;
                    max_mtime = max_mtime.max(mtime);
                    parts.push(format!("dirty:{rel}:{}:{mtime}", meta.len()));
                } else {
                    parts.push(format!("dirty:{rel}:other"));
                }
            } else {
                parts.push(format!("dirty:{rel}:missing"));
            }
        }
        for manifest_rel in [
            "composer.json",
            "composer.lock",
            "package.json",
            "package-lock.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "Cargo.toml",
            "Cargo.lock",
            "pyproject.toml",
            "go.mod",
        ] {
            let path = root.join(manifest_rel);
            if let Some((len, mtime)) = stat_regular_non_symlink(&path) {
                max_mtime = max_mtime.max(mtime);
                parts.push(format!("manifest:{manifest_rel}:{len}:{mtime}"));
            }
        }
        return Some(("ir_git", digest(parts.join("\n").as_bytes()), max_mtime));
    }

    // Non-Git fallback: verify every indexed file and its parent directory against the IR DB.
    let mut seen_dirs = std::collections::BTreeSet::new();
    let mut parts = vec![
        format!("ir_epoch:{write_epoch}"),
        format!("ir_count:{}", rows.len()),
    ];
    for (rel, expected_size, expected_mtime) in rows {
        let full = root.join(&rel);
        let (len, mtime) = stat_regular_non_symlink(&full)?;
        if i64::try_from(len).ok() != Some(expected_size) || i128::from(expected_mtime) != mtime {
            return None;
        }
        max_mtime = max_mtime.max(mtime);
        if let Some(parent) = full.parent()
            && seen_dirs.insert(parent.to_path_buf())
            && let Some(dir_mtime) = parent
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(time_to_unix_ns)
        {
            if dir_mtime > ir_max_mtime {
                return None;
            }
            max_mtime = max_mtime.max(dir_mtime);
        }
        parts.push(format!("{rel}:{len}:{mtime}"));
    }
    Some(("ir_indexed", digest(parts.join("\n").as_bytes()), max_mtime))
}

pub fn verify_tree_stamp_tier(
    root: &Path,
    argv: &[String],
    state_dir: Option<&Path>,
) -> Option<(&'static str, String, i128)> {
    // Tier 1: exact scoped file closure for self-contained commands (composer validate, php -l).
    if let Some(files) = scoped_closure_files(root, argv) {
        let mut max_mtime: i128 = 0;
        let mut parts = Vec::with_capacity(files.len() + 1);
        for file in files {
            let (len, mtime) = stat_regular_non_symlink(&file)?;
            max_mtime = max_mtime.max(mtime);
            let rel = file
                .strip_prefix(root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            parts.push(format!("{rel}:{len}:{mtime}"));
        }
        parts.sort();
        let head = crate::catalog::git_head(root).unwrap_or_default();
        parts.push(format!("git_head:{head}"));
        return Some((
            "scoped_files",
            digest(parts.join("\n").as_bytes()),
            max_mtime,
        ));
    }

    // Tier 2: Graph-IR revision / indexed stat snapshot (no 3000-file walk).
    if let Some(state) = state_dir
        && let Some(ir_stamp) = try_ir_tree_stamp(root, state)
    {
        return Some(ir_stamp);
    }

    // Tier 3: bounded directory walk fallback when no narrower scope or Graph-IR store exists.
    const MAX_FILES: usize = 3000;
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    let mut entries = Vec::new();
    let mut max_mtime = root
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(time_to_unix_ns)
        .unwrap_or(0);
    while let Some(dir) = queue.pop_front() {
        let mut sorted: Vec<_> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok())
            .collect();
        sorted.sort_by_key(|e| e.file_name());
        for entry in sorted {
            let name = entry.file_name();
            let name_s = name.to_string_lossy();
            if matches!(
                name_s.as_ref(),
                ".git" | "target" | "vendor" | "node_modules" | ".phpunit.cache" | ".infra-work"
            ) {
                continue;
            }
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).ok()?;
            if meta.file_type().is_symlink() {
                return None;
            }
            if meta.is_dir() {
                queue.push_back(path);
            } else if meta.is_file() {
                let mtime = meta.modified().ok().and_then(time_to_unix_ns)?;
                max_mtime = max_mtime.max(mtime);
                let rel = path
                    .strip_prefix(root)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/");
                entries.push(format!("{rel}:{}:{mtime}", meta.len()));
                if entries.len() >= MAX_FILES {
                    return None;
                }
            }
        }
    }
    let head = crate::catalog::git_head(root).unwrap_or_default();
    entries.push(format!("git_head:{head}"));
    Some((
        "bounded_walk",
        digest(entries.join("\n").as_bytes()),
        max_mtime,
    ))
}

fn ensure_system_ledger(db_path: &Path) -> Result<rusqlite::Connection> {
    if let Some(parent) = db_path.parent() {
        infra_core::fs::ensure_directory(parent, true)?;
    }
    let conn = rusqlite::Connection::open(db_path)?;
    conn.busy_timeout(Duration::from_secs(2))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_health_runs(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            workflow TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            run_key TEXT NOT NULL,
            step_id TEXT NOT NULL,
            outcome TEXT NOT NULL,
            schema_error INTEGER NOT NULL DEFAULT 0,
            deopt INTEGER NOT NULL DEFAULT 0,
            duration_seconds REAL NOT NULL DEFAULT 0.0,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS workflow_health_lookup
            ON workflow_health_runs(workflow, provider, model, id DESC);
        CREATE TABLE IF NOT EXISTS verify_step_receipts(
            cache_key TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            detail TEXT NOT NULL,
            created_at TEXT NOT NULL
        );",
    )?;
    Ok(conn)
}

pub const HEALTH_DECAY: f64 = 0.82;
pub const HEALTH_PRIOR_ALPHA: f64 = 0.25;
pub const HEALTH_PRIOR_WEIGHT: f64 = 1.5;
pub const HEALTH_HISTORY_LIMIT: usize = 50;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemHealthSummary {
    pub total_runs: usize,
    pub schema_error_runs: usize,
    pub deopt_runs: usize,
    pub schema_error_rate: f64,
    pub deopt_rate: f64,
    pub warnings: Vec<String>,
}

impl SystemHealthSummary {
    pub fn is_degraded(&self) -> bool {
        self.total_runs >= 3 && (self.schema_error_rate > 0.25 || self.deopt_rate > 0.30)
    }

    pub fn recommended_max_attempts(&self, base_limit: u64) -> u64 {
        if !self.is_degraded() {
            return base_limit;
        }
        if self.schema_error_rate > 0.50 || self.deopt_rate > 0.50 {
            base_limit.min(2)
        } else {
            base_limit.min(4)
        }
    }

    pub fn recommended_concurrency(&self, base_concurrency: usize) -> usize {
        let base = base_concurrency.max(1);
        if !self.is_degraded() {
            return base;
        }
        if self.schema_error_rate > 0.50 || self.deopt_rate > 0.50 {
            1
        } else {
            (base / 2).max(1)
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Tracks per-step system workflow health cleanly in SQLite."
)]
pub fn record_system_health(
    db_path: &Path,
    workflow: &str,
    provider: &str,
    model: Option<&str>,
    run_key: &str,
    step_id: &str,
    outcome: &str,
    schema_error: bool,
    deopt: bool,
    duration_seconds: f64,
) {
    let Ok(conn) = ensure_system_ledger(db_path) else {
        return;
    };
    let _ = conn.execute(
        "INSERT INTO workflow_health_runs(workflow,provider,model,run_key,step_id,outcome,schema_error,deopt,duration_seconds,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        rusqlite::params![
            workflow,
            provider,
            model.unwrap_or("default"),
            run_key,
            step_id,
            outcome,
            i64::from(schema_error),
            i64::from(deopt),
            duration_seconds,
            now(),
        ],
    );
}

pub fn summarize_system_health(
    db_path: &Path,
    workflow: &str,
    provider: &str,
    model: Option<&str>,
) -> SystemHealthSummary {
    let mut summary = SystemHealthSummary::default();
    let Ok(conn) = ensure_system_ledger(db_path) else {
        return summary;
    };
    let model_key = model.unwrap_or("default");
    let Ok(mut stmt) = conn.prepare(
        "SELECT schema_error, deopt FROM workflow_health_runs WHERE workflow=?1 AND provider=?2 AND model=?3 ORDER BY id DESC LIMIT ?4",
    ) else {
        return summary;
    };
    let Ok(rows) = stmt.query_map(
        rusqlite::params![workflow, provider, model_key, HEALTH_HISTORY_LIMIT as i64],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    ) else {
        return summary;
    };
    let mut total_weight = 0.0_f64;
    let mut weighted_schema = 0.0_f64;
    let mut weighted_deopt = 0.0_f64;
    for (idx, row) in rows.flatten().enumerate() {
        let weight = HEALTH_DECAY.powi(idx as i32);
        summary.total_runs += 1;
        total_weight += weight;
        if row.0 != 0 {
            summary.schema_error_runs += 1;
            weighted_schema += weight;
        }
        if row.1 != 0 {
            summary.deopt_runs += 1;
            weighted_deopt += weight;
        }
    }
    if summary.total_runs > 0 {
        let denom = total_weight + HEALTH_PRIOR_WEIGHT;
        summary.schema_error_rate = (weighted_schema + HEALTH_PRIOR_ALPHA) / denom;
        summary.deopt_rate = (weighted_deopt + HEALTH_PRIOR_ALPHA) / denom;
    }
    if summary.total_runs >= 3 {
        if summary.schema_error_rate > 0.25 {
            summary.warnings.push(format!(
                "High schema error rate for {workflow}/{provider}/{model_key}: {:.0}% smoothed ({}/{} recent)",
                summary.schema_error_rate * 100.0,
                summary.schema_error_runs,
                summary.total_runs
            ));
        }
        if summary.deopt_rate > 0.30 {
            summary.warnings.push(format!(
                "Degraded ROI/deopt warning for {workflow}/{provider}/{model_key}: {:.0}% smoothed ({}/{} recent)",
                summary.deopt_rate * 100.0,
                summary.deopt_runs,
                summary.total_runs
            ));
        }
    }
    summary
}

/// Selects a healthier tracked model for `(workflow, provider)` when the requested model
/// exhibits degraded smoothed health and another model has >= 3 runs and healthy metrics.
pub fn select_healthier_model(
    db_path: &Path,
    workflow: &str,
    provider: &str,
    requested_model: Option<&str>,
) -> Option<String> {
    let current = summarize_system_health(db_path, workflow, provider, requested_model);
    if !current.is_degraded() {
        return None;
    }
    let current_key = requested_model.unwrap_or("default");
    let current_score = current.schema_error_rate + current.deopt_rate;
    let conn = ensure_system_ledger(db_path).ok()?;
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT model FROM workflow_health_runs WHERE workflow=?1 AND provider=?2 AND model != 'default' ORDER BY model ASC",
        )
        .ok()?;
    let candidates: Vec<String> = stmt
        .query_map(rusqlite::params![workflow, provider], |r| r.get(0))
        .ok()?
        .flatten()
        .collect();
    let mut best: Option<(f64, String)> = None;
    for candidate in candidates {
        if candidate == current_key {
            continue;
        }
        let health = summarize_system_health(db_path, workflow, provider, Some(&candidate));
        if health.total_runs < 3 || health.is_degraded() {
            continue;
        }
        let score = health.schema_error_rate + health.deopt_rate;
        if score + 0.10 >= current_score {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|(best_score, best_name)| (score, &candidate) < (*best_score, best_name))
        {
            best = Some((score, candidate));
        }
    }
    best.map(|(_, model)| model)
}

/// Executes deterministic verification checks with `VERIFY_RACY_NS`-guarded cross-run
/// receipt reuse backed by `health_db` (SQLite `verify_step_receipts`).
pub async fn run_deterministic_verify_checks_cached(
    specs: &[Value],
    state: &Value,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    runner_lock: Option<std::sync::Arc<std::fs::File>>,
    health_db: Option<&Path>,
) -> Result<DeterministicVerifyOutcome> {
    run_deterministic_verify_checks_at(
        specs,
        state,
        cancel,
        runner_lock,
        health_db,
        now_unix_ns(),
        None,
    )
    .await
}

type VerifyStepExec<'a> = &'a (
        dyn Fn(
    &[String],
    &Path,
    &std::collections::BTreeMap<String, String>,
) -> std::result::Result<(i32, String), (String, String)>
            + Sync
    );

async fn run_deterministic_verify_checks_at(
    specs: &[Value],
    state: &Value,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    runner_lock: Option<std::sync::Arc<std::fs::File>>,
    health_db: Option<&Path>,
    now_ns: i128,
    custom_runner: Option<VerifyStepExec<'_>>,
) -> Result<DeterministicVerifyOutcome> {
    let started = std::time::Instant::now();
    let mut checks = Vec::with_capacity(specs.len());
    let mut failure_lines = Vec::new();
    let mut all_passed = true;
    let disable_cache = std::env::var("HARNESS_DISABLE_CHECK_CACHE").as_deref() == Ok("1");

    for (index, item) in specs.iter().take(MAX_ITEMS).enumerate() {
        fields(item, &["name", "argv"], &["cwd_service", "timeout_seconds"])?;
        let name = text(&item["name"], "program check name", 200)?;
        let argv = texts(&item["argv"]);
        if argv.is_empty() {
            return Err(fail(format!(
                "Deterministic verify check `{name}` has empty argv"
            )));
        }
        let cwd_service = item["cwd_service"].as_str().unwrap_or("__system__");
        let root_str = state["roots"][cwd_service].as_str().ok_or_else(|| {
            fail(format!(
                "Unknown service root `{cwd_service}` for check `{name}`"
            ))
        })?;
        let cwd_canon = infra_core::fs::canonicalize(root_str)?;
        let timeout = item["timeout_seconds"].as_u64().unwrap_or(60).clamp(1, 600);

        let mut readable = Vec::new();
        if let Some(roots_obj) = state["roots"].as_object() {
            for (_, val) in roots_obj {
                if let Some(s) = val.as_str()
                    && let Ok(canon) = infra_core::fs::canonicalize(s)
                    && !canon.starts_with(&cwd_canon)
                    && !readable.contains(&canon)
                {
                    readable.push(canon);
                }
            }
        }
        let path_env = std::env::var("PATH").ok();
        let env =
            infra_host::sandbox::scrub_env(std::env::vars(), &std::collections::BTreeMap::new());
        let state_dir = health_db.and_then(resolve_state_dir_from_health_db);
        let cache_key = if !disable_cache {
            health_db.and_then(|_| {
                let (_tier, tree_digest, max_mtime) =
                    verify_tree_stamp_tier(&cwd_canon, &argv, state_dir.as_deref())?;
                (max_mtime > 0 && now_ns > VERIFY_RACY_NS && max_mtime <= now_ns - VERIFY_RACY_NS)
                    .then(|| {
                        let env_digest =
                            digest(serde_json::to_vec(&env).unwrap_or_default().as_slice());
                        digest(
                            object!({
                                "name": &name,
                                "argv": &argv,
                                "cwd": cwd_canon.to_string_lossy(),
                                "timeout": timeout,
                                "env": env_digest,
                                "tree": tree_digest,
                            })
                            .to_string()
                            .as_bytes(),
                        )
                    })
            })
        } else {
            None
        };
        if let (Some(db_path), Some(key)) = (health_db, cache_key.as_deref())
            && let Ok(conn) = ensure_system_ledger(db_path)
            && let Ok(Some(detail)) = rusqlite::OptionalExtension::optional(conn.query_row(
                "SELECT detail FROM verify_step_receipts WHERE cache_key=?1",
                [key],
                |r| r.get::<_, String>(0),
            ))
        {
            checks.push(
                object!({"name": name, "status": "passed", "detail": detail, "cached": true}),
            );
            continue;
        }

        let wrapped = if infra_host::sandbox::available() {
            let spec = infra_host::sandbox::Spec {
                workspace: cwd_canon.clone(),
                target: cwd_canon.clone(),
                protected: vec![],
                readable,
                accounts: vec![],
                runtime: infra_host::sandbox::runtime_dirs_for_argv(&argv, path_env.as_deref()),
                codex: None,
                evidence: None,
                network: false,
            };
            infra_host::sandbox::gate(&spec, &argv).map_err(host)?
        } else {
            argv.clone()
        };

        let (result, raw) = if let Some(runner) = custom_runner {
            match runner(&wrapped, &cwd_canon, &env) {
                Ok((code, out)) => (Ok(code), out),
                Err((code, msg)) => (Err((code, msg)), String::new()),
            }
        } else {
            let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
            let sink = captured.clone();
            let res = infra_host::process::run_raw_with_lock(
                &wrapped,
                &cwd_canon,
                &env,
                &[],
                infra_host::process::Limits {
                    seconds: Some(timeout),
                    output: PROGRAM_STDOUT_BYTES,
                    line: PROGRAM_STDOUT_BYTES,
                },
                cancel.clone(),
                runner_lock.clone(),
                move |chunk| {
                    let mut out = sink.lock().map_err(|_| {
                        infra_host::HarnessError::unavailable("Program check capture lock failed")
                    })?;
                    let left = PROGRAM_STDOUT_BYTES.saturating_sub(out.len());
                    out.extend_from_slice(&chunk[..chunk.len().min(left)]);
                    Ok(())
                },
            )
            .await;
            let raw = captured
                .lock()
                .map(|guard| String::from_utf8_lossy(&guard).into_owned())
                .unwrap_or_default();
            let mapped = match res {
                Ok(output) => Ok(output.code),
                Err(error) => Err((error.code.to_string(), error.message)),
            };
            (mapped, raw)
        };
        let bounded = bound_diagnostic(&raw, PROGRAM_DIAGNOSTIC_BYTES);
        match result {
            Ok(0) => {
                let detail = if bounded.is_empty() {
                    format!("passed (check {})", index + 1)
                } else {
                    bounded
                };
                if let (Some(db_path), Some(key)) = (health_db, cache_key.as_deref())
                    && let Ok(conn) = ensure_system_ledger(db_path)
                {
                    let _ = conn.execute(
                        "INSERT INTO verify_step_receipts(cache_key,name,detail,created_at) VALUES(?1,?2,?3,?4)
                         ON CONFLICT(cache_key) DO UPDATE SET detail=excluded.detail,created_at=excluded.created_at",
                        rusqlite::params![key, &name, &detail, now()],
                    );
                }
                checks.push(object!({"name": name, "status": "passed", "detail": detail}));
            }
            Ok(code) => {
                all_passed = false;
                let reason = format!("exited with code {code}");
                let detail = if bounded.is_empty() {
                    reason
                } else {
                    format!("{reason}: {bounded}")
                };
                let detail = bound_diagnostic(&detail, 2000);
                failure_lines.push(format!("- [{name}] {detail}"));
                checks.push(object!({"name": name, "status": "failed", "detail": detail}));
            }
            Err((code, message)) => {
                all_passed = false;
                let detail =
                    bound_diagnostic(&format!("execution failed ({code}): {message}"), 2000);
                failure_lines.push(format!("- [{name}] {detail}"));
                checks.push(object!({"name": name, "status": "failed", "detail": detail}));
            }
        }
    }

    let diagnostics_summary = bound_diagnostic(&failure_lines.join("\n"), PROGRAM_DIAGNOSTIC_BYTES);
    Ok(DeterministicVerifyOutcome {
        all_passed,
        checks,
        diagnostics_summary,
        duration_seconds: (started.elapsed().as_secs_f64() * 1000.0).round() / 1000.0,
    })
}

#[cfg(test)]
mod usage_components_tests {
    use super::*;

    #[test]
    fn step_usage_tracker_dedupes_claude_messages_and_normalizes_codex() {
        let claude_raw = concat!(
            "{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":400,\"cache_creation\":{\"ephemeral_5m_input_tokens\":50,\"ephemeral_1h_input_tokens\":10}}}}\n",
            "{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":120,\"cache_read_input_tokens\":400,\"cache_creation\":{\"ephemeral_5m_input_tokens\":50,\"ephemeral_1h_input_tokens\":10}}}}\n",
            "{\"type\":\"result\",\"usage\":{\"output_tokens\":33}}\n"
        );
        let parsed = usage_components_from_raw(claude_raw.as_bytes(), "claude");
        assert_eq!(parsed["input_tokens"], 120);
        assert_eq!(parsed["output_tokens"], 33);
        assert_eq!(parsed["cache_read_input_tokens"], 400);
        assert_eq!(parsed["cache_write_5m_input_tokens"], 50);
        assert_eq!(parsed["cache_write_1h_input_tokens"], 10);
        assert_eq!(parsed["model_calls"], 1);
        assert_eq!(parsed["source"], "claude");

        let codex_raw = concat!(
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":500,\"cached_input_tokens\":200,\"output_tokens\":25}}\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"cached_input_tokens\":25,\"output_tokens\":10}}\n"
        );
        let codex_parsed = usage_components_from_raw(codex_raw.as_bytes(), "codex");
        assert_eq!(codex_parsed["input_tokens"], 375);
        assert_eq!(codex_parsed["cache_read_input_tokens"], 225);
        assert_eq!(codex_parsed["output_tokens"], 35);
        assert_eq!(codex_parsed["model_calls"], 2);

        let det = empty_usage_components(Some("deterministic"), 0);
        assert_eq!(det["model_calls"], 0);
        assert_eq!(det["input_tokens"], 0);
        assert_eq!(det["source"], "deterministic");

        let cursor_raw = concat!(
            "{\"type\":\"assistant\",\"model_call_id\":\"mc1\"}\n",
            "{\"type\":\"assistant\",\"model_call_id\":\"mc1\"}\n",
            "{\"type\":\"assistant\",\"model_call_id\":\"mc2\"}\n",
            "{\"type\":\"assistant\",\"model_call_id\":\"\"}\n",
            "{\"type\":\"result\",\"subtype\":\"success\"}\n"
        );
        let cursor_parsed = usage_components_from_raw(cursor_raw.as_bytes(), "cursor");
        assert_eq!(cursor_parsed["model_calls"], 2);
        assert_eq!(cursor_parsed["input_tokens"], Value::Null);
        assert_eq!(cursor_parsed["output_tokens"], Value::Null);
        assert_eq!(cursor_parsed["cache_read_input_tokens"], Value::Null);
        assert_eq!(cursor_parsed["source"], "cursor");

        let cursor_empty =
            usage_components_from_raw(b"{\"type\":\"result\",\"subtype\":\"success\"}\n", "cursor");
        assert_eq!(cursor_empty["model_calls"], Value::Null);
        assert_eq!(cursor_empty["source"], "cursor");
    }

    #[tokio::test]
    async fn system_health_ledger_and_verify_receipt_cache_respect_racy_ns() {
        let dir = std::env::temp_dir().join(format!("infra-sys-health-{}", uuid::Uuid::new_v4()));
        let svc = dir.join("svc");
        std::fs::create_dir_all(&svc).unwrap();
        std::fs::write(svc.join("ok.txt"), "v1\n").unwrap();
        let db = dir.join("system_health.sqlite3");

        for i in 0..4 {
            record_system_health(
                &db,
                "system",
                "codex",
                Some("gpt-5"),
                &format!("r-{i}"),
                "verify",
                if i < 2 { "blocked" } else { "completed" },
                i < 2,
                i < 2,
                1.0,
            );
        }
        let health = summarize_system_health(&db, "system", "codex", Some("gpt-5"));
        assert_eq!(health.total_runs, 4);
        assert_eq!(health.schema_error_runs, 2);
        assert!(
            health
                .warnings
                .iter()
                .any(|w| w.contains("schema error rate"))
        );
        assert!(health.warnings.iter().any(|w| w.contains("ROI/deopt")));

        let svc_canon = infra_core::fs::canonicalize(&svc).unwrap();
        let (_digest, max_mtime) = verify_tree_stamp(&svc_canon).unwrap();
        let argv = if cfg!(windows) {
            object!(["cmd", "/C", "echo verify-ok"])
        } else {
            object!(["/bin/sh", "-c", "echo verify-ok"])
        };
        let specs = vec![object!({
            "name": "svc:echo",
            "argv": argv,
            "cwd_service": "svc",
            "timeout_seconds": 10
        })];
        let state = object!({"roots": {"svc": svc.to_string_lossy().as_ref()}});
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let exec_calls = std::sync::atomic::AtomicUsize::new(0);
        let run_local = |wrapped: &[String],
                         cwd: &Path,
                         env: &std::collections::BTreeMap<String, String>|
         -> std::result::Result<(i32, String), (String, String)> {
            exec_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let out = std::process::Command::new(&wrapped[0])
                .args(&wrapped[1..])
                .current_dir(cwd)
                .env_clear()
                .envs(env)
                .output()
                .map_err(|e| ("spawn".to_owned(), e.to_string()))?;
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            Ok((out.status.code().unwrap_or(1), text))
        };

        // Inside VERIFY_RACY_NS window -> never reads/writes cache.
        let racy = run_deterministic_verify_checks_at(
            &specs,
            &state,
            cancel.clone(),
            None,
            Some(&db),
            max_mtime + VERIFY_RACY_NS - 1,
            Some(&run_local),
        )
        .await
        .unwrap();
        assert!(racy.all_passed, "racy failed: {:?}", racy.checks);
        assert!(racy.checks[0].get("cached").is_none());
        assert_eq!(exec_calls.load(std::sync::atomic::Ordering::Relaxed), 1);

        // Outside VERIFY_RACY_NS -> first executes live, second hits SQLite receipt cache.
        let settled_now = max_mtime + VERIFY_RACY_NS + 100;
        let first = run_deterministic_verify_checks_at(
            &specs,
            &state,
            cancel.clone(),
            None,
            Some(&db),
            settled_now,
            Some(&run_local),
        )
        .await
        .unwrap();
        assert!(first.all_passed, "first failed: {:?}", first.checks);
        assert!(first.checks[0].get("cached").is_none());
        assert_eq!(exec_calls.load(std::sync::atomic::Ordering::Relaxed), 2);

        let second = run_deterministic_verify_checks_at(
            &specs,
            &state,
            cancel.clone(),
            None,
            Some(&db),
            settled_now + 10,
            Some(&run_local),
        )
        .await
        .unwrap();
        assert!(second.all_passed);
        assert_eq!(second.checks[0]["cached"], true);
        assert_eq!(second.checks[0]["detail"], first.checks[0]["detail"]);
        assert_eq!(exec_calls.load(std::sync::atomic::Ordering::Relaxed), 2);

        // Modifying workspace file invalidates cached verify receipt cleanly.
        std::fs::write(svc.join("ok.txt"), "v2\n").unwrap();
        let (_new_digest, new_mtime) = verify_tree_stamp(&svc_canon).unwrap();
        let third = run_deterministic_verify_checks_at(
            &specs,
            &state,
            cancel,
            None,
            Some(&db),
            new_mtime + VERIFY_RACY_NS + 100,
            Some(&run_local),
        )
        .await
        .unwrap();
        assert!(third.all_passed);
        assert!(third.checks[0].get("cached").is_none());
        assert_eq!(exec_calls.load(std::sync::atomic::Ordering::Relaxed), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exponential_health_weights_decay_old_failures_and_select_healthier_model() {
        let dir =
            std::env::temp_dir().join(format!("infra-sys-health-decay-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("system_health.sqlite3");

        // Record 3 bad runs on flaky-model, then 5 good runs -> exponential decay heals score.
        for i in 0..3 {
            record_system_health(
                &db,
                "system",
                "codex",
                Some("flaky-model"),
                &format!("bad-{i}"),
                "verify",
                "blocked",
                true,
                true,
                1.0,
            );
        }
        let degraded = summarize_system_health(&db, "system", "codex", Some("flaky-model"));
        assert!(degraded.is_degraded(), "{degraded:?}");
        assert_eq!(degraded.recommended_max_attempts(100), 2);
        assert_eq!(degraded.recommended_concurrency(4), 1);

        // Also record 4 healthy runs on steady-model.
        for i in 0..4 {
            record_system_health(
                &db,
                "system",
                "codex",
                Some("steady-model"),
                &format!("good-{i}"),
                "verify",
                "completed",
                false,
                false,
                0.5,
            );
        }
        assert_eq!(
            select_healthier_model(&db, "system", "codex", Some("flaky-model")).as_deref(),
            Some("steady-model")
        );

        // 8 consecutive recent good runs on flaky-model outweigh the 3 old failures.
        for i in 0..8 {
            record_system_health(
                &db,
                "system",
                "codex",
                Some("flaky-model"),
                &format!("recover-{i}"),
                "verify",
                "completed",
                false,
                false,
                0.5,
            );
        }
        let recovered = summarize_system_health(&db, "system", "codex", Some("flaky-model"));
        assert!(!recovered.is_degraded(), "{recovered:?}");
        assert!(recovered.warnings.is_empty(), "{recovered:?}");
        assert_eq!(recovered.recommended_max_attempts(100), 100);
        assert_eq!(
            select_healthier_model(&db, "system", "codex", Some("flaky-model")),
            None
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_tree_stamp_tier_prefers_scoped_and_graph_ir_over_walk() {
        let dir =
            std::env::temp_dir().join(format!("infra-sys-stamp-tier-{}", uuid::Uuid::new_v4()));
        let ws = dir.join("ws");
        let state = dir.join("state");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("composer.json"), r#"{"name":"acme/ws"}"#).unwrap();
        std::fs::write(ws.join("composer.lock"), "{}").unwrap();
        std::fs::write(ws.join("src/App.php"), "<?php\n").unwrap();
        std::fs::write(ws.join("unrelated.txt"), "noise-1\n").unwrap();

        let ws_canon = infra_core::fs::canonicalize(&ws).unwrap();

        // Tier 1: `composer validate` depends only on composer.{json,lock}, ignoring unrelated files.
        let composer_argv = vec![
            "composer".into(),
            "validate".into(),
            "--no-interaction".into(),
        ];
        let (tier1, digest1, _) =
            verify_tree_stamp_tier(&ws_canon, &composer_argv, Some(&state)).unwrap();
        assert_eq!(tier1, "scoped_files");
        std::fs::write(ws.join("unrelated.txt"), "noise-2\n").unwrap();
        let (_, digest1_after, _) =
            verify_tree_stamp_tier(&ws_canon, &composer_argv, Some(&state)).unwrap();
        assert_eq!(digest1, digest1_after);
        std::fs::write(
            ws.join("composer.json"),
            r#"{"name":"acme/ws","version":"1.0"}"#,
        )
        .unwrap();
        let (_, digest1_changed, _) =
            verify_tree_stamp_tier(&ws_canon, &composer_argv, Some(&state)).unwrap();
        assert_ne!(digest1, digest1_changed);

        // Tier 1 also handles `php -l src/App.php`.
        let php_argv = vec!["php".into(), "-l".into(), "src/App.php".into()];
        let (php_tier, _, _) = verify_tree_stamp_tier(&ws_canon, &php_argv, Some(&state)).unwrap();
        assert_eq!(php_tier, "scoped_files");

        // Tier 2: Graph-IR database present with synthetic `sys-<sha16>` avoids bounded walk.
        let root_hash16 = &digest(ws_canon.to_string_lossy().as_bytes())[..16];
        let ir_home = state.join("graphs").join(format!("sys-{root_hash16}"));
        std::fs::create_dir_all(&ir_home).unwrap();
        let ir_db = ir_home.join("graph_ir.sqlite3");
        let conn = rusqlite::Connection::open(&ir_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE ir_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE ir_files(path TEXT PRIMARY KEY, language TEXT NOT NULL, size INTEGER NOT NULL, mtime_ns INTEGER NOT NULL, content_sha256 TEXT NOT NULL, namespace TEXT, indexed_at TEXT NOT NULL);
             INSERT INTO ir_meta(key, value) VALUES ('write_epoch', '7');",
        )
        .unwrap();
        let app_meta = std::fs::symlink_metadata(ws.join("src/App.php")).unwrap();
        let app_mtime = time_to_unix_ns(app_meta.modified().unwrap()).unwrap();
        conn.execute(
            "INSERT INTO ir_files(path, language, size, mtime_ns, content_sha256, namespace, indexed_at) VALUES(?1,'php',?2,?3,'abc','','now')",
            rusqlite::params!["src/App.php", i64::try_from(app_meta.len()).unwrap(), i64::try_from(app_mtime).unwrap()],
        )
        .unwrap();
        drop(conn);

        let generic_argv = vec!["vendor/bin/phpunit".into()];
        let (tier2, ir_digest, _) =
            verify_tree_stamp_tier(&ws_canon, &generic_argv, Some(&state)).unwrap();
        assert_eq!(tier2, "ir_indexed");
        assert!(!ir_digest.is_empty());

        // Tier 3: without Graph-IR state_dir or narrow closure, falls back to bounded_walk.
        let (tier3, _, _) = verify_tree_stamp_tier(&ws_canon, &generic_argv, None).unwrap();
        assert_eq!(tier3, "bounded_walk");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
