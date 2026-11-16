//! Optional code graph of a project, built by the external Graphify tool.
//!
//! The Harness finds an installed `graphify`, runs its deterministic `update`
//! pass in the background and tells an agent that the result exists. A build
//! starts without the previous graph, so a deleted file leaves no nodes behind,
//! and the published graph is replaced only by a build that succeeded.
pub(crate) mod shell_reads;

use crate::{
    HarnessError, Result,
    graph_changes::{self, Pull, Resolved, Target},
    graph_map::Sharing,
    projects,
    services::{Domain, Payload, Reply},
    store::{self, SessionStore},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
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
const WORKSPACE_PRINT_ENTRIES: usize = 20_000;
const WORKSPACE_PRINT_DEPTH: usize = 32;
const WORKSPACE_PRINT_IGNORED: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "graphify-out",
    "node_modules",
    "vendor",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    ".pytest_cache",
    ".mypy_cache",
    ".idea",
    ".vscode",
];

fn workspace_print(root: &Path) -> Option<Print> {
    if !root.is_dir() {
        return None;
    }
    let mut stack = vec![(root.to_path_buf(), String::new(), 0usize)];
    let mut files: Vec<(String, u64, u128)> = Vec::new();
    let mut seen = 0usize;
    while let Some((dir, prefix, depth)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) if depth == 0 => return None,
            Err(_) => continue,
        };
        let mut children = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name();
                let name_str = name.to_string_lossy().into_owned();
                if WORKSPACE_PRINT_IGNORED.contains(&name_str.as_str()) {
                    return None;
                }
                Some((name_str, entry.path()))
            })
            .collect::<Vec<_>>();
        children.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, path) in children.into_iter().rev() {
            if seen >= WORKSPACE_PRINT_ENTRIES {
                break;
            }
            seen += 1;
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if meta.is_dir() {
                if depth < WORKSPACE_PRINT_DEPTH {
                    stack.push((path, rel, depth + 1));
                }
            } else if meta.is_file() {
                let changed = meta
                    .modified()
                    .ok()
                    .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |at| at.as_nanos());
                files.push((rel, meta.len(), changed));
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut identity = b"workspace\0".to_vec();
    for (rel, len, changed) in files {
        identity.extend_from_slice(rel.as_bytes());
        identity.extend_from_slice(format!("\0{len}:{changed}\n").as_bytes());
    }
    Some(Print {
        hash: store::sha(&identity),
        commit: None,
    })
}

