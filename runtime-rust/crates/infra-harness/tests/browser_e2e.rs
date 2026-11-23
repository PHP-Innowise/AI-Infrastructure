//! Integration and headless Chrome E2E harness for `infra-harness-browser-e2e-fixture`.
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    time::{Duration, Instant},
};

struct ChildGuard {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
}

impl ChildGuard {
    fn new(mut child: Child) -> Self {
        let stdin = child.stdin.take();
        Self {
            child: Some(child),
            stdin,
        }
    }

    fn shutdown_gracefully(&mut self, timeout: Duration) -> std::process::ExitStatus {
        drop(self.stdin.take());
        let Some(mut child) = self.child.take() else {
            panic!("fixture child already reaped");
        };
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait().expect("poll fixture child") {
                Some(status) => return status,
                None if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                None => {
                    let _ = child.kill();
                    let status = child.wait().expect("wait killed fixture child");
                    panic!(
                        "browser E2E fixture did not exit cleanly within {timeout:?} (killed: {status})"
                    );
                }
            }
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => return,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return;
                    }
                }
            }
        }
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn has_real_wasm_ui(ui_root: &Path) -> bool {
    let wasm = ui_root.join("runtime-rust/target/wasm-ui");
    wasm.join("infra_ui.js").is_file()
        && wasm.join("infra_ui_bg.wasm").is_file()
        && wasm.join("abi-manifest.json").is_file()
}

fn copy_dir_files(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create target directory");
    for entry in fs::read_dir(src).expect("read source directory") {
        let entry = entry.expect("dir entry");
        let ty = entry.file_type().expect("file type");
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_files(&entry.path(), &target);
        } else if ty.is_file() {
            fs::copy(entry.path(), target).expect("copy file");
        }
    }
}

fn prepare_stub_ui_root(repo: &Path, dst: &Path) {
    copy_dir_files(&repo.join("harness/web"), &dst.join("harness/web"));
    copy_dir_files(
        &repo.join("install/open-source-kit"),
        &dst.join("install/open-source-kit"),
    );

    let wasm_dir = dst.join("runtime-rust/target/wasm-ui");
    fs::create_dir_all(&wasm_dir).expect("create stub wasm-ui dir");
    let js = b"export default async function init() {}\n";
    let wasm = b"\x00asm\x01\x00\x00\x00";
    fs::write(wasm_dir.join("infra_ui.js"), js).expect("write stub infra_ui.js");
    fs::write(wasm_dir.join("infra_ui_bg.wasm"), wasm).expect("write stub infra_ui_bg.wasm");
    let manifest = json!({
        "schema_version": 1,
        "js_sha256": infra_harness::store::sha(js),
        "wasm_sha256": infra_harness::store::sha(wasm),
    });
    fs::write(wasm_dir.join("abi-manifest.json"), manifest.to_string())
        .expect("write stub abi-manifest.json");
}

fn spawn_fixture(
    state_root: &Path,
    ui_root: &Path,
    source_root: &Path,
    timeout_secs: u64,
) -> (ChildGuard, Value, std::thread::JoinHandle<String>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_infra-harness-browser-e2e-fixture"));
    cmd.env_remove("RUST_MIN_STACK")
        .arg(state_root)
        .arg("--ui-root")
        .arg(ui_root)
        .arg("--source")
        .arg(source_root)
        .arg("--timeout-secs")
        .arg(timeout_secs.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn browser e2e fixture");
    let stdout = child.stdout.take().expect("fixture stdout");
    let stderr = child.stderr.take().expect("fixture stderr");
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut buf);
        buf
    });
    let guard = ChildGuard::new(child);

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let res = reader.read_line(&mut line).map(|_| line);
        let _ = tx.send(res);
    });

    let line = match rx.recv_timeout(Duration::from_secs(120)) {
        Ok(Ok(line)) if !line.trim().is_empty() => line,
        Ok(Ok(_)) => {
            drop(guard);
            let err = stderr_handle.join().unwrap_or_default();
            panic!("Fixture exited before printing receipt. stderr:\n{err}");
        }
        Ok(Err(e)) => {
            drop(guard);
            let err = stderr_handle.join().unwrap_or_default();
            panic!("Failed reading fixture stdout: {e}. stderr:\n{err}");
        }
        Err(_) => {
            drop(guard);
            let err = stderr_handle.join().unwrap_or_default();
            panic!("Timed out waiting for fixture receipt. stderr:\n{err}");
        }
    };
    let receipt: Value = serde_json::from_str(line.trim())
        .unwrap_or_else(|e| panic!("Invalid fixture receipt JSON `{line}`: {e}"));
    (guard, receipt, stderr_handle)
}

