//! Owner-local deferred mutations; only typed update/explicit-flush are admitted.
use crate::{
    Result, RuntimeContext, RuntimeError,
    domain::{self, clock, json},
    query,
    storage::UpdateTask,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum QueuedOperation {
    Update {
        update: UpdateTask,
        revision: Option<u64>,
        actor: Option<String>,
        phase: Option<String>,
        allow_phase_regression: bool,
    },
    Flush {
        task_id: String,
        max_files: usize,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueuedCommand {
    pub owner: String,
    pub mode: Option<String>,
    pub project: Option<PathBuf>,
    pub operation: QueuedOperation,
}
pub trait QueueDispatcher {
    fn dispatch(&mut self, command: &QueuedCommand) -> Result<Value>;
}
/// Raw filesystem encoding used by Python os.fsencode; no JSON projection,
/// separator rewriting, replacement character or Unicode normalization.
fn queue_path_bytes(value: &std::ffi::OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        value.as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut result = Vec::new();
        for character in std::char::decode_utf16(value.encode_wide()) {
            match character {
                Ok(character) => {
                    let mut buffer = [0; 4];
                    result.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
                }
                Err(error) => {
                    let code = u32::from(error.unpaired_surrogate());
                    result.extend_from_slice(&[
                        (0xe0 | (code >> 12)) as u8,
                        (0x80 | ((code >> 6) & 63)) as u8,
                        (0x80 | (code & 63)) as u8,
                    ]);
                }
            }
        }
        result
    }
    #[cfg(not(any(unix, windows)))]
    {
        value.as_encoded_bytes().to_vec()
    }
}
pub fn key(context: &RuntimeContext) -> Result<String> {
    let state = crate::fs::python_realpath(&context.roots.state)?;
    let project = crate::fs::python_realpath(&context.roots.project)?;
    let mut bytes = queue_path_bytes(state.as_os_str());
    bytes.push(0);
    bytes.extend_from_slice(&queue_path_bytes(project.as_os_str()));
    Ok(query::sha256(&bytes)[..16].into())
}
fn directories() -> Vec<PathBuf> {
    crate::native::spool_directories()
        .into_iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .map(|parent| {
            #[cfg(unix)]
            let uid = unsafe { libc::getuid() }.to_string();
            #[cfg(not(unix))]
            let uid = "user".to_string();
            parent.join(format!("ai-accelerator-memory-queue-{uid}"))
        })
        .collect()
}
fn argv(command: &QueuedCommand) -> Result<Vec<String>> {
    crate::brain::actor(&command.owner, false)?;
    let mut args = vec![format!("--owner={}", command.owner)];
    if let Some(mode) = &command.mode {
        if !["governed", "lightweight"].contains(&mode.as_str()) {
            return Err(RuntimeError::invalid("Invalid queued mode"));
        }
        args.push(format!("--mode={mode}"));
    }
    if let Some(project) = &command.project {
        args.push(format!("--project={}", project.display()));
    }
    match &command.operation {
        QueuedOperation::Update {
            update,
            revision,
            actor,
            phase,
            allow_phase_regression,
        } => {
            crate::storage::validate_task_id(&update.task_id)?;
            if update
                .progress
                .as_ref()
                .is_some_and(|progress| progress.trim().is_empty())
                || update.progress.is_none()
                    && update.next_steps.is_empty()
                    && update.files.is_empty()
                    && update.sources.is_empty()
                    && phase.is_none()
                    && !update.replace_next_steps
            {
                return Err(RuntimeError::invalid(
                    "Working task update requires a changed field",
                ));
            }
            let mut introduced = update.progress.iter().cloned().collect::<Vec<_>>();
            introduced.extend(update.next_steps.clone());
            introduced.extend(update.files.clone());
            introduced.extend(update.sources.clone());
            domain::guard_shared_text("Working task", &introduced)?;
            args.extend(["update".into(), format!("--task-id={}", update.task_id)]);
            if let Some(progress) = &update.progress {
                args.push(format!("--progress={progress}"));
            }
            args.extend(
                update
                    .next_steps
                    .iter()
                    .map(|value| format!("--next-step={value}")),
            );
            args.extend(update.files.iter().map(|value| format!("--file={value}")));
            args.extend(
                update
                    .sources
                    .iter()
                    .map(|value| format!("--source={value}")),
            );
            if let Some(revision) = revision {
                args.push(format!("--revision={revision}"));
            }
            if let Some(actor) = actor {
                args.push(format!("--actor={actor}"));
            }
            if let Some(phase) = phase {
                domain::normalize_phase(phase)?;
                args.push(format!("--phase={phase}"));
            }
            if update.replace_next_steps {
                args.push("--replace-next-steps".into());
            }
            if *allow_phase_regression {
                args.push("--allow-phase-regression".into());
            }
        }
        QueuedOperation::Flush { task_id, max_files } => {
            crate::storage::validate_task_id(task_id)?;
            if *max_files == 0 {
                return Err(RuntimeError::invalid("--max-files must be positive"));
            }
            args.extend([
                "turn".into(),
                format!("--task-id={task_id}"),
                "--flush".into(),
                format!("--max-files={max_files}"),
            ]);
        }
    }
    args.push("--json".into());
    Ok(args)
}
pub fn parse_argv(args: &[String]) -> Result<QueuedCommand> {
    if args.len() < 2 || args.len() > 512 || args.iter().any(|value| value.contains('\0')) {
        return Err(RuntimeError::invalid("Invalid queued command"));
    }
    let mut owner = None;
    let mut mode = None;
    let mut project = None;
    let mut operation = None::<String>;
    let mut update = UpdateTask::default();
    let mut revision = None;
    let mut actor = None;
    let mut phase = None;
    let mut allow_phase_regression = false;
    let mut flush = false;
    let mut max_files = 20;
    let mut json = false;
    let mut at = 0;
    while at < args.len() {
        let raw = &args[at];
        if !raw.starts_with("--") {
            if operation.is_some() || !["update", "turn"].contains(&raw.as_str()) {
                return Err(RuntimeError::invalid("Unsupported queued command"));
            }
            operation = Some(raw.clone());
            at += 1;
            continue;
        }
        let (name, inline) = raw[2..]
            .split_once('=')
            .map(|(name, value)| (name, Some(value)))
            .unwrap_or((&raw[2..], None));
        let flag = [
            "json",
            "flush",
            "replace-next-steps",
            "allow-phase-regression",
        ]
        .contains(&name);
        let value = if flag {
            None
        } else if let Some(value) = inline {
            Some(value)
        } else {
            at += 1;
            Some(
                args.get(at)
                    .ok_or_else(|| RuntimeError::invalid("Missing queued argument"))?
                    .as_str(),
            )
        };
        match name {
            "owner" => owner = value.map(String::from),
            "mode" => mode = value.map(String::from),
            "project" => project = value.map(PathBuf::from),
            "task-id" => update.task_id = value.unwrap_or("").into(),
            "progress" => update.progress = value.map(String::from),
            "next-step" => update.next_steps.push(value.unwrap_or("").into()),
            "file" => update.files.push(value.unwrap_or("").into()),
            "source" => update.sources.push(value.unwrap_or("").into()),
            "revision" => {
                revision = Some(
                    value
                        .unwrap_or("")
                        .parse()
                        .map_err(|_| RuntimeError::invalid("Invalid queued revision"))?,
                )
            }
            "actor" => actor = value.map(String::from),
            "phase" => phase = value.map(String::from),
            "max-files" => {
                max_files = value
                    .unwrap_or("")
                    .parse()
                    .map_err(|_| RuntimeError::invalid("Invalid queued max-files"))?
            }
            "replace-next-steps" => update.replace_next_steps = true,
            "allow-phase-regression" => allow_phase_regression = true,
            "flush" => flush = true,
            "json" => json = true,
            _ => return Err(RuntimeError::invalid("Unsupported queued argument")),
        }
        at += 1;
    }
    if !json {
        return Err(RuntimeError::invalid("Queued command requires JSON output"));
    }
    let operation = match operation.as_deref() {
        Some("update") => QueuedOperation::Update {
            update,
            revision,
            actor,
            phase,
            allow_phase_regression,
        },
        Some("turn") if flush => QueuedOperation::Flush {
            task_id: update.task_id,
            max_files,
        },
        _ => return Err(RuntimeError::invalid("Unsupported queued command")),
    };
    let command = QueuedCommand {
        owner: owner
            .filter(|owner| !owner.is_empty())
            .ok_or_else(|| RuntimeError::invalid("Queued owner required"))?,
        mode,
        project,
        operation,
    };
    argv(&command)?;
    Ok(command)
}
pub fn writable(context: &RuntimeContext, database: &Path) -> Result<bool> {
    let mut folders = vec![
        database
            .parent()
            .ok_or_else(|| RuntimeError::invalid("Database has no parent"))?
            .to_path_buf(),
    ];
    if context.mode == "governed" {
        folders.push(context.roots.brain_root().join("local"));
    }
    for folder in folders {
        let operation = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&folder)?;
            let path = folder.join(format!(
                ".write-probe-{}-{}",
                std::process::id(),
                domain::uuid4()
            ));
            let mut options = std::fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            drop(options.open(&path)?);
            std::fs::remove_file(path)
        })();
        if let Err(error) = operation {
            if error
                .raw_os_error()
                .is_some_and(|code| [libc::EROFS, libc::EACCES, libc::EPERM].contains(&code))
            {
                return Ok(false);
            }
            return Err(error.into());
        }
    }
    Ok(true)
}
pub fn enqueue(context: &RuntimeContext, command: &QueuedCommand) -> Result<Value> {
    if clock::is_controlled() {
        return Err(RuntimeError::coded(
            crate::ErrorKind::Unavailable,
            "eval-clock-child-unsupported",
            "The controlled memory clock cannot be handed to a queued worker; no command was queued",
        ));
    }
    let args = argv(command)?;
    let key = key(context)?;
    let now = clock::proof_now();
    let record = object!({"profile":"memory-command-queue-v1","key":key,"created":now.timestamp_micros()as f64/1_000_000.0,"argv":args});
    let payload = json::python_default(&record, false, false, true)?;
    if payload.len() > 64 * 1024 {
        return Err(RuntimeError::invalid("Queued command exceeds limit"));
    }
    let name = format!(
        "command-{key}-{:020}-{}-{}.json",
        now.timestamp_nanos_opt().unwrap_or(0),
        std::process::id(),
        &domain::uuid4().replace('-', "")[..6]
    );
    let mut stored = false;
    for folder in directories() {
        if crate::fs::ensure_directory(&folder, true).is_err() || !crate::native::owned(&folder) {
            continue;
        }
        if crate::fs::atomic_write(&folder.join(&name), payload.as_bytes()).is_ok() {
            stored = true;
            break;
        }
    }
    if !stored {
        return Err(RuntimeError::unavailable(
            "This sandbox cannot write the project's memory, nor a temporary folder of yours for the command to wait in",
        ));
    }
    let task = match &command.operation {
        QueuedOperation::Update { update, .. } => &update.task_id,
        QueuedOperation::Flush { task_id, .. } => task_id,
    };
    Ok(
        object!({"task_id":task,"queued":true,"detail":"This sandbox cannot write the project's memory: the command waits in a temporary folder of yours, and the session's next hook makes it."}),
    )
}
fn record(path: &Path) -> Option<Value> {
    let operation = (|| -> Result<Value> {
        let image = std::fs::symlink_metadata(path)?;
        if !image.is_file()
            || crate::fs::linked_at(path, &image)?
            || image.len() > 64 * 1024
            || !crate::native::owned(path)
        {
            return Err(RuntimeError::policy("Queued record unavailable"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if image.nlink() != 1 {
                return Err(RuntimeError::policy("Queued record unavailable"));
            }
        }
        let value = crate::results::strict_input(&crate::fs::read_regular(path)?)?;
        if value["profile"] != "memory-command-queue-v1"
            || !value["created"].as_f64().is_some_and(f64::is_finite)
            || !value["argv"].as_array().is_some_and(|arguments| {
                (2..=512).contains(&arguments.len())
                    && arguments.iter().all(|argument| {
                        argument
                            .as_str()
                            .is_some_and(|argument| !argument.contains('\0'))
                    })
            })
        {
            return Err(RuntimeError::invalid("Invalid queued record"));
        }
        Ok(value)
    })();
    operation.ok()
}
pub fn drain(
    context: &RuntimeContext,
    dispatcher: &mut impl QueueDispatcher,
) -> Result<Vec<Value>> {
    if clock::is_controlled() {
        return if has_waiting(context)? {
            Err(RuntimeError::coded(
                crate::ErrorKind::Unavailable,
                "eval-clock-child-unsupported",
                "Queued memory commands require a child that cannot retain the controlled clock",
            ))
        } else {
            Ok(vec![])
        };
    }
    if std::env::var("CONTEXT_QUEUE_DRAIN").as_deref() == Ok("1") {
        return Ok(vec![]);
    }
    let prefix = format!("command-{}-", key(context)?);
    let mut outcomes = Vec::new();
    let mut waiting = Vec::new();
    let mut held = Vec::new();
    for folder in directories() {
        if !crate::native::owned(&folder) || !folder.is_dir() {
            continue;
        }
        let Some(guard) = crate::native::DrainGuard::acquire(
            &folder,
            prefix.strip_suffix('-').unwrap_or(&prefix),
        )?
        else {
            continue;
        };
        held.push(guard);
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with(&prefix) || !name.ends_with(".json") {
                continue;
            }
            let stamp = &name[prefix.len()..name.len() - 5];
            let (stamp, begun) = stamp
                .rsplit_once(".attempt")
                .and_then(|(stamp, attempt)| {
                    if attempt.len() == 1 && matches!(attempt.as_bytes()[0], b'1'..=b'9') {
                        attempt
                            .parse::<usize>()
                            .ok()
                            .map(|attempt| (stamp, attempt))
                    } else {
                        None
                    }
                })
                .unwrap_or((stamp, 0));
            waiting.push((stamp.to_string(), begun, folder.clone(), path));
        }
    }
    waiting.sort();
    for (stamp, begun, folder, path) in waiting {
        let value = record(&path);
        let parsed = value
            .as_ref()
            .and_then(|value| value["argv"].as_array())
            .and_then(|arguments| {
                arguments
                    .iter()
                    .map(|argument| argument.as_str().map(String::from))
                    .collect::<Option<Vec<_>>>()
            })
            .and_then(|arguments| parse_argv(&arguments).ok());
        let name = parsed.as_ref().map(|command| match &command.operation {
            QueuedOperation::Update { .. } => "update",
            QueuedOperation::Flush { .. } => "turn",
        });
        if begun >= 3 {
            if std::fs::remove_file(&path).is_ok() {
                outcomes.push(object!({"command":name,"made":false,"code":"interrupted","detail":"it was stopped before it finished, 3 times"}));
            }
            continue;
        }
        let attempt = folder.join(format!("{prefix}{stamp}.attempt{}.json", begun + 1));
        if std::fs::rename(&path, &attempt).is_err() {
            continue;
        }
        let mut keep = false;
        let outcome = if let (Some(command), Some(value)) = (parsed, value) {
            let age = clock::proof_now().timestamp_micros() as f64 / 1_000_000.0
                - value["created"].as_f64().unwrap_or(f64::INFINITY);
            if value["key"] != key(context)? || !(0.0..=86400.0).contains(&age) {
                object!({"command":name,"made":false,"code":"expired","detail":"it waited more than a day"})
            } else {
                match dispatcher.dispatch(&command) {
                    Ok(_) => object!({"command":name,"made":true}),
                    Err(error) if error.code.as_deref() == Some("queue-worker-unavailable") => {
                        keep = true;
                        object!({"command":name,"made":false,"code":"unfinished","detail":"the native worker is unavailable; the attempt remains queued"})
                    }
                    Err(error) if error.code.as_deref() == Some("queue-worker-unfinished") => {
                        object!({"command":name,"made":false,"code":"unfinished","detail":"it did not finish"})
                    }
                    Err(error) => {
                        object!({"command":name,"made":false,"code":"refused","detail":query::chars(&error.message,200)})
                    }
                }
            }
        } else {
            object!({"command":name,"made":false,"code":"unreadable","detail":"the waiting command could not be read"})
        };
        outcomes.push(outcome);
        if !keep {
            let _ = std::fs::remove_file(attempt);
        }
    }
    drop(held);
    if !outcomes.is_empty() {
        let native = crate::NativeService::new(context.clone());
        for outcome in &outcomes {
            native.journal(
                None,
                "queue",
                if outcome["made"] == true {
                    "made"
                } else {
                    "refused"
                },
                outcome["code"].as_str(),
                &Value::Null,
            );
        }
        let refused = outcomes
            .iter()
            .filter(|outcome| outcome["made"] != true)
            .map(|outcome| {
                format!(
                    "{} - {}",
                    outcome["command"].as_str().unwrap_or("command"),
                    outcome["detail"].as_str().unwrap_or("unavailable")
                )
            })
            .collect::<Vec<_>>();
        if !refused.is_empty() {
            let mut report =
                crate::advisory::load_last_turn(context).unwrap_or(object!({"schema_version":1}));
            report["timestamp"] = object!(domain::utc_now());
            report["queued_refused"] = object!(refused[refused.len().saturating_sub(3)..].to_vec());
            crate::advisory::write_last_turn(context, &report);
        }
    }
    Ok(outcomes)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct QueueScope {
    tooling_root: PathBuf,
    project_root: PathBuf,
    state_root: PathBuf,
    key: String,
    shared: bool,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerPacket {
    profile: String,
    scope: QueueScope,
    command: QueuedCommand,
}
#[derive(Clone, Debug)]
pub struct NativeQueueWorker {
    engine: PathBuf,
    identity: (u64, u64, u32, u64, u64, i128, i128),
    scope: QueueScope,
}
fn queue_scope(context: &RuntimeContext) -> Result<QueueScope> {
    Ok(QueueScope {
        tooling_root: crate::fs::python_realpath(&context.roots.tooling)?,
        project_root: crate::fs::python_realpath(&context.roots.project)?,
        state_root: crate::fs::python_realpath(&context.roots.state)?,
        key: key(context)?,
        shared: context.roots.shared,
    })
}
fn engine_identity(engine: &Path) -> Result<(u64, u64, u32, u64, u64, i128, i128)> {
    if !engine.is_absolute() {
        return Err(RuntimeError::invalid("Queue engine path must be absolute"));
    }
    let parent = engine
        .parent()
        .ok_or_else(|| RuntimeError::invalid("Queue engine has no parent"))?;
    let name = engine
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| RuntimeError::invalid("Queue engine name must be UTF-8"))?;
    let mut files = crate::bank::publication::PublicationFiles::new(parent)?;
    // The caller explicitly selected an executable source. Cargo may hardlink
    // it into deps/: capture is read-only and retains its complete image identity.
    // Publication/data mutation readers continue to require one-link ownership.
    files
        .capture_source(name, 512 * 1024 * 1024)?
        .map(|(_, identity)| identity)
        .ok_or_else(|| {
            RuntimeError::coded(
                crate::ErrorKind::Unavailable,
                "queue-worker-unavailable",
                "Native queue engine is unavailable",
            )
        })
}
pub fn worker_capabilities() -> Value {
    object!({"profile":"memory-queue-capabilities-v1","engine":"infra-engine","protocols":["memory-queue-worker-v1"]})
}
impl NativeQueueWorker {
    pub fn admit(engine: PathBuf, context: &RuntimeContext) -> Result<Self> {
        let engine = std::fs::canonicalize(engine)?;
        let identity = engine_identity(&engine)?;
        #[cfg(unix)]
        if identity.2 & 0o111 == 0 {
            return Err(RuntimeError::invalid("Queue engine is not executable"));
        }
        let scope = queue_scope(context)?;
        let output = crate::queue_process::run(
            &engine,
            &["__memory-queue-capabilities".into()],
            &scope.project_root,
            vec![],
            std::time::Duration::from_secs(2),
            4096,
            &scope_environment(&scope),
        )?;
        if output.code != 0
            || crate::results::strict_input(&output.stdout)? != worker_capabilities()
            || engine_identity(&engine)? != identity
        {
            return Err(RuntimeError::coded(
                crate::ErrorKind::Unavailable,
                "queue-worker-unavailable",
                "Native queue engine capabilities or identity unavailable",
            ));
        }
        Ok(Self {
            engine,
            identity,
            scope,
        })
    }
    pub fn matches(&self, context: &RuntimeContext) -> Result<bool> {
        Ok(self.scope == queue_scope(context)? && engine_identity(&self.engine)? == self.identity)
    }
}
impl QueueDispatcher for NativeQueueWorker {
    fn dispatch(&mut self, command: &QueuedCommand) -> Result<Value> {
        if clock::is_controlled() {
            return Err(RuntimeError::coded(
                crate::ErrorKind::Unavailable,
                "eval-clock-child-unsupported",
                "The controlled memory clock cannot be handed to a native queue child",
            ));
        }
        if engine_identity(&self.engine)? != self.identity {
            return Err(RuntimeError::coded(
                crate::ErrorKind::Unavailable,
                "queue-worker-unavailable",
                "Native queue engine changed",
            ));
        }
        let packet = WorkerPacket {
            profile: "memory-queue-worker-v1".into(),
            scope: self.scope.clone(),
            command: command.clone(),
        };
        let input = json::canonical(&serde_json::to_value(packet)?)?.into_bytes();
        if input.len() > 64 * 1024 {
            return Err(RuntimeError::invalid("Native queue packet exceeds limit"));
        }
        let args = vec![
            "__memory-queue-worker".into(),
            "--tooling-root".into(),
            self.scope.tooling_root.as_os_str().into(),
            "--project-root".into(),
            self.scope.project_root.as_os_str().into(),
        ];
        let output = crate::queue_process::run(
            &self.engine,
            &args,
            &self.scope.project_root,
            input,
            std::time::Duration::from_secs(30),
            64 * 1024,
            &scope_environment(&self.scope),
        )?;
        if engine_identity(&self.engine)? != self.identity {
            return Err(RuntimeError::coded(
                crate::ErrorKind::Unavailable,
                "queue-worker-unavailable",
                "Native queue engine changed during execution",
            ));
        }
        if output.code != 0 {
            return Err(RuntimeError::conflict(
                "Native queue worker refused the command",
            ));
        }
        let reply = crate::results::strict_input(&output.stdout)?;
        if reply.as_object().is_none_or(|fields| fields.len() != 3)
            || reply["profile"] != "memory-queue-worker-v1"
            || reply["ok"] != true
        {
            return Err(RuntimeError::invalid(
                "Native queue worker returned an invalid result",
            ));
        }
        Ok(reply["result"].clone())
    }
}
/// The CLI admits exactly the two pinned root flags before calling this
/// boundary. State/database/operation overrides never pass through argv.
pub fn worker_entry(tooling: &Path, project: &Path, input: &[u8]) -> Result<Value> {
    if input.is_empty() || input.len() > 64 * 1024 {
        return Err(RuntimeError::invalid("Native queue packet exceeds limit"));
    }
    let value = crate::results::strict_input(input)?;
    let packet: WorkerPacket = serde_json::from_value(value.clone())?;
    if packet.profile != "memory-queue-worker-v1" || serde_json::to_value(&packet)? != value {
        return Err(RuntimeError::invalid("Invalid native queue packet"));
    }
    let tooling = crate::fs::python_realpath(tooling)?;
    let project = crate::fs::python_realpath(project)?;
    if packet.scope.tooling_root != tooling
        || packet.scope.project_root != project
        || packet
            .command
            .project
            .as_ref()
            .is_some_and(|root| crate::fs::python_realpath(root).ok().as_ref() != Some(&project))
    {
        return Err(RuntimeError::policy("Native queue project scope changed"));
    }
    let roots = crate::Roots::discover(&packet.scope.state_root, &tooling)?;
    if roots.tooling != tooling
        || roots.project != project
        || roots.state != packet.scope.state_root
        || roots.shared != packet.scope.shared
    {
        return Err(RuntimeError::policy("Native queue root custody changed"));
    }
    let context = RuntimeContext::new(
        roots,
        &packet.command.owner,
        packet.command.mode.as_deref().unwrap_or("governed"),
    )?;
    crate::brain::actor(&packet.command.owner, false)?;
    if queue_scope(&context)? != packet.scope {
        return Err(RuntimeError::policy("Native queue scope key changed"));
    }
    let arguments = argv(&packet.command)?;
    parse_argv(&arguments)?;
    let mut service = crate::MemoryService::new(context, None);
    let result = QueueDispatcher::dispatch(&mut service, &packet.command)?;
    Ok(object!({"profile":"memory-queue-worker-v1","ok":true,"result":result}))
}
pub fn drain_configured(
    context: &RuntimeContext,
    engine: Option<&NativeQueueWorker>,
) -> Result<Vec<Value>> {
    if clock::is_controlled() {
        return if has_waiting(context)? {
            Err(RuntimeError::coded(
                crate::ErrorKind::Unavailable,
                "eval-clock-child-unsupported",
                "Queued memory commands require a child that cannot retain the controlled clock",
            ))
        } else {
            Ok(vec![])
        };
    }
    if let Some(engine) = engine {
        if !engine.matches(context)? {
            return Err(RuntimeError::coded(
                crate::ErrorKind::Unavailable,
                "queue-worker-unavailable",
                "Native queue worker scope is unavailable",
            ));
        }
        return drain(context, &mut engine.clone());
    }
    let prefix = format!("command-{}-", key(context)?);
    let waiting = directories()
        .into_iter()
        .filter(|folder| crate::native::owned(folder))
        .any(|folder| {
            std::fs::read_dir(folder)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(std::result::Result::ok)
                .any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".json"))
                })
        });
    Ok(if waiting {
        vec![
            object!({"command":null,"made":false,"code":"worker-unavailable","detail":"the native queue worker is unavailable; the command remains queued"}),
        ]
    } else {
        vec![]
    })
}

