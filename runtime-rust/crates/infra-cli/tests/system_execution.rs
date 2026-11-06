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
/// (canonical JSON sorts keys, so the top-level `phase` and `service` are
/// followed by `shared_context`, which the earlier reports inside it are not).
/// In edit mode the billing worker changes a registered source and reports it.
fn adapter(root: &Path) -> PathBuf {
    let script = root.join("adapter.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
input=$(cat)
report() { printf '{"status":"completed","summary":"%s","checks":[{"name":"%s","status":"passed","detail":"fixture"}],"changed_files":%s,"service_order":%s}\n' "$1" "$2" "$3" "$4"; }
case "$input" in
  *'"readable":["__system__","billing","orders"]'*'"phase":"contracts","service":"__system__","shared_context"'*) ;;
  *'"phase":"contracts","service":"__system__","shared_context"'*) printf '{"status":"blocked","summary":"System folder not readable","checks":[],"changed_files":[],"service_order":[]}\n'; exit 0 ;;
esac
case "$input" in
  *'"phase":"contracts","service":"__system__","shared_context"'*) report "Contracts agreed" "contracts" '[]' '["orders","billing"]' ;;
  *'"phase":"verify","service":"__system__","shared_context"'*) report "Verified end to end" "end-to-end" '[]' '[]' ;;
  *'"mode":"edit"'*'"service":"billing","shared_context"'*) printf 'after\n' > src/work.txt; report "Billing updated" "unit" '["src/work.txt"]' '[]' ;;
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
        // Brain records and the system's new memory stay visible for review;
        // the clone's own state does not.
        assert!(listed.contains("project-brain/dynamic/tasks/"), "{listed}");
        assert!(
            !listed.contains("memory-bank/local/") && !listed.contains("project-brain/local/"),
            "{}: {listed}",
            folder.display()
        );
        if folder.ends_with("system") {
            assert!(listed.contains("memory-bank/chunks/MEM-"), "{listed}");
        }
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
    run_script_access(root, body, mode, "service")
}
fn run_script_access(root: &Path, body: &str, mode: &str, access: &str) -> (i32, Value, PathBuf) {
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
            "--access",
            access,
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
  *'"phase":"contracts","service":"__system__","shared_context"'*) report agreed '[]' '["orders","billing"]' ;;
  *'"service":"billing","shared_context"'*) printf 'tampered\n' > ../orders/src/work.txt; printf 'after\n' > src/work.txt; report billed '["src/work.txt"]' '[]' ;;
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
  *'"phase":"contracts","service":"__system__","shared_context"'*) printf 'tampered\n' > ../services/billing/src/work.txt; report agreed '[]' '["orders","billing"]' ;;
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
            "#!/bin/sh\ninput=$(cat)\nreport() {{ printf '{{\"status\":\"completed\",\"summary\":\"%s\",\"checks\":[{{\"name\":\"unit\",\"status\":\"passed\",\"detail\":\"ok\"}}],\"changed_files\":[],\"service_order\":%s}}\\n' \"$1\" \"$2\"; }}\ncase \"$input\" in\n  *'\"phase\":\"contracts\",\"service\":\"__system__\",\"shared_context\"'*) report agreed '[\"orders\",\"billing\"]' ;;\n  *'\"service\":\"billing\",\"shared_context\"'*) if [ ! -e '{marker}' ]; then : > '{marker}'; sleep 60; fi; report billed '[]' ;;\n  *) report checked '[]' ;;\nesac\n",
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
  *'"phase":"contracts","service":"__system__","shared_context"'*) report agreed '[]' '["orders","billing"]' ;;
  *'"phase":"verify","service":"__system__","shared_context"'*)
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