fn find_executable_in_path(candidates: &[&str]) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for name in candidates {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn find_chrome_binary() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("HARNESS_CHROME_BIN").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return Some(p);
        }
        return None;
    }
    let direct = Path::new("/opt/google/chrome/chrome");
    if direct.is_file() {
        return Some(direct.to_path_buf());
    }
    find_executable_in_path(&["google-chrome", "chromium", "chromium-browser"])
}

fn http_host_port(url: &str) -> (String, u16) {
    let stripped = url
        .strip_prefix("http://")
        .expect("fixture url starts with http://");
    let (host, port_str) = stripped.split_once(':').expect("fixture url has port");
    let port = port_str.parse::<u16>().expect("valid port");
    (host.to_owned(), port)
}

fn get_json(client: &reqwest::blocking::Client, url: &str) -> Value {
    let text = client
        .get(url)
        .send()
        .unwrap_or_else(|e| panic!("GET {url} failed: {e}"))
        .error_for_status()
        .unwrap_or_else(|e| panic!("GET {url} returned error status: {e}"))
        .text()
        .unwrap_or_else(|e| panic!("read body for GET {url} failed: {e}"));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("parse JSON from GET {url} failed: {e}\nbody: {text}"))
}

fn post_json(
    client: &reqwest::blocking::Client,
    url: &str,
    csrf: &str,
    body: &Value,
) -> reqwest::blocking::Response {
    client
        .post(url)
        .header("X-Harness-Token", csrf)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .unwrap_or_else(|e| panic!("POST {url} failed: {e}"))
}

