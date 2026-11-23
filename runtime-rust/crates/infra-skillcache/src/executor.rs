//! `skillcache-run`: the state machine that follows a capsule's plan.
//!
//! Each command is short-lived. It runs consecutive programs without going
//! back to the agent and returns as soon as the agent is needed:
//! `needs_bindings` (a program needs a value that is not bound),
//! `your_steps` (the next steps are the agent's own), `program_failed` (this
//! one step is the agent's now) or `complete`. The state file is written
//! before every program starts, so `continue` after a crash knows which
//! attempt it interrupted. A failed program is never run again; the rest of
//! the plan stays compiled.
use crate::{
    capsule::{BINDING_BYTES, Examples, SkillCapsule, Step, is_admitted_value},
    gate1,
    package::Package,
    state::{Awaiting, Failure, RunLog, RunState},
    tool,
};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// A refusal of a command: the agent or the host called the executor in a way
/// the protocol does not allow. The run's state is left as it was.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}
impl Refusal {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn to_value(&self) -> Value {
        json!({"status": "error", "code": self.code, "message": self.message})
    }
}
impl From<infra_core::RuntimeError> for Refusal {
    fn from(error: infra_core::RuntimeError) -> Self {
        Self::new("unavailable", error.message)
    }
}
type Reply = Result<Value, Refusal>;

pub enum Command {
    /// Starts the run with the values known so far.
    Run(Option<Value>),
    /// Supplies what the last answer asked for and goes on.
    Continue(Option<Value>),
    /// Reports where the run stands. Changes nothing.
    Status,
    /// Clears an in-progress run state so the agent can restart with corrected bindings.
    Reset,
}

/// Where one run lives.
pub struct Session<'a> {
    pub package: &'a Package,
    pub workspace: &'a Path,
    pub state_file: &'a Path,
    pub generation: &'a str,
}

struct Runner<'a> {
    session: &'a Session<'a>,
    capsule: &'a SkillCapsule,
    state: RunState,
    log: RunLog,
    /// Results of the programs this command ran, returned with its answer.
    results: Map<String, Value>,
    /// Read-only programs must leave the workspace unchanged on both live runs and replays.
    check_read_only: bool,
    /// Live runs only: record workspace file changes into the sidecar change log.
    record_changes: bool,
    /// Replay only: the diagnostics of every failure, by plan index.
    failures: Vec<Value>,
}

fn is_instruction(step: &Step) -> bool {
    !matches!(step, Step::Program { .. })
}

fn produces(step: &Step) -> &[String] {
    match step {
        Step::Instruction { produces, .. } => produces,
        _ => &[],
    }
}

/// Workspace directory/file names always ignored regardless of allowlist overrides.
pub const ALWAYS_IGNORED_WORKSPACE_NAMES: [&str; 6] = [
    ".git",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".DS_Store",
];

/// Large toolchain/vendor directory names ignored by default during workspace
/// hashing and diffing unless explicitly allowlisted.
pub const DEFAULT_IGNORED_WORKSPACE_NAMES: [&str; 6] =
    ["node_modules", "vendor", ".venv", "dist", "build", ".next"];

/// Workspace directory/file names ignored across workspace comparisons and
/// observed-result captures.
pub const IGNORED_WORKSPACE_NAMES: [&str; 12] = [
    ".git",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".DS_Store",
    "node_modules",
    "vendor",
    ".venv",
    "dist",
    "build",
    ".next",
];

/// Whether `name` may appear in an allowlist overriding default workspace ignores.
/// Rejects path traversal, separators, empty names, and always-ignored metadata.
pub fn is_safe_allowlist_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        && !ALWAYS_IGNORED_WORKSPACE_NAMES.contains(&name)
}

/// Whether `name` is an ignored workspace metadata or cache entry at any depth.
pub fn is_ignored_workspace_name(name: &str) -> bool {
    is_ignored_workspace_name_with(name, &[])
}

/// Whether `name` is ignored when `allowlist` lists explicitly tracked names.
pub fn is_ignored_workspace_name_with(name: &str, allowlist: &[String]) -> bool {
    if ALWAYS_IGNORED_WORKSPACE_NAMES.contains(&name) {
        return true;
    }
    DEFAULT_IGNORED_WORKSPACE_NAMES.contains(&name)
        && !allowlist.iter().any(|allowed| allowed == name)
}

pub type WorkspaceEntry = (PathBuf, u64, Option<std::time::SystemTime>, Option<String>);

