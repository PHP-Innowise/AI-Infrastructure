use flate2::read::GzDecoder;
use infra_harness::{
    accelerators::Accelerators,
    bindings::{ResultsBridge, SessionLinks},
    commands::CommandCatalog,
    http::{self, App, Asset},
    knowledge::KnowledgeDomain,
    provider::ProviderDomain,
    results::Results,
    services::{CloseFuture, Domain, NativeServices, Payload, Reply},
    sessions::{BlackboardFactory, RunReply, SessionLimits, Sessions, SpecialWorkflow},
    store::SessionStore,
};
use reqwest::header;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read as _,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock, atomic::AtomicBool},
    time::Duration,
};
use tokio::net::TcpListener;

struct NoopSpecial;
impl SpecialWorkflow for NoopSpecial {
    fn run<'a>(
        &'a self,
        _session: Value,
        _prompt: String,
        _generation: String,
        _cancel: Arc<AtomicBool>,
        _emit: infra_harness::sessions::Event,
    ) -> RunReply<'a> {
        Box::pin(async { Ok(json!({})) })
    }
}

impl BlackboardFactory for NoopSpecial {
    fn for_session(
        &self,
        _session: &Value,
    ) -> infra_harness::Result<Arc<dyn infra_harness::fleet::Blackboard>> {
        unreachable!()
    }
}

#[derive(Default)]
struct RecordingDomain {
    calls: Mutex<Vec<(String, Value)>>,
}

impl Domain for RecordingDomain {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        let op = operation.to_owned();
        self.calls
            .lock()
            .unwrap()
            .push((op.clone(), request.clone()));
        Box::pin(async move {
            match op.as_str() {
                "graph.map" | "system_graph.service_map" => Ok(Payload::Json(json!({
                    "nodes": [],
                    "edges": [],
                    "paging": {
                        "offset": request["query"]["offset"],
                        "limit": request["query"]["limit"],
                        "depth": request["query"]["depth"],
                    }
                }))),
                "creator.start" | "creator.act" => Ok(Payload::Json(json!({
                    "run": {
                        "id": "cr-101",
                        "project_id": request["body"]["project_id"].as_str().unwrap_or("p1"),
                        "status": "generating",
                        "active": true,
                        "revision": 2,
                        "session_id": request["body"]["session_id"],
                    }
                }))),
                "systems.prepare" | "systems.act" | "systems.apply" | "discovery.start" => {
                    Ok(Payload::Json(json!({
                        "run": {
                            "id": "sys-202",
                            "project_id": request["body"]["project_id"].as_str().unwrap_or("p1"),
                            "status": "running",
                            "active": true,
                            "session_id": request["body"]["session_id"],
                        }
                    })))
                }
                "graph.act" => Ok(Payload::Json(json!({
                    "ok": true,
                    "project_id": request["params"]["project_id"],
                    "state": "building",
                    "revision": 7,
                    "nodes": 12,
                    "edges": 19,
                }))),
                "knowledge.memory_links_batch" => Ok(Payload::Json(json!({"items": []}))),
                "knowledge.memory_links" => Ok(Payload::Json(json!({"links": []}))),
                _ => Ok(Payload::Json(json!({"ok": true}))),
            }
        })
    }
}

struct TestServer {
    _dir: tempfile::TempDir,
    project_id: String,
    store: SessionStore,
    sessions: Arc<Sessions>,
    graph_domain: Arc<RecordingDomain>,
    system_graph_domain: Arc<RecordingDomain>,
    port: u16,
    token: String,
    shutdown: tokio::sync::watch::Sender<bool>,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
}

impl TestServer {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let state = root.join("state");
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let project_root = root.join("project");
        std::fs::create_dir_all(&project_root).unwrap();