#[test]
fn browser_e2e_fixture_serves_graph_sse_and_range_contracts() {
    let repo = repo_root();
    let temp = tempfile::tempdir().expect("tempdir");
    let state_root = temp.path().join("fixture-state");
    fs::create_dir_all(&state_root).expect("create state root");

    let env_ui_root = std::env::var_os("HARNESS_UI_ROOT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let ui_root = match env_ui_root {
        Some(path) if has_real_wasm_ui(&path) => path,
        _ if has_real_wasm_ui(&repo) => repo.clone(),
        _ => {
            let stub = temp.path().join("stub-ui-root");
            prepare_stub_ui_root(&repo, &stub);
            stub
        }
    };

    let (mut guard, receipt, stderr_handle) = spawn_fixture(&state_root, &ui_root, &repo, 60);
    let url = receipt["url"].as_str().expect("receipt url");
    let project_id = receipt["project_id"].as_str().expect("receipt project_id");
    let session_id = receipt["session_id"].as_str().expect("receipt session_id");
    let other_session_id = receipt["other_session_id"]
        .as_str()
        .expect("receipt other_session_id");
    let commit_a = receipt["commit_a"].as_str().expect("receipt commit_a");
    let commit_b = receipt["commit_b"].as_str().expect("receipt commit_b");
    assert!(receipt["root"].as_str().is_some_and(|s| !s.is_empty()));
    assert_ne!(session_id, other_session_id);
    assert_ne!(commit_a, commit_b);

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("build http client");

    let html = client
        .get(format!("{url}/"))
        .send()
        .expect("GET /")
        .error_for_status()
        .expect("GET / status")
        .text()
        .expect("GET / body");
    assert!(html.contains("id=\"approvals-drawer\""));
    assert!(html.contains("id=\"graph-map\""));
    assert!(html.contains("id=\"command-palette\""));

    let boot = get_json(&client, &format!("{url}/api/bootstrap"));
    let csrf = boot["csrf"].as_str().expect("bootstrap csrf").to_owned();
    assert!(!csrf.is_empty());
    let sessions = boot["sessions"].as_array().expect("bootstrap sessions");
    let main_summary = sessions
        .iter()
        .find(|s| s["id"] == session_id)
        .expect("primary session in bootstrap");
    assert_eq!(main_summary["status"], "running");
    assert_eq!(main_summary["waiting_permissions"], 1);
    let other_summary = sessions
        .iter()
        .find(|s| s["id"] == other_session_id)
        .expect("other session in bootstrap");
    assert_eq!(other_summary["status"], "completed");

    let graph_status = get_json(&client, &format!("{url}/api/projects/{project_id}/graph"));
    assert_eq!(graph_status["state"], "ready");

    let dense_map = get_json(
        &client,
        &format!("{url}/api/projects/{project_id}/graph/map?scope=&whole=1"),
    );
    let nodes = dense_map["nodes"].as_array().expect("dense map nodes");
    let edges = dense_map["edges"].as_array().expect("dense map edges");
    assert!(
        nodes.len() >= 180,
        "expected >= 180 nodes for dense threshold, got {}",
        nodes.len()
    );
    assert!(!edges.is_empty(), "expected non-empty graph edges");

    let range_changes = get_json(
        &client,
        &format!(
            "{url}/api/projects/{project_id}/graph/changes?target=range:{commit_a}..{commit_b}"
        ),
    );
    let from_to_changes = get_json(
        &client,
        &format!("{url}/api/projects/{project_id}/graph/changes?from={commit_a}&to={commit_b}"),
    );
    assert_eq!(range_changes["files"], from_to_changes["files"]);
    let changed_files = range_changes["files"]
        .as_array()
        .expect("range changes files");
    assert!(
        changed_files.iter().any(|f| f["path"] == "entry_00.js")
            && changed_files
                .iter()
                .any(|f| f["path"] == "pkg_00/mod_20.js"),
        "unexpected range diff files: {range_changes}"
    );

    let range_map = get_json(
        &client,
        &format!(
            "{url}/api/projects/{project_id}/graph/map?scope=&target=range:{commit_a}..{commit_b}"
        ),
    );
    assert!(
        !range_map["nodes"]
            .as_array()
            .expect("range map nodes")
            .is_empty()
    );

    let session_detail = get_json(
        &client,
        &format!("{url}/api/sessions/{session_id}?limit=50&tail=1"),
    );
    let waiting = session_detail["waiting_permissions"]
        .as_array()
        .expect("waiting_permissions array");
    assert_eq!(waiting.len(), 1);
    assert!(
        waiting[0]["diff_preview"]
            .as_str()
            .is_some_and(|s| s.contains("entry_00.js")),
        "expected diff_preview for pending permission: {waiting:?}"
    );
    let events = session_detail["events"].as_array().expect("session events");
    assert!(events.iter().any(|ev| ev["kind"] == "tool"));
    assert!(
        events.iter().any(|ev| {
            ev["kind"] == "text" && ev["text"].as_str().is_some_and(|t| t.contains("```js"))
        }),
        "expected fenced code block event in initial session events"
    );

    // Open a live SSE stream and trigger a live provider burst via inbox_accept.
    let (host, port) = http_host_port(url);
    let addr: SocketAddr = format!("{host}:{port}").parse().expect("socket addr");
    let mut sse_stream =
        TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("connect SSE stream");
    sse_stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set SSE read timeout");
    write!(
        sse_stream,
        "GET /api/events/stream?session_id={session_id} HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n"
    )
    .expect("write SSE request");
    sse_stream.flush().expect("flush SSE request");

    let mut sse_reader = BufReader::new(sse_stream);
    let mut status_line = String::new();
    sse_reader
        .read_line(&mut status_line)
        .expect("read SSE status");
    assert!(
        status_line.contains("200"),
        "expected 200 SSE response, got {status_line}"
    );
    let mut saw_event_stream_ct = false;
    loop {
        let mut hdr = String::new();
        sse_reader.read_line(&mut hdr).expect("read SSE header");
        if hdr == "\r\n" || hdr == "\n" || hdr.is_empty() {
            break;
        }
        if hdr.to_ascii_lowercase().starts_with("content-type:")
            && hdr.to_ascii_lowercase().contains("text/event-stream")
        {
            saw_event_stream_ct = true;
        }
    }
    assert!(
        saw_event_stream_ct,
        "missing text/event-stream Content-Type"
    );

    let inbox_resp = post_json(
        &client,
        &format!("{url}/api/sessions/{session_id}/inbox"),
        &csrf,
        &json!({
            "request_id": "e2e-live-sse-1",
            "prompt": "Trigger live SSE stream verification."
        }),
    );
    assert_eq!(inbox_resp.status().as_u16(), 202);

    let mut saw_live_tool = false;
    let mut saw_live_md = false;
    let mut current_event = String::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && !(saw_live_tool && saw_live_md) {
        let mut line = String::new();
        let n = sse_reader.read_line(&mut line).expect("read SSE line");
        assert!(n > 0, "SSE stream closed before live events arrived");
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
        if let Some(ev_name) = trimmed.strip_prefix("event:") {
            current_event = ev_name.trim().to_owned();
        } else if let Some(raw_data) = trimmed.strip_prefix("data:")
            && current_event == "session_event"
            && let Ok(frame) = serde_json::from_str::<Value>(raw_data.trim())
        {
            let ev = &frame["event"];
            if ev["kind"] == "tool"
                && ev["text"]
                    .as_str()
                    .is_some_and(|t| t.contains("Read") || t.contains("Tool"))
            {
                saw_live_tool = true;
            }
            if ev["kind"] == "text"
                && ev["text"]
                    .as_str()
                    .is_some_and(|t| t.contains("e2e_sse_probe_1"))
            {
                saw_live_md = true;
            }
        }
    }
    assert!(saw_live_tool, "did not observe live SSE tool session_event");
    assert!(
        saw_live_md,
        "did not observe live SSE markdown session_event"
    );

    drop(sse_reader);

    let omni_status = get_json(&client, &format!("{url}/api/omnigraph/status"));
    assert_eq!(omni_status["connected"], true, "{omni_status}");
    let omni_graph = get_json(
        &client,
        &format!("{url}/api/omnigraph/graph?project_id={project_id}"),
    );
    let omni_atoms = omni_graph["atoms"]
        .as_array()
        .expect("omnigraph atoms array");
    assert!(omni_atoms.len() >= 7, "expected seeded atoms: {omni_graph}");
    let omni_search = get_json(
        &client,
        &format!("{url}/api/omnigraph/search?project_id={project_id}&q=backoff"),
    );
    assert!(
        !omni_search["hits"]
            .as_array()
            .expect("omnigraph search hits")
            .is_empty(),
        "expected backoff hit: {omni_search}"
    );
    let omni_changes = get_json(
        &client,
        &format!("{url}/api/omnigraph/changes?project_id={project_id}&branch=main"),
    );
    assert!(
        omni_changes["events"].is_array() && omni_changes["cursor"] == "cur-1",
        "expected omnigraph changes feed events array: {omni_changes}"
    );

    let system_run_id = receipt["system_run_id"]
        .as_str()
        .expect("receipt system_run_id");
    let sys_map = get_json(
        &client,
        &format!(
            "{url}/api/systems/graph/map?project_id={project_id}&config_path=system.json&run_id={system_run_id}"
        ),
    );
    assert!(
        sys_map["nodes"]
            .as_array()
            .is_some_and(|nodes| nodes.len() >= 3)
            || sys_map["services"]
                .as_array()
                .is_some_and(|svcs| svcs.len() >= 3),
        "expected seeded system topology: {sys_map}"
    );

    let arch_map = get_json(
        &client,
        &format!(
            "{url}/api/projects/{project_id}/graph/map?scope=&patterns=1&focus_session={session_id}"
        ),
    );
    assert!(
        arch_map["architecture"].is_object(),
        "expected architecture analysis with patterns=true: {arch_map}"
    );
    assert!(
        range_map["test_reach"].is_array() || range_map["change"].is_object(),
        "expected range change/test_reach summary: {range_map}"
    );

    let status = guard.shutdown_gracefully(Duration::from_secs(10));
    let stderr = stderr_handle.join().unwrap_or_default();
    assert!(
        status.success(),
        "fixture did not exit cleanly ({status}):\n{stderr}"
    );
}

