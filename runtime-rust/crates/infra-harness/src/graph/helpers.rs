use super::*;

/// Assembly and clustering time relative to file reading, used until a project
/// has a finished build of its own. One repository measured 4.1 with Graphify
/// 0.9.105 and 4.8 with 0.7.13.
pub(super) const TAIL_RATIO: f64 = 4.5;
pub(super) const BUILD_LIMIT: Duration = Duration::from_secs(30 * 60);
/// Automatic rebuilds of one project keep at least this distance.
pub(super) const AUTOMATIC_GAP: Duration = Duration::from_secs(30);
pub(super) const PRINT_AGE: Duration = Duration::from_secs(10);
pub(super) const TOOL_AGE: Duration = Duration::from_secs(30);
pub(super) const PROBE_LIMIT: Duration = Duration::from_secs(5);
pub(super) const LINE_LIMIT: usize = 300;
/// How long a listing of open pull requests is reused.
pub(super) const PULLS_AGE: Duration = Duration::from_secs(60);
/// Graphs of other commits kept for a project; each is as large as its own.
pub(super) const REVISIONS: usize = 3;
/// Maximum preserved source bytes kept per commit/PR revision tree for Access and Crates checks.
pub(super) const MAX_THIN_TREE_BYTES: u64 = 64 * 1024 * 1024;
pub(super) const WATCHER_DEBOUNCE: Duration = Duration::from_millis(150);
pub(super) const WATCHER_MAX_DELAY: Duration = Duration::from_millis(600);
pub(super) const MAX_WATCHER_BATCH: usize = 512;
pub(super) const NATIVE_READY_MESSAGE: &str =
    "Code graph & dependency map ready (native tree-sitter SQLite IR).";
