//! A session's Permissions setting end to end, through the HTTP routes and an
//! actual native process: a fake Claude Code that speaks the stream-json
//! control channel records its command line and asks for one Bash command.
#![cfg(unix)]

use infra_harness::{
    accelerators::Accelerators,
    bindings::{ResultsBridge, SessionLinks},
    commands::CommandCatalog,
    http::{self, App},
    knowledge::KnowledgeDomain,
    provider::{Provider, ProviderDomain},
    results::Results,
    services::{CloseFuture, Domain, NativeServices, Payload, Reply, Services},
    sessions::{BlackboardFactory, SessionLimits, Sessions, SpecialWorkflow},
    store::SessionStore,
};
use reqwest::Method;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, RwLock, atomic::AtomicBool},
    time::{Duration, Instant},
};
use tokio::net::TcpListener;

struct StubDomain;
impl Domain for StubDomain {
    fn execute<'a>(&'a self, _operation: &'a str, _request: Value) -> Reply<'a> {
        Box::pin(async { Ok(Payload::Json(json!({}))) })
    }
}
struct UnusedSpecial;
impl SpecialWorkflow for UnusedSpecial {
    fn run<'a>(
        &'a self,
        _session: Value,
        _prompt: String,
        _generation: String,
        _cancel: Arc<AtomicBool>,
        _emit: infra_harness::sessions::Event,
    ) -> infra_harness::sessions::RunReply<'a> {
        Box::pin(async { Ok(json!({})) })
    }
}
impl BlackboardFactory for UnusedSpecial {
    fn for_session(
        &self,
        _session: &Value,
    ) -> infra_harness::Result<Arc<dyn infra_harness::fleet::Blackboard>> {
        unreachable!()
    }
}

/// A Claude Code stand-in: like Claude Code, its initialize answer reports auto
/// mode either way, and only an explicit request for auto mode is refused
/// where auto mode is unavailable. It asks once to run `touch probe.txt` and
/// reports what it was told. In a one-shot launch it prints its result at once.
fn fake_claude(root: &Path, auto_available: bool) -> (PathBuf, PathBuf) {
    let program = root.join("fake-claude");
    let arguments = root.join("arguments.txt");
    let started = if auto_available { "auto" } else { "default" };
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$@" > "{arguments}"
case " $* " in
  *" --input-format stream-json "*) ;;
  *)
    cat >/dev/null
    echo '{{"type":"system","subtype":"init","session_id":"native-perm-1"}}'
    echo '{{"type":"result","subtype":"success","is_error":false,"result":"Ran without asking.","session_id":"native-perm-1","total_cost_usd":0.01}}'
    exit 0;;
esac
while IFS= read -r line; do
  case "$line" in
    *'"subtype":"initialize"'*)
      echo '{{"type":"control_response","response":{{"subtype":"success","request_id":"harness-initialize","response":{{"current_permission_mode":"auto"}}}}}}';;
    *'"subtype":"set_permission_mode","mode":"auto"'*)
      if [ "{started}" = auto ]; then
        echo '{{"type":"control_response","response":{{"subtype":"success","request_id":"harness-auto-mode","response":{{"mode":"auto"}}}}}}'
      else
        echo '{{"type":"control_response","response":{{"subtype":"error","request_id":"harness-auto-mode","error":"Cannot set permission mode to auto: auto mode unavailable for this model"}}}}'
      fi;;
    *'"subtype":"set_permission_mode"'*)
      echo '{{"type":"control_response","response":{{"subtype":"success","request_id":"harness-permission-mode","response":{{"mode":"acceptEdits"}}}}}}';;
    *'"type":"user"'*)
      echo '{{"type":"system","subtype":"init","session_id":"native-perm-1"}}'
      echo '{{"type":"control_request","request_id":"req-1","request":{{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash","input":{{"command":"touch probe.txt"}},"description":"Create the probe file","permission_suggestions":[{{"type":"addRules","rules":[{{"toolName":"Bash","ruleContent":"touch probe.txt"}}],"behavior":"allow","destination":"localSettings"}}]}}}}';;
    *'"behavior":"allow"'*)
      case "$line" in *'"destination":"session"'*) answer="allowed for the session";; *) answer="allowed";; esac
      echo "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"The person $answer it.\"}}]}},\"session_id\":\"native-perm-1\"}}"
      echo "{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"The person $answer it.\",\"session_id\":\"native-perm-1\",\"total_cost_usd\":0.01}}";;
    *'"behavior":"deny"'*)
      echo '{{"type":"result","subtype":"success","is_error":false,"result":"The request was declined.","session_id":"native-perm-1","total_cost_usd":0.01}}';;
  esac
done
"#,
        arguments = arguments.display(),
    );
    fs::write(&program, script).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    (program, arguments)
}

/// A Codex stand-in: `exec` prints a finished turn at once; `app-server`
/// starts a thread, asks once to run `touch probe.txt` and finishes the turn
/// with what it was told. Like an app server, it keeps running after its
/// input closes, so the Harness has to stop it.
fn fake_codex(root: &Path) -> (PathBuf, PathBuf) {
    let program = root.join("fake-codex");
    let arguments = root.join("codex-arguments.txt");
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$@" > "{arguments}"
case " $* " in
  *" app-server "*) ;;
  *)
    cat >/dev/null
    echo '{{"type":"thread.started","thread_id":"0199-fake-thread"}}'
    echo '{{"type":"turn.started"}}'
    echo '{{"type":"item.completed","item":{{"id":"item-0","type":"agent_message","text":"Ran without asking."}}}}'
    echo '{{"type":"turn.completed","usage":{{"input_tokens":10,"cached_input_tokens":0,"output_tokens":5}}}}'
    exit 0;;
