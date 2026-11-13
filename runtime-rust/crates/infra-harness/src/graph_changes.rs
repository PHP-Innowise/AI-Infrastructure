//! What a pull request, a branch, a commit or the work in progress changed:
//! the files, and the commit whose code they are.
//!
//! Git answers everything about the checkout. A pull request is asked of the
//! GitHub CLI when it is installed and signed in, which also names its base
//! branch and its title; otherwise its head is fetched under the name the
//! forge publishes it by. Nothing here changes the checkout: a fetch brings
//! objects only, and a commit's files are written out through an index of
//! their own.
use crate::{
    HarnessError, Result,
    graph_map::{Edited, Hunk},
    projects,
};
use serde_json::{Value, json};
use std::{
    ffi::OsStr,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
/// Seconds a question to the local repository may take.
const QUERY: u64 = 20;
const COMPARE: u64 = 60;
/// Seconds a fetch or a listing over the network may take.
const NETWORK: u64 = 120;
/// Seconds writing out the files of a commit may take.
const EXPORT: u64 = 600;
/// Pull requests offered in one list.
const LISTED: usize = 50;
/// Commits of the history read at a time.
pub const PAGE: usize = 50;
/// Branches offered in the list.
const BRANCHES: usize = 400;
/// What the dependency map is asked to show the changes of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The checkout against the branch it left, uncommitted files included.
    Work,
    /// Only what is not committed yet.
    Uncommitted,
    Pull(u64),
    /// A branch, a tag or a commit, measured from where it left its base.
    Named(String),
    /// One commit against its parent, as a history lists it.
    Commit(String),
}
impl Target {
    pub fn parse(text: &str) -> Result<Self> {
        let invalid = || {
            HarnessError::invalid(
                "Choose current work, a pull request number, a branch or a commit.",
            )
        };
        match text {
            "work" => return Ok(Self::Work),
            "uncommitted" => return Ok(Self::Uncommitted),
            _ => {}
        }
        if let Some(number) = text.strip_prefix("pr:") {
            return number
                .parse::<u64>()
                .ok()
                .filter(|found| {
                    *found > 0 && number.len() <= 10 && number.bytes().all(|b| b.is_ascii_digit())
                })
                .map(Self::Pull)
                .ok_or_else(invalid);
        }
        if let Some(hash) = text.strip_prefix("commit:") {
            return (hash.len() >= 7
                && hash.len() <= 64
                && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| Self::Commit(hash.to_ascii_lowercase()))
            .ok_or_else(invalid);
        }
        let name = text.strip_prefix("ref:").ok_or_else(invalid)?;
        if name.is_empty()
            || name.len() > 200
            || name.starts_with('-')
            || name
                .chars()
                .any(|letter| letter.is_control() || letter.is_whitespace())
        {
            return Err(invalid());
        }
        Ok(Self::Named(name.to_owned()))
    }
    pub fn text(&self) -> String {
        match self {
            Self::Work => "work".to_owned(),
            Self::Uncommitted => "uncommitted".to_owned(),
            Self::Pull(number) => format!("pr:{number}"),
            Self::Named(name) => format!("ref:{name}"),
            Self::Commit(hash) => format!("commit:{hash}"),
        }
    }
}
/// One changed file: where it is now, what happened to it (Git's letter) and,
/// after a rename or a copy, where it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub status: char,
    pub from: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    pub name: String,
    pub commit: String,
}
/// A pull request as the lists show it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pull {
    pub number: u64,
    pub title: String,
    pub head: String,
    pub base: String,
    pub draft: bool,
    /// `open`, `merged` or `closed`.
    pub state: String,
    pub author: String,
    /// When it was last updated, in seconds since 1970.
    pub at: Option<i64>,
}
impl Pull {
    pub fn json(&self) -> Value {
        json!({
            "number": self.number,
            "title": self.title,
            "head": self.head,
            "base": self.base,
            "draft": self.draft,
            "state": self.state,
            "author": self.author,
            "at": self.at,
        })
    }
}
#[derive(Clone, Debug)]
pub struct Resolved {
    pub target: Target,
    /// `work`, `uncommitted`, `pull`, `branch` or `commit`.
    pub kind: &'static str,
    pub name: String,
    pub title: Option<String>,
    /// The open pull request the current branch belongs to.
    pub pull: Option<u64>,
    pub head: Option<Side>,
    pub base: Option<Side>,
    /// The commit a graph has to be built from; `None` when the changed files
    /// are the checkout's own and the project's graph is theirs.
    pub revision: Option<String>,
    /// The project folder inside the repository, empty at its top.
    pub prefix: String,
    pub files: Vec<Change>,
    pub notes: Vec<String>,
}
fn short(commit: &str) -> String {
    commit.chars().take(12).collect()
}
impl Resolved {
    pub fn json(&self) -> Value {
        let side = |side: &Option<Side>| {
            side.as_ref()
                .map(|side| json!({"name": side.name, "commit": short(&side.commit)}))
        };
        json!({
            "target": self.target.text(),
            "kind": self.kind,
            "name": self.name,
            "title": self.title,
            "pull": self.pull,
            "head": side(&self.head),
            "base": side(&self.base),
            "notes": self.notes,
        })
    }
}
fn raw(root: &Path, arguments: &[&str], seconds: u64) -> Result<Option<Vec<u8>>> {
    let arguments = arguments
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    let output = projects::git(root, &arguments, seconds)?;
    Ok((output.code == 0).then_some(output.stdout))
}
fn line(root: &Path, arguments: &[&str]) -> Result<Option<String>> {
    Ok(raw(root, arguments, QUERY)?
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
        .filter(|text| !text.is_empty()))
}
fn commit(root: &Path, name: &str) -> Result<Option<String>> {
    line(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{name}^{{commit}}"),
        ],
    )
}
fn existing(root: &Path, names: &[String]) -> Result<Option<String>> {
    for name in names {
        if commit(root, name)?.is_some() {
            return Ok(Some(name.clone()));
        }
    }
    Ok(None)
}
fn checkout(root: &Path) -> Result<String> {
    if line(root, &["rev-parse", "--is-inside-work-tree"])?.as_deref() != Some("true") {
        return Err(HarnessError::invalid(
            "This project folder is not a Git checkout, so it has no changes to map.",
        ));
    }
    Ok(line(root, &["rev-parse", "--show-prefix"])?
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_owned())
}
/// The branch new work is measured against: the remote's default branch, or
/// the first of the usual names that exists.
fn default_base(root: &Path) -> Result<Option<String>> {
    if let Some(name) = line(
        root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )? && commit(root, &name)?.is_some()
    {
        return Ok(Some(name));
    }
    existing(
        root,
        &["origin/main", "origin/master", "main", "master"].map(String::from),
    )
}
/// Reads `git diff --name-status -z`: a status, then one path, or two after a
/// rename or a copy.
fn statuses(raw: &[u8]) -> Vec<Change> {
    let mut parts = raw
        .split(|byte| *byte == 0)
        .map(|part| String::from_utf8_lossy(part).into_owned());
    let mut changes = Vec::new();
    while let Some(status) = parts.next() {
        let Some(kind) = status.chars().next() else {
            continue;
        };
        let Some(first) = parts.next() else {
            break;
        };
        if matches!(kind, 'R' | 'C') {
            let Some(second) = parts.next() else {
                break;
            };
            changes.push(Change {
                path: second,
                status: kind,
                from: Some(first),
            });
        } else {
            changes.push(Change {
                path: first,
                status: kind,
                from: None,
            });
        }
    }
    changes
}
/// Files that differ between a commit and another one, or, without the
/// second, the working tree. Paths are relative to the project folder and
/// changes outside it are left out.
fn diff(root: &Path, from: &str, to: Option<&str>) -> Result<Vec<Change>> {
    let mut arguments = vec!["diff", "--name-status", "-z", "-M", "--relative", from];
    arguments.extend(to);
    arguments.push("--");
    let bytes = raw(root, &arguments, COMPARE)?.ok_or_else(|| {
        HarnessError::unavailable("Git could not compare the two sides of the change.")
    })?;
    Ok(statuses(&bytes))
}
fn listed(root: &Path, arguments: &[&str]) -> Result<Vec<Change>> {
    Ok(raw(root, arguments, COMPARE)?
        .unwrap_or_default()
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| Change {
            path: String::from_utf8_lossy(path).into_owned(),
            status: 'A',
            from: None,
        })
        .collect())
}
fn untracked(root: &Path) -> Result<Vec<Change>> {
    listed(root, &["ls-files", "--others", "--exclude-standard", "-z"])
}
/// The checkout itself. `whole` measures it against the branch it left;
/// otherwise only what is not committed counts.
fn work(root: &Path, prefix: String, pulls: &[Pull], whole: bool) -> Result<Resolved> {
    let head = commit(root, "HEAD")?;
    let branch = line(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let name = branch
        .clone()
        .or_else(|| head.as_deref().map(short))
        .unwrap_or_else(|| "no commit yet".to_owned());
    let mut notes = Vec::new();
    let (mut pull, mut base) = (None, None);
    if whole && head.is_some() {
        // A branch with an open pull request is measured against that
        // request's base: in a stack of branches the default branch is
        // several requests away.
        let mut against = None;
        if let Some(branch) = &branch
            && let Some((number, base)) = stacked(root, pulls, branch)?
        {
            pull = Some(number);
            against = Some(base);
        }
        if against.is_none() {
            against = default_base(root)?;
        }
        match against {
            Some(name) => match line(root, &["merge-base", &name, "HEAD"])? {
                Some(commit) => base = Some(Side { name, commit }),
                None => notes.push(format!(
                    "This branch shares no history with {name}, so only uncommitted files are shown."
                )),
            },
            None => notes.push(
                "No base branch was found, so only uncommitted files are shown.".to_owned(),
            ),
        }
    }
    if base.is_none()
        && let Some(head) = &head
    {
        base = Some(Side {
            name: "the last commit".to_owned(),
            commit: head.clone(),
        });
    }
    let files = match &base {
        Some(base) => {
            let mut files = diff(root, &base.commit, None)?;
            files.extend(untracked(root)?);
            files
        }
        // Before the first commit every file is new.
        None => listed(
            root,
            &[
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ],
        )?,
    };
    Ok(Resolved {
        target: if whole {
            Target::Work
        } else {
            Target::Uncommitted
        },
        kind: if whole { "work" } else { "uncommitted" },
        name: name.clone(),
        title: None,
        pull,
        head: head.map(|commit| Side { name, commit }),
        base,
        revision: None,
        prefix,
        files,
        notes,
    })
}
/// The side a commit is compared with: where it left `base`, or, for a commit
/// that is already on that branch, its parent, since it is then one change
/// and not a branch. The flag says which.
fn fork(root: &Path, head: &str, base: Option<(String, String)>) -> Result<(Option<Side>, bool)> {
    if let Some((name, revision)) = base
        && let Some(commit) = line(root, &["merge-base", &revision, head])?
        && commit != head
    {
        return Ok((Some(Side { name, commit }), false));
    }
    Ok((
        commit(root, &format!("{head}^"))?.map(|commit| Side {
            name: "its parent".to_owned(),
            commit,
        }),
        true,
    ))
}
fn against(root: &Path, head: &str, base: &Option<Side>) -> Result<Vec<Change>> {
    match base {
        Some(base) => diff(root, &base.commit, Some(head)),
        // A first commit has nothing before it but the empty tree.
        None => {
            let nothing = if cfg!(windows) { "NUL" } else { "/dev/null" };
            let empty = line(root, &["hash-object", "-t", "tree", nothing])?.ok_or_else(|| {
                HarnessError::unavailable("Git could not compare the two sides of the change.")
            })?;
            diff(root, &empty, Some(head))
        }
    }
}
/// The branch a pull request of `branch` is stacked on, when the list of
/// open requests has one and that branch is known here.
fn stacked(root: &Path, pulls: &[Pull], branch: &str) -> Result<Option<(u64, String)>> {
    let Some(found) = pulls.iter().find(|pull| pull.head == branch) else {
        return Ok(None);
    };
    Ok(existing(
        root,
        &[format!("origin/{}", found.base), found.base.clone()],
    )?
    .map(|base| (found.number, base)))
}
fn named(root: &Path, prefix: String, name: &str, pulls: &[Pull]) -> Result<Resolved> {
    let head = match commit(root, name)? {
        Some(found) => found,
        None => commit(root, &format!("origin/{name}"))?.ok_or_else(|| {
            HarnessError::missing(format!(
                "This checkout has no branch, tag or commit named {name}."
            ))
        })?,
    };
    // A branch with an open pull request is measured against that request's
    // base, as the current work is.
    let of = stacked(root, pulls, name.strip_prefix("origin/").unwrap_or(name))?;
    let pull = of.as_ref().map(|(number, _)| *number);
    let measured = match of {
        Some((_, base)) => Some(base),
        None => default_base(root)?,
    };
    let (base, single) = fork(root, &head, measured.map(|name| (name.clone(), name)))?;
    Ok(Resolved {
        target: Target::Named(name.to_owned()),
        kind: if single { "commit" } else { "branch" },
        name: name.to_owned(),
        title: None,
        pull,
        files: against(root, &head, &base)?,
        head: Some(Side {
            name: name.to_owned(),
            commit: head.clone(),
        }),
        base,
        revision: Some(head),
        prefix,
        notes: Vec::new(),
    })
}
/// One commit as a history shows it: what it changed against its parent,
/// wherever it stands.
fn single(root: &Path, prefix: String, hash: &str) -> Result<Resolved> {
    let head = commit(root, hash)?
        .ok_or_else(|| HarnessError::missing(format!("This checkout has no commit {hash}.")))?;
    let base = commit(root, &format!("{head}^"))?.map(|commit| Side {
        name: "its parent".to_owned(),
        commit,
    });
    Ok(Resolved {
        target: Target::Commit(hash.to_owned()),
        kind: "commit",
        name: short(&head),
        title: line(root, &["log", "-1", "--format=%s", &head, "--"])?,
        pull: None,
        files: against(root, &head, &base)?,
        head: Some(Side {
            name: short(&head),
            commit: head.clone(),
        }),
        base,
        revision: Some(head),
        prefix,
        notes: Vec::new(),
    })
}
/// Bytes of a file whose lines are told apart; a larger one is not read.
const TEXT_LIMIT: usize = 4 * 1024 * 1024;
/// Reads the `@@ -20,2 +21,3 @@` line of a hunk: a missing count is one.
fn header(line: &str) -> Option<Hunk> {
    let mut parts = line.split(' ');
    parts.next()?;
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let pair = |text: &str| match text.split_once(',') {
        Some((at, count)) => Some((at.parse::<u32>().ok()?, count.parse::<u32>().ok()?)),
        None => Some((text.parse::<u32>().ok()?, 1)),
    };
    let ((_, removed), (at, added)) = (pair(old)?, pair(new)?);
    Some(Hunk {
        at,
        added,
        removed,
        blank: removed > 0,
    })
}
/// Reads `git diff -U0` of one file: where each run of new lines stands in
/// the new file, how many old lines it replaced, and whether those held
/// anything but blanks.
fn hunks(diff: &[u8]) -> Vec<Hunk> {
    let mut found: Vec<Hunk> = Vec::new();
    let mut open = false;
    for line in diff.split(|byte| *byte == b'\n') {
        if line.starts_with(b"@@ ") {
            let read = header(&String::from_utf8_lossy(line));
            open = read.is_some();
            found.extend(read);
        } else if line.starts_with(b"diff --git ") {
            open = false;
        } else if open
            && line.first() == Some(&b'-')
            && !line[1..].iter().all(u8::is_ascii_whitespace)
            && let Some(hunk) = found.last_mut()
        {
            hunk.blank = false;
        }
    }
    found
}
/// What a change did to one of its files: the file as the change leaves it
/// and the lines that differ from the other side. `None` for a file the
/// change does not hold, a deleted one, or one that cannot be read.
pub fn edited(root: &Path, resolved: &Resolved, path: &str) -> Result<Option<Edited>> {
    let Some(change) = resolved.files.iter().find(|change| change.path == path) else {
        return Ok(None);
    };
    match change.status {
        'D' => return Ok(None),
        // A new file, or a copy: everything in it is new.
        'A' | 'C' => {
            return Ok(Some(Edited {
                new: true,
                ..Edited::default()
            }));
        }
        _ => {}
    }
    let Some(base) = &resolved.base else {
        return Ok(None);
    };
    let text = match &resolved.revision {
        Some(head) => raw(root, &["show", &format!("{head}:./{path}")], COMPARE)?,
        None => std::fs::read(root.join(path)).ok(),
    };
    let Some(text) = text.filter(|text| text.len() <= TEXT_LIMIT) else {
        return Ok(None);
    };
    let mut arguments = vec![
        "diff",
        "-U0",
        "-M",
        "--relative",
        "--no-color",
        "--no-ext-diff",
        base.commit.as_str(),
    ];
    arguments.extend(resolved.revision.as_deref());
    arguments.push("--");
    // A renamed file is compared with the one it was.
    arguments.extend(change.from.as_deref());
    arguments.push(path);
    let Some(diff) = raw(root, &arguments, COMPARE)? else {
        return Ok(None);
    };
    Ok(Some(Edited {
        new: false,
        text: String::from_utf8_lossy(&text).into_owned(),
        hunks: hunks(&diff),
    }))
}
fn kept(mut stream: impl Read, limit: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    while let Ok(read) = stream.read(&mut chunk) {
        if read == 0 {
            break;
        }
        // What does not fit is still read, so the program is never left
        // waiting on a full pipe.
        let room = limit.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..read.min(room)]);
    }
    bytes
}
/// Runs a program to its end within a time limit: whether it succeeded, what
/// it printed and what it complained about. `None` when it could not start or
/// did not end in time.
fn run(mut command: Command, seconds: u64) -> Option<(bool, Vec<u8>, String)> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let (output, errors) = (child.stdout.take()?, child.stderr.take()?);
    let output = std::thread::spawn(move || kept(output, 8 * 1024 * 1024));
    let errors = std::thread::spawn(move || kept(errors, 16 * 1024));
    let started = Instant::now();
    let done = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status.success()),
            Ok(None) if started.elapsed() < Duration::from_secs(seconds) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let (output, errors) = (output.join().ok()?, errors.join().ok()?);
    Some((done?, output, String::from_utf8_lossy(&errors).into_owned()))
}
fn said(errors: &str) -> String {
    errors
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("it gave no reason")
        .chars()
        .take(200)
        .collect()
}
/// Asks the GitHub CLI, which never prompts here. The error is its own first
/// line, or that it is not installed.
fn hub(root: &Path, arguments: &[&str]) -> std::result::Result<Value, String> {
    let program = infra_host::provider::find_executable("gh")
        .ok_or_else(|| "the GitHub CLI (gh) is not installed".to_owned())?;
    let mut command = Command::new(program);
    command
        .args(arguments)
        .current_dir(root)
        // The repository is the project's own, whatever a variable names.
        .env_remove("GH_REPO")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0");
    match run(command, NETWORK) {
        Some((true, output, _)) => serde_json::from_slice(&output)
            .map_err(|_| "the GitHub CLI gave an answer that could not be read".to_owned()),
        Some((false, _, errors)) => Err(format!("the GitHub CLI said: {}", said(&errors))),
        None => Err("the GitHub CLI did not answer in time".to_owned()),
    }
}
fn remote(root: &Path) -> Result<Option<String>> {
    let names = line(root, &["remote"])?.unwrap_or_default();
    let mut names = names.lines().map(str::trim).filter(|name| !name.is_empty());
    Ok(names
        .clone()
        .find(|name| *name == "origin")
        .or_else(|| names.next())
        .map(str::to_owned))
}
/// Brings the objects of one remote name without moving any branch here.
fn fetch(root: &Path, remote: &str, name: &str) -> Result<bool> {
    // Git before 2.29 does not know how to leave FETCH_HEAD alone.
    for quiet in [&["--no-write-fetch-head"][..], &[]] {
        let mut arguments = vec!["fetch", "--quiet", "--no-tags"];
        arguments.extend(quiet.iter().copied());
        arguments.extend(["--", remote, name]);
        if raw(root, &arguments, NETWORK)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}
/// The names GitHub, GitLab and Bitbucket publish the head of a request by.
fn published(number: u64) -> [String; 3] {
    [
        format!("refs/pull/{number}/head"),
        format!("refs/merge-requests/{number}/head"),
        format!("refs/pull-requests/{number}/from"),
    ]
}
fn pull(root: &Path, prefix: String, number: u64) -> Result<Resolved> {
    let origin = remote(root)?;
    let mut notes = Vec::new();
    let asked = hub(
        root,
        &[
            "pr",
            "view",
            &number.to_string(),
            "--json",
            "title,headRefName,headRefOid,baseRefName,baseRefOid",
        ],
    );
    let text = |value: &Value, key: &str| value[key].as_str().unwrap_or("").to_owned();
    let (head, title, branch, base) = match &asked {
        Ok(known)
            if known["headRefOid"]
                .as_str()
                .is_some_and(|oid| !oid.is_empty()) =>
        {
            let head = text(known, "headRefOid");
            if commit(root, &head)?.is_none()
                && let Some(origin) = &origin
            {
                fetch(root, origin, &published(number)[0])?;
            }
            // The base as it was when the request was last updated says where
            // the request left it, also after the request was merged.
            let name = text(known, "baseRefName");
            let tip = text(known, "baseRefOid");
            if !tip.is_empty()
                && commit(root, &tip)?.is_none()
                && let Some(origin) = &origin
                && !name.is_empty()
            {
                fetch(root, origin, &format!("refs/heads/{name}"))?;
            }
            let base = if !tip.is_empty() && commit(root, &tip)?.is_some() {
                Some((name, tip))
            } else {
                existing(root, &[format!("origin/{name}"), name.clone()])?
                    .map(|found| (name, found))
            };
            (
                head,
                Some(text(known, "title")).filter(|title| !title.is_empty()),
                Some(text(known, "headRefName")).filter(|name| !name.is_empty()),
                base,
            )
        }
        _ => {
            // Without the CLI the head is looked up under the forges' names.
            let origin = origin.as_ref().ok_or_else(|| {
                HarnessError::missing("This checkout has no remote to ask for a pull request.")
            })?;
            let names = published(number);
            let mut arguments = vec!["ls-remote", "--", origin.as_str()];
            arguments.extend(names.iter().map(String::as_str));
            let answer = raw(root, &arguments, NETWORK)?
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default();
            let found = answer
                .lines()
                .filter_map(|line| line.split_once('\t'))
                .find(|(_, name)| names.iter().any(|known| known == name.trim()));
            let Some((head, name)) = found else {
                return Err(HarnessError::missing(format!(
                    "Pull request #{number} was not found on {origin}{}.",
                    match &asked {
                        Err(reason) => format!(", and {reason}"),
                        Ok(_) => String::new(),
                    }
                )));
            };
            fetch(root, origin, name.trim())?;
            notes.push(
                "Without the GitHub CLI the request is compared with the default branch."
                    .to_owned(),
            );
            (
                head.trim().to_owned(),
                None,
                None,
                default_base(root)?.map(|name| (name.clone(), name)),
            )
        }
    };
    if commit(root, &head)?.is_none() {
        return Err(HarnessError::unavailable(format!(
            "The commits of pull request #{number} could not be fetched."
        )));
    }
    let (base, _) = fork(root, &head, base)?;
    Ok(Resolved {
        target: Target::Pull(number),
        kind: "pull",
        name: format!("#{number}"),
        title,
        pull: Some(number),
        files: against(root, &head, &base)?,
        head: Some(Side {
            name: branch.unwrap_or_else(|| format!("#{number}")),
            commit: head.clone(),
        }),
        base,
        revision: Some(head),
        prefix,
        notes,
    })
}
/// The changes of a target. `pulls` are the open requests as last listed,
/// which say what the current branch is stacked on. Only a pull request needs
/// the network.
pub fn resolve(root: &Path, target: &Target, pulls: &[Pull]) -> Result<Resolved> {
    let prefix = checkout(root)?;
    match target {
        Target::Work => work(root, prefix, pulls, true),
        Target::Uncommitted => work(root, prefix, pulls, false),
        Target::Named(name) => named(root, prefix, name, pulls),
        Target::Commit(hash) => single(root, prefix, hash),
        Target::Pull(number) => pull(root, prefix, *number),
    }
}
/// Pull requests of the project's repository in one state (`open`, `closed`,
/// `merged` or `all`), most recently updated first, narrowed by the forge's
/// own search when `query` is given; and when they cannot be listed, why.
pub fn search(root: &Path, state: &str, query: &str) -> (Vec<Pull>, Option<String>) {
    let limit = LISTED.to_string();
    // The forge counts a merged request among the closed ones; here closed
    // means closed without a merge, as the Merged list is its own.
    let wanted = match state {
        "closed" => format!("--search=is:unmerged {query}"),
        _ => format!("--search={query}"),
    };
    let mut arguments = vec![
        "pr",
        "list",
        "--state",
        state,
        "--limit",
        &limit,
        "--json",
        "number,title,headRefName,baseRefName,isDraft,state,author,updatedAt",
    ];
    if state == "closed" || !query.is_empty() {
        arguments.push(wanted.trim_end());
    }
    match hub(root, &arguments) {
        Ok(rows) => (
            rows.as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| {
                    Some(Pull {
                        number: row["number"].as_u64()?,
                        title: row["title"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(160)
                            .collect(),
                        head: row["headRefName"].as_str()?.to_owned(),
                        base: row["baseRefName"].as_str()?.to_owned(),
                        draft: row["isDraft"] == true,
                        state: row["state"].as_str().unwrap_or("open").to_lowercase(),
                        author: row["author"]["login"].as_str().unwrap_or("").to_owned(),
                        at: row["updatedAt"]
                            .as_str()
                            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                            .map(|at| at.timestamp()),
                    })
                })
                .collect(),
            None,
        ),
        Err(reason) => (Vec::new(), Some(reason)),
    }
}
/// The open pull requests of the project's repository.
pub fn pulls(root: &Path) -> (Vec<Pull>, Option<String>) {
    search(root, "open", "")
}
/// The names a commit carries in a history, each with what it is: the
/// checked-out branch, another branch, a remote branch or a tag.
fn carried(names: &str, remotes: &[&str]) -> Vec<Value> {
    names
        .split(", ")
        .filter_map(|name| {
            let named = |name: &str, kind: &str| Some(json!({"name": name, "kind": kind}));
            if name.is_empty() || name.ends_with("/HEAD") {
                // A remote's own HEAD repeats its default branch.
                return None;
            }
            if let Some(branch) = name.strip_prefix("HEAD -> ") {
                return named(branch, "head");
            }
            if let Some(tag) = name.strip_prefix("tag: ") {
                return named(tag, "tag");
            }
            if name == "HEAD" {
                return named(name, "head");
            }
            let remote = name
                .split_once('/')
                .is_some_and(|(first, _)| remotes.contains(&first));
            named(name, if remote { "remote" } else { "branch" })
        })
        .collect()
}
const LOGGED: &str = "--format=%H%x1f%h%x1f%s%x1f%an%x1f%at%x1f%D%x1f%P";
/// Reads commits printed with `LOGGED`, one after each NUL.
fn logged(bytes: &[u8], remotes: &[&str]) -> Vec<Value> {
    bytes
        .split(|byte| *byte == 0)
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let mut fields = text.trim_start_matches('\n').split('\x1f');
            let commit = fields.next().filter(|commit| !commit.is_empty())?;
            let (short, subject, author, at, names, parents) = (
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
            );
            Some(json!({
                "commit": commit,
                "short": short,
                "subject": subject,
                "author": author,
                "at": at.trim().parse::<i64>().ok(),
                "refs": carried(names, remotes),
                "merge": parents.split_whitespace().count() > 1,
            }))
        })
        .collect()
}
/// A page of the history for the picker, newest first: of the checked-out
/// branch (`""`), of every branch (`"*"`) or of one named branch. `query`
/// keeps the commits whose message contains it, whatever the case; a query
/// that begins a commit's hash puts that commit first. The flag says whether
/// more follow.
pub fn log(root: &Path, scope: &str, query: &str, skip: usize) -> Result<(Vec<Value>, bool)> {
    checkout(root)?;
    let count = format!("--max-count={}", PAGE + 1);
    let skipped = format!("--skip={skip}");
    let wanted = format!("--grep={query}");
    let mut arguments = vec!["log", "--no-color", "-z", LOGGED, &count, &skipped];
    if !query.is_empty() {
        arguments.extend(["--fixed-strings", "--regexp-ignore-case", &wanted]);
    }
    let named;
    match scope {
        "" => arguments.push("HEAD"),
        // Not `--all`: that would list stashes among the commits.
        "*" => arguments.extend(["--branches", "--remotes", "HEAD"]),
        name => {
            named = match commit(root, name)? {
                Some(found) => found,
                None => commit(root, &format!("origin/{name}"))?.ok_or_else(|| {
                    HarnessError::missing(format!("This checkout has no branch named {name}."))
                })?,
            };
            arguments.push(&named);
        }
    }
    arguments.push("--");
    let remotes = line(root, &["remote"])?.unwrap_or_default();
    let remotes = remotes
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    // A repository without a commit has no history to list.
    let mut commits = raw(root, &arguments, COMPARE)?
        .map(|bytes| logged(&bytes, &remotes))
        .unwrap_or_default();
    let more = commits.len() > PAGE;
    commits.truncate(PAGE);
    if skip == 0
        && (4..=64).contains(&query.len())
        && query.bytes().all(|byte| byte.is_ascii_hexdigit())
        && let Some(found) = commit(root, query)?
        && let Some(bytes) = raw(
            root,
            &["log", "--no-color", "-z", "-1", LOGGED, &found, "--"],
            QUERY,
        )?
    {
        commits.retain(|row| row["commit"].as_str() != Some(found.as_str()));
        commits.splice(0..0, logged(&bytes, &remotes));
    }
    Ok((commits, more))
}
/// The branches for the picker: local ones, then the remote ones that no
/// local branch of the same name stands at, each most recent first. `pulls`
/// marks the branches that have an open request.
pub fn branches(root: &Path, pulls: &[Pull]) -> Result<Vec<Value>> {
    checkout(root)?;
    let count = format!("--count={BRANCHES}");
    let bytes = raw(
        root,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            &count,
            "--format=%(refname)%1f%(objectname)%1f%(committerdate:unix)%1f%(contents:subject)%1f%(HEAD)%00",
            "refs/heads",
            "refs/remotes",
        ],
        COMPARE,
    )?
    .unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let rows = text
        .split('\0')
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').split('\x1f');
            Some((
                fields.next().filter(|name| !name.is_empty())?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()? == "*",
            ))
        })
        .collect::<Vec<_>>();
    let local = rows
        .iter()
        .filter_map(|(name, commit, ..)| Some((name.strip_prefix("refs/heads/")?, *commit)))
        .collect::<std::collections::HashSet<_>>();
    let mut listed = Vec::new();
    for remote in [false, true] {
        for (name, commit, at, subject, current) in &rows {
            let branch = match name.strip_prefix(if remote {
                "refs/remotes/"
            } else {
                "refs/heads/"
            }) {
                Some(branch) if !branch.ends_with("/HEAD") => branch,
                _ => continue,
            };
            // Without its remote's name, as a request names its head.
            let plain = match (remote, branch.split_once('/')) {
                (true, Some((_, rest))) => rest,
                _ => branch,
            };
            if remote && local.contains(&(plain, *commit)) {
                continue;
            }
            listed.push(json!({
                "name": branch,
                "remote": remote,
                "commit": commit,
                "at": at.trim().parse::<i64>().ok(),
                "subject": subject,
                "current": current,
                "pull": pulls.iter().find(|pull| pull.head == plain).map(|pull| pull.number),
            }));
        }
    }
    Ok(listed)
}
/// What the picker shows before anything is chosen: whether the folder is a
/// checkout, its branch and the branch new work is measured against.
pub fn place(root: &Path) -> Result<Value> {
    if checkout(root).is_err() {
        return Ok(json!({"git": false}));
    }
    Ok(json!({
        "git": true,
        "branch": line(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?,
        "base": default_base(root)?,
    }))
}
/// Writes the files of a commit into `into` and leaves the checkout alone:
/// the commit is read into an index of its own and checked out from there.
/// Large-file pointers stay pointers, so nothing is downloaded.
pub fn export(root: &Path, commit: &str, into: &Path) -> Result<()> {
    std::fs::create_dir_all(into)?;
    let index = into.with_extension("index");
    let plumbing = |arguments: &[&OsStr]| {
        let mut command = Command::new("git");
        command
            .args(["--no-pager", "-c"])
            .arg(format!(
                "core.hooksPath={}",
                if cfg!(windows) { "NUL" } else { "/dev/null" }
            ))
            .args(["-c", "core.fsmonitor=false", "-C"])
            .arg(root)
            .args(arguments);
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(key);
            }
        }
        command
            .env("GIT_INDEX_FILE", &index)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_LFS_SKIP_SMUDGE", "1");
        run(command, EXPORT).is_some_and(|(done, ..)| done)
    };
    let mut prefix = into.to_string_lossy().replace('\\', "/");
    prefix.push('/');
    let done = plumbing(&[OsStr::new("read-tree"), OsStr::new(commit)])
        && plumbing(&[
            OsStr::new("checkout-index"),
            OsStr::new("--all"),
            OsStr::new("--force"),
            OsStr::new(&format!("--prefix={prefix}")),
        ]);
    let _ = std::fs::remove_file(&index);
    if done {
        Ok(())
    } else {
        Err(HarnessError::unavailable(
            "Git could not write out the files of that commit.",
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct Repository {
        _root: tempfile::TempDir,
        folder: PathBuf,
    }
    impl Repository {
        /// `main` with two files, and `origin/main` standing where it began.
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let folder = root.path().join("shop");
            std::fs::create_dir_all(folder.join("src")).unwrap();
            let repository = Self {
                _root: root,
                folder,
            };
            repository.git(&["init", "--quiet", "--initial-branch=main"]);
            repository.write("src/cart.php", "<?php // cart\n");
            repository.write("src/price.php", "<?php // price\n");
            repository.commit("first");
            repository.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
            repository
        }
        fn git(&self, arguments: &[&str]) -> String {
            let done = Command::new("git")
                .args([
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
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
            String::from_utf8_lossy(&done.stdout).trim().to_owned()
        }
        fn write(&self, path: &str, text: &str) {
            let path = self.folder.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        fn commit(&self, message: &str) -> String {
            self.git(&["add", "--all"]);
            self.git(&["commit", "--quiet", "--message", message]);
            self.git(&["rev-parse", "HEAD"])
        }
    }
    fn paths(resolved: &Resolved) -> Vec<(char, &str)> {
        let mut paths = resolved
            .files
            .iter()
            .map(|change| (change.status, change.path.as_str()))
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }
    #[test]
    fn a_target_is_read_from_its_text_and_written_back() {
        for (text, target) in [
            ("work", Target::Work),
            ("uncommitted", Target::Uncommitted),
            ("pr:12", Target::Pull(12)),
            ("ref:feature/cart", Target::Named("feature/cart".into())),
            ("ref:HEAD~2", Target::Named("HEAD~2".into())),
            ("commit:1a2b3c4", Target::Commit("1a2b3c4".into())),
        ] {
            assert_eq!(Target::parse(text).unwrap(), target);
            assert_eq!(target.text(), text);
        }
        for text in [
            "",
            "pr:",
            "pr:0",
            "pr:+5",
            "pr:1x",
            "pr:99999999999",
            "ref:",
            "ref:-x",
            "ref:a b",
            "ref:a\nb",
            "branch",
            "commit:",
            "commit:12345",
            "commit:xyz1234",
            "commit:HEAD~1",
        ] {
            assert!(Target::parse(text).is_err(), "{text:?}");
        }
    }
    #[test]
    fn a_diff_listing_names_each_file_once_with_what_happened_to_it() {
        assert_eq!(
            statuses(
                b"M\0src/a.php\0R087\0src/old.php\0src/new.php\0D\0gone.php\0A\0new file.php\0"
            ),
            [
                ('M', "src/a.php", None),
                ('R', "src/new.php", Some("src/old.php")),
                ('D', "gone.php", None),
                ('A', "new file.php", None),
            ]
            .map(|(status, path, from)| Change {
                path: path.to_owned(),
                status,
                from: from.map(String::from),
            })
        );
        assert!(statuses(b"").is_empty());
        // A listing cut in the middle loses its last entry, not its mind.
        assert_eq!(statuses(b"M\0a\0R100\0b").len(), 1);
    }
    #[test]
    fn current_work_is_the_branch_since_its_base_and_what_is_not_committed() {
        let repository = Repository::new();
        repository.git(&["switch", "--quiet", "--create", "feature"]);
        repository.write("src/cart.php", "<?php // cart, changed\n");
        repository.git(&["mv", "src/price.php", "src/money.php"]);
        repository.write("src/order.php", "<?php // order\n");
        let committed = repository.commit("second");
        repository.write("src/order.php", "<?php // order, edited\n");
        repository.write("notes.txt", "not tracked\n");
        let work = resolve(&repository.folder, &Target::Work, &[]).unwrap();
        assert_eq!(
            paths(&work),
            [
                ('A', "notes.txt"),
                ('A', "src/order.php"),
                ('M', "src/cart.php"),
                ('R', "src/money.php"),
            ]
        );
        assert_eq!((work.kind, work.name.as_str()), ("work", "feature"));
        assert_eq!(work.base.as_ref().unwrap().name, "origin/main");
        assert_eq!(work.head.as_ref().unwrap().commit, committed);
        assert_eq!(work.revision, None);
        // Only what is not committed: the edit and the untracked file.
        let uncommitted = resolve(&repository.folder, &Target::Uncommitted, &[]).unwrap();
        assert_eq!(
            paths(&uncommitted),
            [('A', "notes.txt"), ('M', "src/order.php")]
        );
        assert_eq!(uncommitted.base.unwrap().commit, committed);
    }
    #[test]
    fn a_stacked_branch_is_measured_against_the_base_of_its_pull_request() {
        let repository = Repository::new();
        repository.git(&["switch", "--quiet", "--create", "lower"]);
        repository.write("src/lower.php", "<?php\n");
        repository.commit("lower");
        repository.git(&["switch", "--quiet", "--create", "upper"]);
        repository.write("src/upper.php", "<?php\n");
        repository.commit("upper");
        let alone = resolve(&repository.folder, &Target::Work, &[]).unwrap();
        assert_eq!(
            paths(&alone),
            [('A', "src/lower.php"), ('A', "src/upper.php")]
        );
        let stacked = resolve(
            &repository.folder,
            &Target::Work,
            &[Pull {
                number: 7,
                title: "Upper".into(),
                head: "upper".into(),
                base: "lower".into(),
                ..Pull::default()
            }],
        )
        .unwrap();
        assert_eq!(paths(&stacked), [('A', "src/upper.php")]);
        assert_eq!(stacked.pull, Some(7));
        assert_eq!(stacked.base.unwrap().name, "lower");
        // The same holds for the branch when it is named, not checked out.
        repository.git(&["switch", "--quiet", "main"]);
        let pulls = [Pull {
            number: 7,
            title: "Upper".into(),
            head: "upper".into(),
            base: "lower".into(),
            ..Pull::default()
        }];
        let named = resolve(&repository.folder, &Target::Named("upper".into()), &pulls).unwrap();
        assert_eq!(paths(&named), [('A', "src/upper.php")]);
        assert_eq!((named.kind, named.pull), ("branch", Some(7)));
        let alone = resolve(&repository.folder, &Target::Named("upper".into()), &[]).unwrap();
        assert_eq!(alone.files.len(), 2);
        assert_eq!(alone.pull, None);
    }
    #[test]
    fn a_branch_is_compared_with_where_it_left_and_a_commit_with_its_parent() {
        let repository = Repository::new();
        let first = repository.git(&["rev-parse", "HEAD"]);
        repository.write("src/cart.php", "<?php // on main\n");
        let second = repository.commit("on main");
        repository.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repository.git(&["switch", "--quiet", "--create", "feature", &first]);
        repository.write("src/order.php", "<?php\n");
        let feature = repository.commit("feature");
        repository.git(&["switch", "--quiet", "main"]);
        let branch = resolve(&repository.folder, &Target::Named("feature".into()), &[]).unwrap();
        assert_eq!(paths(&branch), [('A', "src/order.php")]);
        assert_eq!(branch.kind, "branch");
        assert_eq!(branch.revision.as_deref(), Some(feature.as_str()));
        assert_eq!(branch.base.unwrap().commit, first);
        // A commit that is already on the base branch is one change.
        let single = resolve(&repository.folder, &Target::Named(second.clone()), &[]).unwrap();
        assert_eq!(paths(&single), [('M', "src/cart.php")]);
        assert_eq!(single.kind, "commit");
        assert_eq!(single.base.unwrap().commit, first);
        // The first commit is compared with nothing.
        let root = resolve(&repository.folder, &Target::Named(first), &[]).unwrap();
        assert_eq!(
            paths(&root),
            [('A', "src/cart.php"), ('A', "src/price.php")]
        );
        assert!(resolve(&repository.folder, &Target::Named("nowhere".into()), &[]).is_err());
    }
    /// A request whose commits are not here yet is fetched under the name the
    /// forge publishes it by. The forge is a bare repository beside the
    /// checkout, so nothing leaves the machine.
    #[test]
    fn a_pull_request_that_is_not_here_is_fetched_and_no_branch_moves() {
        let repository = Repository::new();
        let forge = repository.folder.with_file_name("forge.git");
        let other = repository.folder.with_file_name("other");
        let run = |folder: &Path, arguments: &[&str]| {
            let done = Command::new("git")
                .args([
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
                .args(arguments)
                .current_dir(folder)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@example.test")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@example.test")
                .output()
                .unwrap();
            assert!(done.status.success(), "{arguments:?}: {done:?}");
            String::from_utf8_lossy(&done.stdout).trim().to_owned()
        };
        let parent = repository.folder.parent().unwrap();
        run(parent, &["clone", "--quiet", "--bare", "shop", "forge.git"]);
        run(parent, &["clone", "--quiet", "forge.git", "other"]);
        // Someone else's work reaches the forge as two requests, one under
        // GitHub's name and one under GitLab's.
        std::fs::write(other.join("src/coupon.php"), "<?php // coupon\n").unwrap();
        run(&other, &["add", "--all"]);
        run(&other, &["commit", "--quiet", "--message", "coupon"]);
        let first = run(&other, &["rev-parse", "HEAD"]);
        run(
            &other,
            &["push", "--quiet", "origin", "HEAD:refs/pull/7/head"],
        );
        std::fs::write(other.join("src/gift.php"), "<?php // gift\n").unwrap();
        run(&other, &["add", "--all"]);
        run(&other, &["commit", "--quiet", "--message", "gift"]);
        let second = run(&other, &["rev-parse", "HEAD"]);
        run(
            &other,
            &[
                "push",
                "--quiet",
                "origin",
                "HEAD:refs/merge-requests/8/head",
            ],
        );
        repository.git(&["remote", "add", "origin", forge.to_str().unwrap()]);
        let missing = |commit: &str| {
            !Command::new("git")
                .args(["cat-file", "-e", commit])
                .current_dir(&repository.folder)
                .output()
                .unwrap()
                .status
                .success()
        };
        assert!(missing(&first) && missing(&second));
        let before = repository.git(&["for-each-ref"]);
        let pull = resolve(&repository.folder, &Target::Pull(7), &[]).unwrap();
        assert_eq!(pull.revision.as_deref(), Some(first.as_str()));
        assert_eq!(paths(&pull), [('A', "src/coupon.php")]);
        assert_eq!(
            (pull.kind, pull.name.as_str(), pull.pull),
            ("pull", "#7", Some(7))
        );
        assert_eq!(pull.base.as_ref().unwrap().name, "origin/main");
        assert!(!missing(&first));
        let merge = resolve(&repository.folder, &Target::Pull(8), &[]).unwrap();
        assert_eq!(merge.revision.as_deref(), Some(second.as_str()));
        assert_eq!(
            paths(&merge),
            [('A', "src/coupon.php"), ('A', "src/gift.php")]
        );
        // Objects came; no branch, no remote branch and nothing checked out moved.
        assert_eq!(repository.git(&["for-each-ref"]), before);
        assert_eq!(repository.git(&["status", "--porcelain=v1"]), "");
        // The files of the fetched commit can be written out.
        let into = parent.join("out").join("tree");
        export(&repository.folder, &first, &into).unwrap();
        assert!(into.join("src/coupon.php").is_file());
        // A number the forge does not have is said to be missing.
        let none = resolve(&repository.folder, &Target::Pull(99), &[]).unwrap_err();
        assert!(
            none.to_string().contains("#99 was not found on origin"),
            "{none}"
        );
    }
    #[test]
    fn a_commit_picked_from_the_history_is_compared_with_its_parent() {
        let repository = Repository::new();
        repository.git(&["switch", "--quiet", "--create", "feature"]);
        repository.write("src/order.php", "<?php\n");
        let one = repository.commit("Add the order");
        repository.write("src/cart.php", "<?php // changed\n");
        let two = repository.commit("Change the cart");
        // As a branch it is everything since it left main; as a commit of
        // the history, its own change.
        let branch = resolve(&repository.folder, &Target::Named(two.clone()), &[]).unwrap();
        assert_eq!(branch.files.len(), 2);
        let picked = resolve(&repository.folder, &Target::Commit(two.clone()), &[]).unwrap();
        assert_eq!(paths(&picked), [('M', "src/cart.php")]);
        assert_eq!(
            (picked.kind, picked.title.as_deref()),
            ("commit", Some("Change the cart"))
        );
        assert_eq!(picked.name, two[..12]);
        assert_eq!(picked.base.as_ref().unwrap().commit, one);
        assert_eq!(picked.revision.as_deref(), Some(two.as_str()));
        // The start of a hash is enough, and one that names nothing is refused.
        let short = resolve(
            &repository.folder,
            &Target::Commit(two[..8].to_owned()),
            &[],
        )
        .unwrap();
        assert_eq!(short.revision.as_deref(), Some(two.as_str()));
        assert!(
            resolve(
                &repository.folder,
                &Target::Commit("0123456789".into()),
                &[]
            )
            .is_err()
        );
    }
    fn told(rows: &[Value], key: &str) -> Vec<String> {
        rows.iter()
            .map(|row| row[key].as_str().unwrap_or("").to_owned())
            .collect()
    }
    #[test]
    fn the_history_is_listed_newest_first_with_the_names_a_commit_carries() {
        let repository = Repository::new();
        repository.git(&["remote", "add", "origin", "/nowhere"]);
        let first = repository.git(&["rev-parse", "HEAD"]);
        repository.git(&["tag", "v1"]);
        repository.write("src/cart.php", "<?php // two\n");
        let second = repository.commit("Round totals once");
        repository.git(&["switch", "--quiet", "--create", "feature"]);
        repository.write("src/order.php", "<?php\n");
        let third = repository.commit("Add the ORDER page");
        let (page, more) = log(&repository.folder, "", "", 0).unwrap();
        assert!(!more);
        assert_eq!(
            told(&page, "commit"),
            [third.clone(), second.clone(), first.clone()]
        );
        assert_eq!(
            told(&page, "subject"),
            ["Add the ORDER page", "Round totals once", "first"]
        );
        assert_eq!(
            page[0]["short"],
            third[..page[0]["short"].as_str().unwrap().len()]
        );
        assert_eq!(
            (&page[0]["author"], &page[0]["merge"]),
            (&json!("t"), &json!(false))
        );
        assert!(page[0]["at"].as_i64().unwrap() > 1_600_000_000);
        let names = |row: &Value| {
            let mut names = row["refs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| {
                    format!(
                        "{} {}",
                        name["kind"].as_str().unwrap(),
                        name["name"].as_str().unwrap()
                    )
                })
                .collect::<Vec<_>>();
            names.sort();
            names
        };
        assert_eq!(names(&page[0]), ["head feature"]);
        assert_eq!(names(&page[1]), ["branch main"]);
        assert_eq!(names(&page[2]), ["remote origin/main", "tag v1"]);
        // A message is searched whatever its case.
        let (found, _) = log(&repository.folder, "", "order", 0).unwrap();
        assert_eq!(told(&found, "commit"), std::slice::from_ref(&third));
        // Another branch's history, and every branch's.
        let (main, _) = log(&repository.folder, "main", "", 0).unwrap();
        assert_eq!(told(&main, "commit"), [second.clone(), first.clone()]);
        let (all, _) = log(&repository.folder, "*", "", 0).unwrap();
        assert_eq!(all.len(), 3);
        // The start of a hash finds its commit, also outside the listed branch.
        let (hashed, _) = log(&repository.folder, "main", &third[..8], 0).unwrap();
        assert_eq!(told(&hashed, "commit"), [third]);
        assert!(log(&repository.folder, "nowhere", "", 0).is_err());
        // A long history comes a page at a time.
        for number in 0..PAGE {
            repository.git(&[
                "commit",
                "--quiet",
                "--allow-empty",
                "--message",
                &format!("step {number}"),
            ]);
        }
        let (page, more) = log(&repository.folder, "", "", 0).unwrap();
        assert_eq!((page.len(), more), (PAGE, true));
        assert_eq!(page[0]["subject"], format!("step {}", PAGE - 1));
        let (rest, more) = log(&repository.folder, "", "", PAGE).unwrap();
        assert_eq!((rest.len(), more), (3, false));
        assert_eq!(rest[2]["commit"], first);
    }
    #[test]
    fn branches_are_listed_local_first_and_a_remote_twin_is_left_out() {
        let repository = Repository::new();
        repository.git(&["remote", "add", "origin", "/nowhere"]);
        repository.git(&[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ]);
        repository.git(&["switch", "--quiet", "--create", "feature"]);
        repository.write("src/order.php", "<?php\n");
        let second = repository.commit("Feature work");
        repository.git(&["update-ref", "refs/remotes/origin/review", "HEAD"]);
        let listed = branches(
            &repository.folder,
            &[Pull {
                number: 7,
                head: "review".into(),
                ..Pull::default()
            }],
        )
        .unwrap();
        // `origin/main` stands where `main` does and says nothing new.
        assert_eq!(told(&listed, "name"), ["feature", "main", "origin/review"]);
        assert_eq!(
            listed
                .iter()
                .map(|row| (
                    row["remote"] == true,
                    row["current"] == true,
                    row["pull"].as_u64()
                ))
                .collect::<Vec<_>>(),
            [
                (false, true, None),
                (false, false, None),
                (true, false, Some(7))
            ]
        );
        assert_eq!(listed[0]["commit"], second);
        assert_eq!(listed[0]["subject"], "Feature work");
        assert!(listed[0]["at"].as_i64().unwrap() > 1_600_000_000);
    }
    #[test]
    fn the_lines_that_differ_are_read_from_a_diff_without_context() {
        let diff = b"diff --git a/Cart.php b/Cart.php\nindex d7c26db..a35808f 100644\n--- a/Cart.php\n+++ b/Cart.php\n@@ -4,0 +5 @@ use Shop\\Notifier;\n+use Shop\\Price;\n@@ -20 +21 @@ class Cart\n-            $sum += $item->cents();\n+            $sum += $item->cents() * 2;\n@@ -25,3 +25,0 @@ class Cart\n-\n-   \n-\t\n@@ -40,2 +37,0 @@\n-\n-    echo 1;\n\\ No newline at end of file\n";
        let at = |at, added, removed, blank| Hunk {
            at,
            added,
            removed,
            blank,
        };
        // A missing count is one. Removed lines that held nothing but
        // blanks are told apart from removed code.
        assert_eq!(
            hunks(diff),
            [
                at(5, 1, 0, false),
                at(21, 1, 1, false),
                at(25, 0, 3, true),
                at(37, 0, 2, false),
            ]
        );
        assert!(hunks(b"Binary files a/logo.png and b/logo.png differ\n").is_empty());
        assert!(hunks(b"@@ broken @@\n-x\n").is_empty());
    }
    #[test]
    fn what_a_change_did_to_a_file_is_its_new_text_and_the_lines_that_differ() {
        let repository = Repository::new();
        let steps = (1..=8)
            .map(|step| format!("    step{step}();\n"))
            .collect::<String>();
        repository.write(
            "src/order.php",
            &format!("<?php\nfunction place() {{\n{steps}}}\n"),
        );
        repository.commit("order");
        repository.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repository.git(&["switch", "--quiet", "--create", "feature"]);
        // The cart's one line becomes four, the order moves and loses a
        // line, a file is new and one is gone.
        let cart = "<?php // the cart\nfunction total() {\n    return 1;\n}\n";
        repository.write("src/cart.php", cart);
        repository.git(&["mv", "src/order.php", "src/checkout.php"]);
        repository.write(
            "src/checkout.php",
            &format!(
                "<?php\nfunction place() {{\n{}}}\n",
                steps.replace("    step4();\n", "")
            ),
        );
        repository.write("src/new.php", "<?php // new\n");
        repository.git(&["rm", "--quiet", "src/price.php"]);
        repository.commit("second");
        repository.git(&["switch", "--quiet", "main"]);
        let folder = &repository.folder;
        let branch = resolve(folder, &Target::Named("feature".to_owned()), &[]).unwrap();
        // A commit that is not checked out is read from the repository.
        let read = edited(folder, &branch, "src/cart.php").unwrap().unwrap();
        assert_eq!((read.new, read.text.as_str()), (false, cart));
        assert_eq!(
            read.hunks,
            [Hunk {
                at: 1,
                added: 4,
                removed: 1,
                blank: false
            }]
        );
        // A renamed file is compared with the one it was: the fourth step
        // went after line five.
        let moved = edited(folder, &branch, "src/checkout.php")
            .unwrap()
            .unwrap();
        assert_eq!(
            moved.hunks,
            [Hunk {
                at: 5,
                added: 0,
                removed: 1,
                blank: false
            }]
        );
        assert!(moved.text.contains("step5") && !moved.text.contains("step4"));
        assert!(edited(folder, &branch, "src/new.php").unwrap().unwrap().new);
        // A deleted file and one the change does not hold have nothing.
        assert!(edited(folder, &branch, "src/price.php").unwrap().is_none());
        assert!(
            edited(folder, &branch, "src/elsewhere.php")
                .unwrap()
                .is_none()
        );
        // The work in the checkout is read from the files themselves.
        repository.write("src/cart.php", "<?php // cart\n// a note\n");
        repository.write("src/draft.php", "<?php // draft\n");
        let work = resolve(folder, &Target::Uncommitted, &[]).unwrap();
        let read = edited(folder, &work, "src/cart.php").unwrap().unwrap();
        assert_eq!(read.text, "<?php // cart\n// a note\n");
        assert_eq!(
            read.hunks,
            [Hunk {
                at: 2,
                added: 1,
                removed: 0,
                blank: false
            }]
        );
        assert!(edited(folder, &work, "src/draft.php").unwrap().unwrap().new);
    }
    #[test]
    fn a_project_below_the_repository_sees_its_own_files_only() {
        let repository = Repository::new();
        repository.write("docs/guide.md", "guide\n");
        repository.write("src/cart.php", "<?php // changed\n");
        let below = repository.folder.join("src");
        let work = resolve(&below, &Target::Uncommitted, &[]).unwrap();
        assert_eq!(work.prefix, "src");
        assert_eq!(paths(&work), [('M', "cart.php")]);
        let outside = tempfile::tempdir().unwrap();
        assert!(resolve(outside.path(), &Target::Work, &[]).is_err());
        assert_eq!(place(outside.path()).unwrap(), json!({"git": false}));
        assert_eq!(
            place(&repository.folder).unwrap(),
            json!({"git": true, "branch": "main", "base": "origin/main"})
        );
    }
    #[test]
    fn the_files_of_a_commit_are_written_out_without_touching_the_checkout() {
        let repository = Repository::new();
        let first = repository.git(&["rev-parse", "HEAD"]);
        repository.write("src/cart.php", "<?php // newer\n");
        repository.write("src/extra.php", "<?php\n");
        repository.commit("second");
        repository.write("src/cart.php", "<?php // not committed\n");
        repository.git(&["add", "src/cart.php"]);
        let before = repository.git(&["status", "--porcelain=v1"]);
        let into = repository.folder.parent().unwrap().join("out").join("tree");
        export(&repository.folder, &first, &into).unwrap();
        // Git writes the line ends its settings ask for, and Git for Windows
        // asks for CRLF in every repository.
        assert_eq!(
            std::fs::read_to_string(into.join("src/cart.php"))
                .unwrap()
                .replace("\r\n", "\n"),
            "<?php // cart\n"
        );
        assert!(into.join("src/price.php").is_file());
        assert!(!into.join("src/extra.php").exists());
        assert!(!into.with_extension("index").exists());
        // The staged edit and the working file are as they were.
        assert_eq!(repository.git(&["status", "--porcelain=v1"]), before);
        assert_eq!(
            std::fs::read_to_string(repository.folder.join("src/cart.php")).unwrap(),
            "<?php // not committed\n"
        );
        assert!(export(&repository.folder, "0000000", &into.with_file_name("none")).is_err());
    }
}