/// Bounded workspace snapshot for tracking regular file creations/edits/deletions
/// during a warm program step without walking huge monorepos.
pub fn workspace_snapshot_for_changes(root: &Path) -> Option<Vec<WorkspaceEntry>> {
    workspace_snapshot_for_changes_with(root, &[])
}

/// Like [`workspace_snapshot_for_changes`], honoring `allowlist` overrides for
/// default-ignored dependency directories.
pub fn workspace_snapshot_for_changes_with(
    root: &Path,
    allowlist: &[String],
) -> Option<Vec<WorkspaceEntry>> {
    workspace_listing_limited(root, 10_000, allowlist)
}

/// Computes bounded `{"path", "kind"}` entries between `before` and `after` snapshots.
pub fn diff_workspace_changes(
    before: &[WorkspaceEntry],
    after: &[WorkspaceEntry],
    root: &Path,
) -> Vec<Value> {
    const LIMIT: usize = 64;
    let map = |entries: &[WorkspaceEntry]| -> std::collections::BTreeMap<
        String,
        (u64, Option<std::time::SystemTime>, Option<String>),
    > {
        let mut out = std::collections::BTreeMap::new();
        for (path, len, mtime, digest) in entries {
            if let Ok(rel) = path.strip_prefix(root)
                && let Some(rel) = rel.to_str()
            {
                let normalized = if cfg!(windows) {
                    rel.replace('\\', "/")
                } else {
                    rel.to_owned()
                };
                if !normalized.is_empty() {
                    out.insert(normalized, (*len, *mtime, digest.clone()));
                }
            }
        }
        out
    };
    let mut prev = map(before);
    let next = map(after);
    let mut changes = Vec::new();
    for (path, sig) in next {
        match prev.remove(&path) {
            None => changes.push(json!({"path": path, "kind": "create"})),
            Some(old) if old != sig => changes.push(json!({"path": path, "kind": "edit"})),
            Some(_) => {}
        }
        if changes.len() >= LIMIT {
            return changes;
        }
    }
    for path in prev.into_keys() {
        changes.push(json!({"path": path, "kind": "delete"}));
        if changes.len() >= LIMIT {
            break;
        }
    }
    changes
}

/// `(path, size, modified, digest)` of everything below `root`, or `None` when
/// the tree exceeds `max_entries`.
fn workspace_listing(root: &Path, allowlist: &[String]) -> Option<Vec<WorkspaceEntry>> {
    workspace_listing_limited(root, 200_000, allowlist)
}

fn workspace_listing_limited(
    root: &Path,
    max_entries: usize,
    allowlist: &[String],
) -> Option<Vec<WorkspaceEntry>> {
    const HASH_LIMIT_BYTES: u64 = 4 * 1024 * 1024;
    let mut entries = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let mut names = std::fs::read_dir(&folder)
            .ok()?
            .collect::<std::io::Result<Vec<_>>>()
            .ok()?;
        names.sort_by_key(|entry| entry.file_name());
        for entry in names {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_str();
            if name_str.is_some_and(|n| is_ignored_workspace_name_with(n, allowlist))
                || (folder == root
                    && name_str.is_some_and(|n| n == ".harness-tmp" || n == "target"))
            {
                continue;
            }
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            if metadata.is_dir() && !infra_core::fs::linked(&metadata) {
                folders.push(path.clone());
            } else if metadata.is_file() {
                let digest =
                    if !infra_core::fs::linked(&metadata) && metadata.len() <= HASH_LIMIT_BYTES {
                        std::fs::read(&path)
                            .ok()
                            .map(|bytes| crate::fingerprint::sha256(&bytes))
                    } else {
                        None
                    };
                entries.push((path, metadata.len(), metadata.modified().ok(), digest));
            }
            if entries.len() > max_entries {
                return None;
            }
        }
    }
    entries.sort();
    Some(entries)
}