/// Identity of the project's files: in Git, the commit plus every uncommitted
/// path with its size and modification time; outside Git, a bounded sorted
/// fingerprint of non-ignored workspace file paths, sizes, and modification times.
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
    if !git(&["rev-parse", "--is-inside-work-tree"]).is_some_and(|out| out.code == 0) {
        return workspace_print(root);
    }
    let head = git(&["rev-parse", "--verify", "HEAD"])?;
    let commit = (head.code == 0)
        .then(|| String::from_utf8_lossy(&head.stdout).trim().to_owned())
        .filter(|value| !value.is_empty());
    let prefix = git(&["rev-parse", "--show-prefix"])
        .filter(|out| out.code == 0)
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default();
    let status = git(&["status", "--porcelain=v1", "-z", "--", "."])?;
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
        let rel = path.strip_prefix(&prefix).unwrap_or(path);
        // A folder's own time moves when a file inside is replaced, which says
        // nothing about the files Git already lists.
        if let Ok(meta) = std::fs::metadata(root.join(rel))
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
/// Removes the written-out files of a commit but for what says which links
/// of its graph cannot be: Cargo manifests, Rust module declarations and
/// Rust test spans. A link is removed and never followed.
fn thin(tree: &Path) -> Option<crate::graph_map::TestSpans> {
    let mut folders = vec![tree.to_path_buf()];
    let mut at = 0;
    let mut complete = true;
    let mut tests = BTreeMap::new();
    while at < folders.len() {
        // The folder is listed before anything is written into it.
        let entries = std::fs::read_dir(&folders[at])
            .and_then(|entries| entries.collect::<std::io::Result<Vec<_>>>());
        at += 1;
        let Ok(entries) = entries else {
            complete = false;
            continue;
        };
        let mut kept = Vec::new();
        for entry in entries {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => {
                    folders.push(path);
                    continue;
                }
                Ok(kind) if kind.is_file() && entry.file_name() == "Cargo.toml" => continue,
                Ok(kind) if kind.is_file() && path.extension().is_some_and(|end| end == "rs") => {
                    if let Some(modules) = crate::graph_crates::kept(&path) {
                        let mut name = path.clone().into_os_string();
                        name.push(".");
                        name.push(crate::graph_crates::MODULES);
                        kept.push((PathBuf::from(name), modules));
                    }
                    match crate::graph_map::kept_tests(&path) {
                        Ok(spans) if !spans.is_empty() => {
                            let relative = path.strip_prefix(tree).ok()?;
                            tests.insert(relative.to_string_lossy().replace('\\', "/"), spans);
                        }
                        Ok(_) => {}
                        Err(_) => complete = false,
                    }
                }
                Err(_) => complete = false,
                _ => {}
            }
            // A link to a folder is removed as a folder on Windows.
            if std::fs::remove_file(&path).is_err() {
                let _ = std::fs::remove_dir(&path);
            }
        }
        for (path, metadata) in kept {
            complete &= std::fs::write(path, metadata).is_ok();
        }
    }
    // The deepest first: a folder that kept nothing goes too.
    for folder in folders.iter().rev() {
        let _ = std::fs::remove_dir(folder);
    }
    complete.then_some(tests)
}
/// What was chosen, for the interface: the two sides and how many files.
fn chosen(resolved: &Resolved) -> Value {
    let mut chosen = resolved.json();
    chosen["files"] = json!(resolved.files.len());
    chosen
}
/// A run of an agent as a change: the files it changed are what is mapped,
/// the files it opened are laid over the map, and the rest is what a picture
/// says of the run beside itself.
pub(crate) struct RunChange {
    pub(crate) resolved: Arc<Resolved>,
    pub(crate) opened: BTreeSet<String>,
    pub(crate) shell_reads: BTreeMap<String, u64>,
    pub(crate) told: Value,
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
fn offer_with_override(
    store: &SessionStore,
    project: &str,
    force_offer: bool,
) -> Result<Option<Offer>> {
    let Some(setting) = store.setting(&key(project))? else {
        return Ok(None);
    };
    let built = &setting["built"];
    let graph = live(&store.state, project);
    let Some(program) = built["program"].as_str().map(Path::new) else {
        return Ok(None);
    };
    if setting["enabled"] != true
        || (!force_offer && setting["agents"] != true)
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
fn offer(store: &SessionStore, project: &str) -> Result<Option<Offer>> {
    offer_with_override(store, project, false)
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
pub(crate) struct ServiceOfferData {
    pub(crate) graph_path: PathBuf,
    pub(crate) program_path: PathBuf,
    pub(crate) nodes: Option<u64>,
    pub(crate) commit: Option<String>,
    pub(crate) claude_allow_rules: Vec<String>,
    pub(crate) environment: Vec<(String, String)>,
    pub(crate) readable_paths: Vec<PathBuf>,
    pub(crate) prompt_block: String,
}
pub(crate) fn service_offer(
    store: &SessionStore,
    project: &str,
    service_label: &str,
    force_offer: bool,
) -> Result<Option<ServiceOfferData>> {
    let Some(Offer {
        command,
        graph,
        built,
    }) = offer_with_override(store, project, force_offer)?
    else {
        return Ok(None);
    };
    let graph_path = live(&store.state, project);
    let Some(program_path) = built["program"].as_str().map(PathBuf::from) else {
        return Ok(None);
    };
    let commit = built["commit"].as_str().map(str::to_owned);
    let nodes = built["nodes"].as_u64();
    let origin = commit
        .as_deref()
        .map(|c| format!(" at commit {c}"))
        .unwrap_or_default();
    let nodes_label = nodes
        .map(|n| format!("{n} nodes"))
        .unwrap_or_else(|| "a graph".to_owned());
    let questions: String = QUESTIONS
        .iter()
        .map(|(verb, arguments)| format!("  {command} {verb} {arguments} --graph {graph}\n"))
        .collect();
    let prompt_block = format!(
        "Code graph for `{service_label}` (optional; files remain the source of truth): Graphify indexed `{service_label}`{origin} into {nodes_label}.\n{questions}Keep --graph and its path last; confirm findings in the source files.\n"
    );
    let claude_allow_rules = QUESTIONS
        .iter()
        .map(|(verb, _)| format!("Bash({command} {verb} * --graph {graph})"))
        .collect();
    let mut readable_paths = vec![graph_path.clone()];
    if let Some(parent) = graph_path.parent() {
        readable_paths.push(parent.to_path_buf());
    }
    readable_paths.push(program_path.clone());
    if let Some(bin_dir) = program_path.parent() {
        readable_paths.push(bin_dir.to_path_buf());
        if bin_dir
            .file_name()
            .is_some_and(|n| n == "bin" || n == "Scripts")
            && let Some(venv) = bin_dir.parent()
            && venv.join("pyvenv.cfg").is_file()
        {
            readable_paths.push(venv.to_path_buf());
        }
    }
    Ok(Some(ServiceOfferData {
        graph_path,
        program_path,
        nodes,
        commit,
        claude_allow_rules,
        environment: vec![(NO_SKILL_REFRESH.0.to_owned(), NO_SKILL_REFRESH.1.to_owned())],
        readable_paths,
        prompt_block,
    }))
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
    /// Replacement revision jobs share this gate until their work has finished.
    revision_gate: Arc<Mutex<()>>,
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
            revision_gate: Arc::new(Mutex::new(())),
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
            let (phase, floor, message) = if stage == WAITING {
                (
                    "wait",
                    5u64,
                    "Preparing workspace snapshot to build code graph… 5%",
                )
            } else {
                (
                    "export",
                    12u64,
                    "Exporting commit files to build code graph… 12%",
                )
            };
            return json!({
                "phase": phase,
                "done": null,
                "total": null,
                "percent": floor,
                "remaining_seconds": null,
                "rough": history.is_none(),
                "elapsed_seconds": elapsed.floor() as u64,
                "overdue": false,
                "message": message,
            });
        }
        let Ok(mut seen) = self.seen.lock() else {
            return Value::Null;
        };
        let (raw_percent, left, overdue) = progress(elapsed, &seen.observed, history, seen.peak);
        let phase = if seen.observed.read.is_some() {
            "assemble"
        } else if seen.observed.first.is_some() {
            "read"
        } else {
            "scan"
        };
        let done = seen.observed.last.map(|(_, done)| done);
        let total = (seen.observed.total > 0).then_some(seen.observed.total);
        let fallback = match phase {
            "scan" => (15 + (elapsed.floor() as u64).min(13)).min(28),
            "read" => match (done, total) {
                (Some(done), Some(total)) if total > 0 => {
                    (28 + (done.saturating_mul(50) / total)).clamp(30, 80)
                }
                _ => 30,
            },
            _ => 85,
        };
        let percent = raw_percent.unwrap_or(fallback).max(seen.peak).clamp(1, 99);
        seen.peak = percent;
        let message = match phase {
            "scan" => format!("Building code graph: scanning project files… {percent}%"),
            "read" => match (done, total) {
                (Some(done), Some(total)) if total > 0 => {
                    format!("Building code graph: analyzing files ({done}/{total})… {percent}%")
                }
                _ => format!("Building code graph: analyzing project files… {percent}%"),
            },
            _ => format!("Computing code graph and dependency map… {percent}%"),
        };
        json!({
            "phase": phase,
            "done": done,
            "total": total,
            "percent": percent,
            "remaining_seconds": left,
            // Without a previous build the silent part is a guess from one ratio.
            "rough": history.is_none(),
            "elapsed_seconds": elapsed.floor() as u64,
            "overdue": overdue,
            "message": message,
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
enum RevisionBuild<'a> {
    Normal,
    Comparison { head: &'a str },
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
fn status_message(state: &str, build: Option<&Value>, failure: Option<&str>) -> String {
    match state {
        "building" => build
            .and_then(|value| value["message"].as_str())
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "Building code graph…".to_owned()),
        "ready" => "Code graph & dependency map ready.".to_owned(),
        "stale" => "Code graph is outdated; changes detected in project.".to_owned(),
        "failed" => match failure.filter(|text| !text.is_empty()) {
            Some(reason) => format!("Code graph build failed: {reason}"),
            None => "Code graph build failed.".to_owned(),
        },
        "missing" => "Graphify is not installed. Install it to build the code graph.".to_owned(),
        "empty" => "Code graph has not been built yet.".to_owned(),
        _ => "Code graph is turned off.".to_owned(),
    }
}

fn revision_message(state: &str, build: &Value, failure: Option<&str>) -> String {
    match state {
        "missing" => "Code graph for this revision is not built yet.".to_owned(),
        _ => status_message(state, Some(build), failure),
    }
}

fn apply_graph_focus(
    store: &SessionStore,
    project: &str,
    focus_session: Option<&str>,
    index: Option<&crate::graph_map::Index>,
    view: &mut Value,
    keep_run: bool,
) -> Result<()> {
    let graph_focus = if let Some(run_id) = focus_session.and_then(|s| s.strip_prefix("sysrun:"))
        && let Some(sys_focus) =
            crate::system_graph::load_system_focus_value(store, run_id, project)?
    {
        sys_focus
    } else {
        crate::graph_live::project_focus_value(store, project, focus_session)?
    };
    if let (Some(index), Ok(focus)) = (
        index,
        serde_json::from_value::<crate::graph_live::GraphFocus>(graph_focus.clone()),
    ) {
        index.overlay_graph_focus(view, &focus);
        if !keep_run && let Some(obj) = view.as_object_mut() {
            obj.remove("run");
        }
    } else {
        view["graph_focus"] = graph_focus.clone();
        if keep_run && view["run"].is_object() {
            view["run"]["graph_focus"] = graph_focus;
        }
    }
    Ok(())
}

/// What of a graph a request for the map asks to see.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shown<'a> {
    /// The folder or the file that is open; the project where it is empty.
    pub scope: &'a str,
    /// The place whose two sides are listed.
    pub focus: Option<&'a str>,
    /// Optional session ID whose live prompt target & entities are overlaid.
    pub focus_session: Option<&'a str>,
    /// Only the links the tool read from the code.
    pub certain: bool,
    /// Only the names that hold this.
    pub filter: &'a str,
    /// A place whose links are followed step by step, instead of a folder.
    pub followed: Option<&'a Followed>,
    /// Without tests: the files that are tests, and the tests inside others.
    pub notests: bool,
    /// Folders of the open one that are spread out in its picture.
    pub spread: &'a [String],
    /// One link of a picture to look into, instead of a picture: the place
    /// that uses and the place that is used.
    pub link: Option<(&'a str, &'a str)>,
    /// Two places to find the shortest ways between, instead of a folder:
    /// the one they lead from and the one they lead to.
    pub path: Option<(&'a str, &'a str)>,
    /// A place to find the shortest directed cycle through.
    pub cycle: Option<&'a str>,
    /// The selected PR's base identifier for a cycle or link inspection.
    pub base: Option<&'a str>,
    /// Two places to find what they share, instead of a folder, with
    /// which part of that to draw.
    pub shared: Option<Sharing<'a>>,
    /// Mark the picture with the patterns found where it is open.
    pub patterns: bool,
    pub history: bool,
    pub modules: bool,
    pub no_incoming: bool,
    pub scenario: Option<(&'a str, usize)>,
    pub launch: Option<&'a str>,
    pub compare_launch: Option<&'a str>,
    pub agent: Option<&'a str>,
    /// One found pattern to draw, instead of a folder: its pattern and the
    /// type it is named after.
    pub pattern: Option<&'a str>,
    /// Unfold all folders to render the entire project graph at once.
    pub whole: bool,
}
struct ImpactContext {
    root: PathBuf,
    index: Option<Arc<crate::graph_map::Index>>,
    resolved: Option<Arc<Resolved>>,
    run_edited: Vec<(String, char)>,
    live_focus: Option<crate::graph_live::GraphFocus>,
    target_label: String,
}
/// How many places a request may name in one list, and in how many bytes.
const UNFOLDED: usize = 256;
const UNFOLDED_BYTES: usize = 32 * 1024;
/// A list of places as a request carries it: a JSON list of paths, which a
/// URL carries whatever the paths hold. `many` and `wrong` say what to do
/// about a list that is too long and one that is none.
pub fn places(text: &str, many: &'static str, wrong: &'static str) -> Result<Vec<String>> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if text.len() > UNFOLDED_BYTES {
        return Err(HarnessError::invalid(many));
    }
    let places =
        serde_json::from_str::<Vec<String>>(text).map_err(|_| HarnessError::invalid(wrong))?;
    if places.len() > UNFOLDED {
        return Err(HarnessError::invalid(many));
    }
    if places
        .iter()
        .any(|place| place.is_empty() || place.len() > 1024)
    {
        return Err(HarnessError::invalid(wrong));
    }
    Ok(places)
}
/// A place whose links are followed step by step in the map, with the places
/// whose uses (`out`) and whose users (`into`) are unfolded, each named once
/// for every step that is unfolded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Followed {
    pub root: String,
    pub out: Vec<String>,
    pub into: Vec<String>,
}
impl Followed {
    /// Reads what a request names: the place, and the unfolded places of
    /// each side as a list of paths. `None` where no place is followed.
    pub fn parse(root: &str, out: &str, into: &str) -> Result<Option<Self>> {
        if root.is_empty() {
            return Ok(None);
        }
        let list = |text: &str| {
            places(
                text,
                "Too much is unfolded; fold something first.",
                "Invalid unfolded places.",
            )
        };
        Ok(Some(Self {
            root: root.to_owned(),
            out: list(out)?,
            into: list(into)?,
        }))
    }
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
    autostarting: AtomicBool,
    maps: crate::graph_map::Maps,
    history: Mutex<BTreeMap<String, HistoryEntry>>,
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
    /// Synthetic system service graph targets (`sys-<hash>` -> `(root, name)`).
    targets: Mutex<BTreeMap<String, (PathBuf, String)>>,
}
type Listing = (Instant, Vec<Pull>, Option<String>);
struct HistoryEntry {
    revision: String,
    read_at: Instant,
    data: Arc<graph_changes::History>,
}
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
            autostarting: AtomicBool::new(false),
            maps: crate::graph_map::Maps::default(),
            history: Mutex::new(BTreeMap::new()),
            pulls: Mutex::new(BTreeMap::new()),
            searches: Mutex::new(BTreeMap::new()),
            chosen: Mutex::new(BTreeMap::new()),
            revisions: Mutex::new(BTreeMap::new()),
            refused: Mutex::new(BTreeMap::new()),
            targets: Mutex::new(BTreeMap::new()),
        })
    }
    /// Registers a synthetic system-service graph workspace (`sys-<hash>`) backed by `root`.
    pub fn register_target(&self, project: &str, root: PathBuf, name: String) -> Result<()> {
        lock(&self.targets)?.insert(project.to_owned(), (root.clone(), name.clone()));
        if self.store.project(project).is_err() {
            let root_str = root.to_string_lossy().into_owned();
            let current = self.setting(project)?;
            if current["target_root"].as_str() != Some(root_str.as_str())
                || current["target_name"].as_str() != Some(name.as_str())
            {
                self.edit(project, |setting| {
                    setting["target_root"] = json!(root_str);
                    setting["target_name"] = json!(name);
                })?;
            }
        }
        Ok(())
    }
    fn target_info(&self, project: &str) -> Result<(PathBuf, String)> {
        if let Ok(record) = self.store.project(project) {
            let root = PathBuf::from(record["path"].as_str().unwrap_or(""));
            let name = record["name"].as_str().unwrap_or("project").to_owned();
            return Ok((root, name));
        }
        if let Some(found) = lock(&self.targets)?.get(project).cloned() {
            return Ok(found);
        }
        let setting = self.setting(project)?;
        if let Some(path) = setting["target_root"].as_str().filter(|s| !s.is_empty()) {
            let root = PathBuf::from(path);
            let name = setting["target_name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    root.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
            lock(&self.targets)?.insert(project.to_owned(), (root.clone(), name.clone()));
            return Ok((root, name));
        }
        Err(HarnessError::missing("Project not found."))
    }
    pub fn live_index(&self, project: &str) -> Option<Arc<crate::graph_map::Index>> {
        let graph = live(&self.store.state, project);
        if !graph.is_file() {
            return None;
        }
        let root = self.root(project).ok()?;
        self.maps.index(project, &graph, &root).ok()
    }
    pub fn auto_start(&self, project: &str) -> Result<()> {
        if self.stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        let root = self.root(project)?;
        if !root.is_dir() {
            return Ok(());
        }
        let setting = self.setting(project)?;
        if setting["disabled_by_user"] == true {
            return Ok(());
        }
        if setting["enabled"] != true {
            let _ = self.tool(true);
            self.edit(project, |setting| {
                setting["enabled"] = json!(true);
                if let Some(fields) = setting.as_object_mut() {
                    fields.remove("disabled_by_user");
                }
            })?;
            return self.refresh(project, Trigger::Enabled);
        }
        let published = setting["built"].is_object() && live(&self.store.state, project).is_file();
        if !published {
            return self.refresh(project, Trigger::Enabled);
        }
        let _ = self.print(project, &root, true);
        self.refresh(project, Trigger::Visit)
    }
    fn auto_start_order(&self) -> Vec<String> {
        let Ok(projects) = self.store.projects() else {
            return Vec::new();
        };
        let mut ordered = Vec::new();
        let mut seen = BTreeSet::new();
        if let Ok(summaries) = self.store.summaries() {
            for session in summaries {
                if let Some(id) = session["project_id"].as_str().filter(|id| !id.is_empty())
                    && seen.insert(id.to_owned())
                {
                    ordered.push(id.to_owned());
                }
            }
        }
        for project in projects {
            if let Some(id) = project["id"].as_str()
                && seen.insert(id.to_owned())
            {
                ordered.push(id.to_owned());
            }
        }
        ordered
    }
    pub fn auto_start_projects(self: &Arc<Self>) {
        if self.stopping.load(Ordering::Acquire) {
            return;
        }
        if self
            .autostarting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let me = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("harness-graph-autostart".into())
            .spawn(move || {
                struct Guard<'a>(&'a AtomicBool);
                impl Drop for Guard<'_> {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::Release);
                    }
                }
                let _guard = Guard(&me.autostarting);
                for id in me.auto_start_order() {
                    if me.stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let _ = me.auto_start(&id);
                }
            });
        match spawned {
            Ok(handle) => {
                if let Ok(mut workers) = self.workers.lock() {
                    workers.retain(|worker| !worker.is_finished());
                    workers.push(handle);
                }
            }
            Err(_) => {
                self.autostarting.store(false, Ordering::Release);
            }
        }
    }
    /// One folder of the published graph's dependency map, or, with a
    /// target, the map of what that target changed. With a place followed,
    /// the links of that place step by step instead, on the same graph and
    /// with the same change marked; a place the graph does not know falls
    /// back to the folder. Without tests, all of it is read from the graph
    /// as it is without them. Reading a graph of a large project takes
    /// seconds, so callers keep this off the runtime.
    pub fn map(&self, project: &str, target: Option<&Target>, shown: &Shown) -> Result<Value> {
        let (root, owned_name) = self.target_info(project)?;
        let name = owned_name.as_str();
        let built = &self.setting(project)?["built"];
        let Shown {
            scope,
            focus,
            focus_session,
            certain,
            filter,
            followed,
            notests,
            spread,
            link,
            path,
            cycle,
            shared,
            base,
            patterns,
            history,
            modules,
            no_incoming,
            scenario,
            launch,
            compare_launch,
            agent,
            pattern,
            whole,
        } = *shown;
        if base.is_some()
            && (!matches!(target, Some(Target::Pull(_)))
                || cycle.is_some() == link.is_some()
                || path.is_some()
                || shared.is_some()
                || pattern.is_some()
                || followed.is_some())
        {
            return Err(HarnessError::invalid(
                "Choose a pull request to inspect a base cycle or link.",
            ));
        }
        if (launch.is_some() || compare_launch.is_some())
            && (!matches!(target, Some(Target::Run(_) | Target::SysRun(_)))
                || compare_launch.is_some() && (launch.is_none() || launch == compare_launch))
        {
            return Err(HarnessError::invalid(
                "Choose two distinct launches of the mapped session to compare.",
            ));
        }
        // The graph as it was read, or as it is without its tests, with how
        // much that leaves out.
        let left = std::cell::Cell::new(None);
        let asked = |index: Arc<crate::graph_map::Index>| {
            if notests {
                let plain = index.without_tests();
                left.set(Some(plain.left_out()));
                plain
            } else {
                index
            }
        };
        use crate::graph_map::layer_rules::Rules;
        let wants_layers = pattern.is_some_and(|id| id.starts_with("layers@"));
        let reads_rules =
            wants_layers || (patterns && !modules && scenario.is_none() && link.is_none());
        // The links followed from a place, where one is followed and the
        // graph knows it; or, before that, one link looked into. `edited`
        // gives what the chosen change did to a file, where that can be told.
        let follow = |index: &crate::graph_map::Index,
                      changed: &[(String, char)],
                      edited: &dyn Fn(&str) -> Option<crate::graph_map::Edited>,
                      rules: &std::result::Result<Option<Rules>, String>| {
            if let Some((origin, skip)) = scenario {
                return Some(index.scenario(name, origin, certain, filter, skip));
            }
            if modules {
                return Some(index.modules(name, focus, certain, filter));
            }
            if let Some((from, to)) = link {
                return Some(index.link(from, to, certain));
            }
            if let Some(place) = cycle {
                return Some(index.cycle(name, place, changed, edited, focus, certain));
            }
            if let Some(scope) = pattern.and_then(|id| id.strip_prefix("layers@")) {
                match rules {
                    Ok(Some(rules)) => {
                        return Some(index.declared_layers(name, scope, focus, certain, rules));
                    }
                    Err(_) => return None,
                    Ok(None) => {}
                }
            }
            // One found pattern; a graph that no longer has it leaves
            // what else was asked.
            if let Some(id) = pattern
                && let Some(view) = index.pattern(name, id, changed, edited, focus, certain)
            {
                return Some(view);
            }
            // The ways between two places, or what the two share, come
            // before a followed place; a place the graph does not know
            // leaves what else was asked.
            if let Some((from, to)) = path
                && let Some(view) = index.path(name, from, to, changed, edited, focus, certain)
            {
                return Some(view);
            }
            if let Some(asked) = shared
                && let Some(view) =
                    index.shared(name, asked, changed, edited, focus, certain, filter)
            {
                return Some(view);
            }
            followed.and_then(|asked| {
                index.follow_with(
                    name,
                    &asked.root,
                    &asked.out,
                    &asked.into,
                    changed,
                    edited,
                    focus,
                    certain,
                )
            })
        };
        // What every answer says of the tests: whether they are left out,
        // and how many that is.
        let told = |view: &mut Value| {
            view["notests"] = json!(notests);
            if let Some((files, inside)) = left.get() {
                view["tests"] = json!({"files": files, "inside": inside});
            }
        };
        // The patterns found where a picture is open, on its boxes and
        // beside it, where they were asked for. A link looked into is no
        // picture of places, and a pattern's own picture is the pattern.
        let marked = |index: &crate::graph_map::Index,
                      view: &mut Value,
                      rules: &std::result::Result<Option<Rules>, String>| {
            if let Err(error) = rules {
                view["architecture_error"] = json!(error);
            }
            if patterns
                && !modules
                && scenario.is_none()
                && link.is_none()
                && view["mode"] != "pattern"
                && view["nodes"].is_array()
            {
                index.mark_patterns(view, certain);
                match rules {
                    Ok(Some(rules)) => index.mark_declared_layers(view, Some(rules)),
                    Err(_) => index.mark_declared_layers(view, None),
                    Ok(None) => {}
                }
                if let Ok(rules) = rules {
                    index.mark_architecture(view, certain, rules.as_ref());
                }
            }
        };
        let mark_history =
            |index: &crate::graph_map::Index, view: &mut Value, revision: Option<&str>| {
                if history
                    && !modules
                    && scenario.is_none()
                    && link.is_none()
                    && !view["path"].is_object()
                    && !view["shared"].is_object()
                    && !view["pattern"].is_object()
                    && !view["cycle"].is_object()
                {
                    match self.git_history(project, &root, revision) {
                        Ok(Some(found)) => index.mark_history(view, &found),
                        Ok(None) => view["git_history"] = json!({"state": "empty"}),
                        Err(_) => view["git_history"] = json!({"state": "unavailable"}),
                    }
                }
            };
        let mark_incoming = |index: &crate::graph_map::Index, view: &mut Value| {
            if no_incoming
                && link.is_none()
                && matches!(view["mode"].as_str(), None | Some("changes" | "file"))
                && view["nodes"].is_array()
            {
                index.mark_without_incoming(view, certain, filter);
            }
        };
        let Some(target) = target else {
            let index = asked(self.maps.index(
                project,
                &live(&self.store.state, project),
                &root,
            )?);
            let rules = if reads_rules {
                Rules::read(&root)
            } else {
                Ok(None)
            };
            let mut view = follow(&index, &[], &|_| None, &rules).unwrap_or_else(|| {
                index.view_with_whole(name, scope, spread, focus, certain, filter, whole)
            });
            marked(&index, &mut view, &rules);
            mark_history(&index, &mut view, None);
            mark_incoming(&index, &mut view);
            view["project_id"] = json!(project);
            view["built"] = json!({"commit": built["commit"], "at": built["at"]});
            view["message"] = json!("Code graph & dependency map ready.");
            apply_graph_focus(
                &self.store,
                project,
                focus_session,
                Some(&index),
                &mut view,
                false,
            )?;
            told(&mut view);
            return Ok(view);
        };
        // A run is read from its session every time: it may still be going.
        let (ran, comparison) = match target {
            Target::Run(session) => {
                let (ran, comparison) =
                    self.run_launches(project, &root, session, launch, compare_launch)?;
                (Some(ran), comparison)
            }
            Target::SysRun(run_id) => {
                let (ran, comparison) = crate::system_graph::sysrun_launches(
                    &self.store,
                    project,
                    &root,
                    run_id,
                    agent,
                    launch,
                    compare_launch,
                )?;
                (Some(ran), comparison)
            }
            _ => (None, None),
        };
        let resolved = match (target, &ran) {
            (_, Some(ran)) => ran.resolved.clone(),
            // The checkout is asked every time: it is local, and it moves.
            (Target::Work | Target::Uncommitted, _) => Arc::new(graph_changes::resolve(
                &root,
                target,
                &self.listed(project, &root),
            )?),
            _ => lock(&self.chosen)?
                .get(&(project.to_owned(), target.text()))
                .cloned()
                .ok_or_else(|| HarnessError::conflict("Choose the change again."))?,
        };
        if let Some(expected) = base {
            let chosen = chosen(&resolved);
            let commit = &resolved
                .base
                .as_ref()
                .filter(|base| base.commit == expected)
                .ok_or_else(|| {
                    HarnessError::conflict("The pull request base changed. Reopen the PR cycle.")
                })?
                .commit;
            let found = self.graph_at(project, &root, Some(commit), &resolved.prefix, true)?;
            let (key, graph, tree) = found.graph.ok_or_else(|| {
                HarnessError::unavailable("The PR base graph is no longer cached. Return to the PR cycle to prepare it again.")
            })?;
            let index = self.maps.index(&key, &graph, &tree)?;
            if notests && index.has_rust_sources() && !index.has_test_spans() {
                return Err(HarnessError::unavailable(
                    "This cached base graph cannot be shown with Without tests.",
                ));
            }
            let index = asked(index);
            let mut view = follow(&index, &[], &|_| None, &Ok(None))
                .ok_or_else(|| HarnessError::invalid("Choose a base cycle or link."))?;
            view["project_id"] = json!(project);
            view["target"] = json!(target.text());
            view["base"] = chosen["base"].clone();
            view["chosen"] = chosen;
            view["state"] = json!("ready");
            told(&mut view);
            return Ok(view);
        }
        let rules = if reads_rules {
            match &resolved.revision {
                Some(revision) => graph_changes::architecture_at(&root, revision, &resolved.prefix),
                None => Rules::read(&root),
            }
        } else {
            Ok(None)
        };
        let found = self.graph_of(project, &root, &resolved)?;
        let pull_cycle = cycle.is_some() && matches!(target, Target::Pull(_));
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
        let (mut view, active_index) = match &found.graph {
            Some((kept, graph, base)) => {
                let raw = self.maps.index(kept, graph, base)?;
                let index = asked(raw.clone());
                // The symbols a change touched are told by line, and the
                // lines of a graph older than the files are not the files'
                // own: then links are followed without those marks.
                let touched = |path: &str| {
                    if stale {
                        return None;
                    }
                    graph_changes::edited(&root, &resolved, path).ok().flatten()
                };
                let mut view = if let Some(mut view) = follow(&index, &changed, &touched, &rules) {
                    if pull_cycle && view["mode"] == "cycle" {
                        let snapshot = kept.contains('@');
                        self.compare_pr_cycle(
                            project, &root, &resolved, &raw, &index, snapshot, &mut view, shown,
                        );
                    }
                    // The layers of a folder say what the change did to
                    // their order. A graph older than the files of the
                    // checkout does not hold what they do to it yet. Tests
                    // inside a Rust file are told by the spans kept beside
                    // the graph of a commit: a graph without them cannot
                    // be compared without its tests.
                    if view["pattern"]["sort"] == "layers" && view["pattern"]["declared"] != true {
                        let untold = if stale {
                            Some("stale")
                        } else if notests
                            && kept.contains('@')
                            && raw.has_rust_sources()
                            && !raw.has_test_spans()
                        {
                            Some("tests")
                        } else {
                            None
                        };
                        if let Some(reason) = untold {
                            view["pattern"]["change"] = json!({
                                "state": "unavailable", "reason": reason,
                                "base": resolved.json()["base"],
                            });
                        } else {
                            self.compare_layers(
                                project, &root, &resolved, &index, &mut view, shown,
                            );
                        }
                    }
                    view
                } else {
                    // An opened file marks the symbols the change touched.
                    // They are told by line, and the lines of a graph older
                    // than the files are not the files' own.
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
                };
                marked(&index, &mut view, &rules);
                mark_history(&index, &mut view, resolved.revision.as_deref());
                mark_incoming(&index, &mut view);
                if !notests && !stale {
                    index.tests_reaching_changes(&mut view, &changed, scope, certain);
                }
                // What a run opened is laid over whatever picture of its
                // change is drawn.
                if let Some(ran) = &ran
                    && !modules
                    && scenario.is_none()
                {
                    view["run"] = ran.told.clone();
                    let followup = comparison.as_ref().map(|run| {
                        let edited = run
                            .resolved
                            .files
                            .iter()
                            .map(|file| file.path.as_str())
                            .collect();
                        (run, edited)
                    });
                    index.overlay_run_with_comparison(
                        &mut view,
                        &changed,
                        (&ran.opened, &ran.shell_reads),
                        certain,
                        followup
                            .as_ref()
                            .map(|(run, edited)| ((&run.opened, &run.shell_reads), edited)),
                    );
                }
                (view, Some(index))
            }
            // Until the commit's own graph is there, the project's graph
            // gives a first picture: the files changed are the commit's, the
            // links are those the checkout's files have.
            None if published.is_file() => {
                let index = asked(self.maps.index(project, &published, &root)?);
                let mut view = follow(&index, &changed, &|_| None, &rules).unwrap_or_else(|| {
                    index.changes(name, &changed, scope, focus, certain, filter)
                });
                marked(&index, &mut view, &rules);
                view["preliminary"] = json!(true);
                if view["pattern"]["sort"] == "layers" && view["pattern"]["declared"] != true {
                    view["pattern"]["change"] = if found.state == "failed" {
                        json!({"state": "unavailable", "reason": "head"})
                    } else {
                        json!({"state": "waiting_head"})
                    };
                }
                if pull_cycle && view["mode"] == "cycle" {
                    view["cycle"]["comparison"] = if found.state == "failed" {
                        json!({"state": "unavailable", "reason": "head"})
                    } else {
                        json!({"state": "waiting_head"})
                    };
                }
                (view, Some(index))
            }
            None => (json!({"mode": "changes"}), None),
        };
        let keep_run = view["run"].is_object();
        view["project_id"] = json!(project);
        view["target"] = json!(target.text());
        view["chosen"] = chosen(&resolved);
        view["state"] = json!(found.state);
        view["build"] = found.build;
        view["failure"] = json!(found.failure);
        view["message"] = json!(revision_message(
            found.state,
            &view["build"],
            found.failure.as_deref(),
        ));
        let sysrun_focus_id;
        let focus_sid = match target {
            Target::Run(session) => Some(session.as_str()),
            Target::SysRun(run_id) => {
                sysrun_focus_id = format!("sysrun:{run_id}");
                Some(sysrun_focus_id.as_str())
            }
            _ => focus_session,
        };
        apply_graph_focus(
            &self.store,
            project,
            focus_sid,
            active_index.as_deref(),
            &mut view,
            keep_run,
        )?;
        if own {
            view["stale"] = json!(stale);
            view["rebuilding"] = json!(lock(&self.jobs)?.contains_key(project));
            view["built"] = json!({"commit": built["commit"], "at": built["at"]});
        }
        told(&mut view);
        Ok(view)
    }
    fn git_history(
        &self,
        project: &str,
        root: &Path,
        revision: Option<&str>,
    ) -> Result<Option<Arc<graph_changes::History>>> {
        let Some(revision) = graph_changes::history_revision(root, revision)? else {
            return Ok(None);
        };
        if let Some(cached) = lock(&self.history)?.get(project).filter(|entry| {
            entry.revision == revision && entry.read_at.elapsed() < Duration::from_secs(60)
        }) {
            return Ok(Some(cached.data.clone()));
        }
        let found = Arc::new(graph_changes::history(root, &revision)?);
        lock(&self.history)?.insert(
            project.to_owned(),
            HistoryEntry {
                revision,
                read_at: Instant::now(),
                data: found.clone(),
            },
        );
        Ok(Some(found))
    }

    /// What a search finds in the graph the map of `target` is drawn from:
    /// the project's own for the whole project and for the checkout's
    /// changes, a commit's own once it is built, and the project's until
    /// then. Without tests, what is a test is not found either. Like the
    /// map, it reads a graph, so callers keep it off the runtime.
    pub fn find(
        &self,
        project: &str,
        target: Option<&Target>,
        query: &str,
        notests: bool,
    ) -> Result<Value> {
        let (root, _) = self.target_info(project)?;
        let published = live(&self.store.state, project);
        let graph = match target {
            None
            | Some(Target::Work | Target::Uncommitted | Target::Run(_) | Target::SysRun(_)) => {
                Some((project.to_owned(), published, root))
            }
            Some(target) => {
                let resolved = lock(&self.chosen)?
                    .get(&(project.to_owned(), target.text()))
                    .cloned()
                    .ok_or_else(|| HarnessError::conflict("Choose the change again."))?;
                match self.graph_of(project, &root, &resolved)?.graph {
                    Some(own) => Some(own),
                    None if published.is_file() => Some((project.to_owned(), published, root)),
                    None => None,
                }
            }
        };
        let mut found = match graph {
            Some((kept, graph, base)) => {
                let index = self.maps.index(&kept, &graph, &base)?;
                if notests {
                    index.without_tests().find(query)
                } else {
                    index.find(query)
                }
            }
            // Neither the commit's own graph nor the project's to search.
            None => {
                json!({"mode": "find", "query": query.trim(), "found": 0, "rows": [], "missing": true})
            }
        };
        found["project_id"] = json!(project);
        found["notests"] = json!(notests);
        Ok(found)
    }

    fn load_project_focus(
        &self,
        project: &str,
        focus_session: Option<&str>,
    ) -> Result<Option<crate::graph_live::GraphFocus>> {
        if let Some(run_id) = focus_session.and_then(|s| s.strip_prefix("sysrun:"))
            && let Some(value) =
                crate::system_graph::load_system_focus_value(&self.store, run_id, project)?
        {
            return Ok(serde_json::from_value(value).ok());
        }
        let value = crate::graph_live::project_focus_value(&self.store, project, focus_session)?;
        if value.is_null() {
            return Ok(None);
        }
        Ok(serde_json::from_value(value).ok())
    }

    fn recent_session_for_project(&self, project: &str) -> Result<Option<String>> {
        let sessions = self.store.summaries()?;
        let active_statuses = ["running", "starting", "queued", "active"];
        for pass_active in [true, false] {
            for session in &sessions {
                if session["project_id"].as_str() != Some(project) {
                    continue;
                }
                let status = session["status"].as_str().unwrap_or("");
                if pass_active && !active_statuses.contains(&status) {
                    continue;
                }
                if let Some(sid) = session["id"].as_str() {
                    return Ok(Some(sid.to_owned()));
                }
            }
        }
        Ok(None)
    }

    fn impact_context(
        &self,
        project: &str,
        target: Option<&Target>,
        focus_session: Option<&str>,
        agent: Option<&str>,
    ) -> Result<ImpactContext> {
        let focus_session = match focus_session.map(str::trim).filter(|s| !s.is_empty()) {
            Some(sid) if sid.len() <= 1024 && !sid.chars().any(char::is_control) => Some(sid),
            Some(_) => return Err(HarnessError::invalid("Invalid code graph session.")),
            None => None,
        };
        let root = self.root(project)?;
        let published = live(&self.store.state, project);
        let live_index = || {
            published
                .is_file()
                .then(|| self.maps.index(project, &published, &root).ok())
                .flatten()
        };
        match target {
            Some(Target::Run(session)) => {
                let ran = self.ran(project, &root, session)?;
                let run_edited = ran
                    .resolved
                    .files
                    .iter()
                    .map(|change| (change.path.clone(), change.status))
                    .collect();
                let live_focus = self.load_project_focus(project, Some(session.as_str()))?;
                let index = live_index();
                Ok(ImpactContext {
                    root,
                    index,
                    resolved: Some(ran.resolved),
                    run_edited,
                    live_focus,
                    target_label: Target::Run(session.clone()).text(),
                })
            }
            Some(Target::SysRun(run_id)) => {
                let (ran, _) = crate::system_graph::sysrun_launches(
                    &self.store,
                    project,
                    &root,
                    run_id,
                    agent,
                    None,
                    None,
                )?;
                let run_edited = ran
                    .resolved
                    .files
                    .iter()
                    .map(|change| (change.path.clone(), change.status))
                    .collect();
                let sys_sid = format!("sysrun:{run_id}");
                let live_focus = self
                    .load_project_focus(project, Some(&sys_sid))?
                    .or(self.load_project_focus(project, focus_session)?);
                let index = live_index();
                Ok(ImpactContext {
                    root,
                    index,
                    resolved: Some(ran.resolved),
                    run_edited,
                    live_focus,
                    target_label: Target::SysRun(run_id.clone()).text(),
                })
            }
            Some(t @ (Target::Named(_) | Target::Commit(_) | Target::Pull(_))) => {
                let existing = lock(&self.chosen)?
                    .get(&(project.to_owned(), t.text()))
                    .cloned();
                let resolved = match existing {
                    Some(res) => res,
                    None => Arc::new(graph_changes::resolve(
                        &root,
                        t,
                        &self.listed(project, &root),
                    )?),
                };
                let index = match self.graph_of(project, &root, &resolved) {
                    Ok(Found {
                        graph: Some((kept, graph, base)),
                        ..
                    }) => self
                        .maps
                        .index(&kept, &graph, &base)
                        .ok()
                        .or_else(live_index),
                    _ => live_index(),
                };
                Ok(ImpactContext {
                    root,
                    index,
                    resolved: Some(resolved),
                    run_edited: Vec::new(),
                    live_focus: None,
                    target_label: t.text(),
                })
            }
            None | Some(Target::Work | Target::Uncommitted) => {
                let is_git = graph_changes::place(&root)
                    .ok()
                    .is_some_and(|place| place["git"] == true);
                let resolved_git = if is_git {
                    let effective = target.unwrap_or(&Target::Work);
                    Some(Arc::new(graph_changes::resolve(
                        &root,
                        effective,
                        &self.listed(project, &root),
                    )?))
                } else {
                    None
                };
                let session_for_edits = if !matches!(target, Some(Target::Uncommitted)) {
                    if let Some(sid) = focus_session {
                        Some(sid.to_owned())
                    } else if !is_git {
                        self.recent_session_for_project(project)?
                    } else {
                        None
                    }
                } else {
                    None
                };
                let run_edited = session_for_edits
                    .as_deref()
                    .and_then(|sid| self.ran(project, &root, sid).ok())
                    .map(|ran| {
                        ran.resolved
                            .files
                            .iter()
                            .map(|change| (change.path.clone(), change.status))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let resolved = match (resolved_git, run_edited.is_empty()) {
                    (res, true) => res,
                    (Some(res), false) => {
                        let mut combined = (*res).clone();
                        let known: BTreeSet<String> = combined
                            .files
                            .iter()
                            .map(|file| file.path.clone())
                            .collect();
                        for (path, status) in &run_edited {
                            if !known.contains(path) {
                                combined.files.push(graph_changes::Change {
                                    path: path.clone(),
                                    status: *status,
                                    from: None,
                                });
                            }
                        }
                        Some(Arc::new(combined))
                    }
                    (None, false) => Some(Arc::new(Resolved {
                        target: Target::Run(session_for_edits.unwrap_or_default()),
                        kind: "run",
                        name: String::new(),
                        title: None,
                        pull: None,
                        head: None,
                        base: None,
                        revision: None,
                        prefix: String::new(),
                        files: run_edited
                            .iter()
                            .map(|(path, status)| graph_changes::Change {
                                path: path.clone(),
                                status: *status,
                                from: None,
                            })
                            .collect(),
                        notes: Vec::new(),
                    })),
                };
                let live_focus = self.load_project_focus(project, focus_session)?;
                let target_label = match target {
                    Some(t) => t.text(),
                    None if is_git => "work".to_owned(),
                    None => "workspace".to_owned(),
                };
                let index = live_index();
                Ok(ImpactContext {
                    root,
                    index,
                    resolved,
                    run_edited,
                    live_focus,
                    target_label,
                })
            }
        }
    }

    /// Untruncated catalog of changed files and changed entities for a project target.
    pub fn changes_catalog(
        &self,
        project: &str,
        target: Option<&Target>,
        focus_session: Option<&str>,
        notests: bool,
    ) -> Result<Value> {
        self.changes_catalog_with_agent(project, target, focus_session, None, notests)
    }

    pub fn changes_catalog_with_agent(
        &self,
        project: &str,
        target: Option<&Target>,
        focus_session: Option<&str>,
        agent: Option<&str>,
        notests: bool,
    ) -> Result<Value> {
        let ctx = self.impact_context(project, target, focus_session, agent)?;
        let mut catalog = crate::graph_impact::changed_catalog(
            &ctx.root,
            ctx.index.as_deref(),
            ctx.resolved.as_deref(),
            ctx.live_focus.as_ref(),
            &ctx.run_edited,
            &ctx.target_label,
            notests,
        );
        catalog["project_id"] = json!(project);
        catalog["notests"] = json!(notests);
        Ok(catalog)
    }

    /// Detailed dependency and reverse-dependent change-impact report for one file.
    pub fn file_impact(
        &self,
        project: &str,
        path: &str,
        target: Option<&Target>,
        focus_session: Option<&str>,
        certain: bool,
    ) -> Result<Value> {
        self.file_impact_with_agent(project, path, target, focus_session, None, certain)
    }

    pub fn file_impact_with_agent(
        &self,
        project: &str,
        path: &str,
        target: Option<&Target>,
        focus_session: Option<&str>,
        agent: Option<&str>,
        certain: bool,
    ) -> Result<Value> {
        if path.trim().is_empty() || path.len() > 1024 || path.chars().any(char::is_control) {
            return Err(HarnessError::invalid("Invalid code graph file path."));
        }
        let ctx = self.impact_context(project, target, focus_session, agent)?;
        let mut report = crate::graph_impact::file_impact(
            &ctx.root,
            ctx.index.as_deref(),
            ctx.resolved.as_deref(),
            ctx.live_focus.as_ref(),
            path,
            certain,
        )?;
        report["project_id"] = json!(project);
        Ok(report)
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
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE project_id=?1 AND system_run IS NULL AND system_discovery IS NULL AND status IN('queued','running'))",
            [project],
            |row| row.get(0),
        )?)
    }
    fn root(&self, project: &str) -> Result<PathBuf> {
        Ok(self.target_info(project)?.0)
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
            let published =
                setting["built"].is_object() && live(&self.store.state, project).is_file();
            if published && (now.is_none() || setting["built"]["print"].as_str() == now) {
                return Ok(());
            }
            if setting["failure"].is_object() && setting["failure"]["print"].as_str() == now {
                return Ok(());
            }
        }
        if !asked {
            if trigger != Trigger::Finish
                && lock(&self.finished)?
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
        if self.stopping.load(Ordering::Acquire) {
            return Ok(());
        }
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
                let target_root = setting.get("target_root").cloned();
                let target_name = setting.get("target_name").cloned();
                *setting = json!({"enabled": false, "disabled_by_user": true});
                if let Some(root) = target_root {
                    setting["target_root"] = root;
                }
                if let Some(name) = target_name {
                    setting["target_name"] = name;
                }
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
        self.graph_at(
            project,
            root,
            resolved.revision.as_deref(),
            &resolved.prefix,
            false,
        )
    }
    fn graph_at(
        &self,
        project: &str,
        root: &Path,
        commit: Option<&str>,
        prefix: &str,
        exact: bool,
    ) -> Result<Found> {
        let ready = |kept: String, graph: PathBuf, base: PathBuf| Found {
            state: "ready",
            build: Value::Null,
            failure: None,
            graph: Some((kept, graph, base)),
        };
        let published = live(&self.store.state, project);
        let Some(commit) = commit else {
            return Ok(ready(project.to_owned(), published, root.to_owned()));
        };
        let own = revision(&self.store.state, project, commit);
        if own.is_file() {
            let tree = own.with_file_name("tree");
            return Ok(ready(
                format!("{project}@{commit}"),
                own,
                if prefix.is_empty() {
                    tree
                } else {
                    tree.join(prefix)
                },
            ));
        }
        let setting = self.setting(project)?;
        if !exact && self.current(project, &setting, commit) {
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
            match lock(&self.refused)?.get(&(project.to_owned(), commit.to_owned())) {
                Some(message) => waiting("failed", Value::Null, Some(message.clone())),
                None => waiting("missing", Value::Null, None),
            },
        )
    }
    /// Reads what a run did to the files of its project from the steps its
    /// session recorded, as the run's own receipt counts them: the files it
    /// changed, each with what was done to it, and the files it opened. A
    /// session of another project is no run of this one.
    fn ran(&self, project: &str, root: &Path, session: &str) -> Result<RunChange> {
        Ok(self.run_launches(project, root, session, None, None)?.0)
    }

    fn run_launches(
        &self,
        project: &str,
        root: &Path,
        session: &str,
        launch: Option<&str>,
        compare: Option<&str>,
    ) -> Result<(RunChange, Option<RunChange>)> {
        let record = self.store.get(session)?;
        if record["project_id"] != project {
            return Err(HarnessError::missing("This project has no such run."));
        }
        let ledger = || {
            crate::run_activity::RunLedger::new(
                record["provider"].as_str().unwrap_or(""),
                None,
                false,
            )
        };
        let mut primary = (ledger(), BTreeMap::new());
        let mut followup = compare.map(|_| (ledger(), BTreeMap::new()));
        let workspace = Path::new(record["project_path"].as_str().unwrap_or(""));
        let mut launches = Vec::new();
        let mut known = BTreeSet::new();
        let mut after = 0;
        loop {
            let events = self.store.events(session, after)?;
            let Some(last) = events.last().and_then(|event| event["id"].as_i64()) else {
                break;
            };
            for event in &events {
                let id = event["launch_id"]
                    .as_str()
                    .filter(|value| !value.is_empty());
                if let Some(id) = id
                    && known.insert(id.to_owned())
                {
                    launches.push(json!({
                        "id": id, "number": launches.len() + 1, "at": event["at"],
                    }));
                }
                let selected = if launch.is_none() || launch == id {
                    Some(&mut primary)
                } else if compare == id {
                    followup.as_mut()
                } else {
                    None
                };
                if let Some((ledger, reads)) = selected {
                    if event["targets"] == "limited" {
                        ledger.limited = true;
                    }
                    if event["kind"] == "tool" {
                        ledger.observe(event, event["state"].is_string().then_some(event));
                        if let Some((id, files)) = shell_reads::completed(event, workspace) {
                            reads.extend(files.into_iter().map(|file| (file, id)));
                        }
                    }
                }
            }
            after = last;
        }
        if launch
            .into_iter()
            .chain(compare)
            .any(|id| !known.contains(id))
        {
            return Err(HarnessError::missing(
                "This session has no such recorded launch.",
            ));
        }
        let mut primary = Self::run_change(root, session, &record, primary);
        primary.told["launches"] = json!(launches);
        primary.told["launch"] = json!(launch);
        primary.told["compare_launch"] = json!(compare);
        Ok((
            primary,
            followup.map(|run| Self::run_change(root, session, &record, run)),
        ))
    }

    fn run_change(
        root: &Path,
        session: &str,
        record: &Value,
        (ledger, shell_reads): (crate::run_activity::RunLedger, BTreeMap<String, u64>),
    ) -> RunChange {
        let plain = |path: &str| path.replace('\\', "/").trim_start_matches("./").to_owned();
        let (opened, edited) = ledger.files();
        let opened = opened.into_iter().map(plain).collect::<BTreeSet<_>>();
        let explicit_opened = opened.len();
        let opened = opened
            .into_iter()
            .chain(shell_reads.keys().cloned())
            .collect::<BTreeSet<_>>();
        let files = edited
            .into_iter()
            .map(|(path, status)| graph_changes::Change {
                path: plain(path),
                status,
                from: None,
            })
            .collect::<Vec<_>>();
        let receipt = ledger.receipt();
        let count = |key: &str| receipt[key].as_u64().unwrap_or(0);
        // A run in a folder of its own, as a worktree, names its files as
        // the project does; the graph is the project's all the same.
        let folder = record["project_path"].as_str().unwrap_or("");
        let elsewhere = !folder.is_empty() && Path::new(folder) != root;
        let title = record["title"].as_str().unwrap_or("");
        let told = json!({
            "session": session,
            "title": title,
            "provider": record["provider"],
            "status": record["status"],
            "steps": receipt["steps"],
            "opened": opened.len(),
            "edited": files.len(),
            "shell_opened": shell_reads.len(),
            "outside": (count("opened") + count("edited"))
                .saturating_sub((explicit_opened + files.len()) as u64),
            "searched": receipt["searched"],
            "commands": receipt["commands"],
            "helpers": receipt["helpers"],
            "limited": receipt["limited"],
            "elsewhere": elsewhere,
        });
        RunChange {
            resolved: Arc::new(Resolved {
                target: Target::Run(session.to_owned()),
                kind: "run",
                name: title.chars().take(80).collect(),
                title: None,
                pull: None,
                head: None,
                base: None,
                revision: None,
                prefix: String::new(),
                files,
                notes: Vec::new(),
            }),
            opened,
            shell_reads,
            told,
        }
    }
    /// Says in a picture of a folder's layers what the chosen change did
    /// to their order: the pairs of files that run against it now and did
    /// not where the change started, and those that did and no longer do.
    /// The graph of that base is one of the cached graphs of commits. Until
    /// it is built the picture says that it is being prepared, and where
    /// the base cannot be told or its graph cannot be built, that the
    /// comparison is not to be had.
    fn compare_layers(
        &self,
        project: &str,
        root: &Path,
        resolved: &Resolved,
        head: &crate::graph_map::Index,
        view: &mut Value,
        shown: &Shown,
    ) {
        let base_side = resolved.json()["base"].clone();
        let compare = |view: &mut Value| -> Result<Value> {
            let Some(base) = &resolved.base else {
                return Ok(json!({"state": "unavailable", "reason": "no_base"}));
            };
            let found =
                self.graph_at(project, root, Some(&base.commit), &resolved.prefix, false)?;
            let Some((key, graph, tree)) = found.graph else {
                if found.state == "failed" {
                    return Ok(json!({"state": "unavailable"}));
                }
                if found.state == "missing" {
                    // Beside the graph of a revision the base is kept as
                    // its pair; the checkout's own work has no revision.
                    self.revise(
                        project,
                        root,
                        &base.commit,
                        &resolved.prefix,
                        match &resolved.revision {
                            Some(head) => RevisionBuild::Comparison { head },
                            None => RevisionBuild::Normal,
                        },
                    )?;
                }
                return Ok(json!({"state": "building"}));
            };
            let base = self.maps.index(&key, &graph, &tree)?;
            // The project's own graph, where it stands for the commit, is
            // read with the files of the checkout; the graph of a commit
            // tells its tests by the spans kept beside it.
            if shown.notests
                && key.contains('@')
                && base.has_rust_sources()
                && !base.has_test_spans()
            {
                return Ok(json!({"state": "unavailable", "reason": "tests"}));
            }
            let base = if shown.notests {
                base.without_tests()
            } else {
                base
            };
            let renamed = resolved
                .files
                .iter()
                .filter_map(|file| {
                    (file.status == 'R').then_some((file.path.as_str(), file.from.as_deref()?))
                })
                .collect();
            head.compare_layers(&base, view, &renamed, shown.certain);
            Ok(view["pattern"]["change"].take())
        };
        let mut change = compare(view).unwrap_or_else(|_| json!({"state": "unavailable"}));
        change["base"] = base_side;
        view["pattern"]["change"] = change;
    }
    #[allow(clippy::too_many_arguments)]
    fn compare_pr_cycle(
        &self,
        project: &str,
        root: &Path,
        resolved: &Resolved,
        raw_head: &crate::graph_map::Index,
        head: &crate::graph_map::Index,
        snapshot: bool,
        view: &mut Value,
        shown: &Shown,
    ) {
        let base_side = resolved.json()["base"].clone();
        let mut compare = || -> Result<Value> {
            let (Some(base), Some(head_commit)) = (&resolved.base, &resolved.revision) else {
                return Ok(json!({"state": "unavailable", "reason": "no_base"}));
            };
            // Keep ordinary PR views able to reuse the matching live graph.
            // Comparison needs a snapshot so its source filters describe
            // the revision even if the checkout subsequently changes.
            if !snapshot {
                let found =
                    self.graph_at(project, root, Some(head_commit), &resolved.prefix, true)?;
                if found.state == "failed" {
                    return Ok(
                        json!({"state": "unavailable", "reason": "head", "base": base_side}),
                    );
                }
                if found.state == "missing" {
                    self.revise(
                        project,
                        root,
                        head_commit,
                        &resolved.prefix,
                        RevisionBuild::Comparison { head: head_commit },
                    )?;
                }
                return Ok(json!({"state": "waiting_head", "base": base_side}));
            }
            let revision = &base.commit;
            let found = self.graph_at(project, root, Some(revision), &resolved.prefix, true)?;
            let Some((key, graph, tree)) = found.graph else {
                if found.state == "failed" {
                    return Ok(json!({"state": "unavailable", "base": base_side}));
                }
                if found.state == "missing" {
                    self.revise(
                        project,
                        root,
                        &base.commit,
                        &resolved.prefix,
                        RevisionBuild::Comparison { head: head_commit },
                    )?;
                }
                return Ok(json!({"state": "building", "base": base_side}));
            };
            let base = self.maps.index(&key, &graph, &tree)?;
            if shown.notests
                && ((snapshot && raw_head.has_rust_sources() && !raw_head.has_test_spans())
                    || (base.has_rust_sources() && !base.has_test_spans()))
            {
                return Ok(json!({"state": "unavailable", "reason": "tests", "base": base_side}));
            }
            let base = if shown.notests {
                base.without_tests()
            } else {
                base
            };
            let renames = resolved
                .files
                .iter()
                .filter_map(|file| {
                    (file.status == 'R').then_some((file.path.as_str(), file.from.as_deref()?))
                })
                .collect();
            let (origin, place) =
                head.compare_cycle(raw_head, &base, view, &renames, shown.certain);
            Ok(json!({
                "state": "ready", "base": base_side, "origin": origin, "place": place,
                "revision": revision,
            }))
        };
        let result =
            compare().unwrap_or_else(|_| json!({"state": "unavailable", "base": base_side}));
        view["cycle"]["comparison"] = result;
    }
    /// Works out what a target changed and, for a commit that is not the
    /// checkout, starts building its graph. A pull request is looked up and
    /// fetched here, so callers keep this off the runtime.
    pub fn choose(&self, project: &str, target: &Target) -> Result<Value> {
        let root = self.root(project)?;
        let resolved = match target {
            Target::Run(session) => self.ran(project, &root, session)?.resolved,
            Target::SysRun(run_id) => {
                crate::system_graph::sysrun_launches(
                    &self.store,
                    project,
                    &root,
                    run_id,
                    None,
                    None,
                    None,
                )?
                .0
                .resolved
            }
            _ => Arc::new(graph_changes::resolve(
                &root,
                target,
                &self.listed(project, &root),
            )?),
        };
        lock(&self.chosen)?.insert((project.to_owned(), target.text()), resolved.clone());
        if let Some(commit) = &resolved.revision {
            self.revise(
                project,
                &root,
                commit,
                &resolved.prefix,
                RevisionBuild::Normal,
            )?;
        }
        let found = self.graph_of(project, &root, &resolved)?;
        let message = revision_message(found.state, &found.build, found.failure.as_deref());
        Ok(json!({
            "project_id": project,
            "target": target.text(),
            "chosen": chosen(&resolved),
            "state": found.state,
            "message": message,
            "build": found.build,
            "failure": found.failure,
        }))
    }
    /// Starts the graph of a commit unless it exists or is being built. A
    /// build for another commit of the project gives way: the last one asked
    /// for is the one being looked at.
    fn revise(
        &self,
        project: &str,
        root: &Path,
        commit: &str,
        prefix: &str,
        build: RevisionBuild<'_>,
    ) -> Result<()> {
        if commit.len() < 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(HarnessError::invalid("That is not a commit."));
        }
        if self.stopping.load(Ordering::Acquire)
            || revision(&self.store.state, project, commit).is_file()
            || (matches!(build, RevisionBuild::Normal)
                && self.current(project, &self.setting(project)?, commit))
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
        let job = {
            let mut job = Job::new();
            job.stage.store(WAITING, Ordering::Release);
            let mut revisions = lock(&self.revisions)?;
            if revision(&self.store.state, project, commit).is_file() {
                return Ok(());
            }
            if let Some((building, running)) = revisions.get(project) {
                if building == commit || matches!(build, RevisionBuild::Comparison { .. }) {
                    return Ok(());
                }
                job.revision_gate = running.revision_gate.clone();
                running.cancel.store(true, Ordering::Release);
            }
            let job = Arc::new(job);
            revisions.insert(project.to_owned(), (commit.to_owned(), job.clone()));
            job
        };
        lock(&self.refused)?.remove(&(project.to_owned(), commit.to_owned()));
        let (name, root, commit, prefix) = (
            project.to_owned(),
            root.to_owned(),
            commit.to_owned(),
            prefix.to_owned(),
        );
        let running = job.clone();
        let paired = match build {
            RevisionBuild::Comparison { head } => Some(head.to_owned()),
            _ => None,
        };
        let worker = std::thread::Builder::new()
            .name("harness-graph-revision".into())
            .spawn(move || {
                let _gate = running
                    .revision_gate
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let outcome = me.grow(&name, &root, &tool, &commit, &prefix, &running);
                me.finish(&name, &commit, &running, outcome, &tool, paired.as_deref());
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
    /// project's graph, the tool reads them as it reads the project, and of
    /// them only what the graph is read with is kept: see `thin`.
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
        if own.is_file() {
            return Outcome::Cancelled;
        }
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
            if let Some(tests) = thin(&tree) {
                let prefix = if prefix.is_empty() {
                    String::new()
                } else {
                    format!("{prefix}/")
                };
                let spans = tests
                    .iter()
                    .filter_map(|(path, spans)| Some((path.strip_prefix(&prefix)?, spans)))
                    .collect::<BTreeMap<_, _>>();
                if let Ok(spans) = serde_json::to_vec(&spans) {
                    let file = own.with_file_name(crate::graph_map::TEST_SPANS);
                    if std::fs::write(&file, spans).is_err() {
                        let _ = std::fs::remove_file(file);
                    }
                }
            }
            if stopped() {
                return Outcome::Cancelled;
            }
            if std::fs::rename(&ran.fresh, &own).is_err() {
                return Outcome::Failed("The new graph could not be published.".into());
            }
            Outcome::Built(ran.record(ran.bytes))
        })();
        // The files and the tool's cache are as large as the project. What
        // the graph is read with is not.
        let _ = std::fs::remove_dir_all(&out);
        if !matches!(outcome, Outcome::Built(_)) {
            let _ = std::fs::remove_dir_all(&home);
        }
        outcome
    }
    fn finish(
        &self,
        project: &str,
        commit: &str,
        job: &Arc<Job>,
        outcome: Outcome,
        tool: &Tool,
        paired: Option<&str>,
    ) {
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
                self.prune(project, commit, paired);
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
        // The graph was deleted while this build was ending: the folder
        // that was left to it goes now.
        if !purge && job.purge.load(Ordering::Acquire) {
            let _ = std::fs::remove_dir_all(folder(&self.store.state, project));
        }
    }
    /// Keeps the newest graphs of other commits and removes the rest.
    fn prune(&self, project: &str, keep: &str, paired: Option<&str>) {
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
                Some((name == keep || Some(name.as_str()) == paired, changed, name))
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
        let build = job.map(|job| job.view(history));
        let failure = if setting["failure"].is_object() {
            Some(json!({"message": setting["failure"]["message"], "at": setting["failure"]["at"]}))
        } else {
            None
        };
        let message = status_message(
            state,
            build.as_ref(),
            failure.as_ref().and_then(|value| value["message"].as_str()),
        );
        Ok(json!({
            "project_id": project,
            "enabled": enabled,
            // Whether launches are told about the graph: a choice of its own.
            "agents": setting["agents"] == true,
            "state": state,
            "message": message,
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
            "build": build,
            "failure": failure,
        }))
    }
    pub fn act(&self, project: &str, action: &str) -> Result<Value> {
        self.root(project)?;
        match action {
            "enable" => {
                self.edit(project, |setting| {
                    setting["enabled"] = json!(true);
                    if let Some(fields) = setting.as_object_mut() {
                        fields.remove("disabled_by_user");
                    }
                })?;
                self.refresh(project, Trigger::Enabled)?;
            }
            "disable" => {
                self.edit(project, |setting| {
                    setting["enabled"] = json!(false);
                    setting["disabled_by_user"] = json!(true);
                })?;
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
                self.edit(project, |setting| {
                    let target_root = setting.get("target_root").cloned();
                    let target_name = setting.get("target_name").cloned();
                    *setting = json!({"enabled": false, "disabled_by_user": true});
                    if let Some(root) = target_root {
                        setting["target_root"] = root;
                    }
                    if let Some(name) = target_name {
                        setting["target_name"] = name;
                    }
                })?;
                // A commit's graph that is still being built writes into
                // the folder. It is told to end first, and removes the
                // folder when it has ended: it is told while it is known to
                // run, so it cannot end without having heard.
                let growing = lock(&self.revisions)?
                    .get(project)
                    .map(|(_, job)| {
                        job.purge.store(true, Ordering::Release);
                        job.cancel.store(true, Ordering::Release);
                    })
                    .is_some();
                let running = lock(&self.jobs)?.get(project).cloned();
                match running {
                    // The build owns the folder until it has ended.
                    Some(job) => {
                        job.purge.store(true, Ordering::Release);
                        job.cancel.store(true, Ordering::Release);
                    }
                    None => {
                        let home = folder(&self.store.state, project);
                        // Under a commit's build the folder may not go at
                        // once: what that build still writes goes with it.
                        if home.exists()
                            && let Err(error) = std::fs::remove_dir_all(home)
                            && !growing
                        {
                            return Err(error.into());
                        }
                    }
                }
                lock(&self.chosen)?.retain(|(name, _), _| name != project);
                lock(&self.refused)?.retain(|(name, _), _| name != project);
                self.maps.forget(project);
                lock(&self.history)?.remove(project);
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
                "graph.auto_start" => {
                    if !project.is_empty()
                        && let Some(me) = self.me.upgrade()
                    {
                        let project = project.to_owned();
                        tokio::task::spawn_blocking(move || me.auto_start(&project))
                            .await
                            .map_err(|_| {
                                HarnessError::unavailable("The code graph could not start.")
                            })??;
                    }
                    json!({"ok": true})
                }
                "graph.auto_start_all" => {
                    if let Some(me) = self.me.upgrade() {
                        me.auto_start_projects();
                    }
                    json!({"ok": true})
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
                    let focus_session = text("focus_session")?;
                    if focus_session.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid code graph session."));
                    }
                    let flag = |name: &str| match query[name].as_str() {
                        None | Some("0") => Ok(false),
                        Some("1") => Ok(true),
                        Some(_) => Err(HarnessError::invalid("Invalid graph filter.")),
                    };
                    let (certain, notests) = (flag("certain")?, flag("notests")?);
                    let patterns = flag("patterns")?;
                    let history = flag("history")?;
                    let modules = flag("modules")?;
                    let no_incoming = flag("no_incoming")?;
                    let whole = flag("whole")?;
                    let origin = text("scenario")?;
                    let launch = text("launch")?;
                    let compare_launch = text("compare_launch")?;
                    let agent = text("agent")?;
                    if agent.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid code graph agent."));
                    }
                    let target = match query["target"].as_str() {
                        None | Some("") => None,
                        Some(text) => Some(Target::parse(text)?),
                    };
                    let followed = Followed::parse(
                        &text("follow")?,
                        query["out"].as_str().unwrap_or(""),
                        query["into"].as_str().unwrap_or(""),
                    )?;
                    let spread = places(
                        query["spread"].as_str().unwrap_or(""),
                        "Too many folders are open here; close some first.",
                        "Invalid folders to open here.",
                    )?;
                    // A link is named by its two ends, and by both.
                    let (from, to) = (text("from")?, text("to")?);
                    if from.is_empty() != to.is_empty() {
                        return Err(HarnessError::invalid("Invalid code graph link."));
                    }
                    // So is a path: a list of the place it leads from and
                    // the place it leads to.
                    let ends = places(
                        query["path"].as_str().unwrap_or(""),
                        "Invalid code graph path.",
                        "Invalid code graph path.",
                    )?;
                    if !ends.is_empty() && ends.len() != 2 {
                        return Err(HarnessError::invalid("Invalid code graph path."));
                    }
                    // And what two places share: a list of the two, asked
                    // for instead of the path between them.
                    let sharing = places(
                        query["shared"].as_str().unwrap_or(""),
                        "Invalid code graph places.",
                        "Invalid code graph places.",
                    )?;
                    if !sharing.is_empty() && (sharing.len() != 2 || !ends.is_empty()) {
                        return Err(HarnessError::invalid("Invalid code graph places."));
                    }
                    let cycle = text("cycle")?;
                    let base = text("base")?;
                    if !base.is_empty()
                        && (!matches!(base.len(), 40 | 64)
                            || !base.bytes().all(|byte| byte.is_ascii_hexdigit()))
                    {
                        return Err(HarnessError::invalid("Invalid pull request base."));
                    }
                    if !cycle.is_empty()
                        && (cycle.chars().any(char::is_control)
                            || !from.is_empty()
                            || !ends.is_empty()
                            || !sharing.is_empty()
                            || followed.is_some())
                    {
                        return Err(HarnessError::invalid("Invalid code graph cycle."));
                    }
                    // One found pattern is asked for by itself.
                    let pattern = text("pattern")?;
                    if !pattern.is_empty()
                        && (pattern.chars().any(char::is_control)
                            || !from.is_empty()
                            || !ends.is_empty()
                            || !sharing.is_empty()
                            || !cycle.is_empty()
                            || followed.is_some())
                    {
                        return Err(HarnessError::invalid("Invalid code graph pattern."));
                    }
                    if !origin.is_empty()
                        && (modules
                            || !base.is_empty()
                            || !from.is_empty()
                            || !ends.is_empty()
                            || !sharing.is_empty()
                            || !cycle.is_empty()
                            || !pattern.is_empty()
                            || followed.is_some())
                    {
                        return Err(HarnessError::invalid("Invalid scenario view."));
                    }
                    // Of what they share one kind is drawn, and a request
                    // may say which.
                    let users = match query["side"].as_str() {
                        None | Some("") => None,
                        Some("both_use") if !sharing.is_empty() => Some(false),
                        Some("uses_both") if !sharing.is_empty() => Some(true),
                        Some(_) => {
                            return Err(HarnessError::invalid("Invalid code graph places."));
                        }
                    };
                    // The picture begins after as many of the nearest as
                    // a request says.
                    let skip = match query["skip"].as_str() {
                        None | Some("") => 0,
                        Some(text) if !sharing.is_empty() || !origin.is_empty() => text
                            .parse::<usize>()
                            .ok()
                            .filter(|skip| *skip <= 1_000_000)
                            .ok_or_else(|| HarnessError::invalid("Invalid code graph places."))?,
                        Some(_) => {
                            return Err(HarnessError::invalid("Invalid code graph places."));
                        }
                    };
                    let me = self.me.upgrade().ok_or_else(|| {
                        HarnessError::unavailable("The code graph is shutting down.")
                    })?;
                    let project = project.to_owned();
                    tokio::task::spawn_blocking(move || {
                        me.map(
                            &project,
                            target.as_ref(),
                            &Shown {
                                scope: &scope,
                                focus: (!focus.is_empty()).then_some(focus.as_str()),
                                focus_session: (!focus_session.is_empty())
                                    .then_some(focus_session.as_str()),
                                certain,
                                filter: &filter,
                                followed: followed.as_ref(),
                                notests,
                                spread: &spread,
                                link: (!from.is_empty()).then_some((from.as_str(), to.as_str())),
                                path: match ends.as_slice() {
                                    [from, to] => Some((from.as_str(), to.as_str())),
                                    _ => None,
                                },
                                cycle: (!cycle.is_empty()).then_some(cycle.as_str()),
                                base: (!base.is_empty()).then_some(base.as_str()),
                                shared: match sharing.as_slice() {
                                    [from, to] => Some(Sharing {
                                        from,
                                        to,
                                        users,
                                        skip,
                                    }),
                                    _ => None,
                                },
                                patterns,
                                history,
                                modules,
                                no_incoming,
                                launch: (!launch.is_empty()).then_some(launch.as_str()),
                                compare_launch: (!compare_launch.is_empty())
                                    .then_some(compare_launch.as_str()),
                                agent: (!agent.is_empty()).then_some(agent.as_str()),
                                scenario: (!origin.is_empty()).then_some((origin.as_str(), skip)),
                                pattern: (!pattern.is_empty()).then_some(pattern.as_str()),
                                whole,
                            },
                        )
                    })
                    .await
                    .map_err(|_| {
                        HarnessError::unavailable("The code graph map could not be read.")
                    })??
                }
                "graph.find" => {
                    let query = &request["query"];
                    // What a person typed into the field: one line of a
                    // hundred letters at most, of any script.
                    let wanted = query["query"].as_str().unwrap_or("");
                    if wanted.len() > 400 || wanted.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid search."));
                    }
                    let notests = match query["notests"].as_str() {
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
                    let (project, wanted) = (project.to_owned(), wanted.to_owned());
                    tokio::task::spawn_blocking(move || {
                        me.find(&project, target.as_ref(), &wanted, notests)
                    })
                    .await
                    .map_err(|_| {
                        HarnessError::unavailable("The code graph could not be searched.")
                    })??
                }
                "graph.changes" | "graph.file_impact" => {
                    let query = &request["query"];
                    let flag = |name: &str| match query[name].as_str() {
                        None | Some("0") => Ok(false),
                        Some("1") => Ok(true),
                        Some(_) => Err(HarnessError::invalid("Invalid graph filter.")),
                    };
                    let target = match query["target"].as_str() {
                        None | Some("") => None,
                        Some(text) => Some(Target::parse(text)?),
                    };
                    let focus_session = query["focus_session"].as_str().unwrap_or("");
                    if focus_session.len() > 1024 || focus_session.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid code graph session."));
                    }
                    let focus_session =
                        (!focus_session.trim().is_empty()).then(|| focus_session.trim().to_owned());
                    let agent = query["agent"].as_str().unwrap_or("");
                    if agent.len() > 256 || agent.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid code graph agent."));
                    }
                    let agent = (!agent.trim().is_empty()).then(|| agent.trim().to_owned());
                    let is_impact = operation == "graph.file_impact";
                    let path = if is_impact {
                        let value = query["path"].as_str().unwrap_or("");
                        if value.trim().is_empty()
                            || value.len() > 1024
                            || value.chars().any(char::is_control)
                        {
                            return Err(HarnessError::invalid("Invalid code graph file path."));
                        }
                        value.to_owned()
                    } else {
                        String::new()
                    };
                    let toggle = if is_impact {
                        flag("certain")?
                    } else {
                        flag("notests")?
                    };
                    let me = self.me.upgrade().ok_or_else(|| {
                        HarnessError::unavailable("The code graph is shutting down.")
                    })?;
                    let project = project.to_owned();
                    tokio::task::spawn_blocking(move || {
                        if is_impact {
                            me.file_impact_with_agent(
                                &project,
                                &path,
                                target.as_ref(),
                                focus_session.as_deref(),
                                agent.as_deref(),
                                toggle,
                            )
                        } else {
                            me.changes_catalog_with_agent(
                                &project,
                                target.as_ref(),
                                focus_session.as_deref(),
                                agent.as_deref(),
                                toggle,
                            )
                        }
                    })
                    .await
                    .map_err(|_| {
                        HarnessError::unavailable("The code graph could not be inspected.")
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
    fn what_is_followed_is_read_from_a_request() {
        assert_eq!(Followed::parse("", "[\"src/a.php\"]", "[]").unwrap(), None);
        // A place named twice is unfolded two steps.
        assert_eq!(
            Followed::parse(
                "src/a.php",
                "[\"src/a.php\",\"src/b, c.php\",\"src/a.php\"]",
                ""
            )
            .unwrap(),
            Some(Followed {
                root: "src/a.php".to_owned(),
                out: vec![
                    "src/a.php".to_owned(),
                    "src/b, c.php".to_owned(),
                    "src/a.php".to_owned()
                ],
                into: vec![],
            })
        );
        let many = serde_json::to_string(&vec!["x"; UNFOLDED + 1]).unwrap();
        let long = serde_json::to_string(&["x".repeat(1025)]).unwrap();
        for wrong in [
            many.as_str(),
            long.as_str(),
            "src/a.php",
            "[1]",
            "[\"\"]",
            "{}",
        ] {
            assert!(Followed::parse("src/a.php", wrong, "").is_err(), "{wrong}");
            assert!(Followed::parse("src/a.php", "", wrong).is_err(), "{wrong}");
        }
        // The map's route lets through every part of a request its handler
        // reads; a part it does not name is refused before the handler.
        let route = crate::http::ROUTES
            .iter()
            .find(|route| route.operation == "graph.map")
            .unwrap();
        for part in [
            "scope",
            "focus",
            "certain",
            "filter",
            "target",
            "follow",
            "out",
            "into",
            "notests",
            "spread",
            "from",
            "to",
            "path",
            "shared",
            "side",
            "skip",
            "cycle",
            "base",
            "patterns",
            "pattern",
            "history",
            "modules",
            "no_incoming",
            "launch",
            "compare_launch",
            "scenario",
            "focus_session",
            "whole",
        ] {
            assert!(route.query.contains(&part), "{part}");
        }
        // So does the route of the search.
        let route = crate::http::ROUTES
            .iter()
            .find(|route| route.operation == "graph.find")
            .unwrap();
        assert_eq!(
            (route.method, route.pattern, route.query),
            (
                "GET",
                "/api/projects/{project_id}/graph/find",
                &["query", "target", "notests"][..]
            )
        );
        let changes_route = crate::http::ROUTES
            .iter()
            .find(|route| route.operation == "graph.changes")
            .unwrap();
        assert_eq!(
            (
                changes_route.method,
                changes_route.pattern,
                changes_route.status,
                changes_route.token,
                changes_route.query
            ),
            (
                "GET",
                "/api/projects/{project_id}/graph/changes",
                200,
                false,
                &["target", "focus_session", "notests"][..]
            )
        );
        let impact_route = crate::http::ROUTES
            .iter()
            .find(|route| route.operation == "graph.file_impact")
            .unwrap();
        assert_eq!(
            (
                impact_route.method,
                impact_route.pattern,
                impact_route.status,
                impact_route.token,
                impact_route.query
            ),
            (
                "GET",
                "/api/projects/{project_id}/graph/file-impact",
                200,
                false,
                &["path", "target", "focus_session", "certain"][..]
            )
        );
        // The folders spread out in a picture come as the same kind of list.
        assert_eq!(
            places("[\"src\",\"src/b, c\"]", "many", "wrong").unwrap(),
            ["src", "src/b, c"]
        );
        assert!(places("", "many", "wrong").unwrap().is_empty());
        assert!(places(&many, "many", "wrong").is_err());
        assert!(places("src", "many", "wrong").is_err());
    }
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
                // The tests run on threads of one process, and a child that
                // another test starts meanwhile inherits whatever is open
                // here until it execs. Were the tool written here, such a
                // child could hold it open for writing, and starting the
                // tool then fails with "Text file busy". So its text is
                // written beside it, and a process of its own makes the
                // tool of that.
                let text = tool.with_extension("text");
                std::fs::write(&text, TOOL).unwrap();
                let copied = Command::new("cp")
                    .arg("--")
                    .arg(&text)
                    .arg(&tool)
                    .status()
                    .unwrap();
                assert!(copied.success());
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
                        .map(&self.project, Some(target), &Shown::default())
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
        fn a_pr_base_cycle_and_its_links_use_the_pinned_revision() {
            let fixture = Fixture::new();
            let (base, head) = fixture.branched();
            let target = Target::Pull(1);
            let resolved = Resolved {
                target: target.clone(),
                kind: "pull",
                name: "#1".to_owned(),
                title: None,
                pull: Some(1),
                head: None,
                base: Some(graph_changes::Side {
                    name: "main".to_owned(),
                    commit: base.clone(),
                }),
                revision: Some(head.clone()),
                prefix: String::new(),
                files: Vec::new(),
                notes: Vec::new(),
            };
            lock(&fixture.graphs.chosen)
                .unwrap()
                .insert((fixture.project.clone(), target.text()), Arc::new(resolved));
            let put = |commit: &str, edges: &[(&str, &str, &str)]| {
                let graph = revision(&fixture.store.state, &fixture.project, commit);
                std::fs::create_dir_all(graph.parent().unwrap()).unwrap();
                let links = edges.iter().map(|(from, to, confidence)| {
                    json!({"source": from, "target": to, "relation": "calls", "confidence": confidence})
                }).collect::<Vec<_>>();
                std::fs::write(graph, json!({
                    "nodes": (["a", "b", "c"].map(|node| json!({"id": node, "source_file": format!("{node}.php")}))),
                    "links": links,
                }).to_string()).unwrap();
            };
            put(
                &base,
                &[
                    ("a", "b", "EXTRACTED"),
                    ("b", "c", "EXTRACTED"),
                    ("c", "a", "INFERRED"),
                ],
            );
            put(&head, &[("a", "b", "EXTRACTED"), ("b", "a", "EXTRACTED")]);
            let map = |shown| fixture.graphs.map(&fixture.project, Some(&target), &shown);
            let current = map(Shown {
                cycle: Some("a.php"),
                ..Shown::default()
            })
            .unwrap();
            assert_eq!(current["cycle"]["steps"], 2);
            let comparison = &current["cycle"]["comparison"];
            assert_eq!(comparison["revision"], base);
            assert_eq!(comparison["place"], "a.php");
            let historical = Shown {
                cycle: Some("a.php"),
                base: Some(&base),
                notests: true,
                ..Shown::default()
            };
            let earlier = map(historical).unwrap();
            assert_eq!(earlier["base"]["commit"], &base[..12]);
            assert_eq!(earlier["cycle"]["steps"], 3);
            assert_eq!(earlier["notests"], true);
            assert_eq!(
                map(Shown {
                    certain: true,
                    ..historical
                })
                .unwrap()["cycle"]["steps"],
                0
            );
            let link = Shown {
                link: Some(("b.php", "c.php")),
                ..Shown::default()
            };
            assert_eq!(map(link).unwrap()["links"], 0);
            assert_eq!(
                map(Shown {
                    base: Some(&base),
                    ..link
                })
                .unwrap()["links"],
                1
            );
            assert!(
                map(Shown {
                    base: Some(&head),
                    ..historical
                })
                .is_err()
            );
            assert!(
                fixture
                    .graphs
                    .map(&fixture.project, None, &historical)
                    .is_err()
            );
            std::fs::remove_file(revision(&fixture.store.state, &fixture.project, &base)).unwrap();
            assert!(map(historical).is_err());
        }
        /// A shop in three layers, one type in each file.
        const SHOP: [(&str, &str); 5] = [
            ("controller", "src/Controller/OrderController.php"),
            ("service", "src/Service/OrderService.php"),
            ("mailer", "src/Service/MailService.php"),
            ("order", "src/Entity/Order.php"),
            ("line", "src/Entity/Line.php"),
        ];
        /// What the shop needs in the order of its layers.
        const NEEDS: [(&str, &str); 3] = [
            ("controller", "service"),
            ("service", "order"),
            ("service", "line"),
        ];
        /// A graph of types, each named with its file, and the calls between
        /// them. A name may carry the line it stands at, after `:`, and a
        /// call that ends in `?` is one the tool only inferred.
        fn sketch(files: &[(&str, &str)], calls: &[(&str, &str)]) -> String {
            let nodes = files
                .iter()
                .map(|(id, path)| match id.split_once(':') {
                    Some((id, line)) => json!({
                        "id": id, "source_file": path, "source_location": format!("L{line}"),
                    }),
                    None => json!({"id": id, "source_file": path}),
                })
                .collect::<Vec<_>>();
            let links = calls
                .iter()
                .map(|(from, to)| {
                    let (to, confidence) = match to.strip_suffix('?') {
                        Some(to) => (to, "INFERRED"),
                        None => (*to, "EXTRACTED"),
                    };
                    json!({
                        "source": from, "target": to, "relation": "calls",
                        "confidence": confidence,
                    })
                })
                .collect::<Vec<_>>();
            json!({"nodes": nodes, "links": links}).to_string()
        }
        /// Writes such a graph where the graph of a commit is kept.
        fn draw(graph: &Path, files: &[(&str, &str)], calls: &[(&str, &str)]) {
            std::fs::create_dir_all(graph.parent().unwrap()).unwrap();
            std::fs::write(graph, sketch(files, calls)).unwrap();
        }
        /// Pull request 1 as chosen, of `head` over `base`, with the files
        /// it is said to have changed; Git is not asked.
        fn pull(
            fixture: &Fixture,
            base: Option<&str>,
            head: &str,
            files: Vec<graph_changes::Change>,
        ) -> Target {
            let target = Target::Pull(1);
            let resolved = Resolved {
                target: target.clone(),
                kind: "pull",
                name: "#1".to_owned(),
                title: None,
                pull: Some(1),
                head: None,
                base: base.map(|commit| graph_changes::Side {
                    name: "main".to_owned(),
                    commit: commit.to_owned(),
                }),
                revision: Some(head.to_owned()),
                prefix: String::new(),
                files,
                notes: Vec::new(),
            };
            lock(&fixture.graphs.chosen)
                .unwrap()
                .insert((fixture.project.clone(), target.text()), Arc::new(resolved));
            target
        }
        /// Waits for the build of a commit's graph to end. It publishes the
        /// graph first, and only then prunes the graphs of other commits and
        /// gives its place to the next build.
        fn rested(fixture: &Fixture) {
            let started = Instant::now();
            while !lock(&fixture.graphs.revisions).unwrap().is_empty() {
                assert!(started.elapsed() < Duration::from_secs(20));
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        /// The picture of the layers of the whole project.
        fn layers() -> Shown<'static> {
            Shown {
                pattern: Some("layers@"),
                ..Shown::default()
            }
        }
        #[test]
        fn the_layers_of_a_change_say_what_it_did_to_their_order() {
            let fixture = Fixture::new();
            let (base, head) = fixture.branched();
            let at = |commit: &str| revision(&fixture.store.state, &fixture.project, commit);
            // At the base a line of the model asks the mailer; at the head
            // it no longer does, and the order asks the service.
            draw(
                &at(&base),
                &SHOP,
                &[&NEEDS[..], &[("line", "mailer")]].concat(),
            );
            draw(
                &at(&head),
                &SHOP,
                &[&NEEDS[..], &[("order", "service")]].concat(),
            );
            let target = pull(&fixture, Some(&base), &head, Vec::new());
            let map = |shown| fixture.graphs.map(&fixture.project, Some(&target), &shown);
            let view = map(layers()).unwrap();
            assert_eq!(view["mode"], "pattern");
            let change = &view["pattern"]["change"];
            assert_eq!(change["state"], "ready");
            assert_eq!(
                change["base"],
                json!({"name": "main", "commit": &base[..12]})
            );
            assert_eq!(change["added"]["pairs"], 1);
            assert_eq!(
                change["added"]["rows"][0]["from"]["path"],
                "src/Entity/Order.php"
            );
            assert_eq!(change["removed"]["pairs"], 1);
            assert_eq!(
                change["removed"]["rows"][0]["to"]["path"],
                "src/Service/MailService.php"
            );
            assert_eq!(view["pattern"]["against"]["rows"][0]["added"], true);
            // A selected link is compared as the picture is.
            let link = map(Shown {
                focus: Some("layer:model>layer:services"),
                ..layers()
            })
            .unwrap();
            assert_eq!(link["focus"]["rows"][0]["added"], true);
            assert_eq!(link["pattern"]["change"]["added"]["pairs"], 1);
            // Any other picture of the change is not compared.
            let plain = map(Shown::default()).unwrap();
            assert!(plain["pattern"].is_null());
            // Without a base to measure from the picture says that it
            // cannot tell, and stays what it was.
            pull(&fixture, None, &head, Vec::new());
            let view = map(layers()).unwrap();
            assert_eq!(
                view["pattern"]["change"],
                json!({"state": "unavailable", "reason": "no_base", "base": null})
            );
            assert_eq!(view["pattern"]["against"]["pairs"], 1);
            assert!(view["pattern"]["against"]["rows"][0]["added"].is_null());
            // A base whose graph is not there yet is being prepared: the
            // picture says so, and has what it was asked for once the graph
            // is built. The tool gives that commit a graph without layers,
            // so every pair against the order is then the change's own.
            pull(&fixture, Some(&base), &head, Vec::new());
            std::fs::remove_file(at(&base)).unwrap();
            // The graph of the change is the oldest of more graphs than
            // are kept: it stays, as the pair of the base built for it.
            std::fs::File::options()
                .write(true)
                .open(at(&head))
                .unwrap()
                .set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
                .unwrap();
            for other in ["a", "b", "c"] {
                draw(&at(&other.repeat(40)), &SHOP, &NEEDS);
            }
            let waiting = map(layers()).unwrap();
            assert_eq!(waiting["pattern"]["change"]["state"], "building");
            assert_eq!(waiting["pattern"]["change"]["base"]["commit"], &base[..12]);
            assert!(waiting["pattern"]["change"]["added"].is_null());
            let started = Instant::now();
            let ready = loop {
                let view = map(layers()).unwrap();
                if view["pattern"]["change"]["state"] != "building" {
                    break view;
                }
                assert!(started.elapsed() < Duration::from_secs(20), "{view}");
                std::thread::sleep(Duration::from_millis(20));
            };
            assert_eq!(ready["pattern"]["change"]["state"], "ready");
            assert_eq!(ready["pattern"]["change"]["added"]["pairs"], 1);
            assert_eq!(ready["pattern"]["change"]["removed"]["pairs"], 0);
            rested(&fixture);
            let kept = fixture.revisions();
            assert_eq!(kept.len(), REVISIONS, "{kept:?}");
            assert!(kept.contains(&head) && kept.contains(&base), "{kept:?}");
        }
        #[test]
        fn the_layers_of_a_change_are_compared_as_far_as_its_two_ends_can_be_read() {
            let fixture = Fixture::new();
            let (base, head) = fixture.branched();
            let at = |commit: &str| revision(&fixture.store.state, &fixture.project, commit);
            let did = |target: &Target, shown: Shown| {
                // The graphs of the two commits are rewritten between the
                // cases, and may be of one size and one moment.
                fixture.graphs.maps.forget(&fixture.project);
                let view = fixture
                    .graphs
                    .map(&fixture.project, Some(target), &shown)
                    .unwrap();
                let change = view["pattern"]["change"].clone();
                (
                    change["state"].as_str().unwrap_or("").to_owned(),
                    change["reason"].as_str().unwrap_or("").to_owned(),
                    change["added"]["pairs"].as_u64().unwrap_or(0),
                    change["removed"]["pairs"].as_u64().unwrap_or(0),
                )
            };
            let told = |state: &str, reason: &str, added: u64, removed: u64| {
                (state.to_owned(), reason.to_owned(), added, removed)
            };
            // The order asks the service at both ends, under the name it
            // had at the base and under the one the change gave it.
            let old = SHOP.map(|(id, path)| match id {
                "order" => (id, "src/Entity/Purchase.php"),
                _ => (id, path),
            });
            let against = [&NEEDS[..], &[("order", "service")]].concat();
            draw(&at(&base), &old, &against);
            draw(&at(&head), &SHOP, &against);
            let moved = |status: char| {
                vec![graph_changes::Change {
                    path: "src/Entity/Order.php".to_owned(),
                    status,
                    from: Some("src/Entity/Purchase.php".to_owned()),
                }]
            };
            // Known to be one file, it is one pair, and the change did
            // nothing to the order. A copy is a file of its own, and so is
            // one of which nothing says where it came from.
            let renamed = pull(&fixture, Some(&base), &head, moved('R'));
            assert_eq!(did(&renamed, layers()), told("ready", "", 0, 0));
            let copied = pull(&fixture, Some(&base), &head, moved('C'));
            assert_eq!(did(&copied, layers()), told("ready", "", 1, 1));
            let unknown = pull(&fixture, Some(&base), &head, Vec::new());
            assert_eq!(did(&unknown, layers()), told("ready", "", 1, 1));
            // What the tool only inferred is left out of both ends where
            // the picture leaves it out.
            let sure = Shown {
                certain: true,
                ..layers()
            };
            draw(&at(&base), &SHOP, &NEEDS);
            draw(
                &at(&head),
                &SHOP,
                &[&NEEDS[..], &[("order", "service?")]].concat(),
            );
            assert_eq!(did(&unknown, layers()), told("ready", "", 1, 0));
            assert_eq!(did(&unknown, sure), told("ready", "", 0, 0));
            // Tests inside a Rust file are told by the spans kept beside
            // the graph of a commit. A graph without them cannot be read
            // without its tests, at either end, and with them it can.
            let plain = Shown {
                notests: true,
                ..layers()
            };
            let rust = |spans: bool, commit: &str| {
                let mut files = SHOP.to_vec();
                files.push(("check:30", "src/Entity/order.rs"));
                files.push(("total:4", "src/Entity/order.rs"));
                // A test inside the model asks the service.
                draw(
                    &at(commit),
                    &files,
                    &[&NEEDS[..], &[("check", "service")]].concat(),
                );
                let beside = at(commit).with_file_name(crate::graph_map::TEST_SPANS);
                if spans {
                    std::fs::write(beside, r#"{"src/Entity/order.rs":[[25,40]]}"#).unwrap();
                } else {
                    let _ = std::fs::remove_file(beside);
                }
            };
            let target = pull(&fixture, Some(&base), &head, Vec::new());
            draw(&at(&head), &SHOP, &against);
            rust(false, &base);
            assert_eq!(did(&target, plain), told("unavailable", "tests", 0, 0));
            assert_eq!(did(&target, layers()), told("ready", "", 1, 1));
            rust(false, &head);
            rust(true, &base);
            assert_eq!(did(&target, plain), told("unavailable", "tests", 0, 0));
            assert_eq!(did(&target, layers()), told("ready", "", 0, 0));
            // With the spans at both ends the test is left out of both:
            // without it nothing runs against the order, and with it one
            // pair does that the change neither added nor took away.
            rust(true, &head);
            assert_eq!(did(&target, plain), told("ready", "", 0, 0));
            assert_eq!(did(&target, layers()), told("ready", "", 0, 0));
            let view = fixture
                .graphs
                .map(&fixture.project, Some(&target), &plain)
                .unwrap();
            assert_eq!(view["pattern"]["against"]["pairs"], 0);
            // A base whose graph could not be built is not built again: the
            // picture says that it cannot compare.
            std::fs::remove_file(at(&base)).unwrap();
            lock(&fixture.graphs.refused)
                .unwrap()
                .insert((fixture.project.clone(), base.clone()), "boom".to_owned());
            assert_eq!(did(&target, layers()), told("unavailable", "", 0, 0));
            assert!(lock(&fixture.graphs.revisions).unwrap().is_empty());
            let view = fixture
                .graphs
                .map(&fixture.project, Some(&target), &layers())
                .unwrap();
            assert_eq!(view["pattern"]["change"]["base"]["commit"], &base[..12]);
            // The project's own graph, built from a clean checkout of the
            // base, stands for the graph of that commit. It is read with
            // the files of the checkout and needs no spans beside it.
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let mut files = SHOP.to_vec();
            files.push(("total:4", "src/Entity/order.rs"));
            draw(
                &live(&fixture.store.state, &fixture.project),
                &files,
                &[&NEEDS[..], &[("line", "mailer")]].concat(),
            );
            assert_eq!(did(&target, plain), told("ready", "", 0, 1));
            // Until the graph of the change itself is there the picture is
            // of the project's own graph, and waits for it before telling;
            // a change whose graph could not be built is never told.
            std::fs::remove_file(at(&head)).unwrap();
            draw(
                &live(&fixture.store.state, &fixture.project),
                &SHOP,
                &against,
            );
            let first = fixture
                .graphs
                .map(&fixture.project, Some(&target), &layers())
                .unwrap();
            assert_eq!(first["preliminary"], true);
            assert_eq!(first["pattern"]["against"]["pairs"], 1);
            assert_eq!(first["pattern"]["change"], json!({"state": "waiting_head"}));
            lock(&fixture.graphs.refused)
                .unwrap()
                .insert((fixture.project.clone(), head.clone()), "boom".to_owned());
            assert_eq!(did(&target, layers()), told("unavailable", "head", 0, 0));
        }
        #[test]
        fn the_layers_of_the_checkouts_own_work_are_compared_with_where_it_started() {
            let fixture = Fixture::new();
            let (main, _) = fixture.branched();
            // The tool gives each commit the graph its `graph.fixture` holds:
            // a shop in layers is committed, one of its files in Rust, and
            // then, not committed, its order asks the service.
            let mut files = SHOP.to_vec();
            files.push(("total:4", "src/Entity/order.rs"));
            fixture.write("graph.fixture", &sketch(&files, &NEEDS));
            let head = fixture.commit("a shop in layers");
            let back = [&NEEDS[..], &[("order", "service")]].concat();
            fixture.write("graph.fixture", &sketch(&files, &back));
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            // The picture, once the graph of the base is built.
            let asked = |target: &Target, shown: Shown| {
                let started = Instant::now();
                loop {
                    let view = fixture
                        .graphs
                        .map(&fixture.project, Some(target), &shown)
                        .unwrap();
                    if view["pattern"]["change"]["state"] != "building" {
                        return view;
                    }
                    assert!(started.elapsed() < Duration::from_secs(20), "{view}");
                    std::thread::sleep(Duration::from_millis(20));
                }
            };
            let told = |target: &Target| asked(target, layers());
            let did = |view: &Value| {
                let change = &view["pattern"]["change"];
                (
                    change["state"].as_str().unwrap_or("").to_owned(),
                    change["base"]["commit"].as_str().unwrap_or("").to_owned(),
                    change["added"]["pairs"].as_u64().unwrap_or(0),
                    change["removed"]["pairs"].as_u64().unwrap_or(0),
                )
            };
            // What is not committed is measured from the last commit, whose
            // graph is built for it: the first answer says that it is being
            // prepared.
            let first = fixture
                .graphs
                .map(&fixture.project, Some(&Target::Uncommitted), &layers())
                .unwrap();
            assert_eq!(first["mode"], "pattern");
            assert_eq!(
                first["pattern"]["change"],
                json!({
                    "state": "building",
                    "base": {"name": "the last commit", "commit": &head[..12]},
                })
            );
            assert_eq!(
                did(&told(&Target::Uncommitted)),
                ("ready".to_owned(), head[..12].to_owned(), 1, 0)
            );
            // The checkout's own graph is read with the files of the
            // checkout, so without its tests it needs no spans beside it.
            let plain = asked(
                &Target::Uncommitted,
                Shown {
                    notests: true,
                    ..layers()
                },
            );
            assert_eq!(
                did(&plain),
                ("ready".to_owned(), head[..12].to_owned(), 1, 0)
            );
            // The whole of the work is measured from the branch it left,
            // where there was no shop yet.
            assert_eq!(
                did(&told(&Target::Work)),
                ("ready".to_owned(), main[..12].to_owned(), 1, 0)
            );
            // A graph older than the files does not hold what they do to
            // the order: until it is rebuilt nothing is told of the change.
            // The rebuild waits for the build of a commit to end.
            rested(&fixture);
            let worse = [&back[..], &[("line", "mailer")]].concat();
            fixture.write("graph.fixture", &sketch(&files, &worse));
            lock(&fixture.graphs.prints).unwrap().clear();
            let stale = fixture
                .graphs
                .map(&fixture.project, Some(&Target::Uncommitted), &layers())
                .unwrap();
            assert_eq!(stale["stale"], true);
            assert_eq!(
                stale["pattern"]["change"],
                json!({
                    "state": "unavailable", "reason": "stale",
                    "base": {"name": "the last commit", "commit": &head[..12]},
                })
            );
            assert_eq!(fixture.settled()["state"], "ready");
            let rebuilt = told(&Target::Uncommitted);
            assert_eq!(rebuilt["stale"], false);
            assert_eq!(
                did(&rebuilt),
                ("ready".to_owned(), head[..12].to_owned(), 2, 0)
            );
        }
        #[test]
        fn map_git_history_finds_indirect_peers_and_ranks_hotspots() {
            let fixture = Fixture::new();
            fixture.git(&["init", "-q", "--initial-branch=main"]);
            let files = [
                ("a", "A.php"),
                ("b", "B.php"),
                ("c", "C.php"),
                ("d", "D.php"),
            ];
            let links = [("a", "b"), ("b", "a"), ("c", "a"), ("b", "d")];
            fixture.write("graph.fixture", &sketch(&files, &links));
            for (_, path) in files {
                fixture.write(path, "first");
            }
            fixture.commit("first");
            for (_, path) in files {
                fixture.write(path, "second");
            }
            fixture.commit("second");
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let map = |shown| fixture.graphs.map(&fixture.project, None, &shown).unwrap();
            assert!(map(Shown::default())["git_history"].is_null());
            let shown = Shown {
                focus: Some("A.php"),
                history: true,
                ..Shown::default()
            };
            let view = map(shown);
            let history = &view["git_history"];
            assert_eq!(history["commits"], 2, "{view}");
            assert_eq!(
                history["cochanges"]["rows"],
                json!([{"path":"D.php","together":2,"commits":2}])
            );
            assert_eq!(
                history["hot"],
                json!([
                    {"path":"A.php","commits":2,"neighbors":2},
                    {"path":"B.php","commits":2,"neighbors":2}
                ])
            );
            fixture.write("A.php", "third");
            fixture.commit("third");
            assert_eq!(map(shown)["git_history"]["cochanges"]["commits"], 3);
        }

        #[test]
        fn layers_use_the_project_rules_and_report_invalid_updates() {
            let fixture = Fixture::new();
            fixture.git(&["init", "-q", "--initial-branch=main"]);
            let files = [("a", "X.php"), ("b", "Y.php"), ("c", "Z.php")];
            fixture.write(
                "graph.fixture",
                &sketch(&files, &[("a", "b"), ("b", "a"), ("b", "c")]),
            );
            for (_, file) in files {
                fixture.write(file, "<?php");
            }
            let rules = json!({"version":1,"layers":[
                {"id":"first","name":"First","paths":["X.php"],"allows":[]},
                {"id":"second","name":"Second","paths":["Y.php"],"allows":["first"]}
            ]});
            let file = crate::graph_map::layer_rules::FILE;
            fixture.write(file, &rules.to_string());
            let revision = fixture.commit("policy");
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let map = |shown| fixture.graphs.map(&fixture.project, None, &shown).unwrap();
            let ordinary = map(Shown {
                patterns: true,
                ..Shown::default()
            });
            assert_eq!(
                ordinary["patterns"]["kinds"][0]["instances"][0]["id"],
                "layers@"
            );
            let nodes = ordinary["nodes"].as_array().unwrap();
            let first = nodes.iter().find(|node| node["path"] == "X.php").unwrap();
            assert_eq!(first["architecture_layer"]["name"], "First");
            assert_eq!(ordinary["architecture"]["reciprocal_total"], 1);
            assert_eq!(ordinary["architecture"]["violations"]["pairs"], 1);
            let edges = ordinary["edges"].as_array().unwrap();
            assert_eq!(
                edges.iter().find(|edge| edge["from"] == "X.php").unwrap()["architecture_violation"],
                1
            );
            assert!(
                edges.iter().find(|edge| edge["to"] == "X.php").unwrap()["architecture_violation"]
                    .is_null()
            );
            let file_view = map(Shown {
                scope: "X.php",
                patterns: true,
                ..Shown::default()
            });
            assert!(
                file_view["nodes"].as_array().unwrap().iter().any(|node| {
                    node["symbol"] == true && node["architecture_layer"]["name"] == "First"
                }),
                "{file_view}"
            );
            let layers = |focus| Shown {
                pattern: Some("layers@"),
                focus,
                ..Shown::default()
            };
            let view = map(layers(None));
            assert_eq!(view["pattern"]["declared"], true);
            assert_eq!(view["pattern"]["apart"], 1);
            assert_eq!(view["pattern"]["against"]["pairs"], 1);
            assert_eq!(
                view["pattern"]["against"]["rows"][0]["from"]["path"],
                "X.php"
            );
            assert_eq!(
                map(layers(Some("layer:second>layer:first")))["focus"]["against"],
                false
            );
            fixture.write(file, "{invalid");
            let invalid = map(layers(None));
            assert!(invalid["architecture_error"].is_string());
            assert_ne!(invalid["mode"], "pattern");
            assert!(
                graph_changes::architecture_at(&fixture.folder, &revision, "")
                    .unwrap()
                    .is_some()
            );
        }

        /// A session of the project with the steps a run recorded, as the
        /// store keeps them: each step started and completed under one call.
        fn recorded(fixture: &Fixture, folder: &str, steps: &[Value]) -> String {
            let session = store::id();
            fixture
                .store
                .connection()
                .unwrap()
                .execute(
                    "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at)VALUES(?1,'Round the totals once',?2,?3,'claude','edit','native',0,'completed','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
                    rusqlite::params![session, fixture.project, folder],
                )
                .unwrap();
            for (call, step) in steps.iter().enumerate() {
                if step["kind"] != "tool" {
                    fixture.store.event(&session, step).unwrap();
                    continue;
                }
                let mut started = step.clone();
                started["state"] = json!("started");
                started["call"] = json!(format!("call-{call}"));
                let failed = started.as_object_mut().unwrap().remove("failed").is_some();
                // What a step changed is known when it ends.
                let changes = started.as_object_mut().unwrap().remove("changes");
                fixture.store.event(&session, &started).unwrap();
                let mut ended = json!({
                    "kind": "tool", "tool": step["tool"], "state": "completed",
                    "call": started["call"], "ok": !failed,
                });
                if let Some(changes) = changes {
                    ended["changes"] = changes;
                }
                fixture.store.event(&session, &ended).unwrap();
            }
            session
        }
        fn step(tool: &str, path: &str) -> Value {
            json!({"kind": "tool", "tool": tool, "path": path, "ok": true})
        }
        #[test]
        fn a_run_is_mapped_as_what_it_changed_with_what_it_opened() {
            let fixture = Fixture::new();
            fixture.branched();
            // A shop in which the service, the mailer and a line use the
            // order.
            let uses = [
                &NEEDS[..],
                &[
                    ("mailer", "order"),
                    ("line", "order"),
                    ("order", "line"),
                    ("test", "controller"),
                ],
            ]
            .concat();
            let mut files = SHOP.to_vec();
            files.extend([
                ("coupon", "src/Entity/Coupon.php"),
                ("test", "tests/OrderTest.php"),
            ]);
            fixture.write("graph.fixture", &sketch(&files, &uses));
            fixture.commit("a shop");
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let folder = fixture.folder.to_string_lossy().into_owned();
            // The run read the order and the service, and a file outside the
            // project; it edited the order, made a coupon, deleted a file,
            // and failed to edit the mailer. It searched once and ran one
            // command.
            let mut failed = step("Edit", "src/Service/MailService.php");
            failed["failed"] = json!(true);
            let mut outside = step("Read", "/etc/hosts");
            outside["outside"] = json!(true);
            let mut patched = json!({"kind": "tool", "tool": "apply_patch", "ok": true});
            patched["changes"] = json!([{"path": "src/Old.php", "kind": "delete"}]);
            let mut searched = step("Grep", "src");
            searched["detail"] = json!("Order");
            let command = json!({"kind": "tool", "tool": "Bash", "detail": "php -l", "ok": true});
            let session = recorded(
                &fixture,
                &folder,
                &[
                    json!({"kind": "text", "text": "Reading the order."}),
                    step("Read", "src/Entity/Order.php"),
                    step("Read", "src\\Service\\OrderService.php"),
                    outside,
                    step("Edit", "./src/Entity/Order.php"),
                    step("Write", "src/Entity/Coupon.php"),
                    patched,
                    failed,
                    searched,
                    command,
                ],
            );
            let target = Target::Run(session.clone());
            let map = |shown| fixture.graphs.map(&fixture.project, Some(&target), &shown);
            let view = map(Shown::default()).unwrap();
            // The change is what the run changed, each file with what was
            // done to it; the mailer it failed to edit is none of it.
            assert_eq!(view["mode"], "changes");
            assert_eq!(view["target"], format!("run:{session}"));
            assert_eq!(
                (
                    &view["chosen"]["kind"],
                    &view["chosen"]["name"],
                    &view["chosen"]["files"]
                ),
                (&json!("run"), &json!("Round the totals once"), &json!(3))
            );
            assert_eq!(
                (
                    &view["change"]["files"],
                    &view["change"]["removed"],
                    &view["change"]["unknown"]
                ),
                (&json!(3), &json!(1), &json!(0))
            );
            let test_reach = view["test_reach"].as_array().unwrap();
            let order_tests = test_reach
                .iter()
                .find(|row| row["path"] == "src/Entity/Order.php")
                .unwrap();
            assert_eq!(order_tests["tests"][0]["path"], "tests/OrderTest.php");
            assert_eq!(order_tests["tests"][0]["steps"], 3);
            assert!(
                map(Shown {
                    notests: true,
                    ..Shown::default()
                })
                .unwrap()["test_reach"]
                    .is_null()
            );
            let status = |path: &str| {
                view["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|node| node["path"] == path)
                    .map(|node| (node["status"].clone(), node["opened"].clone()))
            };
            assert_eq!(
                status("src/Entity/Order.php"),
                Some((json!("M"), json!(true)))
            );
            // A file the run wrote without having opened it is one it made.
            assert_eq!(
                status("src/Entity/Coupon.php"),
                Some((json!("A"), Value::Null))
            );
            // What the run opened is laid over the picture: the service it
            // read, and not the mailer or the line.
            assert_eq!(
                status("src/Service/OrderService.php"),
                Some((Value::Null, json!(true)))
            );
            assert_eq!(
                status("src/Service/MailService.php"),
                Some((Value::Null, Value::Null))
            );
            // Beside the picture: what the run did, and of the three files
            // that use the order the two it never opened.
            let run = &view["run"];
            assert_eq!(
                json!({
                    "session": run["session"], "title": run["title"], "provider": run["provider"],
                    "status": run["status"], "opened": run["opened"], "edited": run["edited"],
                    "outside": run["outside"], "searched": run["searched"],
                    "commands": run["commands"], "helpers": run["helpers"],
                    "limited": run["limited"], "elsewhere": run["elsewhere"],
                }),
                json!({
                    "session": session, "title": "Round the totals once", "provider": "claude",
                    "status": "completed", "opened": 2, "edited": 3, "outside": 1, "searched": 1,
                    "commands": 1, "helpers": 0, "limited": false, "elsewhere": false,
                })
            );
            let reach = &run["reach"];
            assert_eq!(
                (&reach["users"], &reach["opened"], &reach["unopened"]),
                (&json!(3), &json!(1), &json!(2))
            );
            assert_eq!(
                reach["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| row["path"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["src/Entity/Line.php", "src/Service/MailService.php"]
            );
            // The graph is the project's own, as for the checkout's work.
            assert_eq!(
                (&view["state"], &view["stale"], &view["rebuilding"]),
                (&json!("ready"), &json!(false), &json!(false))
            );
            // A folder of the change and a selected file are overlaid alike.
            let order = map(Shown {
                focus: Some("src/Entity/Order.php"),
                ..Shown::default()
            })
            .unwrap();
            let used_by = order["focus"]["used_by"].as_array().unwrap();
            assert_eq!(used_by.len(), 3);
            assert_eq!(
                used_by
                    .iter()
                    .filter(|row| row["opened"] == true)
                    .map(|row| row["path"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["src/Service/OrderService.php"]
            );
            // The run is searched in the project's graph, chosen or not,
            // and is chosen as it is mapped.
            let found = fixture
                .graphs
                .find(&fixture.project, Some(&target), "Order", false)
                .unwrap();
            assert!(found["found"].as_u64().unwrap() > 0, "{found}");
            let chosen = fixture.graphs.choose(&fixture.project, &target).unwrap();
            assert_eq!(chosen["chosen"]["kind"], "run");
            // A session that is not there, and one of another project, are
            // no runs of this project.
            let nowhere = Target::Run(store::id());
            assert!(
                fixture
                    .graphs
                    .map(&fixture.project, Some(&nowhere), &Shown::default())
                    .is_err()
            );
            let other = tempfile::tempdir().unwrap();
            let elsewhere = fixture.store.register(other.path()).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned();
            assert_eq!(
                fixture
                    .graphs
                    .map(&elsewhere, Some(&target), &Shown::default())
                    .unwrap_err()
                    .message,
                "This project has no such run."
            );
        }
        #[test]
        fn comparing_launches_keeps_the_original_changes_and_attributes_followup_actions() {
            let fixture = Fixture::new();
            fixture.branched();
            let links = [
                ("service", "order"),
                ("controller", "order"),
                ("mailer", "order"),
                ("line", "order"),
            ];
            fixture.write("graph.fixture", &sketch(&SHOP, &links));
            fixture.commit("a shop");
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let order = "src/Entity/Order.php";
            let service = "src/Service/OrderService.php";
            let controller = SHOP.iter().find(|(id, _)| *id == "controller").unwrap().1;
            let mailer = "src/Service/MailService.php";
            let line = SHOP.iter().find(|(id, _)| *id == "line").unwrap().1;
            let session = recorded(
                &fixture,
                fixture.folder.to_str().unwrap(),
                &[step("Read", line)],
            );
            let first = store::hexid();
            let second = store::hexid();
            for (id, tool, path, call) in [
                (&first, "Edit", order, "1"),
                (&first, "Read", controller, "2"),
                (&second, "Edit", service, "1"),
                (&second, "Read", mailer, "2"),
            ] {
                for (state, fields) in [
                    ("started", json!({"path": path})),
                    ("completed", json!({"ok": true})),
                ] {
                    let mut event = fields;
                    event["kind"] = json!("tool");
                    event["tool"] = json!(tool);
                    event["state"] = json!(state);
                    event["call"] = json!(call);
                    event["launch_id"] = json!(id);
                    fixture.store.event(&session, &event).unwrap();
                }
            }
            std::fs::create_dir_all(fixture.folder.join(line).parent().unwrap()).unwrap();
            fixture.write(line, "<?php");
            let shell = fixture.store.event(&session, &json!({
                "kind":"tool", "tool":"Shell", "state":"completed", "ok":true, "exit_code":0,
                "launch_id":second, "detail":format!("sed -n '1p' {line}"),
            })).unwrap();
            let target = Target::Run(session);
            let map = |launch, compare_launch| {
                fixture.graphs.map(
                    &fixture.project,
                    Some(&target),
                    &Shown {
                        launch,
                        compare_launch,
                        ..Shown::default()
                    },
                )
            };
            let view = map(Some(&first), Some(&second)).unwrap();
            assert_eq!(view["chosen"]["files"], 1);
            assert_eq!(view["run"]["opened"], 1);
            assert_eq!(view["run"]["launches"][0]["id"], first);
            assert_eq!(view["run"]["launches"][1]["id"], second);
            let comparison = &view["run"]["comparison"];
            assert_eq!(comparison["newly_touched"], 3);
            assert_eq!(comparison["gaps"], 0);
            let row = |path| {
                comparison["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["path"] == path)
                    .unwrap()
            };
            assert_eq!(row(service)["followup_edited"], true);
            assert_eq!(row(service)["opened"], false);
            assert_eq!(row(mailer)["followup_opened"], true);
            assert_eq!(row(controller)["opened"], true);
            assert_eq!(row(line)["opened"], false);
            assert_eq!(row(line)["followup_opened"], true);
            assert_eq!(row(line)["followup_shell_read"], shell);
            assert!(row(line)["shell_read"].is_null());
            assert!(map(None, None).unwrap()["run"]["comparison"].is_null());
            assert!(map(Some(&first), Some(&first)).is_err());
            assert!(map(Some(&first), Some("foreign")).is_err());
        }

        #[test]
        fn a_long_run_and_one_in_a_folder_of_its_own_are_read_as_far_as_they_were_recorded() {
            let fixture = Fixture::new();
            fixture.branched();
            fixture.write("graph.fixture", &sketch(&SHOP, &NEEDS));
            fixture.commit("a shop");
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            // More steps than the store hands out at once, and then the
            // notice that later steps are no longer named: a step after it
            // carries no file.
            let mut steps = (0..300)
                .map(|at| step("Read", &format!("docs/page{at:03}.md")))
                .collect::<Vec<_>>();
            steps.push(step("Edit", "src/Service/OrderService.php"));
            steps.push(json!({
                "kind": "status", "targets": "limited",
                "text": "Step details are not recorded after this point; steps are still counted.",
            }));
            steps.push(json!({"kind": "tool", "text": "Edit: started"}));
            // The run worked in a worktree: its files are named as the
            // project's are.
            let worktree = tempfile::tempdir().unwrap();
            let session = recorded(&fixture, &worktree.path().to_string_lossy(), &steps);
            let view = fixture
                .graphs
                .map(
                    &fixture.project,
                    Some(&Target::Run(session)),
                    &Shown::default(),
                )
                .unwrap();
            let run = &view["run"];
            assert_eq!(
                (
                    &run["opened"],
                    &run["edited"],
                    &run["limited"],
                    &run["elsewhere"]
                ),
                (&json!(300), &json!(1), &json!(true), &json!(true))
            );
            assert_eq!(view["chosen"]["files"], 1);
            // The controller uses the service the run edited, and the run
            // never opened it.
            assert_eq!(
                (&run["reach"]["users"], &run["reach"]["unopened"]),
                (&json!(1), &json!(1))
            );
            assert_eq!(
                run["reach"]["rows"][0]["path"],
                "src/Controller/OrderController.php"
            );
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
                    .map(&fixture.project, Some(&target), &Shown::default())
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
            let mut kept = std::fs::read_dir(
                folder(&fixture.store.state, &fixture.project)
                    .join("revisions")
                    .join(&feature),
            )
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
            kept.sort();
            assert_eq!(kept, ["graph.json", "test-spans.json"]);
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
        /// The files under a folder, by their paths from it.
        fn held(home: &Path) -> Vec<String> {
            let mut found = Vec::new();
            let mut folders = vec![home.to_path_buf()];
            while let Some(folder) = folders.pop() {
                for entry in std::fs::read_dir(&folder).unwrap().flatten() {
                    if entry.file_type().unwrap().is_dir() {
                        folders.push(entry.path());
                    } else {
                        found.push(
                            entry
                                .path()
                                .strip_prefix(home)
                                .unwrap()
                                .to_string_lossy()
                                .replace('\\', "/"),
                        );
                    }
                }
            }
            found.sort();
            found
        }
        #[test]
        fn the_graph_of_a_commit_is_read_with_what_is_kept_of_its_files() {
            let fixture = Fixture::new();
            let write = |name: &str, text: &str| {
                let path = fixture.folder.join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            };
            fixture.git(&["init", "-q", "--initial-branch=main"]);
            write("Cargo.toml", "[workspace]\nmembers = [\"app\", \"core\"]\n");
            write(
                "app/Cargo.toml",
                "[package]\nname = \"shop-app\"\n[dependencies]\nshop-core = { path = \"../core\" }\n",
            );
            write("core/Cargo.toml", "[package]\nname = \"shop-core\"\n");
            write("app/src/main.rs", "fn main() {}\n");
            write("core/src/lib.rs", "pub fn run() {}\n");
            // The tool has the core use the app, which is built the other
            // way round.
            write(
                "graph.fixture",
                r#"{"nodes":[{"id":"main","source_file":"app/src/main.rs"},{"id":"run","source_file":"core/src/lib.rs"}],"links":[{"source":"main","target":"run","relation":"calls","source_file":"app/src/main.rs"},{"source":"run","target":"main","relation":"imports_from","source_file":"core/src/lib.rs"}]}"#,
            );
            fixture.commit("one");
            fixture.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
            fixture.git(&["switch", "-q", "-c", "feature"]);
            // On the branch neither is built with the other, and the core
            // takes one source of the app by its path.
            write("app/Cargo.toml", "[package]\nname = \"shop-app\"\n");
            write("app/src/shared/util.rs", "pub fn helper() {}\n");
            write(
                "core/src/lib.rs",
                "#[path = \"../../app/src/shared/util.rs\"]\nmod util;\npub fn run() {}\n\
                 #[test]\nfn runs_with_fixture() {}\n",
            );
            write(
                "graph.fixture",
                r#"{"nodes":[{"id":"main","source_file":"app/src/main.rs"},{"id":"run","source_file":"core/src/lib.rs"},{"id":"helper","source_file":"app/src/shared/util.rs"},{"id":"runs_with_fixture","source_file":"core/src/lib.rs","source_location":"L5"}],"links":[{"source":"main","target":"run","relation":"calls","source_file":"app/src/main.rs"},{"source":"run","target":"main","relation":"imports_from","source_file":"core/src/lib.rs"},{"source":"runs_with_fixture","target":"helper","relation":"calls","source_file":"core/src/lib.rs"}]}"#,
            );
            let feature = fixture.commit("two");
            fixture.git(&["switch", "-q", "main"]);
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let arrows = |view: &Value| {
                view["edges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|edge| {
                        format!(
                            "{} -> {}",
                            edge["from"].as_str().unwrap(),
                            edge["to"].as_str().unwrap()
                        )
                    })
                    .collect::<Vec<_>>()
            };
            // The project's own graph is read with the files of the
            // checkout: the app is built with the core.
            let own = fixture
                .graphs
                .map(&fixture.project, None, &Shown::default())
                .unwrap();
            assert_eq!(
                (&own["totals"]["links"], &own["totals"]["ruled_out"]),
                (&json!(1), &json!(1))
            );
            assert_eq!(arrows(&own), ["app -> core"]);
            // A folder a request names is spread out in the picture.
            let spread = ["app".to_owned()];
            let opened = fixture
                .graphs
                .map(
                    &fixture.project,
                    None,
                    &Shown {
                        spread: &spread,
                        ..Shown::default()
                    },
                )
                .unwrap();
            assert_eq!(arrows(&opened), ["app/src -> core"]);
            assert_eq!(opened["frames"][0]["path"], "app");
            assert_eq!(opened["nodes"][0]["holder"], "app");
            // A link a request names is looked into instead of a picture.
            let looked = fixture
                .graphs
                .map(
                    &fixture.project,
                    None,
                    &Shown {
                        link: Some(("app", "core")),
                        ..Shown::default()
                    },
                )
                .unwrap();
            assert_eq!(
                (&looked["mode"], &looked["links"], &looked["project_id"]),
                (&json!("link"), &json!(1), &json!(fixture.project))
            );
            assert_eq!(looked["used"]["groups"][0]["path"], "core/src/lib.rs");
            // The graph of the commit is read with the manifests of the
            // commit, which rule out both links between the two, and with
            // what its sources say: the helper is compiled into the core.
            let target = Target::Named("feature".into());
            fixture.graphs.choose(&fixture.project, &target).unwrap();
            let view = fixture.mapped(&target);
            assert_eq!(view["state"], "ready", "{view}");
            assert_eq!(
                (&view["totals"]["links"], &view["totals"]["ruled_out"]),
                (&json!(1), &json!(2))
            );
            assert_eq!(arrows(&view), ["core/src/lib.rs -> app/src/shared/util.rs"]);
            let plain = fixture
                .graphs
                .map(
                    &fixture.project,
                    Some(&target),
                    &Shown {
                        notests: true,
                        ..Shown::default()
                    },
                )
                .unwrap();
            assert_eq!(plain["tests"]["inside"], 1);
            assert!(arrows(&plain).is_empty());
            // Of the files of the commit that much is kept, and no source.
            let home = folder(&fixture.store.state, &fixture.project)
                .join("revisions")
                .join(&feature);
            assert_eq!(
                held(&home),
                [
                    "graph.json",
                    "test-spans.json",
                    "tree/Cargo.toml",
                    "tree/app/Cargo.toml",
                    "tree/core/Cargo.toml",
                    "tree/core/src/lib.rs.modules"
                ]
            );
            assert_eq!(
                std::fs::read_to_string(home.join("tree/core/src/lib.rs.modules")).unwrap(),
                "#[path = \"../../app/src/shared/util.rs\"\nmod util;"
            );
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
                    .map(&fixture.project, Some(target), &Shown::default())
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
                    &Shown {
                        scope: "cart.php",
                        ..Shown::default()
                    },
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
            // The links of a file are followed on the same graph with the
            // change still marked. A place the graph does not know falls
            // back to the change.
            let follow = |root: &str| {
                let followed = Followed {
                    root: root.to_owned(),
                    out: vec![root.to_owned()],
                    into: vec![root.to_owned()],
                };
                fixture
                    .graphs
                    .map(
                        &fixture.project,
                        Some(&Target::Work),
                        &Shown {
                            followed: Some(&followed),
                            ..Shown::default()
                        },
                    )
                    .unwrap()
            };
            let followed = follow("cart.php");
            assert_eq!(
                (
                    followed["mode"].as_str(),
                    followed["chosen"]["kind"].as_str(),
                    followed["state"].as_str()
                ),
                (Some("follow"), Some("work"), Some("ready"))
            );
            let places = followed["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| {
                    format!(
                        "{} {}",
                        node["path"].as_str().unwrap(),
                        node["status"].as_str().unwrap_or("-")
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(places, ["cart.php M", "price.php M", "order.php A"]);
            assert_eq!(follow("nowhere.php")["mode"], "changes");
            // The links of a symbol are followed among symbols, and what
            // the change did to each is read with them. This fixture gives
            // its symbols neither a name nor a line: one is told by its
            // file alone, that of the new file is new, and a changed line
            // marks none.
            let traced = follow("order.php#");
            assert_eq!(
                (
                    traced["mode"].as_str(),
                    traced["followed"]["symbol"].as_bool(),
                    traced["scope"].as_str(),
                    traced["chosen"]["kind"].as_str()
                ),
                (Some("follow"), Some(true), Some("order.php"), Some("work"))
            );
            let symbols = traced["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| {
                    format!(
                        "{} {} {}",
                        node["path"].as_str().unwrap(),
                        node["changed"],
                        node["status"].as_str().unwrap_or("-")
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(symbols, ["order.php# 1 A", "cart.php# 0 -"]);
            // The shortest ways between two places are found on the same
            // graph, with the change marked: the order reaches the price
            // through the cart. An end the graph does not know leaves the
            // change as it is drawn.
            let ways = |from: &str, to: &str| {
                fixture
                    .graphs
                    .map(
                        &fixture.project,
                        Some(&Target::Work),
                        &Shown {
                            path: Some((from, to)),
                            ..Shown::default()
                        },
                    )
                    .unwrap()
            };
            let way = ways("order.php", "price.php");
            assert_eq!(
                (
                    way["mode"].as_str(),
                    way["path"]["steps"].as_u64(),
                    way["path"]["reversed"].as_bool(),
                    way["chosen"]["kind"].as_str()
                ),
                (Some("path"), Some(2), Some(false), Some("work"))
            );
            let places = way["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| {
                    format!(
                        "{} {}",
                        node["path"].as_str().unwrap(),
                        node["status"].as_str().unwrap_or("-")
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(places, ["order.php A", "cart.php M", "price.php M"]);
            assert_eq!(ways("order.php", "nowhere.php")["mode"], "changes");
            // What two places share is asked of the same graph. The order
            // reaches the price through the cart alone, so the order and
            // the cart share nothing, and the answer says so; a place the
            // graph does not know leaves the change as it is drawn.
            let sharing = |from: &str, to: &str, users: Option<bool>| {
                fixture
                    .graphs
                    .map(
                        &fixture.project,
                        Some(&Target::Work),
                        &Shown {
                            shared: Some(Sharing {
                                from,
                                to,
                                users,
                                skip: 0,
                            }),
                            ..Shown::default()
                        },
                    )
                    .unwrap()
            };
            let both = sharing("order.php", "cart.php", None);
            assert_eq!(
                (
                    both["mode"].as_str(),
                    both["shared"]["from"]["name"].as_str(),
                    both["shared"]["side"].as_str(),
                    both["shared"]["both_use"].as_u64(),
                    both["shared"]["uses_both"].as_u64(),
                    both["chosen"]["kind"].as_str()
                ),
                (
                    Some("shared"),
                    Some("order.php"),
                    Some("both_use"),
                    Some(0),
                    Some(0),
                    Some("work")
                )
            );
            // The kind that is asked for is the one answered.
            assert_eq!(
                sharing("order.php", "cart.php", Some(true))["shared"]["side"],
                "uses_both"
            );
            assert_eq!(sharing("order.php", "nowhere.php", None)["mode"], "changes");
            // On the whole project the same links carry no mark.
            let whole = fixture
                .graphs
                .map(
                    &fixture.project,
                    None,
                    &Shown {
                        followed: Some(&Followed {
                            root: "order.php#".to_owned(),
                            out: vec!["order.php#".to_owned()],
                            into: vec![],
                        }),
                        ..Shown::default()
                    },
                )
                .unwrap();
            assert_eq!(
                (&whole["mode"], &whole["nodes"][0]["changed"]),
                (&json!("follow"), &json!(0))
            );
            assert_eq!(whole["nodes"].as_array().unwrap().len(), 2);
            // Every answer says whether the tests are left out, and where
            // they are, how many that is. This project has none.
            assert_eq!(
                (&work["notests"], &work["tests"]),
                (&json!(false), &Value::Null)
            );
            let plain = fixture
                .graphs
                .map(
                    &fixture.project,
                    Some(&Target::Work),
                    &Shown {
                        notests: true,
                        ..Shown::default()
                    },
                )
                .unwrap();
            assert_eq!(
                (&plain["notests"], &plain["tests"]),
                (&json!(true), &json!({"files": 0, "inside": 0}))
            );
            assert_eq!(drawn(&plain), drawn(&work));
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
                    &Shown {
                        scope: "order.php",
                        ..Shown::default()
                    },
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
                    &Shown {
                        scope: "cart.php",
                        ..Shown::default()
                    },
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
        fn a_search_reads_the_graph_the_map_is_drawn_from() {
            let fixture = Fixture::new();
            fixture.branched();
            fixture.graphs.act(&fixture.project, "enable").unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            let paths = |found: &Value| {
                found["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| row["path"].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            };
            let find = |target: Option<&Target>, query: &str| {
                fixture.graphs.find(&fixture.project, target, query, false)
            };
            // The whole project and the checkout's own work are searched in
            // the project's graph, where there is a cart and no order.
            let own = find(None, "CART").unwrap();
            assert_eq!(paths(&own), ["cart.php"]);
            assert_eq!(
                (&own["mode"], &own["project_id"], &own["notests"]),
                (&json!("find"), &json!(fixture.project), &json!(false))
            );
            for target in [None, Some(Target::Work), Some(Target::Uncommitted)] {
                let found = find(target.as_ref(), "order").unwrap();
                assert_eq!(found["found"], 0, "{target:?}");
            }
            // A change that was not chosen has no map, and nothing to search.
            let target = Target::Named("feature".into());
            assert!(find(Some(&target), "order").is_err());
            // Chosen, it is searched in the graph of its own commit.
            fixture.graphs.choose(&fixture.project, &target).unwrap();
            assert_eq!(fixture.mapped(&target)["state"], "ready");
            assert_eq!(paths(&find(Some(&target), "order").unwrap()), ["order.php"]);
            // Whether tests were left out is told back with what was found.
            let plain = fixture
                .graphs
                .find(&fixture.project, None, "cart", true)
                .unwrap();
            assert_eq!(
                (&plain["notests"], &plain["found"]),
                (&json!(true), &json!(1))
            );
            // Where a commit has no graph and the project none either, there
            // is nothing to search, and that is said.
            let bare = Fixture::new();
            bare.branched();
            bare.git(&["switch", "-q", "-c", "broken"]);
            bare.write("fail", "");
            bare.commit("broken");
            bare.git(&["switch", "-q", "main"]);
            let target = Target::Named("broken".into());
            bare.graphs.choose(&bare.project, &target).unwrap();
            assert_eq!(bare.mapped(&target)["state"], "failed");
            let none = bare
                .graphs
                .find(&bare.project, Some(&target), " cart ", false)
                .unwrap();
            assert_eq!(
                (&none["missing"], &none["found"], &none["query"]),
                (&json!(true), &json!(0), &json!("cart"))
            );
            assert!(
                bare.graphs
                    .find(&bare.project, None, "cart", false)
                    .is_err()
            );
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
                .map(&fixture.project, Some(&target), &Shown::default())
                .unwrap();
            // The files are the commit's, the links the checkout's: the
            // cart uses the price in the project's own graph.
            assert_eq!(view["state"], "building");
            assert_eq!(view["preliminary"], true);
            assert_eq!(drawn(&view), ["price.php changed M", "cart.php affected"]);
            assert_eq!(view["change"]["files"], 2);
            // A search reads the project's graph until then, as the map does.
            let found = fixture
                .graphs
                .find(&fixture.project, Some(&target), "cart", false)
                .unwrap();
            assert_eq!(found["rows"][0]["path"], "cart.php");
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
                    .map(&fixture.project, Some(&target), &Shown::default())
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
        #[test]
        fn auto_start_enables_unset_project_and_respects_user_disable() {
            let fixture = Fixture::new();
            assert_eq!(
                fixture.graphs.status(&fixture.project, true).unwrap()["state"],
                "off"
            );
            assert_eq!(
                fixture.graphs.status(&fixture.project, false).unwrap()["message"],
                "Code graph is turned off."
            );

            fixture.graphs.auto_start(&fixture.project).unwrap();
            let ready = fixture.settled();
            assert_eq!(ready["state"], "ready", "{ready}");
            assert_eq!(ready["message"], "Code graph & dependency map ready.");
            assert!(live(&fixture.store.state, &fixture.project).is_file());
            assert!(fixture.graphs.live_index(&fixture.project).is_some());
            assert_eq!(fixture.runs(), 1);

            let disabled = fixture.graphs.act(&fixture.project, "disable").unwrap();
            assert_eq!(disabled["state"], "off");
            fixture.graphs.auto_start(&fixture.project).unwrap();
            assert_eq!(
                fixture.graphs.status(&fixture.project, false).unwrap()["state"],
                "off"
            );
            assert_eq!(fixture.runs(), 1);

            fixture.graphs.act(&fixture.project, "enable").unwrap();
            let ready = fixture.settled();
            assert_eq!(ready["state"], "ready");
            assert_eq!(fixture.runs(), 1);

            fixture.graphs.act(&fixture.project, "delete").unwrap();
            let deleted = fixture.settled();
            assert_eq!(deleted["state"], "off");
            fixture.graphs.auto_start(&fixture.project).unwrap();
            assert_eq!(
                fixture.graphs.status(&fixture.project, false).unwrap()["state"],
                "off"
            );
            assert_eq!(fixture.runs(), 1);

            // auto_start_projects asynchronously starts unset projects when enabled.
            let root2 = tempfile::tempdir().unwrap();
            let store2 = SessionStore::open(&root2.path().join("state")).unwrap();
            let folder2 = root2.path().join("project");
            std::fs::create_dir_all(&folder2).unwrap();
            let p2 = store2.register(&folder2).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned();
            let missing = Graphs::with(
                store2,
                Some(PathBuf::from("/nonexistent/graphify-missing")),
                Duration::ZERO,
            );
            missing.auto_start_projects();
            let deadline = Instant::now();
            let state2 = loop {
                let st = missing.status(&p2, false).unwrap();
                if st["state"] != "off" || deadline.elapsed() > Duration::from_secs(5) {
                    break st;
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(state2["state"], "missing", "{state2}");
            assert!(
                state2["message"]
                    .as_str()
                    .unwrap()
                    .contains("Graphify is not installed"),
                "{state2}"
            );
            missing.close();
        }
        #[test]
        fn map_exposes_graph_focus_and_english_message_without_overwriting_focus() {
            let fixture = Fixture::new();
            fixture.graphs.auto_start(&fixture.project).unwrap();
            assert_eq!(fixture.settled()["state"], "ready");

            let view = fixture
                .graphs
                .map(&fixture.project, None, &Shown::default())
                .unwrap();
            assert_eq!(view["message"], "Code graph & dependency map ready.");
            assert_eq!(view["graph_focus"], Value::Null);
        }
        #[tokio::test]
        async fn domain_execute_auto_start_and_map_focus_session() {
            let fixture = Fixture::new();
            std::fs::write(
                fixture.folder.join("graph.fixture"),
                json!({
                    "nodes": [
                        {"id": "n1", "label": "App", "source_file": "src/app.php"},
                        {"id": "n2", "label": "Auth", "source_file": "src/auth/guard.php"}
                    ],
                    "links": [
                        {"source": "n1", "target": "n2", "relation": "calls", "confidence": "EXTRACTED"}
                    ]
                })
                .to_string(),
            )
            .unwrap();

            let Payload::Json(auto) = fixture
                .graphs
                .execute(
                    "graph.auto_start",
                    json!({
                        "params": {"project_id": fixture.project},
                        "body": {},
                        "query": {},
                    }),
                )
                .await
                .unwrap()
            else {
                panic!("expected json auto_start payload");
            };
            assert_eq!(auto, json!({"ok": true}));
            assert_eq!(fixture.settled()["state"], "ready");

            let created = fixture
                .store
                .create(
                    &json!({
                        "project_id": fixture.project,
                        "project_path": fixture.folder,
                        "provider": "codex",
                    }),
                    "Focus overlay test",
                    &[],
                )
                .unwrap();
            let sid = created["id"].as_str().unwrap().to_owned();
            let focus = crate::graph_live::GraphFocus {
                phase: "active".into(),
                summary: "Live: editing src/auth/guard.php".into(),
                target_folders: vec!["src/auth".into()],
                target_paths: vec!["src/auth/guard.php".into()],
                target_nodes: vec!["src/auth".into()],
                created_entities: crate::graph_live::diff_new_entities(
                    "src/auth/guard.php",
                    Some(""),
                    "<?php\nclass Guard {\n}\n",
                    false,
                ),
            };
            crate::graph_live::save_session_focus(&fixture.store, &sid, &focus).unwrap();

            let Payload::Json(map) = fixture
                .graphs
                .execute(
                    "graph.map",
                    json!({
                        "params": {"project_id": fixture.project},
                        "body": {},
                        "query": {"scope": "src", "focus": "src/app.php", "focus_session": sid},
                    }),
                )
                .await
                .unwrap()
            else {
                panic!("expected json map payload");
            };
            assert_eq!(map["message"], "Code graph & dependency map ready.");
            assert_eq!(map["focus"]["path"], "src/app.php", "{map}");
            assert_eq!(map["graph_focus"]["phase"], "active", "{map}");
            assert!(map.get("run").is_none(), "{map}");
            let auth_node = map["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["path"] == "src/auth")
                .expect("src/auth node present");
            assert_eq!(auth_node["focus_target"], true, "{auth_node}");
            assert_eq!(auth_node["created_entities_count"], 1, "{auth_node}");
        }
        #[test]
        fn auto_start_projects_prioritizes_most_recent_session_project_first() {
            let root = tempfile::tempdir().unwrap();
            let store = SessionStore::open(&root.path().join("state")).unwrap();
            let folder1 = root.path().join("project_first");
            let folder2 = root.path().join("project_second");
            std::fs::create_dir_all(&folder1).unwrap();
            std::fs::create_dir_all(&folder2).unwrap();
            let p1 = store.register(&folder1).unwrap();
            let p1_id = p1["id"].as_str().unwrap().to_owned();
            let p2 = store.register(&folder2).unwrap();
            let p2_id = p2["id"].as_str().unwrap().to_owned();
            // Associate a session with p2 so summaries() ranks p2 ahead of p1.
            store
                .create(
                    &json!({
                        "project_id": p2_id,
                        "project_path": folder2,
                        "provider": "codex",
                    }),
                    "Recent project",
                    &[],
                )
                .unwrap();

            let tool_dir = root.path().join("tool");
            std::fs::create_dir_all(&tool_dir).unwrap();
            let tool = tool_dir.join("graphify");
            let text = tool.with_extension("text");
            std::fs::write(&text, TOOL).unwrap();
            let copied = Command::new("cp")
                .arg("--")
                .arg(&text)
                .arg(&tool)
                .status()
                .unwrap();
            assert!(copied.success());
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();

            let graphs = Graphs::with(store, Some(tool), Duration::ZERO);
            assert_eq!(
                graphs.auto_start_order(),
                vec![p2_id.clone(), p1_id.clone()],
                "most-recently-active session's project must run first"
            );

            graphs.auto_start_projects();
            let started = Instant::now();
            loop {
                let s1 = graphs.status(&p1_id, false).unwrap();
                let s2 = graphs.status(&p2_id, false).unwrap();
                if s1["state"] == "ready" && s2["state"] == "ready" {
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(10),
                    "p1={s1} p2={s2}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            graphs.close();
        }
        #[test]
        fn auto_start_rebuilds_when_live_graph_is_missing() {
            let fixture = Fixture::new();
            fixture.graphs.auto_start(&fixture.project).unwrap();
            assert_eq!(fixture.settled()["state"], "ready");
            assert_eq!(fixture.runs(), 1);

            let graph = live(&fixture.store.state, &fixture.project);
            std::fs::remove_file(&graph).unwrap();
            assert!(!graph.is_file());

            fixture.graphs.auto_start(&fixture.project).unwrap();
            let rebuilt = fixture.settled();
            assert_eq!(rebuilt["state"], "ready", "{rebuilt}");
            assert!(graph.is_file());
            assert_eq!(fixture.runs(), 2);
        }
        #[test]
        fn non_git_workspace_edit_recomputes_on_finish_despite_automatic_gap() {
            let fixture = Fixture::new();
            std::fs::write(fixture.folder.join("service.php"), "<?php // v1\n").unwrap();
            let graphs = Graphs::with(
                fixture.store.clone(),
                Some(fixture.tool.clone()),
                Duration::from_secs(30),
            );
            graphs.auto_start(&fixture.project).unwrap();
            let settle = || {
                let started = Instant::now();
                loop {
                    let current = graphs.status(&fixture.project, false).unwrap();
                    if current["state"] != "building" {
                        return current;
                    }
                    assert!(started.elapsed() < Duration::from_secs(10));
                    std::thread::sleep(Duration::from_millis(20));
                }
            };
            assert_eq!(settle()["state"], "ready");
            assert_eq!(fixture.runs(), 1);

            // Unchanged non-Git workspace must not re-run.
            graphs.refresh(&fixture.project, Trigger::Finish).unwrap();
            assert_eq!(settle()["state"], "ready");
            assert_eq!(fixture.runs(), 1);

            // Ignored cache directories must not count as workspace source changes.
            std::fs::create_dir_all(fixture.folder.join("graphify-out")).unwrap();
            std::fs::write(fixture.folder.join("graphify-out/noise.txt"), "x").unwrap();
            graphs.refresh(&fixture.project, Trigger::Finish).unwrap();
            assert_eq!(settle()["state"], "ready");
            assert_eq!(fixture.runs(), 1);

            // An actual non-Git file edit triggers immediate Finish recompute even within the 30s gap.
            std::fs::write(
                fixture.folder.join("service.php"),
                "<?php // v2 with extra code\n",
            )
            .unwrap();
            graphs.refresh(&fixture.project, Trigger::Finish).unwrap();
            assert_eq!(settle()["state"], "ready");
            assert_eq!(fixture.runs(), 2);
            graphs.close();
        }
        #[test]
        fn job_view_always_reports_monotonic_percentage_and_english_message() {
            let job = Job::new();

            job.stage.store(WAITING, Ordering::Release);
            let wait = job.view(None);
            assert_eq!(wait["phase"], "wait");
            assert_eq!(wait["percent"], 5);
            assert!(wait["message"].as_str().unwrap().contains("5%"), "{wait}");

            job.stage.store(EXPORTING, Ordering::Release);
            let export = job.view(None);
            assert_eq!(export["phase"], "export");
            assert_eq!(export["percent"], 12);
            assert!(
                export["message"].as_str().unwrap().contains("12%"),
                "{export}"
            );

            job.stage.store(0, Ordering::Release);
            let scan = job.view(None);
            let scan_pct = scan["percent"].as_u64().expect("non-null scan percent");
            assert!((15..=28).contains(&scan_pct), "{scan}");
            assert!(
                scan["message"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("{scan_pct}%")),
                "{scan}"
            );

            job.observe(
                "  AST extraction: 1/4 uncached files (25%) [2 workers]",
                false,
            );
            let read = job.view(None);
            let read_pct = read["percent"].as_u64().expect("non-null read percent");
            assert!(
                read_pct >= scan_pct && (1..=99).contains(&read_pct),
                "{read}"
            );
            let read_msg = read["message"].as_str().unwrap();
            assert!(read_msg.contains("(1/4)"), "{read}");
            assert!(read_msg.contains(&format!("{read_pct}%")), "{read}");

            job.observe(
                "  AST extraction: 4/4 uncached files (100%) [2 workers]",
                false,
            );
            let assemble = job.view(None);
            let assemble_pct = assemble["percent"]
                .as_u64()
                .expect("non-null assemble percent");
            assert!(
                assemble_pct >= read_pct && (1..=99).contains(&assemble_pct),
                "{assemble}"
            );
            let assemble_msg = assemble["message"].as_str().unwrap();
            assert!(assemble_msg.contains("Computing"), "{assemble}");
            assert!(
                assemble_msg.contains(&format!("{assemble_pct}%")),
                "{assemble}"
            );
        }
        #[tokio::test]
        async fn domain_execute_graph_map_whole_validates_and_marks_unscoped_overview() {
            let fixture = Fixture::new();
            std::fs::write(
                fixture.folder.join("graph.fixture"),
                json!({
                    "nodes": [
                        {"id": "n1", "label": "App", "source_file": "src/app.php"},
                        {"id": "n2", "label": "Guard", "source_file": "src/auth/guard.php"}
                    ],
                    "links": [
                        {"source": "n1", "target": "n2", "relation": "calls", "confidence": "EXTRACTED"}
                    ]
                })
                .to_string(),
            )
            .unwrap();
            fixture.graphs.auto_start(&fixture.project).unwrap();
            assert_eq!(fixture.settled()["state"], "ready");

            let Payload::Json(overview) = fixture
                .graphs
                .execute(
                    "graph.map",
                    json!({
                        "params": {"project_id": fixture.project},
                        "body": {},
                        "query": {},
                    }),
                )
                .await
                .unwrap()
            else {
                panic!("expected json map payload");
            };
            assert_eq!(overview["whole"], json!(false), "{overview}");

            let Payload::Json(whole) = fixture
                .graphs
                .execute(
                    "graph.map",
                    json!({
                        "params": {"project_id": fixture.project},
                        "body": {},
                        "query": {"whole": "1"},
                    }),
                )
                .await
                .unwrap()
            else {
                panic!("expected json whole map payload");
            };
            assert_eq!(whole["whole"], json!(true), "{whole}");

            let invalid = fixture
                .graphs
                .execute(
                    "graph.map",
                    json!({
                        "params": {"project_id": fixture.project},
                        "body": {},
                        "query": {"whole": "2"},
                    }),
                )
                .await;
            assert!(invalid.is_err(), "expected invalid whole flag error");
        }
        #[tokio::test]
        async fn domain_execute_changes_and_file_impact_cover_non_git_and_git_projects() {
            // Non-Git project with NO Graphify build must still answer without error.
            let non_git = Fixture::new();
            std::fs::create_dir_all(non_git.folder.join("src/auth")).unwrap();
            std::fs::write(
                non_git.folder.join("src/auth/guard.php"),
                "<?php\nclass Guard {\n}\n",
            )
            .unwrap();

            let Payload::Json(empty_catalog) = non_git
                .graphs
                .execute(
                    "graph.changes",
                    json!({
                        "params": {"project_id": non_git.project},
                        "body": {},
                        "query": {"notests": "1"},
                    }),
                )
                .await
                .unwrap()
            else {
                panic!("expected json changes payload");
            };
            assert_eq!(empty_catalog["project_id"], non_git.project);
            assert_eq!(empty_catalog["target"], "workspace");
            assert_eq!(empty_catalog["notests"], true);
            assert_eq!(empty_catalog["total_files"], 0, "{empty_catalog}");

            let created = non_git
                .store
                .create(
                    &json!({
                        "project_id": non_git.project,
                        "project_path": non_git.folder,
                        "provider": "codex",
                    }),
                    "Non-Git focus catalog test",
                    &[],
                )
                .unwrap();
            let sid = created["id"].as_str().unwrap().to_owned();
            let focus = crate::graph_live::GraphFocus {
                phase: "active".into(),
                summary: "Live: editing src/auth/guard.php".into(),
                target_folders: vec!["src/auth".into()],
                target_paths: vec!["src/auth/guard.php".into()],
                target_nodes: vec!["src/auth".into()],
                created_entities: crate::graph_live::diff_new_entities(
                    "src/auth/guard.php",
                    Some(""),
                    "<?php\nclass Guard {\n}\n",
                    false,
                ),
            };
            crate::graph_live::save_session_focus(&non_git.store, &sid, &focus).unwrap();

            let Payload::Json(focused_catalog) = non_git
                .graphs
                .execute(
                    "graph.changes",
                    json!({
                        "params": {"project_id": non_git.project},
                        "body": {},
                        "query": {"focus_session": sid},
                    }),
                )
                .await
                .unwrap()
            else {
                panic!("expected json focused changes payload");
            };
            assert_eq!(focused_catalog["total_files"], 1, "{focused_catalog}");
            assert_eq!(
                focused_catalog["files"][0]["path"], "src/auth/guard.php",
                "{focused_catalog}"
            );

            let Payload::Json(non_git_impact) = non_git
                .graphs
                .execute(
                    "graph.file_impact",
                    json!({
                        "params": {"project_id": non_git.project},
                        "body": {},
                        "query": {"path": "src/auth/guard.php", "focus_session": sid},
                    }),
                )
                .await
                .unwrap()
            else {
                panic!("expected json non-git file impact payload");
            };
            assert_eq!(non_git_impact["project_id"], non_git.project);
            assert_eq!(non_git_impact["path"], "src/auth/guard.php");
            assert_eq!(non_git_impact["in_graph"], false);

            let bad_path = non_git
                .graphs
                .execute(
                    "graph.file_impact",
                    json!({
                        "params": {"project_id": non_git.project},
                        "body": {},
                        "query": {"path": "   "},
                    }),
                )
                .await;
            assert!(bad_path.is_err(), "expected empty path rejection");

            // Git repo without Graphify build still computes diff, dependencies, and callers.
            let git_fix = Fixture::new();
            git_fix.git(&["init", "-b", "main"]);
            std::fs::create_dir_all(git_fix.folder.join("src")).unwrap();
            git_fix.write("src/helper.rs", "pub fn helper() -> i32 {\n    1\n}\n");
            git_fix.write(
                "src/service.rs",
                "use crate::helper::helper;\npub fn run() -> i32 {\n    helper()\n}\n",
            );
            git_fix.write(
                "src/caller.rs",
                "use crate::service::run;\npub fn entry() -> i32 {\n    run()\n}\n",
            );
            git_fix.commit("initial");

            git_fix.write("src/extra.rs", "pub fn extra() -> i32 {\n    2\n}\n");
            git_fix.write(
                "src/service.rs",
                "use crate::extra::extra;\npub fn run_v2() -> i32 {\n    extra()\n}\n",
            );

            let Payload::Json(git_catalog) = git_fix
                .graphs
                .execute(
                    "graph.changes",
                    json!({
                        "params": {"project_id": git_fix.project},
                        "body": {},
                        "query": {"target": "work"},
                    }),
                )
                .await
                .unwrap()
            else {
                panic!("expected json git catalog payload");
            };
            assert_eq!(git_catalog["target"], "work");
            assert_eq!(git_catalog["total_files"], 2, "{git_catalog}");
            assert_eq!(git_catalog["created_files"], 1, "{git_catalog}");
            assert_eq!(git_catalog["modified_files"], 1, "{git_catalog}");

            let Payload::Json(service_impact) = git_fix
                .graphs
                .execute(
                    "graph.file_impact",
                    json!({
                        "params": {"project_id": git_fix.project},
                        "body": {},
                        "query": {"path": "src/service.rs", "target": "work", "certain": "1"},
                    }),
                )
                .await
                .unwrap()
            else {
                panic!("expected json service file impact payload");
            };
            assert_eq!(service_impact["path"], "src/service.rs");
            assert_eq!(service_impact["status"], "modified", "{service_impact}");
            assert_eq!(
                service_impact["summary"]["uses_added"], 2,
                "{service_impact}"
            );
            assert_eq!(
                service_impact["summary"]["uses_removed"], 2,
                "{service_impact}"
            );
            assert!(
                service_impact["uses"]["added"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|dep| dep["target"] == "src/extra.rs"),
                "{service_impact}"
            );
            assert!(
                service_impact["uses"]["removed"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|dep| dep["target"] == "src/helper.rs"),
                "{service_impact}"
            );
            assert!(
                service_impact["used_by"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|caller| caller["path"] == "src/caller.rs"),
                "{service_impact}"
            );
        }
    }
}
