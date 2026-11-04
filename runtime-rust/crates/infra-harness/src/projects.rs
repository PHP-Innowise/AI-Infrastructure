use crate::{
    HarnessError, Result,
    services::{Domain, Payload, Reply},
    store::{self, SessionStore},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, VecDeque},
    ffi::OsString,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub fn git(root: &Path, args: &[String], seconds: u64) -> Result<infra_core::git::GitOutput> {
    let args = args.iter().map(OsString::from).collect::<Vec<_>>();
    Ok(infra_core::git::run(
        root,
        &args,
        None,
        Duration::from_secs(seconds),
        16 * 1024 * 1024,
    )?)
}
fn run(root: &Path, args: &[&str]) -> Result<infra_core::git::GitOutput> {
    git(
        root,
        &args.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
        30,
    )
}
fn output(result: infra_core::git::GitOutput) -> Result<String> {
    if result.code != 0 {
        return Err(HarnessError::unavailable(
            "Git could not read the workspace.",
        ));
    }
    String::from_utf8(result.stdout)
        .map_err(|_| HarnessError::invalid("Git returned invalid path text."))
}
fn stripped(value: String) -> String {
    infra_core::domain::rstrip(&value).to_string()
}
fn norm(path: &Path) -> String {
    let s = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        s.replace('/', "\\").to_lowercase()
    } else {
        s
    }
}
pub fn details(root: &Path, status: bool) -> Result<Value> {
    let root = infra_core::fs::canonicalize(root)?;
    let top = run(&root, &["rev-parse", "--show-toplevel"])?;
    if top.code != 0 {
        return Ok(
            json!({"is_git":false,"branch":null,"head":null,"dirty":if status{json!(false)}else{Value::Null},"worktree_available":false,"reason":"Select a Git working directory to use a worktree."}),
        );
    }
    let top = PathBuf::from(stripped(output(top)?));
    let common = stripped(output(run(&root, &["rev-parse", "--git-common-dir"])?)?);
    let common = infra_core::fs::canonicalize(root.join(common))?;
    let branch = run(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    if ![0, 1].contains(&branch.code) {
        return Err(HarnessError::unavailable(
            "Could not read the project branch.",
        ));
    }
    let branch = String::from_utf8(branch.stdout)
        .map_err(|_| HarnessError::invalid("Invalid branch name."))?;
    let branch = stripped(branch);
    let head = run(&root, &["rev-parse", "--verify", "HEAD"])?;
    let head = if head.code == 0 {
        Some(stripped(output(head)?))
    } else {
        None
    };
    let mut reason = head
        .is_none()
        .then(|| "Create the first commit before starting a worktree.".to_string());
    let prefix = root
        .strip_prefix(&top)
        .map_err(|_| HarnessError::conflict("Git project root changed."))?;
    if reason.is_none() && !prefix.as_os_str().is_empty() {
        let tree = run(
            &root,
            &[
                "cat-file",
                "-t",
                &format!(
                    "{}:{}",
                    head.as_deref().unwrap_or(""),
                    prefix.to_string_lossy().replace('\\', "/")
                ),
            ],
        )?;
        if tree.code != 0 || stripped(output(tree)?) != "tree" {
            reason=Some("The selected project directory is absent from HEAD. Commit it before starting a worktree.".into());
        }
    }
    let dirty = if status {
        json!(
            !output(run(
                &root,
                &["status", "--porcelain", "--untracked-files=normal"]
            )?)?
            .is_empty()
        )
    } else {
        Value::Null
    };
    Ok(
        json!({"is_git":true,"branch":if branch.is_empty(){None}else{Some(branch)},"head":head,"dirty":dirty,"worktree_available":reason.is_none(),"reason":reason,"root":top,"common_dir":common}),
    )
}
pub fn worktree_entries(root: &Path) -> Result<Vec<Value>> {
    let raw = output(run(root, &["worktree", "list", "--porcelain", "-z"])?)?;
    let mut entries: Vec<Value> = Vec::new();
    for field in raw.split('\0') {
        let (name, value) = field.split_once(' ').unwrap_or((field, ""));
        if name == "worktree" {
            entries.push(json!({"path":value,"head":null,"branch":null,"bare":false,"detached":false,"locked":false,"prunable":false}));
        } else if let Some(entry) = entries.last_mut() {
            match name {
                "HEAD" => entry["head"] = value.into(),
                "branch" => {
                    entry["branch"] = value.strip_prefix("refs/heads/").unwrap_or(value).into()
                }
                "bare" | "detached" | "locked" | "prunable" => entry[name] = true.into(),
                _ => {}
            }
        }
    }
    Ok(entries)
}
pub fn choices(store: &SessionStore, project: &Value) -> Result<Value> {
    let root = PathBuf::from(
        project["path"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Project path is missing."))?,
    );
    let info = details(&root, false)?;
    if info["is_git"] != true {
        return Ok(json!({"project_id":project["id"],"git":info,"worktrees":[],"skipped":0}));
    }
    let top = Path::new(info["root"].as_str().unwrap_or(""));
    let prefix = root
        .strip_prefix(top)
        .map_err(|_| HarnessError::conflict("Git project root changed."))?;
    let mut choices = Vec::new();
    let mut skipped = 0;
    for entry in worktree_entries(&root)? {
        let path = PathBuf::from(entry["path"].as_str().unwrap_or(""));
        let folder = path.join(prefix);
        if entry["bare"] == true || path == top || path.starts_with(&store.state) {
            continue;
        }
        if entry["prunable"] == true
            || !path.is_absolute()
            || !folder.is_dir()
            || choices.len() == 200
            || infra_core::fs::canonicalize(&folder).ok().as_ref() != Some(&folder)
        {
            skipped += 1;
            continue;
        }
        let checkout = entry["branch"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| format!("detached:{}", entry["head"].as_str().unwrap_or("")));
        choices.push(json!({"id":&store::sha(format!("{}\0{checkout}",norm(&path)).as_bytes())[..16],"path":path,"folder":folder,"branch":entry["branch"],"head":entry["head"],"detached":entry["detached"],"locked":entry["locked"]}));
    }
    Ok(json!({"project_id":project["id"],"git":info,"worktrees":choices,"skipped":skipped}))
}
pub fn resolve_existing(store: &SessionStore, project: &Value, key: &str) -> Result<PathBuf> {
    if key.len() != 16
        || !key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(HarnessError::invalid("Choose a listed worktree."));
    }
    let listing = choices(store, project)?;
    let choice = listing["worktrees"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|w| w["id"] == key)
        .ok_or_else(|| {
            HarnessError::conflict("Git no longer lists that worktree; refresh and choose again.")
        })?;
    let folder = PathBuf::from(choice["folder"].as_str().unwrap_or(""));
    let current = details(&folder, false)?;
    if current["is_git"] != true
        || current["root"] != choice["path"]
        || current["common_dir"] != listing["git"]["common_dir"]
    {
        return Err(HarnessError::conflict(
            "That folder is no longer a worktree of this repository.",
        ));
    }
    Ok(folder)
}
pub fn workspace(
    store: &SessionStore,
    project: &Value,
    data: &Value,
    sid: &str,
) -> Result<(PathBuf, Option<String>, Option<String>)> {
    let source = PathBuf::from(
        project["path"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Project path is missing."))?,
    );
    let workspace = data["workspace"].as_str().unwrap_or("project");
    let branch = data["worktree_branch"].as_str().unwrap_or("");
    if workspace == "existing-worktree" {
        if !branch.is_empty() {
            return Err(HarnessError::invalid(
                "A new branch name is only valid for a new worktree.",
            ));
        }
        let root = resolve_existing(store, project, data["worktree_id"].as_str().unwrap_or(""))?;
        let d = details(&root, false)?;
        return Ok((
            root,
            d["branch"].as_str().map(String::from),
            d["common_dir"].as_str().map(String::from),
        ));
    }
    if !["project", "worktree"].contains(&workspace)
        || branch.len() > 256
        || branch.chars().any(|c| c.is_control())
        || workspace == "project" && !branch.is_empty()
    {
        return Err(HarnessError::invalid(
            "Select a workspace and valid new branch name.",
        ));
    }
    let info = details(&source, workspace == "worktree")?;
    if workspace == "project" {
        return Ok((
            source,
            info["branch"].as_str().map(String::from),
            info["common_dir"].as_str().map(String::from),
        ));
    }
    if info["worktree_available"] != true {
        return Err(HarnessError::invalid(
            info["reason"]
                .as_str()
                .unwrap_or("Worktree creation is unavailable."),
        ));
    }
    let branch = if branch.is_empty() {
        format!("codex/harness-{}", &sid[..sid.len().min(12)])
    } else {
        branch.to_string()
    };
    if branch.starts_with('-')
        || run(
            &source,
            &["check-ref-format", &format!("refs/heads/{branch}")],
        )?
        .code
            != 0
        || run(
            &source,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )?
        .code
            != 1
    {
        return Err(HarnessError::invalid(
            "Choose a valid new branch name that does not exist.",
        ));
    }
    let storage = store.state.join("worktrees");
    infra_core::fs::ensure_directory(&storage, true)?;
    let root = storage.join(sid);
    infra_core::fs::admit_descendant(&storage, &root)?;
    if run(
        &source,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            "--",
            &root.to_string_lossy(),
            info["head"].as_str().unwrap_or(""),
        ],
    )?
    .code
        != 0
    {
        return Err(HarnessError::unavailable(
            "Git could not create the worktree; inspect disk space and branch name.",
        ));
    }
    let prefix = source
        .strip_prefix(info["root"].as_str().unwrap_or(""))
        .map_err(|_| HarnessError::conflict("Git project root changed."))?;
    let cwd = root.join(prefix);
    if !cwd.is_dir() {
        return Err(HarnessError::unavailable(
            "The selected project directory is absent from HEAD; the new worktree was kept.",
        ));
    }
    Ok((
        cwd,
        Some(branch),
        info["common_dir"].as_str().map(String::from),
    ))
}
fn walk_files(root: &Path, path: &Path, out: &mut Vec<String>, depth: usize) -> Result<()> {
    if out.len() >= 50_000 || depth > 64 {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.is_dir() && !infra_core::fs::linked_at(&path, &meta)? {
            if ![
                ".git",
                ".hg",
                ".svn",
                "node_modules",
                "vendor",
                ".venv",
                "venv",
                "__pycache__",
                ".idea",
                ".vscode",
                ".cache",
                ".pytest_cache",
                ".mypy_cache",
                "dist",
                "build",
            ]
            .contains(&entry.file_name().to_string_lossy().as_ref())
            {
                walk_files(root, &path, out, depth + 1)?;
            }
        } else {
            out.push(
                path.strip_prefix(root)
                    .map_err(|_| HarnessError::forbidden("File search escaped its root."))?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
        if out.len() >= 50_000 {
            break;
        }
    }
    Ok(())
}
pub fn search(root: &Path, query: &str) -> Result<Value> {
    if query.chars().count() > 300 || query.chars().any(|c| c.is_control()) {
        return Err(HarnessError::invalid("Invalid file search."));
    }
    let mut paths = match run(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    ) {
        Ok(result) if result.code == 0 => String::from_utf8(result.stdout)
            .map_err(|_| HarnessError::invalid("File search returned invalid path text."))?
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .take(50_000)
            .collect::<Vec<_>>(),
        _ => {
            let mut out = Vec::new();
            walk_files(root, root, &mut out, 0)?;
            out
        }
    };
    let truncated = paths.len() >= 50_000;
    let mut taken = BTreeSet::new();
    paths.retain(|s| taken.insert(s.clone()));
    let mut folders = BTreeSet::new();
    for path in &paths {
        for (i, _) in path.match_indices('/') {
            folders.insert(format!("{}/", &path[..i]));
        }
    }
    let items = paths
        .into_iter()
        .map(|p| (p, "file"))
        .chain(folders.into_iter().map(|p| (p, "folder")))
        .collect::<Vec<_>>();
    let needle = query.trim().trim_start_matches('@').to_lowercase();
    let mut ranked = Vec::new();
    let (folder, name) = needle.rsplit_once('/').unwrap_or(("", &needle));
    for (path, kind) in items {
        let lower = path.to_lowercase();
        if lower == needle {
            continue;
        }
        let mut bare = lower.as_str();
        while let Some(previous) = bare.strip_suffix('/') {
            bare = previous;
        }
        let (parent, base) = bare.rsplit_once('/').unwrap_or(("", bare));
        let order = if needle.is_empty() {
            if bare.contains('/') {
                continue;
            }
            0
        } else if (!folder.is_empty() && parent == folder && base.starts_with(name))
            || (folder.is_empty() && base.starts_with(&needle))
        {
            0
        } else if (!folder.is_empty() && parent == folder && base.contains(name))
            || (folder.is_empty()
                && bare
                    .split(['/', '.', '_', '-'])
                    .any(|s| s.starts_with(&needle)))
        {
            1
        } else if base.contains(&needle) {
            2
        } else if lower.contains(&needle) {
            3
        } else {
            let mut hay = lower.chars();
            if needle.chars().all(|n| hay.by_ref().any(|c| c == n)) {
                4
            } else {
                continue;
            }
        };
        let listing = !folder.is_empty() && name.is_empty() && order == 0;
        ranked.push((
            order,
            bare.matches('/').count(),
            if listing || needle.is_empty() {
                usize::from(kind != "folder")
            } else {
                0
            },
            if listing || needle.is_empty() {
                0
            } else {
                bare.chars().count()
            },
            bare.to_string(),
            json!({"path":path,"kind":kind}),
        ));
    }
    ranked.sort_by(|a, b| (&a.0, &a.1, &a.2, &a.3, &a.4).cmp(&(&b.0, &b.1, &b.2, &b.3, &b.4)));
    Ok(
        json!({"items":ranked.into_iter().take(50).map(|v|v.5).collect::<Vec<_>>(),"truncated":truncated}),
    )
}
pub fn browse(data: &Value) -> Result<Value> {
    if !data.is_object()
        || data
            .as_object()
            .unwrap()
            .keys()
            .any(|k| !["path", "query", "hidden"].contains(&k.as_str()))
    {
        return Err(HarnessError::invalid(
            "Folder browsing accepts path, query and hidden.",
        ));
    }
    let query = data["query"].as_str().unwrap_or("").trim().to_lowercase();
    if query.chars().count() > 100
        || query.chars().any(|c| c.is_control())
        || data.get("hidden").is_some_and(|v| !v.is_boolean())
    {
        return Err(HarnessError::invalid("Invalid folder search."));
    }
    let default = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or_else(|| HarnessError::unavailable("User home is unavailable."))?;
    let root =
        infra_core::fs::canonicalize(data["path"].as_str().map(PathBuf::from).unwrap_or(default))?;
    let mut queue = VecDeque::from([(root.clone(), 0)]);
    let mut entries = Vec::new();
    let mut examined = 0;
    let mut skipped = 0;
    let mut truncated = false;
    let deadline = Instant::now() + Duration::from_millis(1500);
    while let Some((folder, depth)) = queue.pop_front() {
        if examined >= 20_000 || entries.len() >= 200 || Instant::now() > deadline {
            truncated = true;
            break;
        }
        let children = match std::fs::read_dir(&folder) {
            Ok(v) => v,
            Err(_) => {
                if folder == root {
                    return Err(HarnessError::unavailable(
                        "This folder is no longer accessible.",
                    ));
                }
                skipped += 1;
                continue;
            }
        };
        for child in children {
            if examined >= 20_000 || entries.len() >= 200 || Instant::now() > deadline {
                truncated = true;
                break;
            }
            examined += 1;
            let child = match child {
                Ok(v) => v,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let name = child.file_name().to_string_lossy().into_owned();
            if data["hidden"] != true && name.starts_with('.')
                || name.chars().any(|c| c.is_control())
            {
                continue;
            }
            let path = child.path();
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(v) => v,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            if !meta.is_dir()
                || infra_core::fs::linked_at(&path, &meta)?
                || path.to_string_lossy().len() > 1024
            {
                continue;
            }
            if query.is_empty() || name.to_lowercase().contains(&query) {
                entries.push(json!({"name":name,"path":path,"relative":path.strip_prefix(&root).unwrap_or(&path)}));
            }
            if !query.is_empty()
                && ![
                    ".git",
                    "node_modules",
                    "vendor",
                    "__pycache__",
                    ".venv",
                    "venv",
                ]
                .contains(&name.as_str())
            {
                if depth + 1 < 8 {
                    queue.push_back((path, depth + 1));
                } else {
                    truncated = true;
                }
            }
        }
    }
    entries.sort_by_key(|v| {
        (
            v["name"].as_str().unwrap_or("").to_lowercase(),
            v["path"].as_str().unwrap_or("").to_string(),
        )
    });
    Ok(
        json!({"path":root,"parent":root.parent(),"entries":entries,"search":!query.is_empty(),"truncated":truncated,"skipped":skipped}),
    )
}
#[derive(Clone)]
pub struct ProjectDomain {
    pub store: SessionStore,
}
impl Domain for ProjectDomain {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let store = self.store.clone();
            let operation = operation.to_string();
            tokio::task::spawn_blocking(move || {
                let p = &request["params"];
                let q = &request["query"];
                let key = p["project_id"].as_str().unwrap_or("");
                let value = match operation.as_str() {
                    "projects.git" => {
                        let project = store.project(key)?;
                        let mut value =
                            details(Path::new(project["path"].as_str().unwrap_or("")), true)?;
                        value["project_id"] = key.into();
                        value
                    }
                    "projects.worktrees" => choices(&store, &store.project(key)?)?,
                    "projects.browse" => browse(&request["body"])?,
                    "files.project" => {
                        let project = store.project(key)?;
                        let root = if let Some(choice) = q["worktree"].as_str() {
                            resolve_existing(&store, &project, choice)?
                        } else {
                            PathBuf::from(project["path"].as_str().unwrap_or(""))
                        };
                        let mut v = search(&root, q["q"].as_str().unwrap_or(""))?;
                        v["project_id"] = key.into();
                        v
                    }
                    "files.session" => {
                        let sid = p["session_id"].as_str().unwrap_or("");
                        let session = store.get(sid)?;
                        let mut value = if ["creator", "system_run", "system_discovery"]
                            .iter()
                            .any(|k| !session[*k].is_null())
                        {
                            json!({"items":[],"truncated":false})
                        } else {
                            search(
                                Path::new(session["project_path"].as_str().unwrap_or("")),
                                q["q"].as_str().unwrap_or(""),
                            )?
                        };
                        value["session_id"] = sid.into();
                        value
                    }
                    _ => return Err(HarnessError::missing("Unknown project operation.")),
                };
                Ok(Payload::Json(value))
            })
            .await
            .map_err(|_| HarnessError::unavailable("Project operation worker failed."))?
        })
    }
}
