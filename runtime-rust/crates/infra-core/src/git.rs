//! Bounded native Git discovery and source-policy queries.
use crate::{Result, RuntimeError};
use std::{
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct GitOutput {
    pub code: i32,
    pub stdout: Vec<u8>,
}
pub fn strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn git_override(key: &std::ffi::OsStr) -> bool {
    #[cfg(windows)]
    {
        key.to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
    }
    #[cfg(not(windows))]
    {
        key.to_string_lossy().starts_with("GIT_")
    }
}

pub fn run(
    project: &Path,
    arguments: &[OsString],
    input: Option<Vec<u8>>,
    timeout: Duration,
    output_limit: usize,
) -> Result<GitOutput> {
    if timeout.is_zero() {
        return Err(RuntimeError::unavailable("Git source context unavailable"));
    }
    let mut command = Command::new("git");
    command
        .args(["--no-optional-locks", "--no-pager", "-c"])
        .arg(format!(
            "core.hooksPath={}",
            if cfg!(windows) { "NUL" } else { "/dev/null" }
        ))
        .args(["-c", "core.fsmonitor=false", "-C"])
        .arg(project)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    for (key, _) in std::env::vars_os() {
        if git_override(&key) {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    let mut child = command.spawn().map_err(|cause| {
        let mut error = RuntimeError::unavailable("Git source context unavailable");
        error.os_code = cause.raw_os_error();
        error
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| RuntimeError::unavailable("Git stdout unavailable"))?;
    let waiter = thread::current();
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let read = stdout
            .take(output_limit as u64 + 1)
            .read_to_end(&mut output)
            .map(|_| output);
        // The output ends with Git's exit, which the wait below would
        // otherwise only notice on its next look.
        waiter.unpark();
        read
    });
    let writer = input.map(|input| {
        let stdin = child.stdin.take();
        thread::spawn(move || -> std::io::Result<()> {
            if let Some(mut stdin) = stdin {
                stdin.write_all(&input)?;
            }
            Ok(())
        })
    });
    let started = Instant::now();
    let deadline = started + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            // Most Git queries answer within a few milliseconds: noticing the
            // exit late should cost at most a twentieth of the wait.
            Ok(None) if Instant::now() < deadline => thread::park_timeout(
                (started.elapsed() / 20)
                    .clamp(Duration::from_micros(100), Duration::from_millis(5)),
            ),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                if let Some(writer) = writer {
                    let _ = writer.join();
                }
                return Err(RuntimeError::unavailable("Git source context unavailable"));
            }
        }
    };
    if let Some(writer) = writer {
        writer
            .join()
            .map_err(|_| RuntimeError::unavailable("Git input unavailable"))?
            .map_err(|_| RuntimeError::unavailable("Git input unavailable"))?;
    }
    let output = reader
        .join()
        .map_err(|_| RuntimeError::unavailable("Git output unavailable"))?
        .map_err(|_| RuntimeError::unavailable("Git output unavailable"))?;
    if output.len() > output_limit {
        return Err(RuntimeError::unavailable("Git output limit exceeded"));
    }
    Ok(GitOutput {
        code: status.code().unwrap_or(-1),
        stdout: output,
    })
}

