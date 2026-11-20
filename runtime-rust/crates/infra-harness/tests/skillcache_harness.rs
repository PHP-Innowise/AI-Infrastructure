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

/// An engine stand-in: it runs the `skillcache` commands, the sandbox checks
/// and the fixture programs of the capsules the tests compile.
const FIXTURE: &str = env!("CARGO_BIN_EXE_infra-harness-skillcache-fixture");

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

/// A provider stand-in. As a session's agent it prints the event stream the
/// test wrote for it. As the SkillCache compiler it prints the answer written
/// for the round its prompt names, unless it can read the project it must
/// not see, in which case it says so instead.
fn scripted(root: &Path, name: &str) -> PathBuf {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let program = bin.join(format!("fake-{name}"));
    let script = format!(
        r#"#!/bin/sh
dir=$(dirname "$0")
prompt=$(cat)
case "$prompt" in
  *"You are the SkillCache compiler."*)
    if [ -r "{project}/invoices/a.txt" ] || ls "{project}" >/dev/null 2>&1; then
      cat "$dir/compiler-leak.jsonl"
      exit 0
    fi
    round=$(printf '%s\n' "$prompt" | sed -n 's/^Round: \([0-9]*\).*/\1/p' | head -n 1)
    cat "$dir/compiler-answer-$round.jsonl";;
  *)
    printf '%s\n' "$@" > "$dir/{name}-arguments.txt"
    printf '%s' "$prompt" > "$dir/{name}-prompt.txt"
    cat "$dir/{name}-events.jsonl";;
esac
"#,
        project = root.join("project").display(),
    );
    // Codex also lists its skills over its app server, and in a warm turn it
    // is the agent that follows the executor's protocol: a test writes that
    // agent as a shell fragment.
    let script = if name == "codex" {
        r#"#!/bin/sh
dir=$(dirname "$0")
case " $* " in
  *" app-server "*)
    while IFS= read -r line; do
      case "$line" in
        *'"method":"initialize"'*) echo '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"fake"}}';;
        *'"method":"skills/list"'*) cat "$dir/codex-skills.json";;
      esac
    done
    exit 0;;
esac
prompt=$(cat)
printf '%s\n' "$@" > "$dir/codex-arguments.txt"
printf '%s' "$prompt" > "$dir/codex-prompt.txt"
env > "$dir/codex-environment.txt"
if [ -f "$dir/codex-agent.sh" ]; then
  . "$dir/codex-agent.sh"
else
  cat "$dir/codex-events.jsonl"
fi
"#
        .to_string()
    } else {
        script
    };
    fs::write(&program, script).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(bin.join(format!("{name}-events.jsonl")), "").unwrap();
    program
}

fn git(folder: &Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(folder)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(arguments)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?}");
}

/// The stand's folder. Capsule packages are sealed read-only, so the folder is
/// given back to its owner before the temporary folder removes itself;
/// otherwise every run would leave it behind.
struct Scratch(tempfile::TempDir);
impl Drop for Scratch {
    fn drop(&mut self) {
        infra_skillcache::package::release(self.0.path());
    }
}

