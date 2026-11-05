use super::generation_completion_tests::{Fixture, git_fixture};
use super::*;

fn running(fixture: &Fixture) -> (String, Job, Value) {
    let (sid, job, _) = git_fixture(fixture);
    fixture
        .sessions
        .store
        .patch(
            &sid,
            &json!({"status":"running","native_session_id":"native-1"}),
        )
        .unwrap();
    *fixture.sessions.commands.lock().unwrap() =
        Some(Arc::new(crate::commands::CommandCatalog::new(
            fixture.sessions.store.clone(),
            fixture.sessions.source.clone(),
            fixture.sessions.providers.providers.clone(),
        )));
    fixture
        .results
        .start_launch(&sid, &job.generation, None)
        .unwrap();
    let frozen = crate::inbox::authority(&fixture.sessions.store.get(&sid).unwrap()).unwrap();
    (sid, job, frozen)
}

fn complete(fixture: &Fixture, job: &Job, proof: Option<&str>) {
    assert_eq!(
        finalize_job(
            &fixture.sessions.store,
            job,
            &Ok(json!({})),
            &AtomicBool::new(false)
        )
        .unwrap(),
        "completed"
    );
    fixture
        .results
        .finish_launch(&job.generation, "completed")
        .unwrap();
    fixture
        .sessions
        .store
        .inbox_settle(&job.sid, &job.generation, true, proof)
        .unwrap();
}

fn receipt(fixture: &Fixture, sid: &str) -> Value {
    fixture.sessions.store.inbox_list(sid, None).unwrap()["items"][0].clone()
}

#[tokio::test]
async fn inbox_handoff_preserves_authority_and_assigns_one_generation_after_cleanup() {
    let mut fixture = Fixture::new();
    let (sid, job, frozen) = running(&fixture);
    let token = Arc::new(AtomicBool::new(false));
    fixture
        .sessions
        .active
        .lock()
        .unwrap()
        .insert(sid.clone(), token.clone());
    let accepted = fixture
        .sessions
        .accept_inbox(&sid, "next", " Continue. ")
        .await
        .unwrap();
    assert_eq!(accepted["status"], "pending");
    assert_eq!(
        fixture.sessions.store.generation(&sid).unwrap(),
        job.generation
    );
    assert!(!fixture.sessions.dispatch_inbox().await.unwrap());
    assert!(fixture.queue.try_recv().is_err());
    complete(&fixture, &job, Some("native-1"));
    // Terminal state alone is insufficient while cleanup still owns the session.
    assert!(fixture.sessions.dispatch_inbox().await.unwrap());
    assert!(fixture.queue.try_recv().is_err());
    fixture.sessions.remove_active(&sid, &token);
    assert!(!fixture.sessions.dispatch_inbox().await.unwrap());
    let next = fixture.queue.try_recv().unwrap().into_job();
    assert_ne!(next.generation, job.generation);
    assert_eq!(next.prompt, "Continue.");
    assert_eq!(
        crate::inbox::authority(&fixture.sessions.store.get(&sid).unwrap()).unwrap(),
        frozen
    );
    let replay = fixture
        .sessions
        .accept_inbox(&sid, "next", "Continue.")
        .await
        .unwrap();
    assert_eq!(replay["id"], accepted["id"]);
    assert_eq!(replay["status"], "assigned");
    assert_eq!(replay["delivery_generation"], next.generation);
    complete(&fixture, &job, Some("native-1"));
    assert!(!fixture.sessions.dispatch_inbox().await.unwrap());
    assert!(fixture.queue.try_recv().is_err());
    let events = fixture.sessions.store.events(&sid, 0).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event["inbox_request_id"] == "next")
            .count(),
        1
    );
    assert_eq!(
        fixture.sessions.store.generation(&sid).unwrap(),
        next.generation
    );
}

#[tokio::test]
async fn inbox_backpressure_retains_pending_until_a_waiting_slot_opens() {
    let mut fixture = Fixture::new();
    let (sid, job, _) = running(&fixture);
    fixture
        .sessions
        .accept_inbox(&sid, "next", "Continue.")
        .await
        .unwrap();
    complete(&fixture, &job, Some("native-1"));
    for _ in 0..16 {
        fixture.sessions.queue.try_send(job.clone()).unwrap();
    }
    assert!(fixture.sessions.dispatch_inbox().await.unwrap());
    assert_eq!(receipt(&fixture, &sid)["status"], "pending");
    assert_eq!(
        fixture.sessions.store.generation(&sid).unwrap(),
        job.generation
    );
    drop(fixture.queue.try_recv().unwrap());
    assert!(!fixture.sessions.dispatch_inbox().await.unwrap());
    assert_eq!(receipt(&fixture, &sid)["status"], "assigned");
    assert_eq!(fixture.queue.len(), 16);
}

