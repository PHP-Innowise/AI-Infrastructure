//! Explicit fixture transport; the real capture runs inside its guard-capable binary.
use infra_eval::{
    artifact,
    process::{Budget, CommandSpec},
};
use serde_json::{Value, json};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const REQUEST_LIMIT: usize = 8 * 1024 * 1024;
const RECEIPT_LIMIT: usize = 1024 * 1024;

/// Preserve the production capture request without running it in libtest.
pub fn execute(
    driver: &Path,
    spec: &CommandSpec,
    budget: &Budget,
    request: &Path,
    stdout: &Path,
    stderr: &Path,
) -> Value {
    let milliseconds = u64::try_from(budget.timeout.as_millis()).unwrap();
    assert!((1..=300_000).contains(&milliseconds));
    let packet = json!({
        "argv":spec.argv, "cwd":spec.cwd, "env":spec.env, "input":spec.input,
        "budget":{
            "milliseconds":milliseconds, "cap":budget.cap,
            "max_bytes":budget.max_bytes, "native":budget.native,
            "strict_events":budget.strict_events,
            "error_diagnostics_terminal":budget.error_diagnostics_terminal,
        },
        "stdout":stdout, "stderr":stderr,
    });
    let bytes = serde_json::to_vec(&packet).unwrap();
    assert!(bytes.len() <= REQUEST_LIMIT);
    artifact::fresh_file(request, &bytes, 0o600).unwrap();
    let mut child = Command::new(driver)
        .arg("execute")
        .arg(request)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(&spec.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the explicitly compiled process fixture");
    let output = child.stdout.take().unwrap();
    let errors = child.stderr.take().unwrap();
    let output_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        output
            .take(RECEIPT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    });
    let error_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        errors
            .take(RECEIPT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    });
    // The per-command production budget (30 s, or a long command's 120 s and
    // 300 s on Windows) is in the packet. The fixture transport itself has a
    // separate hard wall bound: 120 s, or the budget where that is longer.
    let bound = budget.timeout.max(Duration::from_secs(120));
    let deadline = Instant::now() + bound;
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break (status, false);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            break (child.wait().unwrap(), true);
        }
        thread::sleep(Duration::from_millis(5));
    };
    let output = output_thread.join().unwrap();
    let errors = error_thread.join().unwrap();
    assert!(
        !timed_out,
        "explicit Eval process fixture exceeded {}s",
        bound.as_secs()
    );
    assert!(output.len() <= RECEIPT_LIMIT && errors.len() <= RECEIPT_LIMIT);
    assert!(errors.is_empty(), "{}", String::from_utf8_lossy(&errors));
    let receipt: Value = serde_json::from_slice(&output).expect("one bounded fixture receipt");
    assert_eq!(status.code(), Some(0), "{receipt}");
    assert_eq!(receipt["ok"], true, "{receipt}");
    assert!(receipt["session"].is_object(), "{receipt}");
    receipt["session"].clone()
}
