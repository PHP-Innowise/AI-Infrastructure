//! Optional code graph of a project, built by the external Graphify tool.
//!
//! The Harness finds an installed `graphify`, runs its deterministic `update`
//! pass in the background and tells an agent that the result exists. A build
//! starts without the previous graph, so a deleted file leaves no nodes behind,
//! and the published graph is replaced only by a build that succeeded.
use crate::{
    HarnessError, Result,
    graph_changes::{self, Pull, Resolved, Target},
    projects,
    services::{Domain, Payload, Reply},
    store::{self, SessionStore},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard, Weak,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
/// Assembly and clustering time relative to file reading, used until a project
/// has a finished build of its own. One repository measured 4.1 with Graphify
/// 0.9.105 and 4.8 with 0.7.13.
const TAIL_RATIO: f64 = 4.5;
const BUILD_LIMIT: Duration = Duration::from_secs(30 * 60);
/// Automatic rebuilds of one project keep at least this distance.
const AUTOMATIC_GAP: Duration = Duration::from_secs(30);
const PRINT_AGE: Duration = Duration::from_secs(10);
const TOOL_AGE: Duration = Duration::from_secs(30);
const PROBE_LIMIT: Duration = Duration::from_secs(5);
const LINE_LIMIT: usize = 300;
/// How long a listing of open pull requests is reused.
const PULLS_AGE: Duration = Duration::from_secs(60);
/// Graphs of other commits kept for a project; each is as large as its own.
const REVISIONS: usize = 3;
/// What a build is doing before the tool runs.
const WAITING: u8 = 1;
const EXPORTING: u8 = 2;
/// Keys that could let the tool reach a model provider. The update pass needs
/// none of them, and the graph of a project stays on this machine.
const PROVIDER_KEYS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GRAPHIFY_API_KEY",
    "MOONSHOT_API_KEY",
    "OPENAI_API_KEY",
];
/// Graphify 0.9 rewrites the assistant skills installed in the user's home
/// when it is newer than they are, on its first command that is not a version
/// or help request. A build started here must change nothing outside the graph
/// folder; the version probe carries the switch as well, in case a release
/// refreshes on that too.
const NO_SKILL_REFRESH: (&str, &str) = ("GRAPHIFY_NO_AUTO_REFRESH", "1");
/// What one output line of the tool says about the build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Files read so far, out of the files this build has to read.
    Reading {
        done: u64,
        total: u64,
    },
    /// Files left to read after the cache answered for the rest.
    Pending(u64),
    Built {
        nodes: u64,
        edges: u64,
    },
}
fn number(text: &str) -> Option<(u64, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    Some((text[..end].parse().ok()?, &text[end..]))
}
/// Reads the progress lines Graphify 0.7 and 0.9 print; any other line is not
/// a signal, so an unknown release only loses the measured part of the bar.
pub fn signal(line: &str) -> Option<Signal> {
    if let Some((_, rest)) = line.split_once("AST extraction: ") {
        let (done, rest) = number(rest)?;
        let (total, _) = number(rest.strip_prefix('/')?)?;
        return Some(Signal::Reading { done, total });
    }
    if let Some((_, rest)) = line.split_once("AST cache: ") {
        let (_, rest) = rest.split_once("; ")?;
        let (left, rest) = number(rest)?;
        return rest
            .starts_with(" to extract")
            .then_some(Signal::Pending(left));
    }
    if let Some((_, rest)) = line.split_once("Rebuilt") {
        let (_, rest) = rest.split_once(": ")?;
        let (nodes, rest) = number(rest)?;
        let (edges, rest) = number(rest.strip_prefix(" nodes, ")?)?;
        return rest
            .starts_with(" edges")
            .then_some(Signal::Built { nodes, edges });
    }
    None
}
/// The measured part of a running build, in seconds since it started.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observed {
    first: Option<(f64, u64)>,
    last: Option<(f64, u64)>,
    total: u64,
    read: Option<f64>,
}
impl Observed {
    pub fn note(&mut self, at: f64, signal: Signal) {
        match signal {
            Signal::Reading { done, total } => {
                self.total = total;
                self.first.get_or_insert((at, done));
                self.last = Some((at, done));
                if done >= total {
                    self.read.get_or_insert(at);
                }
            }
            Signal::Pending(0) => {
                self.read.get_or_insert(at);
            }
            Signal::Pending(left) => self.total = left,
            Signal::Built { .. } => {}
        }
    }
}
/// The last finished build of the same project by the same tool release.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct History {
    pub seconds: f64,
    /// From the last file read to the end; the tool reports nothing there.
    pub tail: Option<f64>,
}
/// Expected length of the running build. The tool counts only the files it
/// reads, so the silent rest comes from this project's previous build, or from
/// `TAIL_RATIO` on a first build. `None` means there is nothing to go by yet.
pub fn expected(elapsed: f64, seen: &Observed, history: Option<History>) -> Option<f64> {
    let tail = |head: f64| match history {
        Some(past) => past.tail.unwrap_or(past.seconds),
        None => TAIL_RATIO * head.max(1.0),
    };
    if let Some(read) = seen.read {
        return Some(read + tail(read));
    }
    if let (Some((start, begun)), Some((at, done))) = (seen.first, seen.last)
        && done > begun
        && at > start
        && seen.total >= done
    {
        let rate = (done - begun) as f64 / (at - start);
        let head = (at + (seen.total - done) as f64 / rate).max(elapsed);
        return Some(head + tail(head));
    }
    history.map(|past| past.seconds)
}
/// Percent, seconds left and whether the build outlasted its estimate. The
/// percent never goes below `floor` and stays under 100 until the build ends.
pub fn progress(
    elapsed: f64,
    seen: &Observed,
    history: Option<History>,
    floor: u64,
) -> (Option<u64>, Option<u64>, bool) {
    match expected(elapsed, seen, history) {
        Some(total) if total > 0.0 => {
            let percent = ((elapsed / total * 100.0).floor() as u64)
                .min(99)
                .max(floor);
            let left = (total - elapsed).max(0.0).ceil() as u64;
            (Some(percent), Some(left), elapsed > total)
        }
        _ => (None, None, false),
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    pub path: PathBuf,
    pub version: Option<String>,
}
fn probe(program: &Path, arguments: &[&str]) -> Option<String> {
    let mut child = Command::new(program)
        .args(arguments)
        .env(NO_SKILL_REFRESH.0, NO_SKILL_REFRESH.1)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if started.elapsed() < PROBE_LIMIT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let mut text = String::new();
    child
        .stdout
        .take()?
        .take(4096)
        .read_to_string(&mut text)
        .ok()?;
    (code == Some(0)).then_some(text)
}
fn release(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|word| {
            let mut parts = word.split('.');
            word.len() <= 40
                && parts.next().is_some_and(|part| {
                    !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
                })
                && parts.next().is_some()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
        })
        .map(str::to_owned)
}
/// Graphify 0.9 answers `--version`; older releases are asked through the
/// interpreter their launcher names.
fn version(program: &Path) -> Option<String> {
    if let Some(found) = probe(program, &["--version"]).and_then(|text| release(&text)) {
        return Some(found);
    }
    let mut head = [0u8; 512];
    let read = std::fs::File::open(program).ok()?.read(&mut head).ok()?;
    let line = String::from_utf8_lossy(&head[..read]);
    let interpreter = Path::new(line.lines().next()?.strip_prefix("#!")?.trim());
    if !interpreter.is_absolute() || !interpreter.is_file() {
        return None;
    }
    probe(
        interpreter,
        &[
            "-I",
            "-c",
            "import importlib.metadata as m;print(m.version('graphifyy'))",
        ],
    )
    .and_then(|text| release(&text))
}
/// Identity of the project's files: the commit plus every uncommitted path with
/// its size and modification time. `None` outside Git, where nothing cheap
/// says that files changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Print {
    pub hash: String,
    pub commit: Option<String>,
}
pub fn print(root: &Path) -> Option<Print> {
    let git = |arguments: &[&str]| {
        let arguments = arguments
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        projects::git(root, &arguments, 20).ok()
    };
    if git(&["rev-parse", "--is-inside-work-tree"])?.code != 0 {
        return None;
    }
    let head = git(&["rev-parse", "--verify", "HEAD"])?;
    let commit = (head.code == 0)
        .then(|| String::from_utf8_lossy(&head.stdout).trim().to_owned())
        .filter(|value| !value.is_empty());
    let status = git(&["status", "--porcelain=v1", "-z"])?;
    if status.code != 0 {
        return None;
    }
    let mut identity = commit.clone().unwrap_or_default().into_bytes();
    identity.push(0);
    identity.extend_from_slice(&status.stdout);
    for entry in status.stdout.split(|byte| *byte == 0) {
        let Some(path) = entry.get(3..).and_then(|raw| std::str::from_utf8(raw).ok()) else {
            continue;
        };
        // A folder's own time moves when a file inside is replaced, which says
        // nothing about the files Git already lists.
        if let Ok(meta) = std::fs::metadata(root.join(path))
            && meta.is_file()
        {
            let changed = meta
                .modified()
                .ok()
                .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |at| at.as_nanos());
            identity.extend_from_slice(format!("\0{}:{changed}", meta.len()).as_bytes());
        }
    }
    Some(Print {
        hash: store::sha(&identity),
        commit: commit.map(|value| value.chars().take(12).collect()),
    })
}
fn key(project: &str) -> String {
    format!("graph.{project}")
}
fn folder(state: &Path, project: &str) -> PathBuf {
    state.join("graphs").join(project)
}
/// The published graph an agent queries.
pub fn live(state: &Path, project: &str) -> PathBuf {
    folder(state, project).join("live").join("graph.json")
}
/// The graph of another commit of the project, as a pull request needs it.
fn revision(state: &Path, project: &str, commit: &str) -> PathBuf {
    folder(state, project)
        .join("revisions")
        .join(commit)
        .join("graph.json")
}
/// What was chosen, for the interface: the two sides and how many files.
fn chosen(resolved: &Resolved) -> Value {
    let mut chosen = resolved.json();
    chosen["files"] = json!(resolved.files.len());
    chosen
}
fn size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => size(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |meta| meta.len()),
            _ => 0,
        })
        .sum()
}
fn tenth(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}
fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| HarnessError::unavailable("The code graph state is unavailable."))
}
/// A path as one word of a shell command.
fn word(path: &Path) -> String {
    spelled(&path.to_string_lossy(), cfg!(windows))
}
/// On Windows a drive path is written with forward separators: Claude Code
/// runs commands in Git Bash, where an unquoted backslash is an escape, while
/// PowerShell and the tool itself take either separator. A network or
/// extended path keeps its backslashes, which mean something there, in quotes.
fn spelled(text: &str, windows: bool) -> String {
    let text = if windows && !text.starts_with("\\\\") {
        text.replace('\\', "/")
    } else {
        text.to_owned()
    };
    if text
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/._-:".contains(&b))
    {
        text
    } else if windows {
        format!("\"{text}\"")
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}
/// The questions an agent may ask a graph, each with the arguments the prompt
/// shows for it. The graph file is always named last.
const QUESTIONS: [(&str, &str); 3] = [
    ("query", "\"<question>\""),
    ("explain", "\"<name>\""),
    ("path", "\"<A>\" \"<B>\""),
];
/// A published graph as a launch offers it: the tool and the graph file, each
/// spelled as one shell word. The prompt's lines and the launch's permission
/// are made from the same two words, because Claude Code matches a permission
/// against the command as it is written.
///
/// A graph is offered only where the person asked for agents to be told
/// about it. That is a switch of its own, off until asked for: the map needs
/// no agent, the lines cost tokens on every call of the model, and an agent
/// that was only offered the graph did not turn to it in the runs this was
/// checked with.
struct Offer {
    command: String,
    graph: String,
    built: Value,
}
fn offer(store: &SessionStore, project: &str) -> Result<Option<Offer>> {
    let Some(setting) = store.setting(&key(project))? else {
        return Ok(None);
    };
    let built = &setting["built"];
    let graph = live(&store.state, project);
    let Some(program) = built["program"].as_str().map(Path::new) else {
        return Ok(None);
    };
    if setting["enabled"] != true
        || setting["agents"] != true
        || !graph.is_file()
        || !program.is_file()
    {
        return Ok(None);
    }
    let command = if infra_host::provider::find_executable("graphify").as_deref() == Some(program) {
        "graphify".to_owned()
    } else {
        word(program)
    };
    Ok(Some(Offer {
        command,
        graph: word(&graph),
        built: built.clone(),
    }))
}
/// What a launch gives the agent along with the prompt's graph lines.
pub struct Grant {
    /// Claude Code permission rules, one per question: this tool, any
    /// arguments, then this project's published graph as the last word. Claude
    /// Code checks each part of a compound command on its own and refuses a
    /// substitution, so a rule runs the question and nothing joined to it; a
    /// command that names another graph after this one does not match.
    pub rules: Vec<String>,
    /// For the agent's shell: a question rewrites no skill in the person's home.
    pub environment: [(&'static str, &'static str); 1],
}
/// The person asked for agents to be told about this project's graph, and
/// that switch says what it permits. Without the rules Claude Code in Edit
/// mode refuses the very commands the prompt offers, since a launch without a
/// terminal has nobody to ask, and in Plan mode it spends a call of its own
/// classifier on each.
pub fn grant(store: &SessionStore, project: &str) -> Result<Option<Grant>> {
    Ok(offer(store, project)?.map(|offer| Grant {
        rules: QUESTIONS
            .iter()
            .map(|(verb, _)| format!("Bash({} {verb} * --graph {})", offer.command, offer.graph))
            .collect(),
        environment: [NO_SKILL_REFRESH],
    }))
}
/// Gives a session's launch what the prompt's graph lines need: the rules in
/// Claude Code's `--settings`, beside whatever the launch and an attached
/// edition already put there, and the environment for any host. A project
/// whose graph is off, unpublished or not offered to agents gets nothing.
pub fn prepare(
    store: &SessionStore,
    project: &str,
    provider: &str,
    command: &mut infra_host::provider::PreparedCommand,
) -> Result<()> {
    let Some(grant) = grant(store, project)? else {
        return Ok(());
    };
    command.environment.extend(
        grant
            .environment
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
    );
    if provider != "claude" {
        return Ok(());
    }
    let wrong = || HarnessError::invalid("Claude settings cannot take the code graph permission.");
    let index = command.argv.iter().position(|value| value == "--settings");
    let mut settings: Value = match index {
        Some(index) => serde_json::from_str(command.argv.get(index + 1).ok_or_else(wrong)?)?,
        None => json!({}),
    };
    let allow = settings
        .as_object_mut()
        .ok_or_else(wrong)?
        .entry("permissions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(wrong)?
        .entry("allow")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(wrong)?;
    for rule in grant.rules.into_iter().map(Value::String) {
        if !allow.contains(&rule) {
            allow.push(rule);
        }
    }
    match index {
        Some(index) => command.argv[index + 1] = settings.to_string(),
        None => command
            .argv
            .extend(["--settings".to_owned(), settings.to_string()]),
    }
    Ok(())
}
/// A graph published for a project without a build, for tests of what a
/// launch does with one.
#[cfg(test)]
pub(crate) fn publish_for_tests(store: &SessionStore, project: &str, program: &Path) {
    let graph = live(&store.state, project);
    std::fs::create_dir_all(graph.parent().unwrap()).unwrap();
    std::fs::write(&graph, "{}").unwrap();
    std::fs::write(program, "").unwrap();
    store
        .set_setting(
            &key(project),
            &json!({"enabled": true, "agents": true, "built": {"program": program.to_string_lossy(), "nodes": 7}}),
        )
        .unwrap();
}
/// The lines a launch adds when the session's project has a published graph
/// that agents are to be told about.
/// The graph is offered, never required, and the report is never inlined: a
/// query answers in about two thousand tokens, the report runs to hundreds of
/// kilobytes.
/// The lines ask for the graph to be named last because the permission does:
/// asked to start from the graph, Codex wrote `--budget 4500` after it, which
/// Claude Code would have refused.
pub fn prompt_line(store: &SessionStore, session: &Value) -> Result<Option<String>> {
    let Some(project) = session["project_id"].as_str().filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    let Some(Offer {
        command,
        graph,
        built,
    }) = offer(store, project)?
    else {
        return Ok(None);
    };
    let origin = built["commit"]
        .as_str()
        .map(|commit| format!(" at commit {commit}"))
        .unwrap_or_default();
    let nodes = built["nodes"]
        .as_u64()
        .map(|nodes| format!("{nodes} nodes"))
        .unwrap_or_else(|| "a graph".to_owned());
    let elsewhere = store
        .project(project)
        .ok()
        .is_some_and(|record| record["path"] != session["project_path"]);
    let questions: String = QUESTIONS
        .iter()
        .map(|(verb, arguments)| format!("{command} {verb} {arguments} --graph {graph}\n"))
        .collect();
    Ok(Some(format!(
        "Code graph (optional; the files remain the source of truth): Graphify indexed the project folder{origin} into {nodes}{}. When a question is where something is defined, what uses it or how two parts connect, one of these can replace a broad search:\n{questions}Keep --graph and its path last; an option such as --budget N goes before them. The graph can lag behind later edits, so confirm in the files.\n\n",
        if elsewhere {
            "; this session runs in another checkout that may differ"
        } else {
            ""
        }
    )))
}
/// Whether a shell command asks a code graph: `graphify` followed by `query`,
/// `explain` or `path`, however the executable is spelled.
pub fn asked(command: &str) -> bool {
    command.split(['\n', ';', '|', '&', '(', ')']).any(|part| {
        let part = part.trim_start();
        let (program, rest) = match part.chars().next() {
            Some(quote @ ('"' | '\'')) => match part[1..].split_once(quote) {
                Some(split) => split,
                None => return false,
            },
            _ => part.split_once(char::is_whitespace).unwrap_or((part, "")),
        };
        let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
        matches!(name, "graphify" | "graphify.exe")
            && matches!(
                rest.split_whitespace().next(),
                Some("query" | "explain" | "path")
            )
    })
}
#[derive(Default)]
struct Seen {
    observed: Observed,
    built: Option<(u64, u64)>,
    /// Last line of the ordinary output and of the error output; a failure is
    /// explained by the error output when there is any.
    line: String,
    complaint: String,
    peak: u64,
}
struct Job {
    started: Mutex<Instant>,
    /// `WAITING` or `EXPORTING` until the tool runs.
    stage: AtomicU8,
    cancel: AtomicBool,
    purge: AtomicBool,
    again: AtomicBool,
    seen: Mutex<Seen>,
}
impl Job {
    fn new() -> Self {
        Self {
            started: Mutex::new(Instant::now()),
            stage: AtomicU8::new(0),
            cancel: AtomicBool::new(false),
            purge: AtomicBool::new(false),
            again: AtomicBool::new(false),
            seen: Mutex::new(Seen::default()),
        }
    }
    fn started(&self) -> Instant {
        *self
            .started
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// The estimate counts the tool's own time, not the wait before it.
    fn restart(&self) {
        *self
            .started
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
    }
    fn observe(&self, line: &str, complaint: bool) {
        let text = line.trim();
        if text.is_empty() {
            return;
        }
        let at = self.started().elapsed().as_secs_f64();
        let Ok(mut seen) = self.seen.lock() else {
            return;
        };
        if let Some(found) = signal(text) {
            if let Signal::Built { nodes, edges } = found {
                seen.built = Some((nodes, edges));
            }
            seen.observed.note(at, found);
        }
        let kept = text.chars().take(LINE_LIMIT).collect();
        if complaint {
            seen.complaint = kept;
        } else {
            seen.line = kept;
        }
    }
    fn view(&self, history: Option<History>) -> Value {
        let elapsed = self.started().elapsed().as_secs_f64();
        let stage = self.stage.load(Ordering::Acquire);
        if stage != 0 {
            return json!({
                "phase": if stage == WAITING { "wait" } else { "export" },
                "done": null,
                "total": null,
                "percent": null,
                "remaining_seconds": null,
                "rough": history.is_none(),
                "elapsed_seconds": elapsed.floor() as u64,
                "overdue": false,
            });
        }
        let Ok(mut seen) = self.seen.lock() else {
            return Value::Null;
        };
        let (percent, left, overdue) = progress(elapsed, &seen.observed, history, seen.peak);
        if let Some(percent) = percent {
            seen.peak = percent;
        }
        let phase = if seen.observed.read.is_some() {
            "assemble"
        } else if seen.observed.first.is_some() {
            "read"
        } else {
            "scan"
        };
        json!({
            "phase": phase,
            "done": seen.observed.last.map(|(_, done)| done),
            "total": (seen.observed.total > 0).then_some(seen.observed.total),
            "percent": percent,
            "remaining_seconds": left,
            // Without a previous build the silent part is a guess from one ratio.
            "rough": history.is_none(),
            "elapsed_seconds": elapsed.floor() as u64,
            "overdue": overdue,
        })
    }
}
enum Outcome {
    Built(Value),
    Failed(String),
    Cancelled,
}
/// A run of the tool that left a graph.
struct Ran {
    fresh: PathBuf,
    elapsed: f64,
    read: Option<f64>,
    built: Option<(u64, u64)>,
    bytes: u64,
}
impl Ran {
    fn record(&self, disk: u64) -> Value {
        json!({
            "at": store::now(),
            "seconds": tenth(self.elapsed),
            "tail": self.read.map(|read| tenth(self.elapsed - read)),
            "nodes": self.built.map(|(nodes, _)| nodes),
            "edges": self.built.map(|(_, edges)| edges),
            "bytes": self.bytes,
            "disk": disk,
        })
    }
}
/// Where the graph of a chosen change is, or how far its build has come.
struct Found {
    state: &'static str,
    build: Value,
    failure: Option<String>,
    /// The name the read graph is kept under, the graph, and the folder its
    /// paths are relative to.
    graph: Option<(String, PathBuf, PathBuf)>,
}
enum Manifest {
    Absent,
    Present(Vec<u8>, Option<std::time::SystemTime>),
    /// It exists and could not be read, so it is left alone.
    Unknown,
}
/// Graphify before 0.9 writes its manifest into the project's own
/// `graphify-out/` even when the graph goes elsewhere. That folder may hold a
/// graph the user built themselves, so it is put back as it was when the build
/// ends, however it ends.
struct Kept {
    folder: PathBuf,
    existed: bool,
    manifest: Manifest,
}
impl Kept {
    fn of(root: &Path) -> Self {
        let folder = root.join("graphify-out");
        let path = folder.join("manifest.json");
        let manifest = match std::fs::read(&path) {
            Ok(bytes) => Manifest::Present(
                bytes,
                std::fs::metadata(&path)
                    .and_then(|meta| meta.modified())
                    .ok(),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Manifest::Absent,
            Err(_) => Manifest::Unknown,
        };
        Self {
            existed: folder.exists(),
            folder,
            manifest,
        }
    }
}
impl Drop for Kept {
    fn drop(&mut self) {
        let path = self.folder.join("manifest.json");
        match &self.manifest {
            Manifest::Present(bytes, changed) => {
                let now = std::fs::metadata(&path)
                    .and_then(|meta| meta.modified())
                    .ok();
                if std::fs::read(&path).ok().as_deref() == Some(bytes.as_slice()) && now == *changed
                {
                    return;
                }
                if std::fs::write(&path, bytes).is_ok()
                    && let Some(changed) = changed
                    && let Ok(file) = std::fs::File::options().write(true).open(&path)
                {
                    let _ = file.set_modified(*changed);
                }
            }
            Manifest::Absent => {
                let _ = std::fs::remove_file(&path);
                if !self.existed {
                    let _ = std::fs::remove_dir(&self.folder);
                }
            }
            Manifest::Unknown => {}
        }
    }
}
/// Why a build is being considered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// The graph was switched on for the project.
    Enabled,
    /// The user asked for a rebuild.
    Manual,
    /// The project was opened in the interface.
    Visit,
    /// A run is starting in the project.
    Launch,
    /// A run in the project ended.
    Finish,
}
fn pump(
    mut stream: impl Read + Send + 'static,
    job: Arc<Job>,
    complaint: bool,
) -> Option<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("harness-graph-output".into())
        .spawn(move || {
            let mut pending = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(read) = stream.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                for byte in &chunk[..read] {
                    if matches!(byte, b'\n' | b'\r') {
                        job.observe(&String::from_utf8_lossy(&pending), complaint);
                        pending.clear();
                    } else if pending.len() < 4096 {
                        pending.push(*byte);
                    }
                }
            }
            job.observe(&String::from_utf8_lossy(&pending), complaint);
        })
        .ok()
}
/// Ends the tool and the worker processes it started.
fn end(child: &mut Child) {
    #[cfg(unix)]
    {
        // The child leads its own process group, so the group id is its pid.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}
pub struct Graphs {
    me: Weak<Graphs>,
    store: SessionStore,
    program: Option<PathBuf>,
    gap: Duration,
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    tool: Mutex<Option<(Instant, Option<Tool>)>>,
    prints: Mutex<BTreeMap<String, (Instant, Option<Print>)>>,
    finished: Mutex<BTreeMap<String, Instant>>,
    writing: Mutex<()>,
    stopping: AtomicBool,
    maps: crate::graph_map::Maps,
    /// Open pull requests of each project as last listed, or why not.
    pulls: Mutex<BTreeMap<String, Listing>>,
    /// Pull requests found for a project, a state and a search.
    searches: Mutex<BTreeMap<(String, String, String), Listing>>,
    /// The change last worked out for each project and target.
    chosen: Mutex<BTreeMap<(String, String), Arc<Resolved>>>,
    /// The commit whose graph is being built for each project.
    revisions: Mutex<BTreeMap<String, (String, Arc<Job>)>>,
    /// Why the last build of a commit's graph failed.
    refused: Mutex<BTreeMap<(String, String), String>>,
}
type Listing = (Instant, Vec<Pull>, Option<String>);
impl Graphs {
    /// `HARNESS_GRAPHIFY` names the executable when it is not on `PATH`.
    pub fn new(store: SessionStore) -> Arc<Self> {
        let program = std::env::var_os("HARNESS_GRAPHIFY")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        Self::with(store, program, AUTOMATIC_GAP)
    }
    pub fn with(store: SessionStore, program: Option<PathBuf>, gap: Duration) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            store,
            program,
            gap,
            jobs: Mutex::new(BTreeMap::new()),
            workers: Mutex::new(Vec::new()),
            tool: Mutex::new(None),
            prints: Mutex::new(BTreeMap::new()),
            finished: Mutex::new(BTreeMap::new()),
            writing: Mutex::new(()),
            stopping: AtomicBool::new(false),
            maps: crate::graph_map::Maps::default(),
            pulls: Mutex::new(BTreeMap::new()),
            searches: Mutex::new(BTreeMap::new()),
            chosen: Mutex::new(BTreeMap::new()),
            revisions: Mutex::new(BTreeMap::new()),
            refused: Mutex::new(BTreeMap::new()),
        })
    }
    /// One folder of the published graph's dependency map, or, with a
    /// target, the map of what that target changed. Reading a graph of a
    /// large project takes seconds, so callers keep this off the runtime.
    pub fn map(
        &self,
        project: &str,
        target: Option<&Target>,
        scope: &str,
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Result<Value> {
        let record = self.store.project(project)?;
        let root = PathBuf::from(record["path"].as_str().unwrap_or(""));
        let name = record["name"].as_str().unwrap_or("project");
        let built = &self.setting(project)?["built"];
        let Some(target) = target else {
            let index = self
                .maps
                .index(project, &live(&self.store.state, project), &root)?;
            let mut view = index.view(name, scope, focus, certain, filter);
            view["project_id"] = json!(project);
            view["built"] = json!({"commit": built["commit"], "at": built["at"]});
            return Ok(view);
        };
        let resolved = match target {
            // The checkout is asked every time: it is local, and it moves.
            Target::Work | Target::Uncommitted => Arc::new(graph_changes::resolve(
                &root,
                target,
                &self.listed(project, &root),
            )?),
            _ => lock(&self.chosen)?
                .get(&(project.to_owned(), target.text()))
                .cloned()
                .ok_or_else(|| HarnessError::conflict("Choose the change again."))?,
        };
        let found = self.graph_of(project, &root, &resolved)?;
        let changed = resolved
            .files
            .iter()
            .map(|change| (change.path.clone(), change.status))
            .collect::<Vec<_>>();
        let published = live(&self.store.state, project);
        // The checkout's own graph may be older than its files, and
        // looking at its changes is a reason to bring it up to date.
        let own = resolved.revision.is_none();
        let mut stale = false;
        if own {
            self.refresh(project, Trigger::Visit)?;
            let now = self.print(project, &root, false);
            stale = now.is_some_and(|now| built["print"].as_str() != Some(now.hash.as_str()));
        }
        let mut view = match &found.graph {
            Some((kept, graph, base)) => {
                let index = self.maps.index(kept, graph, base)?;
                // An opened file marks the symbols the change touched. They
                // are told by line, and the lines of a graph older than the
                // files are not the files' own.
                let edited = match index.opens(scope) {
                    Some(path) if !stale => {
                        graph_changes::edited(&root, &resolved, path).ok().flatten()
                    }
                    _ => None,
                };
                match index.symbols(
                    name,
                    scope,
                    &changed,
                    edited.as_ref(),
                    focus,
                    certain,
                    filter,
                ) {
                    Some(view) => view,
                    None => index.changes(name, &changed, scope, focus, certain, filter),
                }
            }
            // Until the commit's own graph is there, the project's graph
            // gives a first picture: the files changed are the commit's, the
            // links are those the checkout's files have.
            None if published.is_file() => {
                let mut view = self
                    .maps
                    .index(project, &published, &root)?
                    .changes(name, &changed, scope, focus, certain, filter);
                view["preliminary"] = json!(true);
                view
            }
            None => json!({"mode": "changes"}),
        };
        view["project_id"] = json!(project);
        view["target"] = json!(target.text());
        view["chosen"] = chosen(&resolved);
        view["state"] = json!(found.state);
        view["build"] = found.build;
        view["failure"] = json!(found.failure);
        if own {
            view["stale"] = json!(stale);
            view["rebuilding"] = json!(lock(&self.jobs)?.contains_key(project));
            view["built"] = json!({"commit": built["commit"], "at": built["at"]});
        }
        Ok(view)
    }
    fn setting(&self, project: &str) -> Result<Value> {
        Ok(self
            .store
            .setting(&key(project))?
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({"enabled": false})))
    }
    fn edit(&self, project: &str, change: impl FnOnce(&mut Value)) -> Result<()> {
        let _writing = lock(&self.writing)?;
        let mut setting = self.setting(project)?;
        change(&mut setting);
        self.store.set_setting(&key(project), &setting)
    }
    fn tool(&self, fresh: bool) -> Option<Tool> {
        let mut cached = self.tool.lock().ok()?;
        if !fresh
            && let Some((at, found)) = cached.as_ref()
            && at.elapsed() < TOOL_AGE
        {
            return found.clone();
        }
        let path = match &self.program {
            Some(path) => path.is_file().then(|| path.clone()),
            None => infra_host::provider::find_executable("graphify"),
        };
        let found = path.map(|path| Tool {
            version: version(&path),
            path,
        });
        *cached = Some((Instant::now(), found.clone()));
        found
    }
    fn print(&self, project: &str, root: &Path, fresh: bool) -> Option<Print> {
        let mut prints = self.prints.lock().ok()?;
        if !fresh
            && let Some((at, found)) = prints.get(project)
            && at.elapsed() < PRINT_AGE
        {
            return found.clone();
        }
        let found = print(root);
        prints.insert(project.to_owned(), (Instant::now(), found.clone()));
        found
    }
    fn busy(&self, project: &str) -> Result<bool> {
        Ok(self.store.connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE project_id=?1 AND status IN('queued','running'))",
            [project],
            |row| row.get(0),
        )?)
    }
    fn root(&self, project: &str) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.store.project(project)?["path"].as_str().unwrap_or(""),
        ))
    }
    /// Starts a build when the trigger warrants one. An automatic trigger
    /// builds only what changed since the last attempt, never within the gap
    /// after the previous build, and never under a run except at its launch.
    pub fn refresh(&self, project: &str, trigger: Trigger) -> Result<()> {
        if self.stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        let setting = self.setting(project)?;
        if setting["enabled"] != true {
            return Ok(());
        }
        if let Some(job) = lock(&self.jobs)?.get(project) {
            if trigger == Trigger::Finish {
                job.again.store(true, Ordering::Release);
            }
            return Ok(());
        }
        let asked = matches!(trigger, Trigger::Enabled | Trigger::Manual);
        // One run of the tool at a time unless the person asks for another.
        if !asked && lock(&self.revisions)?.contains_key(project) {
            return Ok(());
        }
        let Some(tool) = self.tool(asked) else {
            return Ok(());
        };
        let root = self.root(project)?;
        let print = self.print(project, &root, trigger != Trigger::Visit);
        if trigger != Trigger::Manual {
            let now = print.as_ref().map(|print| print.hash.as_str());
            if setting["built"].is_object() {
                // Outside Git nothing says that files changed; rebuild on request.
                if now.is_none() || setting["built"]["print"].as_str() == now {
                    return Ok(());
                }
            }
            if setting["failure"].is_object() && setting["failure"]["print"].as_str() == now {
                return Ok(());
            }
        }
        if !asked {
            if lock(&self.finished)?
                .get(project)
                .is_some_and(|at| at.elapsed() < self.gap)
            {
                return Ok(());
            }
            if trigger != Trigger::Launch && self.busy(project)? {
                return Ok(());
            }
        }
        self.begin(project, root, tool, print)
    }
    fn begin(&self, project: &str, root: PathBuf, tool: Tool, print: Option<Print>) -> Result<()> {
        let Some(me) = self.me.upgrade() else {
            return Ok(());
        };
        let job = Arc::new(Job::new());
        {
            let mut jobs = lock(&self.jobs)?;
            if jobs.contains_key(project) {
                return Ok(());
            }
            jobs.insert(project.to_owned(), job.clone());
        }
        let name = project.to_owned();
        let worker = std::thread::Builder::new()
            .name("harness-graph".into())
            .spawn(move || {
                let outcome = me.build(&name, &root, &tool, &job);
                me.settle(&name, &job, outcome, print, &tool);
            });
        match worker {
            Ok(handle) => {
                let mut workers = lock(&self.workers)?;
                workers.retain(|worker| !worker.is_finished());
                workers.push(handle);
                Ok(())
            }
            Err(_) => {
                lock(&self.jobs)?.remove(project);
                Err(HarnessError::unavailable(
                    "The code graph build could not start.",
                ))
            }
        }
    }
    /// Runs the tool over `root` with its output in `out` and waits for it.
    /// `Err` is how the build ended when no graph came of it.
    fn run(
        &self,
        tool: &Tool,
        root: &Path,
        out: &Path,
        job: &Arc<Job>,
    ) -> std::result::Result<Ran, Outcome> {
        // A build that starts from the previous graph keeps nodes of files that
        // no longer exist; the cache beside it makes the clean build no slower.
        let fresh = out.join("graph.json");
        if let Err(error) = std::fs::remove_file(&fresh)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(Outcome::Failed(
                "The previous build output could not be cleared.".into(),
            ));
        }
        let mut command = Command::new(&tool.path);
        command
            .arg("update")
            .arg(root)
            .current_dir(root)
            .env("GRAPHIFY_OUT", out)
            .env("GRAPHIFY_BACKEND", "none")
            .env("GRAPHIFY_NO_TIPS", "1")
            .env(NO_SKILL_REFRESH.0, NO_SKILL_REFRESH.1)
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in PROVIDER_KEYS {
            command.env_remove(name);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let Ok(mut child) = command.spawn() else {
            return Err(Outcome::Failed("Graphify could not start.".into()));
        };
        let readers = [
            child
                .stdout
                .take()
                .and_then(|pipe| pump(pipe, job.clone(), false)),
            child
                .stderr
                .take()
                .and_then(|pipe| pump(pipe, job.clone(), true)),
        ];
        let stopped =
            || job.cancel.load(Ordering::Acquire) || self.stopping.load(Ordering::Acquire);
        let code = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code(),
                Ok(None) if stopped() => {
                    end(&mut child);
                    return Err(Outcome::Cancelled);
                }
                Ok(None) if job.started().elapsed() > BUILD_LIMIT => {
                    end(&mut child);
                    return Err(Outcome::Failed(
                        "Graphify did not finish within 30 minutes.".into(),
                    ));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(_) => {
                    end(&mut child);
                    return Err(Outcome::Failed("Graphify could not be watched.".into()));
                }
            }
        };
        // Workers can outlive the leader and hold its pipes open.
        end(&mut child);
        let waiting = Instant::now();
        for reader in readers.into_iter().flatten() {
            while !reader.is_finished() && waiting.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(10));
            }
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
        let elapsed = job.started().elapsed().as_secs_f64();
        let (read, built, line) = match job.seen.lock() {
            Ok(seen) => (
                seen.observed.read,
                seen.built,
                if seen.complaint.is_empty() {
                    seen.line.clone()
                } else {
                    seen.complaint.clone()
                },
            ),
            Err(_) => (None, None, String::new()),
        };
        if code != Some(0) {
            return Err(Outcome::Failed(if line.is_empty() {
                "Graphify stopped without a message.".into()
            } else {
                line
            }));
        }
        let bytes = match std::fs::metadata(&fresh) {
            Ok(meta) if meta.is_file() && meta.len() > 0 => meta.len(),
            _ => {
                return Err(Outcome::Failed(
                    "Graphify finished without writing a graph.".into(),
                ));
            }
        };
        if stopped() {
            return Err(Outcome::Cancelled);
        }
        Ok(Ran {
            fresh,
            elapsed,
            read,
            built,
            bytes,
        })
    }
    fn build(&self, project: &str, root: &Path, tool: &Tool, job: &Arc<Job>) -> Outcome {
        let home = folder(&self.store.state, project);
        let work = home.join("work");
        let live = home.join("live");
        if std::fs::create_dir_all(&work).is_err()
            || std::fs::create_dir_all(&live).is_err()
            || infra_host::util::private_directory(&home).is_err()
        {
            return Outcome::Failed("The graph folder could not be prepared.".into());
        }
        let _kept = Kept::of(root);
        let ran = match self.run(tool, root, &work, job) {
            Ok(ran) => ran,
            Err(outcome) => return outcome,
        };
        if std::fs::rename(&ran.fresh, live.join("graph.json")).is_err() {
            return Outcome::Failed("The new graph could not be published.".into());
        }
        Outcome::Built(ran.record(size(&home)))
    }
    fn settle(
        &self,
        project: &str,
        job: &Job,
        outcome: Outcome,
        print: Option<Print>,
        tool: &Tool,
    ) {
        let purge = job.purge.load(Ordering::Acquire);
        if purge {
            let _ = std::fs::remove_dir_all(folder(&self.store.state, project));
        }
        let hash = print.as_ref().map(|print| print.hash.clone());
        let _ = self.edit(project, |setting| {
            if purge {
                *setting = json!({"enabled": false});
                return;
            }
            match outcome {
                Outcome::Built(mut built) => {
                    built["print"] = json!(hash);
                    built["commit"] = json!(print.as_ref().and_then(|print| print.commit.clone()));
                    built["tool"] = json!(tool.version);
                    built["program"] = json!(tool.path.to_string_lossy());
                    setting["built"] = built;
                    if let Some(fields) = setting.as_object_mut() {
                        fields.remove("failure");
                    }
                }
                Outcome::Failed(message) => {
                    setting["failure"] =
                        json!({"at": store::now(), "message": message, "print": hash});
                }
                Outcome::Cancelled => {}
            }
        });
        if let Ok(mut finished) = self.finished.lock() {
            finished.insert(project.to_owned(), Instant::now());
        }
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(project);
        }
        if job.again.load(Ordering::Acquire) && !purge {
            if let Ok(mut finished) = self.finished.lock() {
                finished.remove(project);
            }
            let _ = self.refresh(project, Trigger::Finish);
        }
    }
    /// The open pull requests as last listed, and listed now when they never
    /// were: what the current work is measured against must not depend on
    /// which request reaches the server first.
    fn listed(&self, project: &str, root: &Path) -> Vec<Pull> {
        if let Ok(pulls) = self.pulls.lock()
            && let Some((_, pulls, _)) = pulls.get(project)
        {
            return pulls.clone();
        }
        let listed = graph_changes::pulls(root);
        if let Ok(mut pulls) = self.pulls.lock() {
            pulls.insert(
                project.to_owned(),
                (Instant::now(), listed.0.clone(), listed.1.clone()),
            );
        }
        listed.0
    }
    /// What the checkout is on and the open pull requests of its repository.
    /// Asking the forge takes a moment, so callers keep this off the runtime.
    pub fn pulls(&self, project: &str) -> Result<Value> {
        let root = self.root(project)?;
        let mut place = graph_changes::place(&root)?;
        if place["git"] != true {
            return Ok(place);
        }
        let known = lock(&self.pulls)?
            .get(project)
            .filter(|(at, ..)| at.elapsed() < PULLS_AGE)
            .map(|(_, pulls, reason)| (pulls.clone(), reason.clone()));
        let (pulls, reason) = match known {
            Some(known) => known,
            None => {
                let listed = graph_changes::pulls(&root);
                lock(&self.pulls)?.insert(
                    project.to_owned(),
                    (Instant::now(), listed.0.clone(), listed.1.clone()),
                );
                listed
            }
        };
        // The request the current branch belongs to names what it is stacked on.
        let own = pulls
            .iter()
            .find(|pull| place["branch"].as_str() == Some(pull.head.as_str()));
        place["pull"] = json!(own.map(|pull| pull.number));
        place["against"] = json!(own.map(|pull| pull.base.clone()));
        place["pulls"] = pulls.iter().map(Pull::json).collect();
        place["reason"] = json!(reason);
        Ok(place)
    }
    /// Pull requests in one state, narrowed by the forge's own search, for
    /// the picker. The forge is asked again after a minute.
    pub fn found(&self, project: &str, state: &str, query: &str) -> Result<Value> {
        let root = self.root(project)?;
        let key = (project.to_owned(), state.to_owned(), query.to_owned());
        let known = lock(&self.searches)?
            .get(&key)
            .filter(|(at, ..)| at.elapsed() < PULLS_AGE)
            .map(|(_, pulls, reason)| (pulls.clone(), reason.clone()));
        let (pulls, reason) = match known {
            Some(known) => known,
            None => {
                let listed = graph_changes::search(&root, state, query);
                let mut searches = lock(&self.searches)?;
                searches.retain(|_, (at, ..)| at.elapsed() < PULLS_AGE);
                searches.insert(key, (Instant::now(), listed.0.clone(), listed.1.clone()));
                listed
            }
        };
        Ok(json!({
            "pulls": pulls.iter().map(Pull::json).collect::<Vec<_>>(),
            "reason": reason,
        }))
    }
    /// A page of the project's history for the picker.
    pub fn commits(&self, project: &str, scope: &str, query: &str, skip: usize) -> Result<Value> {
        let (commits, more) = graph_changes::log(&self.root(project)?, scope, query, skip)?;
        Ok(json!({"commits": commits, "more": more, "skip": skip}))
    }
    /// The project's branches for the picker.
    pub fn branches(&self, project: &str) -> Result<Value> {
        let root = self.root(project)?;
        Ok(json!({
            "branches": graph_changes::branches(&root, &self.listed(project, &root))?,
        }))
    }
    /// How long builds of this project take, for the estimate of a commit's
    /// graph: the last such build, else the project's own, which runs the
    /// same passes over nearly the same files.
    fn pace(&self, setting: &Value) -> Option<History> {
        let version = json!(self.tool(false).and_then(|tool| tool.version));
        [&setting["revision"], &setting["built"]]
            .into_iter()
            .find(|built| built["tool"] == version && built["seconds"].is_number())
            .and_then(|built| {
                Some(History {
                    seconds: built["seconds"].as_f64()?,
                    tail: built["tail"].as_f64(),
                })
            })
    }
    /// Whether the project's own graph is of this commit: it is when it was
    /// built from a clean checkout of it.
    fn current(&self, project: &str, setting: &Value, commit: &str) -> bool {
        live(&self.store.state, project).is_file()
            && setting["built"]["print"].as_str()
                == Some(store::sha(format!("{commit}\0").as_bytes()).as_str())
    }
    fn graph_of(&self, project: &str, root: &Path, resolved: &Resolved) -> Result<Found> {
        let ready = |kept: String, graph: PathBuf, base: PathBuf| Found {
            state: "ready",
            build: Value::Null,
            failure: None,
            graph: Some((kept, graph, base)),
        };
        let published = live(&self.store.state, project);
        let Some(commit) = &resolved.revision else {
            return Ok(ready(project.to_owned(), published, root.to_owned()));
        };
        let own = revision(&self.store.state, project, commit);
        if own.is_file() {
            let tree = own.with_file_name("tree");
            return Ok(ready(
                format!("{project}@{commit}"),
                own,
                if resolved.prefix.is_empty() {
                    tree
                } else {
                    tree.join(&resolved.prefix)
                },
            ));
        }
        let setting = self.setting(project)?;
        if self.current(project, &setting, commit) {
            return Ok(ready(project.to_owned(), published, root.to_owned()));
        }
        let waiting = |state, build, failure| Found {
            state,
            build,
            failure,
            graph: None,
        };
        if let Some((building, job)) = lock(&self.revisions)?.get(project)
            && building == commit
        {
            return Ok(waiting("building", job.view(self.pace(&setting)), None));
        }
        Ok(
            match lock(&self.refused)?.get(&(project.to_owned(), commit.clone())) {
                Some(message) => waiting("failed", Value::Null, Some(message.clone())),
                None => waiting("missing", Value::Null, None),
            },
        )
    }
    /// Works out what a target changed and, for a commit that is not the
    /// checkout, starts building its graph. A pull request is looked up and
    /// fetched here, so callers keep this off the runtime.
    pub fn choose(&self, project: &str, target: &Target) -> Result<Value> {
        let root = self.root(project)?;
        let resolved = Arc::new(graph_changes::resolve(
            &root,
            target,
            &self.listed(project, &root),
        )?);
        lock(&self.chosen)?.insert((project.to_owned(), target.text()), resolved.clone());
        if let Some(commit) = &resolved.revision {
            self.revise(project, &root, commit, &resolved.prefix)?;
        }
        let found = self.graph_of(project, &root, &resolved)?;
        Ok(json!({
            "project_id": project,
            "target": target.text(),
            "chosen": chosen(&resolved),
            "state": found.state,
            "build": found.build,
            "failure": found.failure,
        }))
    }
    /// Starts the graph of a commit unless it exists or is being built. A
    /// build for another commit of the project gives way: the last one asked
    /// for is the one being looked at.
    fn revise(&self, project: &str, root: &Path, commit: &str, prefix: &str) -> Result<()> {
        if commit.len() < 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(HarnessError::invalid("That is not a commit."));
        }
        if self.stopping.load(Ordering::Acquire)
            || revision(&self.store.state, project, commit).is_file()
            || self.current(project, &self.setting(project)?, commit)
        {
            return Ok(());
        }
        let tool = self.tool(true).ok_or_else(|| {
            HarnessError::unavailable(
                "Graphify is not installed. Install it to build the graph of that commit.",
            )
        })?;
        let Some(me) = self.me.upgrade() else {
            return Ok(());
        };
        let job = Arc::new(Job::new());
        job.stage.store(WAITING, Ordering::Release);
        {
            let mut revisions = lock(&self.revisions)?;
            if let Some((building, running)) = revisions.get(project) {
                if building == commit {
                    return Ok(());
                }
                running.cancel.store(true, Ordering::Release);
            }
            revisions.insert(project.to_owned(), (commit.to_owned(), job.clone()));
        }
        lock(&self.refused)?.remove(&(project.to_owned(), commit.to_owned()));
        let (name, root, commit, prefix) = (
            project.to_owned(),
            root.to_owned(),
            commit.to_owned(),
            prefix.to_owned(),
        );
        let running = job.clone();
        let worker = std::thread::Builder::new()
            .name("harness-graph-revision".into())
            .spawn(move || {
                let outcome = me.grow(&name, &root, &tool, &commit, &prefix, &running);
                me.finish(&name, &commit, &running, outcome, &tool);
            });
        match worker {
            Ok(handle) => {
                let mut workers = lock(&self.workers)?;
                workers.retain(|worker| !worker.is_finished());
                workers.push(handle);
                Ok(())
            }
            Err(_) => {
                let mut revisions = lock(&self.revisions)?;
                if revisions
                    .get(project)
                    .is_some_and(|(_, running)| Arc::ptr_eq(running, &job))
                {
                    revisions.remove(project);
                }
                Err(HarnessError::unavailable(
                    "The code graph build could not start.",
                ))
            }
        }
    }
    /// Builds the graph of one commit: its files are written out beside the
    /// project's graph, the tool reads them as it reads the project, and only
    /// the graph is kept.
    fn grow(
        &self,
        project: &str,
        root: &Path,
        tool: &Tool,
        commit: &str,
        prefix: &str,
        job: &Arc<Job>,
    ) -> Outcome {
        let stopped =
            || job.cancel.load(Ordering::Acquire) || self.stopping.load(Ordering::Acquire);
        // One graph is built for a project at a time: two runs of the tool
        // take twice the memory.
        loop {
            if stopped() {
                return Outcome::Cancelled;
            }
            match self.jobs.lock() {
                Ok(jobs) if !jobs.contains_key(project) => break,
                _ => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        let own = revision(&self.store.state, project, commit);
        let home = own.with_file_name("");
        let (tree, out) = (own.with_file_name("tree"), own.with_file_name("work"));
        let _ = std::fs::remove_dir_all(&home);
        if std::fs::create_dir_all(&out).is_err()
            || infra_host::util::private_directory(&folder(&self.store.state, project)).is_err()
        {
            return Outcome::Failed("The graph folder could not be prepared.".into());
        }
        job.stage.store(EXPORTING, Ordering::Release);
        let outcome = (|| {
            if graph_changes::export(root, commit, &tree).is_err() {
                return Outcome::Failed("Git could not write out the files of that commit.".into());
            }
            let source = if prefix.is_empty() {
                tree.clone()
            } else {
                tree.join(prefix)
            };
            if !source.is_dir() {
                return Outcome::Failed("That commit does not have this project's folder.".into());
            }
            if stopped() {
                return Outcome::Cancelled;
            }
            job.restart();
            job.stage.store(0, Ordering::Release);
            let ran = match self.run(tool, &source, &out, job) {
                Ok(ran) => ran,
                Err(outcome) => return outcome,
            };
            if std::fs::rename(&ran.fresh, &own).is_err() {
                return Outcome::Failed("The new graph could not be published.".into());
            }
            Outcome::Built(ran.record(ran.bytes))
        })();
        // The files and the tool's cache are as large as the project.
        let _ = std::fs::remove_dir_all(&tree);
        let _ = std::fs::remove_dir_all(&out);
        if !matches!(outcome, Outcome::Built(_)) {
            let _ = std::fs::remove_dir_all(&home);
        }
        outcome
    }
    fn finish(&self, project: &str, commit: &str, job: &Arc<Job>, outcome: Outcome, tool: &Tool) {
        let purge = job.purge.load(Ordering::Acquire);
        if purge {
            let _ = std::fs::remove_dir_all(folder(&self.store.state, project));
        }
        match outcome {
            Outcome::Built(built) if !purge => {
                let _ = self.edit(project, |setting| {
                    setting["revision"] = json!({
                        "seconds": built["seconds"],
                        "tail": built["tail"],
                        "tool": tool.version,
                    });
                });
                self.prune(project, commit);
            }
            Outcome::Failed(message) => {
                if let Ok(mut refused) = self.refused.lock() {
                    refused.insert((project.to_owned(), commit.to_owned()), message);
                }
            }
            _ => {}
        }
        if let Ok(mut revisions) = self.revisions.lock()
            && revisions
                .get(project)
                .is_some_and(|(_, running)| Arc::ptr_eq(running, job))
        {
            revisions.remove(project);
        }
    }
    /// Keeps the newest graphs of other commits and removes the rest.
    fn prune(&self, project: &str, keep: &str) {
        let home = folder(&self.store.state, project).join("revisions");
        let Ok(entries) = std::fs::read_dir(&home) else {
            return;
        };
        let mut built = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let changed = std::fs::metadata(entry.path().join("graph.json"))
                    .and_then(|meta| meta.modified())
                    .ok()?;
                Some((name == keep, changed, name))
            })
            .collect::<Vec<_>>();
        built.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        for (.., name) in built.into_iter().skip(REVISIONS) {
            let _ = std::fs::remove_dir_all(home.join(&name));
        }
    }
    /// The state the interface draws. `visit` also considers a rebuild, as when
    /// the project is opened.
    pub fn status(&self, project: &str, visit: bool) -> Result<Value> {
        let root = self.root(project)?;
        if visit {
            self.refresh(project, Trigger::Visit)?;
        }
        let setting = self.setting(project)?;
        let enabled = setting["enabled"] == true;
        let tool = self.tool(false);
        let job = lock(&self.jobs)?.get(project).cloned();
        let built = &setting["built"];
        let published = built.is_object() && live(&self.store.state, project).is_file();
        let history = built["seconds"]
            .as_f64()
            .filter(|_| {
                published
                    && built["tool"] == json!(tool.as_ref().and_then(|tool| tool.version.clone()))
            })
            .map(|seconds| History {
                seconds,
                tail: built["tail"].as_f64(),
            });
        let stale = enabled
            && job.is_none()
            && published
            && self
                .print(project, &root, false)
                .is_some_and(|now| built["print"].as_str() != Some(now.hash.as_str()));
        let state = if !enabled {
            "off"
        } else if job.is_some() {
            "building"
        } else if tool.is_none() {
            "missing"
        } else if setting["failure"].is_object() {
            "failed"
        } else if !published {
            "empty"
        } else if stale {
            "stale"
        } else {
            "ready"
        };
        Ok(json!({
            "project_id": project,
            "enabled": enabled,
            // Whether launches are told about the graph: a choice of its own.
            "agents": setting["agents"] == true,
            "state": state,
            "tool": {
                "available": tool.is_some(),
                "version": tool.as_ref().and_then(|tool| tool.version.clone()),
                "path": tool.as_ref().map(|tool| tool.path.to_string_lossy().into_owned()),
            },
            "graph": if published {
                json!({
                    "commit": built["commit"],
                    "built_at": built["at"],
                    "nodes": built["nodes"],
                    "edges": built["edges"],
                    "bytes": built["bytes"],
                    "disk": built["disk"],
                    "seconds": built["seconds"],
                })
            } else {
                Value::Null
            },
            "build": job.map(|job| job.view(history)),
            "failure": if setting["failure"].is_object() {
                json!({"message": setting["failure"]["message"], "at": setting["failure"]["at"]})
            } else {
                Value::Null
            },
        }))
    }
    pub fn act(&self, project: &str, action: &str) -> Result<Value> {
        self.root(project)?;
        match action {
            "enable" => {
                self.edit(project, |setting| setting["enabled"] = json!(true))?;
                self.refresh(project, Trigger::Enabled)?;
            }
            "disable" => {
                self.edit(project, |setting| setting["enabled"] = json!(false))?;
                if let Some(job) = lock(&self.jobs)?.get(project) {
                    job.cancel.store(true, Ordering::Release);
                }
            }
            // Telling agents about the graph is asked for on its own: the
            // map needs none of it, and it costs tokens on every launch.
            "offer" => {
                if self.setting(project)?["enabled"] != true {
                    return Err(HarnessError::invalid(
                        "Turn the code graph on before telling agents about it.",
                    ));
                }
                self.edit(project, |setting| setting["agents"] = json!(true))?;
            }
            "withhold" => {
                self.edit(project, |setting| setting["agents"] = json!(false))?;
            }
            "rebuild" => {
                if self.setting(project)?["enabled"] != true {
                    return Err(HarnessError::invalid(
                        "Turn the code graph on before rebuilding it.",
                    ));
                }
                if self.tool(true).is_none() {
                    return Err(HarnessError::unavailable(
                        "Graphify is not installed. Install it, then rebuild.",
                    ));
                }
                self.edit(project, |setting| {
                    if let Some(fields) = setting.as_object_mut() {
                        fields.remove("failure");
                    }
                })?;
                self.refresh(project, Trigger::Manual)?;
            }
            "delete" => {
                self.edit(project, |setting| *setting = json!({"enabled": false}))?;
                let running = lock(&self.jobs)?.get(project).cloned();
                match running {
                    // The build owns the folder until it has ended.
                    Some(job) => {
                        job.purge.store(true, Ordering::Release);
                        job.cancel.store(true, Ordering::Release);
                    }
                    None => {
                        let home = folder(&self.store.state, project);
                        if home.exists() {
                            std::fs::remove_dir_all(home)?;
                        }
                    }
                }
                // A commit's graph that is still being built removes the
                // folder again when its build has ended.
                if let Some((_, job)) = lock(&self.revisions)?.get(project) {
                    job.purge.store(true, Ordering::Release);
                    job.cancel.store(true, Ordering::Release);
                }
                lock(&self.chosen)?.retain(|(name, _), _| name != project);
                lock(&self.refused)?.retain(|(name, _), _| name != project);
                self.maps.forget(project);
            }
            _ => {
                return Err(HarnessError::invalid(
                    "Choose enable, disable, offer, withhold, rebuild or delete.",
                ));
            }
        }
        self.status(project, false)
    }
    /// Ends every build; the tool and its workers do not outlive the Harness.
    pub fn close(&self) {
        self.stopping.store(true, Ordering::Release);
        let workers = match self.workers.lock() {
            Ok(mut workers) => std::mem::take(&mut *workers),
            Err(_) => return,
        };
        for worker in workers {
            let _ = worker.join();
        }
    }
}
impl Domain for Graphs {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let project = request["params"]["project_id"].as_str().unwrap_or("");
            let value = match operation {
                "graph.status" => {
                    let visit = match request["query"]["check"].as_str() {
                        None | Some("0") => false,
                        Some("1") => true,
                        Some(_) => return Err(HarnessError::invalid("Invalid graph check.")),
                    };
                    self.status(project, visit)?
                }
                "graph.act" => {
                    let body = &request["body"];
                    if !store::exact(body, &["action"], &["target"]) {
                        return Err(HarnessError::invalid("Invalid code graph action."));
                    }
                    match (body["action"].as_str().unwrap_or(""), body.get("target")) {
                        ("changes", Some(target)) => {
                            let target = Target::parse(target.as_str().unwrap_or(""))?;
                            let me = self.me.upgrade().ok_or_else(|| {
                                HarnessError::unavailable("The code graph is shutting down.")
                            })?;
                            let project = project.to_owned();
                            tokio::task::spawn_blocking(move || me.choose(&project, &target))
                                .await
                                .map_err(|_| {
                                    HarnessError::unavailable("The change could not be read.")
                                })??
                        }
                        (action, None) if action != "changes" => self.act(project, action)?,
                        _ => return Err(HarnessError::invalid("Invalid code graph action.")),
                    }
                }
                "graph.pulls" | "graph.commits" | "graph.branches" => {
                    let query = &request["query"];
                    // What a person typed into a search: short, one line.
                    let typed = |name: &str| {
                        let value = query[name].as_str().unwrap_or("");
                        (value.len() <= 200 && !value.chars().any(char::is_control))
                            .then(|| value.to_owned())
                            .ok_or_else(|| HarnessError::invalid("Invalid search."))
                    };
                    let (state, wanted, scope) =
                        (typed("state")?, typed("query")?, typed("scope")?);
                    if !["", "open", "closed", "merged", "all"].contains(&state.as_str())
                        || scope.starts_with('-')
                        || scope.chars().any(char::is_whitespace)
                    {
                        return Err(HarnessError::invalid("Invalid search."));
                    }
                    let skip = match query["skip"].as_str() {
                        None | Some("") => 0,
                        Some(text) => text
                            .parse::<usize>()
                            .ok()
                            .filter(|skip| *skip <= 1_000_000)
                            .ok_or_else(|| HarnessError::invalid("Invalid search."))?,
                    };
                    let me = self.me.upgrade().ok_or_else(|| {
                        HarnessError::unavailable("The code graph is shutting down.")
                    })?;
                    let (project, operation) = (project.to_owned(), operation.to_owned());
                    tokio::task::spawn_blocking(move || match operation.as_str() {
                        "graph.commits" => me.commits(&project, &scope, &wanted, skip),
                        "graph.branches" => me.branches(&project),
                        // The open requests, unsearched, are the list the
                        // checkout's own work is measured with.
                        _ if wanted.is_empty() && ["", "open"].contains(&state.as_str()) => {
                            me.pulls(&project)
                        }
                        _ => me.found(&project, &state, &wanted),
                    })
                    .await
                    .map_err(|_| HarnessError::unavailable("The list could not be read."))??
                }
                "graph.map" => {
                    let query = &request["query"];
                    let text = |name: &str| {
                        let value = query[name].as_str().unwrap_or("");
                        (value.len() <= 1024)
                            .then(|| value.to_owned())
                            .ok_or_else(|| HarnessError::invalid("Invalid code graph place."))
                    };
                    let (scope, focus, filter) = (text("scope")?, text("focus")?, text("filter")?);
                    let certain = match query["certain"].as_str() {
                        None | Some("0") => false,
                        Some("1") => true,
                        Some(_) => return Err(HarnessError::invalid("Invalid graph filter.")),
                    };
                    let target = match query["target"].as_str() {
                        None | Some("") => None,
                        Some(text) => Some(Target::parse(text)?),
                    };
                    let me = self.me.upgrade().ok_or_else(|| {
                        HarnessError::unavailable("The code graph is shutting down.")
                    })?;
                    let project = project.to_owned();
                    tokio::task::spawn_blocking(move || {
                        let focus = (!focus.is_empty()).then_some(focus.as_str());
                        me.map(&project, target.as_ref(), &scope, focus, certain, &filter)
                    })
                    .await
                    .map_err(|_| {
                        HarnessError::unavailable("The code graph map could not be read.")
                    })??
                }
                _ => return Err(HarnessError::missing("Unknown code graph operation.")),
            };
            Ok(Payload::Json(value))
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_lines_of_both_releases_are_read() {
        assert_eq!(
            signal("  AST extraction: 500/994 uncached files (50%) [14 workers]"),
            Some(Signal::Reading {
                done: 500,
                total: 994
            })
        );
        assert_eq!(
            signal("  AST extraction: 1000/1000 files (100%)"),
            Some(Signal::Reading {
                done: 1000,
                total: 1000
            })
        );
        assert_eq!(
            signal("  AST cache: 3514/3520 files served from cache (99%); 6 to extract"),
            Some(Signal::Pending(6))
        );
        assert_eq!(
            signal("  AST cache: 12/12 files served from cache (100%); 0 to extract"),
            Some(Signal::Pending(0))
        );
        assert_eq!(
            signal("[graphify watch] Rebuilt: 22546 nodes, 50403 edges, 1053 communities"),
            Some(Signal::Built {
                nodes: 22546,
                edges: 50403
            })
        );
        assert_eq!(
            signal("[graphify watch] Rebuilt (no clustering): 50900 nodes, 102253 edges"),
            Some(Signal::Built {
                nodes: 50900,
                edges: 102253
            })
        );
        for other in [
            "",
            "Updating code graph in /repo (no LLM needed)...",
            "AST extraction: many/files",
            "AST cache: warming",
            "Rebuilt: nothing",
            "[graphify watch] No code-graph topology changes detected; outputs left untouched.",
        ] {
            assert_eq!(signal(other), None, "{other}");
        }
    }
    fn reading(points: &[(f64, u64)], total: u64) -> Observed {
        let mut seen = Observed::default();
        for (at, done) in points {
            seen.note(*at, Signal::Reading { done: *done, total });
        }
        seen
    }
    #[test]
    fn a_first_build_has_no_estimate_until_two_counts_were_seen() {
        assert_eq!(expected(2.0, &Observed::default(), None), None);
        assert_eq!(expected(3.2, &reading(&[(3.1, 100)], 3520), None), None);
        assert_eq!(
            progress(3.2, &reading(&[(3.1, 100)], 3520), None, 0),
            (None, None, false)
        );
    }
    #[test]
    fn a_first_build_extends_the_reading_rate_and_adds_the_silent_part() {
        // 100 files in 0.2 s: the other 3,320 need 6.64 s more, ending at 9.94 s.
        let seen = reading(&[(3.1, 100), (3.3, 200)], 3520);
        let total = expected(3.3, &seen, None).unwrap();
        assert!((total - 9.94 * (1.0 + TAIL_RATIO)).abs() < 0.01, "{total}");
        let (percent, left, overdue) = progress(3.3, &seen, None, 0);
        assert_eq!((percent, overdue), (Some(6), false));
        assert_eq!(left, Some((total - 3.3).ceil() as u64));
    }
    #[test]
    fn the_silent_part_comes_from_the_projects_previous_build() {
        let past = Some(History {
            seconds: 44.0,
            tail: Some(38.0),
        });
        let mut seen = reading(&[(3.1, 100), (9.3, 3520)], 3520);
        assert_eq!(expected(20.0, &seen, past), Some(9.3 + 38.0));
        assert_eq!(progress(20.0, &seen, past, 0), (Some(42), Some(28), false));
        // Nothing left to read: the cache line ends the measured part.
        seen = Observed::default();
        seen.note(6.4, Signal::Pending(0));
        assert_eq!(expected(7.0, &seen, past), Some(6.4 + 38.0));
        // A build that printed no count at all is as long as the previous one.
        assert_eq!(expected(5.0, &Observed::default(), past), Some(44.0));
        // A previous build without a measured tail bounds the silent part.
        let whole = Some(History {
            seconds: 14.0,
            tail: None,
        });
        assert_eq!(
            expected(1.0, &reading(&[(0.9, 3)], 3), whole),
            Some(0.9 + 14.0)
        );
    }
    #[test]
    fn percent_never_falls_and_stops_short_of_done_when_overdue() {
        let past = Some(History {
            seconds: 10.0,
            tail: None,
        });
        let seen = Observed::default();
        assert_eq!(progress(5.0, &seen, past, 0), (Some(50), Some(5), false));
        assert_eq!(progress(5.0, &seen, past, 70), (Some(70), Some(5), false));
        assert_eq!(progress(12.0, &seen, past, 70), (Some(99), Some(0), true));
    }
    #[test]
    fn a_graph_question_is_told_from_other_commands() {
        for command in [
            "graphify query \"where is the session started\" --graph /state/graphs/ab/live/graph.json",
            "cd src && /home/u/.local/bin/graphify explain \"Sessions\" --graph g.json",
            "'/opt/my tools/graphify' path A B --graph g.json | head -40",
        ] {
            assert!(asked(command), "{command}");
        }
        for command in [
            "",
            "graphify update .",
            "rg graphify query",
            "echo graphify",
            "cat graphify-out/GRAPH_REPORT.md",
        ] {
            assert!(!asked(command), "{command}");
        }
    }
    /// The prompt shows a path inside a command, and Claude Code compares its
    /// permission with the command as written, so the spelling has to survive
    /// the shell the agent uses.
    #[test]
    fn a_path_is_one_shell_word_on_either_system() {
        assert_eq!(
            spelled("/home/me/.state/graph.json", false),
            "/home/me/.state/graph.json"
        );
        assert_eq!(
            spelled("/home/me/my state/it's.json", false),
            "'/home/me/my state/it'\\''s.json'"
        );
        assert_eq!(spelled("/tmp/a\\b", false), "'/tmp/a\\b'");
        assert_eq!(
            spelled("C:\\Users\\me\\AppData\\graph.json", true),
            "C:/Users/me/AppData/graph.json"
        );
        assert_eq!(
            spelled("C:\\Users\\Ann Lee\\graph.json", true),
            "\"C:/Users/Ann Lee/graph.json\""
        );
        assert_eq!(
            spelled("\\\\server\\share\\graph.json", true),
            "\"\\\\server\\share\\graph.json\""
        );
    }
    /// The launch of a session whose project has a published graph: Claude
    /// Code gets the three rules beside the settings it already had, every
    /// host gets the switch that keeps a question from rewriting skills.
    #[test]
    fn a_launch_carries_the_permission_for_claude_and_the_switch_for_any_host() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let folder = root.path().join("project");
        std::fs::create_dir_all(&folder).unwrap();
        let project = store.register(&folder).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let command = |argv: &[&str]| infra_host::provider::PreparedCommand {
            argv: argv.iter().map(|word| (*word).to_owned()).collect(),
            environment: std::collections::BTreeMap::from([("KEPT".to_owned(), "1".to_owned())]),
            input: vec![],
            readable: vec![],
            remove: vec![],
        };
        let before =
            r#"{"env":{"A":"1"},"permissions":{"allow":["Bash(php:*)"],"deny":["Read(.env)"]}}"#;
        // No graph: the launch is left as it was.
        let mut untouched = command(&["claude", "--settings", before, "--resume", "id"]);
        prepare(&store, &project, "claude", &mut untouched).unwrap();
        assert_eq!(untouched.argv[2], before);
        assert_eq!(untouched.environment.len(), 1);