impl<'a> Runner<'a> {
    fn open(session: &'a Session<'a>, state: RunState) -> Result<Self, Refusal> {
        Ok(Self {
            session,
            capsule: &session.package.capsule,
            log: RunLog::open(session.state_file, &state.generation)?,
            state,
            results: Map::new(),
            check_read_only: true,
            record_changes: true,
            failures: Vec::new(),
        })
    }
    fn save(&self) -> Result<(), Refusal> {
        Ok(self.state.save(self.session.state_file)?)
    }
    fn reply(&self, mut answer: Map<String, Value>) -> Value {
        if !self.results.is_empty() && !answer.contains_key("result") {
            answer.insert("results".into(), Value::Object(self.results.clone()));
        }
        Value::Object(answer)
    }
    /// Checks and binds supplied values. Nothing is bound unless all of them
    /// are admitted.
    fn bind(&mut self, supplied: Option<&Value>, expected: &[String]) -> Result<(), Refusal> {
        let Some(supplied) = supplied else {
            return Ok(());
        };
        let supplied = supplied.as_object().ok_or_else(|| {
            Refusal::new(
                "invalid_values",
                "Supply values as one JSON object: `{\"name\": value, ...}`.",
            )
        })?;
        for (name, value) in supplied {
            let declared = self.capsule.binding(name);
            if declared.is_none() && !expected.contains(name) {
                return Err(Refusal::new(
                    "unexpected_name",
                    format!("`{name}` is not a value this step of the plan takes."),
                ));
            }
            if self
                .state
                .bindings
                .get(name)
                .is_some_and(|bound| bound != value)
            {
                return Err(Refusal::new(
                    "already_bound",
                    format!("`{name}` is already bound; a bound value does not change."),
                ));
            }
            if !is_admitted_value(value) {
                return Err(Refusal::new(
                    "invalid_values",
                    format!("`{name}` must be a string, a number, a boolean or a list of those."),
                ));
            }
            if serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len()) > BINDING_BYTES {
                return Err(Refusal::new(
                    "value_too_large",
                    format!(
                        "`{name}` exceeds {BINDING_BYTES} bytes. Bind a path to a file, not its content."
                    ),
                ));
            }
            if let Some(binding) = declared
                && !binding.kind.admits(value)
            {
                return Err(Refusal::new(
                    "type_mismatch",
                    format!("`{name}` must be a {}.", binding.kind.as_str()),
                ));
            }
        }
        for (name, value) in supplied {
            self.state.bindings.insert(name.clone(), value.clone());
        }
        Ok(())
    }
    fn instruction_steps(&self, from: usize) -> (Vec<usize>, Vec<String>) {
        let mut indexes = Vec::new();
        let mut names = Vec::new();
        for (index, step) in self.capsule.plan().enumerate().skip(from) {
            if !is_instruction(step) {
                break;
            }
            indexes.push(index);
            names.extend(produces(step).iter().cloned());
        }
        (indexes, names)
    }
    fn your_steps(&mut self, indexes: Vec<usize>, names: Vec<String>, missing: &[String]) -> Reply {
        let steps = indexes
            .iter()
            .filter_map(|index| Some((*index, self.capsule.plan_step(*index)?)))
            .map(|(index, step)| {
                let (text, source) = match step {
                    Step::Instruction { text, source, .. } | Step::Inspection { text, source } => {
                        (text.as_str(), source.as_ref())
                    }
                    Step::Program { .. } => ("", None),
                };
                let mut entry = Map::new();
                entry.insert("step_index".into(), index.into());
                entry.insert("kind".into(), step.kind().into());
                entry.insert("instruction".into(), text.into());
                if let Some(source) = source {
                    entry.insert(
                        "source".into(),
                        json!({"path": source.path, "section": source.section}),
                    );
                }
                if !produces(step).is_empty() {
                    entry.insert("produces".into(), json!(produces(step)));
                }
                Value::Object(entry)
            })
            .collect::<Vec<_>>();
        let mut answer = Map::new();
        answer.insert("status".into(), "your_steps".into());
        answer.insert("steps".into(), steps.into());
        answer.insert("produces".into(), json!(names));
        if !missing.is_empty() {
            answer.insert("missing".into(), json!(missing));
        }
        answer.insert(
            "next".into(),
            if names.is_empty() {
                "Do these steps yourself, then run `skillcache-run continue`.".into()
            } else {
                format!(
                    "Do these steps yourself, then run `skillcache-run continue '{{...}}'` with: {}.",
                    names.join(", ")
                )
                .into()
            },
        );
        Ok(self.reply(answer))
    }
    fn failed_answer(&self, step_index: usize, reason: &str, diagnostics: Value) -> Value {
        let program = self
            .capsule
            .plan_step(step_index)
            .and_then(Step::program)
            .and_then(|id| self.capsule.program(id));
        let required = self.capsule.outputs_read_later(step_index);
        let skills = self
            .capsule
            .skills
            .iter()
            .map(|skill| skill.path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let what = program
            .and_then(|program| program.description.as_deref())
            .map(|description| format!(" It was to: {description}."))
            .unwrap_or_default();
        let then = if required.is_empty() {
            "then run `skillcache-run continue`".to_string()
        } else {
            format!(
                "then run `skillcache-run continue '{{...}}'` with: {}",
                required.join(", ")
            )
        };
        let mut answer = Map::new();
        answer.insert("step_index".into(), step_index.into());
        answer.insert("status".into(), "program_failed".into());
        if let Some(program) = program {
            answer.insert("program".into(), program.id.clone().into());
        }
        answer.insert(
            "instruction".into(),
            format!(
                "Do this one step yourself with your ordinary tools, following the source skill ({skills}).{what} Do not repair or re-run the program; the rest of the plan stays compiled. When the step is done, {then}."
            )
            .into(),
        );
        let mut diagnostic = Map::new();
        diagnostic.insert("reason".into(), reason.into());
        if let Value::Object(more) = diagnostics {
            diagnostic.extend(more);
        }
        answer.insert("diagnostics".into(), diagnostic.into());
        // The agent may read what a plan step was meant to do. A verification
        // program's source is not offered: its check must stay independent.
        if !self.capsule.is_verification(step_index)
            && let Some(file) = program.and_then(|program| {
                program.entry.iter().skip(1).find_map(|argument| {
                    let candidate = argument
                        .split_once('=')
                        .map_or(argument.as_str(), |(_, value)| value);
                    candidate.starts_with("programs/").then_some(candidate)
                })
            })
        {
            answer.insert(
                "program_source_path".into(),
                self.session
                    .package
                    .root
                    .join(file)
                    .display()
                    .to_string()
                    .into(),
            );
        }
        answer.insert("required_outputs".into(), json!(required));
        self.reply(answer)
    }
    fn fail(&mut self, step_index: usize, failed: &tool::Failed, attempts: u8) -> Reply {
        let diagnostics = json!({
            "exit_code": failed.exit_code,
            "attempts": attempts,
            "detail": failed.diagnostics,
            "note": "A partial run may have changed files in the workspace; nothing was rolled back.",
        });
        self.state.failed.push(Failure {
            step_index,
            reason: failed.reason.into(),
        });
        self.state.awaiting = Awaiting::Outputs {
            step_index,
            names: self.capsule.outputs_read_later(step_index),
        };
        self.save()?;
        self.log.append(
            "program_failed",
            json!({
                "step_index": step_index,
                "program": self.capsule.plan_step(step_index).and_then(Step::program),
                "reason": failed.reason,
                "verification": self.capsule.is_verification(step_index),
            }),
        )?;
        let answer = self.failed_answer(step_index, failed.reason, diagnostics);
        self.failures.push(answer.clone());
        Ok(answer)
    }
    /// Runs the program at `step_index`, starting with `attempt`.
    fn run_program(
        &mut self,
        step_index: usize,
        mut attempt: u8,
    ) -> Result<Option<Value>, Refusal> {
        let capsule = self.capsule;
        let program = capsule
            .plan_step(step_index)
            .and_then(Step::program)
            .and_then(|id| capsule.program(id))
            .ok_or_else(|| Refusal::new("invalid_capsule", "The plan names an unknown program."))?;
        loop {
            // Written and flushed before the program starts: a crash leaves
            // the attempt on disk.
            self.state.awaiting = Awaiting::Running {
                step_index,
                attempt,
            };
            self.save()?;
            self.log.append(
                "program_started",
                json!({"step_index": step_index, "program": program.id, "attempt": attempt}),
            )?;
            let allowlist = &self.session.package.capsule.capabilities.allow_ignored;
            let before = if self.check_read_only && program.read_only {
                workspace_listing(self.session.workspace, allowlist)
            } else if self.record_changes {
                workspace_snapshot_for_changes_with(self.session.workspace, allowlist)
            } else {
                None
            };
            let outcome = tool::run(
                self.session.package,
                program,
                tool::Mode::Run(&self.state.bindings),
                self.session.workspace,
            )?;
            let after = if before.is_some() {
                if self.check_read_only && program.read_only {
                    workspace_listing(self.session.workspace, allowlist)
                } else {
                    workspace_snapshot_for_changes_with(self.session.workspace, allowlist)
                }
            } else {
                None
            };
            let changed_files = match (&before, &after) {
                (Some(prev), Some(next)) if self.record_changes => {
                    diff_workspace_changes(prev, next, self.session.workspace)
                }
                _ => Vec::new(),
            };
            if !changed_files.is_empty() {
                let _ = crate::state::append_changes(
                    self.log.path(),
                    self.log.generation(),
                    &changed_files,
                );
            }
            let outcome = match outcome {
                tool::Outcome::Completed(completed)
                    if self.check_read_only
                        && program.read_only
                        && before.is_some()
                        && after != before =>
                {
                    tool::Outcome::Failed(tool::Failed {
                        reason: "read_only_violation",
                        exit_code: Some(0),
                        diagnostics: "The program is declared read-only but changed the workspace."
                            .into(),
                        missing: Vec::new(),
                        seconds: completed.seconds,
                    })
                }
                other => other,
            };
            match outcome {
                tool::Outcome::Completed(completed) => {
                    for output in &program.outputs {
                        if let Some(value) = completed.result.get(output) {
                            self.state.bindings.insert(output.clone(), value.clone());
                        }
                    }
                    let result = Value::Object(completed.result);
                    self.state
                        .results
                        .insert(program.id.clone(), result.clone());
                    self.results.insert(program.id.clone(), result);
                    self.state.completed.push(step_index);
                    self.state.position = step_index + 1;
                    self.state.awaiting = Awaiting::None;
                    self.save()?;
                    self.log.append(
                        "program_completed",
                        json!({
                            "step_index": step_index,
                            "program": program.id,
                            "attempt": attempt,
                            "seconds": completed.seconds,
                        }),
                    )?;
                    return Ok(None);
                }
                tool::Outcome::Failed(failed) if failed.retryable() && attempt == 1 => {
                    attempt = 2;
                }
                tool::Outcome::Failed(failed) => {
                    return self.fail(step_index, &failed, attempt).map(Some);
                }
            }
        }
    }
    /// Follows the plan until the agent is needed or it ends.
    fn advance(&mut self, mut resume: Option<u8>) -> Reply {
        loop {
            let index = self.state.position;
            let Some(step) = self.capsule.plan_step(index) else {
                self.state.awaiting = Awaiting::None;
                self.save()?;
                self.log.append(
                    "complete",
                    json!({"failed": self.state.failed.len(), "completed": self.state.completed.len()}),
                )?;
                return Ok(self.complete());
            };
            if is_instruction(step) {
                let (indexes, names) = self.instruction_steps(index);
                self.state.awaiting = Awaiting::Steps {
                    step_indexes: indexes.clone(),
                    produces: names.clone(),
                };
                self.save()?;
                self.log
                    .append("your_steps", json!({"step_indexes": indexes}))?;
                return self.your_steps(indexes, names, &[]);
            }
            if self.state.is_failed(index) {
                self.state.position = index + 1;
                continue;
            }
            let program = step
                .program()
                .and_then(|id| self.capsule.program(id))
                .ok_or_else(|| {
                    Refusal::new("invalid_capsule", "The plan names an unknown program.")
                })?;
            let missing = program
                .inputs
                .iter()
                .filter(|input| !self.state.bindings.contains_key(*input))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                self.state.awaiting = Awaiting::Bindings {
                    step_index: index,
                    names: missing.clone(),
                };
                self.save()?;
                self.log.append(
                    "needs_bindings",
                    json!({"step_index": index, "names": missing}),
                )?;
                return Ok(self.needs_bindings(index, &missing));
            }
            if let Some(answer) = self.run_program(index, resume.take().unwrap_or(1))? {
                return Ok(answer);
            }
        }
    }
    fn needs_bindings(&self, step_index: usize, names: &[String]) -> Value {
        let described = names
            .iter()
            .map(|name| match self.capsule.binding(name) {
                Some(binding) => json!({
                    "name": name,
                    "type": binding.kind.as_str(),
                    "description": binding.description,
                }),
                None => json!({"name": name}),
            })
            .collect::<Vec<_>>();
        let mut answer = Map::new();
        answer.insert("status".into(), "needs_bindings".into());
        answer.insert("step_index".into(), step_index.into());
        answer.insert("names".into(), json!(names));
        answer.insert("bindings".into(), described.into());
        answer.insert(
            "next".into(),
            format!(
                "Run `skillcache-run continue '{{...}}'` with: {}.",
                names.join(", ")
            )
            .into(),
        );
        self.reply(answer)
    }
    fn complete(&self) -> Value {
        json!({
            "status": "complete",
            "result": self.bounded_complete_results(),
            "failed": self.state.failed.iter().map(|failure| json!({
                "step_index": failure.step_index, "reason": failure.reason,
            })).collect::<Vec<_>>(),
        })
    }

    /// Trims large non-output/raw fields from `complete["result"]` while preserving
    /// declared program outputs up to bounded limits so `complete` stays small.
    fn bounded_complete_results(&self) -> Map<String, Value> {
        const DECLARED_STR_LIMIT: usize = 4096;
        const EXTRA_STR_LIMIT: usize = 512;
        const TOTAL_BYTES_LIMIT: usize = 16 * 1024;
        fn trim_str(text: &str, max_bytes: usize) -> Value {
            if text.len() <= max_bytes {
                return Value::String(text.to_owned());
            }
            let mut end = max_bytes;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            Value::String(format!(
                "{}... [truncated {} bytes]",
                &text[..end],
                text.len() - end
            ))
        }
        fn trim_value(value: &Value, str_limit: usize, array_limit: usize) -> Value {
            match value {
                Value::String(text) => trim_str(text, str_limit),
                Value::Array(items) => {
                    let mut out: Vec<Value> = items
                        .iter()
                        .take(array_limit)
                        .map(|item| trim_value(item, str_limit.min(512), 16))
                        .collect();
                    if items.len() > array_limit {
                        out.push(Value::String(format!(
                            "... [+{} more items]",
                            items.len() - array_limit
                        )));
                    }
                    Value::Array(out)
                }
                Value::Object(map) => {
                    let mut out = Map::new();
                    for (k, v) in map {
                        out.insert(k.clone(), trim_value(v, str_limit.min(512), 16));
                    }
                    Value::Object(out)
                }
                other => other.clone(),
            }
        }
        let mut total = 0usize;
        let mut out = Map::new();
        for (program_id, step_value) in &self.state.results {
            let declared: &[String] = self
                .capsule
                .program(program_id)
                .map(|p| p.outputs.as_slice())
                .unwrap_or(&[]);
            let over_budget = total >= TOTAL_BYTES_LIMIT;
            let trimmed = match step_value {
                Value::Object(fields) => {
                    let mut obj = Map::new();
                    for (key, value) in fields {
                        let is_declared = declared.contains(key);
                        let str_limit = if over_budget {
                            128
                        } else if is_declared {
                            DECLARED_STR_LIMIT
                        } else {
                            EXTRA_STR_LIMIT
                        };
                        let arr_limit = if over_budget {
                            4
                        } else if is_declared {
                            64
                        } else {
                            16
                        };
                        obj.insert(key.clone(), trim_value(value, str_limit, arr_limit));
                    }
                    Value::Object(obj)
                }
                other => trim_value(other, EXTRA_STR_LIMIT, 16),
            };
            total += serde_json::to_vec(&trimmed).map_or(0, |b| b.len());
            out.insert(program_id.clone(), trimmed);
        }
        out
    }
    fn start(&mut self, supplied: Option<&Value>) -> Reply {
        let (leading, names) = self.instruction_steps(0);
        self.bind(supplied, &names)?;
        // The opening instruction steps are shown to the agent before it
        // starts the run; a step that produces values is done once everything
        // it produces is bound. Steps with no declared outputs are never
        // skipped silently before the agent confirms them.
        for index in leading {
            let needed = self.capsule.plan_step(index).map(produces).unwrap_or(&[]);
            let done = !needed.is_empty()
                && needed
                    .iter()
                    .all(|name| self.state.bindings.contains_key(name));
            if !done {
                break;
            }
            self.state.completed.push(index);
            self.state.position = index + 1;
        }
        self.save()?;
        self.log.append(
            "started",
            json!({"capsule_sha256": self.state.capsule_sha256, "steps": self.capsule.plan_len()}),
        )?;
        self.advance(None)
    }
    fn resume(&mut self, supplied: Option<&Value>) -> Reply {
        match self.state.awaiting.clone() {
            Awaiting::Running {
                step_index,
                attempt,
            } => {
                self.bind(supplied, &[])?;
                self.log.append(
                    "interrupted",
                    json!({"step_index": step_index, "attempt": attempt}),
                )?;
                if attempt >= 2 {
                    let failed = tool::Failed {
                        reason: "interrupted",
                        exit_code: None,
                        diagnostics:
                            "The command was interrupted during the program's second attempt."
                                .into(),
                        missing: Vec::new(),
                        seconds: 0.0,
                    };
                    return self.fail(step_index, &failed, attempt);
                }
                self.state.position = step_index;
                self.advance(Some(2))
            }
            Awaiting::Bindings { names, .. } => {
                self.bind(supplied, &names)?;
                self.advance(None)
            }
            Awaiting::Steps {
                step_indexes,
                produces,
            } => {
                self.bind(supplied, &produces)?;
                let missing = produces
                    .iter()
                    .filter(|name| !self.state.bindings.contains_key(*name))
                    .cloned()
                    .collect::<Vec<_>>();
                if !missing.is_empty() {
                    self.save()?;
                    return self.your_steps(step_indexes, produces, &missing);
                }
                self.state.position = step_indexes
                    .last()
                    .map_or(self.state.position, |last| last + 1);
                self.state.completed.extend(step_indexes);
                self.state.awaiting = Awaiting::None;
                self.advance(None)
            }
            Awaiting::Outputs { step_index, names } => {
                self.bind(supplied, &names)?;
                let reason = self
                    .state
                    .failed
                    .iter()
                    .find(|failure| failure.step_index == step_index)
                    .map_or("failed", |failure| failure.reason.as_str())
                    .to_string();
                if names
                    .iter()
                    .any(|name| !self.state.bindings.contains_key(name))
                {
                    self.save()?;
                    return Ok(self.failed_answer(
                        step_index,
                        &reason,
                        json!({"detail": "The outputs later programs read are still missing."}),
                    ));
                }
                self.state.position = step_index + 1;
                self.state.awaiting = Awaiting::None;
                self.advance(None)
            }
            Awaiting::None => {
                // A finished run answers `complete` again and records nothing.
                if self.state.position >= self.capsule.plan_len() {
                    return Ok(self.complete());
                }
                self.bind(supplied, &[])?;
                self.advance(None)
            }
        }
    }
    fn status(&self) -> Value {
        json!({
            "status": "status",
            "position": self.state.position,
            "steps": self.capsule.plan_len(),
            "awaiting": self.state.to_value()["awaiting"],
            "completed": self.state.completed,
            "failed": self.state.failed.iter().map(|failure| json!({
                "step_index": failure.step_index, "reason": failure.reason,
            })).collect::<Vec<_>>(),
            "finished": self.state.position >= self.capsule.plan_len(),
        })
    }
}

