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
        &["rejection", "failure"],
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