/// What a build is doing before the tool runs.
pub(super) const WAITING: u8 = 1;
pub(super) const EXPORTING: u8 = 2;
/// Keys that could let the tool reach a model provider. The update pass needs
/// none of them, and the graph of a project stays on this machine.
pub(super) const PROVIDER_KEYS: &[&str] = &[
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
pub(super) const NO_SKILL_REFRESH: (&str, &str) = ("GRAPHIFY_NO_AUTO_REFRESH", "1");
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
pub(super) fn number(text: &str) -> Option<(u64, &str)> {
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
    pub(super) first: Option<(f64, u64)>,
    pub(super) last: Option<(f64, u64)>,
    pub(super) total: u64,
    pub(super) read: Option<f64>,
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
/// How long the removal of a graph's folder is tried again on Windows.
pub(super) const REMOVAL_SETTLE: Duration = Duration::from_secs(3);
/// Removes a graph's folder. On Windows a file that was closed a moment ago,
/// by the store that was just let go or by a scanner reading what a build
/// wrote, can still refuse to be deleted, and its folder with it. There the
/// removal is tried again for a short while before its error is believed;
/// elsewhere an open file does not hold its folder, and one attempt is the
/// answer.
pub(super) fn remove_settling(home: &Path) -> std::io::Result<()> {
    let settle = if cfg!(windows) {
        REMOVAL_SETTLE
    } else {
        Duration::ZERO
    };
    remove_until(home, settle, |home| std::fs::remove_dir_all(home))
}
pub(super) fn remove_until(
    home: &Path,
    settle: Duration,
    remove: impl Fn(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let until = Instant::now() + settle;
    loop {
        match remove(home) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) if Instant::now() >= until => return Err(error),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}
pub(super) fn probe(program: &Path, arguments: &[&str]) -> Option<String> {
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
pub(super) fn release(text: &str) -> Option<String> {
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
pub(super) fn version(program: &Path) -> Option<String> {
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
pub(super) const WORKSPACE_PRINT_ENTRIES: usize = 20_000;
pub(super) const WORKSPACE_PRINT_DEPTH: usize = 32;
pub(super) const WORKSPACE_PRINT_IGNORED: &[&str] = &[
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

pub(super) fn workspace_print(root: &Path) -> Option<Print> {
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
pub(super) fn key(project: &str) -> String {
    format!("graph.{project}")
}
pub(super) fn folder(state: &Path, project: &str) -> PathBuf {
    state.join("graphs").join(project)
}
/// The published graph an agent queries.
pub fn live(state: &Path, project: &str) -> PathBuf {
    folder(state, project).join("live").join("graph.json")
}
/// The graph of another commit of the project, as a pull request needs it.
pub(super) fn revision(state: &Path, project: &str, commit: &str) -> PathBuf {
    folder(state, project)
        .join("revisions")
        .join(commit)
        .join("graph.json")
}
/// Preserves source files in a checked-out commit/PR tree up to per-file and
/// per-tree byte bounds so `Access` and crate checks work accurately, falling
/// back to `.rs.modules` when Rust files exceed bounds and stripping non-source clutter.
pub(super) fn is_preserved_source_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    if name.ends_with(".min.js") || name.ends_with(".min.mjs") || name.ends_with(".min.cjs") {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "composer.json"
            | "package.json"
            | "pyproject.toml"
            | "cargo.toml"
            | "go.mod"
            | "tsconfig.json"
            | "jsconfig.json"
    ) || ((lower.starts_with("tsconfig.") || lower.starts_with("jsconfig."))
        && lower.ends_with(".json"))
    {
        return true;
    }
    crate::graph_live::detect_language(name).is_some()
}

pub(super) fn thin(tree: &Path) -> Option<crate::graph_map::TestSpans> {
    thin_with_limits(tree, crate::graph_map::SOURCE_LIMIT, MAX_THIN_TREE_BYTES)
}

pub(super) fn thin_with_limits(
    tree: &Path,
    max_file_bytes: u64,
    max_total_bytes: u64,
) -> Option<crate::graph_map::TestSpans> {
    let mut folders = vec![tree.to_path_buf()];
    let mut at = 0;
    let mut complete = true;
    let mut tests = BTreeMap::new();
    let mut total_kept_bytes = 0u64;
    while at < folders.len() {
        // The folder is listed before anything is written into it.
        let mut entries = match std::fs::read_dir(&folders[at])
            .and_then(|entries| entries.collect::<std::io::Result<Vec<_>>>())
        {
            Ok(entries) => entries,
            Err(_) => {
                at += 1;
                complete = false;
                continue;
            }
        };
        entries.sort_by_key(|entry| entry.file_name());
        at += 1;
        let mut kept = Vec::new();
        for entry in entries {
            let path = entry.path();
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => {
                    if WORKSPACE_PRINT_IGNORED.contains(&name_str.as_ref()) {
                        let _ = std::fs::remove_dir_all(&path);
                    } else {
                        folders.push(path);
                    }
                    continue;
                }
                Ok(kind) if kind.is_file() && name_str == "Cargo.toml" => continue,
                Ok(kind) if kind.is_file() && path.extension().is_some_and(|end| end == "rs") => {
                    match crate::graph_map::kept_tests(&path) {
                        Ok(spans) if !spans.is_empty() => {
                            let relative = path.strip_prefix(tree).ok()?;
                            tests.insert(relative.to_string_lossy().replace('\\', "/"), spans);
                        }
                        Ok(_) => {}
                        Err(_) => complete = false,
                    }
                    let len = std::fs::symlink_metadata(&path).ok().map(|m| m.len());
                    if let Some(len) = len
                        && len <= max_file_bytes
                        && total_kept_bytes.saturating_add(len) <= max_total_bytes
                    {
                        total_kept_bytes += len;
                        continue;
                    }
                    if let Some(modules) = crate::graph_crates::kept(&path) {
                        let mut name = path.clone().into_os_string();
                        name.push(".");
                        name.push(crate::graph_crates::MODULES);
                        kept.push((PathBuf::from(name), modules));
                    }
                }
                Ok(kind) if kind.is_file() && is_preserved_source_path(&path) => {
                    let len = std::fs::symlink_metadata(&path).ok().map(|m| m.len());
                    if let Some(len) = len
                        && len <= max_file_bytes
                        && total_kept_bytes.saturating_add(len) <= max_total_bytes
                    {
                        total_kept_bytes += len;
                        continue;
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

pub(super) fn stream_graph_summary_fields(path: &Path) -> Option<(usize, usize, Option<String>)> {
    use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
    use std::fmt;

    struct CountSeq;
    impl<'de> Visitor<'de> for CountSeq {
        type Value = usize;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a JSON array")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<usize, A::Error> {
            let mut count = 0usize;
            while seq.next_element::<IgnoredAny>()?.is_some() {
                count += 1;
            }
            Ok(count)
        }
    }
    struct ArrayCount(usize);
    impl<'de> serde::Deserialize<'de> for ArrayCount {
        fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            Ok(Self(d.deserialize_seq(CountSeq)?))
        }
    }

    struct SummaryVisitor;
    impl<'de> Visitor<'de> for SummaryVisitor {
        type Value = (usize, usize, Option<String>);
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a graph object")
        }
        fn visit_map<A: MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut nodes = 0usize;
            let mut edges = 0usize;
            let mut commit: Option<String> = None;
            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "nodes" => nodes = map.next_value::<ArrayCount>()?.0,
                    "links" | "edges" => edges = map.next_value::<ArrayCount>()?.0,
                    "commit" => {
                        let val: Option<String> = map.next_value()?;
                        commit = val.filter(|s| !s.is_empty());
                    }
                    _ => {
                        let _ = map.next_value::<IgnoredAny>()?;
                    }
                }
            }
            Ok((nodes, edges, commit))
        }
    }

    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    let mut de = serde_json::Deserializer::from_reader(reader);
    de.deserialize_map(SummaryVisitor).ok()
}

pub(super) fn write_live_summary(
    live_graph: &Path,
    commit: Option<&str>,
    exact_counts: Option<(u64, u64)>,
    fallback: Option<(u64, u64)>,
) {
    let Ok(meta) = std::fs::metadata(live_graph) else {
        return;
    };
    let streamed = if exact_counts.is_none() {
        stream_graph_summary_fields(live_graph)
    } else {
        None
    };
    let Some((nodes, edges)) = exact_counts
        .map(|(n, e)| (n as usize, e as usize))
        .or_else(|| streamed.as_ref().map(|(n, e, _)| (*n, *e)))
        .or_else(|| fallback.map(|(n, e)| (n as usize, e as usize)))
    else {
        return;
    };
    let resolved_commit = commit
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| streamed.and_then(|(_, _, c)| c));
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
    let summary = json!({
        "nodes": nodes,
        "edges": edges,
        "commit": resolved_commit,
        "graph_len": meta.len(),
        "graph_mtime_secs": modified.map(|d| d.as_secs()),
        "graph_mtime_nanos": modified.map(|d| d.subsec_nanos()),
    });
    let Ok(bytes) = serde_json::to_vec(&summary) else {
        return;
    };
    let summary_path = live_graph.with_file_name("summary.json");
    let tmp_path = live_graph.with_file_name("summary.json.tmp");
    if std::fs::write(&tmp_path, bytes).is_err()
        || std::fs::rename(&tmp_path, &summary_path).is_err()
    {
        let _ = std::fs::remove_file(&tmp_path);
    }
}
/// What was chosen, for the interface: the two sides and how many files.
pub(super) fn chosen(resolved: &Resolved) -> Value {
    let mut chosen = resolved.json();
    chosen["files"] = json!(resolved.files.len());
    chosen
}
/// A run of an agent as a change: the files it changed are what is mapped,
/// the files it opened are laid over the map, and the rest is what a picture
/// says of the run beside itself.
#[derive(Clone)]
pub(crate) struct RunChange {
    pub(crate) resolved: Arc<Resolved>,
    pub(crate) opened: BTreeSet<String>,
    pub(crate) shell_reads: BTreeMap<String, u64>,
    pub(crate) told: Value,
}
pub(super) fn size(path: &Path) -> u64 {
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
pub(super) fn tenth(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}
pub(super) fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| HarnessError::unavailable("The code graph state is unavailable."))
}
/// A path as one word of a shell command.
pub(super) fn word(path: &Path) -> String {
    spelled(&path.to_string_lossy(), cfg!(windows))
}
/// On Windows a drive path is written with forward separators: Claude Code
/// runs commands in Git Bash, where an unquoted backslash is an escape, while
/// PowerShell and the tool itself take either separator. A network or
/// extended path keeps its backslashes, which mean something there, in quotes.
pub(super) fn spelled(text: &str, windows: bool) -> String {
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
pub(super) const QUESTIONS: [(&str, &str); 3] = [
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
pub(super) struct Offer {
    pub(super) command: String,
    pub(super) graph: String,
    pub(super) built: Value,
}
pub(super) fn offer_with_override(
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
pub(super) fn offer(store: &SessionStore, project: &str) -> Result<Option<Offer>> {
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
pub(super) struct Seen {
    pub(super) observed: Observed,
    pub(super) built: Option<(u64, u64)>,
    /// Last line of the ordinary output and of the error output; a failure is
    /// explained by the error output when there is any.
    pub(super) line: String,
    pub(super) complaint: String,
    pub(super) peak: u64,
}
pub(super) struct Job {
    pub(super) started: Mutex<Instant>,
    /// `WAITING` or `EXPORTING` until the tool runs.
    pub(super) stage: AtomicU8,
    pub(super) cancel: AtomicBool,
    pub(super) purge: AtomicBool,
    pub(super) again: AtomicBool,
    /// Deferred watcher relative paths collected while a build job was active.
    pub(super) pending_paths: Mutex<Vec<String>>,
    pub(super) seen: Mutex<Seen>,
    /// Replacement revision jobs share this gate until their work has finished.
    pub(super) revision_gate: Arc<Mutex<()>>,
}
impl Job {
    pub(super) fn new() -> Self {
        Self {
            started: Mutex::new(Instant::now()),
            stage: AtomicU8::new(0),
            cancel: AtomicBool::new(false),
            purge: AtomicBool::new(false),
            again: AtomicBool::new(false),
            pending_paths: Mutex::new(Vec::new()),
            seen: Mutex::new(Seen::default()),
            revision_gate: Arc::new(Mutex::new(())),
        }
    }
    pub(super) fn started(&self) -> Instant {
        *self
            .started
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// The estimate counts the tool's own time, not the wait before it.
    pub(super) fn restart(&self) {
        *self
            .started
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
    }
    pub(super) fn observe(&self, line: &str, complaint: bool) {
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
    pub(super) fn view(&self, history: Option<History>) -> Value {
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
pub(super) enum Outcome {
    Built(Value),
    Failed(String),
    Cancelled,
}
/// A run of the tool that left a graph.
pub(super) struct Ran {
    pub(super) fresh: PathBuf,
    pub(super) elapsed: f64,
    pub(super) read: Option<f64>,
    pub(super) built: Option<(u64, u64)>,
    pub(super) bytes: u64,
}
impl Ran {
    pub(super) fn record(&self, disk: u64) -> Value {
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
pub(super) struct Found {
    pub(super) state: &'static str,
    pub(super) build: Value,
    pub(super) failure: Option<String>,
    /// The name the read graph is kept under, the graph, and the folder its
    /// paths are relative to.
    pub(super) graph: Option<(String, PathBuf, PathBuf)>,
}
pub(super) enum RevisionBuild<'a> {
    Normal,
    Comparison { head: &'a str },
}
pub(super) enum Manifest {
    Absent,
    Present(Vec<u8>, Option<std::time::SystemTime>),
    /// It exists and could not be read, so it is left alone.
    Unknown,
}
/// Graphify before 0.9 writes its manifest into the project's own
/// `graphify-out/` even when the graph goes elsewhere. That folder may hold a
/// graph the user built themselves, so it is put back as it was when the build
/// ends, however it ends.
pub(super) struct Kept {
    pub(super) folder: PathBuf,
    pub(super) existed: bool,
    pub(super) manifest: Manifest,
}
impl Kept {
    pub(super) fn of(root: &Path) -> Self {
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
pub(super) fn pump(
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
pub(super) fn end(child: &mut Child) {
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
pub(super) fn status_message(state: &str, build: Option<&Value>, failure: Option<&str>) -> String {
    status_message_for_tool(state, build, failure, None)
}

pub(super) fn status_message_for_tool(
    state: &str,
    build: Option<&Value>,
    failure: Option<&str>,
    tool: Option<&str>,
) -> String {
    match state {
        "building" => build
            .and_then(|value| value["message"].as_str())
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "Building code graph…".to_owned()),
        "ready" => {
            if tool == Some(crate::graph_ir::NATIVE_IR_TOOL) {
                NATIVE_READY_MESSAGE.to_owned()
            } else {
                "Code graph & dependency map ready.".to_owned()
            }
        }
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

pub(super) fn revision_message(state: &str, build: &Value, failure: Option<&str>) -> String {
    match state {
        "missing" => "Code graph for this revision is not built yet.".to_owned(),
        _ => status_message(state, Some(build), failure),
    }
}

pub(super) fn apply_graph_focus(
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
