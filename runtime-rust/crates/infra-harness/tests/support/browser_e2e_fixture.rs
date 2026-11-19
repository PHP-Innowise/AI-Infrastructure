//! Offline headless browser E2E fixture server using real `http::App`, `NativeServices`,
//! `Graphs` (native tree-sitter SQLite IR), `Sessions`, `PermissionDesk`, and live SSE.
use infra_harness::{
    HarnessError, Result,
    accelerators::Accelerators,
    bindings::{ResultsBridge, SessionLinks, SystemBridge, SystemsDomain},
    commands::CommandCatalog,
    creator::CreatorManager,
    delivery::Delivery,
    fleet,
    graph::{Graphs, Shown},
    http,
    knowledge::KnowledgeDomain,
    omnigraph::{OmnigraphDomain, OmnigraphManager},
    projects::ProjectDomain,
    provider::{Provider, ProviderDomain},
    results::{ResultService, Results},
    services::{CloseFuture, Domain, NativeServices, Payload, Reply, Services},
    sessions::{BlackboardFactory, Event, RunReply, SessionLimits, Sessions, SpecialWorkflow},
    setup::{Setup, SetupDomain},
    skills::SkillsDomain,
    store::SessionStore,
    system_graph::{HarnessCodeGraphBridge, SystemGraphDomain},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, Read, Write},
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const HOLD_MARKER: &str = "[E2E_HOLD_PERMISSION]";
const DEFAULT_TIMEOUT_SECS: u64 = 180;

fn require(valid: bool, reason: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(HarnessError::invalid(reason))
    }
}

fn json_payload(payload: Payload) -> Result<Value> {
    match payload {
        Payload::Json(value) => Ok(value),
        Payload::Bytes { .. } => Err(HarnessError::invalid("Expected fixture JSON payload.")),
    }
}

struct Unused;

impl BlackboardFactory for Unused {
    fn for_session(&self, _: &Value) -> Result<Arc<dyn fleet::Blackboard>> {
        Err(HarnessError::unavailable("No fixture blackboard"))
    }
}

impl SpecialWorkflow for Unused {
    fn run<'a>(
        &'a self,
        _: Value,
        _: String,
        _: String,
        _: Arc<AtomicBool>,
        _: Event,
    ) -> RunReply<'a> {
        Box::pin(async { Err(HarnessError::unavailable("No fixture special workflow")) })
    }
}

struct Offline;

impl Domain for Offline {
    fn execute<'a>(&'a self, operation: &'a str, _: Value) -> Reply<'a> {
        Box::pin(async move {
            Ok(Payload::Json(match operation {
                "providers.sign_in" => json!({
                    "providers": [{
                        "id": "claude",
                        "state": "signed_in",
                        "detail": "Offline E2E fixture",
                        "login": ""
                    }]
                }),
                "updates.status" => json!({"available": false, "state": "fixture"}),
                _ => json!({"fixture": true}),
            }))
        })
    }
}

struct InboxLiveTrigger {
    sessions: Arc<Sessions>,
    root: PathBuf,
    seq: AtomicU64,
}

impl Domain for InboxLiveTrigger {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let result = self.sessions.execute(operation, request).await?;
            if operation == "sessions.inbox_accept" {
                let id = self.seq.fetch_add(1, Ordering::AcqRel) + 1;
                let trigger = self.root.join(format!("emit-live-sse-{id:06}"));
                let _ = fs::write(trigger, b"emit");
            }
            Ok(result)
        })
    }
}

async fn wait_until(
    timeout: Duration,
    label: &str,
    mut ready: impl FnMut() -> Result<bool>,
) -> Result<()> {
    let until = Instant::now() + timeout;
    while !ready()? {
        require(Instant::now() < until, label)?;
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    Ok(())
}

fn find_repo_root() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(manifest) = option_env!("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(manifest).join("../../..");
        candidates.push(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        candidates.extend(exe.ancestors().map(Path::to_path_buf));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.extend(cwd.ancestors().map(Path::to_path_buf));
    }
    candidates.into_iter().find_map(|candidate| {
        let canon = infra_core::fs::canonicalize(&candidate).ok()?;
        if canon.join("harness/web/index.html").is_file()
            && canon.join("Infrastructure-Creator/AGENTS.md").is_file()
        {
            Some(canon)
        } else {
            None
        }
    })
}

fn emit_line(out: &mut impl Write, val: &Value) -> Result<()> {
    writeln!(out, "{val}")?;
    out.flush()?;
    Ok(())
}

fn take_live_sse_triggers(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("emit-live-sse-"))
        })
        .collect::<Vec<_>>();
    found.sort();
    found
        .into_iter()
        .filter(|p| fs::remove_file(p).is_ok())
        .collect()
}

