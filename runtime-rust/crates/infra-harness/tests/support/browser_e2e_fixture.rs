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
    omnigraph::{
        HARNESS_CONTEXT_PG, HARNESS_GRAPH_ID, OmnigraphDomain, OmnigraphManager,
        session_branch_name,
    },
    projects::ProjectDomain,
    provider::{Provider, ProviderDomain},
    results::{ResultService, Results},
    services::{CloseFuture, Domain, NativeServices, Payload, Reply, Services},
    sessions::{BlackboardFactory, Event, RunReply, SessionLimits, Sessions, SpecialWorkflow},
    setup::{Setup, SetupDomain},
    skillcache::SkillCacheDomain,
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
const SKILLCACHE_CAPSULE: &str = "e2ecapsule000001";
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
                                    "commands": [
                                        {
                                            "name": "model",
                                            "description": "Choose the model for the next turn",
                                            "argumentHint": "[model]",
                                            "builtin": true
                                        },
                                        {
                                            "name": "new",
                                            "description": "Start a new session",
                                            "builtin": true
                                        }
                                    ],
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

    let pdf_skill = project.join(".agents/skills/pdf");
    fs::create_dir_all(&pdf_skill)?;
    fs::write(
        pdf_skill.join("SKILL.md"),
        "---\nname: pdf\ndescription: Extract rows from PDF documents\n---\n\n# PDF\n\nExtract the rows and check the totals.\n",
    )?;

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

    fs::create_dir_all(project.join("coverage"))?;
    fs::write(
        project.join("coverage/lcov.info"),
        concat!(
            "TN:testCore\n",
            "SF:pkg_00/mod_00.js\n",
            "DA:1,1\n",
            "DA:2,1\n",
            "DA:4,1\n",
            "DA:5,1\n",
            "LF:4\n",
            "LH:4\n",
            "end_of_record\n",
            "TN:testChain\n",
            "SF:pkg_01/mod_00.js\n",
            "DA:1,1\n",
            "DA:4,1\n",
            "LF:2\n",
            "LH:2\n",
            "end_of_record\n",
            "TN:uncovered\n",
            "SF:pkg_00/mod_02.js\n",
            "DA:1,0\n",
            "DA:2,0\n",
            "LF:2\n",
            "LH:0\n",
            "end_of_record\n",
        ),
    )?;

    // Seed multi-service System Orchestration workspace inside `project` (3 waves + unmatched risk).
    fs::write(
        project.join("ARCHITECTURE.md"),
        "# Shared System Architecture\nContracts between billing, orders, and gateway.\n",
    )?;
    for (svc, rel_file, code, provides, consumes, cap_desc) in [
        (
            "billing",
            "src/BillingLedger.js",
            "export function emitBillingInvoice(id) {\n  return { id, event: 'billing-events-v1' };\n}\n",
            json!([{
                "id": "billing-events-v1",
                "kind": "event",
                "version": "1.0.0",
                "sources": ["src/BillingLedger.js"]
            }]),
            json!([]),
            "Core billing ledger event emitter",
        ),
        (
            "orders",
            "src/OrderProcessor.js",
            "// Consumes billing-events-v1\nexport function processOrder() {\n  return 'orders-http-v1';\n}\n",
            json!([{
                "id": "orders-http-v1",
                "kind": "http",
                "version": "1.0.0",
                "sources": ["src/OrderProcessor.js"]
            }]),
            json!([{
                "service": "billing",
                "contract": "billing-events-v1",
                "version": "1.0.0"
            }]),
            "Core order processing HTTP service",
        ),
        (
            "gateway",
            "src/GatewayProxy.js",
            "// Consumes orders-http-v1 and ghost-rpc-v1\nexport function routeGateway() {\n  return true;\n}\n",
            json!([]),
            json!([
                {
                    "service": "orders",
                    "contract": "orders-http-v1",
                    "version": "2.0.0"
                },
                {
                    "service": "payments",
                    "contract": "payments-rpc-v1",
                    "version": "1.0.0"
                }
            ]),
            "Core API gateway router",
        ),
    ] {
        let svc_dir = project.join("services").join(svc);
        fs::create_dir_all(svc_dir.join("src"))?;
        fs::write(svc_dir.join(rel_file), code)?;
        let passport = json!({
            "schema_version": 1,
            "id": svc,
            "description": cap_desc,
            "owner": "platform",
            "relationships_complete": true,
            "capabilities": [{
                "id": format!("{svc}-core"),
                "description": cap_desc,
                "status": "implemented",
                "sources": [rel_file]
            }],
            "provides": provides,
            "consumes": consumes,
            "sources": [{"path": rel_file, "kind": "code"}]
        });
        fs::write(
            svc_dir.join("ai-service.json"),
            serde_json::to_vec_pretty(&passport)?,
        )?;
    }
    // Un-manifested 4th service so gateway->payments triggers unmatched contract risk on unfold.
    let payments_dir = project.join("services/payments/src");
    fs::create_dir_all(&payments_dir)?;
    fs::write(
        payments_dir.join("PaymentsStub.js"),
        "export function pingPayments() { return true; }\n",
    )?;
    fs::write(
        project.join("system.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "name": "e2e-system-platform",
            "shared_sources": [{"path": "ARCHITECTURE.md", "kind": "spec"}],
            "services": [
                {"id": "billing", "root": "services/billing", "manifest": "ai-service.json"},
                {"id": "orders", "root": "services/orders", "manifest": "ai-service.json"},
                {"id": "gateway", "root": "services/gateway", "manifest": "ai-service.json"},
                {"id": "payments", "root": "services/payments", "manifest": "ai-service.json"}
            ]
        }))?,
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

