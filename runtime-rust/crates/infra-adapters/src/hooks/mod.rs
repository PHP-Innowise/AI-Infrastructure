//! Native implementations of the nine canonical host hook roles.
pub mod banner;
pub mod guards;
pub mod memory;
use crate::{output::Output, process};
use infra_core::{Result, Roots, RuntimeError, git::GitFrame};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
pub const ROLES: &[&str] = &[
    "bash-validator",
    "file-naming-validator",
    "loop-detection",
    "subagent-gate",
    "subagent-dispatch",
    "working-memory-read",
    "working-memory-write",
    "context-continuity",
    "local-context",
];
/// Roles that only read or record memory. Their own failure never refuses a
/// prompt, a stop or a session start: hosts read exit 2 there as a refusal.
pub const MEMORY_ROLES: &[&str] = &[
    "working-memory-read",
    "working-memory-write",
    "context-continuity",
    "local-context",
];
/// Roles that act on the session's task, which is inferred from the branch.
const TASK_ROLES: &[&str] = &[
    "working-memory-read",
    "working-memory-write",
    "subagent-dispatch",
];
#[derive(Clone, Debug)]
pub struct Context {
    pub roots: Roots,
    pub host: String,
    pub payload: Value,
    pub raw: Vec<u8>,
    pub task: Option<String>,
    pub inferred: bool,
    pub entered: bool,
    pub other_worktree: bool,
    /// The host's session directory when it lies outside the project these
    /// hooks are bound to and outside that project's repository.
    pub outside: Option<PathBuf>,
    pub budget: Duration,
}
pub fn first<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    match value {
        Value::Object(map) => map
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| map.values().find_map(|value| first(value, key))),
        Value::Array(items) => items.iter().find_map(|value| first(value, key)),
        _ => None,
    }
}
pub fn strings(value: &Value, keys: &[&str]) -> Vec<String> {
    fn visit(value: &Value, keys: &[&str], output: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for key in keys {
                    if let Some(value) = map
                        .get(*key)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        && !output.iter().any(|prior| prior == value)
                    {
                        output.push(value.into());
                    }
                }
                for value in map.values() {
                    visit(value, keys, output);
                }
            }
            Value::Array(items) => {
                for value in items {
                    visit(value, keys, output);
                }
            }
            _ => {}
        }
    }
    let mut output = vec![];
    visit(value, keys, &mut output);
    output
}
pub fn valid_path(value: Option<&str>, limit: usize) -> Option<PathBuf> {
    let value = value?;
    if value.chars().count() > limit
        || value.contains(['\n', '\0'])
        || !Path::new(value).is_absolute()
    {
        None
    } else {
        Some(PathBuf::from(value))
    }
}
pub fn project_root(project: &Path) -> PathBuf {
    GitFrame::discover(project)
        .ok()
        .flatten()
        .and_then(|frame| infra_core::fs::python_realpath_spelling(project, frame.top).ok())
        .unwrap_or_else(|| project.to_path_buf())
}
pub fn task(project: &Path) -> Result<Option<String>> {
    if let Ok(task) = std::env::var("CONTEXT_TASK_ID")
        && !task.is_empty()
    {
        return Ok(Some(task));
    }
    let Some(frame) = GitFrame::discover(project)? else {
        return Ok(None);
    };
    frame.working_identity()
}
/// The session directory the host names when every such directory lies
/// outside the bound roots and outside the project's repository. Claude Code's
/// `CLAUDE_PROJECT_DIR` stays where the session started when its shell changes
/// directory, so it is preferred to `cwd`; Cursor names `workspace_roots`.
/// Installed wiring carries the absolute paths of the checkout it was
/// installed in, so a copy or a second clone of the project would otherwise
/// read and record the original checkout's memory.
fn outside_session(host: &str, payload: &Value, roots: &Roots) -> Option<PathBuf> {
    let started = std::env::var("CLAUDE_PROJECT_DIR")
        .ok()
        .filter(|path| host == "claude" && !path.is_empty());
    let named = match (started, payload["cwd"].as_str()) {
        (Some(started), _) => vec![started],
        (None, Some(cwd)) => vec![cwd.to_owned()],
        (None, None) => payload["workspace_roots"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    };
    let directories = named
        .iter()
        .filter_map(|path| valid_path(Some(path), 4096))
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    let mut repository = None;
    for directory in &directories {
        if roots.contains(directory).unwrap_or(true) {
            return None;
        }
        let ours = repository.get_or_insert_with(|| {
            GitFrame::discover(&roots.project)
                .ok()
                .flatten()
                .and_then(|frame| frame.common_dir().ok())
        });
        if ours.is_some()
            && GitFrame::discover(directory)
                .ok()
                .flatten()
                .and_then(|frame| frame.common_dir().ok())
                == *ours
        {
            return None;
        }
    }
    directories.into_iter().next()
}
/// What a hook prints when it does nothing for this turn.
pub fn passed(host: &str, role: &str) -> Output {
    if host == "cursor" && role == "working-memory-read" {
        Output::text("{\"continue\": true}\n")
    } else {
        Output::default()
    }
}
fn outside_notice(session: &Path, project: &Path) -> Output {
    Output::text(format!(
        "Memory not consulted: this session runs in {}, outside the project these hooks are bound to ({}). Installed hook wiring names the checkout it was installed in; install the accelerator in this checkout to give it its own memory.\nWorking memory was NOT consulted this turn. Read the canonical sources directly.\n",
        session.display(),
        project.display()
    ))
}
fn payload(raw: &[u8]) -> Value {
    serde_json::from_slice::<Value>(raw)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}
impl Context {
    pub fn new(root: &Path, host: &str, raw: &[u8]) -> Result<Self> {
        Self::create(root, host, raw, true)
    }
    /// Without `with_task` the session's task is not inferred from the branch,
    /// which costs Git queries the guards do not need.
    fn create(root: &Path, host: &str, raw: &[u8], with_task: bool) -> Result<Self> {
        Self::create_with_payload(root, host, raw, with_task, None)
    }
    fn create_with_payload(
        root: &Path,
        host: &str,
        raw: &[u8],
        with_task: bool,
        parsed: Option<Value>,
    ) -> Result<Self> {
        if !["claude", "codex", "cursor"].contains(&host) {
            return Err(RuntimeError::invalid("Unsupported hook host"));
        }
        std::fs::metadata(root)?;
        let root = infra_core::fs::python_realpath(root)?;
        let state = std::env::var_os("ACCELERATOR_STATE_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| root.clone());
        let mut roots = Roots::discover(&state, &root)?;
        let payload = parsed.unwrap_or_else(|| payload(raw));
        let mut entered = false;
        let mut other_worktree = false;
        // Git metadata of the session directory that cannot be read only means
        // that directory is no linked worktree of this project; the hooks of
        // this project still run.
        if let Some(start) = valid_path(payload["cwd"].as_str(), 4096).filter(|path| path.is_dir())
            && let (Some(theirs), Some(mine)) = (
                GitFrame::discover(&start).ok().flatten(),
                GitFrame::discover(&roots.project)?,
            )
            && theirs.top != mine.top
            && theirs.common_dir()? == mine.common_dir()?
        {
            other_worktree = true;
            if host != "cursor" && (roots.state != roots.tooling || roots.project == roots.tooling)
            {
                let mine_top =
                    infra_core::fs::python_realpath_spelling(&roots.project, mine.top.clone())?;
                let suffix = roots
                    .project
                    .strip_prefix(&mine_top)
                    .map_err(|_| RuntimeError::policy("Served project escapes checkout"))?;
                let theirs_top =
                    infra_core::fs::python_realpath_spelling(&start, theirs.top.clone())?;
                let served = theirs_top.join(suffix);
                let entered_start = infra_core::fs::python_realpath(&start)?;
                if served.is_dir()
                    && entered_start.starts_with(&served)
                    && !installed_engine(&served).is_file()
                {
                    roots.project = served;
                    roots.shared = true;
                    entered = true;
                    other_worktree = false;
                }
            }
        }
        let outside = if other_worktree || entered {
            None
        } else {
            outside_session(host, &payload, &roots)
        };
        let task = if with_task {
            task(&roots.project)?
        } else {
            None
        };
        let inferred = std::env::var("CONTEXT_TASK_ID")
            .ok()
            .is_none_or(|value| value.is_empty());
        let seconds = std::env::var("CONTEXT_HOOK_BUDGET")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(5.0);
        Ok(Self {
            roots,
            host: host.into(),
            payload,
            raw: raw.to_vec(),
            task,
            inferred,
            entered,
            other_worktree,
            outside,
            budget: Duration::from_secs_f64(seconds),
        })
    }
    pub fn environment(&self) -> BTreeMap<OsString, OsString> {
        let mut env = BTreeMap::new();
        if self.roots.shared || self.roots.attached() {
            env.insert(
                "ACCELERATOR_HOME".into(),
                self.roots.tooling.as_os_str().into(),
            );
            env.insert(
                "ACCELERATOR_STATE_DIR".into(),
                self.roots.state.as_os_str().into(),
            );
            env.insert(
                "ACCELERATOR_PROJECT_DIR".into(),
                self.roots.project.as_os_str().into(),
            );
        }
        env
    }
    pub fn arguments(&self, operation: &str, extra: &[String]) -> Vec<String> {
        let mut args = vec![
            "memory".into(),
            "--root".into(),
            self.roots.state.to_string_lossy().into_owned(),
        ];
        if self.roots.shared {
            args.extend([
                "--project".into(),
                self.roots.project.to_string_lossy().into_owned(),
            ]);
        }
        args.push(operation.into());
        args.extend_from_slice(extra);
        args
    }
    pub fn call(
        &self,
        operation: &str,
        extra: &[String],
        input: &[u8],
        timeout: Duration,
    ) -> Result<Output> {
        process::engine(
            &self.arguments(operation, extra),
            &self.roots.project,
            input,
            timeout,
            &self.environment(),
        )
    }
    pub fn json(&self, operation: &str, extra: &[String], input: &[u8]) -> Result<Value> {
        let mut extra = extra.to_vec();
        extra.push("--json".into());
        let output = self.call(operation, &extra, input, self.budget)?;
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|_| RuntimeError::unavailable("Native runtime returned invalid JSON"))?;
        if output.code != 0 && !(operation == "validate" && value["valid"] == false) {
            return Err(RuntimeError::unavailable(format!(
                "Native runtime operation exited {}",
                output.code
            )));
        }
        Ok(value)
    }
    pub fn branch_args(&self) -> Vec<String> {
        if self.inferred {
            self.task
                .as_ref()
                .map(|task| vec!["--expected-branch".into(), task.clone()])
                .unwrap_or_default()
        } else {
            vec![]
        }
    }
    pub fn append(&self, name: &str, row: &Value) {
        let operation = (|| -> Result<()> {
            let directory = self.roots.state.join("memory-bank/local");
            if self.roots.attached() {
                for path in [
                    &self.roots.state,
                    &self.roots.state.join("memory-bank"),
                    &directory,
                ] {
                    if let Ok(meta) = std::fs::symlink_metadata(path)
                        && infra_core::fs::linked(&meta)
                    {
                        return Err(RuntimeError::policy("Linked hook journal directory"));
                    }
                }
            }
            infra_core::fs::ensure_directory(&directory, self.roots.attached())?;
            let path = directory.join(name);
            if let Ok(meta) = std::fs::symlink_metadata(&path)
                && (infra_core::fs::linked(&meta) || !meta.is_file())
            {
                return Err(RuntimeError::policy("Linked hook journal"));
            }
            let mut options = std::fs::OpenOptions::new();
            options.write(true).append(true).create(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            }
            let mut output = options.open(path)?;
            if !output.metadata()?.is_file() {
                return Err(RuntimeError::policy("Invalid hook journal"));
            }
            let mut data = serde_json::to_vec(row)?;
            data.push(b'\n');
            output.write_all(&data)?;
            Ok(())
        })();
        let _ = operation;
    }
}
pub fn installed_engine(root: &Path) -> PathBuf {
    root.join(if cfg!(windows) {
        "memory-bank/bin/infra-engine.exe"
    } else {
        "memory-bank/bin/infra-engine"
    })
}
pub fn shim(host: &str, role: &str, windows: bool) -> Result<String> {
    if !["claude", "codex", "cursor"].contains(&host) || !ROLES.contains(&role) {
        return Err(RuntimeError::invalid("Unknown hook shim role"));
    }
    let binary = if windows {
        "infra-engine.exe"
    } else {
        "infra-engine"
    };
    // The engine is ignored by Git, so a fresh clone has none until it is
    // installed there: the shell guard then refuses rather than letting every
    // command through, and the other roles pass the turn. Hosts differ only in
    // the literal host argument, which the tool mirrors substitute. The
    // directory comes from `$0` without `dirname`, one process fewer on every
    // hook call.
    let install = "From an accelerator clone run: infra-engine install --edition <edition> --target <this checkout> --merge-existing";
    let missing = match role {
        "bash-validator" => format!(
            "  echo \"BLOCKED: no accelerator engine is installed for $TOOLING_ROOT. {install}\" >&2\n  exit 2\n"
        ),
        // A continuation every host accepts on its prompt event.
        "working-memory-read" => "  echo '{\"continue\": true}'\n  exit 0\n".into(),
        "local-context" => format!(
            "  echo \"No accelerator engine is installed for $TOOLING_ROOT: its hooks and memory are off. {install}\"\n  exit 0\n"
        ),
        _ => "  exit 0\n".into(),
    };
    Ok(format!(
        "#!/bin/sh\n# Launcher; the complete hook implementation is Rust. It runs the engine of\n# this checkout or, in a linked worktree, the one of its main checkout.\ncase $0 in */*) TOOLING_ROOT=${{0%/*}}/../.. ;; *) TOOLING_ROOT=../.. ;; esac\nTOOLING_ROOT=$(CDPATH= cd -- \"$TOOLING_ROOT\" && pwd -P) || exit 127\nENGINE=$TOOLING_ROOT/memory-bank/bin/{binary}\n{fallback}if [ ! -x \"$ENGINE\" ]; then\n{missing}fi\nexec \"$ENGINE\" hook {host} {role} --root \"$TOOLING_ROOT\" \"$@\"\n",
        fallback = infra_tooling::launch::worktree_fallback("TOOLING_ROOT", "ENGINE", binary)
    ))
}
/// Local desktop notification transport; no shell or hook-input interpolation.
pub fn notification() -> Result<Output> {
    let fallback = || Output::text("Claude Code notification\n");
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(_) => return Ok(fallback()),
    };
    let (program, args) = match std::env::consts::OS {
        "macos" => (
            Path::new("/usr/bin/osascript"),
            vec![
                "-e".into(),
                "display notification \"Claude needs your attention\" with title \"Claude Code\""
                    .into(),
            ],
        ),
        "linux" => (
            Path::new("notify-send"),
            vec!["Claude Code".into(), "Claude needs your attention".into()],
        ),
        _ => return Ok(fallback()),
    };
    match process::run(
        program,
        &args,
        &cwd,
        &[],
        Duration::from_secs(3),
        &BTreeMap::new(),
    ) {
        Ok(output) if output.code == 0 => Ok(Output::default()),
        _ => Ok(fallback()),
    }
}
pub fn execute(root: &Path, host: &str, role: &str, raw: &[u8]) -> Result<Output> {
    if role == "notification" {
        return if host == "claude" {
            notification()
        } else {
            Err(RuntimeError::invalid("Notification is a Claude hook role"))
        };
    }
    if !ROLES.contains(&role) {
        return Err(RuntimeError::invalid("Unknown hook role"));
    }
    let pass = || passed(host, role);
    if std::env::var("CONTEXT_MEMORY_RECOVERY").as_deref() == Ok("1")
        && MEMORY_ROLES.contains(&role)
    {
        return Ok(pass());
    }
    let parsed = if matches!(
        role,
        "bash-validator" | "file-naming-validator" | "loop-detection"
    ) {
        let payload = payload(raw);
        if guards::payload_noop(role, &payload) {
            return Ok(pass());
        }
        Some(payload)
    } else {
        None
    };
    let context =
        match Context::create_with_payload(root, host, raw, TASK_ROLES.contains(&role), parsed) {
            Ok(context) => context,
            // The shell guard never lets a command through because its own
            // context failed: it then checks the command from the bound root.
            Err(_) if role == "bash-validator" => return guards::bash_without_context(root, raw),
            Err(_) => return Ok(pass()),
        };
    if host == "claude"
        && context.payload.get("cursor_version").is_some()
        && context.roots.tooling.join(".cursor/hooks.json").is_file()
        && [
            "working-memory-read",
            "working-memory-write",
            "context-continuity",
        ]
        .contains(&role)
    {
        return Ok(pass());
    }
    if let Some(session) = &context.outside
        && MEMORY_ROLES.contains(&role)
    {
        return Ok(
            if host != "cursor" && ["working-memory-read", "local-context"].contains(&role) {
                outside_notice(session, &context.roots.project)
            } else {
                pass()
            },
        );
    }
    let result = match role {
        "bash-validator" => {
            guards::bash(&context).or_else(|_| guards::bash_without_context(root, raw))
        }
        "file-naming-validator" => guards::file_naming(&context),
        "loop-detection" => guards::edit_loop(&context),
        "subagent-gate" => guards::subagent_gate(&context),
        "subagent-dispatch" => guards::subagent_dispatch(&context),
        "working-memory-read" => memory::read(&context),
        "working-memory-write" => memory::write(&context),
        "local-context" => banner::render(&context),
        "context-continuity" => continuity(&context),
        _ => Err(RuntimeError::invalid("Unknown hook role")),
    };
    if MEMORY_ROLES.contains(&role) {
        return Ok(result.unwrap_or_else(|_| pass()));
    }
    result
}
fn continuity(context: &Context) -> Result<Output> {
    if context.other_worktree {
        return Ok(Output::default());
    }
    let value = infra_core::chat_continuity::execute(
        &infra_core::RuntimeContext::new(
            context.roots.clone(),
            std::env::var("PROJECT_BRAIN_OWNER").unwrap_or_else(|_| "local".into()),
            infra_core::RuntimeConfig::load(&context.roots)?.mode(None)?,
        )?,
        &context.host,
        "hook",
        &context.raw,
        &json!({}),
    )?;
    Output::envelope(value)
}