fn emit_turn_events(
    out: &mut impl Write,
    cwd: &Path,
    session_id: &str,
    call_id: &str,
    file_rel: &str,
    md_text: &str,
) -> Result<()> {
    let abs_file = cwd.join(file_rel).to_string_lossy().into_owned();
    emit_line(
        out,
        &json!({
            "type": "assistant",
            "session_id": session_id,
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": call_id,
                    "name": "Read",
                    "input": { "file_path": abs_file }
                }]
            }
        }),
    )?;
    emit_line(
        out,
        &json!({
            "type": "user",
            "session_id": session_id,
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": format!("Read {file_rel} from E2E fixture workspace.")
                }]
            }
        }),
    )?;
    emit_line(
        out,
        &json!({
            "type": "assistant",
            "session_id": session_id,
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": md_text
                }]
            }
        }),
    )?;
    Ok(())
}

fn spawn_stdin_reader() -> std::sync::mpsc::Receiver<Option<String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = std::io::BufReader::new(stdin.lock());
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(None);
                    break;
                }
                Ok(_) => {
                    if tx.send(Some(line)).is_err() {
                        break;
                    }
                }
            }
        }
    });
    rx
}

fn handle_stream_json_provider(cwd: &Path, root: &Path) -> Result<()> {
    let rx = spawn_stdin_reader();
    let mut out = std::io::stdout().lock();
    let mut live_seq = 0u64;
    let mut holding_permission = false;

    loop {
        let msg = if holding_permission {
            for _trigger in take_live_sse_triggers(root) {
                live_seq += 1;
                let call_id = format!("tool-e2e-live-{live_seq}");
                let file_rel = if live_seq % 2 == 1 {
                    "pkg_00/mod_00.js"
                } else {
                    "pkg_01/mod_00.js"
                };
                let md_text = format!(
                    "Live SSE stream update #{live_seq} for `{file_rel}`.\n\n```rust\npub fn e2e_sse_probe_{live_seq}() -> &'static str {{\n    \"{file_rel}\"\n}}\n```\n\nStreamed over `/api/events/stream` while permission remains pending."
                );
                emit_turn_events(
                    &mut out,
                    cwd,
                    "native-e2e-main",
                    &call_id,
                    file_rel,
                    &md_text,
                )?;
            }
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(Some(line)) => line,
                Ok(None) | Err(_) => break,
            }
        };

        let trimmed = msg.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        match val["type"].as_str().unwrap_or("") {
            "control_request" => {
                let req_id = val["request_id"].clone();
                let subtype = val["request"]["subtype"].as_str().unwrap_or("");
                if subtype == "initialize" {
                    emit_line(
                        &mut out,
                        &json!({
                            "type": "control_response",
                            "response": {
                                "subtype": "success",
                                "request_id": req_id,
                                "response": {
                                    "commands": [],
                                    "output_style": "default"
                                }
                            }
                        }),
                    )?;
                } else if subtype == "set_permission_mode" {
                    let mode = val["request"]["mode"].clone();
                    emit_line(
                        &mut out,
                        &json!({
                            "type": "control_response",
                            "response": {
                                "subtype": "success",
                                "request_id": req_id,
                                "response": { "mode": mode }
                            }
                        }),
                    )?;
                }
            }
            "user" => {
                let prompt = val["message"]["content"].as_str().unwrap_or("");
                if prompt.contains(HOLD_MARKER) {
                    emit_line(
                        &mut out,
                        &json!({
                            "type": "system",
                            "subtype": "init",
                            "session_id": "native-e2e-main"
                        }),
                    )?;
                    emit_turn_events(
                        &mut out,
                        cwd,
                        "native-e2e-main",
                        "tool-e2e-read-1",
                        "entry_00.js",
                        "Inspected `entry_00.js` and verified the module dependency graph.\n\n```js\nimport { fn_0_0 } from './pkg_00/mod_00.js';\nexport function runEntry() {\n  return fn_0_0(42);\n}\n```\n\nRequesting permission to update `entry_00.js`.",
                    )?;
                    emit_line(
                        &mut out,
                        &json!({
                            "type": "control_request",
                            "request_id": "perm-e2e-1",
                            "request": {
                                "subtype": "can_use_tool",
                                "tool_name": "Edit",
                                "input": {
                                    "file_path": "entry_00.js",
                                    "old_string": "export function runEntry() { return fn_0_0(1); }",
                                    "new_string": "export function runEntry() { return fn_0_0(42); }"
                                },
                                "decision_reason": "Apply E2E verification edit to entry_00.js",
                                "permission_suggestions": [{
                                    "type": "addRules",
                                    "rules": [{
                                        "toolName": "Edit",
                                        "ruleContent": "entry_00.js"
                                    }]
                                }]
                            }
                        }),
                    )?;
                    holding_permission = true;
                } else {
                    emit_line(
                        &mut out,
                        &json!({
                            "type": "system",
                            "subtype": "init",
                            "session_id": "native-e2e-other"
                        }),
                    )?;
                    emit_turn_events(
                        &mut out,
                        cwd,
                        "native-e2e-other",
                        "tool-e2e-followup-1",
                        "entry_01.js",
                        "Follow-up turn inspected `entry_01.js`.\n\n```js\nexport function verifyFollowup() {\n  return true;\n}\n```\n\nFollow-up complete.",
                    )?;
                    emit_line(
                        &mut out,
                        &json!({
                            "type": "result",
                            "subtype": "success",
                            "is_error": false,
                            "session_id": "native-e2e-other",
                            "result": "Fixture turn complete."
                        }),
                    )?;
                    break;
                }
            }
            "control_response" if holding_permission => {
                let behavior = val["response"]["response"]["behavior"]
                    .as_str()
                    .unwrap_or("allow");
                emit_line(
                    &mut out,
                    &json!({
                        "type": "assistant",
                        "session_id": "native-e2e-main",
                        "message": {
                            "role": "assistant",
                            "content": [{
                                "type": "text",
                                "text": format!("Permission request handled (`{behavior}`).\n\n```js\nexport const permissionResult = '{behavior}';\n```")
                            }]
                        }
                    }),
                )?;
                emit_line(
                    &mut out,
                    &json!({
                        "type": "result",
                        "subtype": "success",
                        "is_error": false,
                        "session_id": "native-e2e-main",
                        "result": "Permission decision completed."
                    }),
                )?;
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

fn fake_provider(arguments: &[String]) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = cwd
        .parent()
        .ok_or_else(|| HarnessError::invalid("No fixture parent."))?;
    let stream_json_input = arguments
        .windows(2)
        .any(|w| w[0] == "--input-format" && w[1] == "stream-json");
    if stream_json_input {
        return handle_stream_json_provider(&cwd, root);
    }

    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt)?;
    let mut out = std::io::stdout().lock();
    emit_line(
        &mut out,
        &json!({
            "type": "system",
            "subtype": "init",
            "session_id": "native-e2e-other"
        }),
    )?;
    emit_turn_events(
        &mut out,
        &cwd,
        "native-e2e-other",
        "tool-e2e-other-1",
        "entry_01.js",
        "Baseline E2E turn verified `entry_01.js`.\n\n```js\nimport { fn_1_0 } from './pkg_01/mod_00.js';\nexport function baseline() {\n  return fn_1_0(7);\n}\n```\n\nBaseline ready.",
    )?;
    emit_line(
        &mut out,
        &json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "session_id": "native-e2e-other",
            "result": "Baseline fixture complete."
        }),
    )?;
    Ok(())
}