fn scope_environment(scope: &QueueScope) -> BTreeMap<std::ffi::OsString, std::ffi::OsString> {
    let mut environment = BTreeMap::new();
    if scope.project_root != scope.state_root && !scope.shared {
        for (name, value) in [
            ("ACCELERATOR_HOME", &scope.tooling_root),
            ("ACCELERATOR_STATE_DIR", &scope.state_root),
            ("ACCELERATOR_PROJECT_DIR", &scope.project_root),
        ] {
            environment.insert(name.into(), value.as_os_str().to_os_string());
        }
    }
    environment
}

pub fn has_waiting(context: &RuntimeContext) -> Result<bool> {
    let prefix = format!("command-{}-", key(context)?);
    for folder in directories() {
        if !crate::native::owned(&folder) || !folder.is_dir() {
            continue;
        }
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Err(RuntimeError::coded(
                    crate::ErrorKind::Unavailable,
                    "eval-clock-child-unsupported",
                    "The controlled memory clock cannot establish whether native queued commands are waiting",
                ));
            }
        };
        for entry in entries {
            let entry=entry.map_err(|_|RuntimeError::coded(crate::ErrorKind::Unavailable,"eval-clock-child-unsupported","The controlled memory clock cannot establish whether native queued commands are waiting"))?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".json"))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod queued_record_contract_tests {
    use super::*;

    struct WireFile {
        directory: PathBuf,
        path: PathBuf,
    }
    impl WireFile {
        fn new(arguments: Value) -> Self {
            let directory = std::env::temp_dir()
                .join(format!("infra-queued-record-{}", crate::domain::uuid4()));
            std::fs::create_dir(&directory).unwrap();
            let path = directory.join("record.json");
            std::fs::write(
                &path,
                serde_json::to_vec(&object!({
                    "profile":"memory-command-queue-v1","created":1,"key":"fixture",
                    "argv":arguments
                }))
                .unwrap(),
            )
            .unwrap();
            Self { directory, path }
        }
    }
    impl Drop for WireFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
    fn command() -> Value {
        object!([
            "--owner=local",
            "update",
            "--task-id=TASK-QUEUE",
            "--progress=Bounded update",
            "--revision=1",
            "--json"
        ])
    }

    #[test]
    fn queued_record_rejects_every_non_string_argument_instead_of_dropping_it() {
        let good = WireFile::new(command());
        let value = record(&good.path).expect("valid wire record");
        let arguments = value["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(parse_argv(&arguments).is_ok());
        for invalid in [
            Value::Null,
            object!(false),
            object!(7),
            object!({}),
            object!([]),
        ] {
            let mut arguments = command().as_array().unwrap().clone();
            arguments.insert(2, invalid.clone());
            let wire = WireFile::new(object!(arguments));
            assert!(
                record(&wire.path).is_none(),
                "an argv item {invalid:?} must reject the whole persisted command"
            );
        }
    }

    #[test]
    fn queued_record_enforces_original_argument_count_and_nul_boundaries() {
        for arguments in [
            object!([]),
            object!(["--json"]),
            object!(vec!["--json"; 513]),
            object!(["--owner=local", "update", "--task-id=TASK\0QUEUE", "--json"]),
        ] {
            let wire = WireFile::new(arguments);
            assert!(record(&wire.path).is_none());
        }
    }
}

#[cfg(test)]
mod selected_queue_engine_source_tests {
    use super::*;
    struct Owned(PathBuf);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> Owned {
        let root = std::env::temp_dir().join(format!(
            "infra-queue-engine-source-{}",
            crate::domain::uuid4()
        ));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Owned(root)
    }

    #[test]
    fn selected_queue_engine_accepts_hardlinks_and_full_identity_detects_replacement() {
        let owned = fixture();
        let engine = owned.0.join("selected-engine");
        let alias = owned.0.join("second-engine-name");
        std::fs::copy(std::env::current_exe().unwrap(), &engine).unwrap();
        std::fs::hard_link(&engine, &alias).unwrap();
        let before = engine_identity(&engine).expect("selected Cargo-style source is admitted");
        assert_eq!(before.3, 2);
        let mut files = crate::bank::publication::PublicationFiles::new(&owned.0).unwrap();
        assert!(
            files.image_identity("selected-engine").is_err(),
            "ordinary publication identity must retain its one-link boundary"
        );
        let (_, captured) = files
            .capture_source("selected-engine", 512 * 1024 * 1024)
            .unwrap()
            .unwrap();
        assert_eq!(before, captured);
        let retained = owned.0.join("retained-original");
        std::fs::rename(&engine, &retained).unwrap();
        std::fs::copy(&alias, &engine).unwrap();
        let after = engine_identity(&engine).unwrap();
        assert_ne!(
            (before.0, before.1),
            (after.0, after.1),
            "an equal-byte replacement must not inherit the admitted source identity"
        );
        assert_ne!(before, after);
        assert_eq!(std::fs::metadata(&engine).unwrap().len(), before.4);
        assert_eq!(std::fs::metadata(&retained).unwrap().len(), before.4);
    }

    #[test]
    fn selected_queue_engine_keeps_its_source_size_and_nofollow_bounds() {
        let owned = fixture();
        let oversized = owned.0.join("oversized-engine");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&oversized)
            .unwrap();
        file.set_len(512 * 1024 * 1024 + 1).unwrap();
        assert!(engine_identity(&oversized).is_err());
        let missing = engine_identity(&owned.0.join("missing-engine")).unwrap_err();
        assert_eq!(missing.code.as_deref(), Some("queue-worker-unavailable"));
        #[cfg(unix)]
        {
            let source = owned.0.join("ordinary");
            std::fs::write(&source, b"source identity control\n").unwrap();
            let link = owned.0.join("linked-engine");
            std::os::unix::fs::symlink(&source, &link).unwrap();
            assert!(engine_identity(&link).is_err());
        }
    }
}
