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