fn run_git(project: &Path, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .current_dir(project)
        .args(args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(HarnessError::unavailable(format!(
            "git {} failed: {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn create_fixture_project(project: &Path) -> Result<(String, String)> {
    fs::create_dir_all(project)?;
    let init_main = std::process::Command::new("git")
        .current_dir(project)
        .args(["init", "--quiet", "-b", "main"])
        .status()?;
    if !init_main.success() {
        run_git(project, &["init", "--quiet"])?;
        run_git(project, &["checkout", "-B", "main"])?;
    }

    // 10 subdirectories x 20 files = 200 files + 12 root files + 2 test files = 214 files.
    // Under WHOLE_LIMITS (Unfold all), framed caps at 180 + 12 root files = 192 nodes (>= 180).
    for g in 0..10usize {
        let dir = project.join(format!("pkg_{g:02}"));
        fs::create_dir_all(&dir)?;
        for i in 0..20usize {
            let mut body = String::new();
            if i > 0 {
                body.push_str(&format!(
                    "import {{ fn_{g}_{prev} }} from './mod_{prev:02}.js';\n",
                    prev = i - 1
                ));
            }
            if g < 9 {
                let next_g = g + 1;
                body.push_str(&format!(
                    "import {{ fn_{next_g}_{i} }} from '../pkg_{next_g:02}/mod_{i:02}.js';\n"
                ));
            } else if i == 19 {
                // Intentional cycle + architecture violation from infra (pkg_09) -> domain (pkg_00).
                body.push_str("import { fn_0_19 } from '../pkg_00/mod_19.js';\n");
            }
            body.push_str(&format!(
                "export class Service_{g:02}_{i:02} {{\n  run(v) {{\n    return fn_{g}_{i}(v);\n  }}\n}}\n\nexport function fn_{g}_{i}(v) {{\n  let acc = v + {i};\n"
            ));
            if i > 0 {
                body.push_str(&format!("  acc += fn_{g}_{prev}(v);\n", prev = i - 1));
            }
            if g < 9 {
                body.push_str(&format!("  acc += fn_{next_g}_{i}(v);\n", next_g = g + 1));
            } else if i == 19 {
                body.push_str("  acc += fn_0_19(v);\n");
            }
            body.push_str("  return acc;\n}\n");
            fs::write(dir.join(format!("mod_{i:02}.js")), body)?;
        }
    }

    for k in 0..12usize {
        let pkg = k % 10;
        let extra = if k == 0 {
            "import { fn_0_1 } from './pkg_00/mod_01.js';\n"
        } else {
            ""
        };
        let extra_call = if k == 0 { " + fn_0_1(1)" } else { "" };
        let content = format!(
            "import {{ fn_{pkg}_0 }} from './pkg_{pkg:02}/mod_00.js';\n{extra}export function runEntry_{k:02}() {{\n  return fn_{pkg}_0(1){extra_call};\n}}\nexport function runEntry() {{ return fn_{pkg}_0(1); }}\n"
        );
        fs::write(project.join(format!("entry_{k:02}.js")), content)?;
    }

    let tests_dir = project.join("tests");
    fs::create_dir_all(&tests_dir)?;
    fs::write(
        tests_dir.join("core.test.js"),
        "import { fn_0_0 } from '../pkg_00/mod_00.js';\nexport function testCore() {\n  return fn_0_0(1) > 0;\n}\n",
    )?;
    fs::write(
        tests_dir.join("chain.test.js"),
        "import { fn_1_0 } from '../pkg_01/mod_00.js';\nexport function testChain() {\n  return fn_1_0(2) > 0;\n}\n",
    )?;

    fs::write(
        project.join(".harness-architecture.json"),
        json!({
            "version": 1,
            "layers": [
                {
                    "id": "entry",
                    "name": "Entrypoints",
                    "paths": ["entry_*.js"],
                    "allows": ["domain"]
                },
                {
                    "id": "domain",
                    "name": "Domain",
                    "paths": ["pkg_00/**", "pkg_01/**", "pkg_02/**", "pkg_03/**", "pkg_04/**"],
                    "allows": ["infra"]
                },
                {
                    "id": "infra",
                    "name": "Infrastructure",
                    "paths": ["pkg_05/**", "pkg_06/**", "pkg_07/**", "pkg_08/**", "pkg_09/**"],
                    "allows": []
                }
            ]
        })
        .to_string(),
    )?;

    run_git(project, &["add", "-A"])?;
    run_git(
        project,
        &[
            "-c",
            "user.name=E2E Fixture",
            "-c",
            "user.email=e2e@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "commit A: initial graph",
        ],
    )?;
    let commit_a = run_git(project, &["rev-parse", "HEAD"])?;

    fs::write(
        project.join("entry_00.js"),
        "import { fn_0_0 } from './pkg_00/mod_00.js';\nimport { fn_0_1 } from './pkg_00/mod_01.js';\nexport function runEntry() { return fn_0_0(1) + fn_0_1(2); }\n",
    )?;
    fs::write(
        project.join("pkg_00/mod_00.js"),
        "import { fn_1_0 } from '../pkg_01/mod_00.js';\nexport class Service_00_00 {\n  run(v) { return fn_0_0(v); }\n}\nexport function fn_0_0(v) {\n  return v + 10 + fn_1_0(v);\n}\n",
    )?;
    fs::write(
        project.join("pkg_01/mod_00.js"),
        "import { fn_2_0 } from '../pkg_02/mod_00.js';\nexport class Service_01_00 {\n  run(v) { return fn_1_0(v); }\n}\nexport function fn_1_0(v) {\n  return v + 20 + fn_2_0(v);\n}\n",
    )?;
    fs::write(
        project.join("pkg_00/mod_20.js"),
        "import { fn_0_0 } from './mod_00.js';\nexport function fn_0_20(v) {\n  return fn_0_0(v) + 20;\n}\n",
    )?;

    run_git(project, &["add", "-A"])?;
    run_git(
        project,
        &[
            "-c",
            "user.name=E2E Fixture",
            "-c",
            "user.email=e2e@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "commit B: range diff updates",
        ],
    )?;
    let commit_b = run_git(project, &["rev-parse", "HEAD"])?;

    // Uncommitted edit so Changes lens (`target=work`) & Changes & Diff tab have active diff data.
    fs::write(
        project.join("entry_01.js"),
        "import { fn_1_0 } from './pkg_01/mod_00.js';\nexport function runEntry_01() {\n  return fn_1_0(2);\n}\nexport function verifyUncommitted() {\n  return fn_1_0(3);\n}\n",
    )?;

    Ok((commit_a, commit_b))
}

fn spawn_shutdown_watchers(
    shutdown: tokio::sync::watch::Sender<bool>,
    timeout_secs: u64,
) -> tokio::task::JoinHandle<()> {
    #[cfg(unix)]
    {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let is_pipe = unsafe { libc::fstat(0, &mut stat) == 0 }
            && (stat.st_mode & libc::S_IFMT) == libc::S_IFIFO;
        if is_pipe {
            let stdin_shutdown = shutdown.clone();
            std::thread::spawn(move || {
                let mut stdin = std::io::stdin();
                let mut buf = [0u8; 256];
                loop {
                    match stdin.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => continue,
                    }
                }
                let _ = stdin_shutdown.send(true);
            });
        }
    }

    let sig_shutdown = shutdown.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = async {
                    if let Some(ref mut s) = sigterm {
                        s.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let _ = sig_shutdown.send(true);
    });

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(timeout_secs)).await;
        let _ = shutdown.send(true);
    })
}