#[tokio::test]
async fn inbox_drift_failure_missing_proof_and_cancel_never_start_a_followup() {
    for failure in ["scope", "authority", "failure", "proof", "cancel"] {
        let mut fixture = Fixture::new();
        let (sid, job, _) = running(&fixture);
        fixture
            .sessions
            .accept_inbox(&sid, "next", "Continue.")
            .await
            .unwrap();
        match failure {
            "failure" => {
                let result = Err(HarnessError::conflict("fixture provider failed"));
                finalize_job(
                    &fixture.sessions.store,
                    &job,
                    &result,
                    &AtomicBool::new(false),
                )
                .unwrap();
                fixture
                    .sessions
                    .store
                    .inbox_settle(&sid, &job.generation, false, None)
                    .unwrap();
            }
            "cancel" => {
                fixture.sessions.cancel(&sid).unwrap();
            }
            "proof" => complete(&fixture, &job, None),
            _ => {
                complete(&fixture, &job, Some("native-1"));
                if failure == "authority" {
                    fixture
                        .sessions
                        .store
                        .patch(&sid, &json!({"budget_revision":2}))
                        .unwrap();
                } else {
                    let root = fixture.temporary.path().join("project");
                    std::fs::rename(root.join(".git"), root.join("previous-git")).unwrap();
                    assert_eq!(
                        projects::git(&root, &["init".into(), "--quiet".into()], 5)
                            .unwrap()
                            .code,
                        0
                    );
                }
            }
        }
        fixture.sessions.dispatch_inbox().await.unwrap();
        assert!(fixture.queue.try_recv().is_err(), "{failure}");
        assert_eq!(
            receipt(&fixture, &sid)["status"],
            if failure == "cancel" {
                "cancelled"
            } else {
                "blocked"
            },
            "{failure}"
        );
        assert_eq!(
            fixture.sessions.store.generation(&sid).unwrap(),
            job.generation
        );
    }
}

#[tokio::test]
async fn inbox_rechecks_scope_and_authority_at_execution_in_serial_mode() {
    for changed_scope in [false, true] {
        let mut fixture = Fixture::new();
        let (sid, job, _) = running(&fixture);
        fixture
            .sessions
            .accept_inbox(&sid, "next", "Continue.")
            .await
            .unwrap();
        complete(&fixture, &job, Some("native-1"));
        fixture.sessions.dispatch_inbox().await.unwrap();
        let next = fixture.queue.try_recv().unwrap().into_job();
        if changed_scope {
            let root = fixture.temporary.path().join("project");
            std::fs::rename(root.join(".git"), root.join("previous-git")).unwrap();
            assert_eq!(
                projects::git(&root, &["init".into(), "--quiet".into()], 5)
                    .unwrap()
                    .code,
                0
            );
        } else {
            fixture
                .sessions
                .store
                .patch(&sid, &json!({"mode":"edit"}))
                .unwrap();
        }
        // None is the serial scheduler path; the fixture panics if native work starts.
        fixture.sessions.execute_job(next, None).await;
        assert_eq!(
            fixture.sessions.store.get(&sid).unwrap()["status"],
            "failed"
        );
        assert_eq!(receipt(&fixture, &sid)["status"], "uncertain");
        assert!(!fixture.sessions.stopping.load(Ordering::Acquire));
    }
}

