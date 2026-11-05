//! Seeded native hook/Git lifecycle experiment. Reports observations and invariant violations.
use crate::{
    Result, artifact, reconstruction,
    rehearsal::{self, Harness},
    rng::Random,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};
const TIMEOUT: Duration = Duration::from_secs(60);
#[derive(Clone)]
struct Session {
    id: String,
    host: String,
    kind: String,
    checkout: String,
    launch: String,
    sub: String,
}
struct Fuzz {
    h: Harness,
    seed: String,
    rng: Random,
    checkouts: BTreeMap<String, PathBuf>,
    removed: BTreeSet<String>,
    sessions: BTreeMap<String, Session>,
    saved: BTreeMap<String, (String, Option<String>, String)>,
    counts: BTreeMap<String, u64>,
    events: BTreeMap<String, u64>,
    violations: Vec<String>,
    log: Vec<String>,
    counter: usize,
    interrupt: f64,
    contention: f64,
    verbose: bool,
    clone_before: String,
}
fn inc(map: &mut BTreeMap<String, u64>, key: &str) {
    *map.entry(key.into()).or_default() += 1;
}
fn tail(s: &str, n: usize) -> String {
    s.chars()
        .rev()
        .take(n)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}
fn branch(folder: &Path) -> String {
    reconstruction::git(folder, &["branch", "--show-current"]).unwrap_or_default()
}
fn git(folder: &Path, args: &[&str]) -> Result<String> {
    let mut argv = vec![
        "git",
        "-c",
        "user.name=Fuzz",
        "-c",
        "user.email=fuzz@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "maintenance.auto=false",
        "-c",
        "core.editor=true",
    ];
    argv.extend_from_slice(args);
    let result = rehearsal::run_command(
        argv.into_iter().map(String::from).collect(),
        folder,
        &crate::bench_driver::clean_env(),
        vec![],
        TIMEOUT,
    )?;
    if result["exit"] != 0 {
        return Err(artifact::invalid(&tail(
            result["stderr"].as_str().unwrap_or("Git action failed"),
            300,
        )));
    }
    Ok(artifact::strip_trailing_space(result["stdout"].as_str().unwrap_or("")).into())
}
fn rebase(folder: &Path) -> String {
    let admin =
        reconstruction::git(folder, &["rev-parse", "--absolute-git-dir"]).unwrap_or_default();
    for state in ["rebase-merge", "rebase-apply"] {
        if let Ok(name) =
            std::fs::read_to_string(PathBuf::from(&admin).join(state).join("head-name"))
            && let Some(name) = name.trim().strip_prefix("refs/heads/")
        {
            return name.into();
        }
    }
    String::new()
}
fn task_branch(folder: &Path) -> String {
    let rebasing = rebase(folder);
    if !rebasing.is_empty() {
        return rebasing;
    }
    let branch = branch(folder);
    if !branch.is_empty() {
        return branch;
    }
    let admin =
        reconstruction::git(folder, &["rev-parse", "--absolute-git-dir"]).unwrap_or_default();
    artifact::read_json(&PathBuf::from(admin).join("codex-thread.json"), 65536)
        .ok()
        .and_then(|v| {
            v["ownerThreadId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| format!("codex-thread/{s}"))
        })
        .unwrap_or_default()
}
#[derive(Clone)]
struct Interruption {
    delay: Duration,
    how: String,
    pick: f64,
}
fn interrupted_command(
    argv: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    input: Vec<u8>,
    stop: &Interruption,
) -> Result<i32> {
    use infra_host::capture::{ExperimentSignal, ExperimentStop};
    let signal = match stop.how.as_str() {
        "term" => ExperimentSignal::TerminateLeader,
        "int" => ExperimentSignal::InterruptGroup,
        "kill" => ExperimentSignal::KillGroup,
        _ => return Err(artifact::invalid("Unsupported lifecycle experiment signal")),
    };
    crate::process::experiment(
        &crate::process::CommandSpec {
            argv: argv.to_vec(),
            cwd: cwd.to_path_buf(),
            env: env.clone(),
            input,
        },
        ExperimentStop {
            after: stop.delay,
            signal,
            grace: TIMEOUT,
        },
    )
}
struct HeldLock {
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for HeldLock {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
fn lock(root: &Path, hold: Duration) -> Result<HeldLock> {
    #[cfg(not(unix))]
    let _ = hold;
    let p = root.join("project-brain/local/runtime.lock");
    if let Some(p) = p.parent() {
        std::fs::create_dir_all(p)?;
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let thread = std::thread::spawn(move || {
        let result = (|| -> std::io::Result<()> {
            let f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)?;
            #[cfg(not(unix))]
            let _ = &f;
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let _ = tx.send(Ok(()));
                std::thread::sleep(hold);
                unsafe {
                    libc::flock(f.as_raw_fd(), libc::LOCK_UN);
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tx.send(Err("POSIX contention needs Unix"));
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = tx.send(Err("contention lock unavailable"));
        }
    });
    rx.recv_timeout(TIMEOUT)
        .map_err(|_| artifact::invalid("contention lock unavailable"))?
        .map_err(artifact::invalid)?;
    Ok(HeldLock {
        thread: Some(thread),
    })
}
/// The exit status of every refresh a prompt hook ran, in the order the
/// hooks recorded them.
fn refreshes(h: &Harness) -> Vec<Value> {
    std::fs::read_to_string(h.state.join("memory-bank/local/refresh-health.ndjson"))
        .unwrap_or_default()
        .lines()
        .filter_map(|s| artifact::strict(s.as_bytes()).ok())
        .filter(|v| v["source"] == "hook")
        .map(|v| v["hook_status"].clone())
        .collect()
}
fn place(_h: &Harness, s: &Session, checkouts: &BTreeMap<String, PathBuf>) -> Option<PathBuf> {
    let folder = checkouts.get(&s.checkout)?.join(&s.sub);
    folder.is_dir().then_some(folder)
}
fn event(
    h: &Harness,
    s: &Session,
    checkouts: &BTreeMap<String, PathBuf>,
    stage: usize,
    fields: Value,
    stop: Option<&Interruption>,
    wait: Option<Duration>,
) -> Result<Vec<Value>> {
    let Some(place) = place(h, s, checkouts) else {
        return Ok(vec![]);
    };
    let launch = checkouts.get(&s.launch).unwrap_or(&h.project);
    let names = rehearsal::event_names(&s.host)?;
    let mut fields = fields;
    fields["transcript_path"] = json!(h.work.join("transcripts").join(format!("{}.jsonl", s.id)));
    let payload = rehearsal::payload(&s.host, &s.id, names[stage], &place, fields);
    let through = (s.kind == "worktree" && s.host == "codex")
        || s.host == "claude" && ["worktree", "sub"].contains(&s.kind.as_str());
    let (mut env, settings) = h.hooks(
        &s.host,
        if h.attached {
            launch
        } else if s.kind == "entered" {
            &h.project
        } else {
            &place
        },
    )?;
    let _lock = wait.map(|hold| lock(&h.state, hold)).transpose()?;
    if wait.is_some() {
        env.insert("CONTEXT_HOOK_BUDGET".into(), "1".into());
    }
    let calls = h.argv(&settings, &s.host, names[stage], through && !h.attached)?;
    let chosen = stop.map(|s| (s.pick * calls.len() as f64) as usize);
    let mut out = vec![];
    for (index, (label, argv)) in calls.into_iter().enumerate() {
        if chosen == Some(index) {
            let code = interrupted_command(
                &argv,
                &place,
                &env,
                serde_json::to_vec(&payload)?,
                stop.unwrap(),
            )?;
            out.push(json!({"hook":label,"interrupted":true,"how":stop.unwrap().how,"exit":code,"stdout":"","stderr":""}));
        } else {
            let mut result =
                rehearsal::run_command(argv, &place, &env, serde_json::to_vec(&payload)?, TIMEOUT)?;
            result["hook"] = label.into();
            out.push(result);
        }
    }
    Ok(out)
}
#[derive(Default)]
struct Turn {
    violations: Vec<String>,
    saved: Option<(String, Option<String>, String)>,
    events: BTreeMap<String, u64>,
    log: Vec<String>,
    session: Option<Session>,
    rng: Option<Random>,
}
#[expect(
    clippy::too_many_arguments,
    reason = "A lifecycle turn carries the harness, session, checkout map, generation marker, seed, RNG and independent interruption controls."
)]
fn turn(
    h: Harness,
    mut s: Session,
    checkouts: BTreeMap<String, PathBuf>,
    marker: String,
    seed: String,
    mut rng: Random,
    interrupt: f64,
    contention: f64,
    burst: bool,
) -> Result<Turn> {
    let mut out = Turn::default();
    let Some(cwd) = place(&h, &s, &checkouts) else {
        out.rng = Some(rng);
        return Ok(out);
    };
    let mut draw = Random::text(&format!("{seed}:{marker}:interrupt"));
    let at = if !burst && draw.random() < interrupt {
        Some(draw.below(4))
    } else {
        None
    };
    let stopped = at.map(|_| {
        let delay = (draw.random() * 0.3 * 1000.0).round() / 1000.0;
        let how = ["kill", "kill", "term", "int"][draw.below(4)].to_string();
        Interruption {
            delay: Duration::from_secs_f64(delay),
            how,
            pick: draw.random(),
        }
    });
    if let Some(at) = at {
        inc(
            &mut out.events,
            [
                "prompt hook stopped",
                "Stop hook stopped",
                "save stopped",
                "Stop after the save stopped",
            ][at],
        );
    }
    let mut contention_draw = Random::text(&format!("{seed}:{marker}:contention"));
    let waits = if !burst && at.is_none() && contention_draw.random() < contention {
        Some(contention_draw.below(2))
    } else {
        None
    };
    let hold =
        Duration::from_secs_f64(((1.2 + contention_draw.random() * 1.3) * 100.0).round() / 100.0);
    if let Some(stage) = waits {
        inc(
            &mut out.events,
            if stage == 0 {
                "prompt waited on a lock"
            } else {
                "stop after the save waited on a lock"
            },
        );
    }
    let absorb = |rows: &[Value], out: &mut Turn| {
        for row in rows {
            if row["interrupted"] == true {
                inc(
                    &mut out.events,
                    &format!("stopped by {}", row["how"].as_str().unwrap_or("")),
                );
                if row["exit"].as_i64().is_some_and(|n| n < 0) {
                    inc(&mut out.events, "stopped while running");
                }
            } else if row["exit"] != 0 {
                out.violations.push(format!(
                    "{} {}: exit {} {}",
                    s.host,
                    row["hook"],
                    row["exit"],
                    tail(row["stderr"].as_str().unwrap_or(""), 200)
                ));
            }
        }
    };
    // An attached Cursor's prompt hook gives Cursor no capsule - its rule
    // would be written into the project - so a refresh that degraded or ran
    // out of its budget cannot say so, and the status its hook records is
    // all there is to count.
    let unseen = (h.attached && s.host == "cursor").then(|| refreshes(&h).len());
    let prompt = event(
        &h,
        &s,
        &checkouts,
        1,
        json!({"prompt":format!("How is the invoice numbered? {marker}"),"attachments":[]}),
        if at == Some(0) {
            stopped.as_ref()
        } else {
            None
        },
        (waits == Some(0)).then_some(hold),
    )?;
    absorb(&prompt, &mut out);
    if let Some(before) = unseen {
        let recorded = refreshes(&h).get(before..).unwrap_or_default().to_vec();
        if recorded.contains(&json!(1)) {
            inc(&mut out.events, "refresh exited 1 without a capsule");
        }
        if waits == Some(0) && recorded.contains(&json!(124)) {
            inc(&mut out.events, "refresh over its budget");
        }
    }
    let said = rehearsal::capsule(&cwd, &s.host, &prompt);
    if said.contains("Memory refresh incomplete") {
        inc(&mut out.events, "refresh degraded");
    }
    for phrase in ["Memory not consulted", "Memory refresh unavailable"] {
        if said.contains(phrase) {
            if waits == Some(0) && phrase == "Memory refresh unavailable" {
                inc(&mut out.events, "refresh over its budget");
            } else {
                out.violations.push(format!("prompt {}: {phrase}", s.id));
            }
        }
    }
    let live = checkouts
        .iter()
        .filter(|(name, p)| name.as_str() != "root" && p.is_dir())
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    if !burst && s.host == "claude" && s.kind == "root" && !live.is_empty() && rng.random() < 0.15 {
        s.kind = "entered".into();
        s.checkout = live[rng.below(live.len())].clone();
        s.sub.clear();
        inc(&mut out.events, "worktree entered");
        out.session = Some(s.clone());
    }
    let Some(cwd) = place(&h, &s, &checkouts) else {
        out.rng = Some(rng);
        return Ok(out);
    };
    let stop = event(
        &h,
        &s,
        &checkouts,
        2,
        json!({"stop_hook_active":false,"last_assistant_message":"Checked.","status":"completed","loop_count":0}),
        if at == Some(1) {
            stopped.as_ref()
        } else {
            None
        },
        None,
    )?;
    for row in &stop {
        if row["interrupted"] == true {
            inc(
                &mut out.events,
                &format!("stopped by {}", row["how"].as_str().unwrap_or("")),
            );
        } else if row["exit"] != 0 {
            out.violations
                .push(format!("Stop {} exit {}", s.id, row["exit"]));
        }
    }
    let current = task_branch(&cwd);
    let Some(request) = rehearsal::request_of(&stop) else {
        out.rng = Some(rng);
        return Ok(out);
    };
    let mut moved = false;
    let roll = rng.random();
    if rebase(&cwd).is_empty() {
        if !burst && roll < 0.12 {
            let new = format!("feature/SHOP-moved-{marker}");
            let _ = git(&cwd, &["checkout", "-q", "-b", &new]);
            moved = true;
        } else if !burst && roll < 0.2 {
            let name = format!("src/Commit{marker}.php");
            std::fs::write(cwd.join(&name), "<?php\n")?;
            let _ = git(&cwd, &["add", "src"]);
            let _ = git(&cwd, &["commit", "-qm", &format!("Commit {marker}")]);
            inc(&mut out.events, "commit during a save");
        }
    }
    let launch = checkouts.get(&s.launch).unwrap_or(&h.project);
    let through = ["worktree", "sub"].contains(&s.kind.as_str());
    let env = h.session_env(
        &s.host,
        if h.attached {
            launch
        } else if s.kind == "entered" {
            &h.project
        } else {
            &cwd
        },
        &cwd,
        through,
        TIMEOUT,
    )?;
    let choice = rng.random();
    let held = !burst
        && !h.attached
        && !moved
        && !current.is_empty()
        && crate::isolation::which("bwrap").is_some()
        && Random::text(&marker).random() < 0.2
        && at != Some(2);
    let data = json!({"progress":format!("Turn {marker}."),"verified":true,"learnings":[{"type":"finding","title":format!("Invoice numbering {marker}"),"consequence":format!("Invoices are numbered per tenant ({marker})."),"sources":["src/Invoice.php"]}]});
    let (command, input) = match rehearsal::save_command(&request, &data) {
        Ok(v) => v,
        Err(_) => {
            out.violations.push("request without exact command".into());
            out.rng = Some(rng);
            return Ok(out);
        }
    };
    if choice < 0.8 {
        let mut argv = vec!["bash".into(), "-c".into(), command];
        if held {
            let work = h.work.to_string_lossy().into_owned();
            let temp = h.work.join("tmp").to_string_lossy().into_owned();
            let mut prefix = vec![
                "bwrap".into(),
                "--ro-bind".into(),
                "/".into(),
                "/".into(),
                "--dev".into(),
                "/dev".into(),
                "--proc".into(),
                "/proc".into(),
                "--ro-bind".into(),
                work.clone(),
                work,
                "--bind".into(),
                temp.clone(),
                temp,
                "--".into(),
            ];
            prefix.extend(argv);
            argv = prefix;
        }
        if at == Some(2) {
            let code = interrupted_command(&argv, &cwd, &env, input, stopped.as_ref().unwrap())?;
            inc(
                &mut out.events,
                &format!("stopped by {}", stopped.as_ref().unwrap().how),
            );
            if code < 0 {
                inc(&mut out.events, "stopped while running");
            }
        } else {
            let saved = rehearsal::run_command(argv, &cwd, &env, input, TIMEOUT)?;
            let value = artifact::strict(saved["stdout"].as_str().unwrap_or("").as_bytes()).ok();
            if held {
                if saved["exit"] != 0 || value.as_ref().is_none_or(|v| v["state"] != "spooled") {
                    out.violations
                        .push(format!("save {} under sandbox did not spool", s.id));
                }
            } else if saved["exit"] != 0 && !moved {
                out.violations.push(format!(
                    "save {} refused: {}",
                    s.id,
                    tail(saved["stderr"].as_str().unwrap_or(""), 240)
                ));
            } else if saved["exit"] == 0 {
                out.saved = Some((
                    current.clone(),
                    value.and_then(|v| v["task"]["id"].as_str().map(String::from)),
                    s.id.clone(),
                ));
            }
        }
    } else if choice < 0.9 {
        let (command, _) = rehearsal::save_command(&request, &data)?;
        let command = command
            .replace("echo -E '<JSON>' | ", "")
            .replace("--input - --json", "--skip --json");
        let command = if command.contains("--skip") {
            command
        } else {
            command.replacen(" --json", " --skip --json", 1)
        };
        let _ = rehearsal::run_command(
            vec!["bash".into(), "-c".into(), command],
            &cwd,
            &env,
            vec![],
            TIMEOUT,
        );
    }
    let closing = event(
        &h,
        &s,
        &checkouts,
        2,
        json!({"stop_hook_active":true,"last_assistant_message":"Done.","status":"completed","loop_count":1}),
        if at == Some(3) {
            stopped.as_ref()
        } else {
            None
        },
        (waits == Some(1)).then_some(hold),
    )?;
    for row in closing {
        if row["interrupted"] == true {
            inc(
                &mut out.events,
                &format!("stopped by {}", row["how"].as_str().unwrap_or("")),
            );
        } else if row["exit"] != 0 {
            out.violations
                .push(format!("closing Stop {} exit {}", s.id, row["exit"]));
        }
    }
    if choice < 0.8 && held {
        let find = || -> Result<Option<String>> {
            let ctx = h.context(&cwd, "local")?;
            Ok(infra_core::BrainRepository::new(ctx)?
                .inventory(false)?
                .records
                .iter()
                .find(|r| {
                    r.record["external_id"] == current
                        && r.record["progress"] == format!("Turn {marker}.")
                })
                .and_then(|r| r.record["id"].as_str().map(String::from)))
        };
        let mut id = find()?;
        if id.is_none() && (at == Some(3) || waits == Some(1)) {
            event(
                &h,
                &s,
                &checkouts,
                1,
                json!({"prompt":"And the credit notes?","attachments":[]}),
                None,
                None,
            )?;
            id = find()?;
            if id.is_some() {
                inc(&mut out.events, "held save made by the next prompt");
            }
        }
        if id.is_none() {
            out.violations
                .push(format!("{marker}: held save was not made by its next hook"));
        } else {
            out.saved = Some((current, id, s.id.clone()));
            inc(&mut out.events, "save held");
        }
    }
    out.log
        .push(format!("turn {} {} {} ({marker})", s.id, s.host, s.kind));
    out.rng = Some(rng);
    Ok(out)
}
impl Fuzz {
    /// The memory CLI as a session runs it: the installation's engine -
    /// attached, the one its hooks are bound to, which the session's
    /// variables point at the state - from the session's folder, naming no
    /// root, so the runtime finds the root, the worktree it serves and the
    /// task where the session is, as a person's command does.
    fn memory_cli(&self) -> Vec<String> {
        let engine = if self.h.attached {
            self.h.runtime.clone()
        } else {
            self.h.project.join(if cfg!(windows) {
                "memory-bank/bin/infra-engine.exe"
            } else {
                "memory-bank/bin/infra-engine"
            })
        };
        vec![engine.to_string_lossy().into_owned(), "memory".into()]
    }
    fn note(&mut self, s: String) {
        if self.verbose {
            eprintln!("{s}");
        }
        self.log.push(s);
    }
    fn absorb(&mut self, marker: String, result: Turn) {
        self.violations.extend(result.violations);
        if let Some(saved) = result.saved {
            self.saved.insert(marker, saved);
        }
        for (k, n) in result.events {
            *self.events.entry(k).or_default() += n;
        }
        for note in result.log {
            self.note(note);
        }
        if let Some(s) = result.session {
            self.sessions.insert(s.id.clone(), s);
        }
    }
    fn new_session(&mut self) -> Result<()> {
        let live = self
            .checkouts
            .iter()
            .filter(|(name, p)| {
                name.as_str() != "root" && !self.removed.contains(*name) && p.is_dir()
            })
            .map(|(n, _)| n.clone())
            .collect::<Vec<_>>();
        let mut kinds = vec!["root"];
        if !self.h.attached {
            kinds.push("sub");
        }
        if !live.is_empty() {
            kinds.push("worktree");
        }
        let kind = kinds[self.rng.below(kinds.len())].to_string();
        let checkout = if kind == "worktree" {
            live[self.rng.below(live.len())].clone()
        } else {
            "root".into()
        };
        let sub =
            if kind == "sub" || kind == "worktree" && !self.h.attached && self.rng.random() < 0.3 {
                "src"
            } else {
                ""
            };
        // Installed, Cursor below the root was never checked, and in a
        // worktree without an installation of its own it runs no hooks.
        // Attached, the Harness launches Cursor in the project or in any of
        // its worktrees, with a plugin image built for that folder.
        let hosts = if kind == "root" {
            vec!["claude", "claude", "codex", "cursor"]
        } else if self.h.attached {
            vec!["claude", "codex", "cursor"]
        } else {
            vec!["claude", "codex"]
        };
        let host = hosts[self.rng.below(hosts.len())].to_string();
        let id = format!("s{}", self.sessions.len() + 1);
        let session = Session {
            id: id.clone(),
            host: host.clone(),
            kind: kind.clone(),
            checkout: checkout.clone(),
            launch: checkout,
            sub: sub.into(),
        };
        inc(&mut self.counts, &format!("{host} {kind}"));
        let rows = event(
            &self.h,
            &session,
            &self.checkouts,
            0,
            json!({"source":"startup"}),
            None,
            None,
        )?;
        for row in rows {
            if row["exit"] != 0 {
                self.violations
                    .push(format!("session start {id}: {}", row["exit"]));
            }
        }
        self.note(format!("session {id}: {host} {kind}"));
        self.sessions.insert(id, session);
        Ok(())
    }
    fn next_marker(&mut self, rng: &mut Random) -> String {
        self.counter += 1;
        format!("m{:03}x{:04x}", self.counter, rng.below(65536))
    }
    fn one_turn(&mut self, id: &str) -> Result<()> {
        let session = self
            .sessions
            .get(id)
            .ok_or_else(|| artifact::invalid("session unavailable"))?
            .clone();
        let marker = {
            self.counter += 1;
            format!("m{:03}x{:04x}", self.counter, self.rng.below(65536))
        };
        let mut local = Random::new(0);
        std::mem::swap(&mut local, &mut self.rng);
        let result = turn(
            self.h.clone(),
            session,
            self.checkouts.clone(),
            marker.clone(),
            self.seed.clone(),
            local,
            self.interrupt,
            self.contention,
            false,
        );
        let mut result = result?;
        self.rng = result
            .rng
            .take()
            .ok_or_else(|| artifact::invalid("lifecycle random stream unavailable"))?;
        self.absorb(marker, result);
        Ok(())
    }
    fn burst(&mut self) -> Result<()> {
        let ids = self.sessions.keys().cloned().collect::<Vec<_>>();
        let picked = self.rng.sample(ids.len(), ids.len().min(3));
        // Two Cursor sessions in one folder would share its rule file. An
        // attached Cursor has none: two Harness chats on one folder build
        // and are handed the same plugin image at once.
        let mut cursors = BTreeSet::new();
        let mut threads = vec![];
        for i in picked {
            let s = self.sessions[&ids[i]].clone();
            if place(&self.h, &s, &self.checkouts).is_none()
                || s.host == "cursor"
                    && !self.h.attached
                    && !cursors.insert((s.checkout.clone(), s.sub.clone()))
            {
                continue;
            }
            let mut rng = Random::float(self.rng.random());
            let marker = self.next_marker(&mut rng);
            let h = self.h.clone();
            let checkouts = self.checkouts.clone();
            let seed = self.seed.clone();
            let interrupt = self.interrupt;
            let contention = self.contention;
            threads.push((
                marker.clone(),
                std::thread::spawn(move || {
                    turn(
                        h, s, checkouts, marker, seed, rng, interrupt, contention, true,
                    )
                }),
            ));
        }
        inc(&mut self.events, "burst");
        for (marker, thread) in threads {
            let result = thread
                .join()
                .map_err(|_| artifact::invalid("lifecycle burst panicked"))??;
            self.absorb(marker, result);
        }
        Ok(())
    }
    fn git_action(&mut self) -> Result<()> {
        let live = self
            .checkouts
            .iter()
            .filter(|(n, p)| !self.removed.contains(*n) && p.is_dir())
            .map(|(n, _)| n.clone())
            .collect::<Vec<_>>();
        if live.is_empty() {
            return Ok(());
        }
        let name = &live[self.rng.below(live.len())];
        let folder = self.checkouts[name].clone();
        let stopped = rebase(&folder);
        if !stopped.is_empty() {
            let abort = self.rng.random() < 0.5;
            if abort {
                let _ = git(&folder, &["rebase", "--abort"]);
            } else {
                let _ = git(&folder, &["checkout", "--theirs", "--", "src"]);
                let _ = git(&folder, &["add", "src"]);
                let _ = git(&folder, &["rebase", "--continue"]);
            }
            inc(
                &mut self.events,
                if abort {
                    "rebase aborted"
                } else {
                    "rebase continued"
                },
            );
            return Ok(());
        }
        let current = branch(&folder);
        let weights = if current.is_empty() {
            [10, 10, 0, 0, 80, 0]
        } else {
            [30, 25, 15, 10, 20, 8]
        };
        let total = weights.iter().sum::<i32>() as f64;
        let draw = self.rng.random() * total;
        let mut sum = 0;
        let action = weights
            .iter()
            .position(|w| {
                sum += *w;
                draw < sum as f64
            })
            .unwrap_or(5);
        match action {
            0 => {
                let path = format!("src/Note{}.php", self.counter);
                std::fs::write(folder.join(path), "<?php\n")?;
                let _ = git(&folder, &["add", "src"]);
                let _ = git(
                    &folder,
                    &["commit", "-qm", &format!("Note {}", self.counter)],
                );
            }
            1 => {
                let stem = if self.rng.random() < 0.3 {
                    "feature/ШОП-"
                } else {
                    "feature/SHOP-"
                };
                let _ = git(
                    &folder,
                    &[
                        "checkout",
                        "-q",
                        "-b",
                        &format!("{stem}{}", 100 + self.log.len()),
                    ],
                );
            }
            2 => {
                let _ = git(
                    &folder,
                    &["branch", "-m", &current, &format!("{current}-renamed")],
                );
            }
            3 => {
                let _ = git(&folder, &["checkout", "-q", "--detach"]);
            }
            4 => {
                let busy =
                    git(&self.h.project, &["worktree", "list", "--porcelain"]).unwrap_or_default();
                let free = git(&folder, &["branch"])
                    .unwrap_or_default()
                    .lines()
                    .map(|s| s.trim_start_matches(['*', '+', ' ']).trim().to_string())
                    .filter(|s| {
                        !s.contains("HEAD")
                            && !busy
                                .split_whitespace()
                                .any(|w| w == format!("refs/heads/{s}"))
                    })
                    .collect::<Vec<_>>();
                if !free.is_empty() {
                    let _ = git(
                        &folder,
                        &["checkout", "-q", &free[self.rng.below(free.len())]],
                    );
                }
            }
            _ => {
                let side = format!("{current}-side-{}", self.counter);
                let path = folder.join(format!("src/Conflict{}.php", self.counter));
                if git(&folder, &["checkout", "-q", "-b", &side]).is_ok() {
                    for (body, title) in [("side", "Side"), ("main", "Main")] {
                        std::fs::write(&path, format!("<?php // {body}\n"))?;
                        let _ = git(&folder, &["add", "src"]);
                        let _ = git(&folder, &["commit", "-qm", title]);
                        if body == "side" {
                            let _ = git(&folder, &["checkout", "-q", &current]);
                        }
                    }
                    let _ = git(&folder, &["rebase", &side]);
                    if !rebase(&folder).is_empty() {
                        inc(&mut self.events, "rebase stopped");
                    }
                }
            }
        }
        self.note(format!(
            "git action {action} in {name}: {current} -> {}",
            branch(&folder)
        ));
        Ok(())
    }
    fn worktree_action(&mut self) -> Result<()> {
        let live = self
            .checkouts
            .keys()
            .filter(|n| n.as_str() != "root" && !self.removed.contains(*n))
            .cloned()
            .collect::<Vec<_>>();
        if !live.is_empty() && self.rng.random() < 0.3 {
            let name = live[self.rng.below(live.len())].clone();
            let folder = self.checkouts[&name].clone();
            if git(
                &self.h.project,
                &[
                    "worktree",
                    "remove",
                    "--force",
                    folder
                        .to_str()
                        .ok_or_else(|| artifact::invalid("worktree encoding"))?,
                ],
            )
            .is_ok()
            {
                self.removed.insert(name);
            }
            return Ok(());
        }
        let name = format!("wt{}", self.checkouts.len());
        if !self.h.attached && self.rng.random() < 0.25 {
            let folder = self.h.work.join("codex-worktrees").join(&name).join("shop");
            std::fs::create_dir_all(folder.parent().unwrap())?;
            if git(
                &self.h.project,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "--detach",
                    folder.to_str().unwrap(),
                ],
            )
            .is_ok()
            {
                let admin = git(&folder, &["rev-parse", "--absolute-git-dir"])?;
                let owner = format!(
                    "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                    self.rng.bits(32),
                    self.rng.bits(16),
                    self.rng.bits(16),
                    self.rng.bits(16),
                    self.rng.bits(48)
                );
                artifact::write_json(
                    &PathBuf::from(admin).join("codex-thread.json"),
                    &json!({"version":1,"ownerThreadId":owner}),
                )?;
                self.checkouts.insert(name, folder);
                inc(&mut self.events, "codex worktree made");
            }
            return Ok(());
        }
        let spelled = if self.rng.random() < 0.3 {
            format!("{name} with spaces")
        } else {
            name.clone()
        };
        let folder = if self.rng.random() < 0.5 {
            self.h.project.join(".claude/worktrees").join(spelled)
        } else {
            self.h.work.join(format!("shop-{spelled}"))
        };
        std::fs::create_dir_all(folder.parent().unwrap())?;
        if git(
            &self.h.project,
            &[
                "worktree",
                "add",
                "-q",
                folder.to_str().unwrap(),
                "-b",
                &format!("feature/SHOP-{}", 200 + self.checkouts.len()),
            ],
        )
        .is_ok()
        {
            self.checkouts.insert(name, folder);
        }
        Ok(())
    }
    fn checkpoint(&mut self) -> Result<()> {
        let live = self
            .sessions
            .values()
            .filter(|s| place(&self.h, s, &self.checkouts).is_some())
            .cloned()
            .collect::<Vec<_>>();
        if live.is_empty() {
            return Ok(());
        }
        let s = &live[self.rng.below(live.len())];
        let folder = place(&self.h, s, &self.checkouts).unwrap();
        let task = task_branch(&folder);
        if task.is_empty() || !rebase(&folder).is_empty() {
            return Ok(());
        }
        let env = self.h.session_env(
            &s.host,
            &self.checkouts[&s.launch],
            &folder,
            ["worktree", "sub"].contains(&s.kind.as_str()),
            TIMEOUT,
        )?;
        let mut args = self.memory_cli();
        args.extend([
            "get".into(),
            "--task-id".into(),
            task.clone(),
            "--json".into(),
        ]);
        if rehearsal::run_command(args.clone(), &folder, &env, vec![], TIMEOUT)?["exit"] != 0 {
            return Ok(());
        }
        let locked = if !self.h.attached && self.rng.random() < 0.5 {
            ["project-brain/local", "memory-bank/local"]
                .iter()
                .map(|p| self.h.state.join(p))
                .filter(|p| p.is_dir())
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        let modes = locked
            .iter()
            .map(|p| artifact::mode(p))
            .collect::<Result<Vec<_>>>()?;
        for p in &locked {
            artifact::chmod(p, 0o555)?;
        }
        args.truncate(2);
        args.extend([
            "update".into(),
            "--task-id".into(),
            task,
            "--revision".into(),
            "auto".into(),
            "--progress".into(),
            format!("Checkpoint {}.", self.counter),
            "--json".into(),
        ]);
        let done = rehearsal::run_command(args, &folder, &env, vec![], TIMEOUT);
        for (p, mode) in locked.iter().zip(modes) {
            artifact::chmod(p, mode)?;
        }
        let done = done?;
        if done["exit"] != 0 {
            self.violations
                .push(format!("checkpoint {} exit {}", s.id, done["exit"]));
            return Ok(());
        }
        let queued = artifact::strict(done["stdout"].as_str().unwrap_or("").as_bytes())
            .is_ok_and(|v| v["queued"] == true);
        inc(
            &mut self.events,
            if queued {
                "checkpoint held"
            } else {
                "checkpoint made"
            },
        );
        if queued {
            let before = self.h.journal().len();
            event(
                &self.h,
                s,
                &self.checkouts,
                2,
                json!({"stop_hook_active":false,"last_assistant_message":"Checkpointed."}),
                None,
                None,
            )?;
            event(
                &self.h,
                s,
                &self.checkouts,
                2,
                json!({"stop_hook_active":true,"last_assistant_message":"Done."}),
                None,
                None,
            )?;
            let made = self
                .h
                .journal()
                .into_iter()
                .skip(before)
                .filter(|r| r["stage"] == "queue")
                .map(|r| r["event"].as_str().unwrap_or("").to_string())
                .collect::<Vec<_>>();
            if made != vec!["made"] {
                self.violations
                    .push(format!("checkpoint {} was not made by Stop", s.id));
            }
        }
        Ok(())
    }
    fn run(&mut self, steps: usize) -> Result<()> {
        self.new_session()?;
        for _ in 0..steps {
            if crate::cancel::interrupted() {
                break;
            }
            let roll = self.rng.random();
            if roll < 0.12 && self.sessions.len() > 1 {
                self.burst()?;
            } else if roll < 0.55 {
                let ids = self.sessions.keys().cloned().collect::<Vec<_>>();
                let id = ids[self.rng.below(ids.len())].clone();
                self.one_turn(&id)?;
            } else if roll < 0.65 {
                self.new_session()?;
            } else if roll < 0.8 {
                self.git_action()?;
            } else if roll < 0.89 {
                self.worktree_action()?;
            } else if roll < 0.93 {
                self.checkpoint()?;
            } else {
                let ids = self.sessions.keys().cloned().collect::<Vec<_>>();
                let s = self.sessions[&ids[self.rng.below(ids.len())]].clone();
                let source = if self.rng.below(2) == 0 {
                    "clear"
                } else {
                    "compact"
                };
                event(
                    &self.h,
                    &s,
                    &self.checkouts,
                    0,
                    json!({"source":source}),
                    None,
                    None,
                )?;
            }
        }
        Ok(())
    }
    fn check(&mut self) -> Result<Value> {
        for (name, folder) in &self.checkouts {
            if !self.removed.contains(name) && !rebase(folder).is_empty() {
                let _ = git(folder, &["rebase", "--abort"]);
            }
        }
        let ctx = self.h.context(&self.h.project, "local")?;
        let records = infra_core::BrainRepository::new(ctx.clone())?.inventory(true)?;
        let tasks = records
            .records
            .iter()
            .filter(|r| r.record["type"] == "task")
            .filter_map(|r| {
                Some((
                    r.record["id"].as_str()?.to_string(),
                    r.record["external_id"].as_str()?.to_string(),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        for (marker, (branch, task, session)) in &self.saved {
            if task
                .as_ref()
                .and_then(|id| tasks.get(id))
                .is_none_or(|id| !id.starts_with(branch))
            {
                self.violations.push(format!(
                    "{marker} ({session}): not saved under asked branch {branch}"
                ));
            }
        }
        self.violations.extend(
            records
                .problems
                .iter()
                .map(|p| format!("Brain validation: {}", p.error)),
        );
        self.violations.extend(
            infra_core::BankService::new(ctx.clone())
                .validate()?
                .into_iter()
                .map(|e| format!("Bank validation: {e}")),
        );
        for (name, folder) in &self.checkouts {
            if name == "root" && !self.h.attached || self.removed.contains(name) {
                continue;
            }
            let status = git(
                folder,
                &[
                    "status",
                    "--porcelain",
                    "--ignored",
                    "--untracked-files=all",
                ],
            )
            .unwrap_or_default();
            let written = status
                .lines()
                .filter(|s| {
                    !s.get(3..)
                        .unwrap_or("")
                        .trim_matches('"')
                        .starts_with(".claude/worktrees/")
                })
                .collect::<Vec<_>>();
            if !written.is_empty() {
                self.violations.push(format!(
                    "checkout {name} written into: {}",
                    written.join(", ")
                ));
            }
        }
        if self.h.attached
            && crate::freshness::snapshot_digest(&self.h.source.join(&self.h.edition))?
                != self.clone_before
        {
            self.violations.push("tooling edition written into".into());
        }
        let saved = self.saved.keys().cloned().collect::<Vec<_>>();
        for i in self.rng.sample(saved.len(), saved.len().min(4)) {
            let marker = &saved[i];
            for (name, folder) in &self.checkouts {
                if self.removed.contains(name) {
                    continue;
                }
                let env = self.h.hook_env("claude", folder)?;
                let mut argv = self.memory_cli();
                argv.extend(["search".into(), marker.clone(), "--json".into()]);
                let result = rehearsal::run_command(argv, folder, &env, vec![], TIMEOUT)?;
                if result["exit"] != 0
                    || !result["stdout"]
                        .as_str()
                        .is_some_and(|s| s.contains(marker))
                {
                    self.violations
                        .push(format!("search {marker} from {name} unavailable"));
                }
            }
        }
        let root_task = branch(&self.h.project);
        if !root_task.is_empty() {
            let env = self.h.hook_env("claude", &self.h.project)?;
            let cli = |flush: bool| {
                let mut args = self.memory_cli();
                args.extend(["turn".into(), "--task-id".into(), root_task.clone()]);
                if flush {
                    args.push("--flush".into());
                }
                rehearsal::run_command(args, &self.h.project, &env, vec![], TIMEOUT)
            };
            let _ = cli(false);
            std::fs::write(self.h.project.join("src/Final.php"), "<?php\n")?;
            let _ = git(&self.h.project, &["add", "src"]);
            let _ = git(&self.h.project, &["commit", "-qm", "Final"]);
            let _ = cli(false);
            let _ = cli(true);
            let live = self
                .checkouts
                .iter()
                .filter(|(n, _)| n.as_str() != "root" && !self.removed.contains(*n))
                .filter_map(|(_, p)| infra_core::fs::canonicalize(p).ok())
                .collect::<BTreeSet<_>>();
            let root = self.h.state.join("memory-bank/local/worktrees");
            if let Ok(entries) = std::fs::read_dir(root) {
                for e in entries.flatten() {
                    let marker = e.path().join("worktree");
                    if let Ok(path) = std::fs::read_to_string(marker)
                        && !live.contains(&PathBuf::from(path))
                    {
                        self.violations
                            .push("index of removed worktree remains".into());
                    }
                }
            }
        }
        let rows = self.h.journal();
        let mut over = self
            .events
            .get("stop after the save waited on a lock")
            .copied()
            .unwrap_or(0);
        let mut saves = BTreeMap::<String, u64>::new();
        for row in rows {
            if row["stage"] == "stop-hook" && row["status"] == 124 && over > 0 {
                over -= 1;
                continue;
            }
            if row["event"] == "error"
                || row["stage"] == "stop-hook" && !row["status"].is_null() && row["status"] != 0
                || row["stage"] == "queue" && row["event"] != "made"
            {
                self.violations.push(format!("journal error: {row}"));
            }
            if row["stage"] == "save" {
                inc(
                    &mut saves,
                    row["code"]
                        .as_str()
                        .or_else(|| row["event"].as_str())
                        .unwrap_or("unknown"),
                );
            }
        }
        let hooks = refreshes(&self.h);
        let mut over = self
            .events
            .get("refresh over its budget")
            .copied()
            .unwrap_or(0);
        for code in &hooks {
            if *code == 124 && over > 0 {
                over -= 1;
                continue;
            }
            if ![json!(0), json!(1), json!(4)].contains(code) {
                self.violations
                    .push(format!("prompt hook refresh exited {code}"));
            }
        }
        let degraded = self.events.get("refresh degraded").copied().unwrap_or(0);
        let unseen = self
            .events
            .get("refresh exited 1 without a capsule")
            .copied()
            .unwrap_or(0);
        let unexplained = (hooks.iter().filter(|v| **v == 1).count() as u64).saturating_sub(
            degraded
                + unseen
                + over
                + self
                    .violations
                    .iter()
                    .filter(|s| s.contains("Memory refresh unavailable"))
                    .count() as u64,
        );
        if unexplained > 0 {
            self.violations.push(format!(
                "{unexplained} refreshes exited1 without degraded capsule"
            ));
        }
        Ok(
            json!({"seed":artifact::strict(self.seed.as_bytes())?,"rng_profile":"cpython-mt19937-integer-text-sha512-float53-v2","edition":self.h.edition,"attached":self.h.attached,"sessions":self.counts,"events":self.events,"saves":self.saved.len(),"save_lines":saves,"worktrees":self.checkouts.len().saturating_sub(1),"removed":self.removed.len(),"violations":self.violations,"log_tail":if self.violations.is_empty(){vec![]}else{self.log.iter().rev().take(15).cloned().collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>()},"interrupted":crate::cancel::interrupted()}),
        )
    }
}
pub fn run(source: &Path, runtime: &Path, v: &Value) -> Result<Value> {
    let seed = match &v["seed"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => "1".into(),
    };
    let random = Random::decimal(&seed)?;
    let steps = v["steps"]
        .as_u64()
        .or_else(|| v["steps"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(60) as usize;
    let share = |key: &str| -> Result<f64> {
        let n = v[key]
            .as_f64()
            .or_else(|| v[key].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0.0);
        if !n.is_finite() || !(0.0..=1.0).contains(&n) {
            return Err(artifact::invalid("interrupt/contention share must be0..1"));
        }
        Ok(n)
    };
    let interrupt = share("interrupt")?;
    let contention = share("contention")?;
    let edition = v["edition"].as_str().unwrap_or("PHP Core");
    let attached = v["attached"] == true;
    let (work, temporary) = if let Some(work) = v["work"].as_str() {
        let work = artifact::ordinary(Path::new(work))?;
        if work.exists() && (!work.is_dir() || std::fs::read_dir(&work)?.next().is_some()) {
            return Err(artifact::invalid("--work must be missing or empty"));
        }
        artifact::private_dir(&work)?;
        (work, None)
    } else {
        let scratch = artifact::Scratch::new(&std::env::temp_dir(), "memory-fuzz")?;
        (scratch.0.clone(), Some(scratch))
    };
    let project = work.join("shop");
    std::fs::create_dir_all(project.join("src"))?;
    std::fs::write(project.join("composer.json"), "{\"name\":\"acme/shop\"}\n")?;
    std::fs::write(
        project.join("README.md"),
        "# Shop\n\nThe saffron checkout settles every order.\n",
    )?;
    std::fs::write(
        project.join("src/Invoice.php"),
        "<?php\nfinal class Invoice {}\n",
    )?;
    let env = reconstruction::runtime_env(&work, chrono::Utc::now(), false)?;
    reconstruction::commit_corpus(&project, &env, None, None, "feature/SHOP-1")?;
    if !attached {
        infra_tooling::installer::Installer::new(source)?
            .with_engine(runtime)?
            .install(
                &project,
                edition,
                &["claude".into(), "codex".into(), "cursor".into()],
                &infra_tooling::installer::InstallOptions {
                    merge_existing: true,
                    ..Default::default()
                },
            )?;
    }
    let harness = Harness::new(source, runtime, &project, &work, edition, attached)?;
    let clone_before = if attached {
        crate::freshness::snapshot_digest(&source.join(edition))?
    } else {
        String::new()
    };
    let mut fuzz = Fuzz {
        h: harness,
        seed,
        rng: random,
        checkouts: BTreeMap::from([("root".into(), project)]),
        removed: BTreeSet::new(),
        sessions: BTreeMap::new(),
        saved: BTreeMap::new(),
        counts: BTreeMap::new(),
        events: BTreeMap::new(),
        violations: vec![],
        log: vec![],
        counter: 0,
        interrupt,
        contention,
        verbose: v["verbose"] == true,
        clone_before,
    };
    fuzz.run(steps)?;
    let summary = fuzz.check()?;
    let code = if crate::cancel::interrupted() {
        128 + crate::cancel::signal()
    } else if summary["violations"].as_array().is_some_and(Vec::is_empty) {
        0
    } else {
        1
    };
    let stdout = format!(
        "{}\n",
        infra_core::domain::json::serialize(&summary, false, false, true, false)?
    );
    drop(temporary);
    Ok(json!({"_cli":{"code":code,"stdout":stdout,"stderr":""},"result":summary}))
}
