use crate::executable;
use crate::{
    Result, artifact,
    outcome::{self, CorpusPreparation},
    reconstruction::{self, Reconstructor},
    rng, transcript,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Debug)]
pub struct HumanTurn {
    pub prompt: String,
    pub ts: String,
    pub end: String,
    pub changed: Vec<String>,
    pub read: Vec<String>,
    pub answer: String,
}
pub fn prompt_text(r: &Value) -> String {
    if r["type"] == "response_item" {
        r["payload"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("")
    } else if let Some(s) = r["message"]["content"].as_str() {
        s.into()
    } else {
        r["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("")
    }
}
pub fn human(r: &Value) -> bool {
    if !infra_core::continuity::transcript_prompt(r) {
        return false;
    }
    if r["type"] != "response_item" {
        return true;
    }
    let text = prompt_text(r);
    let text = text.trim();
    if text.starts_with("<system-reminder") {
        return false;
    }
    if text.starts_with('<')
        && !text.starts_with("<pasted_content")
        && let Some(end) = text.find('>')
    {
        let tag = text[1..end].split_whitespace().next().unwrap_or("");
        if !tag.is_empty() && text.ends_with(&format!("</{tag}>")) {
            return false;
        }
    }
    true
}
fn call_words(name: &str, data: &Value, base: &Path) -> Vec<(String, bool, PathBuf)> {
    let mut out = vec![];
    match name {
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => {
            if let Some(s) = data["file_path"]
                .as_str()
                .or_else(|| data["notebook_path"].as_str())
            {
                out.push((s.into(), true, base.into()));
            }
        }
        "Read" | "Grep" => {
            if let Some(s) = data["file_path"].as_str().or_else(|| data["path"].as_str()) {
                out.push((s.into(), false, base.into()));
            }
        }
        "Bash" | "PowerShell" => {
            if let Some(s) = data["command"].as_str() {
                out.extend(infra_core::continuity::shell_file_words(
                    s,
                    base,
                    if name == "PowerShell" {
                        "powershell"
                    } else {
                        "sh"
                    },
                ));
            }
        }
        "apply_patch" => {
            if let Some(s) = data["patch"].as_str()
                && let Ok(re) = regex::Regex::new(
                    r"(?m)^\*\*\* (?:Update|Add|Delete) File: (.+)$|^\*\*\* Move to: (.+)$",
                )
            {
                for m in re.captures_iter(s) {
                    if let Some(p) = m.get(1).or_else(|| m.get(2)) {
                        out.push((p.as_str().trim().into(), true, base.into()));
                    }
                }
            }
        }
        _ => {}
    }
    out
}
pub fn human_turns(rows: &[Value], root: &Path) -> Vec<HumanTurn> {
    let mut out = vec![];
    let mut turn = None::<HumanTurn>;
    for r in rows {
        if human(r) {
            if let Some(t) = turn.take() {
                out.push(t);
            }
            turn = Some(HumanTurn {
                prompt: prompt_text(r),
                ts: r["timestamp"].as_str().unwrap_or("").into(),
                end: r["timestamp"].as_str().unwrap_or("").into(),
                changed: vec![],
                read: vec![],
                answer: String::new(),
            });
            continue;
        }
        let Some(t) = turn.as_mut() else {
            continue;
        };
        if let Some(at) = r["timestamp"].as_str() {
            t.end = at.into();
        }
        if r["type"] == "assistant"
            && let Some(s) = r["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| p["type"] == "text")
                .filter_map(|p| p["text"].as_str())
                .rfind(|s| !s.trim().is_empty())
        {
            t.answer = s.into();
        }
        if r["type"] == "response_item"
            && r["payload"]["type"] == "message"
            && r["payload"]["role"] == "assistant"
        {
            let text = r["payload"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("");
            if !text.trim().is_empty() {
                t.answer = text;
            }
        }
        for (name, data, base) in infra_core::continuity::transcript_calls(r, root) {
            for (word, changed, base) in call_words(&name, &data, &base) {
                if let Some(path) = outcome::project_path(&word, &base, root) {
                    if changed {
                        t.changed.push(path);
                    } else {
                        t.read.push(path);
                    }
                }
            }
        }
    }
    if let Some(t) = turn {
        out.push(t);
    }
    out
}
pub type TranscriptSession = (String, PathBuf, Vec<HumanTurn>, String);

pub fn sessions(
    project: &Path,
    claude: &Path,
    codex: &Path,
    hosts: &[String],
) -> Result<Vec<TranscriptSession>> {
    let mut out = vec![];
    let mangled = project
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>();
    if hosts.iter().any(|h| h == "claude") {
        let folder = claude.join(mangled);
        if folder.is_dir() {
            let mut paths = std::fs::read_dir(folder)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            paths.sort();
            for p in paths {
                if p.extension().is_some_and(|e| e == "jsonl") {
                    let rows = transcript::records(&p)?;
                    let branch = rows
                        .iter()
                        .find_map(|r| r["gitBranch"].as_str())
                        .unwrap_or("")
                        .to_string();
                    out.push(("claude".into(), p, human_turns(&rows, project), branch));
                }
            }
        }
    }
    if hosts.iter().any(|h| h == "codex") {
        for p in transcript::files(codex)? {
            let rows = match transcript::records(&p) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let cwd = rows.first().and_then(|r| r["payload"]["cwd"].as_str());
            if !cwd
                .is_some_and(|cwd| Path::new(cwd) == project || Path::new(cwd).starts_with(project))
            {
                continue;
            }
            let branch = rows
                .first()
                .and_then(|r| r["payload"]["git"]["branch"].as_str())
                .unwrap_or("")
                .into();
            out.push(("codex".into(), p, human_turns(&rows, project), branch));
        }
    }
    Ok(out)
}
fn unique(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .collect()
}
fn known(project: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for args in [
        vec!["log", "--all", "--no-renames", "--name-only", "--format="],
        vec!["ls-files"],
    ] {
        if let Ok(s) = reconstruction::git(project, &args) {
            out.extend(s.lines().filter(|s| !s.is_empty()).map(String::from));
        }
    }
    out
}
fn eligible(path: &str) -> bool {
    infra_core::continuity::recent_file_kept(path)
        && ![
            "memory-bank/",
            "project-brain/",
            ".claude/",
            ".codex/",
            ".cursor/",
            ".agents/",
            ".git/",
            ".context-handoff/",
            "tasks/",
        ]
        .iter()
        .any(|p| path.starts_with(p))
        && !["AGENTS.md", "CLAUDE.md", "ACCELERATOR.md"].contains(&path)
        && !path
            .chars()
            .any(|c| c.is_whitespace() || "$`*?\\\"'|;&<>(){}".contains(c))
}
fn id(project: &str, stamp: &str) -> String {
    format!(
        "{project}-{}",
        &artifact::sha(format!("{project}\0{stamp}").as_bytes())[..12]
    )
}
fn before_commit(project: &Path, at: DateTime<Utc>) -> Option<String> {
    reconstruction::git(
        project,
        &[
            "rev-list",
            "-1",
            &format!("--before={}", at.format("%Y-%m-%d %H:%M:%S +0000")),
            "HEAD",
        ],
    )
    .ok()
    .filter(|s| !s.is_empty())
}
fn present(project: &Path, at: DateTime<Utc>) -> Option<BTreeSet<String>> {
    let commit = before_commit(project, at)?;
    reconstruction::git(project, &["ls-tree", "-r", "--name-only", &commit])
        .ok()
        .map(|s| s.lines().map(String::from).collect())
}
pub fn history(
    sessions: &[(String, PathBuf, Vec<HumanTurn>, String)],
) -> Vec<(DateTime<Utc>, Vec<String>)> {
    let mut turns = sessions
        .iter()
        .flat_map(|(_, _, turns, _)| turns.iter())
        .filter_map(|t| {
            reconstruction::instant(&t.ts)
                .map(|at| (at, unique(t.changed.iter().chain(&t.read).cloned())))
        })
        .collect::<Vec<_>>();
    turns.sort_by_key(|t| t.0);
    turns
}
pub fn recent_at(
    history: &[(DateTime<Utc>, Vec<String>)],
    at: DateTime<Utc>,
    present: &BTreeSet<String>,
) -> Vec<String> {
    let mut recent = vec![];
    let selected = history.iter().filter(|(t, _)| *t < at).collect::<Vec<_>>();
    for (_, files) in selected
        .into_iter()
        .rev()
        .take(400)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        recent = unique(
            files
                .iter()
                .filter(|p| present.contains(*p) && infra_core::continuity::recent_file_kept(p))
                .cloned()
                .chain(recent),
        )
        .into_iter()
        .take(64)
        .collect();
    }
    recent
}
pub fn ticket_files(
    project: &Path,
    task: &Value,
    history: &[(DateTime<Utc>, Vec<String>)],
) -> Result<(Vec<String>, Vec<String>)> {
    let pattern = regex::Regex::new(r"[A-Za-z][A-Za-z0-9]*-[0-9]+")
        .map_err(|_| artifact::invalid("ticket expression"))?;
    let text = format!(
        "{} {}",
        task["prompt"].as_str().unwrap_or(""),
        task["branch"].as_str().unwrap_or("")
    );
    let tickets = unique(pattern.find_iter(&text).map(|m| m.as_str().to_uppercase()));
    let Some(at) = task["ts"].as_str().and_then(reconstruction::instant) else {
        return Ok((tickets, vec![]));
    };
    let Some(commit) = before_commit(project, at).filter(|_| !tickets.is_empty()) else {
        return Ok((tickets, vec![]));
    };
    let tree = present(project, at).unwrap_or_default();
    let shown = recent_at(history, at, &tree)
        .into_iter()
        .take(10)
        .collect::<BTreeSet<_>>();
    let mut args = vec![
        "log".into(),
        commit,
        "--no-merges".into(),
        "--no-renames".into(),
        "--relative".into(),
        "-z".into(),
        "--regexp-ignore-case".into(),
        "--extended-regexp".into(),
        "--format=%x1e".into(),
        "--name-only".into(),
        "--max-count=200".into(),
    ];
    args.extend(
        tickets
            .iter()
            .map(|t| format!("--grep=(^|[^A-Za-z0-9]){t}([^0-9]|$)")),
    );
    let raw = reconstruction::git_raw(project, &args, None)?;
    let files = unique(
        raw.split(|b| *b == 0)
            .filter_map(|b| std::str::from_utf8(b).ok())
            .map(|s| s.trim_start_matches('\n'))
            .filter(|s| {
                !s.is_empty()
                    && *s != "\u{1e}"
                    && tree.contains(*s)
                    && !shown.contains(*s)
                    && infra_core::continuity::recent_file_kept(s)
                    && !transcript::mentioned(s, task["prompt"].as_str().unwrap_or(""))
            })
            .map(String::from),
    );
    Ok((tickets, files))
}
pub struct Selection<'a> {
    pub projects: &'a [PathBuf],
    pub claude: &'a Path,
    pub codex: &'a Path,
    pub hosts: &'a [String],
    pub before: DateTime<Utc>,
    pub after: Option<DateTime<Utc>>,
    pub excludes: &'a BTreeMap<String, Vec<DateTime<Utc>>>,
}
pub fn sample(
    selection: &Selection<'_>,
    n: usize,
    seed: &str,
    require_tickets: bool,
) -> Result<Value> {
    let mut candidates = vec![];
    for project in selection.projects {
        let name = project.file_name().unwrap_or_default().to_string_lossy();
        let known = known(project);
        let sessions = sessions(project, selection.claude, selection.codex, selection.hosts)?;
        let history = history(&sessions);
        for (host, path, turns, branch) in sessions {
            let Some(t) = turns.first() else {
                continue;
            };
            let Some(at) = reconstruction::instant(&t.ts) else {
                continue;
            };
            if at >= selection.before
                || selection.after.is_some_and(|a| at < a)
                || selection
                    .excludes
                    .get(name.as_ref())
                    .into_iter()
                    .flatten()
                    .any(|e| (at - *e).num_seconds().unsigned_abs() <= 5)
            {
                continue;
            }
            let gold = unique(
                t.changed
                    .iter()
                    .filter(|p| {
                        known.contains(*p) && eligible(p) && !transcript::mentioned(p, &t.prompt)
                    })
                    .cloned(),
            );
            if gold.is_empty() || t.prompt.trim().is_empty() {
                continue;
            }
            let mut task = json!({"id":id(&name,&t.ts),"project":name,"ts":t.ts,"prompt":t.prompt,"gold":gold,"session":path.file_name().unwrap_or_default().to_string_lossy().split('.').next().unwrap_or(""),"host":host,"seed_hosts":selection.hosts,"branch":branch});
            if require_tickets {
                let (tickets, files) = ticket_files(project, &task, &history)?;
                if files.is_empty() {
                    continue;
                }
                task["tickets"] = json!(tickets);
            }
            candidates.push(task);
        }
    }
    candidates.sort_by(|a, b| {
        (a["project"].as_str(), a["ts"].as_str()).cmp(&(b["project"].as_str(), b["ts"].as_str()))
    });
    let total = candidates.len();
    let mut chosen = if n >= total {
        candidates
    } else {
        rng::Random::decimal(seed)?
            .sample(total, n)
            .into_iter()
            .map(|i| candidates[i].clone())
            .collect()
    };
    chosen.sort_by(|a, b| {
        (a["project"].as_str(), a["ts"].as_str()).cmp(&(b["project"].as_str(), b["ts"].as_str()))
    });
    Ok(
        json!({"version":1,"created":reconstruction::now(),"before":selection.before.to_rfc3339(),"after":selection.after.map(|a|a.to_rfc3339()),"seed":artifact::strict(seed.as_bytes())?,"hosts":selection.hosts,"require_ticket_commits":require_tickets,"candidates":total,"tasks":chosen}),
    )
}
pub fn recall_sources(selection: &Selection<'_>) -> Result<Value> {
    let mut sources = vec![];
    for project in selection.projects {
        let name = project.file_name().unwrap_or_default().to_string_lossy();
        let known = known(project);
        let sessions = sessions(project, selection.claude, selection.codex, selection.hosts)?;
        let history = history(&sessions);
        let mut starts = sessions
            .iter()
            .filter_map(|(_, _, ts, _)| ts.first().and_then(|t| reconstruction::instant(&t.ts)))
            .collect::<Vec<_>>();
        starts.sort();
        for (host, path, turns, _) in sessions {
            let Some(t) = turns.first() else {
                continue;
            };
            let Some(asked) = reconstruction::instant(&t.ts) else {
                continue;
            };
            let Some(ended) = reconstruction::instant(&t.end) else {
                continue;
            };
            if t.prompt.trim().is_empty()
                || asked >= selection.before
                || selection.after.is_some_and(|a| asked < a)
                || selection
                    .excludes
                    .get(name.as_ref())
                    .into_iter()
                    .flatten()
                    .any(|e| (asked - *e).num_seconds().unsigned_abs() <= 5)
                || t.changed.iter().any(|p| known.contains(p) && eligible(p))
            {
                continue;
            }
            let read = unique(
                t.read
                    .iter()
                    .filter(|p| known.contains(*p) && eligible(p))
                    .cloned(),
            );
            let named = read
                .iter()
                .filter(|p| transcript::mentioned(p, &t.answer))
                .cloned()
                .collect::<Vec<_>>();
            if t.answer.chars().count() < 300 || named.is_empty() {
                continue;
            }
            let at = ended + chrono::Duration::seconds(60);
            let Some(tree) = present(project, at) else {
                continue;
            };
            if named.iter().any(|p| !tree.contains(p)) {
                continue;
            }
            let later = starts
                .iter()
                .copied()
                .filter(|s| *s > at && *s < selection.before)
                .find(|start| {
                    present(project, *start).is_some_and(|tree| {
                        named.iter().all(|p| tree.contains(p))
                            && recent_at(&history, *start, &tree)
                                .iter()
                                .take(10)
                                .all(|p| !named.contains(p))
                    })
                });
            let stamp = at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
            sources.push(json!({"id":id(&name,&stamp),"project":name,"ts":stamp,"asked":t.ts,"session":path.file_name().unwrap_or_default().to_string_lossy().split('.').next().unwrap_or(""),"host":host,"seed_hosts":selection.hosts,"prompt":t.prompt,"answer":t.answer.chars().take(6000).collect::<String>(),"read":read.into_iter().take(20).collect::<Vec<_>>(),"named":named,"later":later.map(|s|s.to_rfc3339_opts(chrono::SecondsFormat::AutoSi,true))}));
        }
    }
    sources.sort_by(|a, b| {
        (a["project"].as_str(), a["ts"].as_str()).cmp(&(b["project"].as_str(), b["ts"].as_str()))
    });
    Ok(
        json!({"version":1,"kind":"recall-sources","created":reconstruction::now(),"before":selection.before.to_rfc3339(),"after":selection.after.map(|a|a.to_rfc3339()),"hosts":selection.hosts,"sources":sources}),
    )
}
pub struct NativePreparation {
    pub reconstruction: Reconstructor,
    pub projects_root: PathBuf,
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub timeout: Duration,
    selected_engine: RefCell<executable::SelectedExecutableSource>,
    engine_slot: String,
    inventory_engine_source: PathBuf,
}
fn engine_custody<T>(result: Result<T>) -> Result<T> {
    if let Err(error) = &result {
        crate::process::record_custody_error(error);
    }
    crate::process::ensure_custody()?;
    result
}
impl NativePreparation {
    pub fn new(
        mut reconstruction: Reconstructor,
        projects_root: PathBuf,
        claude: PathBuf,
        codex: PathBuf,
        timeout: Duration,
    ) -> Result<Self> {
        let result = (|| {
            crate::process::ensure_custody()?;
            let mut selected =
                executable::SelectedExecutableSource::capture(&reconstruction.runtime)?;
            let inventory = reconstruction.installer.inventory(
                "PHP Core",
                &reconstruction::TOOLS
                    .iter()
                    .map(|tool| tool.to_string())
                    .collect::<Vec<_>>(),
            )?;
            let mut engines = inventory
                .files
                .into_iter()
                .filter(|file| executable::is_engine_slot(&file.path));
            let file = engines
                .next()
                .ok_or_else(|| artifact::invalid("selected engine inventory role absent"))?;
            if engines.next().is_some() || !file.executable || file.component != "shared" {
                return Err(artifact::invalid("selected engine inventory role mismatch"));
            }
            // Installer and explicit source spellings can differ on Windows.
            // Bind those two names once by the complete held image, not text.
            selected.recheck()?;
            let mut inventory_source = executable::SelectedExecutableSource::capture(&file.source)?;
            if inventory_source.identity() != selected.identity()
                || inventory_source.bytes() != selected.bytes()
            {
                return Err(artifact::invalid(
                    "selected engine inventory source image mismatch",
                ));
            }
            selected.recheck()?;
            inventory_source.recheck()?;
            let inventory_engine_source = inventory_source.path().to_path_buf();
            executable::CanonicalEngine::new(&file.path, selected.bytes(), 0o755)?;
            // Child cwd is a corpus, so retain the exact admitted absolute source.
            reconstruction.runtime = selected.path().to_path_buf();
            Ok(Self {
                reconstruction,
                projects_root,
                claude,
                codex,
                timeout,
                selected_engine: RefCell::new(selected),
                engine_slot: file.path,
                inventory_engine_source,
            })
        })();
        engine_custody(result)
    }
    pub fn seed(&self, project: &Path, task: &Value, root: &Path) -> Result<usize> {
        let at = task["ts"]
            .as_str()
            .and_then(reconstruction::instant)
            .ok_or_else(|| artifact::invalid("task timestamp"))?;
        let anchor = at.to_rfc3339();
        let _clock = crate::clock::enter(Some(&anchor))?;
        let hosts = task["seed_hosts"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_else(|| vec!["claude".into()]);
        let sessions = sessions(project, &self.claude, &self.codex, &hosts)?;
        let history = history(&sessions);
        let ctx = infra_core::RuntimeContext::new(
            infra_core::Roots::installed(root)?,
            "local",
            "governed",
        )?;
        let mut store = infra_core::Store::open(
            root.join("memory-bank/local/context.db"),
            infra_core::OpenOptions::default(),
        )?;
        let selected = history
            .iter()
            .filter(|(when, _)| *when < at)
            .collect::<Vec<_>>();
        for (_, files) in selected
            .into_iter()
            .rev()
            .take(400)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            let paths = files
                .iter()
                .filter(|p| root.join(p).is_file() && infra_core::continuity::recent_file_kept(p))
                .cloned()
                .collect::<Vec<_>>();
            infra_core::continuity::record_recent(&mut store, &ctx, &paths)?;
        }
        Ok(infra_core::continuity::load_recent(&store, "recent_files").len())
    }
    pub fn plant(&self, task: &Value, root: &Path) -> Result<Value> {
        let branch = outcome::branch(task)?;
        let at = task["ts"]
            .as_str()
            .and_then(reconstruction::instant)
            .ok_or_else(|| artifact::invalid("task timestamp"))?;
        let env = reconstruction::runtime_env(&self.reconstruction.cache, at, false)?;
        let anchor = at.to_rfc3339();
        let _clock = crate::clock::enter(Some(&anchor))?;
        let call = |args: Vec<String>| -> Result<Value> {
            let (code, raw, _) =
                self.reconstruction
                    .runtime_at(root, &args, &env, self.timeout, Some(at))?;
            if code != 0 {
                return Err(artifact::invalid("plant runtime unavailable"));
            }
            artifact::strict(&raw)
        };
        let source = task["source_task_id"].as_str().unwrap_or(&branch);
        let saves = task["saves"].as_array().cloned().unwrap_or_default();
        if saves.is_empty() && task.get("save_receipt").is_none() {
            return Ok(json!({}));
        }
        let context = infra_core::RuntimeContext::new(
            infra_core::Roots::installed(root)?,
            "local",
            "governed",
        )?;
        let service = infra_core::results::ResultService::new(context.clone());
        let before = self
            .with_engine(|engine| {
                artifact::image_with_canonical_engine(
                    root,
                    artifact::ImageKind::Public,
                    None,
                    engine,
                )
            })?
            .into_iter()
            .map(|f| f.path)
            .collect::<BTreeSet<_>>();
        let mut planted = json!({"records":0,"promoted":0,"receipts":[]});
        if !saves.is_empty() {
            if task["save_provenance"]
                .as_str()
                .is_some_and(|s| s != "authored")
            {
                return Err(artifact::invalid("injected saves are authored"));
            }
            let started = call(vec![
                "start".into(),
                "--task-id".into(),
                source.into(),
                "--goal".into(),
                infra_core::storage::derive_goal(source),
                "--json".into(),
            ])?;
            planted["source_task_uuid"] = started["task_uuid"].clone();
            planted["save_provenance"] = "authored".into();
            for (index, save) in saves.iter().enumerate() {
                let temp = artifact::Scratch::new(&self.reconstruction.cache, "save-input")?;
                let request = temp.0.join("request.json");
                artifact::write_json(&request, save)?;
                let result_id = format!("save-{}", index + 1);
                let report = call(vec![
                    "record-result".into(),
                    "--task-id".into(),
                    source.into(),
                    "--result-id".into(),
                    result_id.clone(),
                    "--revision".into(),
                    "auto".into(),
                    "--input".into(),
                    request.to_string_lossy().into(),
                    "--json".into(),
                ])?;
                let uuid = planted["source_task_uuid"]
                    .as_str()
                    .ok_or_else(|| artifact::invalid("source task UUID"))?;
                let receipt = infra_core::result_store::EvidenceStore::new(context.clone())
                    .receipt_path(uuid, &result_id);
                let row = json!({"result_id":result_id,"task_uuid":uuid,"sha256":artifact::sha(&artifact::read_file(&receipt,4096)?.0),"record_ids":report["records"].as_array().into_iter().flatten().map(|r|r["id"].clone()).collect::<Vec<_>>()});
                planted["receipts"].as_array_mut().unwrap().push(row);
                planted["records"] = json!(
                    planted["records"].as_u64().unwrap()
                        + report["records"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|r| r["state"] == "created")
                            .count() as u64
                );
                planted["promoted"] = json!(
                    planted["promoted"].as_u64().unwrap()
                        + report["promotion"]["promoted"]
                            .as_array()
                            .map(Vec::len)
                            .unwrap_or(0) as u64
                );
            }
        } else {
            let proof = &task["save_receipt"];
            let saved = task["saved_at"]
                .as_str()
                .and_then(reconstruction::instant)
                .ok_or_else(|| artifact::invalid("captured save time"))?;
            if !["observed", "generated"].contains(&task["save_provenance"].as_str().unwrap_or(""))
                || saved >= at
                || !artifact::exact(proof, &["path", "sha256"])
            {
                return Err(artifact::invalid("captured receipt provenance invalid"));
            }
            let raw = artifact::read_file(
                Path::new(
                    proof["path"]
                        .as_str()
                        .ok_or_else(|| artifact::invalid("receipt path"))?,
                ),
                4096,
            )?
            .0;
            if artifact::sha(&raw) != proof["sha256"].as_str().unwrap_or("") {
                return Err(artifact::invalid("captured receipt mismatch"));
            }
            let receipt = artifact::strict(&raw)?;
            let record = service.task(source, false)?.record;
            if receipt["task_id"] != record["id"]
                || receipt["task_revision"]
                    .as_u64()
                    .is_some_and(|n| n > record["revision"].as_u64().unwrap_or(0))
            {
                return Err(artifact::invalid("captured receipt does not match task"));
            }
            planted["source_task_uuid"] = record["id"].clone();
            planted["save_provenance"] = task["save_provenance"].clone();
            planted["authorship_verified"] = false.into();
            planted["saved_at"] = task["saved_at"].clone();
            planted["question_at"] = task["ts"].clone();
            planted["receipts"] = json!([{"result_id":receipt["result_id"],"task_uuid":record["id"],"sha256":proof["sha256"]}]);
        }
        let served = if source == branch {
            json!({"task_uuid":planted["source_task_uuid"]})
        } else {
            call(vec![
                "start".into(),
                "--task-id".into(),
                branch.clone(),
                "--goal".into(),
                infra_core::storage::derive_goal(&branch),
                "--json".into(),
            ])?
        };
        planted["served_task_uuid"] = served["task_uuid"].clone();
        if source != branch && planted["source_task_uuid"] == planted["served_task_uuid"] {
            return Err(artifact::invalid("cross-task save resolved to same UUID"));
        }
        let index = call(vec!["index".into(), "--json".into()])?;
        let _ = index;
        let store = infra_core::Store::open_read_only(
            root.join("memory-bank/local/context.db"),
            Duration::from_secs(1),
        )?;
        let source_task = service
            .task(planted["source_task_uuid"].as_str().unwrap_or(""), false)?
            .record;
        let documents = infra_core::retrieval::own_saved_documents(&store, &context, &source_task)?;
        planted["documents"] = if saves.is_empty() {
            json!(documents)
        } else {
            json!(
                documents
                    .into_iter()
                    .filter(
                        |p| !before.contains(p) && !p.starts_with("project-brain/dynamic/tasks/")
                    )
                    .collect::<Vec<_>>()
            )
        };
        if saves.is_empty() && planted["documents"].as_array().is_none_or(Vec::is_empty) {
            return Err(artifact::invalid("captured source knowledge absent"));
        }
        for args in [
            vec!["add".into(), "-A".into()],
            vec![
                "-c".into(),
                "commit.gpgsign=false".into(),
                "commit".into(),
                "-q".into(),
                "--no-verify".into(),
                "-m".into(),
                "memory: saved results".into(),
            ],
        ] {
            reconstruction::git_raw(root, &args, Some(&env))?;
        }
        Ok(planted)
    }
}
impl CorpusPreparation for NativePreparation {
    fn with_engine<T>(
        &self,
        operation: impl FnOnce(&executable::CanonicalEngine<'_>) -> Result<T>,
    ) -> Result<T> {
        let result = (|| {
            crate::process::ensure_custody()?;
            let mut selected = self.selected_engine.try_borrow_mut().map_err(|_| {
                crate::RuntimeError::coded(
                    infra_core::ErrorKind::Conflict,
                    "eval-native-engine-authority-borrowed",
                    "selected native engine authority is already borrowed",
                )
            })?;
            if artifact::ordinary(&self.reconstruction.runtime)? != selected.path() {
                return Err(artifact::invalid("selected runtime path changed"));
            }
            selected.recheck()?;
            let engine =
                executable::CanonicalEngine::new(&self.engine_slot, selected.bytes(), 0o755)?;
            let result = operation(&engine);
            if let Err(error) = &result {
                crate::process::record_custody_error(error);
            }
            // Never replace the first custody fatal with a later metadata failure.
            crate::process::ensure_custody()?;
            selected.recheck()?;
            result
        })();
        engine_custody(result)
    }
    fn build(&mut self, task: &Value, destination: &Path) -> Result<Value> {
        self.with_engine(|_| Ok(()))?;
        let project = Path::new(
            task["project"]
                .as_str()
                .ok_or_else(|| artifact::invalid("task project"))?,
        );
        let project = if project.is_absolute() {
            project.to_path_buf()
        } else {
            self.projects_root.join(project)
        };
        let stamp = task["ts"]
            .as_str()
            .and_then(reconstruction::instant)
            .ok_or_else(|| artifact::invalid("task timestamp"))?;
        let edition = self.reconstruction.edition(&project, None)?;
        let mut built = self.reconstruction.build(
            &project,
            destination,
            stamp,
            &edition,
            false,
            false,
            true,
        )?;
        let branch = outcome::branch(task)?;
        reconstruction::git(destination, &["branch", "-m", &branch])?;
        built["recent_files"] = json!(self.seed(&project, task, destination)?);
        built["planted"] = self.plant(task, destination)?;
        self.with_engine(|_| Ok(()))?;
        Ok(built)
    }
    fn runtime_digest(&self, response: bool) -> Result<String> {
        let _ = response;
        self.with_engine(|engine| {
            let inventory = self.reconstruction.installer.inventory(
                "PHP Core",
                &reconstruction::TOOLS
                    .iter()
                    .map(|tool| tool.to_string())
                    .collect::<Vec<_>>(),
            )?;
            let engine_sha = engine.sha256();
            let mut rows = vec![];
            let mut engines = 0usize;
            for file in inventory.files {
                if executable::is_engine_slot(&file.path) {
                    if file.path != engine.path()
                        || !file.executable
                        || file.component != "shared"
                        || artifact::ordinary(&file.source)? != self.inventory_engine_source
                    {
                        return Err(artifact::invalid("selected engine inventory role changed"));
                    }
                    engines += 1;
                    rows.push(json!([file.path, engine_sha, true]));
                } else {
                    let (bytes, _) = artifact::read_file(&file.source, artifact::MAX_FILE)?;
                    rows.push(json!([file.path, artifact::sha(&bytes), file.executable]));
                }
            }
            if engines != 1 {
                return Err(artifact::invalid(
                    "selected engine inventory membership changed",
                ));
            }
            artifact::digest(&json!([
                rows,
                engine_sha,
                artifact::sha(include_bytes!("outcome.rs")),
                artifact::sha(include_bytes!("response.rs")),
                artifact::sha(include_bytes!("intake.rs"))
            ]))
        })
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Recall sampling binds source and authored inputs, roots, regime, exclusions, sample size, seed and tooling independently."
)]
pub fn recall_set(
    sources: &Value,
    authored: &Value,
    projects_root: &Path,
    regime: &str,
    exclude: &BTreeSet<String>,
    n: usize,
    seed: &str,
    tooling: &Path,
) -> Result<Value> {
    if !["next", "later"].contains(&regime) {
        return Err(artifact::invalid("invalid recall regime"));
    }
    let sources = sources["sources"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| artifact::invalid("recall sources required"))?;
    let scenarios = authored
        .get("scenarios")
        .unwrap_or(authored)
        .as_array()
        .ok_or_else(|| artifact::invalid("scenario list required"))?;
    let context = infra_core::RuntimeContext::new(
        infra_core::Roots::installed(tooling)?,
        "local",
        "governed",
    )?;
    let service = infra_core::results::ResultService::new(context);
    let (mut valid, mut refused) = (vec![], BTreeMap::<String, String>::new());
    for scenario in scenarios {
        let identity = scenario["id"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| scenario["id"].to_string());
        let problem = (|| -> Result<Value> {
            if exclude.contains(&identity) {
                return Err(artifact::invalid("left out (--exclude-id)"));
            }
            let source = sources
                .iter()
                .find(|s| s["id"] == identity)
                .ok_or_else(|| artifact::invalid("no such source"))?;
            if scenario.get("skip").is_some() {
                return Err(artifact::invalid(&format!("skipped: {}", scenario["skip"])));
            }
            let learning = &scenario["learning"];
            if !artifact::exact(learning, &["type", "title", "consequence", "sources"])
                || !["finding", "decision"].contains(&learning["type"].as_str().unwrap_or(""))
            {
                return Err(artifact::invalid(
                    "learning needs exactly type, title, consequence and sources",
                ));
            }
            for (key, limit) in [("title", 200), ("consequence", 1000)] {
                if !learning[key]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= limit)
                {
                    return Err(artifact::invalid("learning text exceeds declared limit"));
                }
            }
            let cited = learning["sources"]
                .as_array()
                .filter(|a| {
                    !a.is_empty()
                        && a.len() <= 10
                        && a.iter()
                            .all(|p| source["read"].as_array().is_some_and(|r| r.contains(p)))
                })
                .ok_or_else(|| artifact::invalid("learning sources are files source turn read"))?;
            let needed = scenario["needed"]
                .as_array()
                .filter(|a| !a.is_empty() && a.iter().all(|p| cited.contains(p)))
                .ok_or_else(|| artifact::invalid("needed files are among learning sources"))?;
            let question = scenario["prompt"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| artifact::invalid("question missing"))?;
            if needed
                .iter()
                .filter_map(Value::as_str)
                .any(|p| transcript::mentioned(p, question))
            {
                return Err(artifact::invalid("question names a needed file"));
            }
            service.normalize(&json!({"learnings":[learning],"verified":true}), false)?;
            let stamp = if regime == "later" {
                source["later"]
                    .as_str()
                    .ok_or_else(|| artifact::invalid("no later session without named files"))?
            } else {
                source["ts"]
                    .as_str()
                    .ok_or_else(|| artifact::invalid("source timestamp"))?
            };
            let at = reconstruction::instant(stamp)
                .ok_or_else(|| artifact::invalid("source timestamp"))?;
            let project = projects_root.join(
                source["project"]
                    .as_str()
                    .ok_or_else(|| artifact::invalid("source project"))?,
            );
            let tree = present(&project, at)
                .ok_or_else(|| artifact::invalid("corpus at question cannot be read"))?;
            if cited
                .iter()
                .filter_map(Value::as_str)
                .any(|p| !tree.contains(p))
            {
                return Err(artifact::invalid("learning sources missing from corpus"));
            }
            let task_id = if regime == "next" {
                identity.clone()
            } else {
                id(
                    source["project"].as_str().unwrap(),
                    &format!("{stamp}#{identity}"),
                )
            };
            Ok(
                json!({"id":task_id,"source":identity,"regime":regime,"project":source["project"],"ts":stamp,"session":source["session"],"prompt":question.trim(),"gold":needed,"needed":needed,"saves":[{"learnings":[learning],"verified":true}]}),
            )
        })();
        match problem {
            Ok(task) => valid.push(task),
            Err(error) => {
                refused.insert(identity, error.message);
            }
        }
    }
    valid.sort_by(|a, b| {
        (a["project"].as_str(), a["ts"].as_str()).cmp(&(b["project"].as_str(), b["ts"].as_str()))
    });
    let candidates = valid.len();
    let mut chosen = if n >= candidates {
        valid
    } else {
        rng::Random::decimal(seed)?
            .sample(candidates, n)
            .into_iter()
            .map(|i| valid[i].clone())
            .collect()
    };
    chosen.sort_by(|a, b| {
        (a["project"].as_str(), a["ts"].as_str()).cmp(&(b["project"].as_str(), b["ts"].as_str()))
    });
    Ok(
        json!({"version":1,"kind":"recall","regime":regime,"created":reconstruction::now(),"seed":artifact::strict(seed.as_bytes())?,"candidates":candidates,"refused":refused,"tasks":chosen}),
    )
}
