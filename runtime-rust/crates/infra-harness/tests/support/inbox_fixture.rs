//! Offline process fixture: itself is the guardian and the fake provider CLI.
use infra_harness::{
    HarnessError, Result,
    accelerators::Accelerators,
    bindings::{ResultsBridge, SessionLinks},
    commands::CommandCatalog,
    fleet,
    knowledge::KnowledgeDomain,
    provider::{Provider, ProviderDomain},
    results::Results,
    services::{Domain, Payload},
    sessions::{BlackboardFactory, Event, RunReply, SessionLimits, Sessions, SpecialWorkflow},
    store::SessionStore,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::Path,
    sync::{Arc, RwLock, atomic::AtomicBool},
    time::{Duration, Instant},
};
mod inbox_browser;

fn require(valid: bool, reason: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(HarnessError::invalid(reason))
    }
}
fn value(payload: Payload) -> Result<Value> {
    match payload {
        Payload::Json(value) => Ok(value),
        _ => Err(HarnessError::invalid("Expected fixture JSON.")),
    }
}
struct Unused;
impl BlackboardFactory for Unused {
    fn for_session(&self, _: &Value) -> Result<Arc<dyn fleet::Blackboard>> {
        panic!("No fixture blackboard")
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
        panic!("No fixture specialized workflow")
    }
}
async fn wait(mut ready: impl FnMut() -> Result<bool>) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(15);
    while !ready()? {
        require(Instant::now() < until, "Inbox process fixture deadline.")?;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

fn fake_provider(arguments: &[String]) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = cwd
        .parent()
        .ok_or_else(|| HarnessError::invalid("No fixture parent."))?;
    // Reserve every process attempt, even an unexpected duplicate, before
    // reading stdin or refusing it. A missing call-2 must be observable proof.
    let mut index = 0;
    let mut record = loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(format!("call-{index}.json")))
        {
            Ok(record) => break record,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => index += 1,
            Err(error) => return Err(error.into()),
        }
    };
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt)?;
    write!(
        record,
        "{}",
        json!({"argv":arguments,"cwd":cwd,"prompt":prompt,"helper_limit":std::env::var("CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS").ok()})
    )?;
    record.flush()?;
    let browser = root.join("browser-fixture").exists();
    require(
        index < if browser { 16 } else { 2 },
        "Unexpected provider turn.",
    )?;
    let mut output = std::io::stdout().lock();
    writeln!(
        output,
        "{}",
        json!({"type":"system","subtype":"init","session_id":"native-fixture"})
    )?;
    output.flush()?;
    if index == 0 {
        let until = Instant::now() + Duration::from_secs(if browser { 600 } else { 15 });
        while !root.join("release-source").exists() {
            require(Instant::now() < until, "Fake provider release deadline.")?;
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let identity = if index == 0 && root.join("conflicting-result").exists() {
        "wrong-native"
    } else {
        "native-fixture"
    };
    writeln!(
        output,
        "{}",
        json!({"type":"result","subtype":"success","is_error":false,"session_id":identity,"result":"Fixture complete."})
    )?;
    output.flush()?;
    Ok(())
}

async fn run(root: &Path, conflict: bool) -> Result<Value> {
    let root = infra_core::fs::canonicalize(root)?;
    let project = root.join("project");
    let source = root.join("source");
    fs::create_dir(&project)?;
    fs::create_dir(&source)?;
    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .arg(&project)
        .status()?;
    require(init.success(), "Fixture Git init failed.")?;
    if conflict {
        fs::write(root.join("conflicting-result"), b"fixture")?;
    }
    let store = SessionStore::open(&root.join("state"))?;
    let registered = store.register(&project)?;
    let executable = std::env::current_exe()?;
    let providers = Arc::new(ProviderDomain {
        providers: Arc::new(RwLock::new(BTreeMap::from([(
            "claude".into(),
            Provider {
                id: "claude".into(),
                name: "offline fixture".into(),
                available: true,
                executable: Some(executable),
                detail: "No model or network".into(),
            },
        )]))),
    });
    // Claude's versioned catalog is static; this does not discover or call a CLI.
    infra_host::model_catalog::model_options("claude", None, None).await?;
    let knowledge = Arc::new(KnowledgeDomain::new(store.clone(), source.clone()));
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
            seconds: 20,
            concurrency: 1,
        },
    )?;
    let links = Arc::new(SessionLinks::new());
    links.bind(Arc::downgrade(&sessions))?;
    let results = Arc::new(Results::new(
        store.clone(),
        Arc::new(ResultsBridge {
            links,
            knowledge,
            operation: sessions.operation_lock(),
            cancel: sessions.shutdown_cancel(),
        }),
    )?);
    sessions.install_results(results)?;
    sessions.install_commands(Arc::new(CommandCatalog::new(
        store.clone(),
        source.clone(),
        providers.providers.clone(),
    )))?;
    sessions.install_accelerators(Arc::new(Accelerators::new(
        store.clone(),
        source,
        providers,
        Arc::downgrade(&sessions),
    )?))?;
    sessions.activate()?;
    let outcome: Result<Value> = async {
        let created = value(sessions.execute("sessions.create", json!({"body":{"project_id":registered["id"],"provider":"claude","workflow":"native","mode":"plan","agents_enabled":false,"agent_count":1,"project_context":false,"budgets":{"seconds":20},"prompt":"Source fixture question."}})).await?)?;
        let sid = created["session"]["id"].as_str().ok_or_else(|| HarnessError::invalid("Missing fixture session."))?;
        wait(|| Ok(store.get(sid)?["native_session_id"] == "native-fixture")).await?;
        let original = store.generation(sid)?;
        let accepted = value(sessions.execute("sessions.inbox_accept", json!({"params":{"session_id":sid},"body":{"request_id":"next","prompt":"Follow-up fixture question."}})).await?)?;
        require(accepted["status"] == "pending", "Expected saved pending receipt.")?;
        require(!root.join("call-1.json").exists(), "Follow-up started before source release.")?;
        fs::write(root.join("release-source"), b"ready")?;
        let mut terminal = Value::Null;
        wait(|| {
            terminal = value_sync(&sessions, sid)?;
            Ok(terminal["status"] == if conflict { "blocked" } else { "completed" })
        }).await?;
        require(store.get(sid)?["status"] == if conflict { "failed" } else { "completed" }, "Unexpected terminal session state.")?;
        if conflict {
            require(!root.join("call-1.json").exists(), "Conflicting identity started a follow-up.")?;
            require(store.generation(sid)? == original, "Blocked source changed generation.")?;
        } else {
            let raw = fs::read(root.join("call-1.json"))?;
            let call: Value = serde_json::from_slice(&raw)?;
            let argv = call["argv"].as_array().ok_or_else(|| HarnessError::invalid("Missing argv."))?;
            require(argv.windows(2).any(|pair| pair == [json!("--resume"),json!("native-fixture")]), "Missing preserved resume ID.")?;
            require(argv.windows(2).any(|pair| pair == [json!("--permission-mode"),json!("plan")]), "Mode changed.")?;
            require(argv.windows(4).any(|pair| pair == [json!("--disallowedTools"),json!("Agent"),json!("Task"),json!("Workflow")]), "Helper tool restrictions changed.")?;
            require(call["helper_limit"] == "0", "Native helper concurrency changed.")?;
            require(call["cwd"] == project.to_string_lossy().as_ref(), "Workspace changed.")?;
            let prompt = call["prompt"].as_str().unwrap_or("");
            require(prompt.matches("Follow-up fixture question.").count() == 1 && !prompt.contains("Source fixture question."), "Follow-up prompt missing or duplicated.")?;
            require(terminal["delivery_generation"] == store.generation(sid)?, "Receipt generation does not match final turn.")?;
            let events = store.events(sid, 0)?;
            require(events.iter().filter(|event| event["inbox_request_id"] == "next").count() == 1, "Duplicate local assignment.")?;
        }
        let replay = value(sessions.execute("sessions.inbox_accept", json!({"params":{"session_id":sid},"body":{"request_id":"next","prompt":"Follow-up fixture question."}})).await?)?;
        require(replay["id"] == accepted["id"] && replay["status"] == terminal["status"], "Terminal idempotency failed.")?;
        Ok(json!({"scenario":if conflict {"conflict"} else {"success"},"receipt":terminal["status"]}))
    }.await;
    let closed = sessions.close();
    let drained = sessions.wait_closed().await;
    closed?;
    drained?;
    let mut outcome = outcome?;
    let calls = fs::read_dir(&root)?
        .collect::<std::io::Result<Vec<_>>>()?
        .iter()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("call-") && name.ends_with(".json"))
        })
        .count();
    require(
        calls == if conflict { 1 } else { 2 },
        "Unexpected actual provider process attempt count after cleanup.",
    )?;
    outcome["calls"] = calls.into();
    Ok(outcome)
}