fn admit(session: &Session<'_>) -> Result<(), Refusal> {
    let capsule = &session.package.capsule;
    if capsule.form != crate::Form::Hybrid {
        return Err(Refusal::new(
            "invalid_capsule",
            "A textual capsule is read, not run.",
        ));
    }
    // The structural rules again: a package that was changed by hand is
    // refused before anything of it runs.
    if let Some(violation) = gate1::structure(capsule, &session.package.files).first() {
        return Err(Refusal::new(
            "invalid_capsule",
            format!(
                "The capsule does not pass its structural gate ({} at {}).",
                violation.code, violation.at
            ),
        ));
    }
    Ok(())
}

/// One `skillcache-run` command. One command at a time per run: a second one
/// that arrives while the first still works is refused as `busy`.
pub fn execute(session: &Session<'_>, command: Command) -> Reply {
    admit(session)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(session.state_file.with_extension("lock"))
        .map_err(|error| {
            Refusal::new(
                "unavailable",
                format!("The run's lock is unavailable: {error}"),
            )
        })?;
    if lock.try_lock().is_err() {
        return Err(Refusal::new(
            "busy",
            "Another skillcache-run command is still working on this run.",
        ));
    }
    if matches!(command, Command::Reset) {
        let had_state = match std::fs::remove_file(session.state_file) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(Refusal::new(
                    "unavailable",
                    format!("The run's state could not be cleared: {error}"),
                ));
            }
        };
        let mut log = RunLog::open(session.state_file, session.generation)?;
        log.append(
            "reset",
            json!({"capsule_sha256": session.package.capsule_sha256, "cleared": had_state}),
        )?;
        return Ok(json!({"status": "reset", "cleared": had_state}));
    }
    let stored = RunState::load(session.state_file)?;
    if let Some(state) = &stored
        && state.capsule_sha256 != session.package.capsule_sha256
    {
        return Err(Refusal::new(
            "capsule_changed",
            "The run was started with another capsule.",
        ));
    }
    match (command, stored) {
        (Command::Reset, _) => unreachable!(),
        (Command::Run(_), Some(_)) => Err(Refusal::new(
            "already_started",
            "This run has started. Use `skillcache-run continue` or `skillcache-run status`.",
        )),
        (Command::Run(supplied), None) => {
            let state = RunState::new(&session.package.capsule_sha256, session.generation);
            Runner::open(session, state)?.start(supplied.as_ref())
        }
        (Command::Continue(_) | Command::Status, None) => Err(Refusal::new(
            "not_started",
            "No run has started. Use `skillcache-run run '{...}'` first.",
        )),
        (Command::Continue(supplied), Some(state)) => {
            Runner::open(session, state)?.resume(supplied.as_ref())
        }
        (Command::Status, Some(state)) => Ok(Runner::open(session, state)?.status()),
    }
}