#[test]
fn a_saved_plan_is_never_replaced_and_the_refusal_says_so() {
    let root = Root::new();
    let config = system(&root.0);
    let services = root.0.join("services");
    let plan = root.0.join("plan.json");
    let args = [
        "plan",
        "--system",
        config.to_str().unwrap(),
        "--allow-root",
        services.to_str().unwrap(),
        "--task",
        "Bill every placed order",
        "--change-id",
        "bill-orders",
        "--service",
        "orders",
        "--output",
        plan.to_str().unwrap(),
    ];
    let (code, _, text) = cli(&root.0, &args);
    assert_eq!(code, 0, "{text}");
    let saved = fs::read(&plan).unwrap();
    let (code, _, text) = cli(&root.0, &args);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("already exists"), "{text}");
    assert_eq!(fs::read(&plan).unwrap(), saved);
}

/// Verification passes what it could run and reports one check it could not.
const UNRUN_GATE: &str = r#"case "$input" in
  *'"phase":"contracts","service":"__system__","shared_context"'*) report agreed '[]' '["orders","billing"]' ;;
  *'"phase":"verify","service":"__system__","shared_context"'*) printf '{"status":"completed","summary":"Verified locally","checks":[{"name":"unit","status":"passed","detail":"ok"},{"name":"broker_integration","status":"not_run","detail":"no broker here"}],"changed_files":[],"service_order":[]}\n' ;;
  *) report checked '[]' '[]' ;;
esac"#;

#[test]
fn a_person_may_accept_the_checks_verification_could_not_run() {
    let root = Root::new();
    let (code, summary, directory) = run_script(&root.0, UNRUN_GATE, "edit");
    assert_eq!(
        code, 1,
        "verifying a change with an unrun check stops the run: {summary:#}"
    );
    let (code, _, text) = resume(
        &root.0,
        &["--retry-step", "verify", "--accept-unrun-checks"],
    );
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("not both"), "{text}");
    let (code, summary, text) = resume(&root.0, &["--accept-unrun-checks"]);
    assert_eq!(code, 0, "{text}");
    assert_completed(&summary, &directory);
    let verify = summary["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == "verify")
        .unwrap();
    assert_eq!(
        verify["accepted_unrun_checks"],
        json!(["broker_integration"])
    );
    let handoff: Value =
        serde_json::from_slice(&fs::read(directory.join("handoff.json")).unwrap()).unwrap();
    let reported = handoff["reports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|report| report["phase"] == "verify")
        .unwrap();
    assert_eq!(
        reported["accepted_unrun_checks"],
        json!(["broker_integration"]),
        "the handoff says a person accepted the gaps: {reported:#}"
    );
    // The accepted receipt keeps the journal valid for later reads.
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
fn a_failed_verification_check_cannot_be_accepted() {
    let root = Root::new();
    let (code, summary, _) = run_script(&root.0, REGISTRY_GATE, "read-only");
    assert_eq!(code, 1, "{summary:#}");
    let (code, _, text) = resume(&root.0, &["--accept-unrun-checks"]);
    assert_eq!(code, 2, "{text}");
    assert!(
        text.contains("Only a completed verification whose only gaps are checks it could not run"),
        "{text}"
    );
}

#[test]
fn a_worker_that_exits_with_an_error_says_why_in_its_receipt_and_the_run() {
    let root = Root::new();
    let (code, summary, directory) = run_script(
        &root.0,
        r#"echo 'Not signed in: run the CLI login first'; exit 1"#,
        "read-only",
    );
    assert_eq!(code, 1, "{summary:#}");
    let receipt: Value =
        serde_json::from_slice(&fs::read(directory.join("contracts-a1.json")).unwrap()).unwrap();
    assert_eq!(receipt["error"], "worker_exit_failure");
    assert_eq!(receipt["failure"], "Not signed in: run the CLI login first");
    let journal: Value =
        serde_json::from_slice(&fs::read(directory.join("run.json")).unwrap()).unwrap();
    assert_eq!(
        journal["error"],
        "The command CLI exited with code 1: Not signed in: run the CLI login first"
    );
}

#[test]
fn in_the_all_scope_a_worker_may_edit_another_selected_service_it_reports() {
    let root = Root::new();
    let (code, summary, directory) = run_script_access(
        &root.0,
        r#"case "$input" in
  *'"phase":"contracts","service":"__system__","shared_context"'*) report agreed '[]' '["orders","billing"]' ;;
  *'"service":"billing","shared_context"'*) printf 'billed\n' > ../orders/src/work.txt; printf 'after\n' > src/work.txt; report billed '["orders/src/work.txt","billing/src/work.txt"]' '[]' ;;
  *) report checked '[]' '[]' ;;
esac"#,
        "edit",
        "all",
    );
    assert_eq!(code, 0, "{summary:#}");
    assert_completed(&summary, &directory);
    assert_eq!(
        fs::read_to_string(root.0.join("services/orders/src/work.txt")).unwrap(),
        "billed\n"
    );
    // An unreported edit in another service still stops the run in this scope.
    let other = Root::new();
    let (code, summary, _) = run_script_access(
        &other.0,
        r#"case "$input" in
  *'"phase":"contracts","service":"__system__","shared_context"'*) report agreed '[]' '["orders","billing"]' ;;
  *'"service":"billing","shared_context"'*) printf 'billed\n' > ../orders/src/work.txt; report billed '[]' '[]' ;;
  *) report checked '[]' '[]' ;;
