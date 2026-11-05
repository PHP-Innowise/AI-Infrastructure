//! `infra-engine system plan` and `system execute` run a reviewed plan end to end
//! with the native Brain. Every dispatch re-ensures its native task, so a run
//! of more than one dispatch must reuse the bindings it created; the whole run
//! is one future on the CLI's worker thread. A deterministic command adapter
//! stands in for the model.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "infra-cli-system-execution-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(fs::canonicalize(root).unwrap())
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn engine() -> PathBuf {
    std::env::var_os("INFRA_NATIVE_TEST_ENGINE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_infra-engine")))
}

fn write_json(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

/// Two services: `orders` publishes `order.placed`, `billing` consumes it.
fn system(root: &Path) -> PathBuf {
    let config = root.join("system/system.json");
    write_json(
        &config,
        &json!({"schema_version":1,"name":"Fixture system","services":[
            {"id":"orders","root":"../services/orders","manifest":"ai-service.json"},
            {"id":"billing","root":"../services/billing","manifest":"ai-service.json"}],
            "shared_sources":[{"path":"AGENTS.md","kind":"policy"}]}),
    );
    fs::write(
        root.join("system/AGENTS.md"),
        "Coordinate contract changes first.\n",
    )
    .unwrap();
    for (sid, consumes, provides) in [
        (
            "orders",
            json!([]),
            json!([{"id":"order.placed","kind":"event","version":"1","sources":["contracts/events.yaml"]}]),
        ),
        (
            "billing",
            json!([{"service":"orders","contract":"order.placed","version":"1"}]),
            json!([]),
        ),
    ] {
        let service = root.join("services").join(sid);
        write_json(
            &service.join("ai-service.json"),
            &json!({"schema_version":1,"id":sid,"description":format!("{sid} fixture service"),
                "owner":"Fixture team","relationships_complete":true,
                "capabilities":[{"id":"work","description":"Fixture work","status":"implemented","sources":["src/work.txt"]}],
                "provides":provides,"consumes":consumes,
                "sources":[{"path":"AGENTS.md","kind":"policy"}]}),
        );
        fs::create_dir_all(service.join("src")).unwrap();
        fs::create_dir_all(service.join("contracts")).unwrap();
        fs::write(service.join("AGENTS.md"), format!("{sid} policy\n")).unwrap();
        fs::write(service.join("src/work.txt"), "before\n").unwrap();
        fs::write(service.join("contracts/events.yaml"), "order.placed: v1\n").unwrap();
    }
    config
}

/// Answers each dispatch from the top-level phase and service of its prompt
/// (canonical JSON sorts keys, so `phase` is followed by `previous_receipts`).
/// In edit mode the billing worker changes a registered source and reports it.
fn adapter(root: &Path) -> PathBuf {
    let script = root.join("adapter.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
input=$(cat)
report() { printf '{"status":"completed","summary":"%s","checks":[{"name":"%s","status":"passed","detail":"fixture"}],"changed_files":%s,"service_order":%s}\n' "$1" "$2" "$3" "$4"; }
case "$input" in
  *'"readable":["__system__","billing","orders"]'*'"phase":"contracts","previous_receipts"'*) ;;
  *'"phase":"contracts","previous_receipts"'*) printf '{"status":"blocked","summary":"System folder not readable","checks":[],"changed_files":[],"service_order":[]}\n'; exit 0 ;;
esac
case "$input" in
  *'"phase":"contracts","previous_receipts"'*) report "Contracts agreed" "contracts" '[]' '["orders","billing"]' ;;
  *'"phase":"verify","previous_receipts"'*) report "Verified end to end" "end-to-end" '[]' '[]' ;;
  *'"mode":"edit"'*'"service":"billing","task"'*) printf 'after\n' > src/work.txt; report "Billing updated" "unit" '["src/work.txt"]' '[]' ;;
  *) report "Service checked" "unit" '[]' '[]' ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn cli(root: &Path, args: &[&str]) -> (i32, Value, String) {
    let output = Command::new(engine())
        .arg("system")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .expect("run infra-engine system");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (
        output.status.code().unwrap_or(-1),
        serde_json::from_str(&stdout).unwrap_or(Value::Null),
        format!("{stdout}{}", String::from_utf8_lossy(&output.stderr)),
    )
}