async fn serve(root: &Path, ui_root: &Path, source_root: &Path, timeout_secs: u64) -> Result<()> {
    let root = infra_core::fs::canonicalize(root)?;
    let ui_root = infra_core::fs::canonicalize(ui_root)?;
    let source = infra_core::fs::canonicalize(source_root)?;

    let project = root.join("project");
    let (commit_a, commit_b) = create_fixture_project(&project)?;

    let store = SessionStore::open(&root.join("state"))?;
    let registered = store.register(&project)?;
    let project_id = registered["id"]
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Missing registered project id."))?
        .to_owned();

    let providers = Arc::new(ProviderDomain {
        providers: Arc::new(RwLock::new(BTreeMap::from([(
            "claude".into(),
            Provider {
                id: "claude".into(),
                name: "Offline E2E Claude".into(),
                available: true,
                executable: Some(std::env::current_exe()?),
                detail: "Offline E2E fixture provider".into(),
            },
        )]))),
    });
    infra_host::model_catalog::model_options("claude", None, None).await?;

    let knowledge = Arc::new(KnowledgeDomain::new(store.clone(), source.clone()));
    let graphs = Graphs::with_native_backend(store.clone(), Duration::from_secs(1));
    knowledge.install_graphs(graphs.clone());
    let code_graph_bridge = HarnessCodeGraphBridge::new(store.clone(), graphs.clone());

    let special = ["creator", "system_run", "system_discovery"]
        .into_iter()
        .map(|name| (name.into(), Arc::new(Unused) as Arc<dyn SpecialWorkflow>))
        .collect();
    let sessions = Sessions::new(
        store.clone(),
        source.clone(),
        providers.clone(),
        knowledge.clone(),
        Arc::new(Unused),
        special,
        SessionLimits {
            seconds: 600,
            concurrency: 2,
        },
    )?;
    let links = Arc::new(SessionLinks::new());
    links.bind(Arc::downgrade(&sessions))?;

    let results = Arc::new(Results::new(
        store.clone(),
        Arc::new(ResultsBridge {
            links: links.clone(),
            knowledge: knowledge.clone(),
            operation: sessions.operation_lock(),
            cancel: sessions.shutdown_cancel(),
        }),
    )?);
    let omnigraph = Arc::new(OmnigraphManager::new(
        store.state.clone(),
        Some(store.clone()),
    )?);
    let omnigraph_domain = Arc::new(OmnigraphDomain::new(omnigraph.clone()));
    sessions.install_omnigraph(omnigraph.clone())?;
    results.install_omnigraph(omnigraph.clone())?;
    sessions.install_results(results.clone())?;
    sessions.install_commands(Arc::new(CommandCatalog::new(
        store.clone(),
        source.clone(),
        providers.providers.clone(),
    )))?;
    let accelerators = Arc::new(Accelerators::new(
        store.clone(),
        source.clone(),
        providers.clone(),
        Arc::downgrade(&sessions),
    )?);
    sessions.install_accelerators(accelerators.clone())?;
    sessions.install_graphs(graphs.clone())?;

    let delivery = Arc::new(Delivery::open(store.clone(), results.clone()).await?);
    let target: Arc<dyn infra_harness::results::TargetCheckWorkflow> = delivery.clone();
    results.bind_target(Arc::downgrade(&target))?;

    let system_bridge = Arc::new(SystemBridge {
        links,
        store: store.clone(),
    });
    let system_manager = Arc::new(infra_system::SystemManager::new(
        source.clone(),
        store.state.clone(),
        system_bridge,
    )?);
    system_manager.set_code_graph_bridge(code_graph_bridge.clone())?;
    let systems = Arc::new(SystemsDomain::new(system_manager.clone()));
    let system_graphs = Arc::new(SystemGraphDomain::new(
        store.clone(),
        graphs.clone(),
        code_graph_bridge,
        Some(system_manager),
    ));
    let creator = Arc::new(CreatorManager::new(
        store.clone(),
        source.clone(),
        Arc::downgrade(&sessions),
    )?);
    let setup = Arc::new(SetupDomain::new(Arc::new(Setup::new(
        store.clone(),
        source.clone(),
        Arc::downgrade(&sessions),
        accelerators.clone(),
    ))));
    let skills = Arc::new(SkillsDomain::new(store.clone(), source.clone())?);
    let projects_domain = Arc::new(ProjectDomain {
        store: store.clone(),
    });

    sessions.activate()?;

    // Pre-build native SQLite Graph IR synchronously before serving HTTP.
    let graphs_build = graphs.clone();
    let build_pid = project_id.clone();
    tokio::task::spawn_blocking(move || graphs_build.act(&build_pid, "enable"))
        .await
        .map_err(|_| HarnessError::unavailable("Graph build task panicked."))??;
    wait_until(
        Duration::from_secs(30),
        "Native Graph IR build timed out.",
        || {
            let st = graphs.status(&project_id, false)?;
            if st["state"] == "failed" {
                return Err(HarnessError::unavailable(format!(
                    "Native Graph IR build failed: {}",
                    st["failure"]
                )));
            }
            Ok(st["state"] == "ready")
        },
    )
    .await?;

    let whole_view = {
        let g = graphs.clone();
        let pid = project_id.clone();
        tokio::task::spawn_blocking(move || {
            let shown = Shown {
                whole: true,
                ..Shown::default()
            };
            g.map(&pid, None, &shown)
        })
        .await
        .map_err(|_| HarnessError::unavailable("Graph map verification task panicked."))??
    };
    let node_count = whole_view["nodes"].as_array().map_or(0, Vec::len);
    require(
        node_count >= 180,
        &format!(
            "Expected >= 180 whole-graph nodes for dense Canvas2D threshold, got {node_count}"
        ),
    )?;

    // Create completed secondary session first so the primary active session can hold the repo lease cleanly.
    let other_created = json_payload(
        sessions
            .execute(
                "sessions.create",
                json!({
                    "body": {
                        "project_id": project_id,
                        "provider": "claude",
                        "workflow": "native",
                        "mode": "edit",
                        "permissions": "bypass",
                        "agents_enabled": false,
                        "agent_count": 1,
                        "project_context": false,
                        "budgets": { "seconds": 600 },
                        "prompt": "Baseline E2E inspection of entry_01.js."
                    }
                }),
            )
            .await?,
    )?;
    let other_session_id = other_created["session"]["id"]
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Missing other session id."))?
        .to_owned();
    wait_until(
        Duration::from_secs(15),
        "Secondary fixture session did not complete.",
        || Ok(store.get(&other_session_id)?["status"] == "completed"),
    )
    .await?;

    // Create primary active session holding a real pending permission + initial tool & Markdown events.
    let main_created = json_payload(
        sessions
            .execute(
                "sessions.create",
                json!({
                    "body": {
                        "project_id": project_id,
                        "provider": "claude",
                        "workflow": "native",
                        "mode": "edit",
                        "permissions": "manual",
                        "agents_enabled": false,
                        "agent_count": 1,
                        "project_context": false,
                        "budgets": { "seconds": 600 },
                        "prompt": format!("{HOLD_MARKER} Inspect entry_00.js and propose an edit.")
                    }
                }),
            )
            .await?,
    )?;
    let session_id = main_created["session"]["id"]
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Missing primary session id."))?
        .to_owned();
    wait_until(
        Duration::from_secs(15),
        "Primary fixture session did not reach pending permission state.",
        || {
            let s = store.get(&session_id)?;
            let events = store.events(&session_id, 0)?;
            let has_tool = events.iter().any(|ev| ev["kind"] == "tool");
            let has_text = events.iter().any(|ev| ev["kind"] == "text");
            Ok(s["status"] == "running"
                && s["native_session_id"] == "native-e2e-main"
                && has_tool
                && has_text
                && sessions.waiting_permissions(&session_id).len() == 1)
        },
    )
    .await?;

    let offline: Arc<dyn Domain> = Arc::new(Offline);
    let mut domains: BTreeMap<String, Arc<dyn Domain>> = BTreeMap::new();
    for group in ["sessions", "brain", "context"] {
        domains.insert(group.into(), sessions.clone());
    }
    domains.insert("setup".into(), setup.clone());
    domains.insert("accelerators".into(), accelerators.clone());
    domains.insert("systems".into(), systems.clone());
    domains.insert("discovery".into(), systems.clone());
    domains.insert("creator".into(), creator);
    domains.insert("skills".into(), skills);
    domains.insert("results".into(), Arc::new(ResultService { results }));
    domains.insert("delivery".into(), delivery);
    domains.insert("projects".into(), projects_domain.clone());
    domains.insert("files".into(), projects_domain);
    domains.insert(
        "commands".into(),
        Arc::new(CommandCatalog::new(
            store.clone(),
            source.clone(),
            providers.providers.clone(),
        )),
    );
    for group in ["memory", "knowledge"] {
        domains.insert(group.into(), knowledge.clone());
    }
    domains.insert("updates".into(), offline.clone());
    domains.insert("providers".into(), providers.clone());
    domains.insert("graph".into(), graphs.clone());
    domains.insert("system_graph".into(), system_graphs);
    domains.insert("omnigraph".into(), omnigraph_domain);

    let inbox_trigger: Arc<dyn Domain> = Arc::new(InboxLiveTrigger {
        sessions: sessions.clone(),
        root: root.clone(),
        seq: AtomicU64::new(0),
    });
    let mut overrides: BTreeMap<String, Arc<dyn Domain>> = BTreeMap::new();
    overrides.insert("updates.status".into(), offline.clone());
    overrides.insert("providers.sign_in".into(), offline);
    overrides.insert("sessions.inbox_accept".into(), inbox_trigger);
    overrides.insert("context.project".into(), knowledge);
    overrides.insert("memory.save".into(), sessions.clone());

    let boot_store = store.clone();
    let boot_acc = accelerators.clone();
    let boot_providers = providers.clone();
    let boot_omni = omnigraph
        .status_json("")
        .await
        .unwrap_or_else(|_| json!({"connected": false, "state": "unconfigured", "mode": "off"}));
    let source_version = accelerators
        .source_version()
        .unwrap_or_else(|_| "e2e-fixture".to_owned());
    let engine_desc = infra_tooling::installer::Installer::new(&source)
        .ok()
        .and_then(|inst| inst.engine_capabilities().ok())
        .unwrap_or_else(
            || json!({"engine": "infra-engine", "version": source_version, "capabilities": []}),
        );

    let closing_sessions = sessions.clone();
    let closing_graphs = graphs.clone();
    let closing_systems = systems;
    let closing_setup = setup;
    let closing_omni = omnigraph;
    let services = Arc::new(NativeServices::new(
        store,
        domains,
        Arc::new(move || {
            let mut projects = Vec::new();
            for mut p in boot_store.projects()? {
                p["accelerator"] = boot_acc.summary(p["id"].as_str().unwrap_or(""))?;
                projects.push(p);
            }
            Ok(json!({
                "projects": projects,
                "providers": boot_providers.listing()?,
                "sessions": boot_store.summaries()?,
                "update": { "available": false, "state": "fixture" },
                "omnigraph": boot_omni,
                "workflows": [
                    { "id": "native", "name": "Workspace", "description": "Native workspace agent." }
                ],
                "sdd_phases": [],
                "clash": { "workflows": ["native"], "stages": {}, "max_rounds": 3, "default_rounds": 2 },
                "accelerators": [],
                "runtime": {
                    "engine": engine_desc,
                    "source_version": boot_acc.source_version().unwrap_or_else(|_| "e2e-fixture".to_owned()),
                    "timeout_seconds": 600,
                    "max_active": 2,
                    "discovery_sandbox": Value::Null,
                    "max_agents": 40,
                    "default_agent_count": 3,
                    "context_excerpt_bytes": 3000,
                    "fleet": { "available": false, "backend": "native", "max_worker_timeout": 300, "lenses": [] }
                }
            }))
        }),
        overrides,
        Arc::new(move || {
            let sessions = closing_sessions.clone();
            let graphs = closing_graphs.clone();
            let systems = closing_systems.clone();
            let setup = closing_setup.clone();
            let omni = closing_omni.clone();
            Box::pin(async move {
                graphs.close();
                let requested = sessions.close();
                setup.close().await;
                systems.close().await;
                let reaped = sessions.wait_closed().await;
                let _ = omni.close().await;
                requested.and(reaped)
            }) as CloseFuture<'static>
        }),
    )?);

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let app = http::App::new(services.clone(), port, ui_root, shutdown.clone())?;
    let receipt = json!({
        "url": format!("http://127.0.0.1:{port}"),
        "project_id": project_id,
        "session_id": session_id,
        "other_session_id": other_session_id,
        "commit_a": commit_a,
        "commit_b": commit_b,
        "root": root,
    });
    fs::write(root.join("browser-e2e.json"), receipt.to_string())?;
    {
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{receipt}")?;
        stdout.flush()?;
    }

    let deadline = spawn_shutdown_watchers(shutdown.clone(), timeout_secs);
    let serve_result = http::serve_listener(listener, app).await;
    let _ = shutdown.send(true);
    deadline.abort();
    let close_result = services.close().await;
    serve_result.and(close_result)
}