esac"#,
        "edit",
        "all",
    );
    assert_eq!(code, 2, "{summary:#}");
    let journal: Value =
        serde_json::from_slice(&fs::read(other.0.join("run/run.json")).unwrap()).unwrap();
    assert!(
        journal["error"].as_str().unwrap().contains(
            "orders/src/work.txt changed (content changed) without being reported in changed_files"
        ),
        "{journal:#}"
    );
}

/// The journal a run leaves when it stops between dispatches (cancelled at a
/// loop start, or a source changed before the next dispatch): a completed
/// prefix and the next step pending.
fn stopped_between_dispatches(root: &Path) -> PathBuf {
    let (code, summary, directory) = run_script(root, REGISTRY_GATE, "read-only");
    assert_eq!(code, 1, "{summary:#}");
    let path = directory.join("run.json");
    let mut journal: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for step in journal["steps"].as_array_mut().unwrap() {
        if step["id"] == "verify" {
            step["status"] = json!("pending");
        }
    }
    fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
    directory
}

#[test]
fn between_dispatches_a_persons_system_edit_needs_acceptance_and_others_a_new_plan() {
    let root = Root::new();
    let directory = stopped_between_dispatches(&root.0);
    fs::write(
        root.0.join("system/AGENTS.md"),
        "Coordinate contract changes first.\nRegistry lists every consumer.\n",
    )
    .unwrap();
    let (code, _, text) = resume(&root.0, &[]);
    assert_eq!(code, 2, "{text}");
    assert!(
        text.contains("System folder AGENTS.md changed (content changed), and no worker may write the system folder"),
        "{text}"
    );
    let (code, summary, text) = resume(&root.0, &["--accept-source-changes"]);
    assert_eq!(code, 0, "{text}");
    assert_completed(&summary, &directory);

    let other = Root::new();
    stopped_between_dispatches(&other.0);
    fs::write(other.0.join("services/billing/src/work.txt"), "later\n").unwrap();
    let (code, _, text) = resume(&other.0, &["--accept-source-changes"]);
    assert_eq!(code, 2, "{text}");
    assert!(
        text.contains("billing/src/work.txt changed (content changed) while no worker was running; prepare a new plan"),
        "{text}"
    );
}

