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
      case "$line" in *chained*) cmd='touch probe.txt && true';; *) cmd='touch probe.txt';; esac
      printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"chat-fake-1","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"Creating the probe."}}}}}}}}'
      printf '%s\n' "{{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{{\"sessionId\":\"chat-fake-1\",\"update\":{{\"sessionUpdate\":\"tool_call\",\"toolCallId\":\"call-1\",\"title\":\"$cmd\",\"kind\":\"execute\",\"status\":\"pending\",\"rawInput\":{{\"command\":\"$cmd\"}}}}}}}}"
      printf '%s\n' "{{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"session/request_permission\",\"params\":{{\"sessionId\":\"chat-fake-1\",\"toolCall\":{{\"toolCallId\":\"call-1\",\"title\":\"$cmd\",\"kind\":\"execute\",\"rawInput\":{{\"command\":\"$cmd\"}}}},\"options\":[{{\"optionId\":\"allow-once\",\"name\":\"Allow once\",\"kind\":\"allow_once\"}},{{\"optionId\":\"allow-always\",\"name\":\"Always allow\",\"kind\":\"allow_always\"}},{{\"optionId\":\"reject-once\",\"name\":\"Reject\",\"kind\":\"reject_once\"}}]}}}}";;
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
        self.answer_with(sid, request, json!({"decision": decision}))
            .await
    }
    async fn answer_with(&self, sid: &str, request: &str, body: Value) -> (u16, Value) {
        self.http(
            Method::POST,
            &format!("/api/sessions/{sid}/permissions/{request}"),
            Some(body),
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
    let sid = fx
        .start_task("cursor", "manual", None, "Create the probe file chained")
        .await;
    let events = fx
        .until(&sid, "the Cursor request", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let pending = permission_events(&events, "pending")[0].clone();
    let request = pending["request_id"].as_str().unwrap().to_owned();
    assert!(request.starts_with("cursor-0-"), "{request}");
    assert_eq!(pending["tool"], "Shell");
    assert_eq!(pending["summary"], "touch probe.txt && true");
    assert_eq!(pending["session_option"], false);
    assert_eq!(pending["has_session_rule"], false);
    let arguments = lines(&fx.cursor_arguments);
    assert_eq!(arguments.last().map(String::as_str), Some("acp"));
    assert!(!arguments.contains(&"--print".to_owned()));
    // Chained commands never receive a Harness-local session rule, so session approval is refused.
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
    fx.send(&sid, "manual", "Once more chained").await;
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

fn write_fake_host(root: &Path, name: &str, script: String) -> PathBuf {
    let program = root.join(name);
    fs::write(&program, script).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    program
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rich_diff_preview_and_rule_summary_events_from_claude_codex_and_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let root = infra_core::fs::canonicalize(dir.path()).unwrap();
    let unused = root.join("unused");

    let claude = write_fake_host(
        &root,
        "fake-claude-rich",
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"subtype":"initialize"'*)
      printf '%s\n' '{"type":"control_response","response":{"subtype":"success","request_id":"harness-initialize","response":{"current_permission_mode":"default"}}}' ;;
    *'"subtype":"set_permission_mode"'*)
      printf '%s\n' '{"type":"control_response","response":{"subtype":"success","request_id":"harness-permission-mode","response":{"mode":"default"}}}' ;;
    *'"type":"user"'*)
      printf '%s\n' '{"type":"system","subtype":"init","session_id":"claude-rich-1"}'
      printf '%s\n' '{"type":"control_request","request_id":"req-edit","request":{"subtype":"can_use_tool","tool_name":"Edit","display_name":"Edit","input":{"file_path":"src/app.rs","old_string":"let x = 1;","new_string":"let x = 2;"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Edit","ruleContent":"src/app.rs"}],"behavior":"allow","destination":"localSettings"}]}}' ;;
    *'"behavior":"allow"'*)
      printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Edited."}],"id":"msg-1"},"session_id":"claude-rich-1"}'
      printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"Edited.","session_id":"claude-rich-1","total_cost_usd":0.01}' ;;
  esac
done
"#
        .into(),
    );

    let codex = write_fake_host(
        &root,
        "fake-codex-rich",
        r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/^{"id":\([0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' "{\"id\":$id,\"result\":{\"userAgent\":\"fake\"}}" ;;
    *'"method":"thread/start"'*|*'"method":"thread/resume"'*)
      printf '%s\n' "{\"id\":$id,\"result\":{\"thread\":{\"id\":\"thread-rich-1\"}}}" ;;
    *'"method":"turn/start"'*)
      printf '%s\n' "{\"id\":$id,\"result\":{\"turn\":{\"id\":\"turn-1\"}}}"
      printf '%s\n' '{"method":"turn/started","params":{"threadId":"thread-rich-1","turn":{"id":"turn-1"}}}'
      printf '%s\n' '{"method":"item/started","params":{"threadId":"thread-rich-1","turnId":"turn-1","item":{"type":"fileChange","id":"item-fc","status":"inProgress","changes":[{"path":"src/lib.rs","kind":{"type":"update"},"diff":"@@ -1,1 +1,1 @@\n-old\n+new"}]}}}'
      printf '%s\n' '{"id":7,"method":"item/fileChange/requestApproval","params":{"threadId":"thread-rich-1","turnId":"turn-1","itemId":"item-fc","reason":"Update library"}}' ;;
    *'"id":7,'*'"decision":"accept"'*)
      printf '%s\n' '{"method":"item/completed","params":{"threadId":"thread-rich-1","turnId":"turn-1","item":{"type":"fileChange","id":"item-fc","status":"completed","changes":[{"path":"src/lib.rs","kind":{"type":"update"}}]}}}'
      printf '%s\n' '{"method":"item/completed","params":{"threadId":"thread-rich-1","turnId":"turn-1","item":{"type":"agentMessage","id":"item-msg","text":"Codex updated."}}}'
      printf '%s\n' '{"method":"turn/completed","params":{"threadId":"thread-rich-1","turn":{"id":"turn-1","status":"completed"}}}' ;;
  esac
done
"#
        .into(),
    );

    let cursor = write_fake_host(
        &root,
        "fake-cursor-rich",
        r#"#!/bin/sh
prompt=0
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/^{"jsonrpc":"2.0","id":\([0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":1}}" ;;
    *'"method":"session/new"'*|*'"method":"session/load"'*)
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"sessionId\":\"cursor-rich-1\"}}" ;;
    *'"method":"session/prompt"'*)
      prompt=$id
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"cursor-rich-1","update":{"sessionUpdate":"tool_call","toolCallId":"edit-1","title":"Edit src/main.rs","kind":"edit","status":"pending","rawInput":{"path":"src/main.rs","oldText":"fn a() {}","newText":"fn b() {}"}}}}'
      printf '%s\n' '{"jsonrpc":"2.0","id":101,"method":"session/request_permission","params":{"sessionId":"cursor-rich-1","toolCall":{"toolCallId":"edit-1","title":"Edit src/main.rs","kind":"edit"},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Always allow","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}' ;;
    *'"optionId":"allow-once"'*)
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"cursor-rich-1","update":{"sessionUpdate":"tool_call_update","toolCallId":"edit-1","status":"completed"}}}'
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"cursor-rich-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Cursor updated."}}}}'
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{\"stopReason\":\"end_turn\"}}" ;;
  esac
done
"#
        .into(),
    );

    let fx = Fixture::with_hosts(
        dir,
        [claude, codex, cursor],
        [unused.clone(), unused.clone(), unused],
        Duration::from_secs(20),
    )
    .await;

    // 1. Claude Edit diff_preview & permission_suggestions rule_summary
    let claude_sid = fx.start_with("claude", "manual").await;
    let events = fx
        .until(&claude_sid, "claude rich ask", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let pending = permission_events(&events, "pending")[0].clone();
    assert_eq!(pending["ask_kind"], "file_change");
    assert_eq!(pending["session_option"], true);
    assert_eq!(pending["has_session_rule"], false);
    assert_eq!(pending["rule_summary"], "Session rule: Edit(src/app.rs)");
    let claude_diff = pending["diff_preview"].as_str().unwrap();
    assert!(claude_diff.contains("--- a/src/app.rs"), "{claude_diff}");
    assert!(
        claude_diff.contains("-let x = 1;\n+let x = 2;"),
        "{claude_diff}"
    );
    let (status, got) = fx
        .http(Method::GET, &format!("/api/sessions/{claude_sid}"), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(got["waiting_permissions"][0]["diff_preview"], claude_diff);
    assert_eq!(
        got["waiting_permissions"][0]["rule_summary"],
        "Session rule: Edit(src/app.rs)"
    );
    let req_id = pending["request_id"].as_str().unwrap();
    assert_eq!(fx.answer(&claude_sid, req_id, "allow").await.0, 200);
    let events = fx
        .until(&claude_sid, "claude finished", |session, _| {
            finished(session)
        })
        .await;
    assert!(completed(&fx.store.get(&claude_sid).unwrap()));
    let resolved = permission_events(&events, "allowed")[0];
    assert_eq!(resolved["diff_preview"], claude_diff);
    assert_eq!(resolved["rule_summary"], "Session rule: Edit(src/app.rs)");

    // 2. Codex File change diff_preview & thread rule_summary
    let codex_sid = fx.start_with("codex", "manual").await;
    let events = fx
        .until(&codex_sid, "codex rich ask", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let pending = permission_events(&events, "pending")[0].clone();
    assert_eq!(pending["ask_kind"], "file_change");
    assert_eq!(pending["session_option"], true);
    assert_eq!(pending["has_session_rule"], false);
    assert_eq!(
        pending["rule_summary"],
        "Remember approval for this Codex thread"
    );
    let codex_diff = pending["diff_preview"].as_str().unwrap();
    assert!(codex_diff.contains("--- a/src/lib.rs"), "{codex_diff}");
    assert!(codex_diff.contains("-old\n+new"), "{codex_diff}");
    let req_id = pending["request_id"].as_str().unwrap();
    assert_eq!(fx.answer(&codex_sid, req_id, "allow").await.0, 200);
    let events = fx
        .until(&codex_sid, "codex finished", |session, _| finished(session))
        .await;
    assert!(completed(&fx.store.get(&codex_sid).unwrap()));
    assert_eq!(
        permission_events(&events, "allowed")[0]["diff_preview"],
        codex_diff
    );

    // 3. Cursor Edit diff_preview & Harness-local session rule summary
    let cursor_sid = fx.start_with("cursor", "manual").await;
    let events = fx
        .until(&cursor_sid, "cursor rich ask", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let pending = permission_events(&events, "pending")[0].clone();
    assert_eq!(pending["ask_kind"], "file_change");
    assert_eq!(pending["session_option"], true);
    assert_eq!(pending["has_session_rule"], true);
    assert_eq!(
        pending["rule_summary"],
        "Remember exact command/path in Harness for this session (never writes ~/.cursor)"
    );
    let cursor_diff = pending["diff_preview"].as_str().unwrap();
    assert!(cursor_diff.contains("--- a/src/main.rs"), "{cursor_diff}");
    assert!(
        cursor_diff.contains("-fn a() {}\n+fn b() {}"),
        "{cursor_diff}"
    );
    let req_id = pending["request_id"].as_str().unwrap();
    assert_eq!(fx.answer(&cursor_sid, req_id, "allow").await.0, 200);
    let events = fx
        .until(&cursor_sid, "cursor finished", |session, _| {
            finished(session)
        })
        .await;
    assert!(completed(&fx.store.get(&cursor_sid).unwrap()));
    assert_eq!(
        permission_events(&events, "allowed")[0]["diff_preview"],
        cursor_diff
    );

    fx.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cursor_allow_session_caches_ephemeral_rules_without_writing_cursor_config() {
    let dir = tempfile::tempdir().unwrap();
    let root = infra_core::fs::canonicalize(dir.path()).unwrap();
    let unused = root.join("unused");
    let replies_log = root.join("cursor-cache-replies.jsonl");

    let cursor = write_fake_host(
        &root,
        "fake-cursor-cache",
        r#"#!/bin/sh
prompt=0
mode=turn1
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/^{"jsonrpc":"2.0","id":\([0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":1}}" ;;
    *'"method":"session/new"'*|*'"method":"session/load"'*)
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"sessionId\":\"chat-cache-1\"}}" ;;
    *'"method":"session/prompt"'*)
      prompt=$id
      case "$line" in *turn2*) mode=turn2;; *) mode=turn1;; esac
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call","toolCallId":"c1","title":"touch probe.txt","kind":"execute","status":"pending","rawInput":{"command":"touch probe.txt"}}}}'
      printf '%s\n' '{"jsonrpc":"2.0","id":101,"method":"session/request_permission","params":{"sessionId":"chat-cache-1","toolCall":{"toolCallId":"c1","title":"touch probe.txt","kind":"execute","rawInput":{"command":"touch probe.txt"}},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Always allow","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}' ;;
    *'"id":101,'*|*'"id":101}'*)
      printf '%s\n' "$line" >> "__REPLIES_LOG__"
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call_update","toolCallId":"c1","status":"completed"}}}'
      if [ "$mode" = turn1 ]; then
        printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call","toolCallId":"c2","title":"touch probe.txt","kind":"execute","status":"pending","rawInput":{"command":"touch probe.txt"}}}}'
        printf '%s\n' '{"jsonrpc":"2.0","id":102,"method":"session/request_permission","params":{"sessionId":"chat-cache-1","toolCall":{"toolCallId":"c2","title":"touch probe.txt","kind":"execute","rawInput":{"command":"touch probe.txt"}},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Always allow","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}'
      else
        printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call","toolCallId":"c2","title":"Edit src/app.rs","kind":"edit","status":"pending","rawInput":{"path":"src/app.rs","oldText":"1","newText":"2"}}}}'
        printf '%s\n' '{"jsonrpc":"2.0","id":102,"method":"session/request_permission","params":{"sessionId":"chat-cache-1","toolCall":{"toolCallId":"c2","title":"Edit src/app.rs","kind":"edit","rawInput":{"path":"src/app.rs","oldText":"1","newText":"2"}},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Always allow","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}'
      fi ;;
    *'"id":102,'*|*'"id":102}'*)
      printf '%s\n' "$line" >> "__REPLIES_LOG__"
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call_update","toolCallId":"c2","status":"completed"}}}'
      if [ "$mode" = turn1 ]; then
        printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call","toolCallId":"c3","title":"Edit ./src/./app.rs","kind":"edit","status":"pending","rawInput":{"path":"./src/./app.rs","oldText":"1","newText":"2"}}}}'
        printf '%s\n' '{"jsonrpc":"2.0","id":103,"method":"session/request_permission","params":{"sessionId":"chat-cache-1","toolCall":{"toolCallId":"c3","title":"Edit ./src/./app.rs","kind":"edit","rawInput":{"path":"./src/./app.rs","oldText":"1","newText":"2"}},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Always allow","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}'
      else
        printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call","toolCallId":"c3","title":"touch other.txt","kind":"execute","status":"pending","rawInput":{"command":"touch other.txt"}}}}'
        printf '%s\n' '{"jsonrpc":"2.0","id":103,"method":"session/request_permission","params":{"sessionId":"chat-cache-1","toolCall":{"toolCallId":"c3","title":"touch other.txt","kind":"execute","rawInput":{"command":"touch other.txt"}},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Always allow","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}'
      fi ;;
    *'"id":103,'*|*'"id":103}'*)
      printf '%s\n' "$line" >> "__REPLIES_LOG__"
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call_update","toolCallId":"c3","status":"completed"}}}'
      if [ "$mode" = turn1 ]; then
        printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Turn 1 done."}}}}'
        printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{\"stopReason\":\"end_turn\"}}"
      else
        printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call","toolCallId":"c4","title":"Edit src/other.rs","kind":"edit","status":"pending","rawInput":{"path":"src/other.rs","oldText":"a","newText":"b"}}}}'
        printf '%s\n' '{"jsonrpc":"2.0","id":104,"method":"session/request_permission","params":{"sessionId":"chat-cache-1","toolCall":{"toolCallId":"c4","title":"Edit src/other.rs","kind":"edit","rawInput":{"path":"src/other.rs","oldText":"a","newText":"b"}},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Always allow","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}'
      fi ;;
    *'"id":104,'*|*'"id":104}'*)
      printf '%s\n' "$line" >> "__REPLIES_LOG__"
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call_update","toolCallId":"c4","status":"completed"}}}'
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call","toolCallId":"c5","title":"touch other.txt","kind":"execute","status":"pending","rawInput":{"command":"touch other.txt"}}}}'
      printf '%s\n' '{"jsonrpc":"2.0","id":105,"method":"session/request_permission","params":{"sessionId":"chat-cache-1","toolCall":{"toolCallId":"c5","title":"touch other.txt","kind":"execute","rawInput":{"command":"touch other.txt"}},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Always allow","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}' ;;
    *'"id":105,'*|*'"id":105}'*)
      printf '%s\n' "$line" >> "__REPLIES_LOG__"
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"tool_call_update","toolCallId":"c5","status":"completed"}}}'
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-cache-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Turn 2 done."}}}}'
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{\"stopReason\":\"end_turn\"}}" ;;
  esac
done
"#
        .replace("__REPLIES_LOG__", &replies_log.display().to_string()),
    );

    let fx = Fixture::with_hosts(
        dir,
        [unused.clone(), unused.clone(), cursor],
        [unused.clone(), unused.clone(), unused],
        Duration::from_secs(20),
    )
    .await;

    // Session A Turn 1: allow_session on touch probe.txt caches and auto-approves the repeat;
    // then allow_session on ./src/./app.rs caches the normalized edit path.
    let sid_a = fx.start_task("cursor", "manual", None, "turn1").await;
    let events = fx
        .until(&sid_a, "first cursor ask", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let req1 = permission_events(&events, "pending")[0]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        permission_events(&events, "pending")[0]["summary"],
        "touch probe.txt"
    );
    assert_eq!(fx.answer(&sid_a, &req1, "allow_session").await.0, 200);

    // Ask 102 ("touch probe.txt") must NOT become pending; the next pending is 103 ("./src/./app.rs").
    let events = fx
        .until(&sid_a, "edit prompt after cached repeat", |_, events| {
            permission_events(events, "pending").len() == 2
        })
        .await;
    let pending_edit = permission_events(&events, "pending")[1];
    assert_eq!(pending_edit["summary"], "./src/./app.rs");
    assert_eq!(pending_edit["ask_kind"], "file_change");
    let req_edit = pending_edit["request_id"].as_str().unwrap().to_owned();
    assert_eq!(fx.answer(&sid_a, &req_edit, "allow_session").await.0, 200);
    let turn1_events = fx
        .until(&sid_a, "turn 1 completed", |session, _| finished(session))
        .await;
    assert!(completed(&fx.store.get(&sid_a).unwrap()));
    let turn1_session_rule: Vec<_> = permission_events(&turn1_events, "allowed_session")
        .into_iter()
        .filter(|e| e["by"] == "session_rule")
        .collect();
    assert_eq!(turn1_session_rule.len(), 1, "{turn1_events:#?}");
    assert_eq!(turn1_session_rule[0]["summary"], "touch probe.txt");

    // Session A Turn 2: cached command ("touch probe.txt") and normalized cached edit ("src/app.rs")
    // auto-resolve immediately; different command/path and allow-once repeats still prompt.
    let turn2_start = turn1_events.len();
    fx.send(&sid_a, "manual", "turn2").await;
    let events = fx
        .until(&sid_a, "different command prompt", |_, events| {
            permission_events(&events[turn2_start..], "pending").len() == 1
        })
        .await;
    let req_other_cmd = permission_events(&events[turn2_start..], "pending")[0]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        permission_events(&events[turn2_start..], "pending")[0]["summary"],
        "touch other.txt"
    );
    assert_eq!(fx.answer(&sid_a, &req_other_cmd, "allow").await.0, 200);

    let events = fx
        .until(&sid_a, "different edit path prompt", |_, events| {
            permission_events(&events[turn2_start..], "pending").len() == 2
        })
        .await;
    let req_other_edit = permission_events(&events[turn2_start..], "pending")[1]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        permission_events(&events[turn2_start..], "pending")[1]["summary"],
        "src/other.rs"
    );
    assert_eq!(fx.answer(&sid_a, &req_other_edit, "allow").await.0, 200);

    // Repeat of "touch other.txt" (which only got Decision::Allow) must still prompt!
    let events = fx
        .until(&sid_a, "repeat after allow-once", |_, events| {
            permission_events(&events[turn2_start..], "pending").len() == 3
        })
        .await;
    let req_repeat_once = permission_events(&events[turn2_start..], "pending")[2]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        permission_events(&events[turn2_start..], "pending")[2]["summary"],
        "touch other.txt"
    );
    assert_eq!(fx.answer(&sid_a, &req_repeat_once, "allow").await.0, 200);
    let all_a = fx
        .until(&sid_a, "turn 2 completed", |session, events| {
            finished(session)
                && events
                    .iter()
                    .filter(|event| event["kind"] == "result")
                    .count()
                    >= 2
        })
        .await;
    assert!(completed(&fx.store.get(&sid_a).unwrap()));
    let turn2_rule_hits: Vec<_> = permission_events(&all_a[turn2_start..], "allowed_session")
        .into_iter()
        .filter(|e| e["by"] == "session_rule")
        .collect();
    assert_eq!(turn2_rule_hits.len(), 2, "{:#?}", &all_a[turn2_start..]);

    // Session B on the same PermissionDesk does NOT inherit Session A's cached rules.
    let sid_b = fx.start_task("cursor", "manual", None, "turn1").await;
    for expected_summary in ["touch probe.txt", "touch probe.txt", "./src/./app.rs"] {
        let count_before = permission_events(&fx.events(&sid_b), "allowed").len();
        let events = fx
            .until(&sid_b, "session B prompt", |_, events| {
                permission_events(events, "pending").len() > count_before
            })
            .await;
        let pending = permission_events(&events, "pending")[count_before].clone();
        assert_eq!(pending["summary"], expected_summary);
        let req = pending["request_id"].as_str().unwrap();
        assert_eq!(fx.answer(&sid_b, req, "allow").await.0, 200);
    }
    fx.until(&sid_b, "session B completed", |session, _| {
        finished(session)
    })
    .await;
    assert!(completed(&fx.store.get(&sid_b).unwrap()));

    // Cursor CLI must NEVER receive allow-always (so ~/.cursor/cli-config.json is never touched).
    let raw_replies = lines(&replies_log);
    assert_eq!(raw_replies.len(), 11, "{raw_replies:#?}");
    for line in &raw_replies {
        let parsed: Value = serde_json::from_str(line).unwrap();
        assert_eq!(
            parsed["result"]["outcome"]["optionId"], "allow-once",
            "{parsed}"
        );
    }

    fx.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http_permission_rules_api_lists_enforces_csrf_and_revokes_session_rules() {
    let dir = tempfile::tempdir().unwrap();
    let root = infra_core::fs::canonicalize(dir.path()).unwrap();
    let unused = root.join("unused");

    let cursor = write_fake_host(
        &root,
        "fake-cursor-revoke",
        r#"#!/bin/sh
prompt=0
turn=0
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/^{"jsonrpc":"2.0","id":\([0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":1}}" ;;
    *'"method":"session/new"'*|*'"method":"session/load"'*)
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"sessionId\":\"chat-rev-1\"}}" ;;
    *'"method":"session/prompt"'*)
      prompt=$id
      turn=$((turn + 1))
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-rev-1","update":{"sessionUpdate":"tool_call","toolCallId":"r1","title":"touch probe.txt","kind":"execute","status":"pending","rawInput":{"command":"touch probe.txt"}}}}'
      printf '%s\n' '{"jsonrpc":"2.0","id":201,"method":"session/request_permission","params":{"sessionId":"chat-rev-1","toolCall":{"toolCallId":"r1","title":"touch probe.txt","kind":"execute","rawInput":{"command":"touch probe.txt"}},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Always allow","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}' ;;
    *'"id":201,'*|*'"id":201}'*)
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-rev-1","update":{"sessionUpdate":"tool_call_update","toolCallId":"r1","status":"completed"}}}'
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-rev-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"ok"}}}}'
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{\"stopReason\":\"end_turn\"}}" ;;
  esac
done
"#
        .to_string(),
    );

    let fx = Fixture::with_hosts(
        dir,
        [unused.clone(), unused.clone(), cursor],
        [unused.clone(), unused.clone(), unused],
        Duration::from_secs(20),
    )
    .await;

    let sid = fx.start_task("cursor", "manual", None, "turn1").await;
    let events = fx
        .until(&sid, "initial permission ask", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let req1 = permission_events(&events, "pending")[0]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(fx.answer(&sid, &req1, "allow_session").await.0, 200);
    fx.until(&sid, "turn 1 done", |session, _| finished(session))
        .await;

    // Inspect active session rules over HTTP.
    let (status, listed) = fx
        .http(
            Method::GET,
            &format!("/api/sessions/{sid}/permission-rules"),
            None,
        )
        .await;
    assert_eq!(status, 200);
    let rules = listed["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 1, "{listed}");
    let rule_id = rules[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(rules[0]["provider"], "cursor");

    // DELETE without X-Harness-Token must return 403.
    let unauth = fx
        .client
        .delete(format!(
            "http://127.0.0.1:{}/api/sessions/{sid}/permission-rules/{rule_id}",
            fx.port
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status().as_u16(), 403);

    // DELETE with token revokes the remembered rule cleanly.
    let (del_status, del_body) = fx
        .http(
            Method::DELETE,
            &format!("/api/sessions/{sid}/permission-rules/{rule_id}"),
            None,
        )
        .await;
    assert_eq!(del_status, 200);
    assert_eq!(del_body["ok"], true);
    assert_eq!(del_body["removed"], 1);
    assert_eq!(del_body["rules"], json!([]));

    // Sending identical command on Turn 2 must prompt again rather than auto-approving.
    let turn2_start = fx.events(&sid).len();
    fx.send(&sid, "manual", "turn2").await;
    let events = fx
        .until(&sid, "re-prompt after revocation", |_, events| {
            !permission_events(&events[turn2_start..], "pending").is_empty()
        })
        .await;
    let req2 = permission_events(&events[turn2_start..], "pending")[0]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(fx.answer(&sid, &req2, "allow").await.0, 200);
    fx.until(&sid, "turn 2 done", |session, events| {
        finished(session)
            && events
                .iter()
                .filter(|event| event["kind"] == "result")
                .count()
                >= 2
    })
    .await;
    assert!(completed(&fx.store.get(&sid).unwrap()));
    assert!(
        permission_events(&fx.events(&sid)[turn2_start..], "allowed_session").is_empty(),
        "revoked rule must not auto-approve"
    );

    fx.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_structured_questions_route_answers_for_claude_codex_and_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let root = infra_core::fs::canonicalize(dir.path()).unwrap();
    let unused = root.join("unused");
    let claude_log = root.join("claude-q-replies.jsonl");
    let codex_log = root.join("codex-q-replies.jsonl");
    let cursor_log = root.join("cursor-q-replies.jsonl");

    let claude = write_fake_host(
        &root,
        "fake-claude-q",
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"subtype":"initialize"'*)
      printf '%s\n' '{"type":"control_response","response":{"subtype":"success","request_id":"harness-initialize","response":{"current_permission_mode":"default"}}}' ;;
    *'"subtype":"set_permission_mode"'*)
      printf '%s\n' '{"type":"control_response","response":{"subtype":"success","request_id":"harness-permission-mode","response":{"mode":"default"}}}' ;;
    *'"type":"user"'*)
      printf '%s\n' '{"type":"system","subtype":"init","session_id":"claude-q-1"}'
      printf '%s\n' '{"type":"control_request","request_id":"req-q","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","display_name":"AskUserQuestion","input":{"questions":[{"header":"Database","question":"Which database?","multiSelect":false,"options":[{"label":"PostgreSQL","description":"Relational"},{"label":"SQLite","description":"Embedded"}]},{"header":"Features","question":"Which features?","multiSelect":true,"options":[{"label":"Auth","description":"JWT"},{"label":"Audit","description":"Logs"}]}]}}}' ;;
    *'"request_id":"req-q"'*)
      printf '%s\n' "$line" >> "__CLAUDE_LOG__"
      printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Got question answers."}]},"session_id":"claude-q-1"}'
      printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"Got question answers.","session_id":"claude-q-1","total_cost_usd":0.01}' ;;
  esac
done
"#
        .replace("__CLAUDE_LOG__", &claude_log.display().to_string()),
    );

    let codex = write_fake_host(
        &root,
        "fake-codex-q",
        r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/^{"id":\([0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' "{\"id\":$id,\"result\":{\"userAgent\":\"fake\"}}" ;;
    *'"method":"thread/start"'*|*'"method":"thread/resume"'*)
      printf '%s\n' "{\"id\":$id,\"result\":{\"thread\":{\"id\":\"thread-q-1\"}}}" ;;
    *'"method":"turn/start"'*)
      printf '%s\n' "{\"id\":$id,\"result\":{\"turn\":{\"id\":\"turn-1\"}}}"
      printf '%s\n' '{"method":"turn/started","params":{"threadId":"thread-q-1","turn":{"id":"turn-1"}}}'
      printf '%s\n' '{"id":77,"method":"item/tool/requestUserInput","params":{"threadId":"thread-q-1","turnId":"turn-1","questions":[{"id":"db","header":"Database","question":"Which database?","isOther":true,"options":[{"label":"PostgreSQL","description":"Full SQL"},{"label":"SQLite","description":"File DB"}]},{"id":"flags","header":"Flags","question":"Select flags","multiSelect":true,"isOther":false,"options":[{"label":"Verbose","description":"More logs"},{"label":"Strict","description":"Fail fast"}]}]}}' ;;
    *'"id":77,'*|*'"id":77}'*)
      printf '%s\n' "$line" >> "__CODEX_LOG__"
      printf '%s\n' '{"method":"item/completed","params":{"threadId":"thread-q-1","turnId":"turn-1","item":{"type":"agentMessage","id":"item-msg","text":"Codex got answers."}}}'
      printf '%s\n' '{"method":"turn/completed","params":{"threadId":"thread-q-1","turn":{"id":"turn-1","status":"completed"}}}' ;;
  esac
done
"#
        .replace("__CODEX_LOG__", &codex_log.display().to_string()),
    );

    let cursor = write_fake_host(
        &root,
        "fake-cursor-q",
        r#"#!/bin/sh
prompt=0
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/^{"jsonrpc":"2.0","id":\([0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":1}}" ;;
    *'"method":"session/new"'*|*'"method":"session/load"'*)
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"sessionId\":\"cursor-q-1\"}}" ;;
    *'"method":"session/prompt"'*)
      prompt=$id
      case "$line" in *decline*) qid=89;; *) qid=88;; esac
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$qid,\"method\":\"cursor/ask_question\",\"params\":{\"toolCallId\":\"call-q\",\"title\":\"Configuration\",\"questions\":[{\"id\":\"q1\",\"prompt\":\"Pick runtime\",\"allowMultiple\":false,\"options\":[{\"id\":\"opt_tokio\",\"label\":\"Tokio\"},{\"id\":\"opt_async\",\"label\":\"Async-std\"}]},{\"id\":\"q2\",\"prompt\":\"Extra flags?\",\"allowMultiple\":true,\"options\":[{\"id\":\"opt_a\",\"label\":\"Flag A\"}]}]}}" ;;
    *'"id":88,'*|*'"id":88}'*|*'"id":89,'*|*'"id":89}'*)
      printf '%s\n' "$line" >> "__CURSOR_LOG__"
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"cursor-q-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Cursor got answers."}}}}'
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{\"stopReason\":\"end_turn\"}}" ;;
  esac
done
"#
        .replace("__CURSOR_LOG__", &cursor_log.display().to_string()),
    );

    let fx = Fixture::with_hosts(
        dir,
        [claude, codex, cursor],
        [unused.clone(), unused.clone(), unused],
        Duration::from_secs(20),
    )
    .await;

    // 1. Claude AskUserQuestion + validation
    let claude_sid = fx.start_with("claude", "manual").await;
    let events = fx
        .until(&claude_sid, "claude question ask", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let pending = permission_events(&events, "pending")[0].clone();
    assert_eq!(pending["ask_kind"], "question");
    assert_eq!(pending["session_option"], false);
    assert_eq!(pending["questions"].as_array().unwrap().len(), 2);
    let req_id = pending["request_id"].as_str().unwrap();

    // Non-object, oversized, allow_session, and unknown-field bodies are rejected.
    assert_eq!(
        fx.answer_with(
            &claude_sid,
            req_id,
            json!({"decision": "allow", "answers": ["bad"]}),
        )
        .await
        .0,
        400
    );
    assert_eq!(
        fx.answer_with(
            &claude_sid,
            req_id,
            json!({"decision": "allow", "answers": {"big": "x".repeat(40_000)}}),
        )
        .await
        .0,
        400
    );
    assert_eq!(
        fx.answer_with(
            &claude_sid,
            req_id,
            json!({"decision": "allow_session", "answers": {"Which database?": "PostgreSQL"}}),
        )
        .await
        .0,
        400
    );
    assert_eq!(
        fx.answer_with(
            &claude_sid,
            req_id,
            json!({"decision": "allow", "answers": {}, "extra": true}),
        )
        .await
        .0,
        400
    );

    let submitted = json!({
        "Which database?": "PostgreSQL",
        "Which features?": ["Auth", "Audit"],
    });
    let (status, res) = fx
        .answer_with(
            &claude_sid,
            req_id,
            json!({"decision": "allow", "answers": submitted}),
        )
        .await;
    assert_eq!(status, 200, "{res}");
    let events = fx
        .until(&claude_sid, "claude question finished", |session, _| {
            finished(session)
        })
        .await;
    assert!(completed(&fx.store.get(&claude_sid).unwrap()));
    assert_eq!(
        permission_events(&events, "allowed")[0]["answers"],
        submitted
    );
    let claude_reply: Value = serde_json::from_str(&lines(&claude_log)[0]).unwrap();
    let updated = &claude_reply["response"]["response"]["updatedInput"];
    assert_eq!(claude_reply["response"]["response"]["behavior"], "allow");
    assert_eq!(updated["answers"]["Which database?"], "PostgreSQL");
    assert_eq!(updated["answers"]["Which features?"], "Auth, Audit");

    // 2. Codex item/tool/requestUserInput
    let codex_sid = fx.start_with("codex", "manual").await;
    let events = fx
        .until(&codex_sid, "codex question ask", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let pending = permission_events(&events, "pending")[0].clone();
    assert_eq!(pending["ask_kind"], "question");
    assert_eq!(pending["session_option"], false);
    assert_eq!(pending["questions"].as_array().unwrap().len(), 2);
    let req_id = pending["request_id"].as_str().unwrap();
    let (status, res) = fx
        .answer_with(
            &codex_sid,
            req_id,
            json!({
                "decision": "allow",
                "answers": {"db": "PostgreSQL", "flags": ["Verbose", "Strict"]}
            }),
        )
        .await;
    assert_eq!(status, 200, "{res}");
    fx.until(&codex_sid, "codex question finished", |session, _| {
        finished(session)
    })
    .await;
    assert!(completed(&fx.store.get(&codex_sid).unwrap()));
    let codex_reply: Value = serde_json::from_str(&lines(&codex_log)[0]).unwrap();
    assert_eq!(codex_reply["id"], 77);
    assert_eq!(
        codex_reply["result"]["answers"],
        json!({
            "db": {"answers": ["PostgreSQL"]},
            "flags": {"answers": ["Verbose", "Strict"]},
        })
    );

    // 3. Cursor cursor/ask_question (answered + declined)
    let cursor_sid = fx.start_task("cursor", "manual", None, "ask").await;
    let events = fx
        .until(&cursor_sid, "cursor question ask", |_, events| {
            !permission_events(events, "pending").is_empty()
        })
        .await;
    let pending = permission_events(&events, "pending")[0].clone();
    assert_eq!(pending["ask_kind"], "question");
    assert_eq!(pending["session_option"], false);
    assert_eq!(pending["questions"].as_array().unwrap().len(), 2);
    let req_id = pending["request_id"].as_str().unwrap();
    let (status, res) = fx
        .answer_with(
            &cursor_sid,
            req_id,
            json!({
                "decision": "allow",
                "answers": {"q1": "Tokio", "q2": ["opt_a", "custom-flag"]}
            }),
        )
        .await;
    assert_eq!(status, 200, "{res}");
    fx.until(&cursor_sid, "cursor question finished", |session, _| {
        finished(session)
    })
    .await;
    assert!(completed(&fx.store.get(&cursor_sid).unwrap()));

    fx.send(&cursor_sid, "manual", "decline").await;
    let events = fx
        .until(&cursor_sid, "cursor declined question", |_, events| {
            permission_events(events, "pending").len() == 2
        })
        .await;
    let req_decline = permission_events(&events, "pending")[1]["request_id"]
        .as_str()
        .unwrap();
    assert_eq!(fx.answer(&cursor_sid, req_decline, "deny").await.0, 200);
    fx.until(
        &cursor_sid,
        "cursor declined question finished",
        |session, events| {
            finished(session)
                && events
                    .iter()
                    .filter(|event| event["kind"] == "result")
                    .count()
                    >= 2
        },
    )
    .await;
    assert!(completed(&fx.store.get(&cursor_sid).unwrap()));

    let cursor_replies = lines(&cursor_log);
    assert_eq!(cursor_replies.len(), 2, "{cursor_replies:#?}");
    let answered_reply: Value = serde_json::from_str(&cursor_replies[0]).unwrap();
    assert_eq!(answered_reply["id"], 88);
    assert_eq!(
        answered_reply["result"]["outcome"],
        json!({
            "outcome": "answered",
            "answers": [
                {"questionId": "q1", "selectedOptionIds": ["opt_tokio"]},
                {"questionId": "q2", "selectedOptionIds": ["opt_a"], "freeform": "custom-flag"}
            ]
        })
    );
    let declined_reply: Value = serde_json::from_str(&cursor_replies[1]).unwrap();
    assert_eq!(declined_reply["id"], 89);
    assert_eq!(
        declined_reply["result"]["outcome"],
        json!({"outcome": "cancelled"})
    );

    fx.close().await;
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