struct CliOptions {
    state_root: Option<PathBuf>,
    ui_root: Option<PathBuf>,
    source_root: Option<PathBuf>,
    timeout_secs: u64,
}

fn parse_cli(args: &[String]) -> Result<CliOptions> {
    let mut opts = CliOptions {
        state_root: None,
        ui_root: None,
        source_root: None,
        timeout_secs: DEFAULT_TIMEOUT_SECS,
    };
    let mut i = 0usize;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--ui-root" {
            i += 1;
            let val = args
                .get(i)
                .ok_or_else(|| HarnessError::invalid("Missing --ui-root value."))?;
            opts.ui_root = Some(PathBuf::from(val));
        } else if let Some(val) = arg.strip_prefix("--ui-root=") {
            opts.ui_root = Some(PathBuf::from(val));
        } else if arg == "--source" {
            i += 1;
            let val = args
                .get(i)
                .ok_or_else(|| HarnessError::invalid("Missing --source value."))?;
            opts.source_root = Some(PathBuf::from(val));
        } else if let Some(val) = arg.strip_prefix("--source=") {
            opts.source_root = Some(PathBuf::from(val));
        } else if arg == "--root" {
            i += 1;
            let val = args
                .get(i)
                .ok_or_else(|| HarnessError::invalid("Missing --root value."))?;
            opts.state_root = Some(PathBuf::from(val));
        } else if let Some(val) = arg.strip_prefix("--root=") {
            opts.state_root = Some(PathBuf::from(val));
        } else if arg == "--timeout-secs" {
            i += 1;
            let val = args
                .get(i)
                .ok_or_else(|| HarnessError::invalid("Missing --timeout-secs value."))?;
            let secs = val
                .parse::<u64>()
                .map_err(|_| HarnessError::invalid("Invalid --timeout-secs value."))?;
            opts.timeout_secs = secs.clamp(5, 3600);
        } else if let Some(val) = arg.strip_prefix("--timeout-secs=") {
            let secs = val
                .parse::<u64>()
                .map_err(|_| HarnessError::invalid("Invalid --timeout-secs value."))?;
            opts.timeout_secs = secs.clamp(5, 3600);
        } else if !arg.starts_with('-') && opts.state_root.is_none() {
            opts.state_root = Some(PathBuf::from(arg));
        } else {
            return Err(HarnessError::invalid(format!("Unexpected argument: {arg}")));
        }
        i += 1;
    }
    Ok(opts)
}

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().is_some_and(|arg| arg == "internal-guard") {
        std::process::exit(infra_host::process::guard_entry(&arguments[1..]).unwrap_or(125));
    }
    if arguments.iter().any(|arg| arg == "--print") {
        if let Err(error) = fake_provider(&arguments) {
            eprintln!("{}", error.message);
            std::process::exit(2);
        }
        return;
    }
    if arguments.first().is_some_and(|arg| arg == "auth") {
        println!("{{\"loggedIn\":true,\"apiProvider\":\"firstParty\"}}");
        return;
    }

    let opts = match parse_cli(&arguments) {
        Ok(opts) => opts,
        Err(error) => {
            eprintln!("{}", error.message);
            std::process::exit(2);
        }
    };
    let repo_root = find_repo_root();
    let ui_root = opts
        .ui_root
        .or_else(|| {
            std::env::var_os("HARNESS_UI_ROOT")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| repo_root.clone())
        .unwrap_or_else(|| {
            eprintln!("Could not locate UI root. Pass --ui-root or set HARNESS_UI_ROOT.");
            std::process::exit(2);
        });
    let source_root = opts
        .source_root
        .or(repo_root)
        .unwrap_or_else(|| ui_root.clone());

    let _owned_temp = if opts.state_root.is_none() {
        Some(tempfile::tempdir().expect("create fixture temp dir"))
    } else {
        None
    };
    let state_root = opts
        .state_root
        .unwrap_or_else(|| _owned_temp.as_ref().expect("tempdir").path().to_path_buf());
    if let Err(err) = fs::create_dir_all(&state_root) {
        eprintln!("Failed to create fixture state root: {err}");
        std::process::exit(2);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .build()
        .expect("build tokio runtime");
    if let Err(error) = runtime.block_on(serve(
        &state_root,
        &ui_root,
        &source_root,
        opts.timeout_secs,
    )) {
        eprintln!("{}", error.message);
        std::process::exit(2);
    }
}