struct Stand {
    _dir: Scratch,
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
        // Below the build folder, not `/tmp`: the agent sandbox mounts a
        // private `/tmp` over whatever a test would put there.
        let dir = Scratch(tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap());
        let root = infra_core::fs::canonicalize(dir.0.path()).unwrap();
        let project = root.join("project");
        let source = root.join("source");
        fs::create_dir_all(project.join(".claude/skills/pdf/scripts")).unwrap();
        fs::create_dir_all(project.join(".agents/skills/pdf")).unwrap();
        fs::write(
            project.join(".agents/skills/pdf/SKILL.md"),
            "---\nname: pdf\ndescription: Work with PDF files\n---\n# PDF\n\nExtract one row per invoice.\n",
        )
        .unwrap();
        fs::create_dir_all(project.join(".agents/skills/other")).unwrap();
        fs::write(
            project.join(".agents/skills/other/SKILL.md"),
            "---\nname: other\ndescription: Another skill\n---\n# Other\n",
        )
        .unwrap();
        fs::create_dir_all(project.join("invoices")).unwrap();
        fs::write(project.join("invoices/a.txt"), "Invoice 1\nTotal: 10.00\n").unwrap();
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
        // A checkout: an observed turn snapshots its starting state from it.
        git(&project, &["init", "-q"]);
        git(&project, &["add", "-A"]);
        git(&project, &["commit", "-q", "-m", "base"]);
        // What Codex answers when the Harness asks it for its skills.
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(
            root.join("bin/codex-skills.json"),
            format!(
                "{}\n",
                json!({"jsonrpc": "2.0", "id": 2, "result": {"data": [{"skills": [
                    {"name": "pdf", "description": "Work with PDF files", "enabled": true, "scope": "repo",
                     "path": project.join(".agents/skills/pdf/SKILL.md")},
                    {"name": "other", "description": "Another skill", "enabled": true, "scope": "repo",
                     "path": project.join(".agents/skills/other/SKILL.md")}
                ]}]}})
            ),
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
        let skillcache_providers = providers.clone();
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
        let skillcache = Arc::new(
            SkillCacheDomain::new(
                store.clone(),
                skillcache_providers,
                sessions.shutdown_cancel(),
            )
            .with_engine(PathBuf::from(FIXTURE)),
        );
        sessions.install_skillcache(skillcache.runtime()).unwrap();
        domains.insert("skillcache".into(), skillcache);
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
            self.root.join(format!("bin/{provider}-events.jsonl")),
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
        // A session that asks for warm turns is told why this one ran cold;
        // one that asks for nothing hears nothing.
        let phases = stand
            .skillcache_events(&sid)
            .iter()
            .map(|event| event["phase"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        if skillcache["mode"] == "warm_only" {
            assert_eq!(phases, ["miss"]);
        } else {
            assert!(phases.is_empty(), "{phases:?}");
        }
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
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["phase"], "miss");
    assert_eq!(events[1]["phase"], "not_observed");
    assert_eq!(events[1]["reason"], "no_skill");
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

// ---------------------------------------------------------------------------
// Compilation and the gates. They run for real: the compiler stand-in inside
// the agent sandbox, the candidate's programs inside the gate sandbox.

fn sandbox_usable() -> bool {
    cfg!(target_os = "linux")
        && std::process::Command::new("bwrap")
            .args([
                "--unshare-pid",
                "--unshare-net",
                "--ro-bind",
                "/",
                "/",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "true",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

fn runtime() -> String {
    Path::new(FIXTURE)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

/// A hybrid answer whose programs are fixture programs: `extract` needs a file
/// of the task and writes a result, `verify` only reads.
fn hybrid_answer() -> Value {
    let program = |id: &str, inputs: Value, outputs: Value, read_only: bool| {
        json!({
            "id": id,
            "description": format!("stage {id}"),
            "entry": [runtime(), format!("programs/{id}/program.json")],
            "inputs": inputs,
            "outputs": outputs,
            "check": [runtime(), format!("programs/{id}/program.json"), "--check"],
            "timeout_seconds": 30,
            "read_only": read_only
        })
    };
    json!({
        "form": "hybrid",
        "capsule": {
            "mu": {"name": "pdf-invoice-rows", "description": "Extract one row per invoice of a folder"},
            "source": {"skills": [{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md"}]},
            "capabilities": {"network": "none", "runtimes": [runtime()]},
            "bindings": [{"name": "input_dir", "type": "path", "origin": "request", "description": "Folder with the invoices"}],
            "programs": [
                program("extract", json!(["input_dir"]), json!(["rows_file"]), false),
                program("verify", json!(["rows_file"]), json!([]), true)
            ],
            "steps": [{"kind": "program", "program": "extract"}],
            "verification": [
                {"kind": "program", "program": "verify"},
                {"kind": "inspection", "text": "Open the rows file and confirm one row per invoice."}
            ]
        },
        "files": [
            {"path": "programs/extract/program.json", "content": json!({
                "result": {"rows_file": "out/rows.json", "invoices": 1},
                "require_file": "invoices/a.txt",
                "write": {"out/rows.json": "[]"}
            }).to_string()},
            {"path": "programs/verify/program.json", "content": json!({
                "result": {"rows": 1}, "require_file": "out/rows.json"
            }).to_string()}
        ],
        "examples": {
            "bindings": {"input_dir": "invoices-2026/"},
            "steps": {"0": {"result": {"rows_file": "observed/rows.json"}}}
        }
    })
}

fn textual_answer() -> Value {
    json!({
        "form": "textual",
        "capsule": {
            "mu": {"name": "pdf-invoice-rows", "description": "Extract one row per invoice of a folder"},
            "source": {"skills": [{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md"}]}
        },
        "markdown": "# Extract invoice rows\n\nWhen to use: a request for the rows of a folder of invoices.\n\n## Steps\n\n1. List the invoices of the folder.\n2. Run the skill's extract script on them.\n\n## Verify\n\n1. Confirm one row per invoice.\n"
    })
}

impl Stand {
    /// What the compiler stand-in answers in `round`.
    fn compiler(&self, round: usize, answer: &Value) {
        let lines = [
            json!({"type": "system", "subtype": "init", "session_id": format!("compiler-{round}")}),
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "",
                "structured_output": answer, "session_id": format!("compiler-{round}"),
                "usage": {"input_tokens": 1000, "output_tokens": 200, "cache_read_input_tokens": 50},
                "total_cost_usd": 0.05, "num_turns": 3}),
        ]
        .map(|line| line.to_string())
        .join("\n");
        fs::write(
            self.root.join(format!("bin/compiler-answer-{round}.jsonl")),
            format!("{lines}\n"),
        )
        .unwrap();
        // What the stand-in prints if the sandbox lets it see the project.
        fs::write(
            self.root.join("bin/compiler-leak.jsonl"),
            format!(
                "{}\n",
                json!({"type": "result", "subtype": "success", "is_error": false, "result": "",
                    "structured_output": {"form": "leaked", "capsule": {}}, "session_id": "leak"})
            ),
        )
        .unwrap();
    }
    /// An observed turn with the `pdf` skill; returns the session and its launch.
    async fn observed(&self) -> (String, String) {
        self.script("claude", &skill_turn(&self.project()));
        let sid = self
            .turn(
                "Extract the rows of invoices-2026/",
                json!({"mode": "observe"}),
            )
            .await;
        let generation = self.skillcache_events(&sid)[0]["generation"]
            .as_str()
            .unwrap()
            .to_string();
        (sid, generation)
    }
    async fn start_compile(&self, sid: &str, generation: &str) -> (String, String) {
        let (status, queued) = self
            .http(
                Method::POST,
                &format!("/api/sessions/{sid}/skillcache/compile"),
                Some(json!({"generation": generation})),
            )
            .await;
        assert_eq!(status, 202, "{queued}");
        assert_eq!(queued["status"], "queued");
        (
            queued["compile_id"].as_str().unwrap().to_string(),
            queued["capsule_id"].as_str().unwrap().to_string(),
        )
    }
    async fn capsule(&self, capsule_id: &str) -> Value {
        let (status, capsule) = self
            .http(
                Method::GET,
                &format!(
                    "/api/projects/{}/skillcache/capsules/{capsule_id}",
                    self.project_id
                ),
                None,
            )
            .await;
        assert_eq!(status, 200, "{capsule}");
        capsule
    }
    /// Waits until the capsule is no longer in `status`.
    async fn settled(&self, capsule_id: &str, status: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let capsule = self.capsule(capsule_id).await;
            let compiling = capsule["compile"]["status"] == "queued"
                || capsule["compile"]["status"] == "running";
            if capsule["status"] != status && !compiling {
                return capsule;
            }
            assert!(
                Instant::now() < deadline,
                "the capsule stayed {status}: {capsule}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    fn count(&self, sql: &str) -> i64 {
        self.store
            .connection()
            .unwrap()
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }
}

fn gate_summary(capsule: &Value) -> Vec<String> {
    capsule["gates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|gate| {
            format!(
                "{}:{}:{}",
                gate["round"],
                gate["gate"].as_str().unwrap(),
                if gate["passed"] == true {
                    "pass"
                } else {
                    "fail"
                }
            )
        })
        .collect()
}
fn violation_codes(capsule: &Value, round: i64, gate: &str) -> Vec<String> {
    capsule["gates"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["round"] == round && row["gate"] == gate)
        .flat_map(|row| {
            row["report"]["violations"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .map(|violation| violation["code"].as_str().unwrap_or("").to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_observed_turn_compiles_into_a_qualified_capsule() {
    if !sandbox_usable() {
        eprintln!("skipped: no usable bubblewrap sandbox on this host");
        return;
    }
    let stand = Stand::new().await;
    let (sid, generation) = stand.observed().await;
    let launches = stand.count("SELECT COUNT(*) FROM launches");
    stand.compiler(1, &hybrid_answer());
    let (compile_id, capsule_id) = stand.start_compile(&sid, &generation).await;
    let capsule = stand.settled(&capsule_id, "compiling").await;
    assert_eq!(capsule["status"], "active", "{capsule}");
    assert_eq!(capsule["form"], "hybrid");
    assert_eq!(capsule["mu"]["name"], "pdf-invoice-rows");
    assert_eq!(capsule["skill_name"], "pdf");
    assert_eq!(capsule["skill_path"], ".claude/skills/pdf/SKILL.md");
    // One round: all three gates, none of them with a model.
    assert_eq!(
        gate_summary(&capsule),
        ["1:gate1:pass", "1:gate2:pass", "1:gate3:pass"]
    );
    let replay = &capsule["gates"][2]["report"];
    assert_eq!(replay["completed"], json!([0, 1, 2]), "{replay}");
    assert_eq!(replay["substituted"], json!([]));
    assert_eq!(replay["network"], "none");
    // The plan is shown; the programs' sources and the observed values are not.
    assert_eq!(capsule["capsule"]["steps"][0]["program"], "extract");
    assert_eq!(capsule["capsule"]["source"]["skills"][0]["name"], "pdf");
    assert_eq!(capsule["files"].as_array().unwrap().len(), 2);
    let shown = capsule.to_string();
    assert!(
        !shown.contains("invoices-2026/") && !shown.contains("observed/rows.json"),
        "{shown}"
    );
    assert!(capsule.get("artifact_dir").is_none());
    // The compilation is accounted apart from the session's turns:
    // history_excludes_compiles.
    assert_eq!(capsule["compile"]["compile_id"], compile_id.as_str());
    assert_eq!(capsule["compile"]["status"], "completed");
    assert_eq!(capsule["compile"]["rounds"], 1);
    assert_eq!(capsule["compile"]["usage"]["input_tokens"], 1000);
    assert_eq!(capsule["compile"]["usage"]["cost_usd"], 0.05);
    assert!(capsule["compile"]["usage"]["cache_creation_input_tokens"].is_null());
    assert_eq!(
        capsule["compile"]["receipt"]["rounds"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(stand.count("SELECT COUNT(*) FROM launches"), launches);
    // The warm package holds no observed value; the replay left the real
    // workspace alone and its folders are gone.
    let state = stand.root.join("state/skillcache");
    let warm = state.join(format!("capsules/{}/{capsule_id}/1/warm", stand.project_id));
    let files = infra_skillcache::fingerprint::stat_tree(&warm)
        .unwrap()
        .into_iter()
        .map(|file| file.path)
        .collect::<Vec<_>>();
    assert_eq!(
        files,
        [
            "capsule.json",
            "programs/extract/program.json",
            "programs/verify/program.json"
        ]
    );
    for file in &files {
        let text = fs::read_to_string(warm.join(file)).unwrap();
        assert!(!text.contains("invoices-2026/") && !text.contains("observed/rows.json"));
    }
    assert!(
        state
            .join(format!("qualification/{capsule_id}/1/examples.json"))
            .is_file()
    );
    assert!(!stand.project().join("out/rows.json").exists());
    assert!(!state.join("compile").join(&compile_id).exists());
    // The session and the project both list it.
    let (_, listed) = stand
        .http(
            Method::GET,
            &format!("/api/sessions/{sid}/skillcache"),
            None,
        )
        .await;
    assert_eq!(listed["traces"][0]["replayable"], true);
    assert_eq!(listed["traces"][0]["compiling"], false);
    assert_eq!(listed["compiles"][0]["status"], "completed");
    assert_eq!(listed["capsules"][0]["capsule_id"], capsule_id.as_str());
    let (_, project) = stand
        .http(
            Method::GET,
            &format!(
                "/api/projects/{}/skillcache/capsules?status=active&form=hybrid&skill=pdf",
                stand.project_id
            ),
            None,
        )
        .await;
    assert_eq!(
        project["capsules"].as_array().unwrap().len(),
        1,
        "{project}"
    );
    let (_, none) = stand
        .http(
            Method::GET,
            &format!(
                "/api/projects/{}/skillcache/capsules?form=textual",
                stand.project_id
            ),
            None,
        )
        .await;
    assert_eq!(none["capsules"], json!([]));
    // Requests outside the contract.
    let (status, _) = stand
        .http(
            Method::POST,
            &format!("/api/sessions/{sid}/skillcache/compile"),
            Some(json!({"generation": "no-such-launch"})),
        )
        .await;
    assert_eq!(status, 404);
    let (status, _) = stand
        .http(
            Method::POST,
            &format!("/api/sessions/{sid}/skillcache/compile"),
            Some(json!({"skill": "pdf"})),
        )
        .await;
    assert_eq!(
        status, 400,
        "a capsule is compiled from an observed turn, not from a skill name"
    );
    let (status, _) = stand
        .http(
            Method::POST,
            &format!("/api/sessions/{sid}/skillcache/compile"),
            Some(json!({"generation": generation, "compiler": {"provider": "codex"}})),
        )
        .await;
    assert_eq!(status, 400);
    stand.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn compile_rounds_then_textual_then_source() {
    if !sandbox_usable() {
        eprintln!("skipped: no usable bubblewrap sandbox on this host");
        return;
    }
    let stand = Stand::new().await;
    let (sid, generation) = stand.observed().await;
    // Round 1 fails Gate 1: a program reads a value nothing defines.
    let mut first = hybrid_answer();
    first["capsule"]["programs"][0]["inputs"] = json!(["input_dir", "nowhere"]);
    stand.compiler(1, &first);
    // Round 2 fails Gate 2: a program's own check exits non-zero.
    let mut second = hybrid_answer();
    second["files"][0]["content"] = json!({"result": {"rows_file": "x"}, "check_exit": 1})
        .to_string()
        .into();
    stand.compiler(2, &second);
    // Round 3 fails Gate 3 twice over: one program fails, and the read-only
    // verification program writes. One replay reports both.
    let mut third = hybrid_answer();
    third["files"][0]["content"] = json!({"exit": 3, "stderr": "extract cannot parse the invoice"})
        .to_string()
        .into();
    third["files"][1]["content"] = json!({"result": {"rows": 1}, "write": {"verified.txt": "x"}})
        .to_string()
        .into();
    stand.compiler(3, &third);
    stand.compiler(4, &textual_answer());
    let (_, capsule_id) = stand.start_compile(&sid, &generation).await;
    let capsule = stand.settled(&capsule_id, "compiling").await;
    assert_eq!(capsule["status"], "active", "{capsule}");
    assert_eq!(capsule["form"], "textual");
    assert_eq!(
        gate_summary(&capsule),
        [
            "1:gate1:fail",
            "2:gate1:pass",
            "2:gate2:fail",
            "3:gate1:pass",
            "3:gate2:pass",
            "3:gate3:fail",
            "4:gate1t:pass"
        ]
    );
    assert_eq!(violation_codes(&capsule, 1, "gate1"), ["dangling_input"]);
    assert_eq!(violation_codes(&capsule, 2, "gate2"), ["check_failed"]);
    // gate3_substitutes_examples_and_collects_downstream, through the Harness.
    assert_eq!(
        violation_codes(&capsule, 3, "gate3"),
        ["nonzero_exit", "read_only_violation"]
    );
    let replay = capsule["gates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["gate"] == "gate3")
        .unwrap();
    assert_eq!(replay["report"]["substituted"], json!([0, 1]));
    assert!(
        replay["report"]["violations"][0]["message"]
            .as_str()
            .unwrap()
            .contains("extract cannot parse the invoice")
    );
    assert_eq!(capsule["compile"]["rounds"], 4);
    assert_eq!(capsule["compile"]["usage"]["input_tokens"], 4000);
    assert!(
        capsule["compile"]["outcome"]
            .as_str()
            .unwrap()
            .contains("written procedure")
    );
    assert!(
        capsule["markdown"]
            .as_str()
            .unwrap()
            .starts_with("# Extract invoice rows")
    );
    assert!(capsule["capsule"].get("programs").is_none());
    let warm = stand.root.join(format!(
        "state/skillcache/capsules/{}/{capsule_id}/1/warm",
        stand.project_id
    ));
    assert!(warm.join("capsule.md").is_file() && !warm.join("programs").exists());
    // When the textual form is refused as well, nothing is compiled: the
    // skill is used as it is.
    let mut bad = textual_answer();
    bad["markdown"] = "No steps here.".into();
    stand.compiler(4, &bad);
    let (compile_id, failed_id) = stand.start_compile(&sid, &generation).await;
    let failed = stand.settled(&failed_id, "compiling").await;
    assert_eq!(failed["status"], "failed", "{failed}");
    assert_eq!(failed["compile"]["status"], "failed");
    assert_eq!(failed["compile"]["compile_id"], compile_id.as_str());
    assert!(
        failed["reason"]
            .as_str()
            .unwrap()
            .contains("No form qualified")
    );
    assert_eq!(
        violation_codes(&failed, 4, "gate1t"),
        ["invalid_markdown", "invalid_markdown"]
    );
    assert!(failed["capsule"].is_null());
    stand.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_capsule_is_invalidated_requalified_goes_stale_and_is_deleted() {
    if !sandbox_usable() {
        eprintln!("skipped: no usable bubblewrap sandbox on this host");
        return;
    }
    let stand = Stand::new().await;
    let (sid, generation) = stand.observed().await;
    stand.compiler(1, &hybrid_answer());
    let (_, capsule_id) = stand.start_compile(&sid, &generation).await;
    assert_eq!(
        stand.settled(&capsule_id, "compiling").await["status"],
        "active"
    );
    let route = format!(
        "/api/projects/{}/skillcache/capsules/{capsule_id}",
        stand.project_id
    );
    // A person takes it out of use; the gates alone bring it back.
    let (status, invalidated) = stand
        .http(
            Method::POST,
            &format!("{route}/invalidate"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 200, "{invalidated}");
    assert_eq!(invalidated["status"], "stale");
    let (status, queued) = stand
        .http(Method::POST, &format!("{route}/requalify"), Some(json!({})))
        .await;
    assert_eq!(status, 202, "{queued}");
    let capsule = stand.settled(&capsule_id, "stale").await;
    assert_eq!(capsule["status"], "active", "{capsule}");
    assert_eq!(
        gate_summary(&capsule)[3..],
        ["2:gate1:pass", "2:gate2:pass", "2:gate3:pass"]
    );
    assert_eq!(
        capsule["compile"]["rounds"], 1,
        "a requalification calls no model"
    );
    // fingerprint_detects_script_change, as a person sees it: any file of the
    // skill, not only its entry document.
    fs::write(
        stand
            .project()
            .join(".claude/skills/pdf/scripts/extract.py"),
        "print('changed')\n",
    )
    .unwrap();
    let stale = stand.capsule(&capsule_id).await;
    assert_eq!(stale["status"], "stale");
    assert!(
        stale["reason"].as_str().unwrap().contains("`pdf` changed"),
        "{stale}"
    );
    // A changed skill is not requalified: its capsule describes the old one.
    let (status, _) = stand
        .http(Method::POST, &format!("{route}/requalify"), Some(json!({})))
        .await;
    assert_eq!(status, 202);
    let deadline = Instant::now() + Duration::from_secs(60);
    let refused = loop {
        let capsule = stand.capsule(&capsule_id).await;
        if capsule["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("Observe a new turn"))
        {
            break capsule;
        }
        assert!(Instant::now() < deadline, "{capsule}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(refused["status"], "stale");
    // Deleting it removes its files, its trace and its snapshot.
    let state = stand.root.join("state/skillcache");
    assert!(state.join("snapshots").join(&generation).is_dir());
    let (status, deleted) = stand.http(Method::DELETE, &route, None).await;
    assert_eq!(status, 200, "{deleted}");
    assert!(
        !state
            .join(format!("capsules/{}/{capsule_id}", stand.project_id))
            .exists()
    );
    assert!(!state.join(format!("qualification/{capsule_id}")).exists());
    assert!(!state.join("snapshots").join(&generation).exists());
    assert!(stand.traces().is_empty());
    assert_eq!(stand.count("SELECT COUNT(*) FROM skillcache_capsules"), 0);
    assert_eq!(stand.count("SELECT COUNT(*) FROM skillcache_gates"), 0);
    let (status, _) = stand.http(Method::GET, &route, None).await;
    assert_eq!(status, 404);
    // A capsule of another project is not reachable through this one.
    let (status, _) = stand
        .http(
            Method::GET,
            &format!(
                "/api/projects/{}/skillcache/capsules/unknown",
                stand.project_id
            ),
            None,
        )
        .await;
    assert_eq!(status, 404);
    stand.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changing_routes_need_the_token() {
    let stand = Stand::new().await;
    for (method, path) in [
        (
            Method::POST,
            "/api/sessions/s/skillcache/compile".to_string(),
        ),
        (
            Method::POST,
            format!(
                "/api/projects/{}/skillcache/capsules/c/requalify",
                stand.project_id
            ),
        ),
        (
            Method::POST,
            format!(
                "/api/projects/{}/skillcache/capsules/c/invalidate",
                stand.project_id
            ),
        ),
        (
            Method::DELETE,
            format!("/api/projects/{}/skillcache/capsules/c", stand.project_id),
        ),
    ] {
        let response = stand
            .client
            .request(
                method.clone(),
                format!("http://127.0.0.1:{}{path}", stand.port),
            )
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 403, "{method} {path}");
    }
    stand.close().await;
}

// ---------------------------------------------------------------------------
// Warm turns. Codex is the provider: its `$skill` mention is what routes a
// message to exactly one skill. The stand-in agent follows the executor's
// protocol with the staged `skillcache-run`, so the capsule's programs run
// for real in the session's workspace.

impl Stand {
    /// A Codex turn of a new session; `agent` is the shell the stand-in runs
    /// as the agent, or its scripted events when there is none.
    async fn codex_turn(&self, prompt: &str, options: Value, agent: Option<&str>) -> String {
        let script = self.root.join("bin/codex-agent.sh");
        match agent {
            Some(agent) => fs::write(&script, agent).unwrap(),
            None => {
                let _ = fs::remove_file(&script);
            }
        }
        let mut body = json!({
            "project_id": self.project_id,
            "provider": "codex",
            "mode": "edit",
            "workflow": "native",
            "project_context": false,
            "permissions": "auto",
            "prompt": prompt,
        });
        for (key, value) in options.as_object().unwrap() {
            body[key] = value.clone();
        }
        let (status, created) = self.http(Method::POST, "/api/sessions", Some(body)).await;
        assert_eq!(status, 201, "{created}");
        let sid = created["session"]["id"].as_str().unwrap().to_owned();
        self.finished(&sid, 1).await;
        sid
    }
    fn phases(&self, sid: &str) -> Vec<String> {
        self.skillcache_events(sid)
            .iter()
            .map(|event| event["phase"].as_str().unwrap().to_string())
            .collect()
    }
    fn recorded(&self, name: &str) -> String {
        fs::read_to_string(self.root.join("bin").join(name)).unwrap_or_default()
    }
}

/// A cold Codex turn that loads the project's `pdf` skill and runs a command.
fn codex_skill_turn() -> Vec<Value> {
    vec![
        json!({"type": "thread.started", "thread_id": "0199-cold"}),
        json!({"type": "turn.started"}),
        json!({"type": "item.completed", "item": {"id": "i1", "type": "command_execution",
            "command": "bash -lc 'cat .agents/skills/pdf/SKILL.md'", "aggregated_output": "# PDF\n",
            "exit_code": 0, "status": "completed"}}),
        json!({"type": "item.completed", "item": {"id": "i2", "type": "command_execution",
            "command": "python3 extract.py invoices/", "aggregated_output": "rows written",
            "exit_code": 0, "status": "completed"}}),
        json!({"type": "item.completed", "item": {"id": "i3", "type": "agent_message", "text": "Rows extracted."}}),
        json!({"type": "turn.completed", "usage": {"input_tokens": 10, "cached_input_tokens": 0, "output_tokens": 5}}),
    ]
}

/// The agent of a warm turn: it starts the plan with the value the request
/// states, does the inspection it is handed, and answers.
fn warm_agent(root: &Path, by_hand: &str) -> String {
    let fixture = Path::new(FIXTURE).parent().unwrap().display().to_string();
    let bin = root.join("bin").display().to_string();
    format!(
        r#"PATH="{fixture}:$PATH"
echo '{{"type":"thread.started","thread_id":"0199-warm"}}'
echo '{{"type":"turn.started"}}'
call() {{
  number=$1; shift
  answer=$("$@")
  printf '%s\n' "$answer" > "{bin}/warm-answer-$number.json"
  echo "{{\"type\":\"item.completed\",\"item\":{{\"id\":\"w$number\",\"type\":\"command_execution\",\"command\":\"skillcache-run\",\"aggregated_output\":\"\",\"exit_code\":0,\"status\":\"completed\"}}}}"
}}
call 1 skillcache-run run '{{"input_dir":"invoices/"}}'
{by_hand}
call 3 skillcache-run continue
echo '{{"type":"item.completed","item":{{"id":"done","type":"agent_message","text":"Rows extracted."}}}}'
echo '{{"type":"turn.completed","usage":{{"input_tokens":10,"cached_input_tokens":0,"output_tokens":5}}}}'
"#
    )
}

impl Stand {
    /// Observes a cold Codex turn with `$pdf` and compiles it; returns the
    /// capsule and the session it came from.
    async fn compiled_capsule(&self, answer: &Value) -> (String, String) {
        self.script("codex", &codex_skill_turn());
        let sid = self
            .codex_turn(
                "$pdf extract the rows of invoices/",
                json!({"skillcache": {"mode": "auto"}}),
                None,
            )
            .await;
        let events = self.skillcache_events(&sid);
        // No capsule yet: the turn is cold, and in `auto` it is observed.
        assert_eq!(events[0]["phase"], "miss", "{events:?}");
        assert_eq!(events[0]["reason"], "no_capsule");
        assert_eq!(events[1]["phase"], "observed", "{events:?}");
        let generation = events[1]["generation"].as_str().unwrap().to_string();
        self.compiler(1, answer);
        let (_, capsule_id) = self.start_compile(&sid, &generation).await;
        let capsule = self.settled(&capsule_id, "compiling").await;
        assert_eq!(capsule["status"], "active", "{capsule}");
        assert_eq!(capsule["skill_path"], ".agents/skills/pdf/SKILL.md");
        (capsule_id, sid)
    }
}

fn codex_answer() -> Value {
    let mut answer = hybrid_answer();
    answer["capsule"]["source"]["skills"][0]["path"] = ".agents/skills/pdf/SKILL.md".into();
    answer
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_warm_turn_runs_the_capsule_instead_of_reading_the_skill() {
    if !sandbox_usable() {
        eprintln!("skipped: no usable bubblewrap sandbox on this host");
        return;
    }
    let stand = Stand::new().await;
    let (capsule_id, cold) = stand.compiled_capsule(&codex_answer()).await;
    // The cold turn's prompt asked the agent to load and follow the skill.
    let cold_prompt = stand.recorded("codex-prompt.txt");
    assert!(
        cold_prompt.contains("Harness skill request"),
        "{cold_prompt}"
    );
    let traces = stand.traces().len();
    // The composer's preflight says what a turn would do before it is sent,
    // and making that preview prepares nothing.
    let preview = |skillcache: Value| {
        let stand = &stand;
        async move {
            let (status, preflight) = stand
                .http(
                    Method::POST,
                    &format!("/api/projects/{}/preflight", stand.project_id),
                    Some(json!({
                        "prompt": "$pdf extract the rows of invoices/",
                        "risk_caller_hints": false,
                        "skillcache": skillcache,
                    })),
                )
                .await;
            assert_eq!(status, 200, "{preflight}");
            preflight["skillcache"].clone()
        }
    };
    let composer = |mode: &str, session_mode: &str| json!({"mode": mode, "provider": "codex", "session_mode": session_mode, "permissions": "auto"});
    let seen = preview(composer("auto", "edit")).await;
    assert_eq!(seen["decision"], "hit", "{seen}");
    assert_eq!(seen["capsule"]["capsule_id"], capsule_id.as_str());
    assert_eq!(seen["capsule"]["mu"], "pdf-invoice-rows");
    assert_eq!(
        preview(composer("auto", "plan")).await["reason"],
        "plan_mode"
    );
    assert_eq!(
        preview(composer("observe", "edit")).await["decision"],
        "off"
    );
    assert!(
        !stand.root.join("state/skillcache/bin").exists(),
        "a preview stages nothing"
    );
    assert!(!stand.root.join("state/skillcache/sessions").exists());
    let (_, plain) = stand
        .http(
            Method::POST,
            &format!("/api/projects/{}/preflight", stand.project_id),
            Some(json!({"prompt": "$pdf extract the rows", "risk_caller_hints": false})),
        )
        .await;
    assert!(
        plain.get("skillcache").is_none(),
        "a preflight that does not ask is unchanged"
    );
    let sid = stand
        .codex_turn(
            "$pdf extract the rows of invoices/",
            json!({"skillcache": {"mode": "auto"}}),
            Some(&warm_agent(&stand.root, "call 2 skillcache-run status")),
        )
        .await;
    assert_eq!(stand.store.get(&sid).unwrap()["status"], "completed");
    assert_eq!(
        stand.phases(&sid),
        [
            "hit",
            "started",
            "program_completed",
            "program_completed",
            "your_steps",
            "complete"
        ]
    );
    let hit = &stand.skillcache_events(&sid)[0];
    assert_eq!(hit["capsule_id"], capsule_id.as_str());
    assert_eq!(hit["mu"], "pdf-invoice-rows");
    // The prompt carries the capsule's block where the skill request was, and
    // nothing that was observed, no program and no place of the package.
    let prompt = stand.recorded("codex-prompt.txt");
    assert!(
        prompt.contains("SkillCache capsule for this request: pdf-invoice-rows"),
        "{prompt}"
    );
    assert!(prompt.contains("`input_dir` (path): Folder with the invoices"));
    assert!(prompt.contains("skillcache-run run"));
    assert!(!prompt.contains("Harness skill request"), "{prompt}");
    for withheld in [
        "invoices-2026/",
        "observed/rows.json",
        "programs/extract",
        "program.json",
        "skillcache/capsules",
        &capsule_id,
    ] {
        assert!(
            !prompt.contains(withheld),
            "the warm prompt shows {withheld:?}:\n{prompt}"
        );
    }
    // The executor answered as its protocol says, and its programs ran in the
    // session's own workspace.
    let first: Value = serde_json::from_str(&stand.recorded("warm-answer-1.json")).unwrap();
    assert_eq!(first["status"], "your_steps", "{first}");
    assert_eq!(first["steps"][0]["kind"], "inspection");
    assert_eq!(first["results"]["extract"]["invoices"], 1);
    assert_eq!(first["results"]["verify"]["rows"], 1);
    let last: Value = serde_json::from_str(&stand.recorded("warm-answer-3.json")).unwrap();
    assert_eq!(last["status"], "complete", "{last}");
    assert_eq!(last["failed"], json!([]));
    assert!(stand.project().join("out/rows.json").is_file());
    // What the provider's process was started with.
    let environment = stand.recorded("codex-environment.txt");
    let variable = |name: &str| {
        environment
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .unwrap_or_default()
            .to_string()
    };
    let state = stand.root.join("state/skillcache");
    assert!(
        variable("PATH").starts_with(state.join("bin").to_str().unwrap()),
        "{}",
        variable("PATH")
    );
    assert!(variable("SKILLCACHE_PACKAGE").ends_with(&format!("{capsule_id}/1/warm")));
    assert_eq!(
        variable("SKILLCACHE_WORKSPACE"),
        stand.project().to_str().unwrap()
    );
    let state_file = variable("SKILLCACHE_STATE_FILE");
    assert!(
        state_file.contains(&format!("skillcache/sessions/{sid}/")),
        "{state_file}"
    );
    // Codex writes only where it is told it may: the run's state folder.
    let arguments = stand.recorded("codex-arguments.txt");
    let lines = arguments.lines().collect::<Vec<_>>();
    let at = lines
        .iter()
        .position(|line| *line == "--add-dir")
        .expect("--add-dir");
    assert_eq!(
        Path::new(lines[at + 1]),
        Path::new(&state_file).parent().unwrap()
    );
    // The turn is recorded as a run of the capsule, not as a new observation.
    assert_eq!(
        stand.traces().len(),
        traces,
        "a warm turn is not an observed trace"
    );
    let (_, listed) = stand
        .http(
            Method::GET,
            &format!("/api/sessions/{sid}/skillcache"),
            None,
        )
        .await;
    assert_eq!(listed["runs"][0]["capsule_id"], capsule_id.as_str());
    assert_eq!(listed["runs"][0]["outcome"], "complete");
    assert_eq!(listed["runs"][0]["deopts"], 0);
    let capsule = stand.capsule(&capsule_id).await;
    assert_eq!(capsule["runs"]["count"], 1);
    assert_eq!(capsule["runs"]["complete"], 1);
    assert_eq!(capsule["runs"]["handed_back"], 0);
    assert_eq!(capsule["runs"]["deopts"], 0);
    assert_eq!(capsule["runs"]["recent"][0]["session_id"], sid.as_str());
    assert_eq!(capsule["runs"]["recent"][0]["outcome"], "complete");
    // What compiling cost is known; what a reuse saves is not, and is not
    // shown as a zero.
    assert_eq!(capsule["amortization"]["compile_usd"], 0.05);
    assert_eq!(capsule["amortization"]["warm_turns"], 1);
    assert!(capsule["amortization"]["saving_per_reuse_usd"].is_null());
    assert!(capsule["amortization"]["breakeven_reuses"].is_null());
    assert!(capsule["last_used_at"].is_string());
    let recorded: String = stand
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT settings FROM launches WHERE session_id=?1",
            [&sid],
            |row| row.get(0),
        )
        .unwrap();
    let settings: Value = serde_json::from_str(&recorded).unwrap();
    assert_eq!(settings["skillcache"]["decision"], "hit");
    assert_eq!(settings["skillcache"]["capsule_id"], capsule_id.as_str());
    // prompt_hit_ledger: the block is accounted as its own category.
    let context: Option<String> = stand
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT context FROM launches WHERE session_id=?1",
            [&sid],
            |row| row.get(0),
        )
        .unwrap();
    let context: Value = serde_json::from_str(&context.unwrap_or_default()).unwrap_or(Value::Null);
    assert!(
        context.to_string().contains("\"skillcache\":"),
        "the launch's prompt ledger has no skillcache category: {context}"
    );
    // The run's working state is gone with the turn.
    assert!(!Path::new(&state_file).parent().unwrap().exists());
    // The cold session's launch says it was a miss.
    let cold_settings: String = stand
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT settings FROM launches WHERE session_id=?1",
            [&cold],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        cold_settings.contains("\"decision\":\"miss\""),
        "{cold_settings}"
    );
    stand.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_program_hands_one_step_back_and_the_plan_goes_on() {
    if !sandbox_usable() {
        eprintln!("skipped: no usable bubblewrap sandbox on this host");
        return;
    }
    let stand = Stand::new().await;
    let (capsule_id, _) = stand.compiled_capsule(&codex_answer()).await;
    // The project drifted since the capsule was qualified: the file its first
    // program needs is gone. The capsule itself is unchanged and still active.
    fs::remove_file(stand.project().join("invoices/a.txt")).unwrap();
    let by_hand = format!(
        r#"mkdir -p out && echo '[]' > out/rows.json
refused=$(skillcache-tool extract --input '{{"input_dir":"invoices/"}}' 2>&1 >/dev/null; echo "exit=$?")
printf '%s\n' "$refused" > "{}/warm-tool.txt"
call 2 skillcache-run continue '{{"rows_file":"out/rows.json"}}'"#,
        stand.root.join("bin").display()
    );
    let sid = stand
        .codex_turn(
            "$pdf extract the rows of invoices/",
            json!({"skillcache": {"mode": "warm_only"}}),
            Some(&warm_agent(&stand.root, &by_hand)),
        )
        .await;
    assert_eq!(stand.store.get(&sid).unwrap()["status"], "completed");
    assert_eq!(
        stand.phases(&sid),
        [
            "hit",
            "started",
            "program_failed",
            "program_completed",
            "your_steps",
            "complete"
        ]
    );
    // The first answer hands exactly that step to the agent, with the source
    // of a plan step and the output the next program reads.
    let first: Value = serde_json::from_str(&stand.recorded("warm-answer-1.json")).unwrap();
    assert_eq!(first["status"], "program_failed", "{first}");
    assert_eq!(first["step_index"], 0);
    assert_eq!(first["diagnostics"]["reason"], "nonzero_exit");
    assert_eq!(first["diagnostics"]["attempts"], 2);
    assert_eq!(first["required_outputs"], json!(["rows_file"]));
    assert!(
        first["program_source_path"]
            .as_str()
            .unwrap()
            .ends_with("programs/extract/program.json")
    );
    assert!(
        first["instruction"]
            .as_str()
            .unwrap()
            .contains(".agents/skills/pdf/SKILL.md")
    );
    // The failed program is not run again by hand either.
    let tool = stand.recorded("warm-tool.txt");
    assert!(
        tool.contains("refused_failed_program") && tool.contains("exit=2"),
        "{tool}"
    );
    // The rest of the plan stayed compiled: the verification program ran.
    let second: Value = serde_json::from_str(&stand.recorded("warm-answer-2.json")).unwrap();
    assert_eq!(second["status"], "your_steps", "{second}");
    assert_eq!(second["results"]["verify"]["rows"], 1);
    let last: Value = serde_json::from_str(&stand.recorded("warm-answer-3.json")).unwrap();
    assert_eq!(
        last["failed"],
        json!([{"step_index": 0, "reason": "nonzero_exit"}])
    );
    let failed = stand
        .skillcache_events(&sid)
        .into_iter()
        .find(|event| event["phase"] == "program_failed")
        .unwrap();
    assert_eq!(failed["program"], "extract");
    assert_eq!(failed["reason"], "nonzero_exit");
    assert_eq!(failed["verification"], false);
    let capsule = stand.capsule(&capsule_id).await;
    assert_eq!(capsule["runs"]["count"], 1);
    assert_eq!(capsule["runs"]["handed_back"], 1);
    assert_eq!(capsule["runs"]["deopts"], 1);
    // A deoptimized step is a property of that run; the capsule stays active.
    assert_eq!(capsule["status"], "active");
    // warm_only records nothing, hit or not.
    assert_eq!(stand.traces().len(), 1);

    // recompile_thresholds_are_off_until_set: with the setting, enough turns
    // that handed a step back make the capsule wait for a person. The numbers
    // are the setting's own; the Harness has none.
    stand
        .store
        .set_setting(
            "skillcache.recompile_thresholds",
            &json!({"window": 3, "failed_runs": 2}),
        )
        .unwrap();
    let again = stand
        .codex_turn(
            "$pdf extract the rows of invoices/",
            json!({"skillcache": {"mode": "warm_only"}}),
            Some(&warm_agent(&stand.root, &by_hand)),
        )
        .await;
    assert_eq!(stand.phases(&again).last().unwrap(), "needs_recompile");
    let capsule = stand.capsule(&capsule_id).await;
    assert_eq!(capsule["status"], "needs_recompile", "{capsule}");
    assert_eq!(
        capsule["reason"],
        "2 of its last 2 warm turns handed a program step back."
    );
    assert_eq!(capsule["runs"]["handed_back"], 2);
    // It is not used again until someone requalifies it or compiles anew.
    stand.script("codex", &codex_skill_turn());
    let cold = stand
        .codex_turn(
            "$pdf extract the rows of invoices/",
            json!({"skillcache": {"mode": "warm_only"}}),
            None,
        )
        .await;
    let events = stand.skillcache_events(&cold);
    assert_eq!(events[0]["phase"], "miss", "{events:?}");
    assert_eq!(events[0]["reason"], "no_capsule");
    stand.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_outside_what_a_capsule_was_qualified_for_runs_cold() {
    if !sandbox_usable() {
        eprintln!("skipped: no usable bubblewrap sandbox on this host");
        return;
    }
    let stand = Stand::new().await;
    let (capsule_id, _) = stand.compiled_capsule(&codex_answer()).await;
    stand.script("codex", &codex_skill_turn());
    let cold = |prompt: &'static str, options: Value| {
        let stand = &stand;
        async move {
            let sid = stand.codex_turn(prompt, options, None).await;
            assert_eq!(stand.store.get(&sid).unwrap()["status"], "completed");
            let events = stand.skillcache_events(&sid);
            let prompt = stand.recorded("codex-prompt.txt");
            assert!(
                !prompt.contains("SkillCache capsule"),
                "a miss changes nothing of the prompt"
            );
            (events, prompt)
        }
    };
    // warm_requires_edit_mode
    let (events, prompt) = cold(
        "$pdf extract the rows of invoices/",
        json!({"skillcache": {"mode": "warm_only"}, "mode": "plan"}),
    )
    .await;
    assert_eq!(events[0]["reason"], "plan_mode", "{events:?}");
    assert!(prompt.contains("Harness skill request"));
    // warm_single_routed_skill: two skills named, and none.
    let (events, _) = cold(
        "$pdf and $other extract the rows",
        json!({"skillcache": {"mode": "warm_only"}}),
    )
    .await;
    assert_eq!(events[0]["reason"], "no_single_skill", "{events:?}");
    let (events, _) = cold(
        "extract the rows of invoices/",
        json!({"skillcache": {"mode": "warm_only"}}),
    )
    .await;
    assert_eq!(events[0]["reason"], "no_single_skill");
    // A skill without a capsule.
    let (events, _) = cold("$other do it", json!({"skillcache": {"mode": "warm_only"}})).await;
    assert_eq!(events[0]["reason"], "no_capsule");
    // A session that did not ask for warm turns is not looked at.
    let (events, untouched) = cold("$pdf extract the rows of invoices/", json!({})).await;
    assert!(events.is_empty(), "{events:?}");
    let (events, _) = cold(
        "$pdf extract the rows of invoices/",
        json!({"skillcache": {"mode": "observe"}}),
    )
    .await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["phase"], "observed");
    // warm_skips_stale_capsule: the skill's text changed.
    fs::write(
        stand.project().join(".agents/skills/pdf/SKILL.md"),
        "---\nname: pdf\ndescription: Work with PDF files\n---\n# PDF\n\nNow with a new rule.\n",
    )
    .unwrap();
    // A preview notices it and changes nothing: only a turn marks a capsule.
    let (_, preflight) = stand
        .http(
            Method::POST,
            &format!("/api/projects/{}/preflight", stand.project_id),
            Some(json!({
                "prompt": "$pdf extract the rows of invoices/",
                "risk_caller_hints": false,
                "skillcache": {"mode": "auto", "provider": "codex", "session_mode": "edit", "permissions": "auto"},
            })),
        )
        .await;
    assert_eq!(preflight["skillcache"]["reason"], "stale", "{preflight}");
    assert_eq!(
        stand.count("SELECT COUNT(*) FROM skillcache_capsules WHERE status='active'"),
        1
    );
    let (events, _) = cold(
        "$pdf extract the rows of invoices/",
        json!({"skillcache": {"mode": "warm_only"}}),
    )
    .await;
    assert_eq!(events[0]["phase"], "miss");
    assert_eq!(events[0]["reason"], "stale", "{events:?}");
    assert_eq!(stand.capsule(&capsule_id).await["status"], "stale");
    // Once stale it is no capsule at all for the next turn.
    let (events, missed) = cold(
        "$pdf extract the rows of invoices/",
        json!({"skillcache": {"mode": "warm_only"}}),
    )
    .await;
    assert_eq!(events[0]["reason"], "no_capsule");
    // A miss sends exactly the prompt a session without SkillCache sends.
    assert_eq!(missed, untouched);
    stand.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn warm_network_capability_mismatch_is_miss() {
    if !sandbox_usable() {
        eprintln!("skipped: no usable bubblewrap sandbox on this host");
        return;
    }
    let stand = Stand::new().await;
    // A capsule whose programs declare the network: Gate 3 replays it with
    // the network, and Codex outside Bypass has none for it.
    let mut answer = codex_answer();
    answer["capsule"]["capabilities"]["network"] = "required".into();
    let (capsule_id, _) = stand.compiled_capsule(&answer).await;
    let capsule = stand.capsule(&capsule_id).await;
    assert_eq!(capsule["capabilities"]["network"], "required");
    assert_eq!(capsule["gates"][2]["report"]["network"], "required");
    stand.script("codex", &codex_skill_turn());
    let sid = stand
        .codex_turn(
            "$pdf extract the rows of invoices/",
            json!({"skillcache": {"mode": "warm_only"}}),
            None,
        )
        .await;
    let events = stand.skillcache_events(&sid);
    assert_eq!(events[0]["phase"], "miss");
    assert_eq!(events[0]["reason"], "network", "{events:?}");
    // With Bypass, Codex runs without its sandbox and the capsule is used.
    let sid = stand
        .codex_turn(
            "$pdf extract the rows of invoices/",
            json!({"skillcache": {"mode": "warm_only"}, "permissions": "bypass"}),
            Some(&warm_agent(&stand.root, "call 2 skillcache-run status")),
        )
        .await;
    assert_eq!(stand.phases(&sid)[0], "hit");
    // Without a sandbox there is nothing to add a writable folder to.
    assert!(
        !stand
            .recorded("codex-arguments.txt")
            .lines()
            .any(|line| line == "--add-dir")
    );
    stand.close().await;
}
