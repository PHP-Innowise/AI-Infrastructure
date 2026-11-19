use infra_harness::{
    http::{App, serve_listener},
    services::{CloseFuture, Payload, Reply, Services},
    store::{ReplayStart, SessionStore},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};

struct PipeServices {
    store: SessionStore,
}

impl Services for PipeServices {
    fn request<'a>(&'a self, operation: &'a str, _request: Value) -> Reply<'a> {
        let op = operation.to_owned();
        Box::pin(async move {
            match op.as_str() {
                "health" => Ok(Payload::Json(json!({"ok": true}))),
                other => Ok(Payload::Json(json!({"op": other}))),
            }
        })
    }

    fn close<'a>(&'a self) -> CloseFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn subscribe_stream(&self, last_event_id: Option<u64>) -> Option<ReplayStart> {
        Some(self.store.stream().subscribe(last_event_id))
    }
}

async fn read_until(stream: &mut TcpStream, buf: &mut Vec<u8>, needle: &[u8]) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut tmp = [0_u8; 4096];
    while !buf.windows(needle.len()).any(|w| w == needle) {
        let rem = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !rem.is_zero(),
            "timed out waiting for {:?} in {:?}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(buf)
        );
        let n = tokio::time::timeout(rem, stream.read(&mut tmp))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "socket read timeout waiting for {:?}; received so far: {:?}",
                    String::from_utf8_lossy(needle),
                    String::from_utf8_lossy(buf)
                )
            })
            .expect("socket read error");
        assert!(
            n > 0,
            "unexpected EOF before {:?}: {:?}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(buf)
        );
        buf.extend_from_slice(&tmp[..n]);
    }
}

#[tokio::test]
async fn pipelined_http1_requests_before_sse_upgrade_work_and_shutdown_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(&dir.path().join("state")).unwrap();
    let services: Arc<dyn Services> = Arc::new(PipeServices {
        store: store.clone(),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, _) = watch::channel(false);
    let app = App {
        services,
        token: "test-token".to_owned(),
        instance: "inst-1".to_owned(),
        port: addr.port(),
        source: dir.path().to_path_buf(),
        shutdown: shutdown_tx.clone(),
        assets: Arc::new(BTreeMap::new()),
    };

    let server_task = tokio::spawn(serve_listener(listener, app));

    // Send 3 pipelined HTTP/1.1 requests in ONE write before reading any response:
    // 1) GET /api/health (JSON body)
    // 2) HEAD /api/health (must NOT expect body despite any length)
    // 3) GET /api/events/stream?session_id=pipe-s1 (SSE handoff)
    let mut client = TcpStream::connect(addr).await.unwrap();
    let host = format!("127.0.0.1:{}", addr.port());
    let pipeline = format!(
        "GET /api/health HTTP/1.1\r\nHost: {host}\r\n\r\n\
         HEAD /api/health HTTP/1.1\r\nHost: {host}\r\n\r\n\
         GET /api/events/stream?session_id=pipe-s1 HTTP/1.1\r\nHost: {host}\r\n\r\n"
    );
    client.write_all(pipeline.as_bytes()).await.unwrap();

    let mut received = Vec::new();
    read_until(&mut client, &mut received, b": connected\n\n").await;
    let preamble = String::from_utf8_lossy(&received);
    assert!(
        preamble.contains("{\"ok\":true,\"instance\":\"inst-1\"}"),
        "missing first pipelined GET body: {preamble}"
    );
    assert!(
        !preamble.contains("x-harness-sse-handoff"),
        "internal SSE handoff header leaked: {preamble}"
    );
    assert!(
        preamble.contains("transfer-encoding: chunked"),
        "expected chunked SSE response headers after pipeline: {preamble}"
    );

    store.publish_event(
        "session_event",
        Some("pipe-s1".to_owned()),
        None,
        json!({"pipelined": true}),
    );
    read_until(&mut client, &mut received, b"\"pipelined\":true").await;

    // Trigger graceful server shutdown; SseTaskTracker signals and waits for SSE stream to end.
    let _ = shutdown_tx.send(true);
    let mut eof_buf = [0_u8; 1024];
    let eof_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let rem = eof_deadline.saturating_duration_since(tokio::time::Instant::now());
        let n = tokio::time::timeout(rem, client.read(&mut eof_buf))
            .await
            .expect("SSE socket should close on shutdown")
            .unwrap();
        if n == 0 {
            break;
        }
    }
    tokio::time::timeout(Duration::from_secs(5), server_task)
        .await
        .expect("server_task should exit cleanly")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn sse_tasks_cancel_cleanly_when_server_future_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(&dir.path().join("state")).unwrap();
    let services: Arc<dyn Services> = Arc::new(PipeServices { store });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, _) = watch::channel(false);
    let app = App {
        services,
        token: "test-token".to_owned(),
        instance: "inst-2".to_owned(),
        port: addr.port(),
        source: dir.path().to_path_buf(),
        shutdown: shutdown_tx,
        assets: Arc::new(BTreeMap::new()),
    };

    let server_task = tokio::spawn(serve_listener(listener, app));
    let mut client = TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "GET /api/events/stream?session_id=drop-s1 HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        addr.port()
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let mut received = Vec::new();
    read_until(&mut client, &mut received, b": connected\n\n").await;

    // Aborting the server task drops SseTaskAbortGuard, which cancels and aborts spawned SSE tasks.
    server_task.abort();
    let _ = server_task.await;

    let mut tmp = [0_u8; 256];
    let res = tokio::time::timeout(Duration::from_secs(5), client.read(&mut tmp))
        .await
        .expect("aborted server should tear down SSE stream quickly");
    assert!(
        matches!(res, Ok(0) | Err(_)),
        "expected EOF/reset after server abort, got {res:?}"
    );
}
