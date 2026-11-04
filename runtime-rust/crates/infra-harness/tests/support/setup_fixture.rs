//! Noninstalled Setup driver and fake Git; only fixture-owned trees are touched.
use infra_harness::{
    HarnessError, Result,
    accelerators::Accelerators,
    fleet, http,
    knowledge::KnowledgeDomain,
    provider::ProviderDomain,
    services::{CloseFuture, Domain, NativeServices, Payload, Reply, Services},
    sessions::{BlackboardFactory, Event, RunReply, Sessions, SpecialWorkflow},
    setup::{Setup, SetupDomain},
    store::SessionStore,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, RwLock, atomic::AtomicBool},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(15);
const POLICY: &[u8] = b"Client-owned policy.\n";
fn invalid(message: &str) -> HarnessError {
    HarnessError::invalid(message)
}
fn root(raw: &str) -> Result<PathBuf> {
    let selected = Path::new(raw);
    if !selected.is_absolute() || !selected.is_dir() {
        return Err(invalid("Expected an existing absolute fixture directory"));
    }
    Ok(fs::canonicalize(selected)?)
}
fn signal(selected: &Path, name: &str) -> Result<()> {
    fs::write(selected.join(name), b"ready")?;
    Ok(())
}
async fn wait(selected: &Path, name: &str) -> Result<()> {
    let until = Instant::now() + DEADLINE;
    while !selected.join(name).is_file() {
        if Instant::now() >= until {
            return Err(invalid("Setup fixture synchronization deadline"));
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}
async fn within<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(DEADLINE * 2, future)
        .await
        .expect("Setup operation exceeded its fixture deadline")
}
async fn once<F: Future + ?Sized>(mut future: Pin<&mut F>) -> Poll<F::Output> {
    std::future::poll_fn(|context| Poll::Ready(future.as_mut().poll(context))).await
}
fn value(result: Result<Payload>) -> Value {
    match result.expect("Expected a successful real Setup operation") {
        Payload::Json(value) => value,
        Payload::Bytes { .. } => panic!("Setup must return JSON"),
    }
}
fn error(result: Result<Payload>, status: u16, message: &str) {
    let failure = match result {
        Err(failure) => failure,
        Ok(_) => panic!("Expected Setup refusal: {message}"),
    };
    assert_eq!(failure.status, status);
    assert_eq!(failure.message, message);
}

struct NoNativeWork;
impl BlackboardFactory for NoNativeWork {
    fn for_session(&self, _: &Value) -> Result<Arc<dyn fleet::Blackboard>> {
        panic!("Setup fixture must not open a provider or blackboard")
    }
}
impl SpecialWorkflow for NoNativeWork {
    fn run<'a>(
        &'a self,
        _: Value,
        _: String,
        _: String,
        _: Arc<AtomicBool>,
        _: Event,
    ) -> RunReply<'a> {
        panic!("Setup fixture must not launch a workflow")
    }
}
impl Domain for NoNativeWork {
    fn execute<'a>(&'a self, _: &'a str, _: Value) -> Reply<'a> {
        Box::pin(async { Err(invalid("Unexpected unused HTTP fixture domain")) })
    }
}
struct Fixture {
    store: SessionStore,
    sessions: Arc<Sessions>,
    setup: Arc<Setup>,
    domain: Arc<SetupDomain>,
    project: String,
    session: String,
}
impl Fixture {
    fn new(selected: &Path) -> Result<Self> {
        let source = selected.join("source");
        let store = SessionStore::open(&selected.join("state"))?;
        let project = store.register(&selected.join("project"))?;
        let project = project["id"].as_str().unwrap().to_owned();
        let session = store.create(
            &json!({"project_id":project,"provider":"claude","mode":"plan","workflow":"native","project_context":false}),
            "fixture-only stored session",
            &[],
        )?;
        let session = session["id"].as_str().unwrap().to_owned();
        store.patch(&session, &json!({"status":"completed"}))?;
        store.event(&session, &json!({"kind":"status","text":"poll proof"}))?;
        // Constructing an empty registry never discovers or authenticates CLIs.
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::new())),
        });
        let knowledge = Arc::new(KnowledgeDomain::new(store.clone(), source.clone()));
        let special = ["creator", "system_run", "system_discovery"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    Arc::new(NoNativeWork) as Arc<dyn SpecialWorkflow>,
                )
            })
            .collect();
        let sessions = Sessions::new(
            store.clone(),
            source.clone(),
            providers.clone(),
            knowledge,
            Arc::new(NoNativeWork),
            special,
            900,
        )?;
        let accelerators = Arc::new(Accelerators::new(
            store.clone(),
            source.clone(),
            providers,
            Arc::downgrade(&sessions),
        )?);
        let setup = Arc::new(Setup::new(
            store.clone(),
            source,
            Arc::downgrade(&sessions),
            accelerators,
        ));
        let domain = Arc::new(SetupDomain::new(setup.clone()));
        Ok(Self {
            store,
            sessions,
            setup,
            domain,
            project,
            session,
        })
    }
    fn status(&self) -> Value {
        json!({"params":{"project_id":self.project}})
    }
    fn preview(&self) -> Value {
        json!({"body":{"project_id":self.project,"edition":"PHP Core","tools":["codex"]}})
    }
    fn install(&self, id: &str) -> Value {
        json!({"body":{"preview_id":id}})
    }
    fn spawn(
        &self,
        operation: &'static str,
        request: Value,
    ) -> tokio::task::JoinHandle<Result<Payload>> {
        let domain = self.domain.clone();
        tokio::spawn(async move { domain.execute(operation, request).await })
    }
    async fn preview_id(&self) -> String {
        let preview = value(within(self.domain.execute("setup.preview", self.preview())).await);
        assert_eq!(preview["can_install"], true);
        assert_eq!(preview["file_count"], 2);
        preview["preview_id"].as_str().unwrap().to_owned()
    }
    async fn finish(&self) -> Result<()> {
        let requested = self.sessions.close();
        self.domain.close().await;
        let drained = self.sessions.wait_closed().await;
        requested.and(drained)
    }
}

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message))
    }
}
async fn http_get(port: u16, route: &str) -> Result<(u16, Value)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    tokio::time::timeout(DEADLINE * 2, async {
        let mut stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await?;
        let request = format!(
            "GET {route} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        let mut bytes = Vec::new();
        stream.take(65537).read_to_end(&mut bytes).await?;
        require(bytes.len() <= 65536, "HTTP fixture response exceeded its bound")?;
        let boundary = bytes.windows(4).position(|part| part == b"\r\n\r\n")
            .ok_or_else(|| invalid("Incomplete HTTP fixture response headers"))?;
        let headers = std::str::from_utf8(&bytes[..boundary])
            .map_err(|_| invalid("Invalid HTTP fixture response headers"))?;
        let status = headers.lines().next().and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or_else(|| invalid("Invalid HTTP fixture response status"))?;
        let length = headers.lines().skip(1).filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            .ok_or_else(|| invalid("HTTP fixture response omitted Content-Length"))?;
        let body = &bytes[boundary + 4..];
        require(length == body.len(), "Incomplete HTTP fixture response body")?;
        Ok((status, infra_harness::strict_json::parse(body, 65536)?))
    }).await.map_err(|_| invalid("HTTP fixture connection or IO deadline"))?
}
async fn finish_task<T>(
    task: &mut Option<tokio::task::JoinHandle<Result<T>>>,
) -> Result<Option<T>> {
    let Some(mut task) = task.take() else {
        return Ok(None);
    };
    match tokio::time::timeout(DEADLINE * 2, &mut task).await {
        Ok(result) => result
            .map_err(|_| invalid("HTTP fixture task stopped"))?
            .map(Some),
        Err(_) => {
            task.abort();
            let _ = task.await;
            Err(invalid("HTTP fixture task cleanup deadline"))
        }
    }
}
async fn http_responsive(fixture: &Fixture, selected: &Path) -> Result<()> {
    let unused: Arc<dyn Domain> = Arc::new(NoNativeWork);
    let mut domains = [
        "sessions",
        "brain",
        "context",
        "setup",
        "accelerators",
        "systems",
        "discovery",
        "creator",
        "skills",
        "results",
        "delivery",
        "projects",
        "commands",
        "files",
        "memory",
        "updates",
        "providers",
        "knowledge",
    ]
    .into_iter()
    .map(|name| (name.into(), unused.clone()))
    .collect::<BTreeMap<_, _>>();
    domains.insert("setup".into(), fixture.domain.clone());
    let closing = fixture.sessions.clone();
    let setup = fixture.domain.clone();
    let services = Arc::new(NativeServices::new(
        fixture.store.clone(),
        domains,
        Arc::new(|| Ok(json!({"fixture":true}))),
        BTreeMap::new(),
        Arc::new(move || {
            let closing = closing.clone();
            let setup = setup.clone();
            Box::pin(async move {
                let requested = closing.close();
                setup.close().await;
                let drained = closing.wait_closed().await;
                requested.and(drained)
            }) as CloseFuture<'static>
        }),
    )?);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let (shutdown, _shutdown_receiver) = tokio::sync::watch::channel(false);
    let app = http::App {
        services: services.clone(),
        token: "private-setup-fixture-token".into(),
        instance: "private-setup-fixture-instance".into(),
        port,
        source: selected.join("source"),
        shutdown: shutdown.clone(),
        assets: Arc::new(BTreeMap::new()),
    };
    let mut server = Some(tokio::spawn(http::serve_listener(listener, app)));
    let setup_route = format!("/api/projects/{}/setup", fixture.project);
    let mut setup_request = Some(tokio::spawn(
        async move { http_get(port, &setup_route).await },
    ));
    let checked: Result<()> = async {
        wait(selected, "git-entered").await?;
        let session_route = format!("/api/sessions/{}?after=0", fixture.session);
        // This is a second TCP connection through the real router and
        // NativeServices::sessions.get, rather than a direct store assertion.
        let (status, body) = http_get(port, &session_route).await?;
        require(
            status == 200,
            "Independent session HTTP poll was not successful",
        )?;
        require(
            body["session"]["id"] == fixture.session,
            "Session HTTP poll returned a different session",
        )?;
        require(
            body["events"]
                .as_array()
                .is_some_and(|events| events.iter().any(|event| event["text"] == "poll proof")),
            "Session HTTP poll omitted the stored event",
        )?;
        require(
            setup_request
                .as_ref()
                .is_some_and(|request| !request.is_finished()),
            "Setup HTTP response completed before the Git release",
        )?;
        require(
            !selected.join("release-git").exists(),
            "Session HTTP poll completed only after the Git release",
        )?;
        signal(selected, "http-session-polled")?;
        let (status, body) = finish_task(&mut setup_request)
            .await?
            .ok_or_else(|| invalid("Missing setup HTTP request"))?;
        require(
            status == 200,
            "Setup HTTP response was not successful after release",
        )?;
        require(
            body["project_id"] == fixture.project
                && body["available"] == true
                && body["git"]["is_git"] == false,
            "Setup HTTP response did not contain the actual inspected project",
        )?;
        Ok(())
    }
    .await;
    // Run cleanup even if routing, body validation or a bounded client fails.
    // Release the real synchronous call before stopping services or the server.
    let released = signal(selected, "release-git");
    let _ = shutdown.send(true);
    let closed = tokio::time::timeout(DEADLINE * 2, services.close())
        .await
        .map_err(|_| invalid("HTTP fixture services cleanup deadline"))
        .and_then(|result| result);
    let client = finish_task(&mut setup_request).await.map(|_| ());
    let served = finish_task(&mut server).await.map(|_| ());
    checked.and(released).and(closed).and(client).and(served)
}

