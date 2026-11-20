//! SkillCache through the Harness's own routes and an actual native process:
//! a provider stand-in prints a scripted event stream, and the session's
//! `skillcache` option decides what is kept of it.
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
    skillcache::SkillCacheDomain,
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

/// A provider stand-in: it records its command line and prompt, then prints
/// the event stream the test wrote for it.
fn scripted(root: &Path, name: &str) -> PathBuf {
    let program = root.join(format!("fake-{name}"));
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{root}/{name}-arguments.txt\"\ncat > \"{root}/{name}-prompt.txt\"\ncat \"{root}/{name}-events.jsonl\"\n",
        root = root.display(),
    );
    fs::write(&program, script).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(root.join(format!("{name}-events.jsonl")), "").unwrap();
    program
}

struct Stand {
    _dir: tempfile::TempDir,
    root: PathBuf,
    project_id: String,
    store: SessionStore,
    services: Arc<NativeServices>,
    port: u16,
    token: String,
    shutdown: tokio::sync::watch::Sender<bool>,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
}

impl Stand {
    async fn new() -> Self {
        let _ = infra_host::process::use_guardian(PathBuf::from(env!(
            "CARGO_BIN_EXE_infra-harness-inbox-fixture"
        )));
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let project = root.join("project");
        let source = root.join("source");
        fs::create_dir_all(project.join(".claude/skills/pdf/scripts")).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(
            project.join(".claude/skills/pdf/SKILL.md"),
            "---\nname: pdf\ndescription: Work with PDF files\n---\n# PDF\n\nUse scripts/extract.py.\n",
        )
        .unwrap();
        fs::write(
            project.join(".claude/skills/pdf/scripts/extract.py"),
            "print('rows')\n",
        )
        .unwrap();
        let store = SessionStore::open(&root.join("state")).unwrap();
        let project_id = store.register(&project).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let fake = |id: &str| {
            (
                id.to_owned(),
                Provider {
                    id: id.into(),
                    name: format!("Fake {id}"),
                    available: true,
                    executable: Some(scripted(&root, id)),
                    detail: "Test fixture".into(),
                },
            )
        };
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::from([
                fake("claude"),
                fake("codex"),
                fake("cursor"),
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
                seconds: 60,
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
        domains.insert(
            "skillcache".into(),
            Arc::new(SkillCacheDomain::new(store.clone())),
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
        let token = "harness-skillcache-test-token".to_owned();
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
            root,
            project_id,
            store,
            services,
            port,
            token,
            shutdown,
            server,
            client: reqwest::Client::new(),
        }
    }
    fn project(&self) -> PathBuf {
        self.root.join("project")
    }
    /// What the provider stand-in prints on its next turn.
    fn script(&self, provider: &str, events: &[Value]) {
        let lines = events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            self.root.join(format!("{provider}-events.jsonl")),
            format!("{lines}\n"),
        )
        .unwrap();
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
    fn options(&self, prompt: &str, skillcache: Value) -> Value {
        let mut options = json!({
            "project_id": self.project_id,
            "provider": "claude",
            "mode": "edit",
            "workflow": "native",
            "project_context": false,
            "permissions": "bypass",
            "prompt": prompt,
        });
        if !skillcache.is_null() {
            options["skillcache"] = skillcache;
        }
        options
    }
    /// Starts a session and waits for its turn to end.
    async fn turn(&self, prompt: &str, skillcache: Value) -> String {
        let (status, created) = self
            .http(
                Method::POST,
                "/api/sessions",
                Some(self.options(prompt, skillcache)),
            )
            .await;
        assert_eq!(status, 201, "{created}");
        let sid = created["session"]["id"].as_str().unwrap().to_owned();
        self.finished(&sid, 1).await;
        sid
    }
    /// Waits until the session's `turns`-th turn has ended.
    async fn finished(&self, sid: &str, turns: usize) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let session = self.store.get(sid).unwrap();
            let events = self.store.events(sid, 0).unwrap();
            let ended = events
                .iter()
                .filter(|event| event["kind"] == "result")
                .count();
            if ["completed", "failed", "interrupted", "cancelled"]
                .contains(&session["status"].as_str().unwrap_or(""))
                && ended >= turns
            {
                return session;
            }
            assert!(
                Instant::now() < deadline,
                "the turn never ended: {session}\n{events:#?}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    fn skillcache_events(&self, sid: &str) -> Vec<Value> {
        self.store
            .events(sid, 0)
            .unwrap()
            .into_iter()
            .filter(|event| event["kind"] == "skillcache")
            .collect()
    }
    /// The stored traces, as the rows are.
    fn traces(&self) -> Vec<(String, String, Value, Value, Value)> {
        let db = self.store.connection().unwrap();
        let mut query = db
            .prepare("SELECT session_id,request,skill_refs,steps,redaction FROM skillcache_traces ORDER BY created_at,rowid")
            .unwrap();
        query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    serde_json::from_str(&row.get::<_, String>(2)?).unwrap(),
                    serde_json::from_str(&row.get::<_, String>(3)?).unwrap(),
                    serde_json::from_str(&row.get::<_, String>(4)?).unwrap(),
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }
    async fn close(self) {
        let _ = self.shutdown.send(true);
        let _ = self.services.close().await;
        let _ = self.server.await;
    }
}

/// A cold turn that loads the `pdf` skill, runs one command and answers.
fn skill_turn(project: &Path) -> Vec<Value> {
    let skill = project.join(".claude/skills/pdf/SKILL.md");
    vec![
        json!({"type": "system", "subtype": "init", "session_id": "native-skillcache-1"}),
        json!({"type": "assistant", "session_id": "native-skillcache-1", "message": {"role": "assistant", "content": [
            {"type": "thinking", "thinking": "PRIVATE REASONING about which invoices to read"},
            {"type": "text", "text": "I will follow the pdf skill."},
            {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"file_path": skill}}
        ]}}),
        json!({"type": "user", "session_id": "native-skillcache-1", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": "     1→# PDF\n     2→\n     3→Use scripts/extract.py.", "is_error": false}
        ]}}),
        json!({"type": "assistant", "session_id": "native-skillcache-1", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "toolu_2", "name": "Bash", "input": {"command": format!(
                "curl -H 'Authorization: Bearer abcdefghijklmnop1234' https://example.invalid/upload && python3 {}/.claude/skills/pdf/scripts/extract.py invoices/",
                project.display()
            )}}
        ]}}),
        json!({"type": "user", "session_id": "native-skillcache-1", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "toolu_2", "content": format!("rows written to {}/out/rows.json", project.display()), "is_error": false}
        ]}}),
        json!({"type": "assistant", "session_id": "native-skillcache-1", "message": {"role": "assistant", "content": [
            {"type": "text", "text": "The totals are in out/rows.json."}
        ]}}),
        json!({"type": "result", "subtype": "success", "is_error": false, "result": "The totals are in out/rows.json.", "session_id": "native-skillcache-1", "total_cost_usd": 0.01}),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_observed_turn_keeps_its_redacted_trace() {
    let stand = Stand::new().await;
    stand.script("claude", &skill_turn(&stand.project()));
    let sid = stand
        .turn(
            "Summarise the invoices in invoices/",
            json!({"mode": "observe"}),
        )
        .await;
    assert_eq!(stand.store.get(&sid).unwrap()["status"], "completed");
    let traces = stand.traces();
    assert_eq!(traces.len(), 1);
    let (session_id, request, skills, steps, redaction) = &traces[0];
    assert_eq!(session_id, &sid);
    assert_eq!(request, "Summarise the invoices in invoices/");
    // The skill the run read is named with the identity of its whole folder.
    assert_eq!(skills.as_array().unwrap().len(), 1, "{skills}");
    assert_eq!(skills[0]["name"], "pdf");
    assert_eq!(skills[0]["path"], ".claude/skills/pdf/SKILL.md");
    assert_eq!(
        skills[0]["tree_sha256"],
        infra_skillcache::fingerprint::tree_sha256(&stand.project().join(".claude/skills/pdf"))
            .unwrap()
            .sha256
            .as_str()
    );
    let kinds = steps
        .as_array()
        .unwrap()
        .iter()
        .map(|step| {
            format!(
                "{}:{}",
                step["kind"].as_str().unwrap(),
                step["tool"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(kinds, ["message:", "tool:Read", "tool:Bash", "message:"]);
    assert_eq!(
        steps[1]["output"],
        "     1→# PDF\n     2→\n     3→Use scripts/extract.py."
    );
    assert_eq!(steps[2]["error"], false);
    // Nothing the trace must not keep is in the row: no reasoning, no
    // credential, no path of this machine.
    let stored = format!("{request}{skills}{steps}");
    assert!(!stored.contains("PRIVATE REASONING"), "{stored}");
    assert!(!stored.contains("abcdefghijklmnop1234"), "{stored}");
    assert!(
        !stored.contains(stand.project().to_str().unwrap()),
        "the trace names files relative to the workspace: {stored}"
    );
    assert!(
        steps[2]["input"]["command"]
            .as_str()
            .unwrap()
            .contains(".claude/skills/pdf/scripts/extract.py invoices/")
    );
    assert_eq!(redaction["reasoning_dropped"], 1);
    assert!(redaction["redacted"].as_u64().unwrap() >= 1, "{redaction}");
    assert_eq!(redaction["complete"], true);
    // The session says what was kept, and the route lists it without steps.
    let events = stand.skillcache_events(&sid);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["phase"], "observed");
    assert_eq!(events[0]["tool_calls"], 2);
    assert_eq!(events[0]["skills"], json!(["pdf"]));
    let (status, listed) = stand
        .http(
            Method::GET,
            &format!("/api/sessions/{sid}/skillcache"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["mode"], "observe");
    assert_eq!(listed["traces"].as_array().unwrap().len(), 1);
    assert_eq!(listed["traces"][0]["steps"], 4);
    assert_eq!(listed["traces"][0]["skill_refs"][0]["name"], "pdf");
    assert!(listed["traces"][0].get("request").is_none());
    assert_eq!(listed["retention"]["per_session"], 10);
    stand.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_is_recorded_when_off_or_warm_only() {
    let stand = Stand::new().await;
    stand.script("claude", &skill_turn(&stand.project()));
    // trace_not_recorded_in_warm_only, and the default: a session that never
    // opted in stores no tool result anywhere in the SkillCache tables.
    for skillcache in [
        Value::Null,
        json!({"mode": "off"}),
        json!({"mode": "warm_only"}),
    ] {
        let sid = stand
            .turn("Summarise the invoices", skillcache.clone())
            .await;
        assert_eq!(stand.store.get(&sid).unwrap()["status"], "completed");
        assert!(stand.traces().is_empty(), "{skillcache} recorded a trace");
        assert!(stand.skillcache_events(&sid).is_empty());
        let (_, listed) = stand
            .http(
                Method::GET,
                &format!("/api/sessions/{sid}/skillcache"),
                None,
            )
            .await;
        assert_eq!(
            listed["mode"],
            skillcache["mode"].as_str().unwrap_or("off"),
            "{listed}"
        );
        assert_eq!(listed["traces"], json!([]));
    }
    stand.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_failed_or_used_no_skill_keeps_no_trace() {
    let stand = Stand::new().await;
    // A provider error: the turn fails, so its trace is not a recipe.
    let mut failed = skill_turn(&stand.project());
    *failed.last_mut().unwrap() = json!({"type": "result", "subtype": "error_during_execution", "is_error": true, "result": "boom", "session_id": "native-skillcache-1"});
    stand.script("claude", &failed);
    let sid = stand
        .turn("Summarise the invoices", json!({"mode": "observe"}))
        .await;
    assert_eq!(stand.store.get(&sid).unwrap()["status"], "failed");
    assert!(stand.traces().is_empty());
    assert!(stand.skillcache_events(&sid).is_empty());
    // Tools, but no skill of the project: nothing a capsule could be made of.
    stand.script(
        "claude",
        &[
            json!({"type": "system", "subtype": "init", "session_id": "native-skillcache-2"}),
            json!({"type": "assistant", "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "ls"}}
            ]}}),
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "toolu_1", "content": "README.md", "is_error": false}
            ]}}),
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "Listed.", "session_id": "native-skillcache-2"}),
        ],
    );
    let sid = stand.turn("List the files", json!({"mode": "auto"})).await;
    assert_eq!(stand.store.get(&sid).unwrap()["status"], "completed");
    assert!(stand.traces().is_empty());
    let events = stand.skillcache_events(&sid);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["phase"], "not_observed");
    assert_eq!(events[0]["reason"], "no_skill");
    // No tool at all.
    stand.script(
        "claude",
        &[
            json!({"type": "system", "subtype": "init", "session_id": "native-skillcache-3"}),
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "Hello.", "session_id": "native-skillcache-3"}),
        ],
    );
    let sid = stand.turn("Say hello", json!({"mode": "observe"})).await;
    assert_eq!(stand.skillcache_events(&sid)[0]["reason"], "no_tool_calls");
    assert!(stand.traces().is_empty());
    stand.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_option_is_validated_and_can_change_between_turns() {
    let stand = Stand::new().await;
    for (skillcache, extra) in [
        (json!({"mode": "always"}), json!({})),
        (
            json!({"mode": "auto", "compiler": {"provider": "codex"}}),
            json!({}),
        ),
        (json!("auto"), json!({})),
        (
            json!({"mode": "observe"}),
            json!({"workflow": "review", "mode": "plan"}),
        ),
    ] {
        let mut options = stand.options("Summarise", skillcache.clone());
        for (key, value) in extra.as_object().unwrap() {
            options[key] = value.clone();
        }
        let (status, refused) = stand
            .http(Method::POST, "/api/sessions", Some(options))
            .await;
        assert_eq!(status, 400, "{skillcache} {extra}: {refused}");
    }
    assert!(stand.store.summaries().unwrap().is_empty());
    // A follow-up turns observation on for itself, and off again.
    stand.script("claude", &skill_turn(&stand.project()));
    let sid = stand.turn("Summarise the invoices", Value::Null).await;
    assert!(stand.traces().is_empty());
    let send = |turn: usize, skillcache: Value| {
        let stand = &stand;
        let sid = sid.clone();
        async move {
            let (status, sent) = stand
                .http(
                    Method::POST,
                    &format!("/api/sessions/{sid}/messages"),
                    Some(json!({"prompt": "Once more", "skillcache": skillcache})),
                )
                .await;
            assert_eq!(status, 200, "{sent}");
            stand.finished(&sid, turn).await
        }
    };
    let session = send(2, json!({"mode": "observe"})).await;
    assert_eq!(session["skillcache"]["mode"], "observe");
    assert_eq!(stand.traces().len(), 1);
    let session = send(3, json!({"mode": "off"})).await;
    assert!(session["skillcache"].is_null());
    assert_eq!(stand.traces().len(), 1, "the off turn recorded nothing");
    let (status, refused) = stand
        .http(
            Method::POST,
            &format!("/api/sessions/{sid}/messages"),
            Some(json!({"prompt": "Again", "skillcache": {"mode": "sometimes"}})),
        )
        .await;
    assert_eq!(status, 400, "{refused}");
    stand.close().await;
}