/// Gate 3's end-to-end replay: the whole plan in one go, with the observed
/// values answering for the agent. A failed program is recorded, its observed
/// outputs stand in for it, and the replay goes on, so one pass collects every
/// failure. A read-only program that changes the workspace fails as well.
pub fn replay(session: &Session<'_>, examples: &Examples) -> Reply {
    admit(session)?;
    if RunState::load(session.state_file)?.is_some() {
        return Err(Refusal::new(
            "already_started",
            "A replay starts from an empty state.",
        ));
    }
    let capsule = &session.package.capsule;
    let state = RunState::new(&session.package.capsule_sha256, session.generation);
    let mut runner = Runner::open(session, state)?;
    runner.check_read_only = true;
    runner.record_changes = false;
    let take = |names: &[String]| -> Result<Value, Refusal> {
        let mut values = Map::new();
        for name in names {
            let value = examples.bindings.get(name).ok_or_else(|| {
                Refusal::new(
                    "example_missing",
                    format!("No observed value answers for `{name}`."),
                )
            })?;
            values.insert(name.clone(), value.clone());
        }
        Ok(Value::Object(values))
    };
    let declared = capsule
        .bindings
        .iter()
        .map(|binding| binding.name.clone())
        .chain(runner.instruction_steps(0).1)
        .filter(|name| examples.bindings.contains_key(name))
        .collect::<Vec<_>>();
    let mut answer = runner.start(Some(&take(&declared)?))?;
    let mut substituted = Vec::new();
    for _ in 0..=4 * capsule.plan_len() + 4 {
        let supplied = match answer["status"].as_str() {
            Some("complete") => break,
            Some("needs_bindings") => take(&names(&answer["names"]))?,
            Some("your_steps") => take(&names(&answer["produces"]))?,
            Some("program_failed") => {
                let index = answer["step_index"].as_u64().unwrap_or_default() as usize;
                let required = names(&answer["required_outputs"]);
                let observed = examples.steps.get(&index);
                let mut values = Map::new();
                for name in &required {
                    let value = observed.and_then(|result| result.get(name)).ok_or_else(|| {
                        Refusal::new(
                            "example_missing_output",
                            format!(
                                "Step {index} failed and no observed `{name}` can stand in for it."
                            ),
                        )
                    })?;
                    values.insert(name.clone(), value.clone());
                }
                substituted.push(index);
                Value::Object(values)
            }
            _ => return Err(Refusal::new("unavailable", "The replay lost its place.")),
        };
        answer = runner.resume(Some(&supplied))?;
    }
    if answer["status"] != "complete" {
        return Err(Refusal::new(
            "unavailable",
            "The replay did not reach the end of the plan.",
        ));
    }
    Ok(json!({
        "status": "replay",
        "passed": runner.failures.is_empty(),
        "steps": capsule.plan_len(),
        "completed": runner.state.completed,
        "failures": runner.failures,
        "substituted": substituted,
        "results": runner.state.results,
    }))
}