async fn scenario(selected: &Path, name: &str) -> Result<Value> {
    let fixture = Fixture::new(selected)?;
    match name {
        "http-responsive" => http_responsive(&fixture, selected).await?,
        "responsive" => {
            let status = fixture.spawn("setup.status", fixture.status());
            wait(selected, "git-entered").await?;
            let (_, events) = fixture.store.session_events(&fixture.session, 0)?;
            assert!(events.iter().any(|event| event["text"] == "poll proof"));
            assert!(!status.is_finished());
            // Parent must observe this real SQLite poll before releasing Git.
            signal(selected, "session-polled")?;
            let status = value(within(status).await.unwrap());
            assert_eq!(status["available"], true);
            assert_eq!(status["git"]["is_git"], false);
        }
        "queued" => {
            let active = fixture.spawn("setup.status", fixture.status());
            wait(selected, "git-entered").await?;
            let mut queued = fixture.domain.execute("setup.preview", fixture.preview());
            assert!(once(queued.as_mut()).await.is_pending());
            error(
                fixture
                    .domain
                    .execute("setup.install", json!({"body":{"preview_id":null}}))
                    .await,
                409,
                "Accelerator setup is busy.",
            );
            drop(queued);
            // Dropping a lifecycle waiter must release its mutation reservation
            // without submitting it to the blocking pool or consuming a receipt.
            let mut next = fixture.domain.execute("setup.preview", fixture.preview());
            assert!(once(next.as_mut()).await.is_pending());
            drop(next);
            let mut status = fixture.domain.execute("setup.status", fixture.status());
            assert!(once(status.as_mut()).await.is_pending());
            drop(status);
            let mut close = Box::pin(fixture.domain.close());
            assert!(once(close.as_mut()).await.is_pending());
            signal(selected, "queue-checked")?;
            value(within(active).await.unwrap());
            within(close).await;
            error(
                fixture
                    .domain
                    .execute("setup.status", fixture.status())
                    .await,
                409,
                "Accelerator setup is closed.",
            );
            assert_eq!(git_calls(selected, "project")?, 1);
            assert_eq!(git_calls(selected, "source")?, 0);
        }
        "active-cancel" => {
            let active = fixture.spawn("setup.preview", fixture.preview());
            wait(selected, "git-entered").await?;
            active.abort();
            assert!(matches!(within(active).await, Err(failure) if failure.is_cancelled()));
            // Abort drops the requesting future; the actual installer and both
            // guards must remain alive until its synchronous work completes.
            error(
                fixture
                    .domain
                    .execute("setup.install", json!({"body":{"preview_id":null}}))
                    .await,
                409,
                "Accelerator setup is busy.",
            );
            let mut close = Box::pin(fixture.domain.close());
            assert!(once(close.as_mut()).await.is_pending());
            signal(selected, "close-pending")?;
            within(close).await;
            fixture.domain.close().await;
            error(
                fixture
                    .domain
                    .execute("setup.status", fixture.status())
                    .await,
                409,
                "Accelerator setup is closed.",
            );
            error(
                fixture
                    .domain
                    .execute("setup.preview", fixture.preview())
                    .await,
                409,
                "Accelerator setup is closed.",
            );
        }
        "busy" => {
            let active = fixture.spawn("setup.preview", fixture.preview());
            wait(selected, "git-entered").await?;
            for (body, message) in [
                (json!({}), "Select a project, edition and tools."),
                (
                    json!({"project_id":null,"edition":"PHP Core","tools":["codex"]}),
                    "Select a project.",
                ),
                (
                    json!({"project_id":fixture.project,"edition":"bad","tools":["codex"]}),
                    "Select a supported edition.",
                ),
                (
                    json!({"project_id":fixture.project,"edition":"PHP Core","tools":false}),
                    "Expected a path array.",
                ),
                (
                    json!({"project_id":fixture.project,"edition":"PHP Core","tools":[false]}),
                    "Expected a path string.",
                ),
                (
                    json!({"project_id":fixture.project,"edition":"PHP Core","tools":["codex","codex"]}),
                    "Select distinct supported tools.",
                ),
            ] {
                error(
                    fixture
                        .domain
                        .execute("setup.preview", json!({"body":body}))
                        .await,
                    400,
                    message,
                );
                assert_eq!(fixture.setup.preview(&body).unwrap_err().message, message);
            }
            error(
                fixture
                    .domain
                    .execute(
                        "setup.install",
                        json!({"body":{"preview_id":null,"extra":true}}),
                    )
                    .await,
                400,
                "Install using the reviewed preview ID.",
            );
            error(
                fixture
                    .domain
                    .execute("setup.install", json!({"body":{"preview_id":null}}))
                    .await,
                409,
                "Accelerator setup is busy.",
            );
            error(
                fixture
                    .domain
                    .execute("setup.preview", fixture.preview())
                    .await,
                409,
                "Accelerator setup is busy.",
            );
            assert_eq!(
                fixture
                    .setup
                    .install(&json!({"preview_id":null}))
                    .unwrap_err()
                    .message,
                "Accelerator setup is busy."
            );
            signal(selected, "busy-checked")?;
            let preview = value(within(active).await.unwrap());
            let id = preview["preview_id"].as_str().unwrap();
            error(
                fixture
                    .domain
                    .execute("setup.install", json!({"body":{"preview_id":null}}))
                    .await,
                400,
                "Invalid setup preview.",
            );
            // Invalid admission did not consume the actual reviewed receipt.
            assert_eq!(
                value(
                    fixture
                        .domain
                        .execute("setup.install", fixture.install(id))
                        .await
                )["ok"],
                true
            );
        }
        "fifo" => {
            let active = fixture.spawn("setup.status", fixture.status());
            wait(selected, "git-entered").await?;
            let mut earlier = fixture.domain.execute("setup.status", fixture.status());
            assert!(once(earlier.as_mut()).await.is_pending());
            let mut close = Box::pin(fixture.domain.close());
            assert!(once(close.as_mut()).await.is_pending());
            let mut later = fixture.domain.execute("setup.status", fixture.status());
            assert!(once(later.as_mut()).await.is_pending());
            signal(selected, "fifo-checked")?;
            value(within(active).await.unwrap());
            assert!(once(close.as_mut()).await.is_pending());
            value(within(earlier).await);
            within(close).await;
            error(within(later).await, 409, "Accelerator setup is closed.");
            assert_eq!(git_calls(selected, "project")?, 2);
        }
        "install" => {
            let id = fixture.preview_id().await;
            let installed = value(
                within(
                    fixture
                        .domain
                        .execute("setup.install", fixture.install(&id)),
                )
                .await,
            );
            assert_eq!(installed["ok"], true);
            assert!(
                fixture
                    .store
                    .setting(&format!("setup.{}", fixture.project))?
                    .is_some()
            );
            let policy = fs::read_to_string(selected.join("project/AGENTS.md"))?;
            assert!(policy.starts_with(std::str::from_utf8(POLICY).unwrap()));
            assert!(policy.contains("Fixture managed policy."));
            let engine = selected
                .join("project/memory-bank/bin")
                .join(if cfg!(windows) {
                    "infra-engine.exe"
                } else {
                    "infra-engine"
                });
            assert_eq!(
                infra_core::query::sha256(&fs::read(engine)?),
                infra_core::query::sha256(&fs::read(std::env::current_exe()?)?)
            );
            error(
                fixture
                    .domain
                    .execute("setup.install", fixture.install(&id))
                    .await,
                409,
                "The preview expired or was already used. Preview again.",
            );
            error(
                fixture.domain.execute("setup.unknown", json!({})).await,
                404,
                "Unknown setup operation.",
            );
            error(
                fixture.domain.execute("setup.status", json!({})).await,
                404,
                "Project not found.",
            );
        }
        "rollback" => {
            let id = fixture.preview_id().await;
            signal(selected, "gate-source")?;
            let install = fixture.spawn("setup.install", fixture.install(&id));
            // This gate is in apply_plan's source_commit admission, after the
            // existing serial lock, workspace operation lock and runner.idle().
            wait(selected, "git-entered").await?;
            fixture.sessions.close()?;
            let mut close = Box::pin(fixture.domain.close());
            assert!(once(close.as_mut()).await.is_pending());
            signal(selected, "rollback-pending")?;
            let failure = match within(install).await.unwrap() {
                Err(failure) => failure,
                Ok(_) => panic!("Shutdown must stop the observed installation"),
            };
            assert_eq!(failure.status, 409);
            // This exact diagnostic is emitted only after a durable publication
            // reached Setup's observer, then the installer compensated it.
            assert!(
                failure
                    .message
                    .contains("Accelerator setup stopped during installation")
            );
            assert!(
                failure
                    .message
                    .contains("reviewed effects were rolled back")
            );
            within(close).await;
            assert_eq!(fs::read(selected.join("project/AGENTS.md"))?, POLICY);
            let engine = selected
                .join("project/memory-bank/bin")
                .join(if cfg!(windows) {
                    "infra-engine.exe"
                } else {
                    "infra-engine"
                });
            assert!(!engine.exists());
            assert!(
                !selected
                    .join("project/memory-bank/local/accelerator-install.json")
                    .exists()
            );
            assert!(
                fixture
                    .store
                    .setting(&format!("setup.{}", fixture.project))?
                    .is_none()
            );
            // The public synchronous contract still reports the consumed ID,
            // independently of the wrapper's terminal closed state.
            assert_eq!(
                fixture
                    .setup
                    .install(&json!({"preview_id":id}))
                    .unwrap_err()
                    .message,
                "The preview expired or was already used. Preview again."
            );
        }
        _ => return Err(invalid("Unknown Setup fixture scenario")),
    }
    fixture.finish().await?;
    assert!(
        !selected.join("git-failure").exists(),
        "Fake Git rejected an invocation"
    );
    Ok(json!({"ok":true,"scenario":name}))
}

