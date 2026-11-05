//! Visible test server using actual UI assets, isolated state and offline provider.
use super::*;
use infra_harness::{
    http,
    projects::ProjectDomain,
    services::{NativeServices, Reply},
};
use std::path::PathBuf;

struct Offline;
impl Domain for Offline {
    fn execute<'a>(&'a self, operation: &'a str, _: Value) -> Reply<'a> {
        Box::pin(async move {
            Ok(Payload::Json(match operation {
                "providers.sign_in" => {
                    json!({"providers":[{"id":"claude","state":"fixture","detail":"Offline fixture; no account or model"}]})
                }
                "updates.status" => json!({"available":false,"state":"fixture"}),
                _ => json!({"fixture":true}),
            }))
        })
    }
}
struct Faults {
    sessions: Arc<Sessions>,
    root: PathBuf,
}
impl Domain for Faults {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let consume = |name: &str| fs::remove_file(self.root.join(name)).is_ok();
            if operation == "sessions.inbox_list"
                && (self.root.join("fail-lists").exists() || consume("fail-next-list"))
            {
                return Err(HarnessError::unavailable(
                    "Fixture transient receipt read failure.",
                ));
            }
            if operation == "sessions.inbox_accept" && consume("reject-next-retry") {
                return Err(HarnessError::forbidden(
                    "Fixture token rejection; reconnect before confirming again.",
                ));
            }
            let result = self.sessions.execute(operation, request).await?;
            if operation == "sessions.inbox_accept" && consume("lose-next-reply") {
                if self.root.join("keep-receipts-unavailable").exists() {
                    fs::write(self.root.join("fail-lists"), b"fixture")?;
                }
                return Ok(Payload::Json(json!({"fixture_lost_receipt":true})));
            }
            Ok(result)
        })
    }
}

pub(super) async fn serve(root: &Path, source: &Path) -> Result<()> {
    let root = infra_core::fs::canonicalize(root)?;
    let source = infra_core::fs::canonicalize(source)?;
    fs::write(root.join("browser-fixture"), b"offline")?;
    let project = root.join("project");
    fs::create_dir(&project)?;
    require(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(&project)
            .status()?
            .success(),
        "Fixture Git init failed.",
    )?;
    let store = SessionStore::open(&root.join("state"))?;
    let registered = store.register(&project)?;
    let providers = Arc::new(ProviderDomain {
        providers: Arc::new(RwLock::new(BTreeMap::from([(
            "claude".into(),
            Provider {
                id: "claude".into(),
                name: "Offline fixture".into(),
                available: true,
                executable: Some(std::env::current_exe()?),
                detail: "No model/network".into(),
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
            seconds: 600,
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
        source.clone(),
        providers.clone(),
        Arc::downgrade(&sessions),
    )?))?;
    sessions.activate()?;
    let idle=store.create(&json!({"project_id":registered["id"],"provider":"claude","mode":"plan","workflow":"native","project_context":false}),"Other fixture conversation",&[])?;
    store.patch(idle["id"].as_str().unwrap(), &json!({"status":"completed"}))?;
    let source_turn=value(sessions.execute("sessions.create",json!({"body":{"project_id":registered["id"],"provider":"claude","workflow":"native","mode":"plan","agents_enabled":false,"agent_count":1,"project_context":false,"budgets":{"seconds":600},"prompt":"Source fixture question."}})).await?)?;
    let sid = source_turn["session"]["id"].as_str().unwrap().to_string();
    wait(|| Ok(store.get(&sid)?["native_session_id"] == "native-fixture")).await?;
    let unused: Arc<dyn Domain> = Arc::new(Offline);
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
    for name in ["sessions", "brain", "context"] {
        domains.insert(name.into(), sessions.clone());
    }
    domains.insert(
        "projects".into(),
        Arc::new(ProjectDomain {
            store: store.clone(),
        }),
    );
    let fault: Arc<dyn Domain> = Arc::new(Faults {
        sessions: sessions.clone(),
        root: root.clone(),
    });
    let overrides = ["sessions.inbox_accept", "sessions.inbox_list"]
        .into_iter()
        .map(|name| (name.into(), fault.clone()))
        .collect();
    let boot_store = store.clone();
    let boot_providers = providers.clone();
    let closing = sessions.clone();
    let services = Arc::new(NativeServices::new(
        store.clone(),
        domains,
        Arc::new(move || {
            Ok(
                json!({"projects":boot_store.projects()?,"providers":boot_providers.listing()?,"sessions":boot_store.summaries()?,"workflows":[{"id":"native","name":"Workspace"}],"sdd_phases":[],"runtime":{"source_version":"offline fixture","timeout_seconds":600,"max_active":1,"max_agents":40,"default_agent_count":1},"update":{"available":false}}),
            )
        }),
        overrides,
        Arc::new(move || {
            let closing = closing.clone();
            Box::pin(async move {
                closing.close()?;
                closing.wait_closed().await
            })
        }),
    )?);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let app = http::App::new(services, port, source, shutdown.clone())?;
    let receipt = json!({"url":format!("http://127.0.0.1:{port}"),"session_id":sid,"other_session_id":idle["id"],"root":root});
    fs::write(root.join("browser.json"), receipt.to_string())?;
    println!("{receipt}");
    // A file gate exposes the otherwise very short completed→queued interval.
    let operation = sessions.operation_lock();
    let hold_root = root.clone();
    let hold_stop = shutdown.subscribe();
    let holding = tokio::spawn(async move {
        let until = Instant::now() + Duration::from_secs(750);
        while Instant::now() < until && !*hold_stop.borrow() {
            if hold_root.join("hold-assignment").exists() {
                let _guard = operation.lock().await;
                while hold_root.join("hold-assignment").exists()
                    && Instant::now() < until
                    && !*hold_stop.borrow()
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let deadline_shutdown = shutdown.clone();
    let deadline = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(750)).await;
        let _ = deadline_shutdown.send(true);
    });
    let result = http::serve_listener(listener, app).await;
    let _ = shutdown.send(true);
    holding
        .await
        .map_err(|_| HarnessError::unavailable("Fixture hold task failed."))?;
    deadline.abort();
    sessions.close()?;
    sessions.wait_closed().await?;
    result
}