#[test]
fn a_read_only_run_asks_each_dispatch_to_investigate_not_to_agree_a_fix() {
    // Each dispatch completes only when its goal fits the run's mode.
    let gate = r#"case "$input" in
  *'"phase":"contracts","service":"__system__","shared_context"'*)
    case "$input" in *'This run only investigates'*) report agreed '[]' '["orders","billing"]' ;; *) printf '{"status":"blocked","summary":"wrong goal","checks":[],"changed_files":[],"service_order":[]}\n' ;; esac ;;
  *'"phase":"verify","service":"__system__","shared_context"'*)
    case "$input" in *'This run only investigated'*) report verified '[]' '[]' ;; *) printf '{"status":"blocked","summary":"wrong goal","checks":[],"changed_files":[],"service_order":[]}\n' ;; esac ;;
  *)
    case "$input" in *'Investigate the request in this service without editing'*) report checked '[]' '[]' ;; *) printf '{"status":"blocked","summary":"wrong goal","checks":[],"changed_files":[],"service_order":[]}\n' ;; esac ;;
esac"#;
    let root = Root::new();
    let (code, summary, directory) = run_script(&root.0, gate, "read-only");
    assert_eq!(code, 0, "{summary:#}");
    assert_completed(&summary, &directory);
    // An edit run keeps the agreement that may block on missing decisions.
    let edit = Root::new();
    let (code, summary, _) = run_script(&edit.0, gate, "edit");
    assert_eq!(code, 1, "{summary:#}");
}

#[test]
fn in_a_read_only_run_a_failed_check_is_a_finding_and_the_run_completes() {
    // The billing investigation reproduces the defect it was asked about.
    let gate = r#"case "$input" in
  *'"phase":"contracts","service":"__system__","shared_context"'*) report agreed '[]' '["orders","billing"]' ;;
  *'"service":"billing","shared_context"'*) printf '{"status":"completed","summary":"Defect reproduced","checks":[{"name":"reproduction","status":"failed","detail":"the defect is real"},{"name":"unit","status":"passed","detail":"ok"}],"changed_files":[],"service_order":[]}\n' ;;
  *) report checked '[]' '[]' ;;
esac"#;
    let root = Root::new();
    let (code, summary, directory) = run_script(&root.0, gate, "read-only");
    assert_eq!(code, 0, "{summary:#}");
    assert_completed(&summary, &directory);
    let receipt: Value =
        serde_json::from_slice(&fs::read(directory.join("service-billing-a1.json")).unwrap())
            .unwrap();
    assert_eq!(receipt["ok"], true);
    // Implementing a change, the same failed check stops the run.
    let edit = Root::new();
    let (code, summary, _) = run_script(&edit.0, gate, "edit");
    assert_eq!(code, 1, "{summary:#}");
}

#[test]
fn a_changed_contract_may_be_named_by_its_id_alone() {
    let root = Root::new();
    let config = system(&root.0);
    let services = root.0.join("services");
    let plan = |contract: &str| {
        cli(
            &root.0,
            &[
                "plan",
                "--system",
                config.to_str().unwrap(),
                "--allow-root",
                services.to_str().unwrap(),
                "--task",
                "Add the currency to placed orders",
                "--change-id",
                "currency",
                "--contract",
                contract,
            ],
        )
    };
    let (code, value, text) = plan("order.placed");
    assert_eq!(code, 0, "{text}");
    let impact = value["impact"].as_array().unwrap();
    assert!(
        impact
            .iter()
            .any(|entry| entry["service"] == "orders" && entry["reason"] == "contract_provider"),
        "{value:#}"
    );
    assert!(
        impact.iter().any(|entry| entry["service"] == "billing"),
        "its consumer joins: {value:#}"
    );
    let (code, _, text) = plan("order.shipped");
    assert_eq!(code, 2, "{text}");
    assert!(
        text.contains("No registered service provides contract order.shipped"),
        "{text}"
    );
}

#[test]
fn init_creates_a_new_system_folder_and_says_so_when_one_exists() {
    let root = Root::new();
    let folder = root.0.join("new-system");
    let args = [
        "init",
        "--root",
        folder.to_str().unwrap(),
        "--name",
        "New system",
    ];
    let (code, value, text) = cli(&root.0, &args);
    assert_eq!(code, 0, "{text}");
    assert_eq!(value["services"], 0);
    assert!(folder.join("system.json").is_file());
    let (code, _, text) = cli(&root.0, &args);
    assert_eq!(code, 2, "{text}");
    assert!(
        text.contains("already exists; init creates a new system folder"),
        "{text}"
    );
}

