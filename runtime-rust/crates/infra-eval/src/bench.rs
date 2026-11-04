//! Shared native offline-provider and literal checkpoint machinery.
//! Native-host variants supply a required BenchmarkDriver; this module never
//! substitutes a fake successful host observation for an unexecuted variant.
use crate::{Result, artifact};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub const MARKER: &str = "INTERNAL_NATIVE_MEMORY_RESULT";
pub const PROGRESS: &str = "Checkpoint: refunds keep the invoice date.";
pub const NEXT_STEP: &str = "Check credit notes against the invoice date.";
#[derive(Clone, Debug)]
pub struct Step {
    pub name: String,
    pub output: String,
    pub failed: bool,
}
pub struct Checkpoint {
    skill: String,
    pub cli: String,
    pub task: Option<String>,
    pub steps: Vec<Value>,
    pub stopped: Option<String>,
}
impl Checkpoint {
    pub fn new(skill: String) -> Self {
        Self {
            skill,
            cli: String::new(),
            task: None,
            steps: vec![],
            stopped: None,
        }
    }
    pub fn takes_status(&self) -> bool {
        let Some(start) = self.skill.find("## Governed Workflow") else {
            return false;
        };
        let text = &self.skill[start..];
        let text = text
            .split("2. ")
            .next()
            .unwrap_or(text)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        text.contains("On a detached HEAD") && text.contains("task_identity.task_id")
    }
    pub fn next(
        &mut self,
        heard: &str,
        done: &[Step],
        policy_cli: &str,
    ) -> Option<(String, String)> {
        if self.cli.is_empty() {
            let hint = regex::Regex::new(r"Memory CLI: [^`\n]*`([^`\n]+)` in place of")
                .ok()
                .and_then(|r| r.captures(heard))
                .map(|m| m[1].to_string());
            self.cli = hint.unwrap_or_else(|| policy_cli.into());
        }
        let Some(last) = done.last() else {
            return Some((
                "branch".into(),
                "git symbolic-ref --quiet --short HEAD".into(),
            ));
        };
        if last.name == "branch" || last.name == "status" {
            self.task = if last.failed {
                None
            } else if last.name == "branch" {
                let s = last.output.trim();
                (!s.is_empty() && !s.contains(['\n', ' '])).then(|| s.to_string())
            } else {
                artifact::strict(last.output.as_bytes()).ok().and_then(|v| {
                    v["automatic_memory"]["task_identity"]["task_id"]
                        .as_str()
                        .map(String::from)
                })
            };
            if let Some(task) = &self.task {
                return Some((
                    "flush".into(),
                    format!(
                        "{} turn --task-id {} --flush --json",
                        self.cli,
                        shell_words::quote(task)
                    ),
                ));
            }
            if last.name == "branch" && self.takes_status() {
                return Some(("status".into(), format!("{} status --json", self.cli)));
            }
            self.stopped = Some(
                if last.name == "branch" {
                    "the skill stops on a detached HEAD"
                } else {
                    "status named no task"
                }
                .into(),
            );
            return None;
        }
        if last.name == "flush" && !last.failed {
            return Some((
                "update".into(),
                format!(
                    "{} update --task-id {} --revision auto --progress {} --next-step {} --json",
                    self.cli,
                    shell_words::quote(self.task.as_deref().unwrap_or("")),
                    shell_words::quote(PROGRESS),
                    shell_words::quote(NEXT_STEP)
                ),
            ));
        }
        if last.failed {
            self.stopped = Some(format!("{} failed", last.name));
        }
        None
    }
    pub fn record(&mut self, step: &Step) {
        let queued = artifact::strict(step.output.as_bytes()).is_ok_and(|v| v["queued"] == true);
        self.steps.push(json!({"step":step.name,"ok":!step.failed,"queued":queued,"answer":step.output.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect::<String>()}));
    }
    pub fn report(&self, project: &Path, task: &str) -> Result<Value> {
        let ctx = infra_core::RuntimeContext::new(
            infra_core::Roots::installed(project)?,
            "local",
            "governed",
        )?;
        let landed = infra_core::BrainRepository::new(ctx)?
            .inventory(false)?
            .records
            .iter()
            .any(|r| {
                r.record["type"] == "task"
                    && r.record["external_id"] == task
                    && r.record["next_steps"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|s| s.as_str().is_some_and(|s| s.contains(NEXT_STEP)))
            });
        let n = self.steps.len();
        let ok = landed
            && self.task.as_deref() == Some(task)
            && n >= 2
            && self.steps[n - 2]["step"] == "flush"
            && self.steps[n - 1]["step"] == "update"
            && self.steps[n - 2..].iter().all(|s| s["ok"] == true);
        Ok(
            json!({"cli":self.cli,"status_on_detached_head":self.takes_status(),"task":self.task,"steps":self.steps,"stopped":self.stopped,"landed":landed,"ok":ok}),
        )
    }
}
pub fn result_for(source: &str) -> Value {
    json!({"progress":format!("Read \x60{source}\x60: \x60Invoice::next($tenant)\x60 restarts per tenant; fee is $5.\nTab\there; path C:\\shop\\src; it's \"quoted\"; счёт."),"verified":true,"learnings":[{"type":"finding","title":"Invoice numbers are per tenant","consequence":"Call \x60Invoice::next($tenant)\x60; never share one counter between tenants.","sources":[source]}]})
}
pub trait BenchmarkDriver {
    fn run(&mut self, host: &str, request: &Value, provider: &ScriptedProvider) -> Result<Value>;
}
pub fn run(driver: &mut impl BenchmarkDriver, host: &str, request: &Value) -> Result<Value> {
    if !["codex", "claude"].contains(&host) {
        return Err(artifact::invalid("unknown native benchmark host"));
    }
    let provider = ScriptedProvider::start(
        request["source"]
            .as_str()
            .unwrap_or("src/Invoice.php")
            .into(),
        request["queued"]
            .as_f64()
            .map(Duration::from_secs_f64)
            .unwrap_or(Duration::ZERO),
    )?;
    let mut observed = driver.run(host, request, &provider)?;
    observed["provider_requests"] = json!(provider.requests());
    observed["real_provider_contacted"] = false.into();
    observed["provider_tokens_spent"] = 0.into();
    Ok(observed)
}
#[derive(Default)]
struct State {
    requests: Vec<Value>,
    source: String,
    save_delay: Duration,
    save_begun: bool,
    shutdown: bool,
    config: Value,
    checkpoint: Option<Checkpoint>,
    checkpoint_calls: BTreeMap<String, String>,
    cli_answers: Vec<Value>,
}
pub struct ScriptedProvider {
    pub address: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl ScriptedProvider {
    pub fn start(source: String, save_delay: Duration) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = format!("http://{}", listener.local_addr()?);
        listener.set_nonblocking(true)?;
        let state = Arc::new(Mutex::new(State {
            source,
            save_delay,
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let server_state = state.clone();
        let server_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut clients = vec![];
            while !server_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state = server_state.clone();
                        clients.push(std::thread::spawn(move || {
                            let _ = serve(stream, &state);
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
                clients.retain(|t| !t.is_finished());
            }
            for t in clients {
                let _ = t.join();
            }
        });
        Ok(Self {
            address,
            state,
            stop,
            thread: Some(thread),
        })
    }
    pub fn configure(&self, config: Value, checkpoint: Option<Checkpoint>) -> Result<()> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| artifact::invalid("provider state"))?;
        if let Some(source) = config["source"].as_str() {
            s.source = source.into();
        }
        s.config = config;
        s.checkpoint = checkpoint;
        s.checkpoint_calls.clear();
        s.cli_answers.clear();
        Ok(())
    }
    pub fn reset(&self) -> Result<()> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| artifact::invalid("provider state"))?;
        s.requests.clear();
        s.save_begun = false;
        s.checkpoint_calls.clear();
        s.cli_answers.clear();
        Ok(())
    }
    pub fn checkpoint_report(&self, project: &Path, task: &str) -> Result<Option<Value>> {
        let s = self
            .state
            .lock()
            .map_err(|_| artifact::invalid("provider state"))?;
        s.checkpoint
            .as_ref()
            .map(|c| c.report(project, task))
            .transpose()
    }
    pub fn cli_answers(&self) -> Vec<Value> {
        self.state
            .lock()
            .map(|s| s.cli_answers.clone())
            .unwrap_or_default()
    }
    pub fn requests(&self) -> Vec<Value> {
        self.state
            .lock()
            .map(|s| s.requests.clone())
            .unwrap_or_default()
    }
    pub fn save_begun(&self) -> bool {
        self.state.lock().is_ok_and(|s| s.save_begun)
    }
}
impl Drop for ScriptedProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut s) = self.state.lock() {
            s.shutdown = true;
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
fn reply(stream: &mut TcpStream, status: u16, kind: &str, body: &[u8]) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        if status == 200 { "OK" } else { "Not Found" },
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}
fn event(kind: &str, value: &Value) -> Result<String> {
    Ok(format!(
        "event: {kind}\ndata: {}\n\n",
        serde_json::to_string(value)?
    ))
}
fn read_request(stream: &mut TcpStream) -> Result<(String, String, Value)> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut raw = vec![];
    let header_end = loop {
        let mut byte = [0u8; 1];
        if stream.read(&mut byte)? == 0 {
            return Err(artifact::invalid("incomplete benchmark HTTP request"));
        }
        raw.push(byte[0]);
        if raw.len() > 65536 {
            return Err(artifact::invalid("benchmark HTTP header bound"));
        }
        if raw.ends_with(b"\r\n\r\n") {
            break raw.len();
        }
    };
    let header = std::str::from_utf8(&raw[..header_end])
        .map_err(|_| artifact::invalid("HTTP header encoding"))?;
    let first = header.lines().next().unwrap_or("");
    let mut first = first.split_whitespace();
    let method = first.next().unwrap_or("").to_string();
    let path = first
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .to_string();
    let mut length = None;
    for line in header.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            if length.is_some() {
                return Err(artifact::invalid("duplicate content length"));
            }
            length = Some(
                v.trim()
                    .parse::<usize>()
                    .map_err(|_| artifact::invalid("invalid content length"))?,
            );
        }
    }
    let length = length.unwrap_or(0);
    if length > 8 * 1024 * 1024 {
        return Err(artifact::invalid("benchmark request byte bound"));
    }
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body)?;
    Ok((
        method,
        path,
        if body.is_empty() {
            json!({})
        } else {
            artifact::strict(&body)?
        },
    ))
}
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}
fn exact_save(text: &str) -> Option<String> {
    let text = unescape(text);
    let pattern = regex::Regex::new(crate::rehearsal::SAVE_COMMAND).ok()?;
    pattern.captures(&text).map(|m| m[1].to_string())
}
fn command_tool(body: &Value, host: &str, command: &str, number: usize, tag: &str) -> Value {
    if host == "claude" {
        return json!({"type":"tool_use","id":format!("toolu_{tag}_{number}"),"name":"Bash","input":{"command":command}});
    }
    let tools = body["tools"].as_array().cloned().unwrap_or_default();
    let named = tools.iter().filter_map(|t| t["name"].as_str()).find(|n| {
        ["exec_command", "shell_command", "shell"]
            .iter()
            .any(|s| n.ends_with(s))
    });
    let Some(shell) = named else {
        return json!({"type":"custom_tool_call","id":format!("fc_{tag}_{number}"),"call_id":format!("call_{tag}_{number}"),"name":"exec","input":format!("const r = await tools.exec_command({}); text(r);",serde_json::to_string(&json!({"cmd":command})).unwrap_or_default())});
    };
    let input = if shell.ends_with("shell") && !shell.ends_with("exec_command") {
        json!({"command":["bash","-lc",command]})
    } else if shell.ends_with("shell_command") {
        json!({"command":command})
    } else {
        json!({"cmd":command})
    };
    json!({"type":"function_call","id":format!("fc_{tag}_{number}"),"call_id":format!("call_{tag}_{number}"),"name":shell,"arguments":serde_json::to_string(&input).unwrap_or_default()})
}
/// A command's output as Codex reports it - "Chunk ID", wall time, "Process
/// exited with code N", token count, then "Output:" - and whether it exited 0;
/// any other text as it is.
fn codex_output(text: &str) -> (String, Option<bool>) {
    let marker = "Process exited with code ";
    let (Some(exit), true) = (text.find(marker), text.starts_with("Chunk ID:")) else {
        return (text.into(), None);
    };
    let code = text[exit + marker.len()..]
        .split(|c: char| !(c.is_ascii_digit() || c == '-'))
        .next()
        .unwrap_or("");
    let Some(start) = text[exit..].find("Output:") else {
        return (text.into(), None);
    };
    let body = text[exit + start + "Output:".len()..].trim_start_matches(['\n', '\r', ' ']);
    (body.into(), Some(code == "0"))
}
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(text_of).collect::<Vec<_>>().join("\n"),
        Value::Object(m) => ["text", "output", "content"]
            .iter()
            .filter_map(|k| m.get(*k))
            .map(text_of)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