// The dispatcher and the fixture share the same SQLite store; polling is read-only.
fn value_sync(sessions: &Sessions, sid: &str) -> Result<Value> {
    let db = sessions.store.connection()?;
    let raw: String = db.query_row(
        "SELECT status FROM session_inbox WHERE session_id=?1 AND request_id='next'",
        [sid],
        |row| row.get(0),
    )?;
    let generation: Option<String> = db.query_row(
        "SELECT delivery_generation FROM session_inbox WHERE session_id=?1 AND request_id='next'",
        [sid],
        |row| row.get(0),
    )?;
    Ok(json!({"status":raw,"delivery_generation":generation}))
}

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().is_some_and(|arg| arg == "internal-guard") {
        std::process::exit(infra_host::process::guard_entry(&arguments[1..]).unwrap_or(125));
    }
    if arguments.first().is_some_and(|arg| arg == "--print") {
        if let Err(error) = fake_provider(&arguments) {
            eprintln!("{}", error.message);
            std::process::exit(2);
        }
        return;
    }
    if arguments.first().is_some_and(|arg| arg == "browser") {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .unwrap();
        if let Err(error) = runtime.block_on(inbox_browser::serve(
            Path::new(&arguments[1]),
            Path::new(&arguments[2]),
        )) {
            eprintln!("{}", error.message);
            std::process::exit(2);
        }
        return;
    }
    let root = arguments.first().expect("fixture directory");
    let conflict = arguments.get(1).is_some_and(|arg| arg == "conflict");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .unwrap();
    match runtime.block_on(run(Path::new(root), conflict)) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{}", error.message);
            std::process::exit(2);
        }
    }
}
