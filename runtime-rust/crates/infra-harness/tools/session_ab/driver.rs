use super::{corpus, digest, oracle, ordinary, require, write_json};
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
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value as V, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, RwLock, atomic::AtomicBool},
    time::{Duration, Instant},
};

fn payload(value: Payload) -> Result<V> {
    if let Payload::Json(value) = value {
        Ok(value)
    } else {
        Err(HarnessError::invalid("Expected JSON."))
    }
}
fn record(root: &Path, phase: &str, data: V) -> Result<()> {
    let db = Connection::open_with_flags(
        root.join("attempts.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute(
        "INSERT INTO trace(phase,data) VALUES(?1,?2)",
        params![phase, data.to_string()],
    )?;
    Ok(())
}
fn traces(root: &Path) -> Result<Vec<V>> {
    let db = Connection::open_with_flags(
        root.join("attempts.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    let mut stmt = db.prepare("SELECT phase,data FROM trace ORDER BY rowid")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(phase, data)| Ok(json!({"phase":phase,"data":serde_json::from_str::<V>(&data)?})))
        .collect()
}
fn cleanup(receipt: &V) -> bool {
    receipt["full_cost"]["native_diagnostics"]["main"]["boundary_signals"]["cleanup_confirmed"]
        .as_u64()
        .is_some_and(|v| v > 0)
}
pub fn fake(args: &[String]) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = cwd
        .parent()
        .ok_or_else(|| HarnessError::invalid("Fixture root missing."))?;
    record(
        root,
        "invoke",
        json!({"pid":std::process::id(),"cwd":cwd,"argv":args}),
    )?;
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt)?;
    let role = [corpus::A1, corpus::A2, corpus::B]
        .into_iter()
        .find(|role| prompt.starts_with(corpus::prompt(role)))
        .ok_or_else(|| HarnessError::invalid("Only sealed pilot prompts are accepted."))?;
    require(
        args.windows(2)
            .any(|a| a == ["--permission-mode", "acceptEdits"]),
        "Artifact task requires native edit mode.",
    )?;
    require(
        std::env::var("CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS")
            .ok()
            .as_deref()
            == Some("0"),
        "Fixture helper authority changed.",
    )?;
    if role == corpus::A2 {
        require(
            args.windows(2).any(|a| a == ["--resume", "pilot-native-a"]),
            "Wrong inbox native identity.",
        )?;
        let source: V = serde_json::from_slice(&ordinary(&root.join("source.json"), 4096)?)?;
        let db = Connection::open_with_flags(
            root.join("state/sessions.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        let (status, raw): (String, String) = db.query_row(
            "SELECT status,receipt FROM launches WHERE id=?1",
            [source["generation"].as_str().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        require(
            status == "completed" && cleanup(&serde_json::from_str::<V>(&raw)?),
            "Inbox entered before source cleanup.",
        )?;
    } else {
        require(
            !args.iter().any(|a| a == "--resume"),
            "Fresh task resumed another task.",
        )?;
    }
    record(
        root,
        "started",
        json!({"role":role,"prompt_sha256":digest(prompt.as_bytes()),"native_session_id":if role==corpus::B{"pilot-native-b"}else{"pilot-native-a"}}),
    )?;
    let native = if role == corpus::B {
        "pilot-native-b"
    } else {
        "pilot-native-a"
    };
    println!(
        "{}",
        json!({"type":"system","subtype":"init","session_id":native})
    );
    std::io::stdout().flush()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !root.join("release").exists() {
        require(Instant::now() < deadline, "Fixture release deadline.")?;
        std::thread::sleep(Duration::from_millis(5));
    }
    let config: V = serde_json::from_slice(&ordinary(&root.join("fixture.json"), 4096)?)?;
    let policy = config["policy"].as_str().unwrap_or("");
    require(
        corpus::POLICIES.contains(&policy),
        "Fixture policy changed.",
    )?;
    if policy == "deadline" && role == corpus::A1 {
        loop {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let source = match (policy, role) {
        ("no-op", _) => None,
        ("wrong-b", corpus::B) => Some(corpus::B_BASE.to_owned()),
        ("exit-zero", corpus::B) => Some(
            "pub fn parse_duration(_: &str) -> Result<u64,String> { std::process::exit(0) }"
                .to_owned(),
        ),
        ("partial", corpus::A2) => Some(corpus::partial_a()),
        (_, corpus::A1) => Some(corpus::partial_a()),
        (_, corpus::A2) => Some(corpus::A_GOLD.to_owned()),
        (_, corpus::B) => Some(corpus::B_GOLD.to_owned()),
        _ => unreachable!(),
    };
    if let Some(source) = source {
        fs::write(cwd.join("source.rs"), source)?;
    }
    if policy == "forbidden" && role == corpus::B {
        fs::write(cwd.join("README.md"), "forbidden change")?;
    }
    // Provider success deliberately occurs for wrong/no-op controls too.
    println!(
        "{}",
        json!({"type":"result","subtype":"success","is_error":false,"session_id":native,"result":"Fixture reports success; external artifact oracle decides correctness."})
    );
    std::io::stdout().flush()?;
    record(root, "leaving", json!({"role":role}))?;
    Ok(())
}
struct Unused;
impl BlackboardFactory for Unused {
    fn for_session(&self, _: &V) -> Result<Arc<dyn fleet::Blackboard>> {
        Err(HarnessError::invalid("No Brain in pilot."))
    }
}
impl SpecialWorkflow for Unused {
    fn run<'a>(&'a self, _: V, _: String, _: String, _: Arc<AtomicBool>, _: Event) -> RunReply<'a> {
        Box::pin(async { Err(HarnessError::invalid("No special workflows in pilot.")) })
    }
}
struct Fixture {
    store: SessionStore,
    sessions: Arc<Sessions>,
    projects: [V; 2],
    roots: [PathBuf; 2],
    seconds: u64,
}
impl Fixture {
    async fn new(root: &Path, concurrency: u32, seconds: u64) -> Result<Self> {
        let roots = [root.join("project-a"), root.join("project-b")];
        let source = root.join("source");
        let hooks = root.join("no-hooks");
        fs::create_dir(&source)?;
        fs::create_dir(&hooks)?;
        for (index, folder) in roots.iter().enumerate() {
            fs::create_dir(folder)?;
            let git = infra_harness::projects::git(
                folder,
                &[
                    "-c".into(),
                    format!("core.hooksPath={}", hooks.display()),
                    "init".into(),
                    "--quiet".into(),
                ],
                10,
            )?;
            require(git.code == 0, "Pilot Git initialization failed.")?;
            fs::write(
                folder.join("source.rs"),
                if index == 0 {
                    corpus::A_BASE
                } else {
                    corpus::B_BASE
                },
            )?;
            fs::write(folder.join("README.md"), corpus::README)?;
        }
        let store = SessionStore::open(&root.join("state"))?;
        let projects = [store.register(&roots[0])?, store.register(&roots[1])?];
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::from([(
                "claude".into(),
                Provider {
                    id: "claude".into(),
                    name: "Offline pilot".into(),
                    available: true,
                    executable: Some(std::env::current_exe()?),
                    detail: "No network/account/model".into(),
                },
            )]))),
        });
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
                seconds,
                concurrency: concurrency as usize,
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
        Ok(Self {
            store,
            sessions,
            projects,
            roots,
            seconds,
        })
    }
    async fn create(&self, index: usize, role: &str) -> Result<String> {
        let reply=payload(self.sessions.execute("sessions.create",json!({"body":{"project_id":self.projects[index]["id"],"provider":"claude","workflow":"native","mode":"edit","project_context":false,"agents_enabled":false,"agent_count":1,"budgets":{"seconds":self.seconds},"prompt":corpus::prompt(role)}})).await?)?;
        reply["session"]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| HarnessError::invalid("Pilot session missing."))
    }
    fn inbox(&self, sid: &str) -> Result<V> {
        let (status, delivery, source): (String, Option<String>, String) = self.store.connection()?.query_row(
            "SELECT status,delivery_generation,source_generation FROM session_inbox WHERE session_id=?1 AND request_id='pilot-a2'",
            [sid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(json!({"status":status,"delivery_generation":delivery,"source_generation":source}))
    }
}
async fn wait(deadline: Instant, mut predicate: impl FnMut() -> Result<bool>) -> Result<()> {
    while !predicate()? {
        if Instant::now() >= deadline {
            return Err(HarnessError {
                status: 408,
                code: "pilot-deadline",
                message: "Pilot native execution deadline reached.".into(),
            });
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}
fn terminal_failure(session: &V) -> Result<()> {
    if matches!(
        session["status"].as_str(),
        Some("failed" | "cancelled" | "interrupted" | "awaiting_context")
    ) {
        return Err(HarnessError {
            status: 422,
            code: if session["usage"]["limit_reached"].is_string() {
                "pilot-budget-exhausted"
            } else {
                "pilot-native-failure"
            },
            message: "Native task ended without successful completion.".into(),
        });
    }
    Ok(())
}
fn launches(store: &SessionStore) -> Result<Vec<V>> {
    let db = store.connection()?;
    let mut stmt =
        db.prepare("SELECT id,session_id,status,settings,receipt FROM launches ORDER BY rowid")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(|(id,sid,status,settings,receipt)|Ok(json!({"id":id,"session_id":sid,"status":status,"settings":serde_json::from_str::<V>(&settings)?,"receipt":serde_json::from_str::<V>(&receipt)?}))).collect()
}
pub async fn run(
    root: &Path,
    concurrency: u32,
    policy: &str,
    tools: &V,
    settings: &V,
) -> Result<V> {
    fs::create_dir(root)?;
    let db = Connection::open(root.join("attempts.sqlite3"))?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL;CREATE TABLE trace(phase TEXT NOT NULL,data TEXT NOT NULL);",
    )?;
    drop(db);
    write_json(&root.join("fixture.json"), &json!({"policy":policy}))?;
    let setup = Instant::now();
    let fixture = Fixture::new(
        root,
        concurrency,
        settings["turn_seconds"].as_u64().unwrap(),
    )
    .await?;
    let setup_seconds = setup.elapsed().as_secs_f64();
    let start = Instant::now();
    let deadline = start
        + Duration::from_secs(
            settings["native_execution_deadline_seconds"]
                .as_u64()
                .unwrap(),
        );
    let execution:Result<V>=async {
        let a_requested=start.elapsed().as_secs_f64();let a=fixture.create(0,corpus::A1).await?;
        let a1=fixture.store.generation(&a)?;
        wait(deadline,||{let source=fixture.store.get(&a)?;terminal_failure(&source)?;Ok(source["native_session_id"]=="pilot-native-a")}).await?;
        write_json(&root.join("source.json"),&json!({"generation":a1}))?;
        let authority=fixture.store.get(&a)?;
        let b_requested=start.elapsed().as_secs_f64();let b=fixture.create(1,corpus::B).await?;
        let inbox_requested=start.elapsed().as_secs_f64();
        let accepted=payload(fixture.sessions.execute("sessions.inbox_accept",json!({"params":{"session_id":a},"body":{"request_id":"pilot-a2","prompt":corpus::prompt(corpus::A2)}})).await?)?;
        require(accepted["status"]=="pending","Pilot inbox was not pending.")?;
        fs::write(root.join("release"),b"ready")?;
        wait(deadline,||{
            let source=fixture.store.get(&a)?;let neighbor=fixture.store.get(&b)?;
            terminal_failure(&source)?;terminal_failure(&neighbor)?;
            let inbox=fixture.inbox(&a)?;
            require(!matches!(inbox["status"].as_str(),Some("blocked"|"cancelled"|"uncertain")),"Inbox handoff was not completed.")?;
            Ok(source["status"]=="completed" && neighbor["status"]=="completed" && inbox["status"]=="completed" && !fixture.sessions.has_active()?)
        }).await?;
        let after=fixture.store.get(&a)?;
        for field in ["provider","native_session_id","project_id","project_path","workspace","branch","git_common_dir","workflow","mode","model","thinking_effort","model_routing","agents_enabled","agent_count","project_context","budgets","budget_revision","brain","sdd","fleet","clash"] {
            require(authority[field]==after[field],"Pilot continuation changed authority.")?;
        }
        Ok(json!({"a_session":a,"b_session":b,"a_source_generation":a1,"inbox":fixture.inbox(&a)?,"requests_at_seconds":{"a_chain":a_requested,"b":b_requested,"a2":inbox_requested},"setup_seconds":setup_seconds,"native_drain_observed_seconds":start.elapsed().as_secs_f64()}))
    }.await;
    let _ = fs::write(root.join("release"), b"ready");
    let closing = fixture.sessions.close();
    let drained = fixture.sessions.wait_closed().await;
    closing?;
    drained?;
    let attempts = traces(root)?;
    let launches = launches(&fixture.store)?;
    let mut observed = match execution {
        Ok(value) => value,
        Err(error) => {
            let proved = !launches.is_empty() && launches.iter().all(|l| cleanup(&l["receipt"]));
            let budget_observed = launches.iter().any(|l| {
                ["time_budget", "token_budget", "usd_budget"]
                    .iter()
                    .any(|signal| {
                        l["receipt"]["full_cost"]["native_diagnostics"]["main"]["boundary_signals"]
                            [signal]
                            .as_u64()
                            .is_some_and(|n| n > 0)
                    })
            });
            write_json(
                &root.join("failure.json"),
                &json!({"error_code":error.code,"error":error.message,"cleanup_proved":proved,"attempts":attempts,"launches":launches}),
            )?;
            if proved
                && (budget_observed
                    || matches!(error.code, "pilot-deadline" | "pilot-budget-exhausted"))
            {
                return Err(HarnessError {
                    status: 408,
                    code: "pilot-deadline-proved",
                    message: error.message,
                });
            }
            return Err(error);
        }
    };
    require(
        attempts.iter().filter(|r| r["phase"] == "invoke").count() == 3 && launches.len() == 3,
        "Missing or duplicate provider attempts.",
    )?;
    require(
        launches
            .iter()
            .all(|l| l["status"] == "completed" && cleanup(&l["receipt"])),
        "Native cleanup or completion unproved.",
    )?;
    for role in [corpus::A1, corpus::A2, corpus::B] {
        require(
            attempts
                .iter()
                .filter(|r| r["phase"] == "started" && r["data"]["role"] == role)
                .count()
                == 1,
            "Pilot role delivery is not unique.",
        )?;
    }
    require(
        observed["inbox"]["delivery_generation"] != observed["a_source_generation"],
        "Inbox reused launch generation.",
    )?;
    observed["attempts"] = json!(attempts);
    observed["launches"] = json!(launches);
    // Grade only after all native custody is drained. A-chain and B form one
    // stream observation; the predeclared verifier order is identical in arms.
    let mut outcomes = json!({});
    for (index, kind) in ["a", "b"].iter().enumerate() {
        let verified = oracle::verify(
            &fixture.roots[index],
            &root.join(format!("oracle-{kind}")),
            kind,
            tools,
        )
        .await;
        let requested = observed["requests_at_seconds"][if index == 0 { "a_chain" } else { "b" }]
            .as_f64()
            .unwrap();
        outcomes[*kind] = json!({"verification":verified,"request_to_verification_seconds":start.elapsed().as_secs_f64()-requested,"endpoint":"after shared native drain and ordered external artifact verification"});
    }
    observed["correct"] = if outcomes["a"]["verification"]["correct"] == false
        || outcomes["b"]["verification"]["correct"] == false
    {
        false.into()
    } else if outcomes["a"]["verification"]["correct"] == true
        && outcomes["b"]["verification"]["correct"] == true
    {
        true.into()
    } else {
        V::Null
    };
    observed["outcomes"] = outcomes;
    observed["stream_seconds"] = start.elapsed().as_secs_f64().into();
    observed["timing_contract"]="fixture timings include polling, process cleanup and ordered batch verification; no model latency or benefit inference".into();
    observed["total_provider_tokens"] = V::Null;
    observed["total_usd"] = V::Null;
    write_json(&root.join("observation.json"), &observed)?;
    Ok(observed)
}
