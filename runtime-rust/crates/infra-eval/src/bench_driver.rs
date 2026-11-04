//! Concrete native offline host driver. The provider and host share a fresh network namespace.
use crate::{
    Result, artifact,
    bench::{self, BenchmarkDriver, Checkpoint, ScriptedProvider},
    reconstruction,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
const KEY: &str = "offline-bench-key-not-a-real-api-key";
const QUESTION: &str = "How are invoices numbered?";
const ROOT_BRANCH: &str = "feature/SHOP-7-invoices";
const WORK_BRANCH: &str = "feature/SHOP-9-refunds";
fn flag(v: &Value, k: &str) -> bool {
    v[k] == true || v[k].as_str() == Some("")
}
fn number(v: &Value, k: &str, d: f64) -> Result<f64> {
    let n = v[k]
        .as_f64()
        .or_else(|| v[k].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(d);
    if !n.is_finite() || n <= 0.0 {
        return Err(artifact::invalid(
            "benchmark duration must be positive and finite",
        ));
    }
    Ok(n)
}
fn named(v: &Value, k: &str, d: &str) -> String {
    v[k].as_str().unwrap_or(d).into()
}
fn text(path: &Path) -> String {
    artifact::read_file(path, 8 * 1024 * 1024)
        .ok()
        .map(|r| String::from_utf8_lossy(&r.0).into_owned())
        .unwrap_or_default()
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}
fn executable(name: &str) -> Result<PathBuf> {
    let p = PathBuf::from(name);
    if p.components().count() > 1 {
        return artifact::ordinary(&p);
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let p = dir.join(name);
        if p.is_file() {
            return artifact::ordinary(&p);
        }
    }
    Err(artifact::invalid(&format!(
        "native executable unavailable: {name}"
    )))
}
pub(crate) fn clean_env() -> BTreeMap<String, String> {
    std::env::vars()
        .filter(|(k, _)| {
            ![
                "GIT_",
                "CONTEXT_",
                "ACCELERATOR_",
                "CLAUDE",
                "CODEX",
                "ANTHROPIC",
            ]
            .iter()
            .any(|p| k.starts_with(p))
        })
        .collect()
}
struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn command(argv: &[String], cwd: &Path, env: &BTreeMap<String, String>) -> Result<Command> {
    let mut c = Command::new(
        argv.first()
            .ok_or_else(|| artifact::invalid("empty command"))?,
    );
    c.args(&argv[1..]).current_dir(cwd).env_clear().envs(env);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            c.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    Ok(c)
}
#[derive(Default)]
pub(crate) struct Done {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub events: Vec<Value>,
    pub asked: Vec<Value>,
    pub threads: Vec<String>,
    pub screen: String,
}
fn bounded<R: Read + Send + 'static>(mut r: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut out = vec![];
        let mut buf = [0; 8192];
        loop {
            match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if out.len() + n <= 8 * 1024 * 1024 {
                        out.extend_from_slice(&buf[..n]);
                    } else {
                        break;
                    }
                }
            }
        }
        out
    })
}
pub(crate) fn capture(
    argv: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<Done> {
    let mut c = command(argv, cwd, env)?;
    c.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Guard(c.spawn()?);
    let out = bounded(
        child
            .0
            .stdout
            .take()
            .ok_or_else(|| artifact::invalid("host stdout unavailable"))?,
    );
    let err = bounded(
        child
            .0
            .stderr
            .take()
            .ok_or_else(|| artifact::invalid("host stderr unavailable"))?,
    );
    let end = Instant::now() + timeout;
    let code = loop {
        if let Some(status) = child.0.try_wait()? {
            break status.code().unwrap_or(-1);
        }
        if Instant::now() >= end || crate::cancel::interrupted() {
            break -1;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.0.id() as i32), libc::SIGKILL);
    }
    let _ = child.0.wait();
    let stdout = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
    let events = stdout
        .lines()
        .filter_map(|l| artifact::strict(l.as_bytes()).ok())
        .collect();
    Ok(Done {
        code,
        stdout,
        stderr,
        events,
        ..Default::default()
    })
}
struct Rpc {
    child: Guard,
    input: ChildStdin,
    lines: mpsc::Receiver<Result<Value>>,
    error: Option<std::thread::JoinHandle<Vec<u8>>>,
    seen: Vec<Value>,
    asked: Vec<Value>,
}
impl Rpc {
    fn start(argv: &[String], cwd: &Path, env: &BTreeMap<String, String>) -> Result<Self> {
        let mut c = command(argv, cwd, env)?;
        c.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = Guard(c.spawn()?);
        let input = child
            .0
            .stdin
            .take()
            .ok_or_else(|| artifact::invalid("RPC stdin unavailable"))?;
        let output = child
            .0
            .stdout
            .take()
            .ok_or_else(|| artifact::invalid("RPC stdout unavailable"))?;
        let error = child.0.stderr.take().map(bounded);
        let (tx, lines) = mpsc::sync_channel(256);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            let mut total = 0;
            loop {
                let mut raw = vec![];
                match reader
                    .by_ref()
                    .take(1024 * 1024 + 1)
                    .read_until(b'\n', &mut raw)
                {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        total += raw.len();
                        if raw.len() > 1024 * 1024 || total > 8 * 1024 * 1024 {
                            let _ = tx.send(Err(artifact::invalid("RPC output bound")));
                            break;
                        }
                        if let Ok(v) = artifact::strict(&raw)
                            && tx.send(Ok(v)).is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });
        Ok(Self {
            child,
            input,
            lines,
            error,
            seen: vec![],
            asked: vec![],
        })
    }
    fn send(&mut self, v: Value) -> Result<()> {
        serde_json::to_writer(&mut self.input, &v)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        Ok(())
    }
    fn next(&mut self, end: Instant) -> Result<Option<Value>> {
        if crate::cancel::interrupted() || Instant::now() >= end {
            return Ok(None);
        }
        match self.lines.recv_timeout(
            Duration::from_millis(100).min(end.saturating_duration_since(Instant::now())),
        ) {
            Ok(v) => {
                let v = v?;
                self.seen.push(v.clone());
                Ok(Some(v))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(artifact::invalid("native RPC stream ended")),
        }
    }
    fn response(&mut self, id: u64, end: Instant) -> Result<Value> {
        while Instant::now() < end {
            if let Some(v) = self.next(end)? {
                if v["id"] == id {
                    if v.get("error").is_some() {
                        return Err(artifact::invalid(&format!(
                            "RPC request failed: {}",
                            v["error"]
                        )));
                    }
                    return Ok(v["result"].clone());
                }
                self.refuse(&v)?;
            }
        }
        Err(artifact::invalid("RPC response deadline"))
    }
    fn refuse(&mut self, v: &Value) -> Result<()> {
        if v.get("id").is_some() && v["method"].is_string() {
            self.asked.push(v.clone());
            let method = v["method"].as_str().unwrap_or("");
            let result = if method.contains("requestApproval") {
                json!({"decision":"decline"})
            } else {
                json!({"decision":"decline","approved":false})
            };
            self.send(json!({"id":v["id"],"result":result}))?;
        }
        Ok(())
    }
    fn finish(mut self) -> Done {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.0.id() as i32), libc::SIGTERM);
        }
        let end = Instant::now() + Duration::from_secs(2);
        let code = loop {
            if let Ok(Some(status)) = self.child.0.try_wait() {
                break status.code().unwrap_or(-1);
            }
            if Instant::now() >= end {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(self.child.0.id() as i32), libc::SIGKILL);
                }
                break self
                    .child
                    .0
                    .wait()
                    .ok()
                    .and_then(|s| s.code())
                    .unwrap_or(-1);
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let stderr = self
            .error
            .take()
            .map(|t| String::from_utf8_lossy(&t.join().unwrap_or_default()).into_owned())
            .unwrap_or_default();
        Done {
            code,
            stdout: self
                .seen
                .iter()
                .filter_map(|v| serde_json::to_string(v).ok())
                .collect::<Vec<_>>()
                .join("\n"),
            events: self.seen,
            asked: self.asked,
            stderr,
            ..Default::default()
        }
    }
}
fn initialize(rpc: &mut Rpc, end: Instant) -> Result<()> {
    rpc.send(json!({"id":0,"method":"initialize","params":{"clientInfo":{"name":"memory-offline-bench","version":"1.0"},"capabilities":{"experimentalApi":true}}}))?;
    rpc.response(0, end)?;
    rpc.send(json!({"method":"initialized"}))
}
#[expect(
    clippy::too_many_arguments,
    reason = "The native trust probe keeps executable arguments, project, environment, trust source, home, approval and timeout separate."
)]
fn trust_with(
    binary: &str,
    arguments: &[String],
    project: &Path,
    env: &BTreeMap<String, String>,
    source: &str,
    home: &Path,
    approve: bool,
    timeout: Duration,
) -> Result<Value> {
    let end = Instant::now() + timeout.min(Duration::from_secs(30));
    let mut argv = vec![binary.into()];
    argv.extend_from_slice(arguments);
    argv.push("app-server".into());
    let mut rpc = Rpc::start(&argv, project, env)?;
    initialize(&mut rpc, end)?;
    rpc.send(json!({"id":1,"method":"hooks/list","params":{"cwds":[project]}}))?;
    let listed = rpc.response(1, end)?;
    let mut entries = vec![];
    fn collect(v: &Value, source: &str, prefix: &str, out: &mut Vec<Value>) {
        if let Some(m) = v.as_object() {
            if let (Some(key), Some(hash)) = (
                m.get("key").and_then(Value::as_str),
                m.get("currentHash").and_then(Value::as_str),
            ) {
                let own = m.get("source").and_then(Value::as_str) == Some(source);
                let command = m.get("command").and_then(Value::as_str).unwrap_or("");
                if own && (source != "user" || command.starts_with(prefix)) {
                    out.push(
                        json!({"key":key,"currentHash":hash,"trustStatus":m.get("trustStatus")}),
                    );
                }
            }
            for value in m.values() {
                collect(value, source, prefix, out);
            }
        } else if let Some(a) = v.as_array() {
            for value in a {
                collect(value, source, prefix, out);
            }
        }
    }
    let prefix = format!(
        "{} ",
        shell_words::quote(
            &home
                .join("ai-accelerator/ai-accelerator-dispatch.sh")
                .to_string_lossy()
        )
    );
    collect(&listed, source, &prefix, &mut entries);
    let before = entries
        .iter()
        .filter(|v| v["trustStatus"] == "trusted")
        .cloned()
        .collect::<Vec<_>>();
    if approve && !entries.is_empty() {
        let mut values = json!({});
        for e in &entries {
            values[e["key"].as_str().unwrap()] = json!({"trusted_hash":e["currentHash"]});
        }
        rpc.send(json!({"id":2,"method":"config/batchWrite","params":{"edits":[{"keyPath":"hooks.state","value":values,"mergeStrategy":"upsert"}]}}))?;
        rpc.response(2, end)?;
    }
    let _ = rpc.finish();
    Ok(
        json!({"listed":true,"entries":entries,"approvals_before":before,"approved":if approve{entries.len()}else{0}}),
    )
}
fn trust(
    binary: &str,
    project: &Path,
    env: &BTreeMap<String, String>,
    source: &str,
    home: &Path,
    approve: bool,
    timeout: Duration,
) -> Result<Value> {
    trust_with(binary, &[], project, env, source, home, approve, timeout)
}
fn app_server(
    binary: &str,
    start: &Path,
    env: &BTreeMap<String, String>,
    sandbox: &str,
    timeout: Duration,
    owner: Option<&Path>,
) -> Result<Done> {
    let end = Instant::now() + timeout;
    let mut rpc = Rpc::start(&[binary.into(), "app-server".into()], start, env)?;
    initialize(&mut rpc, end)?;
    rpc.send(json!({"id":1,"method":"thread/start","params":{"cwd":start,"approvalPolicy":"on-request","sandbox":sandbox}}))?;
    let result = rpc.response(1, end)?;
    let thread = result["thread"]["id"]
        .as_str()
        .ok_or_else(|| artifact::invalid("thread/start named no thread"))?
        .to_string();
    if let Some(worktree) = owner {
        let admin = reconstruction::git(worktree, &["rev-parse", "--absolute-git-dir"])?;
        artifact::write_json(
            &PathBuf::from(admin).join("codex-thread.json"),
            &json!({"version":1,"ownerThreadId":thread}),
        )?;
    }
    rpc.send(json!({"id":2,"method":"turn/start","params":{"threadId":thread,"input":[{"type":"text","text":QUESTION}]}}))?;
    let mut completed = false;
    while Instant::now() < end && !crate::cancel::interrupted() {
        if let Some(v) = rpc.next(end)? {
            rpc.refuse(&v)?;
            if v["method"] == "turn/completed" {
                completed = true;
                break;
            }
        }
    }
    if completed {
        std::thread::sleep(Duration::from_millis(300));
    }
    let mut done = rpc.finish();
    done.code = if completed { 0 } else { -1 };
    done.threads.push(thread);
    Ok(done)
}
fn desktop(
    argv: &[String],
    start: &Path,
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<Done> {
    let end = Instant::now() + timeout;
    let mut command = argv.to_vec();
    command.extend(
        [
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
            "--permission-prompt-tool=stdio",
            "--setting-sources=user,project,local",
            "--await-initialize",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    let mut rpc = Rpc::start(&command, start, env)?;
    rpc.send(json!({"type":"control_request","request_id":"bench-init","request":{"subtype":"initialize"}}))?;
    rpc.send(json!({"type":"user","message":{"role":"user","content":QUESTION}}))?;
    let mut result = false;
    while Instant::now() < end && !crate::cancel::interrupted() {
        if let Some(v) = rpc.next(end)? {
            if v["type"] == "control_request" {
                let deny = v["request"]["subtype"] == "can_use_tool";
                if deny {
                    rpc.asked.push(v["request"].clone());
                }
                rpc.send(json!({"type":"control_response","response":{"subtype":"success","request_id":v["request_id"],"response":if deny{json!({"behavior":"deny","message":"offline bench declines all desktop permission prompts"})}else{json!({})}}}))?;
            }
            if v["type"] == "result" {
                result = true;
                break;
            }
        }
    }
    let mut done = rpc.finish();
    done.code = if result { 0 } else { -1 };
    Ok(done)
}
#[cfg(unix)]
fn terminal(
    argv: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    provider: &ScriptedProvider,
    journal: &Path,
    timeout: Duration,
    settle: f64,
) -> Result<Done> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::process::CommandExt;
    let (mut master, mut slave) = (0, 0);
    let size = libc::winsize {
        ws_row: 50,
        ws_col: 160,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &size,
        )
    } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut master = unsafe { std::fs::File::from_raw_fd(master) };
    let slave = unsafe { std::fs::File::from_raw_fd(slave) };
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..])
        .current_dir(cwd)
        .env_clear()
        .envs(env)
        .env("TERM", "xterm-256color")
        .env("COLUMNS", "160")
        .env("LINES", "50")
        .stdin(slave.try_clone()?)
        .stdout(slave.try_clone()?)
        .stderr(slave.try_clone()?);
    let fd = slave.as_raw_fd();
    unsafe {
        c.pre_exec(move || {
            if libc::setsid() < 0 || libc::ioctl(fd, libc::TIOCSCTTY, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = Guard(c.spawn()?);
    drop(slave);
    unsafe {
        libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
    }
    let end = Instant::now() + timeout;
    let mut screen = vec![];
    fn pump(file: &mut std::fs::File, screen: &mut Vec<u8>, until: Instant, end: Instant) {
        while Instant::now() < until.min(end) && !crate::cancel::interrupted() {
            let mut buf = [0; 8192];
            match file.read(&mut buf) {
                Ok(n) if n > 0 => {
                    if screen.len() + n <= 8 * 1024 * 1024 {
                        screen.extend_from_slice(&buf[..n]);
                    }
                }
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
    fn type_line(file: &mut std::fs::File, line: &str) -> Result<()> {
        for b in line.as_bytes() {
            file.write_all(&[*b])?;
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(300));
        file.write_all(b"\r")?;
        Ok(())
    }
    pump(
        &mut master,
        &mut screen,
        Instant::now() + Duration::from_secs_f64(settle),
        end,
    );
    for _ in 0..3 {
        let before = provider.requests().len();
        type_line(&mut master, QUESTION)?;
        let retry = (Instant::now() + Duration::from_secs(15)).min(end);
        while provider.requests().len() == before && Instant::now() < retry {
            pump(
                &mut master,
                &mut screen,
                Instant::now() + Duration::from_millis(50),
                end,
            );
        }
        if provider.requests().len() > before {
            break;
        }
    }
    while !provider.save_begun() && Instant::now() < end && !crate::cancel::interrupted() {
        pump(
            &mut master,
            &mut screen,
            Instant::now() + Duration::from_millis(50),
            end,
        );
    }
    if provider.save_begun() {
        pump(
            &mut master,
            &mut screen,
            Instant::now() + Duration::from_secs(1),
            end,
        );
        type_line(&mut master, "What about credit notes?")?;
    }
    while !text(journal).contains("recovery-saved")
        && Instant::now() < end
        && !crate::cancel::interrupted()
    {
        pump(
            &mut master,
            &mut screen,
            Instant::now() + Duration::from_millis(50),
            end,
        );
    }
    pump(
        &mut master,
        &mut screen,
        Instant::now() + Duration::from_secs(2),
        end,
    );
    let _ = master.write_all(b"\x03\x03");
    pump(
        &mut master,
        &mut screen,
        Instant::now() + Duration::from_secs(1),
        end,
    );
    unsafe {
        libc::kill(-(child.0.id() as i32), libc::SIGTERM);
    }
    std::thread::sleep(Duration::from_millis(100));
    unsafe {
        libc::kill(-(child.0.id() as i32), libc::SIGKILL);
    }
    let _ = child.0.wait();
    let screen = String::from_utf8_lossy(&screen);
    let ansi=regex::Regex::new(r"\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][0-9A-Za-z]|\x1b[=>78]").map_err(|_|artifact::invalid("terminal expression"))?;
    Ok(Done {
        code: 0,
        screen: ansi.replace_all(&screen, "").into_owned(),
        ..Default::default()
    })
}
#[cfg(not(unix))]
fn terminal(
    _: &[String],
    _: &Path,
    _: &BTreeMap<String, String>,
    _: &ScriptedProvider,
    _: &Path,
    _: Duration,
    _: f64,
) -> Result<Done> {
    Err(artifact::invalid("offline PTY benchmark needs Linux"))
}
fn read_journal(path: &Path, first: usize) -> Vec<Value> {
    text(path)
        .lines()
        .skip(first)
        .filter_map(|s| artifact::strict(s.as_bytes()).ok())
        .collect()
}
fn events(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .filter(|v| ["prompt", "stop", "save"].contains(&v["stage"].as_str().unwrap_or("")))
        .map(|v| {
            format!(
                "{}:{}{}",
                v["stage"].as_str().unwrap_or(""),
                v["event"].as_str().unwrap_or(""),
                v["code"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| format!(":{s}"))
                    .unwrap_or_default()
            )
        })
        .collect()
}
fn policy_heard(policy: &str, requests: &[Value]) -> bool {
    let comments = regex::Regex::new(r"(?s)<!--.*?-->").ok();
    let cleaned = comments
        .as_ref()
        .map(|r| r.replace_all(policy, "").into_owned())
        .unwrap_or_else(|| policy.into());
    let heard = requests
        .iter()
        .map(|r| {
            format!(
                "{}\n{}",
                r["system"].as_str().unwrap_or(""),
                r["since"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    cleaned
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .all(|s| heard.contains(s))
        && !heard
            .lines()
            .any(|l| l.contains("Warning: truncated output") && l.contains("AGENTS"))
}
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|m| m.is_dir()) {
                walk(&p, out);
            } else if e.file_type().is_ok_and(|m| m.is_file()) {
                out.push(p);
            }
        }
    }
}
fn cleanup_helpers(home: &Path) -> usize {
    #[cfg(target_os = "linux")]
    {
        let marker = format!("CODEX_HOME={}", home.display()).into_bytes();
        let mut killed = BTreeSet::new();
        for _ in 0..2 {
            if let Ok(entries) = std::fs::read_dir("/proc") {
                for e in entries.flatten() {
                    let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() else {
                        continue;
                    };
                    if pid <= 1
                        || pid == std::process::id() as i32
                        || pid == unsafe { libc::getppid() }
                    {
                        continue;
                    }
                    if let Ok(raw) = std::fs::read(e.path().join("environ"))
                        && raw.split(|b| *b == 0).any(|s| s == marker)
                    {
                        unsafe {
                            libc::kill(pid, libc::SIGTERM);
                        }
                        killed.insert(pid);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        for pid in &killed {
            unsafe {
                libc::kill(*pid, libc::SIGKILL);
            }
        }
        killed.len()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = home;
        0
    }
}
pub(crate) fn copy_tree(source: &Path, target: &Path, excluded: &BTreeSet<String>) -> Result<()> {
    fn walk(root: &Path, source: &Path, target: &Path, excluded: &BTreeSet<String>) -> Result<()> {
        std::fs::create_dir_all(target)?;
        for e in std::fs::read_dir(source)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| artifact::invalid("copy root"))?
                .to_string_lossy()
                .replace('\\', "/");
            if excluded.contains(&name) || excluded.contains(&relative) {
                continue;
            }
            let to = target.join(&name);
            let m = e.file_type()?;
            if m.is_symlink() {
                let Ok(resolved) = infra_core::fs::canonicalize(&path) else {
                    continue;
                };
                if !resolved.starts_with(root) {
                    continue;
                }
                let mapped = target
                    .ancestors()
                    .nth(
                        source
                            .strip_prefix(root)
                            .map_err(|_| artifact::invalid("copy root"))?
                            .components()
                            .count(),
                    )
                    .ok_or_else(|| artifact::invalid("copy target"))?
                    .join(
                        resolved
                            .strip_prefix(root)
                            .map_err(|_| artifact::invalid("link root"))?,
                    );
                #[cfg(not(unix))]
                let _ = &mapped;
                #[cfg(unix)]
                std::os::unix::fs::symlink(mapped, &to)?;
                #[cfg(not(unix))]
                return Err(artifact::invalid("target-copy symlink unsupported"));
            } else if m.is_dir() {
                walk(root, &path, &to, excluded)?;
            } else if m.is_file() {
                std::fs::copy(path, &to)?;
            } else {
                return Err(artifact::invalid("target-copy special file"));
            }
        }
        Ok(())
    }
    walk(source, source, target, excluded)?;
    let registry = target.join(".git/worktrees");
    if registry.is_dir() {
        std::fs::remove_dir_all(registry)?;
    }
    Ok(())
}
pub(crate) fn source_in(project: &Path) -> Result<String> {
    let names = reconstruction::git(project, &["ls-files"])?
        .lines()
        .filter(|name| {
            ![
                ".agents/",
                ".claude/",
                ".codex/",
                ".cursor/",
                "memory-bank/",
                "project-brain/",
                "specs/",
                "tasks/",
            ]
            .iter()
            .any(|p| name.starts_with(p))
                && !format!("/{name}").contains("/.")
                && artifact::read_file(&project.join(name), artifact::MAX_FILE).is_ok()
                && Path::new(name).extension().is_some_and(|s| {
                    [
                        "php", "json", "ts", "js", "py", "yaml", "yml", "xml", "twig", "vue", "go",
                        "rs",
                    ]
                    .iter()
                    .any(|x| s == *x)
                })
        })
        .map(String::from)
        .collect::<Vec<_>>();
    for preferred in ["composer.json", "package.json"] {
        if names.iter().any(|s| s == preferred) {
            return Ok(preferred.into());
        }
    }
    names
        .first()
        .cloned()
        .ok_or_else(|| artifact::invalid("no cited tracked source in target copy"))
}
fn validate(host: &str, v: &Value) -> Result<()> {
    let attached =
        v.get("attached").is_some() && v["attached"] != false && v["attached"] != Value::Null;
    let queued = v.get("queued").is_some();
    let sub = v["subfolder"].as_str();
    let work = v.get("worktree").is_some() && v["worktree"] != false;
    let cli = v["cli"].as_str();
    let mode = named(v, "mode", "default");
    let sandbox = named(v, "sandbox", "workspace-write");
    if let Some(sub) = sub
        && artifact::relative(sub).is_err()
    {
        return Err(artifact::invalid("--subfolder must be inside project"));
    }
    if attached && v["target"].is_string() {
        return Err(artifact::invalid("--attached takes no --target"));
    }
    if v["sync"] == true && !v["target"].is_string() {
        return Err(artifact::invalid("--sync needs --target"));
    }
    if host == "codex" {
        if !["workspace-write", "read-only"].contains(&sandbox.as_str()) {
            return Err(artifact::invalid("invalid Codex sandbox"));
        }
        if queued && attached
            || ((sub.is_some() || work) && (attached || sandbox != "workspace-write"))
            || v["worktree"] == "codex" && queued
            || cli.is_some() && (queued || attached || sandbox != "workspace-write")
            || flag(v, "app_server") && (queued || attached)
        {
            return Err(artifact::invalid("incompatible Codex benchmark options"));
        }
        if work
            && !["inside", "beside", "codex"].contains(&v["worktree"].as_str().unwrap_or("inside"))
        {
            return Err(artifact::invalid("invalid worktree variant"));
        }
        if cli.is_some_and(|c| c != "checkpoint") {
            return Err(artifact::invalid("Codex --cli is checkpoint"));
        }
    } else {
        if ![
            "default",
            "acceptEdits",
            "bypassPermissions",
            "plan",
            "dontAsk",
            "auto",
        ]
        .contains(&mode.as_str())
        {
            return Err(artifact::invalid("invalid Claude permission mode"));
        }
        if queued && mode == "plan"
            || flag(v, "permissions") && v["target"].is_string()
            || flag(v, "sandbox") && queued
            || flag(v, "desktop") && (queued || flag(v, "permissions"))
            || flag(v, "enter_worktree") && (work || sub.is_some() || queued || mode == "plan")
            || cli == Some("retrieve") && !(work || sub.is_some())
            || cli == Some("checkpoint")
                && (queued || flag(v, "permissions") || attached || mode == "plan")
            || work && (flag(v, "permissions") || v["target"].is_string())
            || (sub.is_some() || work) && (queued || attached || mode == "plan")
        {
            return Err(artifact::invalid("incompatible Claude benchmark options"));
        }
    }
    if queued {
        number(v, "queued", 0.0)?;
    }
    Ok(())
}
fn benchmark_scratch(base: &Path, host: &str) -> Result<artifact::Scratch> {
    use infra_core::bank::publication::{PublicationFiles, private::RootProtection};
    let base = artifact::ordinary(base)?;
    let owner = PublicationFiles::open_directory_root(&base, RootProtection::CreatedDirectories)?;
    owner.check()?;
    let work = artifact::Scratch::new(&base, &format!("{host}-bench"))?;
    owner.check()?;
    Ok(work)
}

pub fn run(source: &Path, runtime: &Path, host: &str, request: &Value) -> Result<Value> {
    validate(host, request)?;
    if !cfg!(target_os = "linux") {
        return Err(artifact::invalid(
            "offline benchmark needs Linux user and network namespaces",
        ));
    }
    let unshare = executable("unshare")?;
    executable("ip")?;
    let binary = executable(&named(request, host, host))?;
    let mut request = request.clone();
    if flag(&request, "permissions")
        && request["subfolder"].is_null()
        && !flag(&request, "enter_worktree")
    {
        request["subfolder"] = "src".into();
    }
    let disk = request["subfolder"].is_string() || request.get("worktree").is_some();
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let base = if disk {
        home.join(".cache/ai-infrastructure-benches")
    } else if flag(&request, "sandbox") || flag(&request, "enter_worktree") {
        PathBuf::from("/var/tmp")
    } else if Path::new("/dev/shm").is_dir() {
        PathBuf::from("/dev/shm")
    } else {
        std::env::temp_dir()
    };
    let work = benchmark_scratch(&base, host)?;
    let project = work.0.join("shop");
    let edition = named(&request, "edition", "PHP Core");
    let installer = infra_tooling::installer::Installer::new(source)?.with_engine(runtime)?;
    let tools = ["claude", "codex", "cursor"]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let attached = request.get("attached").is_some()
        && request["attached"] != false
        && request["attached"] != Value::Null;
    if let Some(target) = request["target"].as_str() {
        let target = artifact::ordinary(Path::new(target))?;
        if artifact::overlaps(source, &target)?
            || target.join(".git").is_file()
            || !target.join(".git").is_dir()
        {
            return Err(artifact::invalid(
                "--target must be installed standalone project outside clone",
            ));
        }
        if installer.detect_edition(&target)?.is_none() {
            return Err(artifact::invalid("target edition unavailable"));
        }
        let mut excluded = BTreeSet::from([
            "node_modules".into(),
            "vendor".into(),
            "var".into(),
            ".worktrees".into(),
            ".claude/worktrees".into(),
        ]);
        for e in request["exclude"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            artifact::relative(e)?;
            excluded.insert(e.trim_matches('/').into());
        }
        copy_tree(&target, &project, &excluded)?;
        if flag(&request, "sync") {
            installer.sync(&project, &Default::default())?;
        }
    } else {
        write(
            &project.join("src/Invoice.php"),
            b"<?php\nfinal class Invoice {}\n",
        )?;
        write(&project.join("composer.json"), br#"{"name":"acme/shop"}"#)?;
        write(
            &project.join("CLAUDE.md"),
            b"# Shop\n\nThis is the shop project.\n",
        )?;
        let stamp = chrono::Utc::now();
        let env = reconstruction::runtime_env(&work.0, stamp, false)?;
        reconstruction::commit_corpus(&project, &env, None, None, ROOT_BRANCH)?;
        if !attached {
            installer.install(&project, &edition, &tools, &Default::default())?;
        }
    }
    request["work"] = json!(work.0);
    request["project"] = json!(project);
    request["source"] = source_in(&project)?.into();
    request["tooling_root"] = json!(source);
    request["runtime_binary"] = json!(runtime);
    request["binary"] = json!(binary);
    request["host"] = host.into();
    request["original_packages"] = json!(home.join(".codex/packages"));
    request["attached_enabled"] = attached.into();
    let config = work.0.join("inside.json");
    artifact::write_json(&config, &request)?;
    let mut args = vec![unshare.to_string_lossy().into_owned()];
    let env = clean_env();
    if host == "claude" {
        let help = capture(
            &[unshare.to_string_lossy().into(), "--help".into()],
            &work.0,
            &env,
            Duration::from_secs(5),
        )?;
        if help.stdout.contains("--map-user") {
            args.extend([
                "-n".into(),
                "--user".into(),
                format!("--map-user={}", namespace_uid()),
                format!("--map-group={}", namespace_gid()),
                "--keep-caps".into(),
            ]);
        } else {
            args.push("-rn".into());
        }
    } else {
        args.push("-rn".into());
    }
    args.extend([
        runtime.to_string_lossy().into(),
        "__bench-inside".into(),
        config.to_string_lossy().into(),
    ]);
    let timeout = Duration::from_secs_f64(
        number(
            &request,
            "timeout",
            if host == "codex" { 240.0 } else { 180.0 },
        )? + 120.0,
    );
    let done = capture(&args, &work.0, &env, timeout)?;
    let mut result = artifact::strict(done.stdout.as_bytes()).map_err(|_| {
        artifact::invalid(&format!(
            "namespace worker failed ({}): {}",
            done.code,
            done.stderr
                .chars()
                .rev()
                .take(400)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        ))
    })?;
    let code = if crate::cancel::interrupted() {
        128 + crate::cancel::signal()
    } else if result["ok"] == true {
        0
    } else {
        1
    };
    if flag(&request, "keep") {
        result["work"] = json!(work.0);
        std::mem::forget(work);
    }
    let stdout = format!(
        "{}\n",
        infra_core::domain::json::serialize(&result, false, false, true, false)?
    );
    Ok(json!({"_cli":{"code":code,"stdout":stdout,"stderr":""},"result":result}))
}
pub struct NativeDriver {
    pub source: PathBuf,
    pub work: PathBuf,
    pub project: PathBuf,
}
impl BenchmarkDriver for NativeDriver {
    fn run(&mut self, host: &str, v: &Value, provider: &ScriptedProvider) -> Result<Value> {
        self.inside(host, v, provider)
    }
}
impl NativeDriver {
    fn inside(&mut self, host: &str, v: &Value, provider: &ScriptedProvider) -> Result<Value> {
        let timeout = Duration::from_secs_f64(number(
            v,
            "timeout",
            if host == "codex" { 240.0 } else { 180.0 },
        )?);
        let binary = named(v, "binary", host);
        let home = self.work.join("home");
        let codex_home = self.work.join("codex-home");
        for p in [&home, &codex_home, &self.work.join("tmp")] {
            artifact::private_dir(p)?;
        }
        let mut env = clean_env();
        env.insert("HOME".into(), home.to_string_lossy().into());
        env.insert(
            "TMPDIR".into(),
            self.work.join("tmp").to_string_lossy().into(),
        );
        let attached = v["attached_enabled"] == true;
        let edition = named(v, "edition", "PHP Core");
        let mut state = self.project.clone();
        let mut lent = vec![];
        let mut hook_trust = Value::Null;
        let overlay = if attached {
            let path = infra_tooling::attach::state_directory(
                &self.project,
                Some(&self.work.join("attached")),
            )?;
            state = path.clone();
            let o = infra_tooling::attach::build_overlay(
                &self.source,
                &self.project,
                &path,
                &edition,
                host,
            )?;
            env.extend(o.environment.clone());
            Some(o)
        } else {
            None
        };
        if host == "codex" {
            env.insert("CODEX_HOME".into(), codex_home.to_string_lossy().into());
            env.insert("BENCH_API_KEY".into(), KEY.into());
            let sandbox = named(v, "sandbox", "workspace-write");
            let project = serde_json::to_string(&self.project.to_string_lossy())?;
            let config = format!(
                "model=\"gpt-5.1-codex\"\nmodel_provider=\"bench\"\napproval_policy=\"never\"\nsandbox_mode={}\n[model_providers.bench]\nname=\"bench\"\nbase_url={}\nwire_api=\"responses\"\nenv_key=\"BENCH_API_KEY\"\n[projects.{project}]\ntrust_level=\"trusted\"\n",
                serde_json::to_string(&sandbox)?,
                serde_json::to_string(&format!("{}/v1", provider.address))?
            );
            write(&codex_home.join("config.toml"), config.as_bytes())?;
            hook_trust = if let Some(o) = &overlay {
                trust_with(
                    &binary,
                    &o.arguments,
                    &self.project,
                    &env,
                    "sessionFlags",
                    &codex_home,
                    true,
                    timeout,
                )?
            } else {
                trust(
                    &binary,
                    &self.project,
                    &env,
                    "project",
                    &codex_home,
                    true,
                    timeout,
                )?
            };
        } else {
            env.extend([
                ("ANTHROPIC_BASE_URL".into(), provider.address.clone()),
                ("ANTHROPIC_API_KEY".into(), KEY.into()),
                (
                    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
                    "1".into(),
                ),
                ("DISABLE_AUTOUPDATER".into(), "1".into()),
                ("DISABLE_TELEMETRY".into(), "1".into()),
                ("SHELL".into(), named(v, "shell", "/bin/bash")),
            ]);
            let mut projects = json!({});
            projects[self.project.to_string_lossy().as_ref()] =
                json!({"hasTrustDialogAccepted":!attached,"hasCompletedProjectOnboarding":true});
            artifact::write_json(
                &home.join(".claude.json"),
                &json!({"hasCompletedOnboarding":true,"theme":"dark","projects":projects,"customApiKeyResponses":{"approved":[&KEY[KEY.len()-20..]],"rejected":[]}}),
            )?;
            if flag(v, "sandbox") {
                executable("bwrap")?;
                executable("setpriv")?;
                artifact::write_json(
                    &home.join(".claude/settings.json"),
                    &json!({"sandbox":{"enabled":true}}),
                )?;
            }
            if let Some(o) = &overlay {
                let settings = infra_tooling::attach::merge_claude(&json!({}), &o.settings)?;
                lent.extend(["--settings".into(), serde_json::to_string(&settings)?]);
                lent.extend(o.arguments.clone());
            }
        }
        let journal = state.join("memory-bank/local/native-results.ndjson");
        let mut place = self.project.clone();
        let mut setup = Value::Null;
        let worktree = v.get("worktree").is_some() && v["worktree"] != false;
        let managed = v["worktree"] == "codex";
        let sub = v["subfolder"].as_str();
        let preliminary = sub.is_some() || worktree || flag(v, "enter_worktree");
        let policy_cli = if attached {
            format!(
                "{} --root {}",
                shell_words::quote(&named(v, "runtime_binary", "infra")),
                shell_words::quote(&self.project.to_string_lossy())
            )
        } else {
            format!(
                "{}",
                shell_words::quote(
                    &self
                        .project
                        .join("memory-bank/bin/infra-engine")
                        .to_string_lossy()
                )
            )
        };
        provider.configure(json!({"policy_cli":policy_cli}), None)?;
        let mut base = if host == "claude" && flag(v, "sandbox") {
            vec![
                "setpriv".into(),
                "--inh-caps=-all".into(),
                "--ambient-caps=-all".into(),
                binary.clone(),
            ]
        } else {
            vec![binary.clone()]
        };
        let mode = named(v, "mode", "default");
        if host == "claude" && (mode != "default" || flag(v, "desktop")) {
            base.extend(["--permission-mode".into(), mode.clone()]);
        }
        base.extend(lent.clone());
        if preliminary {
            let args = if host == "codex" {
                let mut a = vec![
                    binary.clone(),
                    "exec".into(),
                    "-C".into(),
                    self.project.to_string_lossy().into(),
                    "--skip-git-repo-check".into(),
                    "--sandbox".into(),
                    "workspace-write".into(),
                    QUESTION.into(),
                ];
                a.extend(lent.clone());
                a
            } else {
                let mut a = base.clone();
                a.extend([
                    "-p".into(),
                    QUESTION.into(),
                    "--output-format".into(),
                    "stream-json".into(),
                    "--verbose".into(),
                ]);
                a
            };
            let args = if host == "codex" {
                if let Some(o) = &overlay {
                    infra_tooling::attach::apply_overlay(host, &args, o, true)?
                } else {
                    args
                }
            } else {
                args
            };
            capture(&args, &self.project, &env, timeout)?;
            let registry = if host == "codex" {
                codex_home.join("ai-accelerator/projects")
            } else {
                home.join(".claude/ai-accelerator/projects")
            };
            let until = Instant::now() + Duration::from_secs(15);
            while !text(&registry)
                .lines()
                .any(|p| p == self.project.to_string_lossy())
                && Instant::now() < until
                && !crate::cancel::interrupted()
            {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        if worktree {
            if host == "codex" && managed {
                write(
                    &self.project.join("src/Refund.php"),
                    b"<?php\nfinal class Refund {}\n",
                )?;
                reconstruction::git(&self.project, &["add", "src/Refund.php"])?;
                reconstruction::git(
                    &self.project,
                    &[
                        "-c",
                        "user.name=Bench",
                        "-c",
                        "user.email=bench@example.invalid",
                        "commit",
                        "-q",
                        "-m",
                        "Refunds",
                    ],
                )?;
                std::fs::remove_file(self.project.join("src/Refund.php"))?;
                place = if flag(v, "app_server") {
                    self.work.join("codex-home/worktrees/a7e1/shop")
                } else {
                    self.project.clone()
                };
                if flag(v, "app_server") {
                    if let Some(p) = place.parent() {
                        std::fs::create_dir_all(p)?;
                        write(&p.join(".codex-worktree-name"), b"")?;
                    }
                    reconstruction::git(
                        &self.project,
                        &[
                            "worktree",
                            "add",
                            "-q",
                            "--detach",
                            place
                                .to_str()
                                .ok_or_else(|| artifact::invalid("worktree encoding"))?,
                            "HEAD",
                        ],
                    )?;
                }
            } else {
                place = if host == "codex" && v["worktree"] == "beside" {
                    self.work.join("codex-home/worktrees/b3c4/shop")
                } else {
                    self.project.join(".claude/worktrees/bench")
                };
                if let Some(p) = place.parent() {
                    std::fs::create_dir_all(p)?;
                }
                reconstruction::git(
                    &self.project,
                    &[
                        "worktree",
                        "add",
                        "-q",
                        "-b",
                        WORK_BRANCH,
                        place
                            .to_str()
                            .ok_or_else(|| artifact::invalid("worktree encoding"))?,
                    ],
                )?;
                if host == "codex" {
                    write(
                        &place.join("src/Refund.php"),
                        b"<?php\nfinal class Refund {}\n",
                    )?;
                    reconstruction::git(&place, &["add", "src/Refund.php"])?;
                    reconstruction::git(
                        &place,
                        &[
                            "-c",
                            "user.name=Bench",
                            "-c",
                            "user.email=bench@example.invalid",
                            "commit",
                            "-q",
                            "-m",
                            "Worktree refund source",
                        ],
                    )?;
                }
            }
            if host == "codex" {
                let inspection = trust(&binary, &place, &env, "user", &codex_home, false, timeout)?;
                let first = text(&journal).lines().count();
                provider.reset()?;
                provider.configure(json!({"source":"src/Refund.php","read":if let Some(sub)=sub{format!("{}src/Refund.php","../".repeat(Path::new(sub).components().count()))}else{"src/Refund.php".into()},"policy_cli":policy_cli}),None)?;
                let unapproved = if managed {
                    self.project.join(sub.unwrap_or(""))
                } else {
                    place.join(sub.unwrap_or(""))
                };
                std::fs::create_dir_all(&unapproved)?;
                let mut args = vec![
                    binary.clone(),
                    "exec".into(),
                    "-C".into(),
                    unapproved.to_string_lossy().into(),
                ];
                if managed {
                    args.push("--worktree".into());
                }
                args.extend([
                    "--skip-git-repo-check".into(),
                    "--sandbox".into(),
                    "workspace-write".into(),
                    QUESTION.into(),
                ]);
                capture(&args, &unapproved, &env, timeout)?;
                let requests = provider.requests();
                let heard = requests
                    .iter()
                    .map(|r| r["since"].as_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("\n");
                let unapproved_rows = read_journal(&journal, first);
                let approved = trust(&binary, &place, &env, "user", &codex_home, true, timeout)?;
                setup = json!({"listed":text(&codex_home.join("ai-accelerator/projects")).lines().any(|p|p==self.project.to_string_lossy()),"entries":inspection["entries"],"approvals_before":inspection["approvals_before"],"approved":approved["approved"],"unapproved":{"hook_lines":unapproved_rows.len(),"events":events(&unapproved_rows),"capsule":heard.contains("working:"),"save":heard.contains(bench::MARKER)}});
            }
        }
        let mut first = text(&journal).lines().count();
        provider.reset()?;
        let task = if flag(v, "enter_worktree") {
            "worktree-entered".to_string()
        } else if worktree && !managed {
            WORK_BRANCH.into()
        } else {
            reconstruction::git(&place, &["branch", "--show-current"])
                .unwrap_or_else(|_| ROOT_BRANCH.into())
        };
        let checkpoint = if v["cli"] == "checkpoint" {
            let p = if host == "codex" {
                ".agents/skills/checkpoint/SKILL.md"
            } else {
                ".claude/skills/checkpoint/SKILL.md"
            };
            Some(Checkpoint::new(text(&self.project.join(p))))
        } else {
            None
        };
        provider.configure(json!({"source":if worktree&&host=="codex"{"src/Refund.php"}else{v["source"].as_str().unwrap_or("src/Invoice.php")},"read":if let Some(sub)=sub{let depth=Path::new(sub).components().count();format!("{}{}", "../".repeat(depth),if worktree&&host=="codex"{"src/Refund.php"}else{v["source"].as_str().unwrap_or("src/Invoice.php")})}else{if worktree&&host=="codex"{"src/Refund.php".into()}else{v["source"].as_str().unwrap_or("src/Invoice.php").into()}},"enter":flag(v,"enter_worktree"),"enter_name":"entered","permissions":flag(v,"permissions"),"cli":v["cli"],"task":task,"policy_cli":policy_cli}),checkpoint)?;
        let mut start = place.clone();
        if let Some(sub) = sub {
            start = start.join(sub);
            std::fs::create_dir_all(&start)?;
        }
        let mut permissions_root = None;
        if flag(v, "permissions") {
            provider.configure(
                json!({"source":v["source"],"permissions":true,"policy_cli":policy_cli}),
                None,
            )?;
            for name in [
                ".env",
                "src/.env",
                ".env.local",
                "secrets/key.txt",
                "src/secrets/key.txt",
            ] {
                write(&self.project.join(name), b"BENCH=probe\n")?;
            }
            env.insert("APP_ENV".into(), "bench".into());
            env.insert("FORCE_COLOR".into(), "0".into());
            let mut args = base.clone();
            args.extend([
                "-p".into(),
                QUESTION.into(),
                "--output-format".into(),
                "stream-json".into(),
                "--verbose".into(),
            ]);
            let root = capture(&args, &self.project, &env, timeout)?;
            permissions_root = Some(probe_results(&root.events));
            provider.reset()?;
            provider.configure(json!({"source":v["source"],"enter":flag(v,"enter_worktree"),"enter_name":"entered","permissions":true,"policy_cli":policy_cli}),None)?;
            first = text(&journal).lines().count();
        }
        let mut done = if v.get("queued").is_some() {
            #[cfg(unix)]
            {
                if host == "codex" {
                    let packages = PathBuf::from(named(v, "original_packages", ""));
                    if packages.is_dir() {
                        let _ = std::os::unix::fs::symlink(packages, codex_home.join("packages"));
                    }
                }
            }
            terminal(
                &base,
                &start,
                &env,
                provider,
                &journal,
                timeout,
                if host == "codex" { 8.0 } else { 6.0 },
            )?
        } else if host == "codex" && flag(v, "app_server") {
            app_server(
                &binary,
                &start,
                &env,
                &named(v, "sandbox", "workspace-write"),
                timeout,
                managed.then_some(place.as_path()),
            )?
        } else if host == "claude" && flag(v, "desktop") {
            env.insert("CLAUDE_CODE_ENTRYPOINT".into(), "claude-desktop".into());
            desktop(&base, &start, &env, timeout)?
        } else {
            let args = if host == "codex" {
                let mut a = vec![
                    binary.clone(),
                    "exec".into(),
                    "-C".into(),
                    start.to_string_lossy().into(),
                    "--skip-git-repo-check".into(),
                    "--sandbox".into(),
                    named(v, "sandbox", "workspace-write"),
                ];
                if managed {
                    a.push("--worktree".into());
                }
                a.push(QUESTION.into());
                a.extend(lent);
                a
            } else {
                let mut a = base;
                a.extend([
                    "-p".into(),
                    QUESTION.into(),
                    "--output-format".into(),
                    "stream-json".into(),
                    "--verbose".into(),
                ]);
                a
            };
            let args = if host == "codex" {
                if let Some(o) = &overlay {
                    infra_tooling::attach::apply_overlay(host, &args, o, true)?
                } else {
                    args
                }
            } else {
                args
            };
            capture(&args, &start, &env, timeout)?
        };
        let helpers = if host == "codex" {
            cleanup_helpers(&codex_home)
        } else {
            0
        };
        if !done.screen.is_empty() {
            write(&self.work.join("screen.txt"), done.screen.as_bytes())?;
        }
        if host == "codex" && done.threads.is_empty() {
            let re = regex::Regex::new(r"(?m)^session id: (\S+)$")
                .map_err(|_| artifact::invalid("session expression"))?;
            done.threads = re
                .captures_iter(&done.stderr)
                .map(|m| m[1].into())
                .collect();
        }
        if managed && !flag(v, "app_server") {
            let mut all = vec![];
            walk(&codex_home.join("worktrees"), &mut all);
            let candidates = all
                .iter()
                .filter(|p| p.file_name().is_some_and(|n| n == ".git"))
                .filter_map(|p| p.parent().map(PathBuf::from))
                .collect::<Vec<_>>();
            if let Some(found) = candidates.into_iter().find(|p| {
                reconstruction::git(p, &["rev-parse", "--absolute-git-dir"])
                    .ok()
                    .and_then(|admin| {
                        artifact::read_json(&PathBuf::from(admin).join("codex-thread.json"), 65536)
                            .ok()
                    })
                    .is_some_and(|v| {
                        done.threads
                            .iter()
                            .any(|t| v["ownerThreadId"] == t.as_str())
                    })
            }) {
                place = found;
            }
        }
        let rows = read_journal(&journal, first);
        let events = events(&rows);
        let requests = provider.requests();
        let steps = requests
            .iter()
            .filter(|r| {
                r["since"]
                    .as_str()
                    .is_some_and(|s| s.contains(bench::MARKER))
            })
            .collect::<Vec<_>>();
        let carried = steps
            .first()
            .map(|r| {
                r["since"]
                    .as_str()
                    .unwrap_or("")
                    .matches(bench::MARKER)
                    .count()
            })
            .unwrap_or(0);
        let saved = events.iter().any(|e| e == "stop:skipped:recovery-saved");
        let heard = requests
            .iter()
            .map(|r| r["since"].as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        let policy = if attached {
            text(&self.source.join(&edition).join("AGENTS.md"))
        } else {
            text(&self.project.join("AGENTS.md"))
        };
        let policy_ok = !policy.is_empty() && policy_heard(&policy, &requests);
        let mut records = vec![];
        let mut paths = vec![];
        walk(&home.join(".claude/projects"), &mut paths);
        for p in paths
            .into_iter()
            .filter(|p| p.extension().is_some_and(|s| s == "jsonl"))
        {
            records.extend(
                text(&p)
                    .lines()
                    .filter_map(|s| artifact::strict(s.as_bytes()).ok()),
            );
        }
        let errors = records
            .iter()
            .filter(|v| v["subtype"] == "stop_hook_summary")
            .map(|v| v["hookErrors"].as_array().map(Vec::len).unwrap_or(0))
            .sum::<usize>();
        let notifications = done
            .events
            .iter()
            .filter(|v| {
                v["subtype"] == "notification"
                    && v["key"].as_str().is_some_and(|s| s.contains("error"))
            })
            .count();
        let mut ok = saved && carried == 1 && errors == 0 && notifications == 0 && policy_ok;
        let mut report = json!({"host":host,"binary":binary,"version":capture(&[binary.clone(),"--version".into()],&self.project,&env,Duration::from_secs(30)).map(|d|d.stdout.trim().to_string()).unwrap_or_else(|_|"unknown".into()),"exit":done.code,"mode":mode,"saved":saved,"step_messages":carried,"events":events,"hook_errors":errors,"error_notifications":notifications,"policy":policy_ok,"policy_bytes":policy.len(),"hook_trust":hook_trust,"helpers_stopped":helpers,"stderr_tail":if done.code==0{String::new()}else{done.stderr.chars().rev().take(300).collect::<String>().chars().rev().collect::<String>()}});
        if host == "codex" && v["sandbox"] == "read-only" {
            ok = carried == 0
                && events.iter().any(|e| e == "stop:skipped:read-only-sandbox")
                && policy_ok;
        }
        if host == "claude" && mode == "plan" {
            ok = steps.is_empty()
                && events.iter().any(|e| e == "stop:skipped:plan-mode")
                && errors == 0
                && notifications == 0
                && policy_ok;
        }
        if v.get("queued").is_some() {
            let unsaved = done.screen.contains("not saved completely");
            let landed = events
                .iter()
                .any(|e| e == "save:saved" || e.starts_with("save:saved:"));
            ok = landed
                && !unsaved
                && !events.iter().any(|e| e.contains("request-unavailable"))
                && policy_ok
                && errors == 0;
            report["unsaved_on_screen"] = unsaved.into();
        }
        if host == "codex" && flag(v, "app_server") || host == "claude" && flag(v, "desktop") {
            report["approval_requests"] = json!(done.asked);
            ok &= done.asked.is_empty();
        }
        if sub.is_some() || worktree {
            let spooled = events.iter().any(|e| e == "save:saved:spooled");
            report["started_in"] = json!(start.strip_prefix(&self.work).ok());
            report["spooled"] = spooled.into();
            report["capsule"] = heard.contains("working:").into();
            ok &= heard.contains("working:") && (host != "codex" || spooled);
        }
        let context = infra_core::RuntimeContext::new(
            infra_core::Roots::installed(&state)?,
            "local",
            "governed",
        )?;
        let records = infra_core::BrainRepository::new(context)?
            .inventory(false)?
            .records;
        let has_task = |id: &str| {
            records.iter().any(|r| {
                r.record["type"] == "task"
                    && r.record["external_id"] == id
                    && r.record["progress"].as_str().is_some_and(|s| !s.is_empty())
            })
        };
        if worktree {
            let owner = if managed {
                let admin = reconstruction::git(&place, &["rev-parse", "--absolute-git-dir"])?;
                artifact::read_json(&PathBuf::from(admin).join("codex-thread.json"), 65536)
                    .ok()
                    .and_then(|v| v["ownerThreadId"].as_str().map(String::from))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let expected = if managed {
                format!("codex-thread/{owner}")
            } else {
                WORK_BRANCH.into()
            };
            let worktree_task = has_task(&expected);
            let mut own_indices = vec![];
            walk(&state.join("memory-bank/local/worktrees"), &mut own_indices);
            let own_index = own_indices
                .iter()
                .any(|p| p.file_name().is_some_and(|n| n == "context.db"));
            let clean =
                reconstruction::git(&place, &["status", "--porcelain", "--ignored"])?.is_empty();
            let source = records.iter().any(|r| {
                r.record["type"] == "finding"
                    && r.record["source_fingerprints"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|s| s["path"] == "src/Refund.php")
            });
            report["worktree_task"] = worktree_task.into();
            report["own_index"] = own_index.into();
            report["worktree_clean"] = clean.into();
            report["worktree_source"] = source.into();
            ok &= worktree_task && own_index && clean && (host != "codex" || source);
            if host == "codex" {
                let trusted = setup["approved"].as_u64().unwrap_or(0) > 0
                    && setup["listed"] == true
                    && setup["approvals_before"]
                        .as_array()
                        .is_some_and(Vec::is_empty)
                    && setup["unapproved"]["hook_lines"] == 0
                    && setup["unapproved"]["capsule"] == false
                    && setup["unapproved"]["save"] == false;
                report["setup"] = setup;
                ok &= trusted;
                if managed {
                    let detached = reconstruction::git(
                        &place,
                        &["symbolic-ref", "--quiet", "--short", "HEAD"],
                    )
                    .is_err();
                    let owner_session = done.threads == vec![owner.clone()] && !owner.is_empty();
                    report["owner"] = owner.into();
                    report["owner_is_session"] = owner_session.into();
                    report["detached"] = detached.into();
                    ok &= detached && owner_session;
                }
            }
        }
        if flag(v, "enter_worktree") {
            let entered = self.project.join(".claude/worktrees/entered");
            let task = has_task("worktree-entered");
            let clean = entered.is_dir()
                && reconstruction::git(&entered, &["status", "--porcelain", "--ignored"])
                    .is_ok_and(|s| s.is_empty());
            report["entered"] = entered.is_dir().into();
            report["entered_task"] = task.into();
            report["entered_clean"] = clean.into();
            ok &= task && clean;
        }
        if v["cli"] == "retrieve" {
            let answers = provider.cli_answers();
            let ran = answers.first().is_some_and(|a| {
                a["failed"] == false
                    && artifact::strict(a["output"].as_str().unwrap_or("").as_bytes())
                        .ok()
                        .is_some_and(|v| v["task_id"] == task)
            });
            report["cli_answers"] = json!(answers);
            report["cli_ran"] = ran.into();
            ok &= ran;
        }
        if v["cli"] == "checkpoint" {
            let checkpoint = provider
                .checkpoint_report(&state, &task)?
                .unwrap_or(Value::Null);
            ok &= checkpoint["ok"] == true;
            report["checkpoint"] = checkpoint;
        }
        if let Some(root) = permissions_root {
            let second = probe_results(&done.events);
            let all = bench::probe_calls().len();
            let complete = root.len() == all && second.len() == all;
            let equal = complete && root.iter().all(|(k, v)| second.get(k) == Some(v));
            report["permission_root"] = json!(root);
            report["permission_below"] = json!(second);
            report["permissions_equal"] = equal.into();
            ok = equal && errors == 0 && policy_ok;
        }
        report["ok"] = (ok && !crate::cancel::interrupted()).into();
        report["interrupted"] = crate::cancel::interrupted().into();
        Ok(report)
    }
}
fn probe_results(events: &[Value]) -> BTreeMap<String, String> {
    fn visit(v: &Value, out: &mut BTreeMap<String, String>) {
        if v["type"] == "tool_result"
            && let Some(id) = v["tool_use_id"]
                .as_str()
                .filter(|id| id.starts_with("toolu_probe_"))
        {
            let output = v["content"].to_string().to_lowercase();
            let label = if [
                "denied by your permission",
                "has been denied",
                "project's settings deny",
                "covered by a read deny",
            ]
            .iter()
            .any(|p| output.contains(p))
            {
                "deny"
            } else if ["haven't granted", "requires approval"]
                .iter()
                .any(|p| output.contains(p))
            {
                "ask"
            } else {
                "ran"
            };
            let value = if id == "toolu_probe_16" {
                format!(
                    "{label}:{}",
                    output.split_whitespace().collect::<Vec<_>>().join(" ")
                )
            } else {
                label.into()
            };
            out.insert(id.into(), value);
        }
        match v {
            Value::Array(a) => {
                for v in a {
                    visit(v, out)
                }
            }
            Value::Object(m) => {
                for v in m.values() {
                    visit(v, out)
                }
            }
            _ => {}
        }
    }
    let mut out = BTreeMap::new();
    for v in events {
        visit(v, &mut out);
    }
    out
}
pub fn worker(args: &[String]) -> Result<Option<i32>> {
    if args.first().map(String::as_str) != Some("__bench-inside") {
        return Ok(None);
    }
    let _signals = crate::cancel::Scope::install()?;
    let file = args
        .get(1)
        .ok_or_else(|| artifact::invalid("benchmark worker request required"))?;
    let v = artifact::read_json(Path::new(file), artifact::MAX_TREE)?;
    if !v["eval_clock"].is_null() {
        return Err(artifact::invalid(
            "eval-clock-child-unsupported: offline provider benchmark cannot propagate a historical clock through host processes",
        ));
    }
    #[cfg(target_os = "linux")]
    {
        let me = std::fs::read_link("/proc/self/ns/net")?;
        let host = std::fs::read_link("/proc/1/ns/net")?;
        if me == host {
            return Err(artifact::invalid(
                "offline worker refuses host network namespace",
            ));
        }
    }
    let mut env = clean_env();
    let cwd = PathBuf::from(
        v["work"]
            .as_str()
            .ok_or_else(|| artifact::invalid("work required"))?,
    );
    env.insert("HOME".into(), cwd.join("home").to_string_lossy().into());
    let ip = capture(
        &[
            "ip".into(),
            "link".into(),
            "set".into(),
            "lo".into(),
            "up".into(),
        ],
        &cwd,
        &env,
        Duration::from_secs(10),
    )?;
    if ip.code != 0 {
        return Err(artifact::invalid("cannot raise isolated loopback"));
    }
    let host = v["host"]
        .as_str()
        .ok_or_else(|| artifact::invalid("host required"))?;
    let mut driver = NativeDriver {
        source: PathBuf::from(
            v["tooling_root"]
                .as_str()
                .ok_or_else(|| artifact::invalid("tooling root required"))?,
        ),
        work: cwd,
        project: PathBuf::from(
            v["project"]
                .as_str()
                .ok_or_else(|| artifact::invalid("project required"))?,
        ),
    };
    let result = bench::run(&mut driver, host, &v)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(Some(0))
}

#[cfg(unix)]
fn namespace_uid() -> u32 {
    unsafe { libc::getuid() }
}
#[cfg(unix)]
fn namespace_gid() -> u32 {
    unsafe { libc::getgid() }
}
#[cfg(not(unix))]
fn namespace_uid() -> u32 {
    unreachable!("Linux-only namespace caller")
}
#[cfg(not(unix))]
fn namespace_gid() -> u32 {
    unreachable!("Linux-only namespace caller")
}

#[cfg(all(test, unix))]
mod benchmark_scratch_tests {
    use super::*;
    struct Owned(PathBuf);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn benchmark_scratch_preserves_existing_shared_base_and_protects_only_new_directories() {
        let root = std::env::temp_dir().join(format!("infra-bench-base-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        artifact::chmod(&root, 0o700).unwrap();
        let owned = Owned(root);
        let base = owned.0.join("simulated-shared-base");
        std::fs::create_dir(&base).unwrap();
        artifact::chmod(&base, 0o1777).unwrap();
        let sentinel = base.join("unrelated-sentinel");
        std::fs::write(&sentinel, b"existing shared-base content").unwrap();
        let work = benchmark_scratch(&base, "fixture").unwrap();
        assert_eq!(artifact::mode(&base).unwrap(), 0o1777);
        assert_eq!(artifact::mode(&work.0).unwrap(), 0o700);
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"existing shared-base content"
        );
        let created_base = owned.0.join(r"missing\literal:cache").join("nested");
        let other = benchmark_scratch(&created_base, "fixture").unwrap();
        assert_eq!(artifact::mode(&created_base).unwrap(), 0o700);
        assert_eq!(
            artifact::mode(created_base.parent().unwrap()).unwrap(),
            0o700
        );
        assert_eq!(artifact::mode(&other.0).unwrap(), 0o700);
        assert!(!owned.0.join("missing").exists());
        assert_eq!(artifact::mode(&base).unwrap(), 0o1777);
    }
}
