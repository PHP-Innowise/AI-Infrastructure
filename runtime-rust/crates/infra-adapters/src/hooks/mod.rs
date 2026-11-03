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
    if let Some(branch) = frame.branch()?.filter(|branch| !branch.is_empty()) {
        return Ok(Some(branch));
    }
    for name in ["rebase-merge/head-name", "rebase-apply/head-name"] {
        if let Ok(value) = std::fs::read_to_string(frame.admin.join(name))
            && let Some(branch) = value
                .lines()
                .next()
                .and_then(|value| value.strip_prefix("refs/heads/"))
                .filter(|value| !value.is_empty())
        {
            return Ok(Some(branch.into()));
        }
    }
    if let Ok(value) = std::fs::read_to_string(frame.admin.join("BISECT_START"))
        && let Some(branch) = value.lines().next().filter(|value| {
            !value.is_empty()
                && !(matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit()))
        })
    {
        return Ok(Some(branch.into()));
    }
    let path = frame.admin.join("codex-thread.json");
    if let Ok(meta) = std::fs::symlink_metadata(&path)
        && !infra_core::fs::linked(&meta)
        && meta.is_file()
        && meta.len() <= 4096
    {
        let value: Value =
            serde_json::from_slice(&infra_core::fs::read_regular(&path)?).unwrap_or(Value::Null);
        if value["version"] == 1
            && let Some(id) = value["ownerThreadId"].as_str()
            && regex::Regex::new(r"^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$")
                .is_ok_and(|p| p.is_match(id))
        {
            return Ok(Some(format!("codex-thread/{id}")));
        }
    }
    Ok(None)
}
impl Context {
    pub fn new(root: &Path, host: &str, raw: &[u8]) -> Result<Self> {
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
        let payload = serde_json::from_slice::<Value>(raw)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        let mut entered = false;
        let mut other_worktree = false;
        if let Some(start) = valid_path(payload["cwd"].as_str(), 4096).filter(|path| path.is_dir())
            && let (Some(theirs), Some(mine)) = (
                GitFrame::discover(&start)?,
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
        let task = task(&roots.project)?;
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
    Ok(format!(
        "#!/bin/sh\n# Compatibility entrypoint; the complete hook implementation is Rust.\nTOOLING_ROOT=$(CDPATH= cd -- \"$(dirname -- \"$0\")/../..\" && pwd -P) || exit 127\nexec \"$TOOLING_ROOT/memory-bank/bin/{binary}\" hook {host} {role} --root \"$TOOLING_ROOT\" \"$@\"\n"
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
    let pass = || {
        if host == "cursor" && role == "working-memory-read" {
            Output::text("{\"continue\": true}\n")
        } else {
            Output::default()
        }
    };
    if std::env::var("CONTEXT_MEMORY_RECOVERY").as_deref() == Ok("1")
        && [
            "working-memory-read",
            "working-memory-write",
            "context-continuity",
            "local-context",
        ]
        .contains(&role)
    {
        return Ok(pass());
    }
    let context = match Context::new(root, host, raw) {
        Ok(context) => context,
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
    match role {
        "bash-validator" => guards::bash(&context),
        "file-naming-validator" => guards::file_naming(&context),
        "loop-detection" => guards::edit_loop(&context),
        "subagent-gate" => guards::subagent_gate(&context),
        "subagent-dispatch" => guards::subagent_dispatch(&context),
        "working-memory-read" => memory::read(&context),
        "working-memory-write" => memory::write(&context),
        "local-context" => banner::render(&context),
        "context-continuity" => {
            if context.other_worktree {
                return Ok(pass());
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
        _ => Err(RuntimeError::invalid("Unknown hook role")),
    }
}