#[tokio::test]
async fn inbox_actor_wakes_on_completion_and_drains_when_closed() {
    let mut fixture = Fixture::new();
    let (sid, job, _) = running(&fixture);
    fixture.sessions.inbox_done.store(false, Ordering::Release);
    fixture.sessions.start_inbox();
    fixture
        .sessions
        .accept_inbox(&sid, "next", "Continue.")
        .await
        .unwrap();
    complete(&fixture, &job, Some("native-1"));
    fixture.sessions.inbox_wake.notify_one();
    let next = tokio::time::timeout(std::time::Duration::from_secs(3), fixture.queue.recv())
        .await
        .unwrap()
        .unwrap()
        .into_job();
    assert_ne!(next.generation, job.generation);
    fixture.sessions.close().unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        fixture.sessions.wait_closed(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(fixture.sessions.inbox_done.load(Ordering::Acquire));
}

#[tokio::test]
async fn inbox_checks_identity_on_terminal_result_as_well_as_session_events() {
    let fixture = Fixture::new();
    let (sid, job, _) = running(&fixture);
    fixture
        .sessions
        .accept_inbox(&sid, "next", "Continue.")
        .await
        .unwrap();
    for id in ["native-1", "other-native"] {
        let events = provider::normalize(
            "claude",
            &json!({"type":"result","subtype":"success","is_error":false,"result":"Done","session_id":id}),
        );
        let terminal = events
            .iter()
            .find(|event| event["kind"] == "result")
            .unwrap();
        assert_eq!(terminal["native_session_id"], id);
        assert_eq!(
            check_inbox_identity(&fixture.sessions.store, &sid, &job.generation, terminal).is_ok(),
            id == "native-1"
        );
    }
}

struct NoOtherDomain;
impl Domain for NoOtherDomain {
    fn execute<'a>(&'a self, _: &'a str, _: Value) -> Reply<'a> {
        Box::pin(async { Err(HarnessError::missing("Unused fixture domain.")) })
    }
}

#[tokio::test]
async fn inbox_http_enforces_token_exact_body_idempotency_limits_and_cancellation() {
    use crate::{http, services::NativeServices};
    let fixture = Fixture::new();
    let (sid, _, _) = running(&fixture);
    let unused: Arc<dyn Domain> = Arc::new(NoOtherDomain);
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
    domains.insert("sessions".into(), fixture.sessions.clone());
    let closing = fixture.sessions.clone();
    let services = Arc::new(
        NativeServices::new(
            fixture.sessions.store.clone(),
            domains,
            Arc::new(|| Ok(json!({}))),
            BTreeMap::new(),
            Arc::new(move || {
                let closing = closing.clone();
                Box::pin(async move {
                    closing.close()?;
                    closing.wait_closed().await
                })
            }),
        )
        .unwrap(),
    );
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let app = http::App {
        services,
        token: "private-inbox-token".into(),
        instance: "private-inbox-instance".into(),
        port,
        source: fixture.sessions.source.clone(),
        shutdown: shutdown.clone(),
        assets: Arc::new(BTreeMap::new()),
    };
    let server = tokio::spawn(http::serve_listener(listener, app));
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/api/sessions/{sid}/inbox");
    let body = json!({"request_id":"next","prompt":"Continue."}).to_string();
    let post = |url: String, body: String| {
        client
            .post(url)
            .header("x-harness-token", "private-inbox-token")
            .header("content-type", "application/json")
            .body(body)
    };
    assert_eq!(
        client
            .post(&url)
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let accepted = post(url.clone(), body.clone()).send().await.unwrap();
    assert_eq!(accepted.status(), 202);
    let first: Value = serde_json::from_str(&accepted.text().await.unwrap()).unwrap();
    let replay: Value = serde_json::from_str(
        &post(url.clone(), body)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first, replay);
    assert_eq!(
        post(
            url.clone(),
            json!({"request_id":"next","prompt":"Different."}).to_string()
        )
        .send()
        .await
        .unwrap()
        .status(),
        409
    );
    assert_eq!(
        post(
            url.clone(),
            json!({"request_id":"next","prompt":"Continue.","mode":"edit"}).to_string()
        )
        .send()
        .await
        .unwrap()
        .status(),
        400
    );
    assert_eq!(
        post(
            url.clone(),
            json!({"request_id":"large","prompt":"x".repeat(32001)}).to_string()
        )
        .send()
        .await
        .unwrap()
        .status(),
        400
    );
    let oversized = post(
        url.clone(),
        json!({"request_id":"huge","prompt":"x".repeat(70000)}).to_string(),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(oversized.status(), 400);
    assert!(
        oversized
            .text()
            .await
            .unwrap()
            .contains("JSON body length is missing or outside its byte limit.")
    );
    assert_eq!(
        client
            .get(format!("{url}?before=invalid"))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        client
            .get(format!("{url}?before=0"))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let list = client.get(&url).send().await.unwrap();
    assert_eq!(list.status(), 200);
    let list: Value = serde_json::from_str(&list.text().await.unwrap()).unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    let cancelled = post(format!("{url}/next/cancel"), "{}".into())
        .send()
        .await
        .unwrap();
    assert_eq!(cancelled.status(), 200);
    let cancelled: Value = serde_json::from_str(&cancelled.text().await.unwrap()).unwrap();
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(
        post(
            format!("http://127.0.0.1:{port}/api/sessions/missing/inbox"),
            json!({"request_id":"next","prompt":"Continue."}).to_string()
        )
        .send()
        .await
        .unwrap()
        .status(),
        404
    );
    shutdown.send(true).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