#[test]
fn a_completed_agreement_proceeds_though_a_check_confirms_the_defect_to_fix() {
    // The contract agent agrees the fix and reports the defect it is for.
    let gate = r#"case "$input" in
  *'"phase":"contracts","service":"__system__","shared_context"'*) printf '{"status":"completed","summary":"Agreed: reject missing identities","checks":[{"name":"current_handler","status":"failed","detail":"charges without an identity today"},{"name":"compatibility","status":"passed","detail":"v1 unchanged"}],"changed_files":[],"service_order":["orders","billing"]}\n' ;;
  *) report checked '[]' '[]' ;;
esac"#;
    let root = Root::new();
    let (code, summary, directory) = run_script(&root.0, gate, "edit");
    assert_eq!(code, 0, "{summary:#}");
    assert_completed(&summary, &directory);
}

#[test]
fn every_agent_receives_one_shared_context_with_the_complete_earlier_reports() {
    // The contract agent's answer is longer than the 1000 characters and the
    // ten checks a later agent used to see of it.
    let root = Root::new();
    let agreement = json!({"status":"completed",
        "summary":format!("Agreed: {}", "billing charges each placed order once. ".repeat(40)),
        "checks":(0..12).map(|n|json!({"name":format!("invariant-{n}"),"status":"passed","detail":format!("invariant {n} holds")})).collect::<Vec<_>>(),
        "changed_files":[],"service_order":["orders","billing"]});
    let answer = root.0.join("agreement.json");
    fs::write(&answer, serde_json::to_string(&agreement).unwrap() + "\n").unwrap();
    let gate = format!(
        r#"case "$input" in
  *'"phase":"contracts","service":"__system__","shared_context"'*) cat '{}' ;;
  *'"service":"billing","shared_context"'*) printf 'after\n' > src/work.txt; report billed '["src/work.txt"]' '[]' ;;
  *) report checked '[]' '[]' ;;
esac"#,
        answer.display()
    );
    let (code, summary, directory) = run_script(&root.0, &gate, "edit");
    assert_eq!(code, 0, "{summary:#}");
    assert_completed(&summary, &directory);
    let read = |name: &str| -> Value {
        serde_json::from_slice(&fs::read(directory.join(name)).unwrap()).unwrap()
    };
    let handoff = read("handoff.json");
    let reports = handoff["reports"].as_array().unwrap();
    assert_eq!(reports[0]["summary"], agreement["summary"]);
    assert_eq!(reports[0]["checks"], agreement["checks"]);
    assert!(Path::new(reports[0]["receipt"].as_str().unwrap()).is_file());
    let excerpt = |shared: &Value| {
        shared["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["service"] == "billing" && source["path"] == "src/work.txt")
            .map(|source| source["excerpt"].clone())
            .unwrap()
    };
    let mut unchanged = None;
    for (index, (phase, writable)) in [
        ("contracts", json!([])),
        ("service-orders", json!(["orders"])),
        ("service-billing", json!(["billing"])),
        ("verify", json!([])),
    ]
    .into_iter()
    .enumerate()
    {
        let input = read(&format!("{phase}-a1.input.json"));
        let prompt = input["prompt"].as_str().unwrap();
        // The receipt records the digest of exactly the saved input.
        assert_eq!(
            input["input_sha256"],
            infra_core::query::sha256(prompt.as_bytes())
        );
        assert_eq!(
            read(&format!("{phase}-a1.json"))["input_sha256"],
            input["input_sha256"]
        );
        let payload: Value = serde_json::from_str(prompt.rsplit_once('\n').unwrap().1).unwrap();
        assert_eq!(payload["phase"], phase);
        // Every agent reads the system folder and both services; the
        // own-service scope keeps writes in the agent's own folder.
        assert_eq!(
            payload["access"]["readable"],
            json!(["__system__", "billing", "orders"]),
            "{phase}"
        );
        assert_eq!(payload["access"]["writable"], writable, "{phase}");
        let shared = &payload["shared_context"];
        // Each agent receives the complete reports of every agent before it.
        assert_eq!(shared["reports"], json!(reports[..index]), "{phase}");
        assert_eq!(shared["reports_condensed"], false);
        // An excerpt follows the earlier agents' edits: verification reads
        // what the billing agent wrote.
        assert_eq!(
            excerpt(shared),
            if phase == "verify" {
                "after\n"
            } else {
                "before\n"
            },
            "{phase}"
        );
        let mut rest = shared.clone();
        rest["reports"] = Value::Null;
        rest["sources"] = Value::Null;
        match &unchanged {
            None => unchanged = Some(rest),
            Some(first) => assert_eq!(&rest, first, "{phase}"),
        }
    }
}

