//! Bounded waiting and conservative execution leases for native sessions.
use std::{
    collections::VecDeque,
    future::Future,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc};

pub(crate) const WAITING: usize = 16;
pub(crate) const MAX_CONCURRENCY: usize = 4;

pub(crate) struct Queue<T> {
    sender: mpsc::Sender<Waiting<T>>,
    slots: Arc<Semaphore>,
}
pub(crate) struct Waiting<T> {
    pub job: T,
    _slot: OwnedSemaphorePermit,
}
impl<T> Waiting<T> {
    pub fn into_job(self) -> T {
        self.job
    }
}
impl<T> Queue<T> {
    pub fn capacity(&self) -> usize {
        self.slots.available_permits()
    }
    pub fn try_send(&self, job: T) -> Result<(), ()> {
        let slot = self.slots.clone().try_acquire_owned().map_err(|_| ())?;
        self.sender
            .try_send(Waiting { job, _slot: slot })
            .map_err(|_| ())
    }
}
pub(crate) fn channel<T>() -> (Queue<T>, mpsc::Receiver<Waiting<T>>) {
    let (sender, receiver) = mpsc::channel(WAITING);
    (
        Queue {
            sender,
            slots: Arc::new(Semaphore::new(WAITING)),
        },
        receiver,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Lease {
    sid: String,
    // None is exclusive, including every unverified or specialized workflow.
    repository: Option<[PathBuf; 3]>,
    images: Vec<infra_core::fs::NativeIdentity>,
    mounts: Vec<crate::session_mounts::MountScope>,
}
fn key(value: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(value.to_string_lossy().replace('/', "\\").to_lowercase())
    } else {
        value.to_path_buf()
    }
}
impl Lease {
    pub fn exclusive(sid: &str) -> Self {
        Self {
            sid: sid.into(),
            repository: None,
            images: vec![],
            mounts: vec![],
        }
    }
    pub fn repository(sid: &str, workspace: &Path, top: &Path, common: &Path) -> Self {
        Self {
            sid: sid.into(),
            repository: Some([key(workspace), key(top), key(common)]),
            images: vec![],
            mounts: vec![],
        }
    }
    pub fn with_images(mut self, images: Vec<infra_core::fs::NativeIdentity>) -> Self {
        self.images = images;
        self
    }
    pub fn parallel(&self) -> bool {
        self.repository.is_some()
    }
    pub fn with_mounts(mut self, mounts: Vec<crate::session_mounts::MountScope>) -> Self {
        self.mounts = mounts;
        self
    }
    pub(super) fn conflicts(&self, other: &Self) -> bool {
        if self.sid == other.sid {
            return true;
        }
        if self
            .mounts
            .iter()
            .any(|a| other.mounts.iter().any(|b| a.overlaps(b)))
        {
            return true;
        }
        // Canonical paths do not collapse bind-mounted aliases. Compare stable
        // object IDs as well; content/permission changes must not create a new lease.
        if self.images.iter().any(|a| {
            other
                .images
                .iter()
                .any(|b| a.device == b.device && a.file == b.file)
        }) {
            return true;
        }
        match (&self.repository, &other.repository) {
            (Some(a), Some(b)) => a
                .iter()
                .any(|a| b.iter().any(|b| a.starts_with(b) || b.starts_with(a))),
            _ => true,
        }
    }
}
fn next<T>(pending: &VecDeque<(Waiting<T>, Lease)>, active: &[Lease]) -> Option<usize> {
    for (index, (_, lease)) in pending.iter().enumerate() {
        if !lease.parallel() {
            // Nothing after this fence may overtake an exclusive workflow.
            return (index == 0 && active.is_empty()).then_some(index);
        }
        if !active.iter().any(|other| lease.conflicts(other)) {
            return Some(index);
        }
    }
    None
}

pub(crate) async fn run<T, C, CF, R, RF, P>(
    mut receiver: mpsc::Receiver<Waiting<T>>,
    limit: usize,
    stopping: Arc<AtomicBool>,
    wake: Arc<Notify>,
    mut classify: C,
    mut execute: R,
    failed: P,
) where
    T: Send + 'static,
    C: FnMut(&T) -> CF,
    CF: Future<Output = Lease>,
    R: FnMut(T, Lease) -> RF,
    RF: Future<Output = ()> + Send + 'static,
    P: Fn(),
{
    let mut pending = VecDeque::new();
    let mut active = vec![];
    let mut running = tokio::task::JoinSet::new();
    let mut receiving = true;
    loop {
        if stopping.load(Ordering::Acquire) {
            receiver.close();
            pending.clear();
            while receiver.try_recv().is_ok() {}
            while running.join_next().await.is_some() {}
            break;
        }
        while running.len() < limit {
            let Some(index) = next(&pending, &active) else {
                break;
            };
            let (waiting, lease) = pending.remove(index).unwrap();
            let future = execute(waiting.into_job(), lease.clone());
            active.push(lease.clone());
            running.spawn(async move {
                future.await;
                lease
            });
        }
        if !receiving && pending.is_empty() && running.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            _ = wake.notified() => {},
            outcome = running.join_next(), if !running.is_empty() => {
                match outcome {
                    Some(Ok(lease)) => {
                        if let Some(index) = active.iter().position(|held| held == &lease) { active.remove(index); }
                    }
                    Some(Err(_)) => { stopping.store(true, Ordering::Release); failed(); }
                    None => {},
                }
            }
            waiting = receiver.recv(), if receiving => {
                match waiting {
                    Some(waiting) => {
                        let lease = classify(&waiting.job).await;
                        pending.push_back((waiting, lease));
                    }
                    None => receiving = false,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::oneshot;
    #[derive(Clone)]
    struct Work {
        id: usize,
        lease: Lease,
    }
    struct Started {
        id: usize,
        release: oneshot::Sender<()>,
    }
    struct Fixture {
        queue: Queue<Work>,
        started: mpsc::UnboundedReceiver<Started>,
        classified: mpsc::UnboundedReceiver<usize>,
        stop: Arc<AtomicBool>,
        wake: Arc<Notify>,
        runner: tokio::task::JoinHandle<()>,
    }
    fn repo(sid: &str, root: &str) -> Lease {
        let top = std::env::temp_dir().join(root);
        Lease::repository(sid, &top, &top, &top.join(".git"))
    }
    impl Fixture {
        fn new(limit: usize) -> Self {
            let (queue, receiver) = channel();
            let (started_tx, started) = mpsc::unbounded_channel();
            let (classified_tx, classified) = mpsc::unbounded_channel();
            let stop = Arc::new(AtomicBool::new(false));
            let wake = Arc::new(Notify::new());
            let runner = tokio::spawn(run(
                receiver,
                limit,
                stop.clone(),
                wake.clone(),
                move |work: &Work| {
                    let lease = work.lease.clone();
                    classified_tx.send(work.id).unwrap();
                    async move { lease }
                },
                move |work, _| {
                    let tx = started_tx.clone();
                    async move {
                        let (release, released) = oneshot::channel();
                        tx.send(Started {
                            id: work.id,
                            release,
                        })
                        .unwrap();
                        let _ = released.await;
                    }
                },
                || {},
            ));
            Self {
                queue,
                started,
                classified,
                stop,
                wake,
                runner,
            }
        }
        fn send(&self, id: usize, lease: Lease) {
            assert!(self.queue.try_send(Work { id, lease }).is_ok());
        }
        async fn started(&mut self) -> Started {
            tokio::time::timeout(Duration::from_secs(5), self.started.recv())
                .await
                .unwrap()
                .unwrap()
        }
        async fn finish(self) {
            drop(self.queue);
            tokio::time::timeout(Duration::from_secs(5), self.runner)
                .await
                .unwrap()
                .unwrap();
        }
    }
    #[tokio::test]
    async fn independent_repositories_overlap_and_conflicting_jobs_do_not_block_them() {
        let mut f = Fixture::new(2);
        f.send(1, repo("one", "scheduler-a"));
        let a = f.started().await;
        assert_eq!(a.id, 1);
        f.send(2, repo("two", "scheduler-a"));
        f.send(3, repo("three", "scheduler-b"));
        let b = f.started().await;
        assert_eq!(b.id, 3);
        a.release.send(()).unwrap();
        let same_repo = f.started().await;
        assert_eq!(same_repo.id, 2);
        same_repo.release.send(()).unwrap();
        b.release.send(()).unwrap();
        f.finish().await;
    }
    #[tokio::test]
    async fn serial_limit_never_dispatches_a_second_independent_job_while_first_is_held() {
        let (queue, receiver) = channel();
        let (started_tx, mut started) = mpsc::unbounded_channel();
        queue
            .try_send(Work {
                id: 1,
                lease: repo("one", "scheduler-a"),
            })
            .unwrap();
        queue
            .try_send(Work {
                id: 2,
                lease: repo("two", "scheduler-b"),
            })
            .unwrap();
        drop(queue);
        let mut runner = Box::pin(run(
            receiver,
            1,
            Arc::new(AtomicBool::new(false)),
            Arc::new(Notify::new()),
            |work: &Work| std::future::ready(work.lease.clone()),
            move |work, _| {
                let (release, released) = oneshot::channel();
                started_tx
                    .send(Started {
                        id: work.id,
                        release,
                    })
                    .unwrap();
                async move {
                    released.await.unwrap();
                }
            },
            || {},
        ));
        // Drive the production dispatcher until all immediately available input
        // is consumed; executor invocation is synchronous, so no sleep proves absence.
        let polled =
            std::future::poll_fn(|cx| std::task::Poll::Ready(runner.as_mut().poll(cx))).await;
        assert!(polled.is_pending());
        let a = started.try_recv().unwrap();
        assert_eq!(a.id, 1);
        assert!(started.try_recv().is_err());
        a.release.send(()).unwrap();
        let runner = tokio::spawn(runner);
        let b = tokio::time::timeout(Duration::from_secs(5), started.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(b.id, 2);
        b.release.send(()).unwrap();
        runner.await.unwrap();
    }
    #[tokio::test]
    async fn same_session_replacement_waits_for_old_cleanup_while_other_repository_runs() {
        let mut f = Fixture::new(2);
        f.send(1, repo("same-session", "scheduler-a"));
        let old = f.started().await;
        f.send(2, repo("same-session", "scheduler-b"));
        f.send(3, repo("independent", "scheduler-c"));
        let independent = f.started().await;
        assert_eq!(independent.id, 3);
        old.release.send(()).unwrap();
        let replacement = f.started().await;
        assert_eq!(replacement.id, 2);
        replacement.release.send(()).unwrap();
        independent.release.send(()).unwrap();
        f.finish().await;
    }
    #[tokio::test]
    async fn serial_limit_and_exclusive_fence_preserve_order() {
        for limit in [1, 2] {
            let mut f = Fixture::new(limit);
            f.send(1, repo("one", "scheduler-a"));
            let a = f.started().await;
            f.send(2, Lease::exclusive("two"));
            f.send(3, repo("three", "scheduler-b"));
            a.release.send(()).unwrap();
            let exclusive = f.started().await;
            assert_eq!(exclusive.id, 2);
            exclusive.release.send(()).unwrap();
            let b = f.started().await;
            assert_eq!(b.id, 3);
            b.release.send(()).unwrap();
            f.finish().await;
        }
    }
    #[tokio::test]
    async fn waiting_capacity_includes_the_dequeued_backlog_and_stop_drains_workers() {
        let mut f = Fixture::new(2);
        f.send(0, repo("zero", "scheduler-a"));
        let a = f.started().await;
        for id in 1..=WAITING {
            f.send(id, repo(&id.to_string(), "scheduler-a"));
        }
        for _ in 0..=WAITING {
            tokio::time::timeout(Duration::from_secs(5), f.classified.recv())
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(f.queue.capacity(), 0);
        assert!(
            f.queue
                .try_send(Work {
                    id: 99,
                    lease: repo("free", "scheduler-b")
                })
                .is_err()
        );
        f.stop.store(true, Ordering::Release);
        f.wake.notify_one();
        a.release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), &mut f.runner)
            .await
            .unwrap()
            .unwrap();
        assert!(f.started.try_recv().is_err());
        assert_eq!(f.queue.capacity(), WAITING);
    }
    #[tokio::test]
    async fn failure_stop_waits_for_sibling_cleanup_and_never_dispatches_pending_work() {
        let (queue, receiver) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Notify::new());
        let worker_stop = stop.clone();
        let worker_wake = wake.clone();
        let (started_tx, mut started) = mpsc::unbounded_channel();
        let (failed_tx, mut failed) = mpsc::unbounded_channel();
        for id in 1..=3 {
            queue
                .try_send(Work {
                    id,
                    lease: repo(&id.to_string(), &format!("failure-repo-{id}")),
                })
                .unwrap();
        }
        let mut runner = Box::pin(run(
            receiver,
            2,
            stop,
            wake,
            |work: &Work| std::future::ready(work.lease.clone()),
            move |work, _| {
                let (release, released) = oneshot::channel();
                started_tx
                    .send(Started {
                        id: work.id,
                        release,
                    })
                    .unwrap();
                let stop = worker_stop.clone();
                let wake = worker_wake.clone();
                let failed = failed_tx.clone();
                async move {
                    released.await.unwrap();
                    if work.id == 2 {
                        stop.store(true, Ordering::Release);
                        wake.notify_one();
                        failed.send(()).unwrap();
                    }
                }
            },
            || {},
        ));
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(runner.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        let sibling = started.try_recv().unwrap();
        assert_eq!(sibling.id, 1);
        let fatal = started.try_recv().unwrap();
        assert_eq!(fatal.id, 2);
        fatal.release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), failed.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(runner.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(started.try_recv().is_err());
        sibling.release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), runner)
            .await
            .unwrap();
        assert_eq!(queue.capacity(), WAITING);
        assert!(
            queue
                .try_send(Work {
                    id: 4,
                    lease: repo("four", "failure-repo-4")
                })
                .is_err()
        );
    }
    #[test]
    fn leases_exclude_same_session_linked_worktrees_nested_roots_and_unknowns() {
        let a = repo("one", "scheduler-a");
        assert!(a.conflicts(&repo("one", "scheduler-b")));
        let top = std::env::temp_dir().join("scheduler-linked");
        let common = std::env::temp_dir().join("scheduler-a/.git");
        assert!(a.conflicts(&Lease::repository("two", &top, &top, &common)));
        assert!(a.conflicts(&repo("two", "scheduler-a/nested")));
        assert!(a.conflicts(&Lease::exclusive("two")));
        assert!(!a.conflicts(&repo("two", "scheduler-b")));
    }
    #[test]
    fn physical_aliases_conflict_even_with_different_canonical_paths_and_permissions() {
        let temporary = tempfile::tempdir().unwrap();
        let image = infra_core::fs::native_image(temporary.path())
            .unwrap()
            .unwrap()
            .without_contents();
        let mut changed = image.clone();
        changed.kind ^= 0o100;
        let a = repo("one", "physical-alias-a").with_images(vec![image]);
        let b = repo("two", "physical-alias-b").with_images(vec![changed]);
        assert!(a.conflicts(&b));
    }
}