esac
finish() {{
  echo "{{\"method\":\"item/completed\",\"params\":{{\"threadId\":\"0199-fake-thread\",\"turnId\":\"turn-1\",\"item\":{{\"type\":\"commandExecution\",\"id\":\"item-1\",\"command\":\"touch probe.txt\",\"status\":\"$1\",\"exitCode\":0}}}}}}"
  echo "{{\"method\":\"item/completed\",\"params\":{{\"threadId\":\"0199-fake-thread\",\"turnId\":\"turn-1\",\"item\":{{\"type\":\"agentMessage\",\"id\":\"item-2\",\"text\":\"$2\"}}}}}}"
  echo '{{"method":"thread/tokenUsage/updated","params":{{"threadId":"0199-fake-thread","turnId":"turn-1","tokenUsage":{{"total":{{"inputTokens":40,"cachedInputTokens":0,"outputTokens":8,"reasoningOutputTokens":0,"totalTokens":48}},"last":{{"inputTokens":40,"cachedInputTokens":0,"outputTokens":8,"reasoningOutputTokens":0,"totalTokens":48}}}}}}}}'
  echo '{{"method":"turn/completed","params":{{"threadId":"0199-fake-thread","turn":{{"id":"turn-1","status":"completed"}}}}}}'
}}
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/^{{"id":\([0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      echo "{{\"id\":$id,\"result\":{{\"userAgent\":\"fake\"}}}}";;
    *'"method":"thread/start"'*|*'"method":"thread/resume"'*)
      printf '%s\n' "$line" > "{thread}"
      echo "{{\"id\":$id,\"result\":{{\"thread\":{{\"id\":\"0199-fake-thread\"}}}}}}";;
    *'"method":"turn/start"'*)
      echo "{{\"id\":$id,\"result\":{{\"turn\":{{\"id\":\"turn-1\"}}}}}}"
      echo '{{"method":"turn/started","params":{{"threadId":"0199-fake-thread","turn":{{"id":"turn-1"}}}}}}'
      echo '{{"method":"item/started","params":{{"threadId":"0199-fake-thread","turnId":"turn-1","item":{{"type":"commandExecution","id":"item-1","command":"touch probe.txt","status":"inProgress"}}}}}}'
      echo '{{"id":7,"method":"item/commandExecution/requestApproval","params":{{"threadId":"0199-fake-thread","turnId":"turn-1","itemId":"item-1","command":"touch probe.txt","reason":"Create the probe file","startedAtMs":1}}}}';;
    *'"id":7,'*'"decision":"accept"'*) finish completed "The person allowed it.";;
    *'"id":7,'*'"decision":"decline"'*) finish declined "The request was declined.";;
  esac
done
sleep 30
"#,
        arguments = arguments.display(),
        thread = root.join("codex-thread.json").display(),
    );
    fs::write(&program, script).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    (program, arguments)
}

/// A Cursor stand-in: print mode prints a result at once; `acp` opens a
/// session, asks once to run `touch probe.txt` and ends the turn.
fn fake_cursor(root: &Path) -> (PathBuf, PathBuf) {
    let program = root.join("fake-cursor");
    let arguments = root.join("cursor-arguments.txt");
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$@" > "{arguments}"
case " $* " in
  *" acp "*) ;;
  *)
    echo '{{"type":"system","subtype":"init","session_id":"chat-fake-1"}}'
    echo '{{"type":"result","subtype":"success","is_error":false,"result":"Ran without asking.","session_id":"chat-fake-1"}}'
    exit 0;;
esac
prompt=0
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/^{{"jsonrpc":"2.0","id":\([0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"protocolVersion\":1}}}}";;
    *'"method":"session/new"'*|*'"method":"session/load"'*)
      echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"sessionId\":\"chat-fake-1\"}}}}";;
    *'"method":"session/prompt"'*)
      prompt=$id
      echo '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"chat-fake-1","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"Creating the probe."}}}}}}}}'
      echo '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"chat-fake-1","update":{{"sessionUpdate":"tool_call","toolCallId":"call-1","title":"touch probe.txt","kind":"execute","status":"pending","rawInput":{{"command":"touch probe.txt"}}}}}}}}'
      echo '{{"jsonrpc":"2.0","id":0,"method":"session/request_permission","params":{{"sessionId":"chat-fake-1","toolCall":{{"toolCallId":"call-1","title":"touch probe.txt","kind":"execute","rawInput":{{"command":"touch probe.txt"}}}},"options":[{{"optionId":"allow-once","name":"Allow once","kind":"allow_once"}},{{"optionId":"allow-always","name":"Always allow","kind":"allow_always"}},{{"optionId":"reject-once","name":"Reject","kind":"reject_once"}}]}}}}';;
    *'"optionId":"allow-once"'*|*'"optionId":"reject-once"'*)
      # Cursor reports a declined call completed too.
      case "$line" in *allow-once*) said="The person allowed it.";; *) said="The request was declined.";; esac
      echo '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"chat-fake-1","update":{{"sessionUpdate":"tool_call_update","toolCallId":"call-1","status":"completed"}}}}}}'
      echo "{{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{{\"sessionId\":\"chat-fake-1\",\"update\":{{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{{\"type\":\"text\",\"text\":\"$said\"}}}}}}}}"
      echo "{{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{{\"stopReason\":\"end_turn\"}}}}";;
  esac
done
"#,
        arguments = arguments.display(),
    );
    fs::write(&program, script).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    (program, arguments)
}

