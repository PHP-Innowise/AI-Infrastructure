//! Exercise the real manager/SQLite path at a controllable synchronous host port.
use super::*;
use infra_core::{Result as CoreResult, RuntimeError};
use infra_system::{Project, SessionBridge, SystemManager};
use std::{
    fs::File,
    future::Future,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::{Context, Poll, Waker},
    time::Duration,
};
use tokio::sync::oneshot;

const DEADLINE: Duration = Duration::from_secs(5);
struct Block {
    entered: oneshot::Sender<()>,
    release: mpsc::Receiver<()>,
}
struct Release(Option<mpsc::Sender<()>>);
impl Release {
    fn open(mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
struct Bridge {
    root: PathBuf,
    calls: AtomicUsize,
    next: Mutex<Option<Block>>,
}
impl Bridge {
    fn block_next(&self) -> (oneshot::Receiver<()>, Release) {
        let (entered, receiver) = oneshot::channel();
        let (sender, release) = mpsc::channel();
        assert!(
            self.next
                .lock()
                .unwrap()
                .replace(Block { entered, release })
                .is_none()
        );
        (receiver, Release(Some(sender)))
    }
}
fn unused<T>() -> CoreResult<T> {
    Err(RuntimeError::unavailable(
        "Unexpected system fixture operation",
    ))
}
impl SessionBridge for Bridge {
    fn projects(&self) -> CoreResult<BTreeMap<String, Project>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let blocked = self.next.lock().unwrap().take();
        if let Some(blocked) = blocked {
            let _ = blocked.entered.send(());
            blocked.release.recv_timeout(DEADLINE * 2).map_err(|_| {
                RuntimeError::unavailable("The gated project read was not released")
            })?;
        }
        Ok(BTreeMap::from([(
            "fixture".into(),
            Project {
                id: "fixture".into(),
                name: "Fixture".into(),
                path: self.root.clone(),
            },
        )]))
    }
    fn provider(&self, _: &str) -> CoreResult<Option<PathBuf>> {
        unused()
    }
    fn active(&self) -> CoreResult<bool> {
        Ok(false)
    }
    fn admit(&self) -> CoreResult<()> {
        unused()
    }
    fn runner_lock(&self) -> CoreResult<Arc<File>> {
        unused()
    }
    fn launch(&self, _: &Value, _: Option<&Value>, _: Option<&Value>) -> CoreResult<Value> {
        unused()
    }
    fn session(&self, _: &str) -> CoreResult<Value> {
        unused()
    }
    fn cancel(&self, _: &str) -> CoreResult<Value> {
        unused()
    }
    fn events(&self, _: &str, _: &[&str]) -> CoreResult<Vec<Value>> {
        unused()
    }
    fn find_launch(&self, _: &str, _: &str) -> CoreResult<Option<Value>> {
        unused()
    }
}
fn fixture() -> (tempfile::TempDir, Arc<Bridge>, Arc<SystemsDomain>) {
    let temporary = tempfile::tempdir().unwrap();
    let bridge = Arc::new(Bridge {
        root: temporary.path().to_path_buf(),
        calls: AtomicUsize::new(0),
        next: Mutex::new(None),
    });
    let manager = Arc::new(
        SystemManager::new(
            temporary.path().to_path_buf(),
            temporary.path().join("state"),
            bridge.clone(),
        )
        .unwrap(),
    );
    (temporary, bridge, Arc::new(SystemsDomain::new(manager)))
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}
fn request() -> Value {
    json!({"query":{"project_id":"fixture"}})
}
fn assert_empty(result: Result<Payload>) {
    match result.unwrap() {
        Payload::Json(value) => assert_eq!(value, json!({"runs":[]})),
        Payload::Bytes { .. } => panic!("Expected the manager's JSON result"),
    }
}
fn error(result: Result<Payload>) -> HarnessError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("Expected a manager error"),
    }
}
async fn within<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(DEADLINE * 3, future)
        .await
        .expect("System fixture exceeded its deadline")
}
async fn once<F: Future + ?Sized>(mut future: Pin<&mut F>) -> Poll<F::Output> {
    std::future::poll_fn(|context| Poll::Ready(future.as_mut().poll(context))).await
}