/// What the Skill capsules view and the run view's SkillCache tab show: a
/// capsule that waits for a person, with its package, gates, compilation and
/// warm turns, and the SkillCache events of one turn of the finished session.
/// Nothing is compiled or run here; the rows are written as the Harness
/// would have left them.
fn seed_skillcache(
    store: &SessionStore,
    project: &Path,
    project_id: &str,
    session_id: &str,
) -> Result<()> {
    let generation = store
        .events(session_id, 0)?
        .iter()
        .find_map(|event| event["launch_id"].as_str().map(str::to_owned))
        .ok_or_else(|| HarnessError::invalid("The finished fixture session has no launch."))?;
    let skill_path = ".agents/skills/pdf/SKILL.md";
    let tree = infra_skillcache::fingerprint::tree_sha256(&project.join(".agents/skills/pdf"))
        .map_err(|error| HarnessError::unavailable(error.to_string()))?;
    let zeros = "0".repeat(64);
    let capsule = json!({
        "schema_version": 1,
        "form": "hybrid",
        "mu": {"name": "pdf-rows", "description": "Rows of every invoice PDF in a folder"},
        "source": {"skills": [{"name": "pdf", "path": skill_path, "tree_sha256": tree.sha256}]},
        "capabilities": {"network": "none", "runtimes": ["sh"]},
        "bindings": [{"name": "input_dir", "type": "path", "origin": "request",
                      "description": "Folder with the invoices"}],
        "programs": [
            {"id": "extract", "description": "Reads every PDF and writes the rows",
             "entry": ["sh", "programs/extract/main.sh"], "inputs": ["input_dir"],
             "outputs": ["rows_file"], "check": ["sh", "programs/extract/main.sh", "--check"],
             "timeout_seconds": 60, "read_only": false},
            {"id": "verify", "entry": ["sh", "programs/verify/main.sh"], "inputs": ["rows_file"],
             "outputs": [], "check": ["sh", "programs/verify/main.sh", "--check"],
             "timeout_seconds": 60, "read_only": true}
        ],
        "steps": [
            {"kind": "program", "program": "extract"},
            {"kind": "instruction", "text": "Confirm the totals against the invoice headers.",
             "source": {"path": skill_path, "section": "Totals"}}
        ],
        "verification": [{"kind": "program", "program": "verify"}]
    });
    let parsed = infra_skillcache::SkillCapsule::parse(&serde_json::to_vec(&capsule)?)
        .map_err(|_| HarnessError::invalid("The fixture capsule is not a valid capsule."))?;
    let mut files = infra_skillcache::package::Files::new();
    files.insert(
        "programs/extract/main.sh",
        b"printf '{\"status\":\"complete\",\"result\":{\"rows_file\":\"out/q3-rows.csv\"}}\\n'\n"
            .to_vec(),
    );
    files.insert(
        "programs/verify/main.sh",
        b"printf '{\"status\":\"complete\",\"result\":{}}\\n'\n".to_vec(),
    );
    let artifacts = store
        .state
        .join("skillcache/capsules")
        .join(project_id)
        .join(SKILLCACHE_CAPSULE)
        .join("1");
    let package_sha256 =
        infra_skillcache::package::write_warm(&artifacts.join("warm"), &parsed, None, &files)
            .map_err(|error| HarnessError::unavailable(error.to_string()))?;
    let qual_dir = store
        .state
        .join("skillcache/qualification")
        .join(SKILLCACHE_CAPSULE)
        .join("1");
    infra_core::fs::ensure_directory(&qual_dir, true)?;
    infra_core::fs::atomic_json(
        &qual_dir.join("examples.json"),
        &json!({
            "bindings": {"input_dir": "docs/q3-pdfs"},
            "steps": {"0": {"result": {"rows_file": "out/q3-rows.csv"}}}
        }),
    )?;
    infra_harness::skillcache::snapshot::take(&store.state, project, &generation)?;
    let skill_files = json!({
        "skills": [{
            "name": "pdf",
            "path": skill_path,
            "tree_sha256": tree.sha256,
            "taken_ns": tree.taken_ns.to_string(),
            "files": tree.files.iter().map(|file| json!({
                "path": file.path,
                "size": file.size,
                "mtime_ns": file.mtime_ns.to_string(),
            })).collect::<Vec<_>>()
        }]
    });
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let compile_id = "e2ecompile000001";
    let reason = "2 of its last 3 warm turns handed a program step back.";
    {
        let db = store.connection()?;
        db.execute(
            "INSERT INTO skillcache_traces(generation,session_id,provider,model,request,skill_refs,steps,usage,redaction,trace_sha256,created_at)
             VALUES(?1,?2,'claude',NULL,'Extract the rows of invoices/',?3,'[]',NULL,'{}',?4,?5)",
            rusqlite::params![
                generation,
                session_id,
                json!([{"name": "pdf", "path": skill_path}]).to_string(),
                zeros,
                now
            ],
        )?;
        db.execute(
            "INSERT INTO skillcache_capsules(capsule_id,project_id,skill_name,skill_path,mu_name,mu_description,form,status,revision,capsule_sha256,tree_sha256,skill_files,env_fingerprint,schema_version,compile_rules_version,source_session_id,source_generation,compile_id,artifact_dir,created_at,updated_at,last_used_at,package_sha256,capabilities,reason)
             VALUES(?1,?2,'pdf',?3,'pdf-rows','Rows of every invoice PDF in a folder','hybrid','needs_recompile',1,?4,?5,?6,'e2e',1,'e2e',?7,?8,?9,?10,?11,?11,?11,?12,?13,?14)",
            rusqlite::params![
                SKILLCACHE_CAPSULE,
                project_id,
                skill_path,
                zeros,
                tree.sha256,
                skill_files.to_string(),
                session_id,
                generation,
                compile_id,
                artifacts.to_string_lossy(),
                now,
                package_sha256,
                json!({"network": "none", "runtimes": ["sh"]}).to_string(),
                reason
            ],
        )?;
        db.execute(
            "INSERT INTO skillcache_capsules_fts(capsule_id,project_id,skill_name,mu_name,mu_description)
             VALUES(?1,?2,'pdf','pdf-rows','Rows of every invoice PDF in a folder')",
            rusqlite::params![SKILLCACHE_CAPSULE, project_id],
        )?;
        for (round, gate, passed, report) in [
            (
                1,
                "gate1",
                false,
                json!({"gate": "gate1", "passed": false, "violations": [
                    {"code": "output_unread", "at": "programs[0].outputs[1]", "message": "`pages` is produced and never read."}
                ]}),
            ),
            (
                2,
                "gate1",
                true,
                json!({"gate": "gate1", "passed": true, "violations": []}),
            ),
            (
                2,
                "gate2",
                true,
                json!({"gate": "gate2", "passed": true, "violations": []}),
            ),
            (
                2,
                "gate3",
                true,
                json!({"gate": "gate3", "passed": true, "violations": [], "completed": [0, 1, 2]}),
            ),
        ] {
            db.execute(
                "INSERT INTO skillcache_gates(capsule_id,round,gate,passed,report,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
                rusqlite::params![SKILLCACHE_CAPSULE, round, gate, i64::from(passed), report.to_string(), now],
            )?;
        }
        db.execute(
            "INSERT INTO skillcache_compiles(compile_id,project_id,source_session_id,source_generation,status,provider,model,rounds,outcome,usage,receipt,started_at,finished_at,capsule_id)
             VALUES(?1,?2,?3,?4,'completed','claude',NULL,2,'Qualified as a hybrid capsule in round 2.',?5,'{}',?6,?6,?7)",
            rusqlite::params![
                compile_id,
                project_id,
                session_id,
                generation,
                json!({"input_tokens": 12000, "output_tokens": 3400, "cost_usd": 0.0512}).to_string(),
                now,
                SKILLCACHE_CAPSULE
            ],
        )?;
        for (run, outcome, deopts) in [
            ("e2e-run-1", "complete", 0),
            ("e2e-run-2", "complete", 1),
            ("e2e-run-3", "complete", 1),
        ] {
            db.execute(
                "INSERT INTO skillcache_runs(generation,session_id,capsule_id,provider,model,outcome,steps,deopts,created_at)
                 VALUES(?1,?2,?3,'codex',NULL,?4,'[]',?5,?6)",
                rusqlite::params![run, session_id, SKILLCACHE_CAPSULE, outcome, deopts, now],
            )?;
        }
    }
    for event in [
        json!({"kind": "skillcache", "phase": "observed", "generation": generation, "steps": 2,
               "tool_calls": 2, "skills": ["pdf"],
               "text": "SkillCache kept this turn's trace (2 tool calls; skill: pdf). It can be compiled into a capsule."}),
        json!({"kind": "skillcache", "phase": "hit", "capsule_id": SKILLCACHE_CAPSULE, "form": "hybrid", "mu": "pdf-rows",
               "text": "SkillCache: capsule `pdf-rows` runs this request instead of the skill being read again."}),
        json!({"kind": "skillcache", "phase": "started", "capsule_id": SKILLCACHE_CAPSULE, "seq": 1,
               "text": "SkillCache: the plan started (3 steps)."}),
        json!({"kind": "skillcache", "phase": "program_failed", "capsule_id": SKILLCACHE_CAPSULE, "seq": 2,
               "step_index": 0, "program": "extract", "reason": "nonzero_exit", "verification": false,
               "text": "SkillCache: program `extract` failed (nonzero_exit); step 1 is the agent's now, the rest of the plan stays compiled."}),
        json!({"kind": "skillcache", "phase": "complete", "capsule_id": SKILLCACHE_CAPSULE, "seq": 4,
               "text": "SkillCache: the plan is complete (1 failed)."}),
    ] {
        let mut event = event;
        event["launch_id"] = generation.clone().into();
        store.event(session_id, &event)?;
    }
    Ok(())
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
        providers: Arc::new(RwLock::new(BTreeMap::from([
            (
                "claude".into(),
                Provider {
                    id: "claude".into(),
                    name: "Offline E2E Claude".into(),
                    available: true,
                    executable: Some(std::env::current_exe()?),
                    detail: "Offline E2E fixture provider".into(),
                },
            ),
            (
                "codex".into(),
                Provider {
                    id: "codex".into(),
                    name: "Offline E2E Codex".into(),
                    available: false,
                    executable: None,
                    detail: "Offline placeholder provider".into(),
                },
            ),
            (
                "cursor".into(),
                Provider {
                    id: "cursor".into(),
                    name: "Offline E2E Cursor".into(),
                    available: false,
                    executable: None,
                    detail: "Offline placeholder provider".into(),
                },
            ),
        ]))),
    });
    for id in ["claude", "codex", "cursor"] {
        infra_host::model_catalog::model_options(id, None, None).await?;
    }

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
    seed_skillcache(&store, &project, &project_id, &other_session_id)?;

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
    let skillcache = Arc::new(SkillCacheDomain::new(
        store.clone(),
        providers.clone(),
        sessions.shutdown_cancel(),
    ));
    sessions.install_skillcache(skillcache.runtime())?;
    domains.insert("skillcache".into(), skillcache);

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

    let closing_sessions = sessions.clone();
    let closing_graphs = graphs.clone();
    let closing_systems = systems.clone();
    let closing_setup = setup;
    let closing_omni = omnigraph.clone();

    let system_prep = systems
        .execute(
            "systems.prepare",
            json!({
                "project_id": project_id,
                "config_path": "system.json",
                "change_id": "e2e-wave-change",
                "task": "Evolve billing events and inspect downstream orders + gateway waves",
                "services": ["billing"],
                "depth": 32,
                "budget": 8000,
            }),
        )
        .await?;
    let system_prep_val = json_payload(system_prep)?;
    let system_run_id = system_prep_val["id"]
        .as_str()
        .or_else(|| system_prep_val["run"]["id"].as_str())
        .map(str::to_owned);

    let (omni_url, omni_shutdown, omni_task) =
        offline_omnigraph::spawn(&project_id, &session_id, &other_session_id).await?;
    omnigraph
        .connect_external(&omni_url, Some("tok-e2e"), Some(HARNESS_GRAPH_ID))
        .await?;

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

    let skillcache_requalify = if cfg!(target_os = "linux") {
        std::process::Command::new("bwrap")
            .args(["--ro-bind", "/", "/", "--unshare-net", "/usr/bin/true"])
            .output()
            .is_ok_and(|output| output.status.success())
    } else {
        cfg!(target_os = "macos")
    };
    let receipt = json!({
        "url": format!("http://127.0.0.1:{port}"),
        "project_id": project_id,
        "session_id": session_id,
        "other_session_id": other_session_id,
        "commit_a": commit_a,
        "commit_b": commit_b,
        "system_config_path": "system.json",
        "system_run_id": system_run_id,
        "omnigraph_graph_id": HARNESS_GRAPH_ID,
        "skillcache_capsule_id": SKILLCACHE_CAPSULE,
        "skillcache_requalify": skillcache_requalify,
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
    let _ = omni_shutdown.send(true);
    deadline.abort();
    let close_result = services.close().await;
    let _ = omni_task.await;
    // The seeded capsule's package is sealed read-only; whoever made the
    // state folder removes it, so it is given back first.
    infra_skillcache::package::release(&root.join("state/skillcache"));
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
    if arguments.first().is_some_and(|arg| arg == "skillcache") {
        let (name, args) = arguments[1..]
            .split_first()
            .map_or(("", &[][..]), |(name, args)| (name.as_str(), args));
        std::process::exit(infra_skillcache::cli::execute(name, args));
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
        .thread_stack_size(8 * 1024 * 1024)
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

mod offline_omnigraph {
    use super::*;
    use axum::{
        Router,
        extract::{Path as AxumPath, Query as AxumQuery, State},
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use std::net::SocketAddr;
    use tokio::{net::TcpListener, sync::watch, task::JoinHandle};

    #[derive(Clone)]
    struct Snapshot {
        atoms: Vec<Value>,
        sessions: Vec<Value>,
        files: Vec<Value>,
        symbols: Vec<Value>,
        published_by: Vec<Value>,
        consumed_by: Vec<Value>,
        touches_file: Vec<Value>,
        mentions_symbol: Vec<Value>,
        relates_to: Vec<Value>,
    }

    #[derive(Clone)]
    struct BranchState {
        name: String,
        head_commit: String,
        from: Option<String>,
        from_commit_id: Option<String>,
        snapshot: Snapshot,
    }

    #[derive(Clone)]
    struct OmniState {
        branches: BTreeMap<String, BranchState>,
    }

    const TOKEN: &str = "tok-e2e";
    const TS: &str = "2026-11-19T18:00:00Z";
    const CREATED_US: i64 = 1_763_575_200_000_000;

    fn err(status: StatusCode, code: &str, msg: &str) -> Response {
        (status, axum::Json(json!({ "error": msg, "code": code }))).into_response()
    }

    fn auth_ok(headers: &HeaderMap) -> std::result::Result<(), Box<Response>> {
        let ver = headers
            .get("Omnigraph-Http-Api")
            .and_then(|v| v.to_str().ok());
        if ver != Some("0.14") {
            return Err(Box::new(err(
                StatusCode::BAD_REQUEST,
                "api_contract_mismatch",
                "Expected Omnigraph-Http-Api: 0.14.",
            )));
        }
        let bearer = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        if bearer != Some(TOKEN) {
            return Err(Box::new(err(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Unauthorized offline omnigraph token.",
            )));
        }
        Ok(())
    }

    async fn healthz() -> Response {
        axum::Json(json!({
            "status": "ok",
            "version": "0.15.0",
            "internal_schema_version": 16,
            "source_version": "e2e-fixture"
        }))
        .into_response()
    }

    async fn readyz() -> Response {
        axum::Json(json!({
            "ready": true,
            "status": "serving",
            "served_graph_count": 1,
            "ready_graph_count": 1
        }))
        .into_response()
    }

    async fn graphs(headers: HeaderMap) -> Response {
        if let Err(r) = auth_ok(&headers) {
            return *r;
        }
        axum::Json(json!({
            "graphs": [{
                "graph_id": HARNESS_GRAPH_ID,
                "state": "ready",
                "read_available": true,
                "write_available": true
            }]
        }))
        .into_response()
    }

    async fn schema_route(AxumPath(graph_id): AxumPath<String>, headers: HeaderMap) -> Response {
        if let Err(r) = auth_ok(&headers) {
            return *r;
        }
        if graph_id != HARNESS_GRAPH_ID {
            return err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.");
        }
        axum::Json(json!({
            "schema_source": HARNESS_CONTEXT_PG,
            "system_columns": { "id": "__id", "src": "__src", "dst": "__dst" }
        }))
        .into_response()
    }

    async fn branches(
        State(st): State<Arc<OmniState>>,
        AxumPath(graph_id): AxumPath<String>,
        AxumQuery(q): AxumQuery<BTreeMap<String, String>>,
        headers: HeaderMap,
    ) -> Response {
        if let Err(r) = auth_ok(&headers) {
            return *r;
        }
        if graph_id != HARNESS_GRAPH_ID {
            return err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.");
        }
        let names: Vec<String> = st.branches.keys().cloned().collect();
        if q.get("detailed").map(String::as_str) != Some("true") {
            return axum::Json(json!({ "branches": names })).into_response();
        }
        let details: Vec<Value> = st
            .branches
            .values()
            .map(|b| {
                json!({
                    "name": b.name,
                    "head": {
                        "graph_commit_id": b.head_commit,
                        "graph_branch": b.name,
                        "graph_manifest_version": 1,
                        "parent_commit_id": "c-0001",
                        "merged_parent_commit_id": Value::Null,
                        "actor_id": "harness",
                        "created_at": CREATED_US,
                        "published": true,
                        "kind": "published"
                    },
                    "from": b.from,
                    "from_commit_id": b.from_commit_id,
                    "created_at": CREATED_US,
                    "updated_at": CREATED_US
                })
            })
            .collect();
        axum::Json(json!({ "branches": names, "details": details })).into_response()
    }

    async fn commits(
        State(st): State<Arc<OmniState>>,
        AxumPath(graph_id): AxumPath<String>,
        AxumQuery(q): AxumQuery<BTreeMap<String, String>>,
        headers: HeaderMap,
    ) -> Response {
        if let Err(r) = auth_ok(&headers) {
            return *r;
        }
        if graph_id != HARNESS_GRAPH_ID {
            return err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.");
        }
        let bname = q.get("branch").map(String::as_str).unwrap_or("main");
        let Some(b) = st.branches.get(bname) else {
            return err(StatusCode::NOT_FOUND, "not_found", "Branch not found.");
        };
        let list = vec![
            json!({
                "graph_commit_id": b.head_commit,
                "graph_branch": b.name,
                "graph_manifest_version": 1,
                "parent_commit_id": "c-0001",
                "merged_parent_commit_id": Value::Null,
                "actor_id": "harness",
                "created_at": CREATED_US + 1000,
                "published": true,
                "kind": "published"
            }),
            json!({
                "graph_commit_id": "c-0001",
                "graph_branch": "main",
                "graph_manifest_version": 1,
                "parent_commit_id": Value::Null,
                "merged_parent_commit_id": Value::Null,
                "actor_id": "harness",
                "created_at": CREATED_US,
                "published": true,
                "kind": "published"
            }),
        ];
        axum::Json(json!({ "commits": list, "head_commit_id": b.head_commit })).into_response()
    }

    async fn commit_changes(
        AxumPath((graph_id, commit_id)): AxumPath<(String, String)>,
        headers: HeaderMap,
    ) -> Response {
        if let Err(r) = auth_ok(&headers) {
            return *r;
        }
        if graph_id != HARNESS_GRAPH_ID {
            return err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.");
        }
        axum::Json(json!({
            "cause": {
                "graph_commit_id": commit_id,
                "parent_commit_id": "c-0001",
                "merged_parent_commit_id": Value::Null,
                "authored_branch": "main",
                "actor_id": "harness",
                "authored_at": CREATED_US + 1000
            },
            "changes": []
        }))
        .into_response()
    }

    async fn changes_feed(AxumPath(graph_id): AxumPath<String>, headers: HeaderMap) -> Response {
        if let Err(r) = auth_ok(&headers) {
            return *r;
        }
        if graph_id != HARNESS_GRAPH_ID {
            return err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.");
        }
        axum::Json(json!({
            "blocks": [],
            "cursor": "cur-1",
            "caught_up": true
        }))
        .into_response()
    }

    async fn changes_baseline(
        State(st): State<Arc<OmniState>>,
        AxumPath(graph_id): AxumPath<String>,
        headers: HeaderMap,
        body: Option<axum::Json<Value>>,
    ) -> Response {
        if let Err(r) = auth_ok(&headers) {
            return *r;
        }
        if graph_id != HARNESS_GRAPH_ID {
            return err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.");
        }
        let bname = body
            .as_ref()
            .and_then(|v| v["branch"].as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("main");
        let head = st
            .branches
            .get(bname)
            .or_else(|| st.branches.get("main"))
            .map(|b| b.head_commit.as_str())
            .unwrap_or("c-0002");
        axum::Json(json!({
            "snapshot_commit_id": head,
            "resume_cursor": "cur-1",
            "records": []
        }))
        .into_response()
    }

    fn eval_query(
        snap: &Snapshot,
        qname: &str,
        qtext: &str,
        params: &Value,
    ) -> (Vec<Value>, Option<Value>) {
        let pid = params["project_id"].as_str().unwrap_or("");
        let matches_pid = |v: &Value| pid.is_empty() || v["project_id"].as_str() == Some(pid);

        if qname == "context_search" || qtext.contains("context_search") {
            let needle = params["q"].as_str().unwrap_or("").to_ascii_lowercase();
            let hits: Vec<Value> = snap
                .atoms
                .iter()
                .filter(|a| matches_pid(a))
                .filter(|a| {
                    let hay = format!(
                        "{} {} {} {}",
                        a["atom_id"].as_str().unwrap_or(""),
                        a["title"].as_str().unwrap_or(""),
                        a["summary"].as_str().unwrap_or(""),
                        a["body"].as_str().unwrap_or("")
                    )
                    .to_ascii_lowercase();
                    needle.is_empty() || hay.contains(&needle)
                })
                .map(|a| {
                    let mut row = a.clone();
                    row["score"] = json!(0.92);
                    row
                })
                .collect();
            return (
                hits,
                Some(
                    json!({ "by": "rrf", "order": "descending", "exhaustive": true, "score_columns": ["score"] }),
                ),
            );
        }
        if qname == "recent_atoms" || qtext.contains("recent_atoms") {
            return (
                snap.atoms
                    .iter()
                    .filter(|a| matches_pid(a))
                    .cloned()
                    .collect(),
                None,
            );
        }
        if qname == "list_sessions" || qtext.contains("list_sessions") {
            return (
                snap.sessions
                    .iter()
                    .filter(|s| matches_pid(s))
                    .cloned()
                    .collect(),
                None,
            );
        }
        if qname == "list_files" || qtext.contains("list_files") {
            return (
                snap.files
                    .iter()
                    .filter(|f| matches_pid(f))
                    .cloned()
                    .collect(),
                None,
            );
        }
        if qname == "list_symbols" || qtext.contains("list_symbols") {
            return (
                snap.symbols
                    .iter()
                    .filter(|s| matches_pid(s))
                    .cloned()
                    .collect(),
                None,
            );
        }
        if qname == "list_published_edges" || qtext.contains("list_published_edges") {
            return (snap.published_by.clone(), None);
        }
        if qname == "list_consumed_edges" || qtext.contains("list_consumed_edges") {
            return (snap.consumed_by.clone(), None);
        }
        if qname == "list_file_edges" || qtext.contains("list_file_edges") {
            return (snap.touches_file.clone(), None);
        }
        if qname == "list_symbol_edges" || qtext.contains("list_symbol_edges") {
            return (snap.mentions_symbol.clone(), None);
        }
        if qname == "list_atom_relations" || qtext.contains("list_atom_relations") {
            return (snap.relates_to.clone(), None);
        }
        if qname == "contradictions" || qtext.contains("contradictions") {
            return (vec![], None);
        }
        (vec![], None)
    }

    async fn query_inline(
        State(st): State<Arc<OmniState>>,
        AxumPath(graph_id): AxumPath<String>,
        headers: HeaderMap,
        axum::Json(body): axum::Json<Value>,
    ) -> Response {
        run_query(&st, &graph_id, None, &headers, &body)
    }

    async fn query_stored(
        State(st): State<Arc<OmniState>>,
        AxumPath((graph_id, name)): AxumPath<(String, String)>,
        headers: HeaderMap,
        axum::Json(body): axum::Json<Value>,
    ) -> Response {
        run_query(&st, &graph_id, Some(&name), &headers, &body)
    }

    fn run_query(
        st: &OmniState,
        graph_id: &str,
        stored_name: Option<&str>,
        headers: &HeaderMap,
        body: &Value,
    ) -> Response {
        if let Err(r) = auth_ok(headers) {
            return *r;
        }
        if graph_id != HARNESS_GRAPH_ID {
            return err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.");
        }
        let bname = body["branch"].as_str().unwrap_or("main");
        let Some(b) = st.branches.get(bname) else {
            return err(StatusCode::NOT_FOUND, "not_found", "Branch not found.");
        };
        let qname = stored_name
            .or_else(|| body["name"].as_str())
            .unwrap_or("inline_query");
        let qtext = body["query"].as_str().unwrap_or("");
        let (rows, ranking) = eval_query(&b.snapshot, qname, qtext, &body["params"]);
        axum::Json(json!({
            "query_name": qname,
            "target": { "branch": bname, "snapshot": body["snapshot"] },
            "row_count": rows.len(),
            "columns": [],
            "rows": rows,
            "graph_commit_id": b.head_commit,
            "ranking": ranking
        }))
        .into_response()
    }

    pub async fn spawn(
        project_id: &str,
        session_id: &str,
        other_session_id: &str,
    ) -> Result<(String, watch::Sender<bool>, JoinHandle<()>)> {
        let sess_branch = session_branch_name(session_id);
        let atom = |id: &str, kind: &str, title: &str, summary: &str, sid: &str, branch: &str| {
            json!({
                "atom_id": id,
                "project_id": project_id,
                "kind": kind,
                "title": title,
                "summary": summary,
                "body": format!("{title}\n\n{summary}"),
                "tags": ["e2e", kind],
                "source_branch": branch,
                "source_session_id": sid,
                "confidence": 0.92,
                "pin_priority": if kind == "decision" { 1 } else { 0 },
                "verified": true,
                "origin_layer": "brain",
                "authority": "governed",
                "created_at": TS,
                "updated_at": TS,
            })
        };
        let mut atoms = vec![
            atom(
                "atom-finding-backoff",
                "finding",
                "Retry backoff finding",
                "Exponential backoff prevents upstream saturation.",
                other_session_id,
                "main",
            ),
            atom(
                "atom-decision-schema",
                "decision",
                "Pin Graph IR schema v1",
                "Frozen Graph IR node ID encoding.",
                other_session_id,
                "main",
            ),
            atom(
                "atom-warning-cycle",
                "warning",
                "Infra-to-domain layer cycle",
                "pkg_09/mod_19.js imports pkg_00/mod_19.js.",
                session_id,
                "main",
            ),
            atom(
                "atom-contract-orders",
                "contract",
                "Orders HTTP v1 contract",
                "Consumed by gateway and verified against billing.",
                session_id,
                "main",
            ),
            atom(
                "atom-insight-lcov",
                "insight",
                "LCOV coverage reaches mod_00",
                "Tests lens tracks core.test.js hits.",
                other_session_id,
                "main",
            ),
            atom(
                "atom-php-ns\\Service::boot",
                "artifact",
                "Escaped namespace artifact",
                "Verifies CSS attribute escaping on special node IDs.",
                session_id,
                "main",
            ),
            atom(
                "atom-question-unmatched",
                "question",
                "Ghost RPC provider missing?",
                "Unmatched contract risk ghost-rpc-v1 in gateway.",
                session_id,
                "main",
            ),
        ];
        let sessions = vec![
            json!({
                "session_id": session_id,
                "project_id": project_id,
                "title": "Main active session",
                "branch": sess_branch,
                "provider": "claude",
                "workflow": "native",
                "status": "running",
                "goal": "Inspect entry_00.js and propose edit.",
                "created_at": TS,
                "updated_at": TS,
            }),
            json!({
                "session_id": other_session_id,
                "project_id": project_id,
                "title": "Baseline verification session",
                "branch": "main",
                "provider": "claude",
                "workflow": "native",
                "status": "completed",
                "goal": "Baseline E2E inspection of entry_01.js.",
                "created_at": TS,
                "updated_at": TS,
            }),
        ];
        let files = vec![
            json!({ "path": "entry_00.js", "project_id": project_id, "language": "javascript", "last_touched_at": TS }),
            json!({ "path": "pkg_00/mod_00.js", "project_id": project_id, "language": "javascript", "last_touched_at": TS }),
        ];
        let symbols = vec![
            json!({ "symbol_id": "sym-runEntry", "project_id": project_id, "name": "runEntry_00", "kind": "function", "file_path": "entry_00.js" }),
        ];
        let published_by = vec![
            json!({ "source_id": "atom-finding-backoff", "target_id": other_session_id, "role": "author", "published_at": TS }),
            json!({ "source_id": "atom-warning-cycle", "target_id": session_id, "role": "author", "published_at": TS }),
            json!({ "source_id": "atom-contract-orders", "target_id": session_id, "role": "author", "published_at": TS }),
        ];
        let consumed_by = vec![
            json!({ "source_id": "atom-decision-schema", "target_id": session_id, "role": "reader", "consumed_at": TS }),
        ];
        let touches_file = vec![
            json!({ "source_id": "atom-finding-backoff", "target_id": "entry_00.js", "relation": "mentions" }),
            json!({ "source_id": "atom-insight-lcov", "target_id": "pkg_00/mod_00.js", "relation": "covers" }),
        ];
        let mentions_symbol = vec![
            json!({ "source_id": "atom-finding-backoff", "target_id": "sym-runEntry", "name": "runEntry_00", "kind": "function", "file_path": "entry_00.js", "role": "references" }),
        ];
        let relates_to = vec![
            json!({ "source_id": "atom-decision-schema", "target_id": "atom-finding-backoff", "relation": "supports", "confidence": 0.9, "note": "grounded", "created_at": TS }),
            json!({ "source_id": "atom-contract-orders", "target_id": "atom-decision-schema", "relation": "depends_on", "confidence": 0.85, "note": "schema", "created_at": TS }),
            json!({ "source_id": "atom-warning-cycle", "target_id": "atom-php-ns\\Service::boot", "relation": "refines", "confidence": 0.8, "note": "escaped", "created_at": TS }),
        ];
        let main_snap = Snapshot {
            atoms: atoms.clone(),
            sessions: sessions.clone(),
            files: files.clone(),
            symbols: symbols.clone(),
            published_by: published_by.clone(),
            consumed_by: consumed_by.clone(),
            touches_file: touches_file.clone(),
            mentions_symbol: mentions_symbol.clone(),
            relates_to: relates_to.clone(),
        };
        atoms.push(atom(
            "atom-session-draft",
            "insight",
            "Session branch draft insight",
            "Scoped only to active session branch.",
            session_id,
            &sess_branch,
        ));
        let sess_snap = Snapshot {
            atoms,
            sessions,
            files,
            symbols,
            published_by,
            consumed_by,
            touches_file,
            mentions_symbol,
            relates_to,
        };
        let state = Arc::new(OmniState {
            branches: BTreeMap::from([
                (
                    "main".into(),
                    BranchState {
                        name: "main".into(),
                        head_commit: "c-0002".into(),
                        from: None,
                        from_commit_id: None,
                        snapshot: main_snap,
                    },
                ),
                (
                    sess_branch.clone(),
                    BranchState {
                        name: sess_branch,
                        head_commit: "c-0003".into(),
                        from: Some("main".into()),
                        from_commit_id: Some("c-0001".into()),
                        snapshot: sess_snap,
                    },
                ),
            ]),
        });

        let app = Router::new()
            .route("/healthz", get(healthz))
            .route("/readyz", get(readyz))
            .route("/graphs", get(graphs))
            .route("/graphs/{graph_id}/schema", get(schema_route))
            .route("/graphs/{graph_id}/branches", get(branches))
            .route("/graphs/{graph_id}/commits", get(commits))
            .route(
                "/graphs/{graph_id}/commits/{commit_id}/changes",
                get(commit_changes),
            )
            .route("/graphs/{graph_id}/changes", get(changes_feed))
            .route(
                "/graphs/{graph_id}/changes/baseline",
                post(changes_baseline),
            )
            .route("/graphs/{graph_id}/query", post(query_inline))
            .route("/graphs/{graph_id}/queries/{name}", post(query_stored))
            .with_state(state);

        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr: SocketAddr = listener.local_addr()?;
        let (tx, mut rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    while !*rx.borrow() {
                        if rx.changed().await.is_err() {
                            break;
                        }
                    }
                })
                .await;
        });
        Ok((format!("http://{addr}"), tx, task))
    }
}