/// `infra-engine memory --root PROJECT ...` as a person runs it, isolated from
/// this machine's accelerator and owner settings.
fn memory(root: &Path, project: &Path, args: &[&str]) -> Value {
    let mut command = Command::new(engine());
    command
        .arg("memory")
        .arg("--root")
        .arg(project)
        .args(args)
        .current_dir(root)
        .env("HOME", root.join("home"))
        .stdin(Stdio::null());
    for key in [
        "PROJECT_BRAIN_OWNER",
        "PROJECT_BRAIN_MODE",
        "ACCELERATOR_HOME",
        "ACCELERATOR_STATE_DIR",
        "ACCELERATOR_PROJECT_DIR",
    ] {
        command.env_remove(key);
    }
    let output = command.output().expect("run infra-engine memory");
    assert!(
        output.status.success(),
        "memory {args:?}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn system_chunks(system: &Path) -> Vec<String> {
    fs::read_dir(system.join("memory-bank/chunks"))
        .map(|entries| {
            entries
                .map(|entry| {
                    format!(
                        "memory-bank/chunks/{}",
                        entry.unwrap().file_name().into_string().unwrap()
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}
/// Plans `refund-orders` and runs it read-only with the saved script; returns
/// the plan and each dispatch's shared context.
fn refund_run(root: &Path) -> (Value, Vec<Value>) {
    let config = root.join("system/system.json");
    let services = root.join("services");
    let plan = root.join("refund-plan.json");
    let (code, value, text) = cli(
        root,
        &[
            "plan",
            "--system",
            config.to_str().unwrap(),
            "--allow-root",
            services.to_str().unwrap(),
            "--task",
            "Refund a cancelled placed order",
            "--change-id",
            "refund-orders",
            "--service",
            "orders",
            "--output",
            plan.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{text}");
    let run = root.join("refund-run");
    let script = root.join("script.sh");
    let (code, summary, text) = cli(
        root,
        &[
            "execute",
            "--system",
            config.to_str().unwrap(),
            "--allow-root",
            services.to_str().unwrap(),
            "--plan",
            plan.to_str().unwrap(),
            "--run-dir",
            run.to_str().unwrap(),
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
    assert_eq!(code, 0, "{text}");
    assert_eq!(summary["status"], "completed", "{summary:#}");
    let shared = ["contracts", "service-orders", "service-billing", "verify"]
        .iter()
        .map(|phase| {
            let input: Value = serde_json::from_slice(
                &fs::read(run.join(format!("{phase}-a1.input.json"))).unwrap(),
            )
            .unwrap();
            let prompt = input["prompt"].as_str().unwrap();
            let payload: Value = serde_json::from_str(prompt.rsplit_once('\n').unwrap().1).unwrap();
            payload["shared_context"].clone()
        })
        .collect();
    (value, shared)
}
fn carries(sources: &Value, path: &str, trust: &str) -> bool {
    sources.as_array().unwrap().iter().any(|source| {
        source["service"] == "__system__"
            && source["path"] == path
            && source["kind"] == "memory"
            && source["trust"] == trust
    })
}
const AGREEMENT: &str = r#"case "$input" in
  *'"phase":"contracts","service":"__system__","shared_context"'*) report "Agreed: billing charges each placed order exactly once" '[]' '["orders","billing"]' ;;
  *) report "Checked" '[]' '[]' ;;
esac"#;

#[test]
fn a_runs_outcome_becomes_system_memory_that_every_later_agent_shares() {
    let root = Root::new();
    let (code, summary, directory) = run_script(&root.0, AGREEMENT, "read-only");
    assert_eq!(code, 0, "{summary:#}");
    assert_completed(&summary, &directory);
    let handoff: Value =
        serde_json::from_slice(&fs::read(directory.join("handoff.json")).unwrap()).unwrap();
    let shared = &handoff["memory"];
    assert_eq!(shared["status"], "applied", "{handoff:#}");
    assert_eq!(shared["review_mode"], "automatic");
    assert!(
        handoff["knowledge"]
            .as_str()
            .unwrap()
            .contains("now the system's memory")
    );
    let system = root.0.join("system");
    let chunks = system_chunks(&system);
    assert_eq!(chunks.len(), 1, "{chunks:?}");
    let chunk = &chunks[0];
    let text = fs::read_to_string(system.join(chunk)).unwrap();
    assert!(text.contains("billing charges each placed order exactly once"));
    assert!(text.contains("auto-promoted") && text.contains("agent-attested"));
    // The next change gives it to every agent, marked as memory no person reviewed.
    let (plan, inputs) = refund_run(&root.0);
    assert!(
        carries(&plan["context"]["sources"], chunk, "automatic_memory"),
        "{plan:#}"
    );
    for shared in inputs {
        assert!(carries(&shared["sources"], chunk, "automatic_memory"));
    }
}

#[test]
fn a_system_that_turns_automatic_promotion_off_keeps_the_outcome_for_a_person() {
    let root = Root::new();
    system(&root.0);
    write_json(
        &root.0.join("system/project-brain/config/runtime.json"),
        &json!({"automatic_promotion":false}),
    );
    let (code, summary, directory) = run_script(&root.0, AGREEMENT, "read-only");
    assert_eq!(code, 0, "{summary:#}");
    assert_completed(&summary, &directory);
    let handoff: Value =
        serde_json::from_slice(&fs::read(directory.join("handoff.json")).unwrap()).unwrap();
    let proposed = &handoff["memory"];
    assert_eq!(proposed["status"], "proposed", "{handoff:#}");
    assert_eq!(proposed["review_mode"], "human");
    let system = root.0.join("system");
    assert!(system_chunks(&system).is_empty());
    let promotion = proposed["proposal_id"].as_str().unwrap();
    let reviewed = memory(
        &root.0,
        &system,
        &[
            "promote-review",
            "--promotion-id",
            promotion,
            "--reviewer",
            "person",
            "--json",
        ],
    );
    assert_eq!(reviewed["status"], "reviewed", "{reviewed:#}");
    let applied = memory(
        &root.0,
        &system,
        &["promote-apply", "--promotion-id", promotion, "--json"],
    );
    assert_eq!(applied["status"], "applied", "{applied:#}");
    let chunks = system_chunks(&system);
    assert_eq!(chunks.len(), 1);
    let (plan, inputs) = refund_run(&root.0);
    assert!(
        carries(&plan["context"]["sources"], &chunks[0], "reviewed_memory"),
        "{plan:#}"
    );
    for shared in inputs {
        assert!(carries(&shared["sources"], &chunks[0], "reviewed_memory"));
    }
}

/// A Memory Bank chunk as a person publishes one in `project`: a verified
/// finding citing `sources`, proposed by its author, approved by another.
fn reviewed_memory(project: &Path, title: &str, content: &str, sources: &[&str]) -> String {
    use infra_core::{
        BankService, BrainRepository, CreateRecord, Roots, RuntimeContext, UpdateRecord,
    };
    let context = |actor: &str| {
        RuntimeContext::new(
            Roots::installed(project.to_path_buf()).unwrap(),
            actor,
            "governed",
        )
        .unwrap()
    };
    let brain = BrainRepository::new(context("author")).unwrap();
    let record = brain
        .create(CreateRecord {
            record_type: "finding".into(),
            external_id: "BILLING-ONCE".into(),
            title: title.into(),
            goal: Some(title.into()),
            sources: sources.iter().map(|source| source.to_string()).collect(),
            authority: "verified".into(),
            attestation: Some("person".into()),
            ..CreateRecord::default()
        })
        .unwrap();
    let id = record["id"].as_str().unwrap().to_string();
    brain
        .update(
            &id,
            UpdateRecord {
                expected_revision: record["revision"].as_u64(),
                progress: Some(content.into()),
                transition_to: Some("resolved".into()),
                reason: "Checked against the code".into(),
                attestation: Some("person".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let proposal = BankService::new(context("author"))
        .propose(&[id], title, content, false)
        .unwrap();
    let reviewer = BankService::new(context("reviewer"));
    let promotion = proposal["id"].as_str().unwrap();
    reviewer.review(promotion, Some("reviewer"), true).unwrap();
    let applied = reviewer.apply(promotion).unwrap();
    let memory = applied["destination_memory_id"].as_str().unwrap();
    let name = fs::read_dir(project.join("memory-bank/chunks"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .find(|name| name.starts_with(memory))
        .unwrap();
    format!("memory-bank/chunks/{name}")
}

#[test]
fn memory_a_worker_outdates_leaves_the_context_and_the_run_goes_on() {
    let root = Root::new();
    let config = system(&root.0);
    let chunk = reviewed_memory(
        &root.0.join("services/billing"),
        "Billing work starts from the before state",
        "The billing work file holds the before state until a placed order is billed.",
        &["src/work.txt"],
    );
    let services = root.0.join("services");
    let plan = root.0.join("plan.json");
    let (code, value, text) = cli(
        &root.0,
        &[
            "plan",
            "--system",
            config.to_str().unwrap(),
            "--allow-root",
            services.to_str().unwrap(),
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
    let carries = |sources: &Value| {
        sources
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source["service"] == "billing" && source["path"] == chunk.as_str())
    };
    assert!(carries(&value["context"]["sources"]), "{value:#}");
    // The billing agent rewrites the file the memory describes.
    let script = adapter(&root.0);
    let directory = root.0.join("run");
    let (code, summary, text) = cli(
        &root.0,
        &[
            "execute",
            "--system",
            config.to_str().unwrap(),
            "--allow-root",
            services.to_str().unwrap(),
            "--plan",
            plan.to_str().unwrap(),
            "--run-dir",
            directory.to_str().unwrap(),
            "--provider",
            "command",
            "--executable",
            script.to_str().unwrap(),
            "--mode",
            "edit",
            "--timeout",
            "60",
        ],
    );
    assert_eq!(code, 0, "{text}");
    assert_completed(&summary, &directory);
    let shared = |phase: &str| -> Value {
        let input: Value = serde_json::from_slice(
            &fs::read(directory.join(format!("{phase}-a1.input.json"))).unwrap(),
        )
        .unwrap();
        let prompt = input["prompt"].as_str().unwrap();
        let payload: Value = serde_json::from_str(prompt.rsplit_once('\n').unwrap().1).unwrap();
        payload["shared_context"].clone()
    };
    assert!(carries(&shared("service-billing")["sources"]));
    let after = shared("verify");
    assert!(!carries(&after["sources"]));
    assert!(
        after["omitted_sources"]
            .as_array()
            .unwrap()
            .contains(&json!({"service":"billing","path":chunk,"reason":"memory_no_longer_holds"})),
        "{:#}",
        after["omitted_sources"]
    );
}