fn git_calls(selected: &Path, scope: &str) -> Result<usize> {
    let entries = match fs::read_to_string(selected.join("git-calls.jsonl")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut count = 0;
    for line in entries.lines() {
        let entry: Value = serde_json::from_str(line)?;
        count += usize::from(entry["scope"] == scope);
    }
    Ok(count)
}
fn prepare(selected: &Path) -> Result<Value> {
    for name in [
        "bin",
        "source/install/inventories",
        "source/PHP Core",
        "project",
    ] {
        fs::create_dir_all(selected.join(name))?;
    }
    fs::copy(
        std::env::current_exe()?,
        selected
            .join("bin")
            .join(format!("git{}", std::env::consts::EXE_SUFFIX)),
    )?;
    fs::write(
        selected.join("source/PHP Core/AGENTS.md"),
        b"Fixture managed policy.\n",
    )?;
    fs::write(selected.join("project/AGENTS.md"), POLICY)?;
    let inventory = json!({"schema_version":2,"edition":"PHP Core","release":"fixture-1","excluded_tracked_paths":[],"source_overrides":{},"installed":{"shared":["AGENTS.md"],"claude":[],"cursor":[],"codex":[]}});
    fs::write(
        selected.join("source/install/inventories/php-core.json"),
        serde_json::to_vec(&inventory)?,
    )?;
    Ok(json!({"prepared":true}))
}
struct GitReceipt(PathBuf);
impl Drop for GitReceipt {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn fake_git(arguments: &[String]) -> Result<i32> {
    let own = std::env::current_exe()?;
    let selected = own
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| invalid("Missing fake Git root"))?;
    let prefix = [
        "--no-optional-locks",
        "--no-pager",
        "-c",
        if cfg!(windows) {
            "core.hooksPath=NUL"
        } else {
            "core.hooksPath=/dev/null"
        },
        "-c",
        "core.fsmonitor=false",
        "-C",
    ];
    if arguments.len() != 10 || arguments[..7] != prefix || arguments[8] != "rev-parse" {
        return Err(invalid("Unexpected fake Git invocation"));
    }
    let directory = fs::canonicalize(&arguments[7])?;
    let scope = if directory == fs::canonicalize(selected.join("source"))? && arguments[9] == "HEAD"
    {
        "source"
    } else if directory == fs::canonicalize(selected.join("project"))?
        && arguments[9] == "--show-toplevel"
    {
        "project"
    } else {
        return Err(invalid(
            "Fake Git only admits exact owned source/project queries",
        ));
    };
    let live = selected.join(format!("fake-git-{}.live", std::process::id()));
    fs::write(&live, b"owned")?;
    let _receipt = GitReceipt(live);
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(selected.join("git-calls.jsonl"))?;
    writeln!(
        log,
        "{}",
        json!({"scope":scope,"args":arguments,"pid":std::process::id()})
    )?;
    if selected.join(format!("gate-{scope}")).is_file() {
        signal(selected, "git-entered")?;
        let until = Instant::now() + DEADLINE;
        while !selected.join("release-git").is_file() {
            if Instant::now() >= until {
                return Err(invalid("Fake Git release deadline"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    // Deliberately ordinary non-Git source/project trees. No real Git runs.
    Ok(1)
}
fn ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("Runtime-free synchronous Setup path unexpectedly yielded"),
    }
}
fn runtime_free(selected: &Path) -> Result<Value> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let fixture = runtime.block_on(async { Fixture::new(selected) })?;
    drop(runtime);
    let status = value(ready(
        fixture.domain.execute("setup.status", fixture.status()),
    ));
    assert_eq!(status["available"], true);
    error(
        ready(
            fixture
                .domain
                .execute("setup.install", json!({"body":{"preview_id":null}})),
        ),
        400,
        "Invalid setup preview.",
    );
    ready(fixture.domain.close());
    ready(fixture.domain.close());
    error(
        ready(fixture.domain.execute("setup.status", fixture.status())),
        409,
        "Accelerator setup is closed.",
    );
    fixture.sessions.close()?;
    assert!(
        !selected.join("git-failure").exists(),
        "Fake Git rejected an invocation"
    );
    Ok(json!({"ok":true,"scenario":"runtime-free"}))
}
fn run(arguments: &[String]) -> Result<Value> {
    if arguments.len() == 2 && arguments[0] == "capabilities" && arguments[1] == "--json" {
        return Ok(
            json!({"engine":"infra-engine","version":"fixture-1","capabilities":["memory","brain","bank","hooks","mcp","install"]}),
        );
    }
    if arguments.len() != 2 {
        return Err(invalid("Expected fixture mode and absolute root"));
    }
    let selected = root(&arguments[1])?;
    if arguments[0] == "prepare" {
        return prepare(&selected);
    }
    if arguments[0] == "runtime-free" {
        return runtime_free(&selected);
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(scenario(&selected, &arguments[0]))
}
fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    #[cfg(any(unix, windows))]
    if arguments
        .first()
        .is_some_and(|argument| argument == "internal-guard")
    {
        std::process::exit(infra_host::process::guard_entry(&arguments[1..]).unwrap_or(125));
    }
    let own = std::env::current_exe().expect("fixture executable identity");
    if own.file_stem().is_some_and(|name| name == "git") {
        match fake_git(&arguments) {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                if let Some(selected) = own.parent().and_then(Path::parent) {
                    let _ = fs::write(selected.join("git-failure"), error.message.as_bytes());
                }
                eprintln!("{}", error.message);
                std::process::exit(2);
            }
        }
    }
    match run(&arguments) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{}", error.message);
            std::process::exit(2);
        }
    }
}
