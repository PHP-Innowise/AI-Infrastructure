use crate::{
    Result, artifact,
    process::{self, CommandSpec},
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};
pub const TOOLS: [&str; 3] = ["claude", "cursor", "codex"];
pub fn instant(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if text.len() < 16 || !matches!(text.as_bytes().get(10), Some(b'T' | b't' | b' ')) {
        return None;
    }
    let mut s = text.to_string();
    if s.ends_with(['z', 'Z']) {
        s.pop();
        s.push_str("+00:00");
    }
    DateTime::parse_from_rfc3339(&s.replace('t', "T").replacen(' ', "T", 1))
        .ok()
        .map(|d| d.with_timezone(&Utc))
        .or_else(|| {
            NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .or_else(|| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%.f").ok())
                .map(|d| Utc.from_utc_datetime(&d))
        })
}
pub fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
pub fn safe_name(value: &str) -> String {
    let s = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    let s = s.trim_matches('.').chars().take(80).collect::<String>();
    if s.is_empty() { "prompt".into() } else { s }
}
pub fn clean_env() -> BTreeMap<String, String> {
    std::env::vars()
        .filter(|(k, _)| {
            !k.starts_with("GIT_")
                && ![
                    "ACCELERATOR_PROJECT_DIR",
                    "ACCELERATOR_STATE_DIR",
                    "ACCELERATOR_TOOLING_DIR",
                    "MEMORY_EVAL_CLOCK",
                ]
                .contains(&k.as_str())
        })
        .collect()
}
pub fn runtime_env(
    cache: &Path,
    stamp: DateTime<Utc>,
    _pinned: bool,
) -> Result<BTreeMap<String, String>> {
    let mut env = std::env::vars()
        .filter(|(k, _)| {
            [
                "PATH",
                "LANG",
                "LC_ALL",
                "LC_CTYPE",
                "TMPDIR",
                "TEMP",
                "TMP",
                "SYSTEMROOT",
                "SYSTEMDRIVE",
                "COMSPEC",
                "PATHEXT",
                "WINDIR",
                "USER",
                "LOGNAME",
                "USERNAME",
                "CONTEXT_EVENT_SERIES",
            ]
            .contains(&k.as_str())
        })
        .collect::<BTreeMap<_, _>>();
    let home = cache.join("home");
    artifact::private_dir(&home)?;
    let config = cache.join("gitconfig");
    if !config.exists() {
        artifact::fresh_file(&config, b"", 0o600)?;
    }
    for (k, v) in [
        ("HOME", home.to_string_lossy().into_owned()),
        (
            "XDG_CONFIG_HOME",
            home.join(".config").to_string_lossy().into_owned(),
        ),
        ("GIT_CONFIG_NOSYSTEM", "1".into()),
        ("GIT_CONFIG_GLOBAL", config.to_string_lossy().into_owned()),
        ("GIT_TERMINAL_PROMPT", "0".into()),
        ("GIT_NO_LAZY_FETCH", "1".into()),
        ("GIT_AUTHOR_NAME", "memory-eval".into()),
        ("GIT_AUTHOR_EMAIL", "memory-eval@localhost".into()),
        ("GIT_COMMITTER_NAME", "memory-eval".into()),
        ("GIT_COMMITTER_EMAIL", "memory-eval@localhost".into()),
        ("GIT_AUTHOR_DATE", format!("{} +0000", stamp.timestamp())),
        ("GIT_COMMITTER_DATE", format!("{} +0000", stamp.timestamp())),
        ("TZ", "UTC".into()),
        ("CONTEXT_CAPSULE_PROFILE", "full".into()),
    ] {
        env.insert(k.into(), v);
    }
    Ok(env)
}
pub fn git_raw(
    root: &Path,
    args: &[String],
    env: Option<&BTreeMap<String, String>>,
) -> Result<Vec<u8>> {
    let mut argv = vec![
        "git".into(),
        "-c".into(),
        "core.fsmonitor=false".into(),
        "-c".into(),
        format!(
            "core.hooksPath={}",
            if cfg!(windows) { "NUL" } else { "/dev/null" }
        ),
        "-c".into(),
        "commit.gpgsign=false".into(),
    ];
    argv.extend(args.iter().cloned());
    let spec = CommandSpec {
        argv,
        cwd: root.into(),
        env: env.cloned().unwrap_or_else(clean_env),
        input: vec![],
    };
    let (code, raw) = process::capture(&spec, Duration::from_secs(120), 8 * 1024 * 1024)?;
    if code != 0 {
        return Err(artifact::invalid("corpus Git operation unavailable"));
    }
    Ok(raw)
}
pub fn git(root: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git_raw(
        root,
        &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        None,
    )?)
    .trim()
    .into())
}
pub fn commit_corpus(
    corpus: &Path,
    env: &BTreeMap<String, String>,
    parent: Option<&str>,
    objects: Option<&Path>,
    branch: &str,
) -> Result<()> {
    let run = |args: Vec<String>| {
        git_raw(
            corpus,
            &[
                vec![
                    "-c".into(),
                    "core.autocrlf=false".into(),
                    "-c".into(),
                    "core.safecrlf=false".into(),
                    "-c".into(),
                    "gc.auto=0".into(),
                    "-c".into(),
                    "commit.gpgsign=false".into(),
                ],
                args,
            ]
            .concat(),
            Some(env),
        )
    };
    run(vec!["init".into(), "-q".into()])?;
    run(vec![
        "symbolic-ref".into(),
        "HEAD".into(),
        "refs/heads/main".into(),
    ])?;
    if let (Some(_), Some(objects)) = (parent, objects) {
        let p = corpus.join(".git/objects/info/alternates");
        artifact::fresh_file(&p, format!("{}\n", objects.display()).as_bytes(), 0o600)?;
    }
    run(vec!["add".into(), "-A".into()])?;
    let tree = String::from_utf8_lossy(&run(vec!["write-tree".into()])?)
        .trim()
        .to_string();
    let mut args = vec![
        "commit-tree".into(),
        tree,
        "-m".into(),
        "memory-eval corpus".into(),
    ];
    if let (Some(parent), Some(_)) = (parent, objects) {
        args.extend(["-p".into(), parent.into()]);
    }
    let commit = String::from_utf8_lossy(&run(args)?).trim().to_string();
    run(vec!["update-ref".into(), "refs/heads/main".into(), commit])?;
    run(vec!["branch".into(), branch.into()])?;
    run(vec![
        "symbolic-ref".into(),
        "HEAD".into(),
        format!("refs/heads/{branch}"),
    ])?;
    Ok(())
}
pub fn metadata(path: &Path) -> Value {
    let Ok((raw, _)) = artifact::read_file(path, 262144) else {
        return json!({});
    };
    let text = String::from_utf8_lossy(&raw).replace("\r\n", "\n");
    let text = text.trim_start_matches('\u{feff}');
    let body = text
        .strip_prefix("---\n")
        .and_then(|s| s.split_once("\n---").map(|p| p.0))
        .unwrap_or(text);
    if let Ok(v) = artifact::strict(body.as_bytes())
        && v.is_object()
    {
        return v;
    }
    let mut out = json!({});
    for key in ["id", "created_at", "updated_at", "created", "last_verified"] {
        if let Some(value) = body.lines().find_map(|line| {
            let (k, v) = line.trim().split_once(':')?;
            (k.trim_matches('"') == key)
                .then(|| artifact::strip_tail(v.trim().trim_matches(['"', '\'']), ",").to_string())
        }) {
            out[key] = value.into();
        }
    }
    out
}
fn memory_paths(root: &Path) -> Result<Vec<(String, String)>> {
    let mut out = vec![];
    fn walk(
        root: &Path,
        dir: &Path,
        kind: &str,
        recursive: bool,
        out: &mut Vec<(String, String)>,
    ) -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }
        for e in std::fs::read_dir(dir)? {
            let p = e?.path();
            let m = std::fs::symlink_metadata(&p)?;
            if infra_core::fs::linked(&m) {
                continue;
            }
            if m.is_dir() && recursive {
                if kind == "archive" && p.file_name().is_some_and(|n| n == "handoffs") {
                    continue;
                }
                walk(root, &p, kind, recursive, out)?;
            } else if m.is_file() {
                let n = p.file_name().unwrap_or_default().to_string_lossy();
                if !n.starts_with('.')
                    && (n.ends_with(".md") || kind == "record" && n.ends_with(".json"))
                {
                    out.push((
                        p.strip_prefix(root)
                            .map_err(|_| artifact::invalid("memory root"))?
                            .to_string_lossy()
                            .replace('\\', "/"),
                        kind.into(),
                    ));
                }
            }
        }
        Ok(())
    }
    for (name, kind, recurse) in [
        ("project-brain/dynamic", "record", true),
        ("project-brain/control/handoffs", "handoff", false),
        ("memory-bank/chunks", "chunk", false),
        ("project-brain/archive", "archive", true),
    ] {
        walk(root, &root.join(name), kind, recurse, &mut out)?;
    }
    out.sort();
    Ok(out)
}
type Window = (DateTime<Utc>, DateTime<Utc>);
fn day_window(day: NaiveDate) -> Option<Window> {
    let a = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0)?);
    Some((
        a,
        a + chrono::Duration::days(1) - chrono::Duration::microseconds(1),
    ))
}
fn window(value: &Value) -> Option<Window> {
    let s = value.as_str()?;
    instant(s).map(|d| (d, d)).or_else(|| {
        NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .and_then(day_window)
    })
}
fn promotions(root: &Path) -> BTreeMap<String, Window> {
    let mut out = BTreeMap::new();
    let p = root.join("project-brain/control/promotions");
    if let Ok(entries) = std::fs::read_dir(p) {
        for e in entries.flatten() {
            let m = metadata(&e.path());
            if let Some(id) = m["destination_memory_id"].as_str() {
                let a = m["created_at"]
                    .as_str()
                    .and_then(instant)
                    .unwrap_or(DateTime::<Utc>::MIN_UTC);
                let b = ["updated_at", "reviewed_at"]
                    .iter()
                    .filter_map(|k| m[*k].as_str().and_then(instant))
                    .max()
                    .unwrap_or(DateTime::<Utc>::MAX_UTC);
                if a != DateTime::<Utc>::MIN_UTC || b != DateTime::<Utc>::MAX_UTC {
                    out.insert(id.into(), (a, b));
                }
            }
        }
    }
    out
}
fn creation(path: &Path, kind: &str, promotions: &BTreeMap<String, Window>) -> Option<Window> {
    let meta = metadata(path);
    if kind != "chunk" {
        return window(&meta["created_at"]);
    }
    let name = path.file_name()?.to_str()?;
    let named = regex::Regex::new(r"^(MEM-(\d{8})-[0-9a-f]{8})-")
        .ok()?
        .captures(name);
    let id = meta["id"]
        .as_str()
        .map(String::from)
        .or_else(|| named.as_ref().map(|m| m[1].to_string()));
    let mut windows = vec![];
    if let Some(m) = named {
        windows.push(
            NaiveDate::parse_from_str(&m[2], "%Y%m%d")
                .ok()
                .and_then(day_window),
        );
    } else {
        windows.push(window(&meta["created"]));
    }
    if let Some(w) = id.and_then(|id| promotions.get(&id).copied()) {
        windows.push(Some(w));
    }
    let windows = windows.into_iter().flatten().collect::<Vec<_>>();
    let a = windows.iter().map(|w| w.0).max()?;
    let b = windows.iter().map(|w| w.1).min()?;
    if a <= b {
        Some((a, b))
    } else {
        windows.into_iter().min_by_key(|w| w.1 - w.0)
    }
}
pub fn reconstruct_memory(
    project: &Path,
    corpus: &Path,
    moment: DateTime<Utc>,
    keep_updated: bool,
) -> Result<Value> {
    let mut counts = BTreeMap::<String, u64>::new();
    for key in [
        "from_git",
        "from_worktree",
        "dropped_future",
        "undetermined",
        "updated_after",
        "unreconstructable_updated",
        "archive_from_git",
        "archive_from_worktree",
        "archive_dropped_future",
        "archive_undetermined",
        "archive_unreconstructable_updated",
        "archive_updated_after",
        "archive_still_dynamic",
    ] {
        counts.insert(key.into(), 0);
    }
    let mut promotion = promotions(corpus);
    promotion.extend(promotions(project));
    let committed = memory_paths(corpus)?;
    let mut seen = BTreeSet::new();
    let active = committed
        .iter()
        .filter(|(_, k)| k == "record")
        .filter_map(|(p, _)| Path::new(p).file_name().map(|n| n.to_os_string()))
        .collect::<BTreeSet<_>>();
    for (name, kind) in &committed {
        seen.insert(name.clone());
        let key = if kind == "archive" { "archive_" } else { "" };
        if creation(&corpus.join(name), kind, &promotion).is_some_and(|w| w.0 > moment) {
            std::fs::remove_file(corpus.join(name))?;
            *counts.entry(format!("{key}dropped_future")).or_default() += 1;
        } else {
            *counts.entry(format!("{key}from_git")).or_default() += 1;
        }
    }
    for (name, kind) in memory_paths(project)? {
        if seen.contains(&name) || kind == "handoff" {
            continue;
        }
        let prefix = if kind == "archive" { "archive_" } else { "" };
        if kind == "archive"
            && Path::new(&name)
                .file_name()
                .is_some_and(|n| active.contains(n))
        {
            *counts.entry("archive_still_dynamic".into()).or_default() += 1;
            continue;
        }
        let path = project.join(&name);
        let w = creation(&path, &kind, &promotion);
        let key = if w.is_some_and(|w| w.0 > moment) {
            "dropped_future"
        } else {
            let modified: DateTime<Utc> = std::fs::metadata(&path)?.modified()?.into();
            if !w.is_some_and(|w| w.1 <= moment) && modified > moment {
                "undetermined"
            } else {
                let m = metadata(&path);
                let edited = if kind == "chunk" {
                    m["last_verified"]
                        .as_str()
                        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
                        .is_some_and(|d| d > moment.date_naive())
                } else {
                    m["updated_at"]
                        .as_str()
                        .and_then(instant)
                        .is_some_and(|d| d > moment)
                };
                if edited && !keep_updated {
                    "unreconstructable_updated"
                } else {
                    let (raw, mode) = artifact::read_file(&path, artifact::MAX_FILE)?;
                    artifact::fresh_file(&corpus.join(&name), &raw, mode)?;
                    if edited {
                        *counts.entry(format!("{prefix}updated_after")).or_default() += 1;
                    }
                    "from_worktree"
                }
            }
        };
        *counts.entry(format!("{prefix}{key}")).or_default() += 1;
    }
    Ok(json!(counts))
}
pub struct Reconstructor {
    pub source: PathBuf,
    pub cache: PathBuf,
    pub installer: infra_tooling::installer::Installer,
    pub runtime: PathBuf,
}
impl Reconstructor {
    pub fn new(source: &Path, cache: &Path, runtime: &Path) -> Result<Self> {
        if artifact::overlaps(source, cache)? {
            return Err(artifact::invalid(
                "client corpora cache must be outside tooling clone",
            ));
        }
        artifact::private_dir(cache)?;
        Ok(Self {
            source: source.into(),
            cache: cache.into(),
            installer: infra_tooling::installer::Installer::new(source)?.with_engine(runtime)?,
            runtime: runtime.into(),
        })
    }
    pub fn edition(&self, project: &Path, forced: Option<&str>) -> Result<String> {
        if let Some(s) = forced.filter(|s| *s != "auto") {
            Ok(s.into())
        } else {
            self.installer
                .detect_edition(project)?
                .ok_or_else(|| artifact::invalid("project edition undetermined"))
        }
    }
    pub fn materialize(&self, project: &Path, tree: &str, target: &Path) -> Result<()> {
        let listing = git_raw(
            project,
            &["ls-tree".into(), "-r".into(), "-z".into(), tree.into()],
            None,
        )?;
        let mut rows = vec![];
        for entry in listing.split(|b| *b == 0) {
            let Some(at) = entry.iter().position(|b| *b == b'\t') else {
                continue;
            };
            let meta = String::from_utf8_lossy(&entry[..at]);
            let fields = meta.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3
                || fields[1] != "blob"
                || !["100644", "100755"].contains(&fields[0])
            {
                continue;
            }
            let name = std::str::from_utf8(&entry[at + 1..])
                .map_err(|_| artifact::invalid("Git path is not UTF-8"))?;
            if artifact::native_relative(name).is_err() || !artifact::source_allowed(name) {
                continue;
            }
            let bytes = git_raw(
                project,
                &["cat-file".into(), "blob".into(), fields[2].into()],
                None,
            )?;
            rows.push(artifact::ImageFile {
                path: name.into(),
                bytes,
                mode: if fields[0] == "100755" { 0o755 } else { 0o644 },
            });
        }
        artifact::copy_native_image(&rows, target)
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Historical reconstruction keeps project, target, timestamp, edition, current-image, update and linked-history policies explicit."
    )]
    pub fn build(
        &self,
        project: &Path,
        target: &Path,
        stamp: DateTime<Utc>,
        edition: &str,
        as_of_now: bool,
        keep_updated: bool,
        linked_history: bool,
    ) -> Result<Value> {
        let prefix = git(project, &["rev-parse", "--show-prefix"]).unwrap_or_default();
        let commit = if as_of_now {
            git(project, &["rev-parse", "HEAD"])?
        } else {
            git(
                project,
                &[
                    "rev-list",
                    "-1",
                    &format!("--before={}", stamp.format("%Y-%m-%d %H:%M:%S +0000")),
                    "HEAD",
                ],
            )?
        };
        if commit.is_empty() {
            return Err(artifact::invalid("no-history"));
        }
        if as_of_now {
            let mut names = BTreeSet::new();
            for name in git_raw(
                project,
                &[
                    "ls-files".into(),
                    "-z".into(),
                    "-c".into(),
                    "-o".into(),
                    "--exclude-standard".into(),
                ],
                None,
            )?
            .split(|b| *b == 0)
            {
                if let Ok(n) = std::str::from_utf8(name)
                    && !n.is_empty()
                {
                    names.insert(n.to_string());
                }
            }
            for (n, _) in memory_paths(project)? {
                names.insert(n);
            }
            let mut rows = vec![];
            for name in names {
                if artifact::source_allowed(&name) {
                    let p = project.join(&name);
                    if let Ok((bytes, mode)) = artifact::read_file(&p, artifact::MAX_FILE) {
                        rows.push(artifact::ImageFile {
                            path: name,
                            bytes,
                            mode,
                        });
                    }
                }
            }
            artifact::copy_native_image(&rows, target)?;
        } else {
            let spec = if prefix.is_empty() {
                format!("{commit}^{{tree}}")
            } else {
                format!("{commit}:{}", artifact::strip_tail(&prefix, "/"))
            };
            let tree = git(project, &["rev-parse", "--verify", "--quiet", &spec])?;
            self.materialize(project, &tree, target)?;
        }
        let provenance = if as_of_now {
            json!({"future_present":memory_paths(target)?.iter().filter(|(p,k)|creation(&target.join(p),k,&promotions(target)).is_some_and(|w|w.0>stamp)).count(),"from_worktree":memory_paths(target)?.len()})
        } else {
            reconstruct_memory(project, target, stamp, keep_updated)?
        };
        let config = Path::new("project-brain/config/runtime.json");
        if !target.join(config).exists() && project.join(config).is_file() {
            let (raw, mode) = artifact::read_file(&project.join(config), artifact::MAX_FILE)?;
            artifact::fresh_file(&target.join(config), &raw, mode)?;
        }
        let overlay = self.installer.overlay(
            target,
            edition,
            &TOOLS.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )?;
        let env = runtime_env(&self.cache, stamp, !as_of_now)?;
        let mut objects = None;
        if linked_history && prefix.is_empty() {
            let folder = self.cache.join("history");
            artifact::private_dir(&folder)?;
            let mirror = folder.join(format!(
                "{}.git",
                &artifact::sha(project.to_string_lossy().as_bytes())[..16]
            ));
            if !mirror.exists() {
                git_raw(
                    &folder,
                    &[
                        "clone".into(),
                        "--bare".into(),
                        "--quiet".into(),
                        "--no-hardlinks".into(),
                        project.to_string_lossy().into(),
                        mirror.to_string_lossy().into(),
                    ],
                    None,
                )?;
            }
            if git(&mirror, &["cat-file", "-e", &commit]).is_err() {
                git_raw(
                    &mirror,
                    &[
                        "fetch".into(),
                        "--quiet".into(),
                        "--no-tags".into(),
                        project.to_string_lossy().into(),
                        "+HEAD:refs/memory-eval/head".into(),
                    ],
                    None,
                )?;
            }
            objects = Some(mirror.join("objects"));
        }
        commit_corpus(
            target,
            &env,
            if objects.is_some() {
                Some(&commit)
            } else {
                None
            },
            objects.as_deref(),
            &format!(
                "eval/{}",
                &artifact::sha(target.to_string_lossy().as_bytes())[..12]
            ),
        )?;
        Ok(
            json!({"commit":commit,"edition":edition,"provenance":provenance,"overlay":overlay,"history_linked":objects.is_some()}),
        )
    }
    pub fn runtime(
        &self,
        corpus: &Path,
        args: &[String],
        env: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<(i32, Vec<u8>, f64)> {
        self.runtime_at(corpus, args, env, timeout, None)
    }
    pub fn runtime_at(
        &self,
        corpus: &Path,
        args: &[String],
        env: &BTreeMap<String, String>,
        timeout: Duration,
        anchor: Option<DateTime<Utc>>,
    ) -> Result<(i32, Vec<u8>, f64)> {
        if env.contains_key("MEMORY_EVAL_CLOCK") {
            return Err(artifact::invalid(
                "ambient evaluation clock refused; use explicit runtime_at anchor",
            ));
        }
        let started = std::time::Instant::now();
        let mut argv = vec![self.runtime.to_string_lossy().into_owned()];
        if let Some(anchor) = anchor {
            let value = anchor.to_rfc3339();
            infra_core::domain::clock::parse_python_anchor(&value)?;
            argv.extend([
                "__memory-eval-runtime".into(),
                "--clock".into(),
                value,
                "--".into(),
            ]);
        }
        argv.extend([
            "memory".into(),
            "--root".into(),
            corpus.to_string_lossy().into_owned(),
        ]);
        argv.extend(args.iter().cloned());
        let spec = CommandSpec {
            argv,
            cwd: corpus.into(),
            env: env.clone(),
            input: vec![],
        };
        let (code, raw) = process::capture(&spec, timeout, 8 * 1024 * 1024)?;
        Ok((code, raw, started.elapsed().as_secs_f64()))
    }
}