struct Fixture {
    _dir: tempfile::TempDir,
    project_id: String,
    store: SessionStore,
    services: Arc<NativeServices>,
    port: u16,
    token: String,
    shutdown: tokio::sync::watch::Sender<bool>,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    arguments: PathBuf,
    codex_arguments: PathBuf,
    cursor_arguments: PathBuf,
    root: PathBuf,
    patience: Duration,
}
impl Fixture {
    async fn new(auto_available: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let (claude, arguments) = fake_claude(&root, auto_available);
        let (codex, codex_arguments) = fake_codex(&root);
        let (cursor, cursor_arguments) = fake_cursor(&root);
        Self::with_hosts(
            dir,
            [claude, codex, cursor],
            [arguments, codex_arguments, cursor_arguments],
            Duration::from_secs(20),
        )
        .await
    }
    async fn with_hosts(
        dir: tempfile::TempDir,
        [claude, codex, cursor]: [PathBuf; 3],
        [arguments, codex_arguments, cursor_arguments]: [PathBuf; 3],
        patience: Duration,
    ) -> Self {
        let _ = infra_host::process::use_guardian(PathBuf::from(env!(
            "CARGO_BIN_EXE_infra-harness-inbox-fixture"
        )));
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let project = root.join("project");
        let source = root.join("source");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&source).unwrap();
        let store = SessionStore::open(&root.join("state")).unwrap();
        let project_id = store.register(&project).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let fake = |id: &str, executable: PathBuf| {
            (
                id.to_owned(),
                Provider {
                    id: id.into(),
                    name: format!("Fake {id}"),
                    available: true,
                    executable: Some(executable),
                    detail: "Test fixture".into(),
                },
            )
        };
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::from([
                fake("claude", claude),
                fake("codex", codex),
                fake("cursor", cursor),
            ]))),
        });
        static CATALOG: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
        CATALOG
            .get_or_init(|| async {
                for id in ["claude", "codex", "cursor"] {
                    infra_host::model_catalog::model_options(id, None, None)
                        .await
                        .unwrap();
                }
            })
            .await;
        let knowledge = Arc::new(KnowledgeDomain::new(store.clone(), source.clone()));
        let special = ["creator", "system_run", "system_discovery"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    Arc::new(UnusedSpecial) as Arc<dyn SpecialWorkflow>,
                )
            })
            .collect();
        let sessions = Sessions::new(
            store.clone(),
            source.clone(),
            providers.clone(),
            knowledge.clone(),
            Arc::new(UnusedSpecial),
            special,
            SessionLimits {
                seconds: patience.as_secs().max(30),
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
        sessions.install_results(results.clone()).unwrap();
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
        let stub: Arc<dyn Domain> = Arc::new(StubDomain);
        let mut domains: BTreeMap<String, Arc<dyn Domain>> = [
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
        ]
        .into_iter()
        .map(|name| (name.to_owned(), stub.clone()))
        .collect();
        domains.insert("knowledge".into(), knowledge);
        for name in ["sessions", "brain", "context"] {
            domains.insert(name.into(), sessions.clone());
        }
        domains.insert(
            "results".into(),
            Arc::new(infra_harness::results::ResultService { results }),
        );
        let boot = store.clone();
        let closing = sessions.clone();
        let services = Arc::new(
            NativeServices::new(
                store.clone(),
                domains,
                Arc::new(move || {
                    Ok(json!({"projects": boot.projects()?, "sessions": boot.summaries()?}))
                }),
                BTreeMap::from([(
                    "memory.save".to_owned(),
                    sessions.clone() as Arc<dyn Domain>,
                )]),
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (shutdown, _) = tokio::sync::watch::channel(false);
        let token = "harness-permissions-test-token".to_owned();
        let app = App {
            services: services.clone(),
            token: token.clone(),
            instance: "test-instance".into(),
            port,
            source,
            shutdown: shutdown.clone(),
            assets: Arc::new(BTreeMap::new()),
        };
        let server = tokio::spawn(async move {
            let _ = http::serve_listener(listener, app).await;
        });
        Self {
            _dir: dir,
            project_id,
            store,
            services,
            port,
            token,
            shutdown,
            server,
            client: reqwest::Client::new(),
            arguments,
            codex_arguments,
            cursor_arguments,
            root,
            patience,
        }
    }
    async fn http(&self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut request = self
            .client
            .request(method, format!("http://127.0.0.1:{}{path}", self.port))
            .header("x-harness-token", &self.token);
        if let Some(body) = body {
            let raw = body.to_string();
            request = request
                .header("content-type", "application/json")
                .header("content-length", raw.len())
                .body(raw);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let bytes = response.bytes().await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
    async fn start(&self, permissions: &str) -> String {
        self.start_with("claude", permissions).await
    }
    async fn start_with(&self, provider: &str, permissions: &str) -> String {
        self.start_task(provider, permissions, None, "Create the probe file")
            .await
    }
    async fn start_task(
        &self,
        provider: &str,
        permissions: &str,
        model: Option<&str>,
        prompt: &str,
    ) -> String {
        let (status, created) = self
            .http(
                Method::POST,
                "/api/sessions",
                Some(json!({
                    "project_id": self.project_id,
                    "provider": provider,
                    "mode": "edit",
                    "workflow": "native",
                    "project_context": false,
                    "permissions": permissions,
                    "model": model,
                    "prompt": prompt,
                })),
            )
            .await;
        assert_eq!(status, 201, "{created}");
        assert_eq!(created["session"]["permissions"], permissions);
        created["session"]["id"].as_str().unwrap().to_owned()
    }
    fn events(&self, sid: &str) -> Vec<Value> {
        self.store.events(sid, 0).unwrap()
    }
    fn project(&self) -> PathBuf {
        self.root.join("project")
    }
    async fn until(
        &self,
        sid: &str,
        what: &str,
        done: impl Fn(&Value, &[Value]) -> bool,
    ) -> Vec<Value> {
        let deadline = Instant::now() + self.patience;
        loop {
            let session = self.store.get(sid).unwrap();
            let events = self.events(sid);
            if done(&session, &events) {
                return events;
            }
            assert!(
                Instant::now() < deadline,
                "{what} never happened: {session}\n{events:#?}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    fn arguments(&self) -> Vec<String> {
        lines(&self.arguments)
    }
    async fn answer(&self, sid: &str, request: &str, decision: &str) -> (u16, Value) {
        self.http(
            Method::POST,
            &format!("/api/sessions/{sid}/permissions/{request}"),
            Some(json!({"decision": decision})),
        )
        .await
    }
    async fn follow(&self, sid: &str, permissions: &str) {
        let (status, sent) = self
            .http(
                Method::POST,
                &format!("/api/sessions/{sid}/messages"),
                Some(json!({"prompt": "Once more", "permissions": permissions})),
            )
            .await;
        assert_eq!(status, 200, "{sent}");
        let turns = self
            .events(sid)
            .iter()
            .filter(|event| event["kind"] == "user")
            .count();
        self.until(sid, "the next turn", |session, events| {
            finished(session)
                && events
                    .iter()
                    .filter(|event| event["kind"] == "result")
                    .count()
                    >= turns
        })
        .await;
        assert!(completed(&self.store.get(sid).unwrap()));
    }
    /// Sends the session's next turn without waiting for it.
    async fn send(&self, sid: &str, permissions: &str, prompt: &str) {
        let (status, sent) = self
            .http(
                Method::POST,
                &format!("/api/sessions/{sid}/messages"),
                Some(json!({"prompt": prompt, "permissions": permissions})),
            )
            .await;
        assert_eq!(status, 200, "{sent}");
    }
    async fn close(self) {
        let _ = self.shutdown.send(true);
        let _ = self.services.close().await;
        let _ = self.server.await;
    }
}
fn lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(String::from)
        .collect()
}
fn permission_events<'a>(events: &'a [Value], state: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|event| event["kind"] == "permission" && event["state"] == state)
        .collect()
}
fn completed(session: &Value) -> bool {
    session["status"] == "completed"
}
fn finished(session: &Value) -> bool {
    ["completed", "failed", "interrupted", "cancelled"]
        .contains(&session["status"].as_str().unwrap_or(""))
}
fn pair(arguments: &[String], flag: &str) -> Option<String> {
    arguments
        .iter()
        .position(|argument| argument == flag)
        .and_then(|index| arguments.get(index + 1).cloned())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_manual_session_waits_for_the_persons_answer() {
    let fx = Fixture::new(false).await;
    let sid = fx.start("manual").await;
    let events = fx
        .until(&sid, "the permission request", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let pending = permission_events(&events, "pending")[0].clone();
    // Each run tags the host's id, which hosts reuse from run to run.
    let request = pending["request_id"].as_str().unwrap().to_owned();
    assert!(request.starts_with("claude-req-1-"), "{request}");
    assert_eq!(pending["tool"], "Bash");
    assert_eq!(pending["summary"], "touch probe.txt");
    assert_eq!(pending["detail"], "Create the probe file");
    assert_eq!(pending["session_option"], true);
    // The run waits; nothing finishes without the answer.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fx.store.get(&sid).unwrap()["status"], "running");
    let arguments = fx.arguments();
    assert_eq!(
        pair(&arguments, "--permission-mode").as_deref(),
        Some("default")
    );
    assert_eq!(
        pair(&arguments, "--input-format").as_deref(),
        Some("stream-json")
    );
    assert_eq!(
        pair(&arguments, "--permission-prompt-tool").as_deref(),
        Some("stdio")
    );

    // A request that does not exist, or a decision that is not one, is refused.
    let path = format!("/api/sessions/{sid}/permissions/{request}");
    let (status, _) = fx
        .http(
            Method::POST,
            &format!("/api/sessions/{sid}/permissions/other"),
            Some(json!({"decision": "allow"})),
        )
        .await;
    assert_eq!(status, 409);
    let (status, _) = fx
        .http(Method::POST, &path, Some(json!({"decision": "accept"})))
        .await;
    assert_eq!(status, 400);
    let (status, answer) = fx
        .http(
            Method::POST,
            &path,
            Some(json!({"decision": "allow_session"})),
        )
        .await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["permission"]["state"], "allowed_session");
    assert_eq!(answer["session"]["id"], sid.as_str());
    let events = fx
        .until(&sid, "the answered run", |session, _| finished(session))
        .await;
    assert!(completed(&fx.store.get(&sid).unwrap()), "{events:#?}");
    let resolved = permission_events(&events, "allowed_session");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0]["by"], "person");
    // The host heard a session-scoped approval and acted on it.
    assert!(
        events.iter().any(|event| event["kind"] == "result"
            && event["text"] == "The person allowed for the session it."),
        "{events:#?}"
    );
    // Answering again finds nothing waiting.
    let (status, _) = fx
        .http(Method::POST, &path, Some(json!({"decision": "deny"})))
        .await;
    assert_eq!(status, 409);
    fx.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_auto_session_falls_back_to_accepting_edits_and_declines_on_its_own() {
    let fx = Fixture::new(false).await;
    let sid = fx.start("auto").await;
    let events = fx
        .until(&sid, "the auto run", |session, _| finished(session))
        .await;
    assert!(completed(&fx.store.get(&sid).unwrap()), "{events:#?}");
    let arguments = fx.arguments();
    assert_eq!(
        pair(&arguments, "--permission-mode").as_deref(),
        Some("auto")
    );
    assert_eq!(
        pair(&arguments, "--input-format").as_deref(),
        Some("stream-json")
    );
    // Auto mode was unavailable: the run says so and accepts edits instead.
    assert!(
        events.iter().any(|event| event["kind"] == "status"
            && event["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("Auto mode is unavailable"))),
        "{events:#?}"
    );
    // Nobody was asked: the request was declined automatically.
    assert!(permission_events(&events, "pending").is_empty());
    let declined = permission_events(&events, "declined");
    assert_eq!(declined.len(), 1);
    assert_eq!(declined[0]["by"], "auto");
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "result" && event["text"] == "The request was declined.")
    );
    fx.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bypass_runs_one_shot_without_asking_and_plan_stays_read_only() {
    let fx = Fixture::new(true).await;
    let sid = fx.start("bypass").await;
    let events = fx
        .until(&sid, "the bypass run", |session, _| finished(session))
        .await;
    assert!(completed(&fx.store.get(&sid).unwrap()), "{events:#?}");
    let arguments = fx.arguments();
    assert_eq!(
        pair(&arguments, "--permission-mode").as_deref(),
        Some("bypassPermissions")
    );
    assert!(arguments.contains(&"--allow-dangerously-skip-permissions".to_owned()));
    assert!(!arguments.contains(&"--input-format".to_owned()));
    assert!(events.iter().all(|event| event["kind"] != "permission"));

    // A follow-up may change the setting; an unknown one is refused.
    let (status, refused) = fx
        .http(
            Method::POST,
            &format!("/api/sessions/{sid}/messages"),
            Some(json!({"prompt": "Again", "permissions": "everything"})),
        )
        .await;
    assert_eq!(status, 400, "{refused}");
    let (status, sent) = fx
        .http(
            Method::POST,
            &format!("/api/sessions/{sid}/messages"),
            Some(json!({"prompt": "Again, with Auto", "permissions": "auto"})),
        )
        .await;
    assert_eq!(status, 200, "{sent}");
    assert_eq!(sent["session"]["permissions"], "auto");
    let events = fx
        .until(&sid, "the second turn", |session, events| {
            finished(session)
                && events
                    .iter()
                    .filter(|event| event["kind"] == "result")
                    .count()
                    >= 2
        })
        .await;
    assert_eq!(fx.store.get(&sid).unwrap()["permissions"], "auto");
    let arguments = fx.arguments();
    assert_eq!(
        pair(&arguments, "--permission-mode").as_deref(),
        Some("auto")
    );
    // This host has auto mode: nothing falls back, and the request is declined.
    assert!(!events.iter().any(|event| {
        event["kind"] == "status"
            && event["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("Auto mode is unavailable"))
    }));
    assert_eq!(permission_events(&events, "declined").len(), 1);

    // A planning turn stays read-only whatever the setting.
    let (status, plan) = fx
        .http(
            Method::POST,
            "/api/sessions",
            Some(json!({
                "project_id": fx.project_id,
                "provider": "claude",
                "mode": "plan",
                "workflow": "native",
                "project_context": false,
                "permissions": "bypass",
                "prompt": "Only look",
            })),
        )
        .await;
    assert_eq!(status, 201, "{plan}");
    let planned = plan["session"]["id"].as_str().unwrap().to_owned();
    fx.until(&planned, "the planning run", |session, _| finished(session))
        .await;
    let arguments = fx.arguments();
    assert_eq!(
        pair(&arguments, "--permission-mode").as_deref(),
        Some("plan")
    );
    assert!(!arguments.contains(&"--allow-dangerously-skip-permissions".to_owned()));
    fx.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codex_asks_through_its_app_server_and_is_stopped_after_the_turn() {
    let fx = Fixture::new(false).await;
    let sid = fx.start_with("codex", "manual").await;
    let events = fx
        .until(&sid, "the Codex request", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let pending = permission_events(&events, "pending")[0].clone();
    let request = pending["request_id"].as_str().unwrap().to_owned();
    assert!(request.starts_with("codex-7-"), "{request}");
    assert_eq!(pending["tool"], "Shell");
    assert_eq!(pending["summary"], "touch probe.txt");
    assert_eq!(pending["session_option"], true);
    let arguments = lines(&fx.codex_arguments);
    assert_eq!(arguments.last().map(String::as_str), Some("app-server"));
    assert!(!arguments.contains(&"exec".to_owned()));
    // The thread is read-only and every approval goes to the person.
    let thread: Value =
        serde_json::from_str(&fs::read_to_string(fx.root.join("codex-thread.json")).unwrap())
            .unwrap();
    assert_eq!(thread["method"], "thread/start");
    assert_eq!(thread["params"]["sandbox"], "read-only");
    assert_eq!(thread["params"]["approvalPolicy"], "on-request");
    assert_eq!(thread["params"]["approvalsReviewer"], "user");
    let answered = Instant::now();
    let (status, answer) = fx.answer(&sid, &request, "allow").await;
    assert_eq!(status, 200, "{answer}");
    let events = fx
        .until(&sid, "the Codex turn", |session, _| finished(session))
        .await;
    assert!(completed(&fx.store.get(&sid).unwrap()), "{events:#?}");
    // The server kept running after its turn; the Harness stopped it.
    assert!(answered.elapsed() < Duration::from_secs(15));
    // Codex says its answer as a message; its turn's result carries no text.
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "result" && event["ok"] == true)
    );
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "text" && event["text"] == "The person allowed it.")
    );
    let resolved = permission_events(&events, "allowed");
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        fx.store.get(&sid).unwrap()["native_session_id"],
        "0199-fake-thread"
    );

    // Auto lets Codex's reviewer answer; Bypass turns approvals and the sandbox off.
    fx.follow(&sid, "auto").await;
    let arguments = lines(&fx.codex_arguments);
    assert!(arguments.contains(&"--approve-for-me".to_owned()));
    assert!(!arguments.contains(&"--ask-for-approval".to_owned()));
    assert_eq!(arguments.iter().filter(|a| *a == "resume").count(), 1);
    fx.follow(&sid, "bypass").await;
    let arguments = lines(&fx.codex_arguments);
    assert!(arguments.contains(&"--dangerously-bypass-approvals-and-sandbox".to_owned()));
    assert!(!arguments.contains(&"--sandbox".to_owned()));
    fx.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cursor_asks_over_acp_and_offers_no_session_approval() {
    let fx = Fixture::new(false).await;
    let sid = fx.start_with("cursor", "manual").await;
    let events = fx
        .until(&sid, "the Cursor request", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let pending = permission_events(&events, "pending")[0].clone();
    let request = pending["request_id"].as_str().unwrap().to_owned();
    assert!(request.starts_with("cursor-0-"), "{request}");
    assert_eq!(pending["tool"], "Shell");
    assert_eq!(pending["summary"], "touch probe.txt");
    assert_eq!(pending["session_option"], false);
    let arguments = lines(&fx.cursor_arguments);
    assert_eq!(arguments.last().map(String::as_str), Some("acp"));
    assert!(!arguments.contains(&"--print".to_owned()));
    // Cursor keeps an always-allow beyond the session, so the Harness offers once only.
    let (status, _) = fx.answer(&sid, &request, "allow_session").await;
    assert_eq!(status, 400);
    let (status, answer) = fx.answer(&sid, &request, "deny").await;
    assert_eq!(status, 200, "{answer}");
    let events = fx
        .until(&sid, "the Cursor turn", |session, _| finished(session))
        .await;
    assert!(completed(&fx.store.get(&sid).unwrap()), "{events:#?}");
    assert_eq!(permission_events(&events, "denied").len(), 1);
    assert!(events.iter().any(|event| {
        event["kind"] == "text"
            && event["text"]
                .as_str()
                .is_some_and(|text| text.contains("The request was declined."))
    }));
    // The declined call did not run, though Cursor calls it completed.
    let shell: Vec<&Value> = events
        .iter()
        .filter(|event| event["kind"] == "tool" && event["state"] == "completed")
        .collect();
    assert!(
        !shell.is_empty() && shell.iter().all(|event| event["ok"] == false),
        "{events:#?}"
    );
    assert_eq!(
        fx.store.get(&sid).unwrap()["native_session_id"],
        "chat-fake-1"
    );
    // The next Manual turn's process numbers its request 0 again; the page
    // still gets a request of its own.
    fx.send(&sid, "manual", "Once more").await;
    let events = fx
        .until(&sid, "the next Cursor request", |_, events| {
            permission_events(events, "pending").len() == 2
        })
        .await;
    let next = permission_events(&events, "pending")[1]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(next.starts_with("cursor-0-") && next != request, "{next}");
    assert_eq!(fx.answer(&sid, &request, "allow").await.0, 409);
    let (status, answer) = fx.answer(&sid, &next, "allow").await;
    assert_eq!(status, 200, "{answer}");
    let events = fx
        .until(&sid, "the next Cursor turn", |session, events| {
            finished(session)
                && events
                    .iter()
                    .filter(|event| event["kind"] == "result")
                    .count()
                    >= 2
        })
        .await;
    assert!(completed(&fx.store.get(&sid).unwrap()), "{events:#?}");
    assert_eq!(permission_events(&events, "allowed").len(), 1);
    assert!(
        events
            .iter()
            .filter(|event| event["kind"] == "tool" && event["state"] == "completed")
            .any(|event| event["ok"] == true),
        "{events:#?}"
    );
    assert!(lines(&fx.cursor_arguments).ends_with(&["acp".to_owned()]));

    fx.follow(&sid, "bypass").await;
    let arguments = lines(&fx.cursor_arguments);
    assert!(arguments.contains(&"--force".to_owned()));
    assert_eq!(pair(&arguments, "--sandbox").as_deref(), Some("disabled"));
    fx.follow(&sid, "auto").await;
    let arguments = lines(&fx.cursor_arguments);
    assert!(!arguments.contains(&"--force".to_owned()));
    assert_eq!(pair(&arguments, "--sandbox").as_deref(), Some("enabled"));
    fx.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn waiting_permissions_surface_in_sessions_list_get_and_launch_history() {
    let fx = Fixture::new(false).await;
    let other_sid = fx.start("auto").await;
    fx.until(&other_sid, "the auto session", |session, _| {
        finished(session)
    })
    .await;

    let sid = fx.start("manual").await;
    let events = fx
        .until(&sid, "the waiting permission request", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let request_id = permission_events(&events, "pending")[0]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Direct SessionsDomain::execute operations return live waiting requests and counts.
    let Payload::Json(waiting) = fx
        .services
        .request(
            "sessions.waiting_permissions",
            json!({"params":{"session_id": sid},"query":{},"body":{}}),
        )
        .await
        .unwrap()
    else {
        panic!("expected json");
    };
    assert_eq!(waiting["items"].as_array().unwrap().len(), 1);
    assert_eq!(waiting["items"][0]["request_id"], request_id);

    let Payload::Json(counts) = fx
        .services
        .request(
            "sessions.waiting_counts",
            json!({"params":{},"query":{},"body":{}}),
        )
        .await
        .unwrap()
    else {
        panic!("expected json");
    };
    assert_eq!(counts["counts"][&sid], 1);

    // HTTP GET /api/sessions and GET /api/sessions/{sid} expose waiting counts and items.
    let (list_status, list_body) = fx.http(Method::GET, "/api/sessions", None).await;
    assert_eq!(list_status, 200, "{list_body}");
    let sessions = list_body["sessions"].as_array().unwrap();
    let waiting_summary = sessions.iter().find(|s| s["id"] == sid).unwrap();
    let idle_summary = sessions.iter().find(|s| s["id"] == other_sid).unwrap();
    assert_eq!(waiting_summary["waiting_permissions"], 1);
    assert_eq!(idle_summary["waiting_permissions"], 0);

    let (get_status, get_body) = fx
        .http(Method::GET, &format!("/api/sessions/{sid}"), None)
        .await;
    assert_eq!(get_status, 200, "{get_body}");
    let get_waiting = get_body["waiting_permissions"].as_array().unwrap();
    assert_eq!(get_waiting.len(), 1);
    assert_eq!(get_waiting[0]["request_id"], request_id);

    // Answering clears waiting permissions and launch history retains `"permissions": "manual"`.
    let (answer_status, _) = fx.answer(&sid, &request_id, "allow").await;
    assert_eq!(answer_status, 200);
    fx.until(&sid, "answered manual run", |session, _| finished(session))
        .await;

    let (list_status, list_after) = fx.http(Method::GET, "/api/sessions", None).await;
    assert_eq!(list_status, 200, "{list_after}");
    let after_summary = list_after["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == sid)
        .unwrap();
    assert_eq!(after_summary["waiting_permissions"], 0);

    let (get_status, get_after) = fx
        .http(Method::GET, &format!("/api/sessions/{sid}"), None)
        .await;
    assert_eq!(get_status, 200, "{get_after}");
    assert_eq!(get_after["waiting_permissions"], json!([]));

    let (results_status, results_body) = fx
        .http(
            Method::GET,
            &format!("/api/sessions/{sid}/results?diff=none"),
            None,
        )
        .await;
    assert_eq!(results_status, 200, "{results_body}");
    assert_eq!(
        results_body["launches"][0]["settings"]["permissions"],
        "manual"
    );

    fx.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interactive_deadline_pauses_during_human_wait_and_excludes_wait_from_seconds() {
    use infra_host::process::{Interactive, Limits, run_interactive, use_guardian};
    use std::sync::atomic::AtomicUsize;

    let _ = use_guardian(PathBuf::from(env!(
        "CARGO_BIN_EXE_infra-harness-inbox-fixture"
    )));
    let dir = tempfile::tempdir().unwrap();
    let root = infra_core::fs::canonicalize(dir.path()).unwrap();
    let script = root.join("pause-check.sh");
    fs::write(&script, "#!/bin/sh\necho ask\nIFS= read -r _\necho done\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    // 1. Waiting pauses the short deadline and is excluded from Output.seconds.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let finished = Arc::new(AtomicBool::new(false));
    let waiting = Arc::new(AtomicUsize::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let tx_holder = Arc::new(std::sync::Mutex::new(Some(tx)));
    let started = Instant::now();
    let deadline = started + Duration::from_millis(350);
    let lines_seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let lines_event = lines_seen.clone();
    let waiting_event = waiting.clone();
    let finished_event = finished.clone();
    let tx_event = tx_holder.clone();
    let output = run_interactive(
        &[script.to_string_lossy().into_owned()],
        &root,
        &BTreeMap::new(),
        &[],
        &[],
        Limits {
            seconds: Some(1),
            output: 65536,
            line: 65536,
        },
        cancel.clone(),
        None,
        None,
        Some(deadline),
        Interactive {
            input: rx,
            finished,
            waiting,
        },
        move |packet| {
            let text = String::from_utf8_lossy(packet).trim().to_owned();
            lines_event.lock().unwrap().push(text.clone());
            if text == "ask" {
                waiting_event.store(1, std::sync::atomic::Ordering::Release);
                let w = waiting_event.clone();
                let f = finished_event.clone();
                let t = tx_event.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(650)).await;
                    w.store(0, std::sync::atomic::Ordering::Release);
                    if let Some(sender) = t.lock().unwrap().take() {
                        let _ = sender.send(b"ok\n".to_vec());
                    }
                    f.store(true, std::sync::atomic::Ordering::Release);
                });
            }
            Ok(())
        },
    )
    .await
    .unwrap();
    let wall = started.elapsed();
    assert_eq!(output.code, 0);
    assert_eq!(
        *lines_seen.lock().unwrap(),
        vec!["ask".to_owned(), "done".to_owned()]
    );
    assert!(wall >= Duration::from_millis(650), "{wall:?}");
    assert!(
        output.seconds < 0.45,
        "paused wait must not count toward Output.seconds: {} (wall={wall:?})",
        output.seconds
    );

    // 2. Without waiting, an expired deadline still fails with native-timeout.
    let (_tx2, rx2) = tokio::sync::mpsc::unbounded_channel();
    let err = run_interactive(
        &[script.to_string_lossy().into_owned()],
        &root,
        &BTreeMap::new(),
        &[],
        &[],
        Limits {
            seconds: Some(10),
            output: 65536,
            line: 65536,
        },
        Arc::new(AtomicBool::new(false)),
        None,
        None,
        Some(Instant::now() + Duration::from_millis(250)),
        Interactive {
            input: rx2,
            finished: Arc::new(AtomicBool::new(false)),
            waiting: Arc::new(AtomicUsize::new(0)),
        },
        |_| Ok(()),
    )
    .await
    .err()
    .expect("unpaused deadline must time out");
    assert_eq!(err.code, "native-timeout");
}

/// Live checks against the real Claude Code, Codex and Cursor on PATH, through
/// the same routes. They make paid model calls, so they run only when asked:
/// `HARNESS_LIVE=1 cargo test -p infra-harness --test permission_modes -- --ignored`.
/// Only the hosts a check names must be installed.
fn live(wanted: &[&str]) -> Option<(tempfile::TempDir, [PathBuf; 3])> {
    if std::env::var("HARNESS_LIVE").as_deref() != Ok("1") {
        eprintln!("Set HARNESS_LIVE=1 to run the live permission checks.");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = infra_core::fs::canonicalize(dir.path()).unwrap();
    let host = |provider: &str, name: &str| {
        if !wanted.contains(&provider) {
            return root.join(format!("no-{provider}"));
        }
        infra_host::provider::find_executable(name)
            .map(|path| infra_core::fs::canonicalize(path).unwrap())
            .unwrap_or_else(|| panic!("{name} is not on PATH"))
    };
    // The Harness looks for cursor-agent before agent, a name other tools use too.
    let hosts = [
        host("claude", "claude"),
        host("codex", "codex"),
        host("cursor", "cursor-agent"),
    ];
    Some((dir, hosts))
}
/// A debug build's session run needs more than Tokio's default worker stack.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_stack_size(32 << 20)
        .enable_all()
        .build()
        .unwrap()
}
async fn live_fixture(dir: tempfile::TempDir, hosts: [PathBuf; 3]) -> Fixture {
    let root = infra_core::fs::canonicalize(dir.path()).unwrap();
    let unused = root.join("unused");
    let fx = Fixture::with_hosts(
        dir,
        hosts,
        [unused.clone(), unused.clone(), unused],
        Duration::from_secs(300),
    )
    .await;
    // Codex's exec refuses a folder outside Git.
    let status = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(fx.project())
        .status()
        .unwrap();
    assert!(status.success());
    fx
}
/// Answers every request the session asks until its run ends.
async fn answer_all(fx: &Fixture, sid: &str, decision: &str) -> Vec<Value> {
    answer_turns(fx, sid, decision, 1).await
}
/// Answers every request until the session's `turns`-th turn has ended.
async fn answer_turns(fx: &Fixture, sid: &str, decision: &str, turns: usize) -> Vec<Value> {
    let deadline = Instant::now() + fx.patience;
    let mut answered = std::collections::BTreeSet::new();
    loop {
        let session = fx.store.get(sid).unwrap();
        let events = fx.events(sid);
        let results = events
            .iter()
            .filter(|event| event["kind"] == "result")
            .count();
        if finished(&session) && (results >= turns || !completed(&session)) {
            return events;
        }
        let resolved: std::collections::BTreeSet<&str> = events
            .iter()
            .filter(|event| event["kind"] == "permission" && event["state"] != "pending")
            .filter_map(|event| event["request_id"].as_str())
            .collect();
        for request in permission_events(&events, "pending") {
            let id = request["request_id"].as_str().unwrap().to_owned();
            if resolved.contains(id.as_str()) {
                continue;
            }
            if answered.insert(id.clone()) {
                eprintln!(
                    "{sid}: {} {} → {decision}",
                    request["tool"], request["summary"]
                );
                let (status, answer) = fx.answer(sid, &id, decision).await;
                assert!(status == 200 || status == 409, "{answer}");
            }
        }
        assert!(
            Instant::now() < deadline,
            "the live run never ended: {session}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
fn said(events: &[Value]) -> String {
    events
        .iter()
        .filter(|event| event["kind"] == "text")
        .filter_map(|event| event["text"].as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

#[test]
#[ignore = "makes paid model calls; set HARNESS_LIVE=1"]
fn live_claude_code_asks_declines_and_bypasses() {
    let Some((dir, hosts)) = live(&["claude"]) else {
        return;
    };
    runtime().block_on(async move {
        let fx = live_fixture(dir, hosts).await;
        let probe = |name: &str| fx.project().join(name);
        let task = |file: &str| {
            format!("Use the Bash tool to run exactly this command and nothing else: touch {file}\nThen reply with the single word done.")
        };
        // Manual: the person allows, and the command runs.
        let sid = fx
            .start_task("claude", "manual", Some("haiku"), &task("probe-manual.txt"))
            .await;
        let events = answer_all(&fx, &sid, "allow").await;
        assert!(completed(&fx.store.get(&sid).unwrap()), "{events:#?}");
        let asked = permission_events(&events, "allowed");
        assert!(!asked.is_empty(), "{events:#?}");
        assert!(asked.iter().any(|event| event["tool"] == "Bash"
            && event["summary"].as_str().unwrap().contains("probe-manual.txt")));
        assert!(probe("probe-manual.txt").exists(), "{}", said(&events));
        // Manual: the person denies, and nothing runs.
        let sid = fx
            .start_task("claude", "manual", Some("haiku"), &task("probe-denied.txt"))
            .await;
        let events = answer_all(&fx, &sid, "deny").await;
        assert!(!permission_events(&events, "denied").is_empty(), "{events:#?}");
        assert!(!probe("probe-denied.txt").exists());
        // Auto on a model without auto mode: edits are accepted, the rest declined.
        let sid = fx
            .start_task(
                "claude",
                "auto",
                Some("haiku"),
                "Use the Write tool to create the file probe-auto.txt containing the word hi. Then use the Bash tool to run exactly: python3 -c \"open('probe-auto-bash.txt','w').write('x')\"\nThen reply with the single word done.",
            )
            .await;
        let events = answer_all(&fx, &sid, "allow").await;
        assert!(permission_events(&events, "pending").is_empty(), "{events:#?}");
        assert!(events.iter().any(|event| event["kind"] == "status"
            && event["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("Auto mode is unavailable"))), "{events:#?}");
        assert!(probe("probe-auto.txt").exists(), "{}", said(&events));
        // Accepting edits covers file commands such as touch, not running a
        // program: that needed a person and was declined.
        assert!(!permission_events(&events, "declined").is_empty(), "{events:#?}");
        assert!(!probe("probe-auto-bash.txt").exists(), "{}", said(&events));
        // Bypass: the command runs without anyone being asked.
        let sid = fx
            .start_task("claude", "bypass", Some("haiku"), &task("probe-bypass.txt"))
            .await;
        let events = answer_all(&fx, &sid, "deny").await;
        assert!(events.iter().all(|event| event["kind"] != "permission"), "{events:#?}");
        assert!(probe("probe-bypass.txt").exists(), "{}", said(&events));
        fx.close().await;
    });
}

#[test]
#[ignore = "makes paid model calls; set HARNESS_LIVE=1"]
fn live_codex_asks_reviews_and_bypasses() {
    let Some((dir, hosts)) = live(&["codex"]) else {
        return;
    };
    runtime().block_on(async move {
        let fx = live_fixture(dir, hosts).await;
        let probe = |name: &str| fx.project().join(name);
        let task = |file: &str| {
            format!("Create a file named {file} in the current folder containing the word hi. Make no other change, then stop.")
        };
        // Manual: the read-only thread asks, the person allows.
        let sid = fx
            .start_task("codex", "manual", None, &task("probe-codex-manual.txt"))
            .await;
        let events = answer_all(&fx, &sid, "allow").await;
        assert!(completed(&fx.store.get(&sid).unwrap()), "{events:#?}");
        assert!(!permission_events(&events, "allowed").is_empty(), "{events:#?}");
        assert!(probe("probe-codex-manual.txt").exists(), "{}", said(&events));
        // Auto: the workspace sandbox writes without asking anyone.
        let sid = fx
            .start_task("codex", "auto", None, &task("probe-codex-auto.txt"))
            .await;
        let events = answer_all(&fx, &sid, "deny").await;
        assert!(permission_events(&events, "pending").is_empty(), "{events:#?}");
        assert!(probe("probe-codex-auto.txt").exists(), "{}", said(&events));
        // Bypass: no sandbox, no approvals.
        let sid = fx
            .start_task("codex", "bypass", None, &task("probe-codex-bypass.txt"))
            .await;
        let events = answer_all(&fx, &sid, "deny").await;
        assert!(events.iter().all(|event| event["kind"] != "permission"), "{events:#?}");
        assert!(probe("probe-codex-bypass.txt").exists(), "{}", said(&events));
        fx.close().await;
    });
}

#[test]
#[ignore = "makes paid model calls; set HARNESS_LIVE=1"]
fn live_cursor_asks_over_acp_resumes_and_bypasses() {
    let Some((dir, hosts)) = live(&["cursor"]) else {
        return;
    };
    runtime().block_on(async move {
        let fx = live_fixture(dir, hosts).await;
        let probe = |name: &str| fx.project().join(name);
        let task = |file: &str| {
            format!("Use your shell tool to run exactly this command and nothing else: touch {file}\nThen reply with the single word done.")
        };
        // Manual: Cursor asks over ACP, the person allows, and the command runs.
        let sid = fx
            .start_task("cursor", "manual", None, &task("probe-cursor-manual.txt"))
            .await;
        let events = answer_all(&fx, &sid, "allow").await;
        assert!(completed(&fx.store.get(&sid).unwrap()), "{events:#?}");
        let asked = permission_events(&events, "allowed");
        assert!(asked.iter().any(|event| event["tool"] == "Shell"
            && event["summary"].as_str().unwrap().contains("probe-cursor-manual.txt")), "{events:#?}");
        assert!(probe("probe-cursor-manual.txt").exists(), "{}", said(&events));
        let shell = |events: &[Value], ok: bool| {
            events.iter().any(|event| {
                event["kind"] == "tool"
                    && event["tool"] == "Shell"
                    && event["state"] == "completed"
                    && event["ok"] == ok
            })
        };
        assert!(shell(&events, true) && !shell(&events, false), "{events:#?}");
        let chat = fx.store.get(&sid).unwrap()["native_session_id"].clone();
        assert!(chat.is_string(), "{events:#?}");
        // The next turn loads the same chat over ACP; the person denies.
        let before = events.len();
        fx.send(&sid, "manual", &task("probe-cursor-denied.txt")).await;
        let events = answer_turns(&fx, &sid, "deny", 2).await;
        assert!(completed(&fx.store.get(&sid).unwrap()), "{events:#?}");
        let turn = &events[before..];
        assert!(!permission_events(turn, "denied").is_empty(), "{turn:#?}");
        assert!(permission_events(turn, "denied").iter().all(|denied| {
            asked.iter().all(|allowed| allowed["request_id"] != denied["request_id"])
        }));
        assert!(!probe("probe-cursor-denied.txt").exists(), "{}", said(turn));
        // Cursor calls the declined command completed; it did not run.
        assert!(shell(turn, false) && !shell(turn, true), "{turn:#?}");
        assert_eq!(fx.store.get(&sid).unwrap()["native_session_id"], chat);
        // The loaded chat's replayed history is not shown again: what the
        // agent said and ran this turn is this turn's alone.
        assert!(
            turn.iter()
                .filter(|event| ["text", "tool", "permission"].contains(&event["kind"].as_str().unwrap_or("")))
                .all(|event| !event.to_string().contains("probe-cursor-manual.txt")),
            "{turn:#?}"
        );
        assert_eq!(
            turn.iter()
                .filter(|event| event["kind"] == "tool" && event["state"] == "started")
                .count(),
            1,
            "{turn:#?}"
        );
        // Bypass: print mode runs it without asking anyone.
        let sid = fx
            .start_task("cursor", "bypass", None, &task("probe-cursor-bypass.txt"))
            .await;
        let events = answer_all(&fx, &sid, "deny").await;
        assert!(events.iter().all(|event| event["kind"] != "permission"), "{events:#?}");
        assert!(probe("probe-cursor-bypass.txt").exists(), "{}", said(&events));
        fx.close().await;
    });
}