        let store = SessionStore::open(&state).unwrap();
        let project = store.register(&project_root).unwrap();
        let project_id = project["id"].as_str().unwrap().to_owned();

        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::new())),
        });
        let knowledge = Arc::new(KnowledgeDomain::new(store.clone(), source.clone()));
        let special = ["creator", "system_run", "system_discovery"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    Arc::new(NoopSpecial) as Arc<dyn SpecialWorkflow>,
                )
            })
            .collect();
        let sessions = Sessions::new(
            store.clone(),
            source.clone(),
            providers.clone(),
            knowledge.clone(),
            Arc::new(NoopSpecial),
            special,
            SessionLimits {
                seconds: 30,
                concurrency: 1,
            },
        )
        .unwrap();
        let links = Arc::new(SessionLinks::new());
        links.bind(Arc::downgrade(&sessions)).unwrap();
        let results = Arc::new(
            Results::new(
                store.clone(),
                Arc::new(ResultsBridge {
                    links,
                    knowledge: knowledge.clone(),
                    operation: sessions.operation_lock(),
                    cancel: sessions.shutdown_cancel(),
                }),
            )
            .unwrap(),
        );
        sessions.install_results(results).unwrap();
        sessions
            .install_commands(Arc::new(CommandCatalog::new(
                store.clone(),
                source.clone(),
                providers.providers.clone(),
            )))
            .unwrap();
        sessions
            .install_accelerators(Arc::new(
                Accelerators::new(
                    store.clone(),
                    source.clone(),
                    providers,
                    Arc::downgrade(&sessions),
                )
                .unwrap(),
            ))
            .unwrap();
        sessions.activate().unwrap();

        let graph_domain = Arc::new(RecordingDomain::default());
        let system_graph_domain = Arc::new(RecordingDomain::default());
        let stub = Arc::new(RecordingDomain::default());

        let mut domains: BTreeMap<String, Arc<dyn Domain>> = BTreeMap::new();
        for name in [
            "setup",
            "accelerators",
            "systems",
            "discovery",
            "creator",
            "skills",
            "delivery",
            "projects",
            "commands",
            "files",
            "memory",
            "updates",
            "providers",
            "omnigraph",
            "results",
        ] {
            domains.insert(name.into(), stub.clone());
        }
        domains.insert("knowledge".into(), knowledge);
        domains.insert("graph".into(), graph_domain.clone());
        domains.insert("system_graph".into(), system_graph_domain.clone());
        for name in ["sessions", "brain", "context"] {
            domains.insert(name.into(), sessions.clone());
        }

        let boot = store.clone();
        let closing = sessions.clone();
        let services = Arc::new(
            NativeServices::new(
                store.clone(),
                domains,
                Arc::new(move || {
                    Ok(json!({"projects": boot.projects()?, "sessions": boot.summaries()?}))
                }),
                BTreeMap::new(),
                Arc::new(move || {
                    let closing = closing.clone();
                    Box::pin(async move {
                        let requested = closing.close();
                        let reaped = closing.wait_closed().await;
                        requested.and(reaped)
                    }) as CloseFuture<'static>
                }),
            )
            .unwrap(),
        );

        let wasm_payload = "0123456789abcdef-wasm-module-data-"
            .repeat(256)
            .into_bytes();
        let css_payload = ":root{--bg:#fff;} body{margin:0;}\n"
            .repeat(128)
            .into_bytes();
        let html_payload = "<!doctype html><html><body><div id='app'>Harness</div></body></html>"
            .repeat(64)
            .into_bytes();
        let mut assets = BTreeMap::new();
        assets.insert(
            "/assets/infra_ui_bg.wasm".into(),
            Asset::from_bytes(wasm_payload, "application/wasm"),
        );
        assets.insert(
            "/app.css".into(),
            Asset::from_bytes(css_payload, "text/css; charset=utf-8"),
        );
        assets.insert(
            "/".into(),
            Asset::from_bytes(html_payload, "text/html; charset=utf-8"),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (shutdown, _) = tokio::sync::watch::channel(false);
        let token = "test-http-sse-token".to_owned();
        let app = App {
            services,
            token: token.clone(),
            instance: "test-sse".into(),
            port,
            source,
            shutdown: shutdown.clone(),
            assets: Arc::new(assets),
        };
        let server = tokio::spawn(async move {
            let _ = http::serve_listener(listener, app).await;
        });

        Self {
            _dir: dir,
            project_id,
            store,
            sessions,
            graph_domain,
            system_graph_domain,
            port,
            token,
            shutdown,
            server,
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    async fn get_json(&self, path: &str) -> Value {
        let resp = self.client.get(self.url(path)).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let bytes = resp.bytes().await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn post_json(&self, path: &str, body: Value) {
        let raw = body.to_string();
        let resp = self
            .client
            .post(self.url(path))
            .header("x-harness-token", &self.token)
            .header("content-type", "application/json")
            .header("content-length", raw.len())
            .body(raw)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        assert!(status.is_success(), "unexpected status {status} on {path}");
    }

    async fn close(self) {
        let _ = self.shutdown.send(true);
        let _ = self.sessions.close();
        let _ = self.sessions.wait_closed().await;
        self.server.abort();
    }
}

struct SseEvent {
    id: Option<u64>,
    event: String,
    data: Value,
}

async fn read_next_sse_event(resp: &mut reqwest::Response, buf: &mut String) -> Option<SseEvent> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(pos) = buf.find("\n\n") {
            let block = buf[..pos].to_owned();
            buf.drain(..pos + 2);
            if block.lines().all(|l| l.starts_with(':') || l.is_empty()) {
                continue;
            }
            let mut id = None;
            let mut event = "message".to_owned();
            let mut data_lines = Vec::new();
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("id:") {
                    id = v.trim().parse::<u64>().ok();
                } else if let Some(v) = line.strip_prefix("event:") {
                    event = v.trim().to_owned();
                } else if let Some(v) = line.strip_prefix("data:") {
                    data_lines.push(v.trim_start());
                }
            }
            let data = serde_json::from_str(&data_lines.join("\n")).unwrap_or(Value::Null);
            return Some(SseEvent { id, event, data });
        }
        let chunk = match tokio::time::timeout_at(deadline, resp.chunk()).await {
            Ok(Ok(Some(bytes))) => bytes,
            _ => return None,
        };
        buf.push_str(&String::from_utf8_lossy(&chunk));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn static_assets_serve_precompressed_gzip_vary_and_304_etags() {
    let srv = TestServer::start().await;

    for (path, expected_mime) in [
        ("/assets/infra_ui_bg.wasm", "application/wasm"),
        ("/app.css", "text/css; charset=utf-8"),
        ("/", "text/html; charset=utf-8"),
    ] {
        // 1. Client accepting gzip receives pre-compressed body, gzip ETag, and Vary: Accept-Encoding.
        let gz_resp = srv
            .client
            .get(srv.url(path))
            .header(header::ACCEPT_ENCODING, "gzip, deflate")
            .send()
            .await
            .unwrap();
        assert_eq!(gz_resp.status(), 200);
        assert_eq!(gz_resp.headers()[header::CONTENT_TYPE], expected_mime);
        assert_eq!(gz_resp.headers()[header::CONTENT_ENCODING], "gzip");
        assert_eq!(gz_resp.headers()[header::VARY], "Accept-Encoding");
        let gz_etag = gz_resp.headers()[header::ETAG].to_str().unwrap().to_owned();
        assert!(
            gz_etag.ends_with("-gzip\""),
            "expected gzip etag: {gz_etag}"
        );
        let gz_bytes = gz_resp.bytes().await.unwrap();

        let mut decoded = Vec::new();
        GzDecoder::new(&gz_bytes[..])
            .read_to_end(&mut decoded)
            .unwrap();

        // 2. Identity request (no Accept-Encoding or gzip;q=0) receives raw uncompressed bytes + identity ETag.
        let raw_resp = srv
            .client
            .get(srv.url(path))
            .header(header::ACCEPT_ENCODING, "gzip;q=0")
            .send()
            .await
            .unwrap();
        assert_eq!(raw_resp.status(), 200);
        assert!(raw_resp.headers().get(header::CONTENT_ENCODING).is_none());
        assert_eq!(raw_resp.headers()[header::VARY], "Accept-Encoding");
        let raw_etag = raw_resp.headers()[header::ETAG]
            .to_str()
            .unwrap()
            .to_owned();
        assert_ne!(raw_etag, gz_etag);
        let raw_bytes = raw_resp.bytes().await.unwrap();
        assert_eq!(decoded, raw_bytes.as_ref());
        assert!(gz_bytes.len() < raw_bytes.len());

        // 3. Conditional 304 matches representation-specific ETag cleanly.
        let not_mod_gz = srv
            .client
            .get(srv.url(path))
            .header(header::ACCEPT_ENCODING, "gzip")
            .header(header::IF_NONE_MATCH, &gz_etag)
            .send()
            .await
            .unwrap();
        assert_eq!(not_mod_gz.status(), 304);
        assert_eq!(not_mod_gz.headers()[header::VARY], "Accept-Encoding");
        assert_eq!(not_mod_gz.headers()[header::ETAG], gz_etag);

        // Sending raw ETag while requesting gzip returns 200 (not false 304).
        let mismatch = srv
            .client
            .get(srv.url(path))
            .header(header::ACCEPT_ENCODING, "gzip")
            .header(header::IF_NONE_MATCH, &raw_etag)
            .send()
            .await
            .unwrap();
        assert_eq!(mismatch.status(), 200);
        assert_eq!(mismatch.headers()[header::CONTENT_ENCODING], "gzip");
    }

    srv.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sse_stream_emits_typed_events_replays_last_event_id_and_resyncs_on_eviction() {
    let srv = TestServer::start().await;

    // Warm pooled HTTP/1.1 connection first to verify keep-alive transition into SSE works too.
    let boot = srv
        .client
        .get(srv.url("/api/bootstrap"))
        .send()
        .await
        .unwrap();
    assert_eq!(boot.status(), 200);
    let _ = boot.bytes().await.unwrap();

    // Open SSE stream without custom X-Harness-Token header (browser EventSource contract).
    let mut sse = srv
        .client
        .get(srv.url("/api/events/stream"))
        .send()
        .await
        .unwrap();
    assert_eq!(sse.status(), 200);
    assert_eq!(
        sse.headers()[header::CONTENT_TYPE],
        "text/event-stream; charset=utf-8"
    );
    let mut sse_buf = String::new();

    // Create session & insert > 1 events directly.
    let created = srv
        .store
        .create(
            &json!({
                "project_id": srv.project_id,
                "provider": "codex",
                "mode": "plan",
                "workflow": "native",
                "project_context": false,
            }),
            "hello stream",
            &[],
        )
        .unwrap();
    let sid = created["id"].as_str().unwrap().to_owned();

    // Collect initial session_event and session_status frames.
    let mut seen_events = Vec::new();
    for _ in 0..2 {
        let ev = read_next_sse_event(&mut sse, &mut sse_buf)
            .await
            .expect("expected initial SSE frame");
        seen_events.push(ev);
    }
    assert!(seen_events.iter().any(|e| e.event == "session_event"
        && e.data["session_id"] == sid
        && e.data["event"]["text"] == "hello stream"));
    assert!(
        seen_events
            .iter()
            .any(|e| e.event == "session_status" && e.data["session"]["id"] == sid)
    );
    let checkpoint_id = seen_events.iter().filter_map(|e| e.id).max().unwrap();

    // Emit typed creator_run, system_run, graph_status, waiting_counts, and results_check frames.
    srv.post_json(
        "/api/creator",
        json!({"project_id": srv.project_id, "session_id": sid}),
    )
    .await;
    srv.post_json(
        "/api/system-runs",
        json!({"project_id": srv.project_id, "session_id": sid}),
    )
    .await;
    srv.post_json(
        &format!("/api/projects/{}/graph", srv.project_id),
        json!({"action": "rebuild"}),
    )
    .await;
    srv.store
        .emit_waiting_change(&sid, BTreeMap::from([(sid.clone(), 2)]));
    srv.store.publish_event(
        "results_check",
        Some(sid.clone()),
        Some(srv.project_id.clone()),
        json!({
            "id": "chk-1",
            "check_id": "chk-1",
            "session_id": sid,
            "project_id": srv.project_id,
            "status": "passed",
            "check": {"id": "chk-1", "status": "passed"},
        }),
    );

    let mut kinds = Vec::new();
    while kinds.len() < 5 {
        let ev = read_next_sse_event(&mut sse, &mut sse_buf)
            .await
            .expect("expected live typed SSE frame");
        if [
            "creator_run",
            "system_run",
            "graph_status",
            "waiting_counts",
            "results_check",
        ]
        .contains(&ev.event.as_str())
        {
            kinds.push(ev.event.clone());
        }
    }
    for required in [
        "creator_run",
        "system_run",
        "graph_status",
        "waiting_counts",
        "results_check",
    ] {
        assert!(
            kinds.iter().any(|k| k == required),
            "missing {required}: {kinds:?}"
        );
    }
    drop(sse);

    // Replay from Last-Event-ID header receives missed frames without polling.
    let mut replay = srv
        .client
        .get(srv.url("/api/events/stream"))
        .header("Last-Event-ID", checkpoint_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 200);
    let mut replay_buf = String::new();
    let replayed = read_next_sse_event(&mut replay, &mut replay_buf)
        .await
        .expect("expected replayed frame after Last-Event-ID");
    assert!(replayed.id.unwrap() > checkpoint_id);
    assert_eq!(replayed.event, "creator_run");
    drop(replay);

    // Evict the ring buffer (> 1024 events) and verify an old Last-Event-ID triggers event: resync.
    for i in 0..1035 {
        srv.store.publish_event(
            "session_event",
            Some(sid.clone()),
            Some(srv.project_id.clone()),
            json!({
                "session_id": sid,
                "project_id": srv.project_id,
                "id": i + 100,
                "event": {"kind": "status", "text": format!("burst-{i}")},
            }),
        );
    }
    let mut expired = srv
        .client
        .get(srv.url(&format!("/api/events/stream?last_event_id={checkpoint_id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(expired.status(), 200);
    let mut exp_buf = String::new();
    let first = read_next_sse_event(&mut expired, &mut exp_buf)
        .await
        .expect("expected resync event");
    assert_eq!(first.event, "resync");
    assert_eq!(first.data["reason"], "cursor_expired");

    srv.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_backlog_pagination_and_graph_map_query_params_work_end_to_end() {
    let srv = TestServer::start().await;

    let created = srv
        .store
        .create(
            &json!({
                "project_id": srv.project_id,
                "provider": "codex",
                "mode": "plan",
                "workflow": "native",
                "project_context": false,
            }),
            "seed",
            &[],
        )
        .unwrap();
    let sid = created["id"].as_str().unwrap().to_owned();
    for i in 1..=280 {
        srv.store
            .event(&sid, &json!({"kind": "status", "text": format!("msg-{i}")}))
            .unwrap();
    }

    // tail=1&limit=50 returns latest 50 events out of 281 total with has_older=true.
    let tail_res = srv
        .get_json(&format!("/api/sessions/{sid}?tail=1&limit=50"))
        .await;
    assert_eq!(tail_res["total_events"], 281);
    assert_eq!(tail_res["events"].as_array().unwrap().len(), 50);
    assert_eq!(tail_res["has_older"], true);
    assert_eq!(tail_res["has_more"], false);
    assert_eq!(tail_res["events"][49]["text"], "msg-280");

    let oldest_in_tail = tail_res["events"][0]["id"].as_i64().unwrap();
    let older_res = srv
        .get_json(&format!(
            "/api/sessions/{sid}?before={oldest_in_tail}&limit=100"
        ))
        .await;
    assert_eq!(older_res["events"].as_array().unwrap().len(), 100);
    assert_eq!(older_res["has_more"], true);
    assert_eq!(older_res["has_older"], true);

    // Verify graph.map and system_graph.service_map forward offset, limit, depth query params cleanly.
    let map_res = srv
        .get_json(&format!(
            "/api/projects/{}/graph/map?offset=25&limit=100&depth=3",
            srv.project_id
        ))
        .await;
    assert_eq!(map_res["paging"]["offset"], "25");
    assert_eq!(map_res["paging"]["limit"], "100");
    assert_eq!(map_res["paging"]["depth"], "3");

    let svc_res = srv
        .get_json(&format!(
            "/api/systems/service-graph/map?project_id={}&offset=10&limit=50&depth=2",
            srv.project_id
        ))
        .await;
    assert_eq!(svc_res["paging"]["offset"], "10");
    assert_eq!(svc_res["paging"]["limit"], "50");
    assert_eq!(svc_res["paging"]["depth"], "2");
    assert!(!srv.graph_domain.calls.lock().unwrap().is_empty());
    assert!(!srv.system_graph_domain.calls.lock().unwrap().is_empty());

    srv.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sse_filters_head_validation_permission_timeout_events_and_delete_variants() {
    let srv = TestServer::start().await;

    // 1. HEAD /api/events/stream returns 200 text/event-stream immediately without hanging.
    let head_resp = srv
        .client
        .head(srv.url("/api/events/stream"))
        .send()
        .await
        .unwrap();
    assert_eq!(head_resp.status(), 200);
    assert_eq!(
        head_resp.headers()[header::CONTENT_TYPE],
        "text/event-stream; charset=utf-8"
    );

    // 2. Validation errors on bad SSE / session pagination cursors.
    assert_eq!(
        srv.client
            .get(srv.url("/api/events/stream?last_event_id=abc"))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let s1 = srv
        .store
        .create(
            &json!({
                "project_id": srv.project_id,
                "provider": "codex",
                "mode": "plan",
                "workflow": "native",
                "project_context": false,
            }),
            "s1",
            &[],
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let s2 = srv
        .store
        .create(
            &json!({
                "project_id": srv.project_id,
                "provider": "codex",
                "mode": "plan",
                "workflow": "native",
                "project_context": false,
            }),
            "s2",
            &[],
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        srv.client
            .get(srv.url(&format!("/api/sessions/{s1}?limit=9999")))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        srv.client
            .get(srv.url(&format!("/api/sessions/{s1}?after=1&before=2")))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );

    // 3. ?session_id= filter delivers only matching session frames (+ global waiting_counts/graph_status).
    let mut filtered = srv
        .client
        .get(srv.url(&format!("/api/events/stream?session_id={s1}")))
        .send()
        .await
        .unwrap();
    assert_eq!(filtered.status(), 200);
    let mut fbuf = String::new();
    srv.store
        .event(&s2, &json!({"kind": "status", "text": "other-session"}))
        .unwrap();
    srv.store
        .event(&s1, &json!({"kind": "status", "text": "target-session"}))
        .unwrap();
    let ev = read_next_sse_event(&mut filtered, &mut fbuf)
        .await
        .expect("expected filtered event for s1");
    assert_eq!(ev.event, "session_event");
    assert_eq!(ev.data["session_id"], s1);
    assert_eq!(ev.data["event"]["text"], "target-session");

    // 4. Configured PermissionDesk emits expires_at & timeout_seconds over SSE session_event.
    let desk = Arc::new(
        infra_harness::permissions::PermissionDesk::new_with_timeout(Some(Duration::from_secs(45))),
    );
    let store_emit = srv.store.clone();
    let sid_for_emit = s1.clone();
    let emit: infra_harness::sessions::Event = Arc::new(move |val| {
        store_emit.event(&sid_for_emit, &val)?;
        Ok(())
    });
    let approvals = desk.approvals(&s1, emit);
    let _rx = approvals
        .ask_answer(&infra_host::permission::PermissionAsk {
            id: "ask-timeout".into(),
            provider: "codex".into(),
            tool: "Shell".into(),
            summary: "cargo test".into(),
            session_option: true,
            ..Default::default()
        })
        .unwrap();
    let perm_sse = read_next_sse_event(&mut filtered, &mut fbuf)
        .await
        .expect("expected permission ask over SSE");
    assert_eq!(perm_sse.event, "session_event");
    assert_eq!(perm_sse.data["event"]["kind"], "permission");
    assert_eq!(perm_sse.data["event"]["timeout_seconds"], 45);
    let exp = perm_sse.data["event"]["expires_at"]
        .as_str()
        .expect("expires_at present");
    chrono::DateTime::parse_from_rfc3339(exp).expect("valid RFC3339 expires_at");

    // 5. Permission rules DELETE route variants (missing session -> 404, missing args -> 400, JSON body / ?all=1 -> 200).
    assert_eq!(
        srv.client
            .get(srv.url("/api/sessions/does-not-exist/permission-rules"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let missing_arg = srv
        .client
        .delete(srv.url(&format!("/api/sessions/{s1}/permission-rules")))
        .header("x-harness-token", &srv.token)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_arg.status(), 400);

    let json_body = json!({"rule_key": "missing-rule"}).to_string();
    let del_json = srv
        .client
        .delete(srv.url(&format!("/api/sessions/{s1}/permission-rules")))
        .header("x-harness-token", &srv.token)
        .header("content-type", "application/json")
        .header("content-length", json_body.len())
        .body(json_body)
        .send()
        .await
        .unwrap();
    assert_eq!(del_json.status(), 200);

    let del_all = srv
        .client
        .delete(srv.url(&format!("/api/sessions/{s1}/permission-rules?all=1")))
        .header("x-harness-token", &srv.token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_all.status(), 200);

    srv.close().await;
}