fn source_git(mut error: RuntimeError) -> RuntimeError {
    error.code = Some("source-git".into());
    error
}
/// A Codex thread identifier: a lowercase hyphenated UUID.
fn codex_thread(id: &str) -> bool {
    let groups = id.split('-').collect::<Vec<_>>();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, length)| {
            group.len() == length
                && group
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
}
fn strip_lf(mut text: &str) -> &str {
    while let Some(previous) = text.strip_suffix('\n') {
        text = previous;
    }
    text
}
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    rc::Rc,
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFrame {
    pub project: PathBuf,
    pub top: PathBuf,
    pub prefix: String,
    pub prefix_bytes: Vec<u8>,
    pub admin: PathBuf,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct FrameIdentity(Vec<Option<crate::fs::NativeIdentity>>);
type FrameKey = (PathBuf, Vec<(OsString, OsString)>);
#[derive(Default)]
struct CommandState {
    frames: BTreeMap<FrameKey, (GitFrame, FrameIdentity)>,
    memos: BTreeMap<String, BTreeMap<String, Value>>,
}
thread_local! {static SCOPE:RefCell<Option<CommandState>>=const{RefCell::new(None)};}
pub struct SourceFrameScope {
    previous: Option<CommandState>,
    _same_thread: PhantomData<Rc<()>>,
}
pub fn source_frame_scope() -> SourceFrameScope {
    let previous = SCOPE.with(|scope| scope.replace(Some(CommandState::default())));
    SourceFrameScope {
        previous,
        _same_thread: PhantomData,
    }
}
impl Drop for SourceFrameScope {
    fn drop(&mut self) {
        SCOPE.with(|scope| {
            scope.replace(self.previous.take());
        });
    }
}
pub fn command_memo_get(name: &str, key: &str) -> Option<Value> {
    SCOPE.with(|scope| scope.borrow().as_ref()?.memos.get(name)?.get(key).cloned())
}
pub fn command_memo_set(name: &str, key: String, value: Value) {
    SCOPE.with(|scope| {
        if let Some(scope) = scope.borrow_mut().as_mut() {
            scope
                .memos
                .entry(name.into())
                .or_default()
                .insert(key, value);
        }
    })
}
fn frame_key(project: &Path) -> FrameKey {
    let mut environment = std::env::vars_os()
        .filter(|(k, _)| !git_override(k))
        .collect::<Vec<_>>();
    environment.sort();
    (project.to_owned(), environment)
}
fn image(path: &Path, contents: bool) -> Result<Option<crate::fs::NativeIdentity>> {
    Ok(crate::fs::native_image(path)?.map(|identity| {
        if contents {
            identity
        } else {
            identity.without_contents()
        }
    }))
}
fn frame_identity(project: &Path, frame: &GitFrame) -> Result<FrameIdentity> {
    let project = crate::fs::canonicalize(project)?;
    let mut rows = Vec::new();
    let mut found = false;
    for directory in project.ancestors() {
        rows.push(image(directory, false)?);
        rows.push(image(&directory.join(".git"), true)?);
        if directory == frame.top {
            found = true;
            break;
        }
    }
    if !found {
        return Err(RuntimeError::unavailable(
            "Git frame ancestry is unavailable",
        ));
    }
    for (path, contents) in [
        (&frame.admin, false),
        (&frame.admin.join("config"), true),
        (&frame.admin.join("commondir"), true),
    ] {
        rows.push(image(path, contents)?);
    }
    Ok(FrameIdentity(rows))
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchSnapshot {
    pub admin: PathBuf,
    pub marker: Option<crate::fs::NativeIdentity>,
    pub head: crate::fs::NativeIdentity,
    pub reftable: Option<crate::fs::NativeIdentity>,
}
pub fn branch_snapshot(project: &Path) -> Option<BranchSnapshot> {
    let project = crate::fs::canonicalize(project).ok()?;
    let checkout = checkout(&project).ok()??;
    let marker = checkout.join(".git");
    let admin = admin(&checkout)?;
    let head = image(&admin.join("HEAD"), true).ok()??;
    Some(BranchSnapshot {
        admin: admin.clone(),
        marker: image(&marker, true).ok()?,
        head,
        reftable: image(&admin.join("reftable/tables.list"), true).ok()?,
    })
}
/// The administrative directory a checkout's `.git` marker names.
fn admin(checkout: &Path) -> Option<PathBuf> {
    let marker = checkout.join(".git");
    let metadata = fs::metadata(&marker).ok()?;
    if metadata.is_dir() {
        crate::fs::canonicalize(&marker).ok()
    } else if metadata.is_file() {
        let text = fs::read_to_string(&marker).ok()?;
        let pointer = text.strip_prefix("gitdir: ")?.trim();
        let path = Path::new(pointer);
        crate::fs::canonicalize(if path.is_absolute() {
            path.to_owned()
        } else {
            checkout.join(path)
        })
        .ok()
    } else {
        None
    }
}
/// The identity of the frame `project` would have, read before Git is asked.
fn provisional_identity(project: &Path) -> Option<FrameIdentity> {
    let project = crate::fs::canonicalize(project).ok()?;
    let top = checkout(&project).ok()??;
    let admin = admin(&top)?;
    let frame = GitFrame {
        project: project.clone(),
        top,
        prefix: String::new(),
        prefix_bytes: Vec::new(),
        admin,
    };
    frame_identity(&project, &frame).ok()
}

fn checkout(project: &Path) -> Result<Option<PathBuf>> {
    if !project.is_dir() {
        return Err(RuntimeError::unavailable("Git source context unavailable"));
    }
    for directory in project.ancestors() {
        let marker = directory.join(".git");
        let observed = match fs::symlink_metadata(&marker) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(RuntimeError::unavailable("Git source context unavailable")),
        };
        let metadata =
            if observed.file_type().is_symlink() || crate::fs::linked_at(&marker, &observed)? {
                fs::metadata(&marker)
                    .map_err(|_| RuntimeError::unavailable("Git source context unavailable"))?
            } else {
                observed
            };
        if metadata.is_dir() {
            let head = fs::symlink_metadata(marker.join("HEAD"))
                .ok()
                .is_some_and(|m| m.is_file() || m.file_type().is_symlink());
            if !head || !marker.join("objects").is_dir() || !marker.join("refs").is_dir() {
                continue;
            }
        } else if !metadata.is_file() {
            return Err(RuntimeError::unavailable("Git source context unavailable"));
        }
        return Ok(Some(directory.to_owned()));
    }
    Ok(None)
}

/// The frame of a checkout whose `.git` is a plain directory, read without
/// starting Git (about 1 ms of every engine process): Git answers the same top,
/// prefix and administrative directory there. Every layout Git could see
/// differently is left to Git: a `.git` file or link, a checkout another user
/// owns (`safe.directory`), a configuration that can move or remove the work
/// tree, and a file-system boundary Git would not cross on its way up.
fn plain_frame(project: &Path, checkout: &Path) -> Option<GitFrame> {
    let marker = checkout.join(".git");
    let metadata = fs::symlink_metadata(&marker).ok()?;
    if !metadata.is_dir() || crate::fs::linked_at(&marker, &metadata).ok()? {
        return None;
    }
    if !owned_on_one_device(project, checkout, &metadata) {
        return None;
    }
    let config = fs::read_to_string(marker.join("config"))
        .ok()?
        .to_ascii_lowercase();
    let bare = config.lines().any(|line| {
        let line = line.trim();
        let (key, value) = line.split_once('=').unwrap_or((line, "true"));
        key.trim() == "bare" && value.trim() != "false"
    });
    if bare || config.contains("worktree") || config.contains("include") {
        return None;
    }
    let admin = crate::fs::canonicalize(&marker).ok()?;
    let relative = project.strip_prefix(checkout).ok()?;
    let mut prefix_bytes = os_bytes(relative.as_os_str());
    if !prefix_bytes.is_empty() {
        prefix_bytes.push(b'/');
    }
    Some(GitFrame {
        project: project.to_owned(),
        top: checkout.to_owned(),
        prefix: String::from_utf8_lossy(&prefix_bytes).into_owned(),
        prefix_bytes,
        admin,
    })
}
/// Whether this user owns the checkout and its `.git`, and every directory from
/// `project` up to the checkout lies on the checkout's file system.
#[cfg(unix)]
fn owned_on_one_device(project: &Path, checkout: &Path, marker: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    let user = unsafe { libc::geteuid() };
    let Ok(top) = fs::metadata(checkout) else {
        return false;
    };
    top.uid() == user
        && marker.uid() == user
        && project
            .ancestors()
            .take_while(|directory| *directory != checkout)
            .all(|directory| fs::metadata(directory).is_ok_and(|meta| meta.dev() == top.dev()))
}
/// Ownership has no such cheap reading here, so Git is asked.
#[cfg(not(unix))]
fn owned_on_one_device(_: &Path, _: &Path, _: &fs::Metadata) -> bool {
    false
}

impl GitFrame {
    fn discover_uncached(project: &Path, deadline: Instant) -> Result<Option<Self>> {
        let project = crate::fs::canonicalize(project)?;
        let Some(checkout) = checkout(&project)? else {
            return Ok(None);
        };
        if let Some(frame) = plain_frame(&project, &checkout) {
            return Ok(Some(frame));
        }
        Self::asked(&project, &checkout, deadline).map(Some)
    }
    /// The frame Git reports for `project`, which must lie in `checkout`.
    fn asked(project: &Path, checkout: &Path, deadline: Instant) -> Result<Self> {
        let project = project.to_owned();
        let result = run(
            &project,
            &strings(&[
                "rev-parse",
                "--show-toplevel",
                "--show-prefix",
                "--absolute-git-dir",
            ]),
            None,
            deadline.saturating_duration_since(Instant::now()),
            16384,
        )?;
        let lines = result
            .stdout
            .split(|byte| *byte == b'\n')
            .collect::<Vec<_>>();
        if result.code != 0 || lines.len() != 4 || !lines[3].is_empty() {
            return Err(RuntimeError::unavailable("Git source context unavailable"));
        }
        let top_name = path_bytes(lines[0])?;
        let admin_name = path_bytes(lines[2])?;
        if !top_name.is_absolute() || !admin_name.is_absolute() {
            return Err(RuntimeError::unavailable(
                "Git source frame paths are not absolute",
            ));
        }
        let top = crate::fs::canonicalize(top_name)?;
        let admin = crate::fs::canonicalize(admin_name)?;
        let relative = project
            .strip_prefix(&top)
            .map_err(|_| RuntimeError::unavailable("Git source context unavailable"))?;
        let mut expected = os_bytes(relative.as_os_str());
        if !expected.is_empty() {
            expected.push(b'/')
        }
        if top != checkout || !admin.is_dir() || lines[1] != expected {
            return Err(RuntimeError::unavailable("Git source context unavailable"));
        }
        Ok(Self {
            project,
            top,
            prefix: String::from_utf8_lossy(lines[1]).into_owned(),
            prefix_bytes: lines[1].to_vec(),
            admin,
        })
    }
    pub fn discover(project: &Path) -> Result<Option<Self>> {
        Self::discover_held(project, Instant::now() + Duration::from_secs(5)).map_err(source_git)
    }
    /// Within a command scope, a frame whose checkout ancestry and
    /// administrative files are unchanged is not asked of Git again. A frame is
    /// held only when nothing changed while Git was looking.
    fn discover_held(project: &Path, deadline: Instant) -> Result<Option<Self>> {
        if SCOPE.with(|scope| scope.borrow().is_none()) {
            return Self::discover_uncached(project, deadline);
        }
        let key = frame_key(project);
        let held = SCOPE.with(|scope| {
            scope
                .borrow()
                .as_ref()
                .and_then(|scope| scope.frames.get(&key).cloned())
        });
        if let Some((frame, identity)) = held
            && frame_identity(project, &frame).ok().as_ref() == Some(&identity)
        {
            return Ok(Some(frame));
        }
        let before = provisional_identity(project);
        let frame = Self::discover_uncached(project, deadline)?;
        if let (Some(frame), Some(before)) = (&frame, before)
            && frame_identity(project, frame).ok().as_ref() == Some(&before)
        {
            SCOPE.with(|scope| {
                if let Some(scope) = scope.borrow_mut().as_mut() {
                    scope.frames.insert(key, (frame.clone(), before));
                }
            });
        }
        Ok(frame)
    }
    pub fn branch(&self) -> Result<Option<String>> {
        self.branch_inner().map_err(source_git)
    }
    fn branch_inner(&self) -> Result<Option<String>> {
        let result = run(
            &self.project,
            &strings(&["symbolic-ref", "--quiet", "--short", "HEAD"]),
            None,
            Duration::from_secs(5),
            16384,
        )?;
        if result.code == 1 {
            return Ok(None);
        }
        if result.code != 0 {
            return Err(RuntimeError::unavailable("Git branch unavailable"));
        }
        String::from_utf8(result.stdout)
            .map(|text| Some(strip_lf(&text).into()))
            .map_err(|_| RuntimeError::invalid("Git branch is not UTF-8"))
    }
    /// The branch a checkout works on: its current branch, else the branch a
    /// rebase rewrites or a bisect started from (both detach HEAD on the
    /// commits they visit, and the branch is still the work in hand), else
    /// the Codex thread that owns a detached worktree. The hooks name their
    /// task by it and a capsule is delivered only while it still holds, so
    /// both must read it the same way.
    pub fn working_identity(&self) -> Result<Option<String>> {
        if let Some(branch) = self.branch()?.filter(|branch| !branch.is_empty()) {
            return Ok(Some(branch));
        }
        for name in ["rebase-merge/head-name", "rebase-apply/head-name"] {
            if let Ok(value) = fs::read_to_string(self.admin.join(name))
                && let Some(branch) = value
                    .lines()
                    .next()
                    .and_then(|value| value.strip_prefix("refs/heads/"))
                    .filter(|value| !value.is_empty())
            {
                return Ok(Some(branch.into()));
            }
        }
        // A bisect started on a detached HEAD names a commit, not a branch.
        if let Ok(value) = fs::read_to_string(self.admin.join("BISECT_START"))
            && let Some(branch) = value.lines().next().filter(|value| {
                !value.is_empty()
                    && !(matches!(value.len(), 40 | 64)
                        && value.bytes().all(|b| b.is_ascii_hexdigit()))
            })
        {
            return Ok(Some(branch.into()));
        }
        let path = self.admin.join("codex-thread.json");
        if let Ok(meta) = fs::symlink_metadata(&path)
            && !crate::fs::linked(&meta)
            && meta.is_file()
            && meta.len() <= 4096
        {
            let value: Value =
                serde_json::from_slice(&crate::fs::read_regular(&path)?).unwrap_or(Value::Null);
            if value["version"] == 1
                && let Some(id) = value["ownerThreadId"].as_str()
                && codex_thread(id)
            {
                return Ok(Some(format!("codex-thread/{id}")));
            }
        }
        Ok(None)
    }
    fn pinned(&self) -> Vec<OsString> {
        let mut admin = OsString::from("--git-dir=");
        admin.push(&self.admin);
        let mut top = OsString::from("--work-tree=");
        top.push(&self.top);
        vec![admin, top]
    }
    fn tracked(&self, members: &[Vec<u8>], deadline: Instant) -> Result<BTreeSet<Vec<u8>>> {
        let wanted = members.iter().cloned().collect::<BTreeSet<_>>();
        if members.len() > 64 {
            let prefixes = members
                .iter()
                .map(|name| {
                    let mut pieces = name.split(|b| *b == b'/');
                    let first = pieces.next().unwrap_or(&[]);
                    let second = pieces.next();
                    if pieces.next().is_some() {
                        let mut value = first.to_vec();
                        value.push(b'/');
                        value.extend(second.unwrap_or(&[]));
                        value
                    } else {
                        name.clone()
                    }
                })
                .collect::<BTreeSet<_>>();
            let size = prefixes
                .iter()
                .map(|p| path_character_len(p) + 1)
                .sum::<usize>();
            if size < 16000 {
                let mut arguments = strings(&["--literal-pathspecs"]);
                arguments.extend(self.pinned());
                arguments.extend(strings(&["ls-files", "-z", "--cached", "--"]));
                for prefix in prefixes {
                    arguments.push(path_bytes(&prefix)?.into_os_string());
                }
                let result = run(
                    &self.project,
                    &arguments,
                    None,
                    deadline.saturating_duration_since(Instant::now()),
                    64 * 1024 * 1024,
                )?;
                if result.code != 0
                    || (!result.stdout.is_empty() && result.stdout.last() != Some(&0))
                {
                    return Err(RuntimeError::unavailable(
                        "Git tracked source query unavailable",
                    ));
                }
                let output = if result.stdout.is_empty() {
                    Vec::new()
                } else {
                    result.stdout[..result.stdout.len() - 1]
                        .split(|b| *b == 0)
                        .map(<[u8]>::to_vec)
                        .collect()
                };
                return Ok(output
                    .into_iter()
                    .filter(|name| wanted.contains(name))
                    .collect());
            }
        }
        let mut tracked = BTreeSet::new();
        let mut batch = Vec::new();
        let mut size = 0;
        for (index, name) in members.iter().enumerate() {
            batch.push(name);
            size += path_character_len(name) + 1;
            if size < 16000 && index + 1 < members.len() {
                continue;
            }
            let mut arguments = strings(&["--literal-pathspecs"]);
            arguments.extend(self.pinned());
            arguments.extend(strings(&["ls-files", "-z", "--cached", "--"]));
            for member in &batch {
                arguments.push(path_bytes(member)?.into_os_string());
            }
            let result = run(
                &self.project,
                &arguments,
                None,
                deadline.saturating_duration_since(Instant::now()),
                3 * size,
            )?;
            if result.code != 0 || (!result.stdout.is_empty() && result.stdout.last() != Some(&0)) {
                return Err(RuntimeError::unavailable(
                    "Git tracked source query unavailable",
                ));
            }
            if !result.stdout.is_empty() {
                for member in result.stdout[..result.stdout.len() - 1].split(|b| *b == 0) {
                    if !wanted.contains(member) {
                        return Err(RuntimeError::unavailable("Unexpected tracked source path"));
                    }
                    tracked.insert(member.to_vec());
                }
            }
            batch.clear();
            size = 0;
        }
        Ok(tracked)
    }
    pub fn ignored(&self, paths: &[String]) -> Result<Vec<String>> {
        self.ignored_inner(paths).map_err(source_git)
    }
    fn ignored_inner(&self, paths: &[String]) -> Result<Vec<String>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        if paths
            .iter()
            .any(|p| p.is_empty() || p.as_bytes().contains(&0))
        {
            return Err(RuntimeError::invalid(
                "Empty path or NUL in Git source path",
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let key = frame_key(&self.project);
        let held = SCOPE.with(|scope| {
            scope
                .borrow()
                .as_ref()
                .and_then(|scope| scope.frames.get(&key).cloned())
        });
        let confirmed = held.as_ref().is_some_and(|(frame, identity)| {
            frame_identity(&self.project, frame).ok().as_ref() == Some(identity)
        });
        let before = if confirmed {
            held.as_ref().unwrap().0.clone()
        } else {
            Self::discover_uncached(&self.project, deadline)?
                .ok_or_else(|| RuntimeError::unavailable("Git source frame disappeared"))?
        };
        if before.top != self.top
            || before.admin != self.admin
            || before.prefix_bytes != self.prefix_bytes
        {
            return Err(RuntimeError::conflict("Git source frame retargeted"));
        }
        let requested = paths
            .iter()
            .map(|p| p.as_bytes().to_vec())
            .collect::<Vec<_>>();
        let mut payload = Vec::new();
        for path in &requested {
            payload.extend(path);
            payload.push(0)
        }
        let limit = payload.len();
        let unindexed = paths.len() > 256;
        let mut arguments = before.pinned();
        arguments.push("check-ignore".into());
        if unindexed {
            arguments.push("--no-index".into());
        }
        arguments.extend(strings(&["--stdin", "-z"]));
        let result = run(
            &self.project,
            &arguments,
            Some(payload),
            deadline.saturating_duration_since(Instant::now()),
            limit,
        )?;
        let named = if unindexed
            && result.code == 0
            && !result.stdout.is_empty()
            && result.stdout.last() == Some(&0)
        {
            result.stdout[..result.stdout.len() - 1]
                .split(|b| *b == 0)
                .filter(|p| !p.is_empty())
                .map(<[u8]>::to_vec)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let tracked = if named.is_empty() {
            BTreeSet::new()
        } else {
            before.tracked(&named, deadline)?
        };
        let scoped = SCOPE.with(|scope| scope.borrow().is_some());
        let identity = if scoped {
            frame_identity(&self.project, &before).ok()
        } else {
            None
        };
        if !(confirmed && identity.as_ref() == held.as_ref().map(|(_, id)| id)) {
            if Self::discover_uncached(&self.project, deadline)?.as_ref() != Some(&before) {
                return Err(RuntimeError::conflict(
                    "Git source frame changed during query",
                ));
            }
            if let Some(identity) = identity {
                SCOPE.with(|scope| {
                    if let Some(scope) = scope.borrow_mut().as_mut() {
                        scope.frames.insert(key, (before.clone(), identity));
                    }
                })
            }
        }
        if result.code == 1 && result.stdout.is_empty() {
            return Ok(Vec::new());
        }
        if result.code != 0 || result.stdout.is_empty() || result.stdout.last() != Some(&0) {
            return Err(RuntimeError::unavailable(
                "Git source policy output incomplete",
            ));
        }
        let mut counts = BTreeMap::<Vec<u8>, usize>::new();
        for p in requested {
            *counts.entry(p).or_default() += 1
        }
        let mut selected = BTreeSet::new();
        for member in result.stdout[..result.stdout.len() - 1].split(|b| *b == 0) {
            let count = counts
                .get_mut(member)
                .ok_or_else(|| RuntimeError::unavailable("Unexpected Git policy path"))?;
            if *count == 0 {
                return Err(RuntimeError::unavailable(
                    "Duplicate Git policy path exceeds request multiplicity",
                ));
            }
            *count -= 1;
            if !tracked.contains(member) {
                selected.insert(member.to_vec());
            }
        }
        Ok(paths
            .iter()
            .filter(|name| selected.contains(name.as_bytes()))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }
    pub fn common_dir(&self) -> Result<PathBuf> {
        self.common_dir_inner().map_err(source_git)
    }
    fn common_dir_inner(&self) -> Result<PathBuf> {
        let result = run(
            &self.project,
            &strings(&["rev-parse", "--git-common-dir"]),
            None,
            Duration::from_secs(5),
            16384,
        )?;
        if result.code != 0 {
            return Err(RuntimeError::unavailable(
                "Git common directory unavailable",
            ));
        }
        let mut bytes = result.stdout.as_slice();
        while let Some(previous) = bytes.strip_suffix(b"\n") {
            bytes = previous;
        }
        let path = path_bytes(bytes)?;
        Ok(crate::fs::canonicalize(if path.is_absolute() {
            path
        } else {
            self.project.join(path)
        })?)
    }
}
fn path_character_len(bytes: &[u8]) -> usize {
    std::str::from_utf8(bytes)
        .map(str::chars)
        .map(Iterator::count)
        .unwrap_or(bytes.len())
}
fn os_bytes(value: &std::ffi::OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        value.as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut out = Vec::new();
        let mut units = value.encode_wide().peekable();
        while let Some(unit) = units.next() {
            let code = if (0xd800..=0xdbff).contains(&unit)
                && units.peek().is_some_and(|n| (0xdc00..=0xdfff).contains(n))
            {
                0x10000 + (((unit as u32 - 0xd800) << 10) | (units.next().unwrap() as u32 - 0xdc00))
            } else {
                unit as u32
            };
            if code < 0x80 {
                out.push(if code == b'\\' as u32 {
                    b'/'
                } else {
                    code as u8
                })
            } else if code < 0x800 {
                out.extend([(0xc0 | (code >> 6)) as u8, (0x80 | (code & 63)) as u8])
            } else if code < 0x10000 {
                out.extend([
                    (0xe0 | (code >> 12)) as u8,
                    (0x80 | ((code >> 6) & 63)) as u8,
                    (0x80 | (code & 63)) as u8,
                ])
            } else {
                out.extend([
                    (0xf0 | (code >> 18)) as u8,
                    (0x80 | ((code >> 12) & 63)) as u8,
                    (0x80 | ((code >> 6) & 63)) as u8,
                    (0x80 | (code & 63)) as u8,
                ])
            }
        }
        out
    }
    #[cfg(not(any(unix, windows)))]
    {
        value.as_encoded_bytes().to_vec()
    }
}
fn path_bytes(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut out = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let lead = bytes[at];
            let (width, mut code) = match lead {
                0..=0x7f => (1, lead as u32),
                0xc2..=0xdf => (2, (lead & 31) as u32),
                0xe0..=0xef => (3, (lead & 15) as u32),
                0xf0..=0xf4 => (4, (lead & 7) as u32),
                _ => return Err(RuntimeError::invalid("Git path is not filesystem UTF-8")),
            };
            if at + width > bytes.len() {
                return Err(RuntimeError::invalid("Truncated Git path encoding"));
            }
            for c in &bytes[at + 1..at + width] {
                if c & 0xc0 != 0x80 {
                    return Err(RuntimeError::invalid("Invalid Git path continuation"));
                }
                code = (code << 6) | (c & 63) as u32;
            }
            if (width == 2 && code < 0x80)
                || (width == 3 && code < 0x800)
                || (width == 4 && !(0x10000..=0x10ffff).contains(&code))
            {
                return Err(RuntimeError::invalid("Noncanonical Git path encoding"));
            }
            if code <= 0xffff {
                out.push(code as u16)
            } else {
                let code = code - 0x10000;
                out.push(0xd800 | (code >> 10) as u16);
                out.push(0xdc00 | (code & 1023) as u16)
            }
            at += width;
        }
        Ok(PathBuf::from(OsString::from_wide(&out)))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(PathBuf::from(std::str::from_utf8(bytes).map_err(|_| {
            RuntimeError::invalid("Git path is not UTF-8")
        })?))
    }
}

#[cfg(test)]
mod plain_frame_tests {
    use super::*;
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn git(directory: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=Plain frame",
                "-c",
                "user.email=plain@invalid.example",
            ])
            .args(arguments)
            .current_dir(directory)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {arguments:?}");
    }
    fn repository() -> (Directory, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("infra-plain-frame-{}", crate::domain::uuid4()));
        fs::create_dir_all(root.join("shop/src")).unwrap();
        let root = crate::fs::canonicalize(&root).unwrap();
        git(&root, &["init", "--quiet"]);
        (Directory(root.clone()), root)
    }
    fn checkout_of(project: &Path) -> (PathBuf, PathBuf) {
        let project = crate::fs::canonicalize(project).unwrap();
        let checkout = checkout(&project).unwrap().unwrap();
        (project, checkout)
    }
    fn asked(project: &Path) -> GitFrame {
        let (project, checkout) = checkout_of(project);
        GitFrame::asked(&project, &checkout, Instant::now() + Duration::from_secs(5)).unwrap()
    }
    fn plain(project: &Path) -> Option<GitFrame> {
        let (project, checkout) = checkout_of(project);
        plain_frame(&project, &checkout)
    }
    #[test]
    fn a_plain_checkout_reads_as_git_reports_it() {
        let (_temp, root) = repository();
        let named = root.join("shop/магазин с пробелами");
        fs::create_dir_all(&named).unwrap();
        for project in [
            root.clone(),
            root.join("shop"),
            root.join("shop/src"),
            named,
        ] {
            assert_eq!(
                plain(&project),
                Some(asked(&project)),
                "{}",
                project.display()
            );
        }
    }
    #[test]
    fn layouts_git_could_read_differently_are_left_to_git() {
        let (_temp, root) = repository();
        let config = root.join(".git/config");
        let original = fs::read_to_string(&config).unwrap();
        assert!(original.contains("bare = false") && plain(&root).is_some());
        for extra in [
            "\tbare = true\n",
            "\tbare\n",
            "\tworktree = /elsewhere\n",
            "[include]\n\tpath = more.config\n",
        ] {
            fs::write(&config, format!("{original}[core]\n{extra}")).unwrap();
            assert_eq!(plain(&root), None, "{extra}");
        }
        fs::write(&config, &original).unwrap();
        git(
            &root,
            &["commit", "--quiet", "--allow-empty", "-m", "start"],
        );
        let linked = root.join("linked");
        git(
            &root,
            &["worktree", "add", "--quiet", "-b", "linked", "linked"],
        );
        assert_eq!(plain(&linked), None);
        assert_eq!(
            asked(&linked).top,
            crate::fs::canonicalize(&linked).unwrap()
        );
    }
}