fn names(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|name| name.as_str().map(String::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignored_workspace_names_skip_dependency_trees_unless_safely_allowlisted() {
        let root = std::env::temp_dir().join(format!(
            "sc-exec-ignore-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for dir in [
            "node_modules",
            "vendor",
            ".venv",
            "dist",
            "build",
            ".next",
            ".git",
            "src",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("item.txt"), b"x").unwrap();
        }
        let default = workspace_snapshot_for_changes(&root).unwrap();
        assert_eq!(default.len(), 1);
        assert!(default[0].0.ends_with("src/item.txt"));

        assert!(!is_safe_allowlist_name(".git"));
        assert!(!is_safe_allowlist_name("../dist"));
        assert!(is_safe_allowlist_name("dist"));

        let allow = vec!["dist".into(), ".git".into()];
        let custom = workspace_snapshot_for_changes_with(&root, &allow).unwrap();
        let rels: Vec<String> = custom
            .iter()
            .filter_map(|e| e.0.strip_prefix(&root).ok())
            .map(|p| {
                let s = p.to_string_lossy();
                if cfg!(windows) {
                    s.replace('\\', "/")
                } else {
                    s.into_owned()
                }
            })
            .collect();
        assert!(rels.contains(&"src/item.txt".into()));
        assert!(rels.contains(&"dist/item.txt".into()));
        assert!(!rels.iter().any(|r| r.starts_with(".git")));
        assert!(!rels.iter().any(|r| r.starts_with("node_modules")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