pub fn probe_calls() -> Vec<Value> {
    let mut out = vec![];
    for path in [
        ".env",
        "src/.env",
        ".env.local",
        "secrets/key.txt",
        "src/secrets/key.txt",
        "composer.json",
        "src/Invoice.php",
        "/etc/hostname",
    ] {
        out.push(json!({"name":"Read","input":{"file_path":path}}));
    }
    for path in ["notes.txt", ".env.probe"] {
        out.push(json!({"name":"Write","input":{"file_path":path,"content":"probe\n"}}));
    }
    for command in [
        "rm -rf build",
        "sudo -n true",
        "chmod 777 notes.txt",
        "ls",
        "git status",
        "echo probe && rm -rf build",
        "printenv APP_ENV FORCE_COLOR",
    ] {
        out.push(json!({"name":"Bash","input":{"command":command}}));
    }
    for url in ["https://github.com", "https://example.com"] {
        out.push(json!({"name":"WebFetch","input":{"url":url,"prompt":"Read title"}}));
    }
    out
}
fn decide(body: &Value, host: &str, s: &mut State, number: usize) -> Result<(Vec<Value>, String)> {
    let all = if host == "codex" {
        body["input"].as_array()
    } else {
        body["messages"].as_array()
    }
    .cloned()
    .unwrap_or_default();
    let last = all.iter().rposition(|m| {
        if host == "codex" {
            m["role"] == "assistant"
                || ["function_call", "custom_tool_call"].contains(&m["type"].as_str().unwrap_or(""))
        } else {
            m["role"] == "assistant"
        }
    });
    let tail = &all[last.map(|i| i + 1).unwrap_or(0)..];
    let since = unescape(&text_of(&json!(tail)));
    let source = s.source.clone();
    let read = s.config["read"].as_str().unwrap_or(&source).to_string();
    let text = |t: &str| {
        if host == "claude" {
            json!({"type":"text","text":t})
        } else {
            message(number, t)
        }
    };
    if number > 40 {
        return Ok((vec![text("Loop guard.")], since));
    }
    if host == "claude" && body["tools"].as_array().is_none_or(Vec::is_empty) {
        return Ok((vec![text("Invoice numbering")], since));
    }
    let mut results = vec![];
    for m in tail {
        if ["function_call_output", "custom_tool_call_output"]
            .contains(&m["type"].as_str().unwrap_or(""))
        {
            let (output, exited) = codex_output(&text_of(&m["output"]));
            results.push((
                m["call_id"].as_str().unwrap_or("").to_string(),
                output,
                m["is_error"] == true || exited == Some(false),
            ));
        }
        for block in m["content"].as_array().into_iter().flatten() {
            if block["type"] == "tool_result" {
                results.push((
                    block["tool_use_id"].as_str().unwrap_or("").to_string(),
                    text_of(&block["content"]),
                    block["is_error"] == true,
                ));
            }
        }
    }
    let issued = |prefix: &str| {
        s.requests
            .iter()
            .flat_map(|r| r["output"].as_array().into_iter().flatten())
            .any(|v| {
                v["id"]
                    .as_str()
                    .or_else(|| v["call_id"].as_str())
                    .is_some_and(|id| id.starts_with(prefix))
            })
    };
    if host == "claude" && s.config["enter"] == true && !issued("toolu_enter_") {
        return Ok((
            vec![
                json!({"type":"tool_use","id":format!("toolu_enter_{number}"),"name":"EnterWorktree","input":{"name":s.config["enter_name"].as_str().unwrap_or("entered")}}),
            ],
            since,
        ));
    }
    if host == "claude" && s.config["permissions"] == true {
        let calls = probe_calls();
        let done = s
            .requests
            .iter()
            .flat_map(|r| r["output"].as_array().into_iter().flatten())
            .filter(|v| {
                v["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("toolu_probe_"))
            })
            .count();
        if let Some(call) = calls.get(done) {
            return Ok((
                vec![
                    json!({"type":"tool_use","id":format!("toolu_probe_{done}"),"name":call["name"],"input":call["input"]}),
                ],
                since,
            ));
        }
        return Ok((vec![text("Permission probes complete.")], since));
    }
    for (id, output, failed) in &results {
        if let Some(step) = s.checkpoint_calls.remove(id)
            && let Some(checkpoint) = &mut s.checkpoint
        {
            let step = Step {
                name: step,
                output: output.clone(),
                failed: *failed,
            };
            checkpoint.record(&step);
        }
        if id.starts_with("toolu_cli_") {
            s.cli_answers.push(json!({"output":output,"failed":failed}));
        }
    }
    if s.checkpoint.is_some() && !since.contains(MARKER) {
        let mut completed = vec![];
        for m in &all {
            if host == "codex" {
                if m["type"] == "function_call_output" || m["type"] == "custom_tool_call_output" {
                    let id = m["call_id"].as_str().unwrap_or("");
                    if let Some(step) = id
                        .split("_checkpoint_")
                        .nth(1)
                        .and_then(|s| s.rsplit_once('_').map(|r| r.0))
                    {
                        let (output, exited) = codex_output(&text_of(&m["output"]));
                        completed.push(Step {
                            name: step.into(),
                            output,
                            failed: m["is_error"] == true || exited == Some(false),
                        });
                    }
                }
            } else {
                for b in m["content"].as_array().into_iter().flatten() {
                    let id = b["tool_use_id"].as_str().unwrap_or("");
                    if b["type"] == "tool_result"
                        && let Some(step) = id
                            .strip_prefix("toolu_checkpoint_")
                            .and_then(|s| s.rsplit_once('_').map(|r| r.0))
                    {
                        completed.push(Step {
                            name: step.into(),
                            output: text_of(&b["content"]),
                            failed: b["is_error"] == true,
                        });
                    }
                }
            }
        }
        let cli = s.config["policy_cli"].as_str().unwrap_or("").to_string();
        if let Some((step, command)) = s
            .checkpoint
            .as_mut()
            .and_then(|c| c.next(&since, &completed, &cli))
        {
            let tag = format!("checkpoint_{step}");
            let call = command_tool(body, host, &command, number, &tag);
            let id = call["call_id"]
                .as_str()
                .or_else(|| call["id"].as_str())
                .unwrap_or("")
                .to_string();
            s.checkpoint_calls.insert(id, step);
            return Ok((vec![call], since));
        }
    }
    if host == "claude"
        && s.config["cli"] == "retrieve"
        && !issued("toolu_cli_")
        && let Some(m) = regex::Regex::new(r"Memory CLI: [^\x60]*\x60([^\x60]+)\x60 in place of")
            .ok()
            .and_then(|r| r.captures(&since))
    {
        let task = s.config["task"]
            .as_str()
            .unwrap_or("feature/SHOP-7-invoices");
        let command = format!(
            "{} retrieve 'invoice numbering' --task-id {} --json",
            &m[1],
            shell_words::quote(task)
        );
        return Ok((
            vec![command_tool(body, host, &command, number, "cli")],
            since,
        ));
    }
    if since.contains(MARKER)
        && let Some(command) = exact_save(&since)
    {
        let payload = serde_json::to_string(&result_for(&source))?;
        let command = if command.contains("'<JSON>'") {
            command.replace("'<JSON>'", &shell_words::quote(&payload))
        } else if command.contains("JSON_HERE") {
            command.replace("JSON_HERE", &shell_words::quote(&payload))
        } else {
            format!(
                "printf '%s\\n' {} | {command}",
                shell_words::quote(&payload)
            )
        };
        return Ok((
            vec![command_tool(body, host, &command, number, "save")],
            since,
        ));
    }
    if !since.contains(MARKER)
        && (s.checkpoint.is_some()
            || results
                .iter()
                .any(|(id, _, _)| id.starts_with("toolu_cli_")))
        && !issued(if host == "claude" {
            "toolu_read_"
        } else {
            "fc_read_"
        })
    {
        return Ok((
            vec![if host == "claude" {
                json!({"type":"tool_use","id":format!("toolu_read_{number}"),"name":"Read","input":{"file_path":read}})
            } else {
                command_tool(
                    body,
                    host,
                    &format!("cat -- {}", shell_words::quote(&read)),
                    number,
                    "read",
                )
            }],
            since,
        ));
    }
    if !results.is_empty() {
        let entered = results
            .iter()
            .any(|(id, _, _)| id.starts_with("toolu_enter_"));
        if !entered {
            return Ok((
                vec![text(
                    if results.iter().any(|(id, _, _)| id.contains("_save_")) {
                        "Saved."
                    } else {
                        "Invoices are numbered per tenant."
                    },
                )],
                since,
            ));
        }
    }
    if host == "claude" {
        Ok((
            vec![
                json!({"type":"tool_use","id":format!("toolu_read_{number}"),"name":"Read","input":{"file_path":read}}),
            ],
            since,
        ))
    } else {
        Ok((
            vec![command_tool(
                body,
                host,
                &format!("cat -- {}", shell_words::quote(&read)),
                number,
                "read",
            )],
            since,
        ))
    }
}
fn serve(mut stream: TcpStream, state: &Arc<Mutex<State>>) -> Result<()> {
    let (method, path, body) = read_request(&mut stream)?;
    if method == "GET" && path.ends_with("/models") {
        return reply(
            &mut stream,
            200,
            "application/json",
            br#"{"object":"list","data":[]}"#,
        );
    }
    if method != "POST" {
        return reply(
            &mut stream,
            404,
            "application/json",
            br#"{"error":{"message":"bench","type":"not_found"}}"#,
        );
    }
    if path.ends_with("/count_tokens") {
        return reply(
            &mut stream,
            200,
            "application/json",
            br#"{"input_tokens":1000}"#,
        );
    }
    let host = if path.ends_with("/responses") {
        "codex"
    } else if path.ends_with("/v1/messages") {
        "claude"
    } else {
        return reply(
            &mut stream,
            404,
            "application/json",
            br#"{"error":{"message":"bench","type":"not_found"}}"#,
        );
    };
    let mut s = state
        .lock()
        .map_err(|_| artifact::invalid("benchmark state unavailable"))?;
    let number = s.requests.len() + 1;
    let (content, since) = decide(&body, host, &mut s, number)?;
    let saving = content.iter().any(|c| {
        c["call_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("call_save_"))
            || c["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("toolu_save_"))
    });
    let delay = if saving && !s.save_begun {
        s.save_begun = true;
        s.save_delay
    } else {
        Duration::ZERO
    };
    s.requests.push(json!({"body":body,"output":content,"host":host,"number":number,"since":since,"system":if host=="codex"{body["instructions"].clone()}else{body["system"].clone()},"tools":body["tools"]}));
    drop(s);
    if host == "claude" {
        let stop = if content.iter().any(|c| c["type"] == "tool_use") {
            "tool_use"
        } else {
            "end_turn"
        };
        let response = json!({"id":format!("msg_bench_{number}"),"type":"message","role":"assistant","model":body["model"],"content":content,"stop_reason":stop,"stop_sequence":null,"usage":{"input_tokens":1000,"output_tokens":20,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}});
        if body["stream"] != true {
            if !delay.is_zero() {
                pause(state, delay);
            }
            return reply(
                &mut stream,
                200,
                "application/json",
                &serde_json::to_vec(&response)?,
            );
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
        )?;
        let mut start = response.clone();
        start["content"] = json!([]);
        start["stop_reason"] = Value::Null;
        stream.write_all(
            event(
                "message_start",
                &json!({"type":"message_start","message":start}),
            )?
            .as_bytes(),
        )?;
        stream.flush()?;
        if !delay.is_zero() {
            pause(state, delay);
        }
        for (index, block) in content.iter().enumerate() {
            let (start, delta) = if block["type"] == "text" {
                (
                    json!({"type":"text","text":""}),
                    json!({"type":"text_delta","text":block["text"]}),
                )
            } else {
                (
                    json!({"type":"tool_use","id":block["id"],"name":block["name"],"input":{}}),
                    json!({"type":"input_json_delta","partial_json":serde_json::to_string(&block["input"])?}),
                )
            };
            for (kind, value) in [
                (
                    "content_block_start",
                    json!({"type":"content_block_start","index":index,"content_block":start}),
                ),
                (
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":index,"delta":delta}),
                ),
                (
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":index}),
                ),
            ] {
                stream.write_all(event(kind, &value)?.as_bytes())?;
            }
        }
        stream.write_all(event("message_delta",&json!({"type":"message_delta","delta":{"stop_reason":stop,"stop_sequence":null},"usage":{"output_tokens":20}}))?.as_bytes())?;
        stream.write_all(event("message_stop", &json!({"type":"message_stop"}))?.as_bytes())?;
    } else {
        let response = json!({"id":format!("resp_{number}"),"object":"response","status":"completed","model":body["model"],"output":content,"usage":{"input_tokens":1000,"output_tokens":20,"total_tokens":1020,"input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":0}}});
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
        )?;
        let mut start = response.clone();
        start["status"] = "in_progress".into();
        start["output"] = json!([]);
        stream.write_all(
            event(
                "response.created",
                &json!({"type":"response.created","response":start}),
            )?
            .as_bytes(),
        )?;
        stream.flush()?;
        if !delay.is_zero() {
            pause(state, delay);
        }
        for (index, item) in content.iter().enumerate() {
            stream.write_all(
                event(
                    "response.output_item.added",
                    &json!({"type":"response.output_item.added","output_index":index,"item":item}),
                )?
                .as_bytes(),
            )?;
            if item["type"] == "message" {
                stream.write_all(event("response.output_text.delta",&json!({"type":"response.output_text.delta","item_id":item["id"],"output_index":index,"content_index":0,"delta":item["content"][0]["text"]}))?.as_bytes())?;
            }
            stream.write_all(
                event(
                    "response.output_item.done",
                    &json!({"type":"response.output_item.done","output_index":index,"item":item}),
                )?
                .as_bytes(),
            )?;
        }
        stream.write_all(
            event(
                "response.completed",
                &json!({"type":"response.completed","response":response}),
            )?
            .as_bytes(),
        )?;
    }
    stream.flush()?;
    Ok(())
}
fn message(number: usize, text: &str) -> Value {
    json!({"id":format!("msg_{number}"),"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":text,"annotations":[]}]})
}

fn pause(state: &Arc<Mutex<State>>, delay: Duration) {
    let end = Instant::now() + delay;
    while Instant::now() < end
        && !crate::cancel::interrupted()
        && !state.lock().is_ok_and(|s| s.shutdown)
    {
        std::thread::sleep(
            Duration::from_millis(10).min(end.saturating_duration_since(Instant::now())),
        );
    }
}

#[cfg(test)]
mod codex_output_tests {
    use super::codex_output;

    #[test]
    fn codex_command_reports_yield_their_output_and_exit() {
        let report = "Chunk ID: 0c2743\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 6\nOutput:\nfeature/SHOP-7-invoices\n";
        assert_eq!(
            codex_output(report),
            ("feature/SHOP-7-invoices\n".to_string(), Some(true))
        );
        let failed = "Chunk ID: 1\nWall time: 0.1 seconds\nProcess exited with code 128\nOutput:\nfatal: ref HEAD is not a symbolic ref\n";
        assert_eq!(codex_output(failed).1, Some(false));
        assert_eq!(codex_output("plain text"), ("plain text".to_string(), None));
    }
}