fn run(root: &Path, mode: &str) -> (Value, PathBuf) {
    let config = system(root);
    let config = config.to_str().unwrap();
    let services = root.join("services");
    let services = services.to_str().unwrap();
    let plan = root.join("plan.json");
    let (code, value, text) = cli(
        root,
        &[
            "plan",
            "--system",
            config,
            "--allow-root",
            services,
            "--task",
            "Bill every placed order",
            "--change-id",
            "bill-orders",
            "--service",
            "orders",
            "--output",
            plan.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(value["status"], "needs_review", "{text}");
    assert_eq!(value["context"]["services"], json!(["billing", "orders"]));
    let script = adapter(root);
    let directory = root.join("run");
    let (code, summary, text) = cli(
        root,
        &[
            "execute",
            "--system",
            config,
            "--allow-root",
            services,
            "--plan",
            plan.to_str().unwrap(),
            "--run-dir",
            directory.to_str().unwrap(),
            "--provider",
            "command",
            "--executable",
            script.to_str().unwrap(),
            "--mode",
            mode,
            "--timeout",
            "60",
        ],
    );
    assert_eq!(code, 0, "execute did not complete: {text}");
    (summary, directory)
}

fn assert_completed(summary: &Value, directory: &Path) {
    assert_eq!(summary["status"], "completed", "{summary:#}");
    let steps = summary["steps"].as_array().unwrap();
    assert_eq!(
        steps
            .iter()
            .map(|step| step["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["contracts", "service-orders", "service-billing", "verify"]
    );
    assert!(steps.iter().all(|step| step["status"] == "completed"));
    for (sid, task) in summary["tasks"].as_object().unwrap() {
        assert_eq!(task["closed"], true, "{sid} task left open");
        assert!(task["uuid"].is_string(), "{sid} task has no native record");
    }
    assert!(directory.join("handoff.json").is_file());
}

#[test]
fn a_read_only_run_completes_and_closes_every_native_task() {
    let root = Root::new();
    let (summary, directory) = run(&root.0, "read-only");
    assert_completed(&summary, &directory);
    let config = root.0.join("system/system.json");
    let services = root.0.join("services");
    let (code, status, text) = cli(
        &root.0,
        &[
            "run-status",
            "--system",
            config.to_str().unwrap(),
            "--allow-root",
            services.to_str().unwrap(),
            "--run-dir",
            directory.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(status["status"], "completed");
}

#[test]
fn a_run_keeps_the_native_storage_local_to_this_clone_out_of_git() {
    let root = Root::new();
    let folders = ["system", "services/orders", "services/billing"];
    for folder in folders {
        let status = Command::new("git")
            .args(["init", "-q"])
            .arg(root.0.join(folder))
            .status()
            .expect("run git init");
        assert!(status.success());
    }
    let (summary, directory) = run(&root.0, "read-only");
    assert_completed(&summary, &directory);
    for folder in folders {
        let folder = root.0.join(folder);
        assert!(folder.join("memory-bank/local/context.db").is_file());
        let output = Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=all"])
            .current_dir(&folder)
            .output()
            .expect("run git status");
        let listed = String::from_utf8_lossy(&output.stdout);
        // Brain records stay visible for review; the clone's own state does not.
        assert!(listed.contains("project-brain/dynamic/tasks/"), "{listed}");
        assert!(
            !listed.contains("memory-bank/") && !listed.contains("project-brain/local/"),
            "{}: {listed}",
            folder.display()
        );
    }
}

#[test]
fn an_edit_run_adopts_the_reported_source_change_into_its_checkpoint() {
    let root = Root::new();
    let (summary, directory) = run(&root.0, "edit");
    assert_completed(&summary, &directory);
    assert_eq!(
        fs::read_to_string(root.0.join("services/billing/src/work.txt")).unwrap(),
        "after\n"
    );
    let checkpoint: Value =
        serde_json::from_slice(&fs::read(directory.join("checkpoint.json")).unwrap()).unwrap();
    let file = checkpoint["snapshot"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["service"] == "billing" && entry["path"] == "src/work.txt")
        .expect("billing source fingerprinted");
    assert_eq!(
        file["sha256"],
        infra_core::query::sha256(b"after\n"),
        "the checkpoint follows the reported edit"
    );
}

/// Runs the plan with a one-line adapter that answers every dispatch alike.
fn run_with(root: &Path, answer: &str) -> (i32, Value, PathBuf) {
    let config = system(root);
    let config = config.to_str().unwrap();
    let services = root.join("services");
    let services = services.to_str().unwrap();
    let plan = root.join("plan.json");
    let (code, _, text) = cli(
        root,
        &[
            "plan",
            "--system",
            config,
            "--allow-root",
            services,
            "--task",
            "Bill every placed order",
            "--change-id",
            "bill-orders",
            "--service",
            "orders",
            "--output",
            plan.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{text}");
    let script = root.join("answer.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{answer}'\n"),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let directory = root.join("run");
    let (code, summary, _) = cli(
        root,
        &[
            "execute",
            "--system",
            config,
            "--allow-root",
            services,
            "--plan",
            plan.to_str().unwrap(),
            "--run-dir",
            directory.to_str().unwrap(),
            "--provider",
            "command",
            "--executable",
            script.to_str().unwrap(),
            "--mode",
            "read-only",
            "--timeout",
            "60",
        ],
    );
    (code, summary, directory)
}

#[test]
fn a_blocked_agreement_keeps_its_report_and_proposed_order() {
    let root = Root::new();
    let (code, summary, directory) = run_with(
        &root.0,
        r#"{"status":"blocked","summary":"Refund rule for zero-value orders is undecided","checks":[{"name":"zero_value_refund_decision","status":"failed","detail":"Decide whether zero-value orders are refunded"}],"changed_files":[],"service_order":["orders","billing"]}"#,
    );
    assert_eq!(
        code, 1,
        "a blocked agreement is a saved blocked run: {summary:#}"
    );
    assert_eq!(summary["status"], "blocked");
    assert_eq!(
        summary["error"],
        "worker_blocked_or_verification_incomplete"
    );
    let receipt: Value =
        serde_json::from_slice(&fs::read(directory.join("contracts-a1.json")).unwrap()).unwrap();
    assert_eq!(receipt["report"]["status"], "blocked", "{receipt:#}");
    assert_eq!(
        receipt["report"]["service_order"],
        json!(["orders", "billing"])
    );
    assert!(receipt.get("rejection").is_none(), "{receipt:#}");
}

#[test]
fn a_rejected_answer_records_why_it_was_rejected() {
    let root = Root::new();
    let (code, summary, directory) = run_with(
        &root.0,
        r#"{"status":"completed","summary":"Done","checks":[],"changed_files":[],"service_order":["orders"]}"#,
    );
    assert_eq!(code, 1, "{summary:#}");
    assert_eq!(summary["error"], "invalid_or_sensitive_worker_result");
    let receipt: Value =
        serde_json::from_slice(&fs::read(directory.join("contracts-a1.json")).unwrap()).unwrap();
    assert!(receipt["report"].is_null());
    assert_eq!(
        receipt["rejection"], "Contract worker must order every selected service exactly once",
        "{receipt:#}"
    );
    // The journal still reads back: a rejection is part of the receipt contract.
    let config = root.0.join("system/system.json");
    let services = root.0.join("services");
    let (code, status, text) = cli(
        &root.0,
        &[
            "run-status",
            "--system",
            config.to_str().unwrap(),
            "--allow-root",
            services.to_str().unwrap(),
            "--run-dir",
            directory.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(status["status"], "blocked");
}

#[test]
fn a_read_only_worker_that_reports_writes_stops_the_run_and_keeps_its_report() {
    let root = Root::new();
    let (code, summary, directory) = run_with(
        &root.0,
        r#"{"status":"completed","summary":"Agreed; a tool left a marker","checks":[{"name":"contracts","status":"passed","detail":"agreed"}],"changed_files":["billing/.tool/marker"],"service_order":["orders","billing"]}"#,
    );
    assert_eq!(code, 1, "{summary:#}");
    assert_eq!(summary["error"], "read_only_worker_reported_writes");
    let receipt: Value =
        serde_json::from_slice(&fs::read(directory.join("contracts-a1.json")).unwrap()).unwrap();
    assert_eq!(receipt["ok"], false);
    assert_eq!(
        receipt["report"]["changed_files"],
        json!(["billing/.tool/marker"])
    );
    assert_eq!(receipt["report"]["status"], "completed", "{receipt:#}");
}

#[test]
fn a_service_worker_echoing_the_agreed_order_keeps_its_work() {
    let root = Root::new();
    let (code, summary, directory) = run_with(
        &root.0,
        r#"{"status":"completed","summary":"Done","checks":[{"name":"unit","status":"passed","detail":"ok"}],"changed_files":[],"service_order":["orders","billing"]}"#,
    );
    assert_eq!(code, 0, "{summary:#}");
    assert_eq!(summary["status"], "completed");
    let receipt: Value =
        serde_json::from_slice(&fs::read(directory.join("service-billing-a1.json")).unwrap())
            .unwrap();
    assert_eq!(receipt["ok"], true, "{receipt:#}");
    assert_eq!(
        receipt["report"]["service_order"],
        json!(["orders", "billing"])
    );
}

/// Runs the plan with an adapter script whose body sees the prompt as `$input`.
fn run_script(root: &Path, body: &str, mode: &str) -> (i32, Value, PathBuf) {
    let config = system(root);
    let config = config.to_str().unwrap();
    let services = root.join("services");
    let services = services.to_str().unwrap();
    let plan = root.join("plan.json");
    let (code, _, text) = cli(
        root,
        &[
            "plan",
            "--system",
            config,
            "--allow-root",
            services,
            "--task",
            "Bill every placed order",
            "--change-id",
            "bill-orders",
            "--service",
            "orders",
            "--output",
            plan.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{text}");
    let script = root.join("script.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ninput=$(cat)\nreport() {{ printf '{{\"status\":\"completed\",\"summary\":\"%s\",\"checks\":[{{\"name\":\"unit\",\"status\":\"passed\",\"detail\":\"ok\"}}],\"changed_files\":%s,\"service_order\":%s}}\\n' \"$1\" \"$2\" \"$3\"; }}\n{body}\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let directory = root.join("run");
    let (code, summary, _) = cli(
        root,
        &[
            "execute",
            "--system",
            config,
            "--allow-root",
            services,
            "--plan",
            plan.to_str().unwrap(),
            "--run-dir",
            directory.to_str().unwrap(),
            "--provider",
            "command",
            "--executable",
            script.to_str().unwrap(),
            "--mode",
            mode,
            "--timeout",
            "60",
        ],
    );
    (code, summary, directory)
}

#[test]
fn a_service_worker_editing_another_service_stops_the_run() {
    let root = Root::new();
    let (code, summary, _) = run_script(
        &root.0,
        r#"case "$input" in
  *'"phase":"contracts","previous_receipts"'*) report agreed '[]' '["orders","billing"]' ;;
  *'"service":"billing","task"'*) printf 'tampered\n' > ../orders/src/work.txt; printf 'after\n' > src/work.txt; report billed '["src/work.txt"]' '[]' ;;
  *) report checked '[]' '[]' ;;
esac"#,
        "edit",
    );
    assert_eq!(code, 2, "{summary:#}");
    let journal: Value =
        serde_json::from_slice(&fs::read(root.0.join("run/run.json")).unwrap()).unwrap();
    assert_eq!(journal["status"], "blocked", "{journal:#}");
    assert!(
        journal["error"]
            .as_str()
            .unwrap()
            .contains("orders/src/work.txt changed (content changed) outside current worker scope; prepare a new plan"),
        "{journal:#}"
    );
}

#[test]
fn a_read_only_worker_changing_a_source_unreported_stops_the_run() {
    let root = Root::new();
    let (code, summary, _) = run_script(
        &root.0,
        r#"case "$input" in
  *'"phase":"contracts","previous_receipts"'*) printf 'tampered\n' > ../services/billing/src/work.txt; report agreed '[]' '["orders","billing"]' ;;
  *) report checked '[]' '[]' ;;
esac"#,
        "read-only",
    );
    assert_eq!(code, 2, "{summary:#}");
    let journal: Value =
        serde_json::from_slice(&fs::read(root.0.join("run/run.json")).unwrap()).unwrap();
    assert_eq!(journal["status"], "blocked", "{journal:#}");
    assert!(
        journal["error"]
            .as_str()
            .unwrap()
            .contains("billing/src/work.txt changed (content changed) outside current worker scope; prepare a new plan"),
        "{journal:#}"
    );
}

#[test]
fn an_executor_killed_mid_dispatch_resumes_only_with_an_explicit_retry() {
    let root = Root::new();
    let config = system(&root.0);
    let config = config.to_str().unwrap().to_string();
    let services = root.0.join("services");
    let services = services.to_str().unwrap().to_string();
    let plan = root.0.join("plan.json");
    let (code, _, text) = cli(
        &root.0,
        &[
            "plan",
            "--system",
            &config,
            "--allow-root",
            &services,
            "--task",
            "Bill every placed order",
            "--change-id",
            "bill-orders",
            "--service",
            "orders",
            "--output",
            plan.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{text}");
    // The billing worker hangs the first time, as if the machine stopped under it.
    let marker = root.0.join("billing-started");
    let script = root.0.join("slow.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ninput=$(cat)\nreport() {{ printf '{{\"status\":\"completed\",\"summary\":\"%s\",\"checks\":[{{\"name\":\"unit\",\"status\":\"passed\",\"detail\":\"ok\"}}],\"changed_files\":[],\"service_order\":%s}}\\n' \"$1\" \"$2\"; }}\ncase \"$input\" in\n  *'\"phase\":\"contracts\",\"previous_receipts\"'*) report agreed '[\"orders\",\"billing\"]' ;;\n  *'\"service\":\"billing\",\"task\"'*) if [ ! -e '{marker}' ]; then : > '{marker}'; sleep 60; fi; report billed '[]' ;;\n  *) report checked '[]' ;;\nesac\n",
            marker = marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let directory = root.0.join("run");
    let mut executor = Command::new(engine())
        .args([
            "system",
            "execute",
            "--system",
            &config,
            "--allow-root",
            &services,
            "--plan",
            plan.to_str().unwrap(),
            "--run-dir",
            directory.to_str().unwrap(),
            "--provider",
            "command",
            "--executable",
            script.to_str().unwrap(),
            "--mode",
            "read-only",
            "--timeout",
            "120",
        ])
        .current_dir(&root.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !marker.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "billing never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    executor.kill().unwrap();
    executor.wait().unwrap();
    // Wait until the dispatch's guardian released the run lock.
    std::thread::sleep(std::time::Duration::from_millis(500));

    let status_args = [
        "run-status",
        "--system",
        &config,
        "--allow-root",
        &services,
        "--run-dir",
        directory.to_str().unwrap(),
    ];
    let (code, status, text) = cli(&root.0, &status_args);
    assert_eq!(code, 0, "{text}");
    assert_eq!(
        status["status"], "interrupted",
        "nothing executes this run any more: {status:#}"
    );

    let resume = |extra: &[&str]| {
        let mut args = vec![
            "resume",
            "--system",
            &config,
            "--allow-root",
            &services,
            "--run-dir",
            directory.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        cli(&root.0, &args)
    };
    let (code, _, text) = resume(&[]);
    assert_eq!(code, 2, "{text}");
    assert!(
        text.contains("retry service-billing explicitly"),
        "the refusal says which dispatch to retry: {text}"
    );
    let (code, summary, text) = resume(&["--retry-step", "service-billing"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(summary["status"], "completed", "{summary:#}");
}

/// Verification blocks until the system policy it is given says the registry
/// is current; every other dispatch completes.
const REGISTRY_GATE: &str = r#"case "$input" in
  *'"phase":"contracts","previous_receipts"'*) report agreed '[]' '["orders","billing"]' ;;
  *'"phase":"verify","previous_receipts"'*)
    case "$input" in
      *'Registry lists every consumer'*) report verified '[]' '[]' ;;
      *) printf '{"status":"blocked","summary":"The topic registry misses billing","checks":[{"name":"registry","status":"failed","detail":"system AGENTS.md"}],"changed_files":[],"service_order":[]}\n' ;;
    esac ;;
  *) report checked '[]' '[]' ;;
esac"#;

fn resume(root: &Path, extra: &[&str]) -> (i32, Value, String) {
    let config = root.join("system/system.json");
    let services = root.join("services");
    let directory = root.join("run");
    let mut args = vec![
        "resume",
        "--system",
        config.to_str().unwrap(),
        "--allow-root",
        services.to_str().unwrap(),
        "--run-dir",
        directory.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    cli(root, &args)
}

#[test]
fn a_persons_edit_to_a_system_source_is_adopted_only_by_an_accepting_resume() {
    let root = Root::new();
    let (code, summary, directory) = run_script(&root.0, REGISTRY_GATE, "read-only");
    assert_eq!(code, 1, "{summary:#}");
    // No worker may write the system folder: a person fixes what verification found.
    let policy = root.0.join("system/AGENTS.md");
    fs::write(
        &policy,
        "Coordinate contract changes first.\nRegistry lists every consumer.\n",
    )
    .unwrap();
    let (code, _, text) = resume(&root.0, &["--retry-step", "verify"]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("explicitly accept source changes"), "{text}");
    let (code, summary, text) = resume(
        &root.0,
        &["--retry-step", "verify", "--accept-source-changes"],
    );
    assert_eq!(code, 0, "{text}");
    assert_completed(&summary, &directory);
    let checkpoint: Value =
        serde_json::from_slice(&fs::read(directory.join("checkpoint.json")).unwrap()).unwrap();
    let quoted = checkpoint["context"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["service"] == "__system__" && source["path"] == "AGENTS.md")
        .unwrap();
    assert!(
        quoted["excerpt"]
            .as_str()
            .unwrap()
            .contains("Registry lists every consumer"),
        "{quoted:#}"
    );
}

#[test]
fn an_edited_system_file_still_needs_a_new_plan() {
    let root = Root::new();
    let (code, summary, _) = run_script(&root.0, REGISTRY_GATE, "read-only");
    assert_eq!(code, 1, "{summary:#}");
    let config = root.0.join("system/system.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    registry["name"] = json!("Renamed fixture system");
    write_json(&config, &registry);
    let (code, _, text) = resume(
        &root.0,
        &["--retry-step", "verify", "--accept-source-changes"],
    );
    assert_eq!(code, 2, "{text}");
    assert!(
        text.contains("System folder system.json changed (content changed); the system file"),
        "{text}"
    );
}