#[test]
fn headless_browser_e2e_canvas2d_lenses_sse_modals() {
    let strict = std::env::var("HARNESS_BROWSER_E2E").as_deref() == Ok("1");
    let repo = repo_root();
    let ui_root = std::env::var_os("HARNESS_UI_ROOT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.clone());
    let script = repo.join("harness/tests/browser_e2e.mjs");

    let mut missing = Vec::new();
    let chrome = find_chrome_binary();
    if chrome.is_none() {
        missing.push(
            "Chrome binary not found (checked HARNESS_CHROME_BIN, /opt/google/chrome/chrome, google-chrome, chromium, chromium-browser)".to_owned(),
        );
    }
    let node = find_executable_in_path(&["node"]);
    if node.is_none() {
        missing.push("node binary not found on PATH".to_owned());
    }
    if !script.is_file() {
        missing.push(format!("E2E script missing at {}", script.display()));
    }
    if !has_real_wasm_ui(&ui_root) {
        missing.push(format!(
            "Built wasm-ui artifacts missing under {} (run `infra-engine repo build-ui`)",
            ui_root.join("runtime-rust/target/wasm-ui").display()
        ));
    }

    if !missing.is_empty() {
        let reason = missing.join("; ");
        if strict {
            panic!("HARNESS_BROWSER_E2E=1 failed prerequisite check: {reason}");
        }
        eprintln!("Skipping headless_browser_e2e_canvas2d_lenses_sse_modals: {reason}");
        return;
    }

    let chrome = chrome.expect("chrome checked");
    let node = node.expect("node checked");

    let temp = tempfile::tempdir().expect("tempdir");
    let state_root = temp.path().join("e2e-state");
    fs::create_dir_all(&state_root).expect("create state root");

    let (mut guard, receipt, stderr_handle) = spawn_fixture(&state_root, &ui_root, &repo, 240);
    let url = receipt["url"].as_str().expect("receipt url");
    let project_id = receipt["project_id"].as_str().expect("receipt project_id");
    let session_id = receipt["session_id"].as_str().expect("receipt session_id");
    let other_session_id = receipt["other_session_id"]
        .as_str()
        .expect("receipt other_session_id");
    let commit_a = receipt["commit_a"].as_str().expect("receipt commit_a");
    let commit_b = receipt["commit_b"].as_str().expect("receipt commit_b");
    let system_run_id = receipt["system_run_id"].as_str().unwrap_or("");

    let mut command = Command::new(&node);
    command
        .arg(&script)
        .arg("--url")
        .arg(url)
        .arg("--project-id")
        .arg(project_id)
        .arg("--session-id")
        .arg(session_id)
        .arg("--other-session-id")
        .arg(other_session_id)
        .arg("--commit-a")
        .arg(commit_a)
        .arg("--commit-b")
        .arg(commit_b);
    if !system_run_id.is_empty() {
        command.arg("--system-run-id").arg(system_run_id);
    }
    if let Some(capsule) = receipt["skillcache_capsule_id"].as_str() {
        command.arg("--skillcache-capsule-id").arg(capsule);
    }
    if receipt["skillcache_requalify"] == true {
        command.arg("--skillcache-requalify").arg("1");
    }
    let output = command
        .arg("--chrome")
        .arg(&chrome)
        .arg("--timeout-ms")
        .arg("150000")
        .output()
        .expect("execute browser_e2e.mjs");

    let shutdown_status = guard.shutdown_gracefully(Duration::from_secs(10));
    let fixture_stderr = stderr_handle.join().unwrap_or_default();
    let node_stdout = String::from_utf8_lossy(&output.stdout);
    let node_stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "browser_e2e.mjs failed ({})\n--- node stdout ---\n{node_stdout}\n--- node stderr ---\n{node_stderr}\n--- fixture stderr ---\n{fixture_stderr}",
        output.status
    );
    assert!(
        shutdown_status.success(),
        "fixture server did not exit cleanly after browser_e2e.mjs ({shutdown_status}):\n{fixture_stderr}"
    );
}