        let tool = root.path().join("graphify-tool");
        publish_for_tests(&store, &project, &tool);
        let rules = grant(&store, &project).unwrap().unwrap().rules;
        assert_eq!(rules.len(), 3);
        let mut claude = command(&["claude", "--settings", before, "--resume", "id"]);
        prepare(&store, &project, "claude", &mut claude).unwrap();
        let settings: Value = serde_json::from_str(&claude.argv[2]).unwrap();
        let mut allow = vec![json!("Bash(php:*)")];
        allow.extend(rules.iter().map(|rule| json!(rule)));
        assert_eq!(settings["permissions"]["allow"], Value::Array(allow));
        assert_eq!(settings["permissions"]["deny"], json!(["Read(.env)"]));
        assert_eq!(settings["env"], json!({"A": "1"}));
        assert_eq!(claude.argv[3..], ["--resume", "id"]);
        assert_eq!(claude.environment["GRAPHIFY_NO_AUTO_REFRESH"], "1");
        assert_eq!(claude.environment["KEPT"], "1");
        // The same launch prepared again repeats no rule.
        prepare(&store, &project, "claude", &mut claude).unwrap();
        let again: Value = serde_json::from_str(&claude.argv[2]).unwrap();
        assert_eq!(again, settings);
        // A Claude command without settings gets its own.
        let mut bare = command(&["claude", "--print"]);
        prepare(&store, &project, "claude", &mut bare).unwrap();
        assert_eq!(bare.argv[2], "--settings");
        assert_eq!(
            serde_json::from_str::<Value>(&bare.argv[3]).unwrap()["permissions"]["allow"],
            json!(rules)
        );
        // Codex runs what its sandbox allows: no rule, the switch only.
        let mut codex = command(&["codex", "exec"]);
        prepare(&store, &project, "codex", &mut codex).unwrap();
        assert_eq!(codex.argv, ["codex", "exec"]);
        assert_eq!(codex.environment["GRAPHIFY_NO_AUTO_REFRESH"], "1");
        // Settings that are not an object are refused, not replaced.
        let mut broken = command(&["claude", "--settings", "[]"]);
        assert!(prepare(&store, &project, "claude", &mut broken).is_err());
    }
    #[test]
    fn a_release_is_found_in_version_output() {
        assert_eq!(release("graphify 0.9.105\n").as_deref(), Some("0.9.105"));
        assert_eq!(release("0.7.13\n").as_deref(), Some("0.7.13"));
        assert_eq!(release("error: unknown command '--version'"), None);
    }
    #[cfg(unix)]
    mod builds {
        use super::super::*;
        use std::os::unix::fs::PermissionsExt;
        const TOOL: &str = r#"#!/bin/sh
here=$(dirname "$0")
if [ "$GRAPHIFY_NO_AUTO_REFRESH" != "1" ]; then echo "installed skills would be refreshed" >&2; exit 6; fi
if [ "$1" = "--version" ]; then echo "graphify 9.9.9"; exit 0; fi
echo run >> "$here/runs"
if [ -e "$GRAPHIFY_OUT/graph.json" ]; then echo "previous graph still present" >&2; exit 4; fi
if [ -n "$OPENAI_API_KEY" ]; then echo "provider key leaked" >&2; exit 5; fi
echo "  AST extraction: 1/2 uncached files (50%) [2 workers]"
if [ -e "$2/old" ]; then mkdir -p "$2/graphify-out"; echo overwritten > "$2/graphify-out/manifest.json"; fi
if [ -e "$2/slow" ]; then sleep 30; fi
if [ -e "$2/fail" ]; then echo "boom" >&2; exit 3; fi
echo "  AST extraction: 2/2 uncached files (100%) [2 workers]"
mkdir -p "$GRAPHIFY_OUT/cache"
if [ -e "$2/graph.fixture" ]; then cp "$2/graph.fixture" "$GRAPHIFY_OUT/graph.json"; else printf '{"nodes":[],"links":[]}' > "$GRAPHIFY_OUT/graph.json"; fi
echo "[graphify watch] Rebuilt: 7 nodes, 9 edges, 2 communities"
"#;
        struct Fixture {
            _root: tempfile::TempDir,
            store: SessionStore,
            graphs: Arc<Graphs>,
            project: String,
            folder: PathBuf,
            tool: PathBuf,
        }
        impl Fixture {
            fn new() -> Self {
                let root = tempfile::tempdir().unwrap();
                let store = SessionStore::open(&root.path().join("state")).unwrap();
                let folder = root.path().join("project");
                std::fs::create_dir_all(&folder).unwrap();
                let project = store.register(&folder).unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let tool = root.path().join("tool").join("graphify");
                std::fs::create_dir_all(tool.parent().unwrap()).unwrap();
                std::fs::write(&tool, TOOL).unwrap();
                std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
                let graphs = Graphs::with(store.clone(), Some(tool.clone()), Duration::ZERO);
                Self {
                    _root: root,
                    store,
                    graphs,
                    project,
                    folder,
                    tool,
                }
            }
            fn runs(&self) -> usize {
                std::fs::read_to_string(self.tool.with_file_name("runs"))
                    .map(|text| text.lines().count())
                    .unwrap_or(0)
            }
            fn settled(&self) -> Value {
                let started = Instant::now();
                loop {
                    let status = self.graphs.status(&self.project, false).unwrap();
                    // A build that was switched off is still ending its tool.
                    if status["build"].is_null() {
                        return status;
                    }
                    assert!(started.elapsed() < Duration::from_secs(20), "{status}");
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            fn git(&self, arguments: &[&str]) {
                let done = Command::new("git")
                    .args(arguments)
                    .current_dir(&self.folder)
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_AUTHOR_NAME", "t")
                    .env("GIT_AUTHOR_EMAIL", "t@example.test")
                    .env("GIT_COMMITTER_NAME", "t")
                    .env("GIT_COMMITTER_EMAIL", "t@example.test")
                    .output()
                    .unwrap();
                assert!(done.status.success(), "{arguments:?}: {done:?}");
            }
            fn said(&self, arguments: &[&str]) -> String {
                let done = Command::new("git")
                    .args(arguments)
                    .current_dir(&self.folder)
                    .output()
                    .unwrap();
                assert!(done.status.success(), "{arguments:?}: {done:?}");
                String::from_utf8_lossy(&done.stdout).trim().to_owned()
            }
            fn write(&self, name: &str, text: &str) {
                std::fs::write(self.folder.join(name), text).unwrap();
            }
            fn commit(&self, message: &str) -> String {
                self.git(&["add", "--all"]);
                self.git(&["-c", "commit.gpgsign=false", "commit", "-q", "-m", message]);
                self.said(&["rev-parse", "HEAD"])
            }
            /// `main` with a cart that uses a price, `origin/main` standing
            /// there, and a `feature` branch whose order uses the cart. Each
            /// commit carries the graph the tool gives for its files. Returns
            /// both commits; `main` is checked out.
            fn branched(&self) -> (String, String) {
                self.git(&["init", "-q", "--initial-branch=main"]);
                self.write("cart.php", "<?php // cart\n");
                self.write("price.php", "<?php // price\n");
                self.write(
                    "graph.fixture",
                    r#"{"nodes":[{"id":"cart","source_file":"cart.php"},{"id":"price","source_file":"price.php"}],"links":[{"source":"cart","target":"price","relation":"calls","source_file":"cart.php"}]}"#,
                );
                let main = self.commit("one");
                self.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
                self.git(&["switch", "-q", "-c", "feature"]);
                self.write("cart.php", "<?php // cart, changed\n");
                self.write("order.php", "<?php // order\n");
                self.write(
                    "graph.fixture",
                    r#"{"nodes":[{"id":"cart","source_file":"cart.php"},{"id":"price","source_file":"price.php"},{"id":"order","source_file":"order.php"}],"links":[{"source":"cart","target":"price","relation":"calls","source_file":"cart.php"},{"source":"order","target":"cart","relation":"calls","source_file":"order.php"}]}"#,
                );
                let feature = self.commit("two");
                self.git(&["switch", "-q", "main"]);
                (main, feature)
            }
            /// The map of a chosen change once its graph is built or refused.
            fn mapped(&self, target: &Target) -> Value {
                let started = Instant::now();
                loop {
                    let view = self
                        .graphs
                        .map(&self.project, Some(target), "", None, false, "")
                        .unwrap();
                    if view["state"] != "building" {
                        return view;
                    }
                    assert!(started.elapsed() < Duration::from_secs(20), "{view}");
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            fn revisions(&self) -> Vec<String> {
                let mut names =
                    std::fs::read_dir(folder(&self.store.state, &self.project).join("revisions"))
                        .map(|entries| {
                            entries
                                .flatten()
                                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                names.sort();
                names
            }
            fn session(&self, status: &str) {
                self.store
                    .connection()
                    .unwrap()
                    .execute(
                        "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at)VALUES(?1,'t',?2,?3,'claude','edit','native',0,?4,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
                        rusqlite::params![store::id(), self.project, self.folder.to_string_lossy(), status],
                    )
                    .unwrap();
            }
        }
        #[test]
        fn switching_the_graph_on_builds_and_publishes_it() {
            let fixture = Fixture::new();
            let off = fixture.graphs.status(&fixture.project, true).unwrap();
            assert_eq!(off["state"], "off");
            assert_eq!(off["tool"]["version"], "9.9.9");
            assert_eq!(fixture.runs(), 0);
            let started = fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(started["state"], "building");
            let ready = fixture.settled();
            assert_eq!(ready["state"], "ready", "{ready}");
            assert_eq!(ready["graph"]["nodes"], 7);
            assert_eq!(ready["graph"]["edges"], 9);
            assert_eq!(ready["build"], Value::Null);
            let graph = live(&fixture.store.state, &fixture.project);
            assert!(graph.is_file());
            // The working folder keeps the cache and gives up its graph.
            let work = graph.parent().unwrap().with_file_name("work");
            assert!(work.join("cache").is_dir());
            assert!(!work.join("graph.json").exists());
            // Outside Git only a request rebuilds; the build starts clean.
            fixture
                .graphs
                .refresh(&fixture.project, Trigger::Visit)
                .unwrap();
            fixture
                .graphs
                .refresh(&fixture.project, Trigger::Finish)
                .unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            assert_eq!(fixture.runs(), 1);
            assert_eq!(
                fixture.graphs.act(&fixture.project, "rebuild").unwrap()["state"],
                "building"
            );
            assert_eq!(fixture.settled()["state"], "ready");
            assert_eq!(fixture.runs(), 2);
        }
        #[test]
        fn a_failed_build_keeps_the_published_graph_and_its_reason() {
            let fixture = Fixture::new();
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            std::fs::write(fixture.folder.join("fail"), "").unwrap();
            fixture.graphs.act(&fixture.project, "rebuild").unwrap();
            let failed = fixture.settled();
            assert_eq!(failed["state"], "failed", "{failed}");
            assert_eq!(failed["failure"]["message"], "boom");
            assert_eq!(failed["graph"]["nodes"], 7);
            assert!(live(&fixture.store.state, &fixture.project).is_file());
            // The same files are not tried again on their own.
            fixture
                .graphs
                .refresh(&fixture.project, Trigger::Finish)
                .unwrap();
            assert_eq!(fixture.settled()["state"], "failed");
            assert_eq!(fixture.runs(), 2);
            std::fs::remove_file(fixture.folder.join("fail")).unwrap();
            fixture.graphs.act(&fixture.project, "rebuild").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
        }
        #[test]
        fn deleting_ends_a_running_build_and_removes_the_folder() {
            let fixture = Fixture::new();
            std::fs::write(fixture.folder.join("slow"), "").unwrap();
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            let started = Instant::now();
            while fixture.graphs.status(&fixture.project, false).unwrap()["build"]["phase"]
                != "read"
            {
                assert!(started.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(20));
            }
            fixture.graphs.act(&fixture.project, "delete").unwrap();
            let off = fixture.settled();
            assert_eq!(off["state"], "off");
            assert_eq!(off["graph"], Value::Null);
            assert!(started.elapsed() < Duration::from_secs(10));
            assert!(!folder(&fixture.store.state, &fixture.project).exists());
        }
        #[test]
        fn closing_ends_a_running_build() {
            let fixture = Fixture::new();
            std::fs::write(fixture.folder.join("slow"), "").unwrap();
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            let started = Instant::now();
            fixture.graphs.close();
            assert!(started.elapsed() < Duration::from_secs(10));
            assert!(!live(&fixture.store.state, &fixture.project).exists());
        }
        #[test]
        fn a_changed_checkout_is_rebuilt_but_not_under_a_run() {
            let fixture = Fixture::new();
            fixture.git(&["init", "-q"]);
            std::fs::write(fixture.folder.join("a.rs"), "fn a() {}\n").unwrap();
            fixture.git(&["add", "a.rs"]);
            fixture.git(&["commit", "-q", "-m", "one"]);
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            let ready = fixture.settled();
            assert_eq!(ready["state"], "ready", "{ready}");
            assert_eq!(ready["graph"]["commit"].as_str().unwrap().len(), 12);
            fixture
                .graphs
                .refresh(&fixture.project, Trigger::Launch)
                .unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            assert_eq!(fixture.runs(), 1);
            std::fs::write(fixture.folder.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
            fixture.session("running");
            fixture
                .graphs
                .refresh(&fixture.project, Trigger::Visit)
                .unwrap();
            fixture
                .graphs
                .refresh(&fixture.project, Trigger::Finish)
                .unwrap();
            assert_eq!(fixture.settled()["state"], "stale");
            assert_eq!(fixture.runs(), 1);
            fixture
                .graphs
                .refresh(&fixture.project, Trigger::Launch)
                .unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            assert_eq!(fixture.runs(), 2);
        }
        fn drawn(view: &Value) -> Vec<String> {
            view["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| {
                    format!(
                        "{} {}{}",
                        node["path"].as_str().unwrap(),
                        node["role"].as_str().unwrap(),
                        node["status"]
                            .as_str()
                            .map(|status| format!(" {status}"))
                            .unwrap_or_default()
                    )
                })
                .collect()
        }
        #[test]
        fn a_branch_gets_the_graph_of_its_own_commit_and_the_checkout_is_left_alone() {
            let fixture = Fixture::new();
            let (main, feature) = fixture.branched();
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let target = Target::Named("feature".into());
            // A change is worked out before it is mapped.
            assert!(
                fixture
                    .graphs
                    .map(&fixture.project, Some(&target), "", None, false, "")
                    .is_err()
            );
            let before = fixture.said(&["status", "--porcelain=v1"]);
            let chosen = fixture.graphs.choose(&fixture.project, &target).unwrap();
            assert_eq!(chosen["chosen"]["kind"], "branch");
            assert_eq!(chosen["chosen"]["files"], 3);
            assert_eq!(
                chosen["chosen"]["head"],
                json!({"name": "feature", "commit": feature[..12]})
            );
            assert_eq!(
                chosen["chosen"]["base"],
                json!({"name": "origin/main", "commit": main[..12]})
            );
            let view = fixture.mapped(&target);
            assert_eq!(view["state"], "ready", "{view}");
            // The order exists only on the branch: the graph is of its files.
            assert_eq!(
                drawn(&view),
                [
                    "cart.php changed M",
                    "order.php changed A",
                    "price.php needed"
                ]
            );
            assert_eq!(view["change"]["files"], 3);
            assert_eq!(view["change"]["unknown"], 1);
            assert_eq!(fixture.revisions(), std::slice::from_ref(&feature));
            let kept = std::fs::read_dir(
                folder(&fixture.store.state, &fixture.project)
                    .join("revisions")
                    .join(&feature),
            )
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
            assert_eq!(kept, ["graph.json"]);
            assert_eq!(fixture.said(&["status", "--porcelain=v1"]), before);
            assert_eq!(fixture.said(&["rev-parse", "HEAD"]), main);
            assert_eq!(fixture.said(&["symbolic-ref", "--short", "HEAD"]), "main");
            assert_eq!(fixture.runs(), 2);
            // Asked again, the graph that is there is used.
            let again = fixture.graphs.choose(&fixture.project, &target).unwrap();
            assert_eq!(again["state"], "ready");
            // The project's own graph was built from a clean checkout of
            // this commit, so that commit needs no graph of its own.
            let own = fixture
                .graphs
                .choose(&fixture.project, &Target::Named(main))
                .unwrap();
            assert_eq!(own["state"], "ready");
            assert_eq!(fixture.runs(), 2);
            assert_eq!(fixture.revisions(), [feature]);
        }
        #[test]
        fn the_current_work_is_mapped_on_the_projects_own_graph() {
            let fixture = Fixture::new();
            fixture.branched();
            fixture.git(&["switch", "-q", "feature"]);
            fixture.write("price.php", "<?php // price, edited\n");
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let map = |target: &Target| {
                fixture
                    .graphs
                    .map(&fixture.project, Some(target), "", None, false, "")
                    .unwrap()
            };
            let work = map(&Target::Work);
            assert_eq!(work["state"], "ready");
            assert_eq!(
                (&work["stale"], &work["rebuilding"]),
                (&json!(false), &json!(false))
            );
            assert_eq!(work["chosen"]["kind"], "work");
            assert_eq!(work["chosen"]["name"], "feature");
            assert_eq!(work["chosen"]["base"]["name"], "origin/main");
            assert_eq!(work["chosen"]["files"], 4);
            assert_eq!(
                drawn(&work),
                [
                    "cart.php changed M",
                    "order.php changed A",
                    "price.php changed M"
                ]
            );
            // What is not committed: the price, and the cart that uses it.
            let uncommitted = map(&Target::Uncommitted);
            assert_eq!(uncommitted["chosen"]["files"], 1);
            assert_eq!(
                drawn(&uncommitted),
                ["price.php changed M", "cart.php affected"]
            );
            // A file of the change opens to what it holds and stays in the
            // change: the files around it carry what happened to them.
            let opened = fixture
                .graphs
                .map(
                    &fixture.project,
                    Some(&Target::Work),
                    "cart.php",
                    None,
                    false,
                    "",
                )
                .unwrap();
            assert_eq!(
                (
                    opened["mode"].as_str(),
                    opened["file"]["status"].as_str(),
                    opened["chosen"]["kind"].as_str(),
                    opened["state"].as_str()
                ),
                (Some("file"), Some("M"), Some("work"), Some("ready"))
            );
            let around = opened["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|node| node["symbol"] == false)
                .map(|node| {
                    format!(
                        "{} {} {}",
                        node["path"].as_str().unwrap(),
                        node["role"].as_str().unwrap(),
                        node["status"].as_str().unwrap_or("-")
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(around, ["order.php uses A", "price.php used M"]);
            // What the change did to the file is read with it. The graph of
            // this fixture gives its symbols no line, so a changed line
            // marks none of them, and a new file is new throughout.
            assert_eq!(
                opened["file"]["touched"],
                json!({"symbols": 0, "added": 0, "modified": 0, "between": 0, "top": false})
            );
            let added = fixture
                .graphs
                .map(
                    &fixture.project,
                    Some(&Target::Work),
                    "order.php",
                    None,
                    false,
                    "",
                )
                .unwrap();
            assert_eq!(
                (&added["file"]["status"], &added["file"]["touched"]["added"]),
                (&json!("A"), &json!(1))
            );
            // A file the change left alone has nothing to mark.
            let beside = fixture
                .graphs
                .map(
                    &fixture.project,
                    Some(&Target::Uncommitted),
                    "cart.php",
                    None,
                    false,
                    "",
                )
                .unwrap();
            assert_eq!(
                (&beside["mode"], &beside["file"]["touched"]),
                (&json!("file"), &Value::Null)
            );
            assert_eq!(fixture.runs(), 1);
            assert!(fixture.revisions().is_empty());
        }
        #[test]
        fn a_commit_whose_graph_fails_says_why_and_keeps_nothing() {
            let fixture = Fixture::new();
            fixture.branched();
            fixture.git(&["switch", "-q", "-c", "broken"]);
            fixture.write("fail", "");
            fixture.commit("broken");
            fixture.git(&["switch", "-q", "main"]);
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let target = Target::Named("broken".into());
            fixture.graphs.choose(&fixture.project, &target).unwrap();
            let view = fixture.mapped(&target);
            assert_eq!(view["state"], "failed", "{view}");
            assert_eq!(view["failure"], "boom");
            // What is drawn is the first picture, on the project's graph.
            assert_eq!(view["preliminary"], true);
            assert!(view["nodes"].is_array());
            assert!(fixture.revisions().is_empty());
            assert_eq!(fixture.settled()["state"], "ready");
            // Choosing it again tries again.
            fixture.graphs.choose(&fixture.project, &target).unwrap();
            assert_eq!(fixture.mapped(&target)["state"], "failed");
            assert_eq!(fixture.runs(), 3);
        }
        #[test]
        fn only_the_newest_graphs_of_other_commits_are_kept() {
            let fixture = Fixture::new();
            fixture.branched();
            fixture.git(&["switch", "-q", "-c", "stack"]);
            let mut commits = Vec::new();
            for number in 0..REVISIONS + 1 {
                fixture.write(&format!("step{number}.php"), "<?php\n");
                commits.push(fixture.commit("step"));
            }
            fixture.git(&["switch", "-q", "main"]);
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            for commit in &commits {
                let target = Target::Named(commit.clone());
                fixture.graphs.choose(&fixture.project, &target).unwrap();
                assert_eq!(fixture.mapped(&target)["state"], "ready");
                // File times on some systems move in coarse steps.
                std::thread::sleep(Duration::from_millis(20));
            }
            let mut newest = commits[1..].to_vec();
            newest.sort();
            assert_eq!(fixture.revisions(), newest);
        }
        #[test]
        fn a_picked_commit_is_first_drawn_on_the_projects_graph_while_its_own_is_built() {
            let fixture = Fixture::new();
            fixture.branched();
            fixture.git(&["switch", "-q", "-c", "long"]);
            fixture.write("slow", "");
            fixture.write("price.php", "<?php // price, changed\n");
            let long = fixture.commit("Change the price");
            fixture.git(&["switch", "-q", "main"]);
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let target = Target::Commit(long.clone());
            let chosen = fixture.graphs.choose(&fixture.project, &target).unwrap();
            assert_eq!(chosen["state"], "building");
            assert_eq!(chosen["target"], format!("commit:{long}"));
            assert_eq!(chosen["chosen"]["kind"], "commit");
            assert_eq!(chosen["chosen"]["title"], "Change the price");
            assert_eq!(chosen["chosen"]["name"], long[..12]);
            let view = fixture
                .graphs
                .map(&fixture.project, Some(&target), "", None, false, "")
                .unwrap();
            // The files are the commit's, the links the checkout's: the
            // cart uses the price in the project's own graph.
            assert_eq!(view["state"], "building");
            assert_eq!(view["preliminary"], true);
            assert_eq!(drawn(&view), ["price.php changed M", "cart.php affected"]);
            assert_eq!(view["change"]["files"], 2);
            // The history and the branches are read for the picker.
            let history = fixture
                .graphs
                .commits(&fixture.project, "long", "", 0)
                .unwrap();
            assert_eq!(history["commits"][0]["commit"], long);
            assert_eq!(history["commits"][0]["subject"], "Change the price");
            assert_eq!(history["more"], false);
            let branches = fixture.graphs.branches(&fixture.project).unwrap();
            let names = branches["branches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|branch| branch["name"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            assert!(names.contains(&"long".to_owned()) && names.contains(&"feature".to_owned()));
            fixture.graphs.act(&fixture.project, "delete").unwrap();
            let started = Instant::now();
            while !fixture.graphs.revisions.lock().unwrap().is_empty() {
                assert!(started.elapsed() < Duration::from_secs(20));
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        #[test]
        fn deleting_the_graph_ends_the_build_of_a_commit() {
            let fixture = Fixture::new();
            fixture.branched();
            fixture.git(&["switch", "-q", "-c", "long"]);
            fixture.write("slow", "");
            fixture.commit("long");
            fixture.git(&["switch", "-q", "main"]);
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let target = Target::Named("long".into());
            let chosen = fixture.graphs.choose(&fixture.project, &target).unwrap();
            assert_eq!(chosen["state"], "building");
            let off = fixture.graphs.act(&fixture.project, "delete").unwrap();
            assert_eq!(off["state"], "off");
            let started = Instant::now();
            while !fixture.graphs.revisions.lock().unwrap().is_empty() {
                assert!(started.elapsed() < Duration::from_secs(20));
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(!folder(&fixture.store.state, &fixture.project).exists());
            // What was chosen went with the graph.
            assert!(
                fixture
                    .graphs
                    .map(&fixture.project, Some(&target), "", None, false, "")
                    .is_err()
            );
        }
        #[test]
        fn an_old_release_leaves_the_projects_own_graph_folder_as_it_was() {
            let fixture = Fixture::new();
            std::fs::write(fixture.folder.join("old"), "").unwrap();
            let own = fixture.folder.join("graphify-out");
            // No folder of its own: the one the tool created is removed.
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            assert!(!own.exists());
            // The user's own graph: its manifest keeps its bytes and its time,
            // also when the build is cut short.
            std::fs::create_dir_all(&own).unwrap();
            std::fs::write(own.join("graph.json"), "mine").unwrap();
            std::fs::write(own.join("manifest.json"), "mine").unwrap();
            let before = std::fs::metadata(own.join("manifest.json"))
                .unwrap()
                .modified()
                .unwrap();
            for slow in [false, true] {
                if slow {
                    std::fs::write(fixture.folder.join("slow"), "").unwrap();
                }
                fixture.graphs.act(&fixture.project, "rebuild").unwrap();
                if slow {
                    let started = Instant::now();
                    while std::fs::read_to_string(own.join("manifest.json")).unwrap() == "mine" {
                        assert!(started.elapsed() < Duration::from_secs(10));
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    fixture.graphs.act(&fixture.project, "disable").unwrap();
                }
                fixture.settled();
                assert_eq!(
                    std::fs::read_to_string(own.join("manifest.json")).unwrap(),
                    "mine"
                );
                assert_eq!(
                    std::fs::metadata(own.join("manifest.json"))
                        .unwrap()
                        .modified()
                        .unwrap(),
                    before
                );
                assert_eq!(
                    std::fs::read_to_string(own.join("graph.json")).unwrap(),
                    "mine"
                );
            }
        }
        #[test]
        fn a_launch_is_told_about_a_published_graph_only_where_that_was_asked_for() {
            let fixture = Fixture::new();
            let session = json!({"project_id": fixture.project, "project_path": fixture.folder.to_string_lossy()});
            let silent = || {
                assert_eq!(prompt_line(&fixture.store, &session).unwrap(), None);
                assert!(grant(&fixture.store, &fixture.project).unwrap().is_none());
            };
            silent();
            // A graph that is off has nothing to tell agents about.
            assert!(fixture.graphs.act(&fixture.project, "offer").is_err());
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            let ready = fixture.settled();
            assert_eq!(
                (&ready["state"], &ready["agents"]),
                (&json!("ready"), &json!(false))
            );
            // The graph is built for the map; agents hear of it only when
            // the person asks for that, and no longer once that is undone.
            silent();
            assert_eq!(
                fixture.graphs.act(&fixture.project, "offer").unwrap()["agents"],
                true
            );
            assert!(prompt_line(&fixture.store, &session).unwrap().is_some());
            assert_eq!(
                fixture.graphs.act(&fixture.project, "withhold").unwrap()["agents"],
                false
            );
            silent();
            fixture.graphs.act(&fixture.project, "offer").unwrap();
            let line = prompt_line(&fixture.store, &session).unwrap().unwrap();
            let graph = live(&fixture.store.state, &fixture.project);
            assert!(
                line.contains(&format!("query \"<question>\" --graph {}", graph.display())),
                "{line}"
            );
            assert!(line.contains("into 7 nodes."), "{line}");
            assert!(line.contains("Keep --graph and its path last"), "{line}");
            assert!(
                line.contains(&fixture.tool.to_string_lossy().into_owned()),
                "{line}"
            );
            let elsewhere =
                json!({"project_id": fixture.project, "project_path": "/another/checkout"});
            assert!(
                prompt_line(&fixture.store, &elsewhere)
                    .unwrap()
                    .unwrap()
                    .contains("another checkout")
            );
            // Claude Code is permitted what the prompt offers and no more: each
            // rule opens and closes as one of the offered lines does, with the
            // graph as its last word.
            let permitted = grant(&fixture.store, &fixture.project).unwrap().unwrap();
            let tool = fixture.tool.to_string_lossy();
            assert_eq!(
                permitted.rules,
                ["query", "explain", "path"]
                    .map(|verb| format!("Bash({tool} {verb} * --graph {})", graph.display()))
            );
            for rule in &permitted.rules {
                let (opening, closing) = rule["Bash(".len()..rule.len() - 1]
                    .split_once(" * ")
                    .unwrap();
                assert!(
                    line.lines()
                        .any(|offered| offered.starts_with(opening) && offered.ends_with(closing)),
                    "{rule}\n{line}"
                );
            }
            assert_eq!(permitted.environment, [("GRAPHIFY_NO_AUTO_REFRESH", "1")]);
            // Switching the graph off silences the launch and keeps the
            // choice; deleting the graph forgets it.
            fixture.graphs.act(&fixture.project, "disable").unwrap();
            silent();
            assert!(graph.is_file());
            assert_eq!(
                fixture.graphs.act(&fixture.project, "enable").unwrap()["agents"],
                true
            );
            fixture.settled();
            assert!(prompt_line(&fixture.store, &session).unwrap().is_some());
            assert_eq!(
                fixture.graphs.act(&fixture.project, "delete").unwrap()["agents"],
                false
            );
            silent();
        }
    }
}
