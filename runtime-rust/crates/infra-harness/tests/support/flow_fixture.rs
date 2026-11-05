//! Offline qualification of actual scheduler, inbox, native processes and cleanup.
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
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value as V, json};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions as Files},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, RwLock, atomic::AtomicBool},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const A1: &str = "FLOW_A_SOURCE";
const A2: &str = "FLOW_A_NEXT";
const B: &str = "FLOW_B_INDEPENDENT";
fn require(valid: bool, reason: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(HarnessError::invalid(reason))
    }
}
fn json_file(file: &Path) -> Result<V> {
    Ok(serde_json::from_slice(&fs::read(file)?)?)
}
fn trace(root: &Path, phase: &str, value: V) -> Result<i64> {
    let db = Connection::open_with_flags(
        root.join("observations.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HarnessError::invalid("Fixture clock precedes epoch."))?
        .as_nanos()
        .to_string();
    db.execute(
        "INSERT INTO observations(phase,data,at_ns) VALUES(?1,?2,?3)",
        params![phase, value.to_string(), stamp],
    )?;
    Ok(db.last_insert_rowid())
}
fn observations(root: &Path) -> Result<Vec<V>> {
    let db = Connection::open_with_flags(
        root.join("observations.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    let mut stmt = db.prepare("SELECT id,phase,data,at_ns FROM observations ORDER BY id")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(id, phase, data, at)| {
            Ok(json!({"id":id,"phase":phase,"data":serde_json::from_str::<V>(&data)?,"at_ns":at}))
        })
        .collect()
}
fn launch(store: &SessionStore, generation: &str) -> Result<V> {
    let db = store.connection()?;
    let row = db
        .query_row(
            "SELECT status,receipt FROM launches WHERE id=?1",
            [generation],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    match row {
        None => Ok(V::Null),
        Some((status, receipt)) => Ok(
            json!({"id":generation,"status":status,"receipt":receipt.map(|raw|serde_json::from_str::<V>(&raw)).transpose()?}),
        ),
    }
}
fn cleanup(launch: &V) -> bool {
    launch["receipt"]["full_cost"]["native_diagnostics"]["main"]["boundary_signals"]["cleanup_confirmed"].as_u64().is_some_and(|count|count>0)
}
fn source_snapshot(root: &Path) -> Result<V> {
    let file = root.join("source.json");
    if !file.exists() {
        return Ok(V::Null);
    }
    let source = json_file(&file)?;
    let db = Connection::open_with_flags(
        root.join("state/sessions.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    let (status, raw): (String, Option<String>) = db.query_row(
        "SELECT status,receipt FROM launches WHERE id=?1",
        [source["generation"].as_str().unwrap_or("")],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let receipt = raw.map(|raw| serde_json::from_str::<V>(&raw)).transpose()?;
    Ok(json!({"launch_status":status,"cleanup_confirmed":cleanup(&json!({"receipt":receipt}))}))
}
fn provider_binding(root: &Path, role: &str, scope: &Path) -> Result<V> {
    let db = Connection::open_with_flags(
        root.join("state/sessions.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    let tx = db.unchecked_transaction()?;
    let (sid, generation): (String, String) = tx.query_row(
        "SELECT s.id,g.generation FROM sessions s JOIN harness_generations g ON g.session_id=s.id WHERE s.title=?1",
        [if role == A2 { A1 } else { role }], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let peers = {
        let mut stmt = tx.prepare("SELECT l.id FROM launches l JOIN sessions s ON s.id=l.session_id WHERE l.status='running' AND l.id!=?1 AND s.git_common_dir=?2 ORDER BY l.rowid")?;
        stmt.query_map(params![generation, scope.to_string_lossy()], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let all_peers = {
        let mut stmt =
            tx.prepare("SELECT id FROM launches WHERE status='running' AND id!=?1 ORDER BY rowid")?;
        stmt.query_map([&generation], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    tx.commit()?;
    Ok(
        json!({"session_id":sid,"generation":generation,"other_running_in_scope":peers,"other_running":all_peers}),
    )
}
fn common(cwd: &Path) -> Result<PathBuf> {
    Ok(infra_core::git::GitFrame::discover(cwd)?
        .ok_or_else(|| HarnessError::invalid("Fixture Git scope missing."))?
        .common_dir()?)
}
fn scope_lock(cwd: &Path) -> Result<File> {
    let file = Files::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(common(cwd)?.join("flow-provider.lock"))?;
    Ok(file)
}
fn held(cwd: &Path) -> Result<bool> {
    match scope_lock(cwd)?.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(error) => Err(HarnessError::unavailable(format!(
            "Fixture lock probe: {error}"
        ))),
    }
}
fn release(root: &Path, role: &str) -> Result<()> {
    for phase in ["emit", "exit"] {
        fs::write(root.join(format!("{phase}-{role}")), b"ready")?;
    }
    Ok(())
}
fn child_gate(root: &Path, name: &str, deadline: Instant) -> Result<()> {
    while !root.join(name).exists() {
        require(Instant::now() < deadline, "Fake provider gate deadline.")?;
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
fn fake(arguments: &[String]) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = cwd
        .parent()
        .ok_or_else(|| HarnessError::invalid("Fixture root missing."))?;
    // Every attempt is recorded before input or scope validation can reject it.
    let invocation = trace(
        root,
        "invoke",
        json!({"pid":std::process::id(),"cwd":cwd,"argv":arguments}),
    )?;
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt)?;
    let role = [A1, A2, B]
        .into_iter()
        .find(|role| prompt.starts_with(role))
        .ok_or_else(|| HarnessError::invalid("Unrecognized fixture prompt."))?;
    let scope = common(&cwd)?;
    let guard = scope_lock(&cwd)?;
    if let Err(error) = guard.try_lock() {
        trace(
            root,
            "scope-conflict",
            json!({"role":role,"error":error.to_string()}),
        )?;
        return Err(HarnessError::conflict(
            "Concurrent entry to the same Git common directory.",
        ));
    }
    trace(
        root,
        "started",
        json!({"invocation":invocation,"role":role,"pid":std::process::id(),"cwd":cwd,"common_dir":scope,"argv":arguments,"prompt":prompt,"helper_limit":std::env::var("CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS").ok(),"source":if role==A1 {V::Null}else{source_snapshot(root)?},"binding":provider_binding(root,role,&scope)?}),
    )?;
    let native = if role == B { "native-b" } else { "native-a" };
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{}",
        json!({"type":"system","subtype":"init","session_id":native})
    )?;
    out.flush()?;
    let deadline = Instant::now() + Duration::from_secs(35);
    child_gate(root, &format!("emit-{role}"), deadline)?;
    let wrong = json_file(&root.join("scenario.json"))?["identity_failure"] == true && role == A1;
    let terminal = if wrong { "wrong-native" } else { native };
    writeln!(
        out,
        "{}",
        json!({"type":"result","subtype":"success","is_error":false,"session_id":terminal,"result":format!("{role} complete")})
    )?;
    out.flush()?;
    trace(
        root,
        "terminal-written",
        json!({"role":role,"native_id":terminal}),
    )?;
    child_gate(root, &format!("exit-{role}"), deadline)?;
    trace(root, "leaving", json!({"role":role}))?;
    Ok(())
}
fn started(root: &Path, role: &str) -> Result<Option<V>> {
    Ok(observations(root)?
        .into_iter()
        .find(|event| event["phase"] == "started" && event["data"]["role"] == role)
        .map(|event| event["data"].clone()))
}
async fn wait(
    deadline: Instant,
    label: &str,
    mut predicate: impl FnMut() -> Result<bool>,
) -> Result<()> {
    while !predicate()? {
        require(
            Instant::now() < deadline,
            &format!("Qualification deadline: {label}"),
        )?;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}
fn payload(value: Payload) -> Result<V> {
    match value {
        Payload::Json(value) => Ok(value),
        _ => Err(HarnessError::invalid("Expected fixture JSON.")),
    }
}
struct Unused;
impl BlackboardFactory for Unused {
    fn for_session(&self, _: &V) -> Result<Arc<dyn fleet::Blackboard>> {
        panic!("No fixture blackboard")
    }
}
impl SpecialWorkflow for Unused {
    fn run<'a>(&'a self, _: V, _: String, _: String, _: Arc<AtomicBool>, _: Event) -> RunReply<'a> {
        panic!("No specialized fixture workflow")
    }
}
struct Fixture {
    store: SessionStore,
    sessions: Arc<Sessions>,
    projects: [V; 2],
    roots: [PathBuf; 2],
}
fn git(root: &Path, args: &[&str], hooks: &Path) -> Result<()> {
    let mut options = vec![
        "-c".to_owned(),
        format!("core.hooksPath={}", hooks.display()),
    ];
    options.extend(args.iter().map(|v| v.to_string()));
    require(
        infra_harness::projects::git(root, &options, 10)?.code == 0,
        "Fixture Git command failed.",
    )
}
impl Fixture {
    async fn new(root: &Path, scenario: &str) -> Result<Self> {
        let a = root.join("project-a");
        let b = root.join("project-b");
        let source = root.join("source");
        let hooks = root.join("no-hooks");
        for folder in [&a, &source, &hooks] {
            fs::create_dir(folder)?;
        }
        git(&a, &["init", "--quiet"], &hooks)?;
        if scenario == "shared" {
            git(
                &a,
                &[
                    "-c",
                    "user.name=Offline Fixture",
                    "-c",
                    "user.email=offline@fixture.invalid",
                    "-c",
                    "commit.gpgSign=false",
                    "commit",
                    "--quiet",
                    "--allow-empty",
                    "--no-verify",
                    "-m",
                    "Fixture scope",
                ],
                &hooks,
            )?;
            git(
                &a,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "--detach",
                    b.to_str()
                        .ok_or_else(|| HarnessError::invalid("Fixture path is not UTF-8."))?,
                ],
                &hooks,
            )?;
        } else {
            fs::create_dir(&b)?;
            git(&b, &["init", "--quiet"], &hooks)?;
        }
        require(
            (common(&a)? == common(&b)?) == (scenario == "shared"),
            "Git common-directory negative control failed.",
        )?;
        let store = SessionStore::open(&root.join("state"))?;
        let projects = [store.register(&a)?, store.register(&b)?];
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::from([(
                "claude".into(),
                Provider {
                    id: "claude".into(),
                    name: "Offline qualification".into(),
                    available: true,
                    executable: Some(std::env::current_exe()?),
                    detail: "No account/network/model".into(),
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
                seconds: 30,
                concurrency: if scenario == "serial" { 1 } else { 2 },
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
            roots: [a, b],
        })
    }
    async fn create(&self, index: usize, prompt: &str) -> Result<String> {
        let value=payload(self.sessions.execute("sessions.create",json!({"body":{"project_id":self.projects[index]["id"],"provider":"claude","workflow":"native","mode":"plan","project_context":false,"agents_enabled":false,"agent_count":1,"budgets":{"seconds":30},"prompt":prompt}})).await?)?;
        value["session"]["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| HarnessError::invalid("Fixture session missing."))
    }
    async fn inbox(&self, sid: &str, prompt: &str) -> Result<V> {
        payload(self.sessions.execute("sessions.inbox_accept",json!({"params":{"session_id":sid},"body":{"request_id":"next-a","prompt":prompt}})).await?)
    }
    fn receipt(&self, sid: &str) -> Result<V> {
        let raw:String=self.store.connection()?.query_row("SELECT status || char(9) || COALESCE(delivery_generation,'') FROM session_inbox WHERE session_id=?1 AND request_id='next-a'",[sid],|r|r.get(0))?;
        let (status, generation) = raw
            .split_once('\t')
            .ok_or_else(|| HarnessError::invalid("Malformed fixture receipt."))?;
        Ok(
            json!({"status":status,"delivery_generation":if generation.is_empty(){V::Null}else{json!(generation)}}),
        )
    }
}
fn same_authority(before: &V, after: &V) -> bool {
    [
        "provider",
        "native_session_id",
        "project_id",
        "project_path",
        "workspace",
        "branch",
        "git_common_dir",
        "workflow",
        "mode",
        "model",
        "thinking_effort",
        "model_routing",
        "agents_enabled",
        "agent_count",
        "project_context",
        "budgets",
        "budget_revision",
        "brain",
        "sdd",
        "fleet",
        "clash",
        "creator",
        "system_run",
        "system_discovery",
    ]
    .iter()
    .all(|field| before[*field] == after[*field])
}
fn process_contract(start: &V, role: &str, cwd: &Path, serial: bool) -> Result<()> {
    if serial {
        require(
            start["binding"]["other_running"]
                .as_array()
                .is_some_and(Vec::is_empty),
            "Serial provider entered before another launch finished cleanup.",
        )?;
    }
    require(
        start["binding"]["other_running_in_scope"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "Provider entered before another launch in its Git scope finished cleanup.",
    )?;
    require(
        start["cwd"] == cwd.to_string_lossy().as_ref(),
        "Provider entered a different cwd.",
    )?;
    let argv = start["argv"]
        .as_array()
        .ok_or_else(|| HarnessError::invalid("Provider argv missing."))?;
    require(
        argv.windows(2)
            .any(|pair| pair == [json!("--permission-mode"), json!("plan")]),
        "Provider mode changed.",
    )?;
    require(
        argv.windows(4).any(|pair| {
            pair == [
                json!("--disallowedTools"),
                json!("Agent"),
                json!("Task"),
                json!("Workflow"),
            ]
        }) && start["helper_limit"] == "0",
        "Provider helper authority changed.",
    )?;
    let prompt = start["prompt"].as_str().unwrap_or("");
    require(
        prompt.matches(role).count() == 1,
        "Fixture message missing or duplicated.",
    )?;
    require(
        if role == B {
            ![A1, A2].iter().any(|other| prompt.contains(other))
        } else {
            !prompt.contains(B)
        },
        "Another task leaked into the prompt.",
    )?;
    if role == A2 {
        require(
            argv.windows(2)
                .any(|pair| pair == [json!("--resume"), json!("native-a")]),
            "Follow-up resumed a different native session.",
        )?;
        require(
            start["source"]["launch_status"] == "completed"
                && start["source"]["cleanup_confirmed"] == true,
            "Follow-up entered before proved source cleanup.",
        )?;
    } else {
        require(
            !argv.iter().any(|arg| arg == "--resume"),
            "Fresh task unexpectedly resumed another session.",
        )?;
    }
    Ok(())
}

async fn scenario(root: &Path, name: &str) -> Result<V> {
    require(
        [
            "serial",
            "parallel",
            "shared",
            "cancel-source",
            "cancel-next",
            "identity-failure",
        ]
        .contains(&name),
        "Unknown qualification scenario.",
    )?;
    let root = infra_core::fs::canonicalize(root)?;
    let db = Connection::open(root.join("observations.sqlite3"))?;
    db.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE observations(id INTEGER PRIMARY KEY AUTOINCREMENT,phase TEXT NOT NULL,data TEXT NOT NULL,at_ns TEXT NOT NULL);")?;
    drop(db);
    fs::write(
        root.join("scenario.json"),
        json!({"name":name,"identity_failure":name=="identity-failure"}).to_string(),
    )?;
    let fixture = Fixture::new(&root, name).await?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let outcome:Result<V>=async {
        let a=fixture.create(0,A1).await?;
        let a1=fixture.store.generation(&a)?;
        wait(deadline,"source ready",||Ok(fixture.store.get(&a)?["native_session_id"]=="native-a" && started(&root,A1)?.is_some())).await?;
        // A1 never reads this descriptor; publish it once before enqueueing its readers.
        fs::write(root.join("source.json"),json!({"sid":a,"generation":a1}).to_string())?;
        let frozen=fixture.store.get(&a)?;
        let b=fixture.create(1,B).await?;
        let b1=fixture.store.generation(&b)?;
        let accepted=fixture.inbox(&a,A2).await?;
        require(accepted["status"]=="pending" && fixture.inbox(&a,A2).await?["id"]==accepted["id"],"Admission replay did not return one pending receipt.")?;
        let parallel=!["serial","shared"].contains(&name);
        if parallel {
            wait(deadline,"independent overlap",||Ok(started(&root,B)?.is_some() && fixture.store.get(&b)?["native_session_id"]=="native-b")).await?;
            require(held(&fixture.roots[0])? && held(&fixture.roots[1])?,"Independent provider scopes were not held simultaneously.")?;
            require(launch(&fixture.store,&a1)?["status"]=="running" && launch(&fixture.store,&b1)?["status"]=="running","Overlap lacks two running launches.")?;
            trace(&root,"overlap-proved",json!({"a_generation":a1,"b_generation":b1}))?;
        } else {release(&root,B)?;}
        if name=="cancel-source" {
            fixture.sessions.cancel(&a)?;
            wait(deadline,"source cancellation cleanup",||Ok(launch(&fixture.store,&a1)?["status"]=="cancelled")).await?;
            require(fixture.receipt(&a)?["status"]=="cancelled" && fixture.receipt(&a)?["delivery_generation"].is_null(),"Cancelled source assigned an inbox turn.")?;
        } else {
            fs::write(root.join(format!("emit-{A1}")),b"ready")?;
            if name=="identity-failure" {
                wait(deadline,"source identity rejection",||Ok(launch(&fixture.store,&a1)?["status"]=="failed" && fixture.receipt(&a)?["status"]=="blocked")).await?;
                require(fixture.receipt(&a)?["status"]=="blocked","Wrong native identity did not block continuation.")?;
            } else {
                wait(deadline,"Harness observed terminal before exit",||Ok(fixture.store.events(&a,0)?.iter().any(|event|event["launch_id"]==a1 && event["kind"]=="result"))).await?;
                require(held(&fixture.roots[0])? && launch(&fixture.store,&a1)?["status"]=="running","Terminal event was treated as completed process cleanup.")?;
                require(fixture.receipt(&a)?["status"]=="pending" && started(&root,A2)?.is_none(),"Follow-up ran while source still owned its process.")?;
                trace(&root,"terminal-without-cleanup",json!({"source_generation":a1,"inbox":fixture.receipt(&a)?}))?;
                fs::write(root.join(format!("exit-{A1}")),b"ready")?;
                wait(deadline,"follow-up start",||Ok(started(&root,A2)?.is_some())).await?;
                let a2=fixture.receipt(&a)?["delivery_generation"].as_str().ok_or_else(||HarnessError::invalid("Missing delivery generation."))?.to_string();
                require(a2!=a1 && cleanup(&launch(&fixture.store,&a1)?),"Follow-up lacks independent generation/source cleanup proof.")?;
                if name=="cancel-next" {
                    fixture.sessions.cancel(&a)?;
                    wait(deadline,"assigned cancellation cleanup",||Ok(launch(&fixture.store,&a2)?["status"]=="cancelled")).await?;
                    require(fixture.receipt(&a)?["status"]=="uncertain","Assigned cancellation lost delivery uncertainty.")?;
                } else {release(&root,A2)?;}
            }
        }
        if parallel {
            require(held(&fixture.roots[1])? && fixture.store.generation(&b)?==b1 && fixture.store.get(&b)?["native_session_id"]=="native-b" && launch(&fixture.store,&b1)?["status"]=="running","A lifecycle change cancelled or replaced its independent neighbor.")?;
            release(&root,B)?;
        }
        let final_a=if ["cancel-source","cancel-next"].contains(&name){"cancelled"}else if name=="identity-failure"{"failed"}else{"completed"};
        wait(deadline,"terminal sessions",||Ok(fixture.store.get(&a)?["status"]==final_a && fixture.store.get(&b)?["status"]=="completed" && !fixture.sessions.has_active()?)).await?;
        let expected=if name=="cancel-source"{"cancelled"}else if name=="cancel-next"{"uncertain"}else if name=="identity-failure"{"blocked"}else{"completed"};
        let replay=fixture.inbox(&a,A2).await?;
        require(replay["id"]==accepted["id"] && replay["status"]==expected,"Terminal replay changed the request.")?;
        let conflict=fixture.inbox(&a,"different body").await;
        require(matches!(conflict,Err(ref error) if error.status==409),"Changed replay body was accepted.")?;
        require(same_authority(&frozen,&fixture.store.get(&a)?),"The continuation changed session authority.")?;
        let events=fixture.store.events(&a,0)?;
        let assigned=if ["cancel-source","identity-failure"].contains(&name){0}else{1};
        require(events.iter().filter(|event|event["inbox_request_id"]=="next-a").count()==assigned,"Duplicate or missing inbox user event.")?;
        let launches={let db=fixture.store.connection()?;let mut stmt=db.prepare("SELECT id,status,receipt FROM launches ORDER BY rowid")?;stmt.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?};
        let launches=launches.into_iter().map(|(id,status,receipt)|Ok(json!({"id":id,"status":status,"receipt":receipt.map(|raw|serde_json::from_str::<V>(&raw)).transpose()?}))).collect::<Result<Vec<_>>>()?;
        require(launches.iter().all(cleanup),"A terminal launch lacks observed native cleanup proof.")?;
        require(SessionStore::open(&root.join("state")).is_err(),"Runner custody was released while owner remained alive.")?;
        Ok(json!({"schema":"native-flow-qualification-v1","task_correctness":"not_measured","latency_benefit":"not_measured","scenario":name,"concurrency":if name=="serial"{1}else{2},"shared_common_dir":name=="shared","source_session_id":a,"neighbor_session_id":b,"source_generation":a1,"b_generation":b1,"inbox":replay,"launches":launches,"source_session_status":fixture.store.get(&a)?["status"],"neighbor_status":fixture.store.get(&b)?["status"]}))
    }.await;
    for role in [A1, A2, B] {
        let _ = release(&root, role);
    }
    let closing = fixture.sessions.close();
    let drained = fixture.sessions.wait_closed().await;
    closing?;
    drained?;
    let mut outcome = match outcome {
        Ok(value) => value,
        Err(error) => {
            eprintln!("trace: {}", json!(observations(&root)?));
            return Err(error);
        }
    };
    let events = observations(&root)?;
    let starts = events
        .iter()
        .filter(|event| event["phase"] == "started")
        .map(|event| event["data"].clone())
        .collect::<Vec<_>>();
    let expected = if ["cancel-source", "identity-failure"].contains(&name) {
        2
    } else {
        3
    };
    require(
        events
            .iter()
            .filter(|event| event["phase"] == "invoke")
            .count()
            == expected
            && starts.len() == expected,
        "Actual provider attempt count changed after drain.",
    )?;
    require(
        !events
            .iter()
            .any(|event| event["phase"] == "scope-conflict"),
        "Overlapping provider entry to a common Git scope.",
    )?;
    for role in [A1, B, A2] {
        let items = starts
            .iter()
            .filter(|entry| entry["role"] == role)
            .collect::<Vec<_>>();
        require(
            items.len() == if role == A2 && expected == 2 { 0 } else { 1 },
            "Duplicate or missing role invocation.",
        )?;
        for entry in items {
            process_contract(
                entry,
                role,
                &fixture.roots[usize::from(role == B)],
                name == "serial",
            )?;
            require(
                entry["binding"]["session_id"]
                    == outcome[if role == B {
                        "neighbor_session_id"
                    } else {
                        "source_session_id"
                    }],
                "Observed process belongs to a different Harness session.",
            )?;
            let generation = if role == A1 {
                &outcome["source_generation"]
            } else if role == B {
                &outcome["b_generation"]
            } else {
                &outcome["inbox"]["delivery_generation"]
            };
            require(
                entry["binding"]["generation"] == *generation,
                "Observed process belongs to a different launch generation.",
            )?;
        }
    }
    let bstart = starts.iter().find(|entry| entry["role"] == B).unwrap();
    require(
        bstart["source"]["launch_status"]
            == if ["serial", "shared"].contains(&name) {
                "completed"
            } else {
                "running"
            },
        "Observed overlap differs from the requested scheduling policy.",
    )?;
    outcome["provider_attempts"] = expected.into();
    outcome["observations"] = json!(events);
    drop(fixture);
    let reopened = SessionStore::open(&root.join("state"))?;
    require(
        reopened.connection()?.query_row(
            "SELECT count(*) FROM session_inbox WHERE status IN ('pending','assigned')",
            [],
            |row| row.get::<_, i64>(0),
        )? == 0,
        "Reopen fabricated active delivery.",
    )?;
    outcome["runner_reopened"] = true.into();
    fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(&outcome)?,
    )?;
    Ok(outcome)
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "internal-guard") {
        std::process::exit(infra_host::process::guard_entry(&args[1..]).unwrap_or(125));
    }
    if args.first().is_some_and(|arg| arg == "--print") {
        if let Err(error) = fake(&args) {
            eprintln!("{}", error.message);
            std::process::exit(2);
        }
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .unwrap();
    match runtime.block_on(scenario(Path::new(&args[0]), &args[1])) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{}", error.message);
            std::process::exit(2);
        }
    }
}