#[test]
fn a_gated_real_manager_read_leaves_the_current_thread_runtime_responsive() {
    let (_temporary, bridge, domain) = fixture();
    let (entered, release) = bridge.block_next();
    let (progress, observed) = mpsc::channel();
    let controller = std::thread::spawn(move || {
        let progressed_before_release = observed.recv_timeout(DEADLINE).is_ok();
        release.open();
        progressed_before_release
    });
    runtime().block_on(async {
        let owned = domain.clone();
        let operation = tokio::spawn(async move { owned.execute("systems.list", request()).await });
        within(entered).await.unwrap();
        assert!(
            !operation.is_finished(),
            "The controller has not released the synchronous read"
        );
        progress.send(()).unwrap();
        assert_empty(within(operation).await.unwrap());
    });
    assert!(
        controller.join().unwrap(),
        "Sibling progress must precede the host-port release"
    );
    assert_eq!(bridge.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn dropping_a_queued_request_never_submits_another_manager_operation() {
    let (_temporary, bridge, domain) = fixture();
    let (entered, release) = bridge.block_next();
    runtime().block_on(async {
        let owned = domain.clone();
        let active = tokio::spawn(async move { owned.execute("systems.list", request()).await });
        within(entered).await.unwrap();
        let mut queued = domain.execute("systems.list", request());
        assert!(once(queued.as_mut()).await.is_pending());
        drop(queued);
        assert_eq!(bridge.calls.load(Ordering::SeqCst), 1);
        release.open();
        assert_empty(within(active).await.unwrap());
        assert_empty(within(domain.execute("systems.list", request())).await);
    });
    assert_eq!(bridge.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn an_aborted_active_caller_keeps_its_gate_until_close_can_drain() {
    let (_temporary, bridge, domain) = fixture();
    let (entered, release) = bridge.block_next();
    runtime().block_on(async {
        let owned = domain.clone();
        let active = tokio::spawn(async move { owned.execute("systems.list", request()).await });
        within(entered).await.unwrap();
        active.abort();
        assert!(within(active).await.err().unwrap().is_cancelled());
        assert!(
            domain.gate.try_lock().is_err(),
            "The blocking closure still owns admission"
        );
        let mut closing = Box::pin(domain.close());
        assert!(once(closing.as_mut()).await.is_pending());
        let mut queued = domain.execute("systems.list", request());
        assert!(once(queued.as_mut()).await.is_pending());
        drop(queued);
        release.open();
        within(closing).await;
        within(domain.close()).await;
        let refused = error(within(domain.execute("systems.list", request())).await);
        assert_eq!(refused.status, 409);
        assert_eq!(refused.message, "The system manager is closed.");
    });
    assert_eq!(bridge.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn close_drains_prior_queued_work_and_refuses_a_later_live_waiter() {
    let (_temporary, bridge, domain) = fixture();
    let (entered, release) = bridge.block_next();
    runtime().block_on(async {
        let owned = domain.clone();
        let active = tokio::spawn(async move { owned.execute("systems.list", request()).await });
        within(entered).await.unwrap();

        // Register each live waiter explicitly so the queue order is A, B,
        // close, C without depending on spawned-task scheduling.
        let mut before_close = domain.execute("systems.list", request());
        assert!(once(before_close.as_mut()).await.is_pending());
        let mut closing = Box::pin(domain.close());
        assert!(once(closing.as_mut()).await.is_pending());
        let mut after_close = domain.execute("systems.list", request());
        assert!(once(after_close.as_mut()).await.is_pending());
        assert_eq!(bridge.calls.load(Ordering::SeqCst), 1);

        release.open();
        assert_empty(within(active).await.unwrap());
        assert!(once(closing.as_mut()).await.is_pending());
        assert_empty(within(before_close).await);
        assert_eq!(bridge.calls.load(Ordering::SeqCst), 2);
        within(closing).await;
        let refused = error(within(after_close).await);
        assert_eq!(refused.status, 409);
        assert_eq!(refused.message, "The system manager is closed.");
        within(domain.close()).await;
    });
    assert_eq!(bridge.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn manager_errors_are_preserved_and_release_the_gate_for_later_requests() {
    let (_temporary, _bridge, domain) = fixture();
    runtime().block_on(async {
        for (operation, input) in [
            ("systems.list", json!({"query":{"project_id":"missing"}})),
            ("systems.get", json!({"params":{"id":"invalid-id"}})),
            ("systems.unknown", json!({})),
        ] {
            let expected: HarnessError = domain
                .manager
                .execute(operation, &input)
                .unwrap_err()
                .into();
            let actual = error(within(domain.execute(operation, input)).await);
            assert_eq!(
                (actual.status, actual.code, actual.message),
                (expected.status, expected.code, expected.message)
            );
            assert_empty(within(domain.execute("systems.list", request())).await);
        }
    });
}

#[test]
fn manager_requests_still_work_without_a_tokio_runtime() {
    let (_temporary, _bridge, domain) = fixture();
    assert!(tokio::runtime::Handle::try_current().is_err());
    let mut context = Context::from_waker(Waker::noop());
    let mut operation = domain.execute("systems.list", request());
    match operation.as_mut().poll(&mut context) {
        Poll::Ready(result) => assert_empty(result),
        Poll::Pending => panic!("The synchronous fallback unexpectedly yielded"),
    }
    let input = json!({"query":{"project_id":"missing"}});
    let expected: HarnessError = domain
        .manager
        .execute("systems.list", &input)
        .unwrap_err()
        .into();
    let mut operation = domain.execute("systems.list", input);
    match operation.as_mut().poll(&mut context) {
        Poll::Ready(result) => {
            let actual = error(result);
            assert_eq!(
                (actual.status, actual.code, actual.message),
                (expected.status, expected.code, expected.message)
            );
        }
        Poll::Pending => panic!("The synchronous error path unexpectedly yielded"),
    }
    let mut closing = Box::pin(domain.close());
    assert!(matches!(
        closing.as_mut().poll(&mut context),
        Poll::Ready(())
    ));
    let mut operation = domain.execute("systems.list", request());
    match operation.as_mut().poll(&mut context) {
        Poll::Ready(result) => assert_eq!(error(result).status, 409),
        Poll::Pending => panic!("The closed synchronous fallback unexpectedly yielded"),
    }
}

#[test]
fn system_bridge_waiting_permissions_delegates_through_links() {
    use crate::sessions::{
        BlackboardFactory, Event, RunReply, SessionLimits, Sessions, SpecialWorkflow,
    };
    use std::sync::{Arc, Mutex, RwLock};

    struct NoopSpecial;
    impl SpecialWorkflow for NoopSpecial {
        fn run<'a>(
            &'a self,
            _: Value,
            _: String,
            _: String,
            _: Arc<std::sync::atomic::AtomicBool>,
            _: Event,
        ) -> RunReply<'a> {
            Box::pin(async { Ok(json!({})) })
        }
    }
    impl BlackboardFactory for NoopSpecial {
        fn for_session(&self, _: &Value) -> Result<Arc<dyn crate::fleet::Blackboard>> {
            Err(HarnessError::unavailable("unused"))
        }
    }

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let source = root.join("source");
    let project = root.join("project");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let store = SessionStore::open(&root.join("state")).unwrap();
    let project_id = store.register(&project).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sid = store
        .create(
            &json!({
                "project_id": project_id,
                "provider": "claude",
                "mode": "edit",
                "permissions": "manual"
            }),
            "hi",
            &[],
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    store
        .connection()
        .unwrap()
        .execute("UPDATE sessions SET status='running' WHERE id=?1", [&sid])
        .unwrap();
    let links = Arc::new(SessionLinks::new());
    let bridge = SystemBridge {
        links: links.clone(),
        store: store.clone(),
    };

    // Unbound links fail rather than silently claiming zero waiting permissions.
    assert!(bridge.waiting_permissions(&sid).is_err());

    runtime().block_on(async {
        let providers = Arc::new(crate::provider::ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::new())),
        });
        let knowledge = Arc::new(crate::knowledge::KnowledgeDomain::new(
            store.clone(),
            source.clone(),
        ));
        let noop: Arc<dyn SpecialWorkflow> = Arc::new(NoopSpecial);
        let special = BTreeMap::from([
            ("creator".to_owned(), noop.clone()),
            ("system_run".to_owned(), noop.clone()),
            ("system_discovery".to_owned(), noop),
        ]);
        let sessions = Sessions::new(
            store,
            source,
            providers,
            knowledge,
            Arc::new(NoopSpecial),
            special,
            SessionLimits {
                seconds: 60,
                concurrency: 1,
            },
        )
        .unwrap();
        links.bind(Arc::downgrade(&sessions)).unwrap();

        assert!(bridge.waiting_permissions(&sid).unwrap().is_empty());

        let events = Arc::new(Mutex::new(Vec::<Value>::new()));
        let recorded = events.clone();
        let emit: Event = Arc::new(move |ev| {
            recorded.lock().unwrap().push(ev);
            Ok(())
        });
        let approvals = sessions
            .system_approvals(&sid, &json!({"permissions": "manual"}), &emit)
            .expect("Manual system sessions need an approvals desk handle");
        let rx = approvals
            .ask_answer(&infra_host::permission::PermissionAsk {
                id: "req-bridge-1".into(),
                provider: "claude".into(),
                tool: "Edit".into(),
                summary: "Edit foreign service".into(),
                kind: "file_change".into(),
                ..Default::default()
            })
            .unwrap();

        let waiting = bridge.waiting_permissions(&sid).unwrap();
        assert_eq!(waiting.len(), 1);
        let request_id = waiting[0]["request_id"].as_str().unwrap().to_owned();
        assert!(request_id.starts_with("req-bridge-1-"));
        assert_eq!(waiting[0]["summary"], "Edit foreign service");

        sessions
            .decide_permission(&sid, &request_id, &json!({"decision": "allow"}))
            .unwrap();
        let answer = rx.await.unwrap();
        assert_eq!(answer.decision, infra_host::permission::Decision::Allow);
        assert!(bridge.waiting_permissions(&sid).unwrap().is_empty());

        let _ = sessions.close();
        let _ = sessions.wait_closed().await;
    });
}
