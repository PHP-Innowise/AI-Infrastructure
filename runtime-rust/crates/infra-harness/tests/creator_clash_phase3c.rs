//! Phase 3C end-to-end tests: Creator PermissionMode + PermissionDesk (through bwrap),
//! and Clash cycle-wide budgets (seconds, tokens, usd) + SQLite cost_totals.
#![cfg(unix)]

use infra_generator::{NativeGateServices, NativeOutput};
use infra_harness::{
    accelerators::Accelerators,
    bindings::{ResultsBridge, SessionLinks},
    clash::Cycle,
    commands::CommandCatalog,
    creator::CreatorManager,
    creator_files as files,
    creator_runner::CreatorRunner,
    http::{self, App},
    knowledge::KnowledgeDomain,
    provider::{Launch, LaunchAdapter, PreparedCommand, Provider, ProviderDomain},
    results::Results,
    services::{CloseFuture, Domain, NativeServices, Payload, Reply},
    sessions::{BlackboardFactory, SessionLimits, Sessions, SpecialWorkflow},
    store::{self, SessionStore},
};
use infra_host::permission::PermissionMode;
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

struct NoopGates;
impl NativeGateServices for NoopGates {
    fn validate_host(
        &self,
        _generation_root: &Path,
        _delivery_root: &Path,
        _edition: &str,
        _owned: &BTreeMap<String, String>,
    ) -> infra_core::Result<Vec<String>> {
        Ok(vec![])
    }
    fn runtime_command(
        &self,
        _target: &Path,
        _arguments: &[String],
        _timeout: Duration,
    ) -> infra_core::Result<NativeOutput> {
        Ok(NativeOutput {
            code: 0,
            stdout: vec![],
            stderr: vec![],
        })
    }
}

struct PassThroughAdapter;
impl LaunchAdapter for PassThroughAdapter {
    fn adapt(
        &self,
        _input: &Launch,
        command: PreparedCommand,
    ) -> infra_harness::Result<PreparedCommand> {
        Ok(command)
    }
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Fake Claude for Creator & Clash:
/// - In `ClaudeControl` (`--input-format stream-json`), asks `Bash` once and writes
///   `tasks/TASK-001/harness-questions.md` on `allow` so Creator exits cleanly as `needs_input`.
/// - In `Text` mode, supports modes controlled by `FAKE_CLAUDE_BEHAVIOR` or prompt keywords:
///   - `output-flood`: prints >4MiB of JSON lines to trigger output-limit
///   - `fail-exit`: exits 1 with no result
///   - `token-heavy`: prints multiple usage events of 60 tokens each
///   - `sleep-1`: sleeps 1.2s before completing
///   - `usd-limit`: prints `error_max_budget_usd`
///   - `cost:<cumulative>:<session_id>`: emits init with `claude_code_version: 2.1.300` and cumulative `total_cost_usd`
fn fake_claude(bin_dir: &Path, log_dir: &Path) -> (PathBuf, PathBuf) {
    let program = bin_dir.join("fake-claude");
    let args_log = log_dir.join("claude-args.log");
    let script = format!(
        r#"#!/bin/sh
if [ -d "{log_dir}" ]; then
  printf '%s\n' "$@" >> "{args_log}"
fi
printf '%s\n' "$@" >> "$(pwd)/.fake-claude-args.log"
case " $* " in
  *" --input-format stream-json "*)
    while IFS= read -r line; do
      case "$line" in
        *'"subtype":"initialize"'*)
          echo '{{"type":"control_response","response":{{"subtype":"success","request_id":"harness-initialize","response":{{"current_permission_mode":"auto"}}}}}}';;
        *'"subtype":"set_permission_mode"'*)
          echo '{{"type":"control_response","response":{{"subtype":"success","request_id":"harness-auto-mode","response":{{"mode":"auto"}}}}}}';;
        *'"type":"user"'*)
          echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"creator-claude-1"}}'
          echo '{{"type":"control_request","request_id":"req-c-1","request":{{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash","input":{{"command":"php -v"}},"description":"Inspect PHP version"}}}}';;
        *'"behavior":"allow"'*)
          mkdir -p tasks/TASK-001
          printf 'Need confirmation?\n' > tasks/TASK-001/harness-questions.md
          echo '{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"Clarification needed."}}]}},"session_id":"creator-claude-1"}}'
          echo '{{"type":"result","subtype":"success","is_error":false,"result":"Clarification needed.","session_id":"creator-claude-1","total_cost_usd":0.02,"usage":{{"input_tokens":10,"output_tokens":5}}}}';;
        *'"behavior":"deny"'*)
          echo '{{"type":"result","subtype":"success","is_error":false,"result":"Declined.","session_id":"creator-claude-1","total_cost_usd":0.01,"usage":{{"input_tokens":5,"output_tokens":2}}}}';;
      esac
    done
    exit 0;;
esac
prompt=$(cat)
case "$prompt" in
  *MODE_FAIL*)
    exit 1;;
  *MODE_OUTPUT_LIMIT*)
    echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"flood-1"}}'
    i=0
    pad="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    pad="$pad$pad$pad$pad$pad$pad$pad$pad$pad$pad"
    pad="$pad$pad$pad$pad$pad$pad$pad$pad$pad$pad"
    pad="$pad$pad$pad$pad$pad$pad$pad$pad$pad$pad"
    while [ "$i" -lt 120 ]; do
      echo "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"$pad\"}}]}},\"session_id\":\"flood-1\"}}"
      i=$((i + 1))
    done
    exit 0;;
  *MODE_TOKENS_CREATOR*)
    echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"tok-1"}}'
    echo '{{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"tok-1","total_cost_usd":0.01,"usage":{{"input_tokens":100,"output_tokens":50}}}}'
    sleep 2
    exit 0;;
  *MODE_SLEEP_LONG*)
    echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"clash-p-1"}}'
    sleep 1.2
    echo '{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"Opening complete."}}]}},"session_id":"clash-p-1"}}'
    echo '{{"type":"result","subtype":"success","is_error":false,"result":"Opening complete.","session_id":"clash-p-1","total_cost_usd":0.02,"usage":{{"input_tokens":10,"output_tokens":5}}}}'
    exit 0;;
  *MODE_USD_LIMIT_HIT*)
    echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"clash-p-usd"}}'
    echo '{{"type":"result","subtype":"error_max_budget_usd","is_error":true,"result":"Budget exceeded","session_id":"clash-p-usd","total_cost_usd":0.06,"usage":{{"input_tokens":10,"output_tokens":5}}}}'
    exit 0;;
  *MODE_COST_PROTAGONIST*)
    case " $* " in
      *" --resume "*)
        echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"clash-p-cost"}}'
        echo '{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"{{\"responses\":[{{\"id\":1,\"action\":\"fixed\",\"note\":\"ok\"}}]}}"}}]}},"session_id":"clash-p-cost"}}'
        echo '{{"type":"result","subtype":"success","is_error":false,"result":"{{\"responses\":[{{\"id\":1,\"action\":\"fixed\",\"note\":\"ok\"}}]}}","session_id":"clash-p-cost","total_cost_usd":0.09,"usage":{{"input_tokens":10,"output_tokens":5}}}}';;
      *)
        echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"clash-p-cost"}}'
        echo '{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"Initial implementation done."}}]}},"session_id":"clash-p-cost"}}'
        echo '{{"type":"result","subtype":"success","is_error":false,"result":"Initial implementation done.","session_id":"clash-p-cost","total_cost_usd":0.04,"usage":{{"input_tokens":10,"output_tokens":5}}}}';;
    esac
    exit 0;;
  *MODE_USD_CAP_CHECK*)
    case " $* " in
      *" --resume "*)
        echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"clash-p-cap"}}'
        echo '{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"{{\"responses\":[{{\"id\":1,\"action\":\"fixed\",\"note\":\"ok\"}}]}}"}}]}},"session_id":"clash-p-cap"}}'
        echo '{{"type":"result","subtype":"success","is_error":false,"result":"{{\"responses\":[{{\"id\":1,\"action\":\"fixed\",\"note\":\"ok\"}}]}}","session_id":"clash-p-cap","total_cost_usd":0.08,"usage":{{"input_tokens":10,"output_tokens":5}}}}';;
      *)
        echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"clash-p-cap"}}'
        echo '{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"First turn cost 0.06."}}]}},"session_id":"clash-p-cap"}}'
        echo '{{"type":"result","subtype":"success","is_error":false,"result":"First turn cost 0.06.","session_id":"clash-p-cap","total_cost_usd":0.06,"usage":{{"input_tokens":10,"output_tokens":5}}}}';;
    esac
    exit 0;;
  *MODE_USD_NEAR_EXHAUST*)
    echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"clash-p-near"}}'
    echo '{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"Near cap turn."}}]}},"session_id":"clash-p-near"}}'
    echo '{{"type":"result","subtype":"success","is_error":false,"result":"Near cap turn.","session_id":"clash-p-near","total_cost_usd":0.045,"usage":{{"input_tokens":10,"output_tokens":5}}}}'
    exit 0;;
  *MODE_TOKEN_CYCLE*)
    echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"clash-p-tok"}}'
    echo '{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"First turn 60 tokens."}}]}},"session_id":"clash-p-tok"}}'
    echo '{{"type":"result","subtype":"success","is_error":false,"result":"First turn 60 tokens.","session_id":"clash-p-tok","total_cost_usd":0.01,"usage":{{"input_tokens":40,"output_tokens":20}}}}'
    exit 0;;
esac
mkdir -p tasks/TASK-001
printf 'Need confirmation?\n' > tasks/TASK-001/harness-questions.md
echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"creator-claude-text"}}'
echo '{{"type":"result","subtype":"success","is_error":false,"result":"Done","session_id":"creator-claude-text","total_cost_usd":0.01,"usage":{{"input_tokens":10,"output_tokens":5}}}}'
"#,
        args_log = args_log.display(),
        log_dir = log_dir.display(),
    );
    write_executable(&program, &script);
    (program, args_log)
}

/// Fake Codex for Creator (`app-server` + `exec`) and Clash challenger.
fn fake_codex(bin_dir: &Path, log_dir: &Path) -> (PathBuf, PathBuf) {
    let program = bin_dir.join("fake-codex");
    let args_log = log_dir.join("codex-args.log");
    let script = format!(
        r#"#!/bin/sh
if [ -d "{log_dir}" ]; then
  printf '%s\n' "$@" >> "{args_log}"
fi
printf '%s\n' "$@" >> "$(pwd)/.fake-codex-args.log"
case " $* " in
  *" app-server "*)
    while IFS= read -r line; do
      id=$(printf '%s' "$line" | sed -n 's/^{{"id":\([0-9]*\),.*/\1/p')
      case "$line" in
        *'"method":"initialize"'*)
          echo "{{\"id\":$id,\"result\":{{\"userAgent\":\"fake\"}}}}";;
        *'"method":"thread/start"'*|*'"method":"thread/resume"'*)
          echo "{{\"id\":$id,\"result\":{{\"thread\":{{\"id\":\"codex-thread-1\"}}}}}}";;
        *'"method":"turn/start"'*)
          echo "{{\"id\":$id,\"result\":{{\"turn\":{{\"id\":\"turn-1\"}}}}}}"
          echo '{{"method":"turn/started","params":{{"threadId":"codex-thread-1","turn":{{"id":"turn-1"}}}}}}'
          echo '{{"id":9,"method":"item/commandExecution/requestApproval","params":{{"threadId":"codex-thread-1","turnId":"turn-1","itemId":"item-1","command":"php -v","reason":"Check PHP"}}}}';;
        *'"id":9,'*'"decision":"accept"'*)
          mkdir -p tasks/TASK-001
          printf 'Codex question?\n' > tasks/TASK-001/harness-questions.md
          echo '{{"method":"item/completed","params":{{"threadId":"codex-thread-1","turnId":"turn-1","item":{{"type":"agentMessage","id":"item-2","text":"Codex allowed."}}}}}}'
          echo '{{"method":"thread/tokenUsage/updated","params":{{"threadId":"codex-thread-1","turnId":"turn-1","tokenUsage":{{"total":{{"inputTokens":12,"cachedInputTokens":0,"outputTokens":4,"reasoningOutputTokens":0,"totalTokens":16}},"last":{{"inputTokens":12,"cachedInputTokens":0,"outputTokens":4,"reasoningOutputTokens":0,"totalTokens":16}}}}}}}}'
          echo '{{"method":"turn/completed","params":{{"threadId":"codex-thread-1","turn":{{"id":"turn-1","status":"completed"}}}}}}';;
        *'"id":9,'*'"decision":"decline"'*)
          echo '{{"method":"item/completed","params":{{"threadId":"codex-thread-1","turnId":"turn-1","item":{{"type":"agentMessage","id":"item-2","text":"Codex declined."}}}}}}'
          echo '{{"method":"turn/completed","params":{{"threadId":"codex-thread-1","turn":{{"id":"turn-1","status":"completed"}}}}}}';;
      esac
    done
    sleep 10
    exit 0;;
esac
prompt=$(cat)
case "$prompt" in
  *MODE_SLEEP_LONG*)
    echo '{{"type":"thread.started","thread_id":"codex-sleep-1"}}'
    echo '{{"type":"turn.started"}}'
    sleep 2.5
    echo '{{"type":"item.completed","item":{{"id":"i1","type":"agent_message","text":"{{\"verdict\":\"accept\",\"objections\":[],\"resolved\":[]}}"}}}}'
    echo '{{"type":"turn.completed","usage":{{"input_tokens":5,"cached_input_tokens":0,"output_tokens":5}}}}'
    exit 0;;
  *MODE_TOKEN_CYCLE*)
    echo '{{"type":"thread.started","thread_id":"codex-tok-1"}}'
    echo '{{"type":"turn.started"}}'
    echo '{{"type":"turn.completed","usage":{{"input_tokens":30,"cached_input_tokens":0,"output_tokens":20}}}}'
    sleep 3
    exit 0;;
esac
mkdir -p tasks/TASK-001
printf 'Codex text question?\n' > tasks/TASK-001/harness-questions.md
echo '{{"type":"thread.started","thread_id":"codex-chal-1"}}'
echo '{{"type":"turn.started"}}'
echo '{{"type":"item.completed","item":{{"id":"i1","type":"agent_message","text":"{{\"verdict\":\"reject\",\"objections\":[{{\"id\":1,\"severity\":\"medium\",\"file\":\"src/a.php\",\"line\":1,\"claim\":\"Check edge\",\"evidence\":\"line 1\"}}],\"resolved\":[]}}"}}}}'
echo '{{"type":"turn.completed","usage":{{"input_tokens":10,"cached_input_tokens":0,"output_tokens":5}}}}'
"#,
        args_log = args_log.display(),
        log_dir = log_dir.display(),
    );
    write_executable(&program, &script);
    (program, args_log)
}

/// Fake Cursor for Creator (`acp` + print).
fn fake_cursor(bin_dir: &Path, log_dir: &Path) -> (PathBuf, PathBuf) {
    let program = bin_dir.join("fake-cursor");
    let args_log = log_dir.join("cursor-args.log");
    let script = format!(
        r#"#!/bin/sh
if [ -d "{log_dir}" ]; then
  printf '%s\n' "$@" >> "{args_log}"
fi
printf '%s\n' "$@" >> "$(pwd)/.fake-cursor-args.log"
case " $* " in
  *" acp "*)
    prompt=0
    while IFS= read -r line; do
      id=$(printf '%s' "$line" | sed -n 's/^{{"jsonrpc":"2.0","id":\([0-9]*\),.*/\1/p')
      case "$line" in
        *'"method":"initialize"'*)
          echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"protocolVersion\":1}}}}";;
        *'"method":"session/new"'*|*'"method":"session/load"'*)
          echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"sessionId\":\"cursor-chat-1\"}}}}";;
        *'"method":"session/prompt"'*)
          prompt=$id
          printf '%s\n' '{{"jsonrpc":"2.0","id":0,"method":"session/request_permission","params":{{"sessionId":"cursor-chat-1","toolCall":{{"toolCallId":"call-1","title":"php -v","kind":"execute","rawInput":{{"command":"php -v"}}}},"options":[{{"optionId":"allow-once","name":"Allow once","kind":"allow_once"}},{{"optionId":"reject-once","name":"Reject","kind":"reject_once"}}]}}}}';;
        *'"optionId":"allow-once"'*)
          mkdir -p tasks/TASK-001
          printf 'Cursor question?\n' > tasks/TASK-001/harness-questions.md
          echo '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"cursor-chat-1","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"Cursor allowed."}}}}}}}}'
          echo "{{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{{\"stopReason\":\"end_turn\"}}}}";;
        *'"optionId":"reject-once"'*)
          echo '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"cursor-chat-1","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"Cursor declined."}}}}}}}}'
          echo "{{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{{\"stopReason\":\"end_turn\"}}}}";;
      esac
    done
    exit 0;;
esac
mkdir -p tasks/TASK-001
printf 'Cursor text question?\n' > tasks/TASK-001/harness-questions.md
echo '{{"type":"system","subtype":"init","session_id":"cursor-text-1"}}'
echo '{{"type":"result","subtype":"success","is_error":false,"result":"Done","session_id":"cursor-text-1"}}'
"#,
        args_log = args_log.display(),
        log_dir = log_dir.display(),
    );
    write_executable(&program, &script);
    (program, args_log)
}

struct Phase3cFixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    project_id: String,
    store: SessionStore,
    results: Arc<infra_harness::results::Results>,
    sessions: Arc<Sessions>,
    _creator: Arc<CreatorManager>,
    claude: PathBuf,
    codex: PathBuf,
    cursor: PathBuf,
    claude_args: PathBuf,
    _codex_args: PathBuf,
    _cursor_args: PathBuf,
    port: u16,
    token: String,
    shutdown: tokio::sync::watch::Sender<bool>,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
}

impl Phase3cFixture {
    async fn new() -> Self {
        let _ = infra_host::process::use_guardian(PathBuf::from(env!(
            "CARGO_BIN_EXE_infra-harness-inbox-fixture"
        )));
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let project = root.join("project");
        let source = root.join("source");
        let bin_dir = source.join("bin");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(project.join("composer.json"), "{}").unwrap();
        let (claude, claude_args) = fake_claude(&bin_dir, &root);
        let (codex, codex_args) = fake_codex(&bin_dir, &root);
        let (cursor, cursor_args) = fake_cursor(&bin_dir, &root);
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
                    detail: "Test".into(),
                },
            )
        };
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::from([
                fake("claude", claude.clone()),
                fake("codex", codex.clone()),
                fake("cursor", cursor.clone()),
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
        let links = Arc::new(SessionLinks::new());
        let special: BTreeMap<String, Arc<dyn SpecialWorkflow>> = BTreeMap::from([
            (
                "creator".into(),
                Arc::new(CreatorRunner {
                    store: store.clone(),
                    source: source.clone(),
                    gates: Arc::new(NoopGates),
                    links: Some(links.clone()),
                }) as Arc<dyn SpecialWorkflow>,
            ),
            (
                "system_run".into(),
                Arc::new(UnusedSpecial) as Arc<dyn SpecialWorkflow>,
            ),
            (
                "system_discovery".into(),
                Arc::new(UnusedSpecial) as Arc<dyn SpecialWorkflow>,
            ),
        ]);
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
        let creator = Arc::new(
            CreatorManager::new(store.clone(), source.clone(), Arc::downgrade(&sessions)).unwrap(),
        );
        let stub: Arc<dyn Domain> = Arc::new(StubDomain);
        let mut domains: BTreeMap<String, Arc<dyn Domain>> = [
            "setup",
            "accelerators",
            "systems",
            "discovery",
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
        domains.insert("creator".into(), creator.clone());
        domains.insert("knowledge".into(), knowledge);
        for name in ["sessions", "brain", "context"] {
            domains.insert(name.into(), sessions.clone());
        }
        domains.insert(
            "results".into(),
            Arc::new(infra_harness::results::ResultService {
                results: results.clone(),
            }),
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
        let token = "phase3c-test-token".to_owned();
        let app = App {
            services,
            token: token.clone(),
            instance: "phase3c-test".into(),
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
            results,
            sessions,
            _creator: creator,
            claude,
            codex,
            cursor,
            claude_args,
            _codex_args: codex_args,
            _cursor_args: cursor_args,
            port,
            token,
            shutdown,
            server,
            client: reqwest::Client::new(),
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

    fn seed_creator_run(
        &self,
        provider: &str,
        permissions: Value,
        goal: &str,
        budgets: Value,
    ) -> String {
        let id = store::hexid();
        let project_path = self.root.join("project");
        let run_dir = self.store.state.join("creator").join(&id);
        fs::create_dir_all(run_dir.join("agent").join(files::TASK)).unwrap();
        let identity = files::identity(&project_path).unwrap();
        let run = json!({
            "id": id,
            "project_id": self.project_id,
            "target": project_path,
            "identity": identity,
            "workspace": "project",
            "branch": Value::Null,
            "operation": "generate",
            "tools": ["claude"],
            "provider": provider,
            "model": Value::Null,
            "thinking_effort": Value::Null,
            "agents_enabled": false,
            "agent_count": 3,
            "permissions": permissions,
            "budgets": budgets,
            "goal": goal,
            "created_at": store::now(),
            "revision": 1,
            "status": "review",
            "answers": "",
            "phase": "scan"
        });
        self.store
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO creator_runs(id,data) VALUES(?1,?2)",
                rusqlite::params![id, run.to_string()],
            )
            .unwrap();
        id
    }

    async fn wait_creator(&self, id: &str, what: &str, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let (status, view) = self
                .http(Method::GET, &format!("/api/creator/{id}"), None)
                .await;
            assert_eq!(status, 200, "{view}");
            if done(&view) {
                return view;
            }
            assert!(
                Instant::now() < deadline,
                "Creator {what} timed out: {view:#?}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn close(self) {
        let _ = self.shutdown.send(true);
        let _ = self.server.await;
        let _ = self.sessions.close();
        let _ = self.sessions.wait_closed().await;
    }
}

fn run_on_worker<F>(fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_stack_size(32 << 20)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async move {
        tokio::spawn(fut).await.unwrap();
    });
}

#[test]
fn creator_manual_permissions_stream_through_sandbox_and_permission_desk() {
    run_on_worker(async {
        let fx = Phase3cFixture::new().await;

        // Invalid permission mode rejected via POST /api/creator.
        let (status, err) = fx
            .http(
                Method::POST,
                "/api/creator",
                Some(json!({
                    "project_id": fx.project_id,
                    "provider": "claude",
                    "tools": ["claude"],
                    "permissions": "invalid-mode"
                })),
            )
            .await;
        assert_eq!(status, 400, "{err}");

        // Exercise Manual PermissionDesk across all three providers through bwrap sandbox.
        for provider in ["claude", "codex", "cursor"] {
            let run_id = fx.seed_creator_run(
                provider,
                json!("auto"),
                "Inspect project",
                json!({"usd": Value::Null, "tokens": Value::Null, "seconds": 30}),
            );
            let (status, launched) = fx
                .http(
                    Method::POST,
                    &format!("/api/creator/{run_id}"),
                    Some(json!({
                        "action": "rescan",
                        "revision": 1,
                        "answers": "",
                        "routing": {"permissions": "manual"}
                    })),
                )
                .await;
            assert_eq!(status, 200, "{launched}");
            assert_eq!(launched["run"]["permissions"], "manual");
            let sid = launched["session"]["id"].as_str().unwrap().to_owned();

            // Wait for pending permission to show up in GET /api/creator/{id} waiting_permissions.
            let waiting_view = fx
                .wait_creator(&run_id, &format!("{provider} waiting permission"), |v| {
                    !v["waiting_permissions"].as_array().unwrap().is_empty()
                })
                .await;
            let ask = &waiting_view["waiting_permissions"][0];
            assert_eq!(ask["state"], "pending");
            let req_id = ask["request_id"].as_str().unwrap().to_owned();

            // Approve via HTTP permission endpoint.
            let (ans_status, ans_body) = fx
                .http(
                    Method::POST,
                    &format!("/api/sessions/{sid}/permissions/{req_id}"),
                    Some(json!({"decision": "allow"})),
                )
                .await;
            assert_eq!(ans_status, 200, "{ans_body}");

            // Creator completes into needs_input with permission + result events recorded.
            let finished = fx
                .wait_creator(&run_id, &format!("{provider} finished"), |v| {
                    v["run"]["status"] == "needs_input"
                })
                .await;
            assert!(
                finished["waiting_permissions"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            let events = finished["events"].as_array().unwrap();
            assert!(
                events
                    .iter()
                    .any(|e| e["kind"] == "permission" && e["state"] == "pending")
            );
            assert!(
                events
                    .iter()
                    .any(|e| e["kind"] == "permission" && e["state"] == "allowed")
            );
            assert!(
                events
                    .iter()
                    .any(|e| e["kind"] == "result" && e["ok"] == true)
            );
        }

        // Verify Manual Deny in Creator routes through PermissionDesk and emits denied state.
        let deny_id = fx.seed_creator_run(
            "claude",
            json!("manual"),
            "Deny check",
            json!({"usd": Value::Null, "tokens": Value::Null, "seconds": 30}),
        );
        let (status, launched) = fx
            .http(
                Method::POST,
                &format!("/api/creator/{deny_id}"),
                Some(json!({"action": "rescan", "revision": 1, "answers": ""})),
            )
            .await;
        assert_eq!(status, 200, "{launched}");
        let deny_sid = launched["session"]["id"].as_str().unwrap().to_owned();
        let waiting_deny = fx
            .wait_creator(&deny_id, "claude deny waiting", |v| {
                !v["waiting_permissions"].as_array().unwrap().is_empty()
            })
            .await;
        let deny_req = waiting_deny["waiting_permissions"][0]["request_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let (ds, db) = fx
            .http(
                Method::POST,
                &format!("/api/sessions/{deny_sid}/permissions/{deny_req}"),
                Some(json!({"decision": "deny"})),
            )
            .await;
        assert_eq!(ds, 200, "{db}");
        let deny_done = fx
            .wait_creator(&deny_id, "claude deny finished", |v| {
                !["scanning", "generating"].contains(&v["run"]["status"].as_str().unwrap_or(""))
            })
            .await;
        assert!(
            deny_done["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["kind"] == "permission" && e["state"] == "denied"),
            "{:#?}",
            deny_done["events"]
        );

        // Verify Codex Auto runs Text transport with --skip-git-repo-check inside sandbox,
        // and verify target project and state DB remain read-only inside Creator bwrap.
        let codex_auto_id = fx.seed_creator_run(
            "codex",
            json!("auto"),
            "Auto scan",
            json!({"usd": Value::Null, "tokens": Value::Null, "seconds": 30}),
        );
        let spec = infra_harness::creator_runner::sandbox_spec(&json!({
            "directory": fx.store.state.join("creator").join(&codex_auto_id),
            "target": fx.root.join("project"),
            "provider": "codex",
            "protected_roots": [fx.root.join("project"), fx.store.state],
        }))
        .unwrap();
        if infra_harness::provider::find_executable("bwrap").is_some() {
            let probe_argv = infra_harness::sandbox::creator(
                &spec,
                &[
                    "/bin/sh".into(),
                    "-c".into(),
                    format!(
                        "! (printf x > '{}') 2>/dev/null && ! touch '{}' 2>/dev/null",
                        fx.store.state.join("sessions.sqlite3").display(),
                        fx.root.join("project/nope.txt").display()
                    ),
                ],
            )
            .unwrap();
            let status = std::process::Command::new(&probe_argv[0])
                .args(&probe_argv[1..])
                .status()
                .unwrap();
            assert!(
                status.success(),
                "sandbox read-only protection check failed"
            );
        }
        let (status, _) = fx
            .http(
                Method::POST,
                &format!("/api/creator/{codex_auto_id}"),
                Some(json!({
                    "action": "rescan",
                    "revision": 1,
                    "answers": ""
                })),
            )
            .await;
        assert_eq!(status, 200);
        fx.wait_creator(&codex_auto_id, "codex auto needs_input", |v| {
            v["run"]["status"] == "needs_input"
        })
        .await;
        let codex_args = fs::read_to_string(
            fx.store
                .state
                .join("creator")
                .join(&codex_auto_id)
                .join("agent/.fake-codex-args.log"),
        )
        .unwrap();
        assert!(codex_args.contains("--skip-git-repo-check"), "{codex_args}");
        assert!(codex_args.contains("--approve-for-me"), "{codex_args}");

        // Verify token budget enforcement preserves "The native token budget was reached."
        let tok_id = fx.seed_creator_run(
            "claude",
            json!("bypass"),
            "MODE_TOKENS_CREATOR",
            json!({"usd": Value::Null, "tokens": 100, "seconds": 30}),
        );
        let (status, _) = fx
            .http(
                Method::POST,
                &format!("/api/creator/{tok_id}"),
                Some(json!({"action": "rescan", "revision": 1, "answers": ""})),
            )
            .await;
        assert_eq!(status, 200);
        let tok_done = fx
            .wait_creator(&tok_id, "creator token limit", |v| {
                v["run"]["status"] == "failed"
            })
            .await;
        assert_eq!(
            tok_done["run"]["message"],
            "The native token budget was reached."
        );
        assert_eq!(tok_done["session"]["status"], "failed");

        // Verify output-limit emits Creator's retained-artifacts status before verify().
        let flood_id = fx.seed_creator_run(
            "claude",
            json!("bypass"),
            "MODE_OUTPUT_LIMIT",
            json!({"usd": Value::Null, "tokens": Value::Null, "seconds": 30}),
        );
        let (status, _) = fx
            .http(
                Method::POST,
                &format!("/api/creator/{flood_id}"),
                Some(json!({"action": "rescan", "revision": 1, "answers": ""})),
            )
            .await;
        assert_eq!(status, 200);
        let flood_done = fx
            .wait_creator(&flood_id, "creator output limit", |v| {
                !["scanning", "generating"].contains(&v["run"]["status"].as_str().unwrap_or(""))
            })
            .await;
        assert!(
            flood_done["events"].as_array().unwrap().iter().any(|e| {
                e["kind"] == "status"
                    && e["text"]
                        .as_str()
                        .is_some_and(|t| t.contains("Creator stream output limit reached"))
            }),
            "{:#?}",
            flood_done["events"]
        );

        // Verify non-zero native failure preserves "Native agent did not complete successfully."
        let fail_id = fx.seed_creator_run(
            "claude",
            json!("bypass"),
            "MODE_FAIL",
            json!({"usd": Value::Null, "tokens": Value::Null, "seconds": 30}),
        );
        let (status, _) = fx
            .http(
                Method::POST,
                &format!("/api/creator/{fail_id}"),
                Some(json!({"action": "rescan", "revision": 1, "answers": ""})),
            )
            .await;
        assert_eq!(status, 200);
        let fail_done = fx
            .wait_creator(&fail_id, "creator agent failure", |v| {
                v["run"]["status"] == "failed"
            })
            .await;
        assert_eq!(
            fail_done["run"]["message"],
            "Native agent did not complete successfully."
        );

        fx.close().await;
    });
}

fn create_clash_session(
    fx: &Phase3cFixture,
    provider: &str,
    challenger: &str,
    budgets: Value,
    rounds: u64,
) -> Value {
    let project_path = fx.root.join("project");
    fx.store
        .create(
            &json!({
                "project_id": fx.project_id,
                "_workspace_path": project_path,
                "provider": provider,
                "mode": "edit",
                "workflow": "native",
                "project_context": false,
                "budgets": budgets,
                "clash": {
                    "challenger": challenger,
                    "rounds": rounds,
                    "challenger_model": Value::Null,
                    "challenger_thinking_effort": Value::Null,
                },
                "permissions": "bypass",
            }),
            "Clash test task",
            &[],
        )
        .unwrap()
}

fn base_launch(fx: &Phase3cFixture, provider: &str) -> Launch {
    Launch {
        provider: provider.into(),
        executable: if provider == "claude" {
            fx.claude.clone()
        } else if provider == "codex" {
            fx.codex.clone()
        } else {
            fx.cursor.clone()
        },
        project: fx.root.join("project"),
        prompt: "Clash task".into(),
        mode: "edit".into(),
        model: None,
        session: None,
        agents: false,
        count: 1,
        effort: None,
        usd: None,
        hook_events: false,
        permissions: Some(PermissionMode::Bypass),
        approvals: None,
    }
}

#[test]
fn clash_enforces_cycle_wide_seconds_tokens_budgets_and_persists_cost_totals() {
    run_on_worker(async {
        let fx = Phase3cFixture::new().await;
        let executables = BTreeMap::from([
            ("claude".into(), fx.claude.clone()),
            ("codex".into(), fx.codex.clone()),
            ("cursor".into(), fx.cursor.clone()),
        ]);
        let emit: Arc<dyn Fn(Value) -> infra_harness::Result<()> + Send + Sync> =
            Arc::new(|_| Ok(()));

        // 1. Cycle-wide `seconds` budget across turns (shared deadline, not reset per turn).
        let time_session = create_clash_session(
            &fx,
            "claude",
            "codex",
            json!({"usd": Value::Null, "tokens": Value::Null, "seconds": 2}),
            2,
        );
        let mut cycle = Cycle {
            store: fx.store.clone(),
            session: time_session.clone(),
            launch: base_launch(&fx, "claude"),
            executables: executables.clone(),
            context: String::new(),
            adapter: Arc::new(PassThroughAdapter),
        };
        let started = Instant::now();
        let state = cycle
            .drive(
                "MODE_SLEEP_LONG",
                "implement",
                false,
                Arc::new(AtomicBool::new(false)),
                emit.clone(),
            )
            .await
            .unwrap();
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(3),
            "cycle took too long: {elapsed:?}"
        );
        assert_eq!(state["status"], "finished");
        assert_eq!(state["outcome"], "incomplete");
        assert_eq!(
            state["message"],
            "Clash reached its time budget; cycle incomplete."
        );
        assert_eq!(state["turns"].as_array().unwrap().len(), 2);
        assert_eq!(state["turns"][0]["ok"], true);
        assert_eq!(state["turns"][1]["ok"], false);

        // 2. Cycle-wide `tokens` budget across turns + token recording on state and turns.
        let token_session = create_clash_session(
            &fx,
            "claude",
            "codex",
            json!({"usd": Value::Null, "tokens": 100, "seconds": 30}),
            2,
        );
        let mut cycle = Cycle {
            store: fx.store.clone(),
            session: token_session,
            launch: base_launch(&fx, "claude"),
            executables: executables.clone(),
            context: String::new(),
            adapter: Arc::new(PassThroughAdapter),
        };
        let state = cycle
            .drive(
                "MODE_TOKEN_CYCLE",
                "implement",
                false,
                Arc::new(AtomicBool::new(false)),
                emit.clone(),
            )
            .await
            .unwrap();
        assert_eq!(state["outcome"], "incomplete");
        assert_eq!(
            state["message"],
            "Clash reached its token budget; cycle incomplete."
        );
        assert_eq!(state["tokens"], 110);
        assert_eq!(state["turns"][0]["tokens"], 60);
        assert_eq!(state["turns"][1]["tokens"], 50);

        // 3. SQLite `cost_totals` persisted across turns AND follow-up cycles for BOTH participants.
        // We exercise `Cycle` with fake Claude on both sides (so both report `cost_session_total`)
        // to prove neither protagonist (`native_session_id`) nor challenger (`challenger_session_id`)
        // double-counts across turns or follow-up cycles.
        let p_script = fx.root.join("clash-p-claude");
        let c_script = fx.root.join("clash-c-claude");
        write_executable(
            &p_script,
            r#"#!/bin/sh
prompt=$(cat)
case " $* " in
  *" --resume "*)
    case "$prompt" in
      *cycle\ 2*)
        echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"p-sess-1"}'
        echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Cycle 2 opening."}]},"session_id":"p-sess-1"}'
        echo '{"type":"result","subtype":"success","is_error":false,"result":"Cycle 2 opening.","session_id":"p-sess-1","total_cost_usd":0.12,"usage":{"input_tokens":10,"output_tokens":5}}'
        ;;
      *)
        echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"p-sess-1"}'
        echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"{\"responses\":[{\"id\":1,\"action\":\"fixed\",\"note\":\"ok\"}]}"}]},"session_id":"p-sess-1"}'
        echo '{"type":"result","subtype":"success","is_error":false,"result":"{\"responses\":[{\"id\":1,\"action\":\"fixed\",\"note\":\"ok\"}]}","session_id":"p-sess-1","total_cost_usd":0.09,"usage":{"input_tokens":10,"output_tokens":5}}'
        ;;
    esac
    ;;
  *)
    echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"p-sess-1"}'
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Cycle 1 opening."}]},"session_id":"p-sess-1"}'
    echo '{"type":"result","subtype":"success","is_error":false,"result":"Cycle 1 opening.","session_id":"p-sess-1","total_cost_usd":0.04,"usage":{"input_tokens":10,"output_tokens":5}}'
    ;;
esac
"#,
        );
        write_executable(
            &c_script,
            r#"#!/bin/sh
prompt=$(cat)
case " $* " in
  *" --resume "*)
    case "$prompt" in
      *cycle\ 2*)
        echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"c-sess-1"}'
        echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"{\"verdict\":\"accept\",\"objections\":[],\"resolved\":[1]}"}]},"session_id":"c-sess-1"}'
        echo '{"type":"result","subtype":"success","is_error":false,"result":"{\"verdict\":\"accept\",\"objections\":[],\"resolved\":[1]}","session_id":"c-sess-1","total_cost_usd":0.07,"usage":{"input_tokens":10,"output_tokens":5}}'
        ;;
      *)
        echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"c-sess-1"}'
        echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"{\"verdict\":\"accept\",\"objections\":[],\"resolved\":[1]}"}]},"session_id":"c-sess-1"}'
        echo '{"type":"result","subtype":"success","is_error":false,"result":"{\"verdict\":\"accept\",\"objections\":[],\"resolved\":[1]}","session_id":"c-sess-1","total_cost_usd":0.05,"usage":{"input_tokens":10,"output_tokens":5}}'
        ;;
    esac
    ;;
  *)
    echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"c-sess-1"}'
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"{\"verdict\":\"reject\",\"objections\":[{\"id\":1,\"severity\":\"medium\",\"file\":\"src/a.php\",\"line\":1,\"claim\":\"Check\",\"evidence\":\"e\"}],\"resolved\":[]}"}]},"session_id":"c-sess-1"}'
    echo '{"type":"result","subtype":"success","is_error":false,"result":"{\"verdict\":\"reject\",\"objections\":[{\"id\":1,\"severity\":\"medium\",\"file\":\"src/a.php\",\"line\":1,\"claim\":\"Check\",\"evidence\":\"e\"}],\"resolved\":[]}","session_id":"c-sess-1","total_cost_usd":0.03,"usage":{"input_tokens":10,"output_tokens":5}}'
    ;;
esac
"#,
        );
        let cost_session = create_clash_session(
            &fx,
            "claude",
            "codex",
            json!({"usd": 0.50, "tokens": Value::Null, "seconds": 30}),
            2,
        );
        let cost_sid = cost_session["id"].as_str().unwrap().to_owned();
        let mut cycle1 = Cycle {
            store: fx.store.clone(),
            session: cost_session,
            launch: base_launch(&fx, "claude"),
            executables: BTreeMap::from([
                ("claude".into(), p_script.clone()),
                ("codex".into(), c_script.clone()),
            ]),
            context: String::new(),
            adapter: Arc::new(PassThroughAdapter),
        };
        // Wait: challenger uses `codex` in `clash` config, which in `RunCost` treats `provider != "claude"`
        // as reporting own cost. Let's ALSO patch `clash.challenger` in memory to `"claude"` in Cycle
        // (and `provider` = `"claude"`)? Wait: let's test BOTH:
        // First with challenger = `"claude"` in `session["clash"]["challenger"]` so RunCost applies
        // cumulative restoration for BOTH protagonist (`p-sess-1`) AND challenger (`c-sess-1`)!
        cycle1.session["clash"]["challenger"] = json!("claude");
        cycle1.executables = BTreeMap::from([("claude".into(), p_script.clone())]);
        // Wait: if both are "claude", let's dispatch protagonist vs challenger via a small wrapper or
        // test protagonist cumulative + challenger cost_totals separately!
        // Actually, `p_script` can switch behavior based on prompt ("Act as implementer" vs "Act as challenger")!
        let dual_claude = fx.root.join("clash-dual-claude");
        write_executable(
            &dual_claude,
            r#"#!/bin/sh
prompt=$(cat)
case "$prompt" in
  *"Act as challenger"*)
    case " $* " in
      *" --resume "*)
        case "$prompt" in
          *"cycle 2"*)
            echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"c-sess-1"}'
            echo '{"type":"result","subtype":"success","is_error":false,"result":"{\"verdict\":\"accept\",\"objections\":[],\"resolved\":[1]}","session_id":"c-sess-1","total_cost_usd":0.07,"usage":{"input_tokens":10,"output_tokens":5}}'
            ;;
          *)
            echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"c-sess-1"}'
            echo '{"type":"result","subtype":"success","is_error":false,"result":"{\"verdict\":\"accept\",\"objections\":[],\"resolved\":[1]}","session_id":"c-sess-1","total_cost_usd":0.05,"usage":{"input_tokens":10,"output_tokens":5}}'
            ;;
        esac
        ;;
      *)
        echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"c-sess-1"}'
        echo '{"type":"result","subtype":"success","is_error":false,"result":"{\"verdict\":\"reject\",\"objections\":[{\"id\":1,\"severity\":\"medium\",\"file\":\"src/a.php\",\"line\":1,\"claim\":\"Check\",\"evidence\":\"e\"}],\"resolved\":[]}","session_id":"c-sess-1","total_cost_usd":0.03,"usage":{"input_tokens":10,"output_tokens":5}}'
        ;;
    esac
    ;;
  *)
    case " $* " in
      *" --resume "*)
        case "$prompt" in
          *"cycle 2"*)
            echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"p-sess-1"}'
            echo '{"type":"result","subtype":"success","is_error":false,"result":"Cycle 2 opening.","session_id":"p-sess-1","total_cost_usd":0.12,"usage":{"input_tokens":10,"output_tokens":5}}'
            ;;
          *)
            echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"p-sess-1"}'
            echo '{"type":"result","subtype":"success","is_error":false,"result":"{\"responses\":[{\"id\":1,\"action\":\"fixed\",\"note\":\"ok\"}]}","session_id":"p-sess-1","total_cost_usd":0.09,"usage":{"input_tokens":10,"output_tokens":5}}'
            ;;
        esac
        ;;
      *)
        echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"p-sess-1"}'
        echo '{"type":"result","subtype":"success","is_error":false,"result":"Cycle 1 opening.","session_id":"p-sess-1","total_cost_usd":0.04,"usage":{"input_tokens":10,"output_tokens":5}}'
        ;;
    esac
    ;;
esac
"#,
        );
        cycle1.executables = BTreeMap::from([("claude".into(), dual_claude.clone())]);
        let state1 = cycle1
            .drive(
                "Implement feature",
                "implement",
                false,
                Arc::new(AtomicBool::new(false)),
                emit.clone(),
            )
            .await
            .unwrap();
        assert_eq!(state1["outcome"], "converged");
        // Cycle 1 turns: p(0.04), c(0.03), p(0.09-0.04=0.05), c(0.05-0.03=0.02) => total 0.14.
        assert_eq!(state1["turns"][0]["cost_usd"], 0.04);
        assert_eq!(state1["turns"][1]["cost_usd"], 0.03);
        assert_eq!(state1["turns"][2]["cost_usd"], 0.05);
        assert_eq!(state1["turns"][3]["cost_usd"], 0.02);
        assert_eq!(state1["cost_usd"], 0.14);

        let reloaded = fx.store.get(&cost_sid).unwrap();
        assert_eq!(reloaded["cost_totals"]["p-sess-1"], 0.09);
        assert_eq!(reloaded["cost_totals"]["c-sess-1"], 0.05);

        // Follow-up cycle (Cycle 2) reloads from SQLite `cost_totals` and avoids double-counting!
        let mut reloaded_for_c2 = reloaded.clone();
        reloaded_for_c2["clash"]["challenger"] = json!("claude");
        let mut cycle2 = Cycle {
            store: fx.store.clone(),
            session: reloaded_for_c2,
            launch: base_launch(&fx, "claude"),
            executables: BTreeMap::from([("claude".into(), dual_claude)]),
            context: String::new(),
            adapter: Arc::new(PassThroughAdapter),
        };
        let state2 = cycle2
            .drive(
                "Follow-up check",
                "implement",
                true,
                Arc::new(AtomicBool::new(false)),
                emit.clone(),
            )
            .await
            .unwrap();
        assert_eq!(state2["cycle"], 2);
        assert_eq!(state2["outcome"], "converged");
        // Cycle 2 turns: p(0.12-0.09=0.03), c(0.07-0.05=0.02) => cycle 2 cost 0.05!
        let turns2 = state2["turns"].as_array().unwrap();
        assert_eq!(turns2[turns2.len() - 2]["cost_usd"], 0.03);
        assert_eq!(turns2[turns2.len() - 1]["cost_usd"], 0.02);
        assert_eq!(state2["cost_usd"], 0.05);

        let reloaded2 = fx.store.get(&cost_sid).unwrap();
        assert_eq!(reloaded2["cost_totals"]["p-sess-1"], 0.12);
        assert_eq!(reloaded2["cost_totals"]["c-sess-1"], 0.07);

        // 4. Cycle-wide `budgets["usd"]` passes remaining USD cap to Claude and stops below $0.01.
        fs::write(&fx.claude_args, "").unwrap();
        let usd_cap_session = create_clash_session(
            &fx,
            "claude",
            "codex",
            json!({"usd": 0.10, "tokens": Value::Null, "seconds": 30}),
            2,
        );
        let mut cycle_cap = Cycle {
            store: fx.store.clone(),
            session: usd_cap_session,
            launch: base_launch(&fx, "claude"),
            executables: executables.clone(),
            context: String::new(),
            adapter: Arc::new(PassThroughAdapter),
        };
        let state_cap = cycle_cap
            .drive(
                "MODE_USD_CAP_CHECK",
                "implement",
                false,
                Arc::new(AtomicBool::new(false)),
                emit.clone(),
            )
            .await
            .unwrap();
        // Turn 1 (claude) cost 0.06, Turn 2 (codex) rejected, Turn 3 (claude) got remaining 0.04 and spent 0.02,
        // Turn 4 (codex) finished round 2 -> unresolved.
        assert_eq!(state_cap["outcome"], "unresolved");
        let args_text = fs::read_to_string(&fx.claude_args).unwrap();
        assert!(args_text.contains("--max-budget-usd\n0.1\n"), "{args_text}");
        assert!(
            args_text.contains("--max-budget-usd\n0.04\n"),
            "{args_text}"
        );

        // Remaining < $0.01 stops cleanly before next Claude turn.
        let near_session = create_clash_session(
            &fx,
            "claude",
            "codex",
            json!({"usd": 0.05, "tokens": Value::Null, "seconds": 30}),
            2,
        );
        let mut cycle_near = Cycle {
            store: fx.store.clone(),
            session: near_session,
            launch: base_launch(&fx, "claude"),
            executables: executables.clone(),
            context: String::new(),
            adapter: Arc::new(PassThroughAdapter),
        };
        let state_near = cycle_near
            .drive(
                "MODE_USD_NEAR_EXHAUST",
                "implement",
                false,
                Arc::new(AtomicBool::new(false)),
                emit.clone(),
            )
            .await
            .unwrap();
        assert_eq!(state_near["outcome"], "incomplete");
        assert_eq!(
            state_near["message"],
            "Clash reached its USD budget; cycle incomplete."
        );

        // Exact exhaustion (`cycle_usd >= limit` after Turn 1) stops before Turn 2.
        let exact_session = create_clash_session(
            &fx,
            "claude",
            "codex",
            json!({"usd": 0.06, "tokens": Value::Null, "seconds": 30}),
            2,
        );
        let mut cycle_exact = Cycle {
            store: fx.store.clone(),
            session: exact_session,
            launch: base_launch(&fx, "claude"),
            executables: executables.clone(),
            context: String::new(),
            adapter: Arc::new(PassThroughAdapter),
        };
        let state_exact = cycle_exact
            .drive(
                "MODE_USD_CAP_CHECK",
                "implement",
                false,
                Arc::new(AtomicBool::new(false)),
                emit.clone(),
            )
            .await
            .unwrap();
        assert_eq!(state_exact["outcome"], "incomplete");
        assert_eq!(
            state_exact["message"],
            "Clash reached its USD budget; cycle incomplete."
        );
        assert_eq!(state_exact["turns"].as_array().unwrap().len(), 1);

        // `limit_reached == Some("USD")` (`error_max_budget_usd`) stops cleanly with incomplete.
        let hit_session = create_clash_session(
            &fx,
            "claude",
            "codex",
            json!({"usd": 0.05, "tokens": Value::Null, "seconds": 30}),
            2,
        );
        let mut cycle_hit = Cycle {
            store: fx.store.clone(),
            session: hit_session,
            launch: base_launch(&fx, "claude"),
            executables,
            context: String::new(),
            adapter: Arc::new(PassThroughAdapter),
        };
        let state_hit = cycle_hit
            .drive(
                "MODE_USD_LIMIT_HIT",
                "implement",
                false,
                Arc::new(AtomicBool::new(false)),
                emit,
            )
            .await
            .unwrap();
        assert_eq!(state_hit["outcome"], "incomplete");
        assert_eq!(
            state_hit["message"],
            "Clash reached its USD budget; cycle incomplete."
        );

        fx.close().await;
    });
}

fn fake_system_claude(bin_dir: &Path) -> PathBuf {
    let program = bin_dir.join("fake-system-claude");
    let script = r#"#!/bin/sh
case " $* " in
  *" --input-format stream-json "*)
    while IFS= read -r line; do
      case "$line" in
        *'"subtype":"initialize"'*)
          echo '{"type":"control_response","response":{"subtype":"success","request_id":"harness-initialize","response":{"current_permission_mode":"auto"}}}'
          ;;
        *'"subtype":"set_permission_mode"'*)
          echo '{"type":"control_response","response":{"subtype":"success","request_id":"harness-auto-mode","response":{"mode":"auto"}}}'
          ;;
        *'"type":"user"'*)
          echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"sys-claude-1"}'
          mode="$(cat .test-mode 2>/dev/null || echo manual)"
          if [ "$mode" = "auto" ]; then
            own_file="$(pwd)/src/internal.php"
            printf '%s\n' "{\"type\":\"control_request\",\"request_id\":\"sys-auto-intra\",\"request\":{\"subtype\":\"can_use_tool\",\"tool_name\":\"Edit\",\"display_name\":\"Edit\",\"input\":{\"file_path\":\"${own_file}\"},\"description\":\"${own_file}\"}}"
          else
            echo '{"type":"control_request","request_id":"sys-manual-1","request":{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash","input":{"command":"php -v"},"description":"Run PHP check"}}'
          fi
          ;;
        *'"request_id":"sys-auto-intra"'*'"behavior":"allow"'*|*'"behavior":"allow"'*'"request_id":"sys-auto-intra"'*)
          foreign_file="$(cd ../billing && pwd)/src/api.php"
          printf '%s\n' "{\"type\":\"control_request\",\"request_id\":\"sys-auto-cross\",\"request\":{\"subtype\":\"can_use_tool\",\"tool_name\":\"Edit\",\"display_name\":\"Edit\",\"input\":{\"file_path\":\"${foreign_file}\"},\"description\":\"${foreign_file}\"}}"
          ;;
        *'"behavior":"allow"'*)
          echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Updated orders."}]},"session_id":"sys-claude-1"}'
          echo '{"type":"result","subtype":"success","is_error":false,"result":"Updated orders.","structured_output":{"status":"completed","summary":"Updated orders cleanly","checks":[{"name":"php","status":"passed","detail":"ok"}],"changed_files":[],"service_order":["orders"]},"session_id":"sys-claude-1","total_cost_usd":0.02,"usage":{"input_tokens":10,"output_tokens":5}}'
          exit 0
          ;;
        *'"behavior":"deny"'*)
          echo '{"type":"result","subtype":"success","is_error":false,"result":"Denied.","structured_output":{"status":"blocked","summary":"Permission denied","checks":[{"name":"php","status":"not_run","detail":"denied"}],"changed_files":[],"service_order":["orders"]},"session_id":"sys-claude-1","total_cost_usd":0.01,"usage":{"input_tokens":5,"output_tokens":2}}'
          exit 0
          ;;
      esac
    done
    exit 0
    ;;
esac
cat >/dev/null
echo '{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"sys-verify-1"}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"Verified.","structured_output":{"status":"completed","summary":"Verified orders","checks":[{"name":"verify","status":"passed","detail":"ok"}],"changed_files":[],"service_order":["orders"]},"session_id":"sys-verify-1","total_cost_usd":0.01,"usage":{"input_tokens":5,"output_tokens":3}}'
exit 0
"#;
    fs::write(&program, script).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    program
}

struct SystemPhase3cFixture {
    _dir: tempfile::TempDir,
    sys_project_id: String,
    orders_dir: PathBuf,
    sessions: Arc<Sessions>,
    port: u16,
    token: String,
    client: reqwest::Client,
    shutdown: tokio::sync::watch::Sender<bool>,
    server: tokio::task::JoinHandle<()>,
}

impl SystemPhase3cFixture {
    async fn new() -> Self {
        let _ = infra_host::process::use_guardian(PathBuf::from(env!(
            "CARGO_BIN_EXE_infra-harness-inbox-fixture"
        )));
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let source = root.join("source");
        let bin_dir = source.join("bin");
        let sys_dir = root.join("sys");
        let orders_dir = root.join("orders");
        let billing_dir = root.join("billing");
        for path in [
            bin_dir.clone(),
            sys_dir.join("contracts"),
            orders_dir.join("src"),
            billing_dir.join("src"),
        ] {
            fs::create_dir_all(path).unwrap();
        }
        fs::write(sys_dir.join("AGENTS.md"), "# System Policy\n").unwrap();
        fs::write(sys_dir.join("contracts/shared.md"), "# Shared Contract\n").unwrap();
        fs::write(
            sys_dir.join("system.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "name": "phase3c-sys",
                "services": [
                    { "id": "orders", "root": "../orders", "manifest": "ai-service.json" },
                    { "id": "billing", "root": "../billing", "manifest": "ai-service.json" }
                ],
                "shared_sources": [
                    { "kind": "policy", "path": "AGENTS.md" },
                    { "kind": "contract", "path": "contracts/shared.md" }
                ]
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();
        fs::write(orders_dir.join("src/order.php"), "<?php // order\n").unwrap();
        fs::write(orders_dir.join("src/internal.php"), "<?php // internal\n").unwrap();
        fs::write(
            orders_dir.join("ai-service.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "id": "orders",
                "owner": "@orders",
                "description": "Orders",
                "capabilities": [{
                    "id": "orders.place",
                    "description": "Place order",
                    "status": "implemented",
                    "sources": ["src/order.php"]
                }],
                "provides": [{
                    "id": "order.placed",
                    "kind": "event",
                    "version": "1",
                    "sources": ["src/order.php"]
                }],
                "consumes": [],
                "sources": [{ "kind": "code", "path": "src/order.php" }]
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();
        fs::write(billing_dir.join("src/api.php"), "<?php // billing api\n").unwrap();
        fs::write(
            billing_dir.join("ai-service.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "id": "billing",
                "owner": "@billing",
                "description": "Billing",
                "capabilities": [{
                    "id": "billing.charge",
                    "description": "Charge",
                    "status": "implemented",
                    "sources": ["src/api.php"]
                }],
                "provides": [{
                    "id": "billing.api",
                    "kind": "http",
                    "version": "1",
                    "sources": ["src/api.php"]
                }],
                "consumes": [],
                "sources": [{ "kind": "code", "path": "src/api.php" }]
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();

        let claude = fake_system_claude(&bin_dir);
        let store = SessionStore::open(&root.join("state")).unwrap();
        let sys_project_id = store.register(&sys_dir).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        store.register(&orders_dir).unwrap();
        store.register(&billing_dir).unwrap();

        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::from([
                (
                    "claude".to_owned(),
                    Provider {
                        id: "claude".into(),
                        name: "Fake System Claude".into(),
                        available: true,
                        executable: Some(claude),
                        detail: "Test".into(),
                    },
                ),
                (
                    "codex".to_owned(),
                    Provider {
                        id: "codex".into(),
                        name: "Fake Codex".into(),
                        available: false,
                        executable: None,
                        detail: "Unused".into(),
                    },
                ),
                (
                    "cursor".to_owned(),
                    Provider {
                        id: "cursor".into(),
                        name: "Fake Cursor".into(),
                        available: false,
                        executable: None,
                        detail: "Unused".into(),
                    },
                ),
            ]))),
        });
        static SYS_CATALOG: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
        SYS_CATALOG
            .get_or_init(|| async {
                for id in ["claude", "codex", "cursor"] {
                    infra_host::model_catalog::model_options(id, None, None)
                        .await
                        .unwrap();
                }
            })
            .await;
        let knowledge = Arc::new(KnowledgeDomain::new(store.clone(), source.clone()));
        let links = Arc::new(SessionLinks::new());
        let special: BTreeMap<String, Arc<dyn SpecialWorkflow>> = BTreeMap::from([
            (
                "creator".into(),
                Arc::new(UnusedSpecial) as Arc<dyn SpecialWorkflow>,
            ),
            (
                "system_run".into(),
                Arc::new(infra_harness::bindings::SystemWorkflow {
                    store: store.clone(),
                    links: links.clone(),
                    discovery: false,
                    bridge: None,
                }) as Arc<dyn SpecialWorkflow>,
            ),
            (
                "system_discovery".into(),
                Arc::new(infra_harness::bindings::SystemWorkflow {
                    store: store.clone(),
                    links: links.clone(),
                    discovery: true,
                    bridge: None,
                }) as Arc<dyn SpecialWorkflow>,
            ),
        ]);
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
        links.bind(Arc::downgrade(&sessions)).unwrap();
        let results = Arc::new(
            Results::new(
                store.clone(),
                Arc::new(ResultsBridge {
                    links: links.clone(),
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
        let system_bridge = Arc::new(infra_harness::bindings::SystemBridge {
            links: links.clone(),
            store: store.clone(),
        });
        let system_manager = Arc::new(
            infra_system::SystemManager::new(source.clone(), store.state.clone(), system_bridge)
                .unwrap(),
        );
        let systems = Arc::new(infra_harness::bindings::SystemsDomain::new(system_manager));

        let mut domains: BTreeMap<String, Arc<dyn Domain>> = [
            "setup",
            "accelerators",
            "creator",
            "discovery",
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
        .map(|name| (name.to_owned(), Arc::new(StubDomain) as Arc<dyn Domain>))
        .collect();
        domains.insert("systems".into(), systems);
        domains.insert("knowledge".into(), knowledge);
        for name in ["sessions", "brain", "context"] {
            domains.insert(name.into(), sessions.clone());
        }
        domains.insert(
            "results".into(),
            Arc::new(infra_harness::results::ResultService {
                results: results.clone(),
            }),
        );
        let boot = store.clone();
        let closing = sessions.clone();
        let services = Arc::new(
            NativeServices::new(
                store,
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
        let token = "phase3c-sys-token".to_owned();
        let app = App {
            services,
            token: token.clone(),
            instance: "phase3c-sys-test".into(),
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
            sys_project_id,
            orders_dir,
            sessions,
            port,
            token,
            client: reqwest::Client::new(),
            shutdown,
            server,
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

    async fn wait_run(&self, rid: &str, what: &str, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let (status, view) = self
                .http(Method::GET, &format!("/api/system-runs/{rid}"), None)
                .await;
            assert_eq!(status, 200, "{what}: {view}");
            if done(&view) {
                return view;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {view}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn close(self) {
        let _ = self.shutdown.send(true);
        let _ = self.sessions.close();
        let _ = self.sessions.wait_closed().await;
        let _ = self.server.await;
    }
}

fn run_large_stack<F>(future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_stack_size(32 << 20)
        .build()
        .unwrap();
    rt.block_on(async { tokio::spawn(future).await.unwrap() });
}

#[test]
fn system_workflow_manual_permissions_ask_wait_allow_and_verify_via_http() {
    run_large_stack(async {
        let fx = SystemPhase3cFixture::new().await;
        fs::write(fx.orders_dir.join(".test-mode"), "manual").unwrap();

        let (prep_status, prep) = fx
            .http(
                Method::POST,
                "/api/system-runs",
                Some(json!({
                    "project_id": fx.sys_project_id,
                    "config_path": "system.json",
                    "task": "Update orders service",
                    "change_id": "chg-3c-manual",
                    "services": ["orders"]
                })),
            )
            .await;
        assert_eq!(prep_status, 201, "{prep}");
        let rid = prep["id"].as_str().unwrap().to_owned();

        let (exec_status, exec) = fx
            .http(
                Method::POST,
                &format!("/api/system-runs/{rid}"),
                Some(json!({
                    "action": "execute",
                    "revision": 0,
                    "provider": "claude",
                    "mode": "edit",
                    "timeout": 30,
                    "access": "service",
                    "permissions": "manual"
                })),
            )
            .await;
        assert_eq!(exec_status, 200, "{exec}");
        assert_eq!(exec["permissions"], "manual");

        let waiting_view = fx
            .wait_run(&rid, "manual waiting permission", |view| {
                !view["waiting_permissions"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
            })
            .await;
        assert_eq!(waiting_view["permissions"], "manual");
        let waiting = waiting_view["waiting_permissions"].as_array().unwrap();
        assert_eq!(waiting.len(), 1);
        let req_id = waiting[0]["request_id"].as_str().unwrap().to_owned();
        assert!(req_id.starts_with("claude-sys-manual-1-"), "{waiting:?}");
        let sid = waiting_view["session_id"].as_str().unwrap().to_owned();

        let (decide_status, decide) = fx
            .http(
                Method::POST,
                &format!("/api/sessions/{sid}/permissions/{req_id}"),
                Some(json!({"decision": "allow"})),
            )
            .await;
        assert_eq!(decide_status, 200, "{decide}");

        let completed = fx
            .wait_run(&rid, "manual system run completion", |view| {
                view["active"] == false && view["status"] == "completed"
            })
            .await;
        assert_eq!(completed["permissions"], "manual");
        assert!(
            completed["waiting_permissions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            completed["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|ev| ev["kind"] == "permission"
                    && ev["request_id"] == req_id
                    && ev["state"] == "allowed"
                    && ev["by"] == "person"),
            "{}",
            completed["events"]
        );

        fx.close().await;
    });
}

#[test]
fn system_workflow_auto_mode_auto_allows_own_service_and_escalates_boundary_ask_via_http() {
    run_large_stack(async {
        let fx = SystemPhase3cFixture::new().await;
        fs::write(fx.orders_dir.join(".test-mode"), "auto").unwrap();

        let (prep_status, prep) = fx
            .http(
                Method::POST,
                "/api/system-runs",
                Some(json!({
                    "project_id": fx.sys_project_id,
                    "config_path": "system.json",
                    "task": "Update orders cleanly",
                    "change_id": "chg-3c-auto",
                    "services": ["orders"]
                })),
            )
            .await;
        assert_eq!(prep_status, 201, "{prep}");
        let rid = prep["id"].as_str().unwrap().to_owned();

        let (exec_status, exec) = fx
            .http(
                Method::POST,
                &format!("/api/system-runs/{rid}"),
                Some(json!({
                    "action": "execute",
                    "revision": 0,
                    "provider": "claude",
                    "mode": "edit",
                    "timeout": 30,
                    "access": "service",
                    "permissions": "auto"
                })),
            )
            .await;
        assert_eq!(exec_status, 200, "{exec}");
        assert_eq!(exec["permissions"], "auto");

        // Own-service Edit (`sys-auto-intra`) must auto-allow without surfacing a waiter;
        // foreign-service Edit (`sys-auto-cross`) escalates to human confirmation.
        let waiting_view = fx
            .wait_run(&rid, "auto boundary escalation waiter", |view| {
                !view["waiting_permissions"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
            })
            .await;
        assert_eq!(waiting_view["permissions"], "auto");
        let waiting = waiting_view["waiting_permissions"].as_array().unwrap();
        assert_eq!(waiting.len(), 1);
        let req_id = waiting[0]["request_id"].as_str().unwrap().to_owned();
        assert!(req_id.starts_with("claude-sys-auto-cross-"), "{waiting:?}");
        assert_eq!(waiting[0]["session_option"], false);
        assert!(
            waiting[0]["detail"]
                .as_str()
                .unwrap_or("")
                .contains("[Auto -> Manual boundary escalation:"),
            "{waiting:?}"
        );
        let sid = waiting_view["session_id"].as_str().unwrap().to_owned();

        let (decide_status, decide) = fx
            .http(
                Method::POST,
                &format!("/api/sessions/{sid}/permissions/{req_id}"),
                Some(json!({"decision": "allow"})),
            )
            .await;
        assert_eq!(decide_status, 200, "{decide}");

        let completed = fx
            .wait_run(&rid, "auto boundary escalated run completion", |view| {
                view["active"] == false && view["status"] == "completed"
            })
            .await;
        assert_eq!(completed["permissions"], "auto");
        assert!(
            completed["waiting_permissions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            completed["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|ev| ev["kind"] == "permission"
                    && ev["request_id"] == req_id
                    && ev["state"] == "allowed"
                    && ev["by"] == "person"),
            "{}",
            completed["events"]
        );

        fx.close().await;
    });
}

#[cfg(target_os = "linux")]
#[test]
fn results_capture_sandboxed_scrubs_secrets_and_isolates_network_and_worktree() {
    run_on_worker(async {
        if !infra_host::sandbox::available() {
            return;
        }
        let fx = Phase3cFixture::new().await;
        let ws = fx.root.join("sandboxed-ws");
        fs::create_dir_all(&ws).unwrap();
        let script = r#"echo "if=$(grep -c : /proc/net/dev)"; echo "key=${OPENAI_API_KEY:-unset}"; echo "prompt=$GIT_TERMINAL_PROMPT""#;
        let cap = fx
            .results
            .capture_sandboxed(
                &["/bin/sh".into(), "-c".into(), script.into()],
                &ws,
                5,
                Arc::new(AtomicBool::new(false)),
                infra_harness::results::SandboxOptions::default(),
                |_| Ok(()),
            )
            .await
            .unwrap();
        assert_eq!(cap.exit_code, Some(0), "output: {}", cap.output);
        assert!(
            cap.output.contains("if=1"),
            "expected loopback only: {}",
            cap.output
        );
        assert!(cap.output.contains("prompt=0"), "output: {}", cap.output);
        assert!(!ws.join(".harness-tmp").exists());
        let env = cap.tool_envelope();
        assert_eq!(env["ok"], true);
        assert_eq!(env["exit_code"], 0);
        assert_eq!(env["diagnostic"], "");
        fx.close().await;
    });
}

#[test]
fn system_verify_program_checks_skip_llm_on_pass_and_wake_on_fail() {
    run_large_stack(async {
        let _ = infra_host::process::use_guardian(PathBuf::from(env!(
            "CARGO_BIN_EXE_infra-harness-inbox-fixture"
        )));
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let sys_dir = root.join("sys");
        let orders_dir = root.join("orders");
        fs::create_dir_all(sys_dir.join("contracts")).unwrap();
        fs::create_dir_all(orders_dir.join("src")).unwrap();
        fs::write(sys_dir.join("AGENTS.md"), "# System Policy\n").unwrap();
        fs::write(sys_dir.join("contracts/shared.md"), "# Contract\n").unwrap();
        fs::write(
            sys_dir.join("system.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "name": "det-verify-sys",
                "services": [
                    { "id": "orders", "root": "../orders", "manifest": "ai-service.json" }
                ],
                "shared_sources": [
                    { "kind": "policy", "path": "AGENTS.md" },
                    { "kind": "contract", "path": "contracts/shared.md" }
                ]
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();
        fs::write(orders_dir.join("src/order.php"), "<?php // order\n").unwrap();
        fs::write(
            orders_dir.join("ai-service.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "id": "orders",
                "owner": "@orders",
                "description": "Orders",
                "capabilities": [{
                    "id": "orders.place",
                    "description": "Place order",
                    "status": "implemented",
                    "sources": ["src/order.php"]
                }],
                "provides": [],
                "consumes": [],
                "sources": [{ "kind": "code", "path": "src/order.php" }]
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();

        let calls_log = root.join("claude-calls.log");
        let fake_cli = root.join("fake-claude-sys");
        let script = format!(
            r#"#!/bin/sh
prompt=$(cat)
echo "call" >> "{log}"
case "$prompt" in
  *'"phase":"contracts"'*)
    report='{{"status":"completed","summary":"Agreement ready","checks":[{{"name":"contract","status":"passed","detail":"ok"}}],"changed_files":[],"service_order":["orders"]}}'
    ;;
  *'"phase":"verify"'*)
    report='{{"status":"completed","summary":"Verified after failure","checks":[{{"name":"verify","status":"passed","detail":"ok"}}],"changed_files":[],"service_order":[]}}'
    ;;
  *)
    report='{{"status":"completed","summary":"Checked orders","checks":[{{"name":"orders","status":"passed","detail":"ok"}}],"changed_files":[],"service_order":[]}}'
    ;;
esac
echo '{{"type":"system","subtype":"init","claude_code_version":"2.1.300","session_id":"det-sys-1"}}'
echo '{{"type":"assistant","message":{{"id":"m1","role":"assistant","usage":{{"input_tokens":15,"cache_read_input_tokens":40,"cache_creation_input_tokens":5}}}},"session_id":"det-sys-1"}}'
printf '{{"type":"result","subtype":"success","is_error":false,"result":"ok","structured_output":%s,"session_id":"det-sys-1","total_cost_usd":0.01,"usage":{{"input_tokens":15,"output_tokens":8,"cache_read_input_tokens":40,"cache_creation_input_tokens":5}}}}\n' "$report"
"#,
            log = calls_log.display(),
        );
        write_executable(&fake_cli, &script);
        let cli_str = fake_cli.to_string_lossy().into_owned();

        // 1. Passing deterministic program check skips verify LLM call completely (0 model calls).
        let run_pass = root.join("run-pass");
        let mut sys = infra_system::catalog::System::new(
            &sys_dir.join("system.json"),
            std::slice::from_ref(&orders_dir),
        )
        .unwrap();
        let mut plan = sys
            .plan(
                "Verify order contract",
                "chg-pass",
                &["orders".into()],
                &[],
                16000,
                16,
            )
            .unwrap();
        for step in plan["steps"].as_array_mut().unwrap() {
            if step["id"] == "verify" {
                step["program_checks"] = json!([{
                    "name": "orders-lint",
                    "argv": ["/bin/sh", "-c", "echo orders-lint-ok"],
                    "cwd_service": "orders"
                }]);
            }
        }
        infra_system::native::create_run(
            &mut sys,
            &plan,
            &run_pass,
            "claude",
            Some(&cli_str),
            "read-only",
            30,
            "service",
        )
        .await
        .unwrap();
        let mut events = Vec::new();
        let finished = infra_system::runner::drive_run(
            &mut sys,
            &run_pass,
            None,
            infra_system::runner::Acceptance::default(),
            Arc::new(AtomicBool::new(false)),
            &mut |ev| {
                events.push(ev);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(finished["status"], "completed", "{finished}");
        let log_pass = fs::read_to_string(&calls_log).unwrap_or_default();
        assert_eq!(
            log_pass.lines().count(),
            2,
            "verify step should NOT invoke LLM on passing program_checks:\n{log_pass}"
        );
        let verify_receipt: Value =
            serde_json::from_slice(&fs::read(run_pass.join("verify-a1.json")).unwrap()).unwrap();
        assert_eq!(verify_receipt["deterministic"], true);
        assert_eq!(verify_receipt["ok"], true);
        assert_eq!(verify_receipt["usage_components"]["model_calls"], 0);
        assert_eq!(
            verify_receipt["usage_components"]["source"],
            "deterministic"
        );
        let svc_receipt: Value =
            serde_json::from_slice(&fs::read(run_pass.join("service-orders-a1.json")).unwrap())
                .unwrap();
        assert_eq!(svc_receipt["deterministic"], false);
        assert_eq!(svc_receipt["usage_components"]["input_tokens"], 15);
        assert_eq!(svc_receipt["usage_components"]["output_tokens"], 8);
        assert_eq!(
            svc_receipt["usage_components"]["cache_read_input_tokens"],
            40
        );

        // 2. Failing deterministic program check wakes verify LLM with bounded diagnostics.
        fs::remove_file(&calls_log).unwrap();
        let run_fail = root.join("run-fail");
        let mut plan_fail = sys
            .plan(
                "Verify order contract",
                "chg-fail",
                &["orders".into()],
                &[],
                16000,
                16,
            )
            .unwrap();
        for step in plan_fail["steps"].as_array_mut().unwrap() {
            if step["id"] == "verify" {
                step["program_checks"] = json!([{
                    "name": "orders-lint",
                    "argv": ["/bin/sh", "-c", "echo lint-error-xyz && exit 1"],
                    "cwd_service": "orders"
                }]);
            }
        }
        infra_system::native::create_run(
            &mut sys,
            &plan_fail,
            &run_fail,
            "claude",
            Some(&cli_str),
            "read-only",
            30,
            "service",
        )
        .await
        .unwrap();
        let finished_fail = infra_system::runner::drive_run(
            &mut sys,
            &run_fail,
            None,
            infra_system::runner::Acceptance::default(),
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .await
        .unwrap();
        assert_eq!(finished_fail["status"], "completed");
        let log_fail = fs::read_to_string(&calls_log).unwrap_or_default();
        assert_eq!(
            log_fail.lines().count(),
            3,
            "failing program_checks must wake verify LLM:\n{log_fail}"
        );
        let verify_input: Value =
            serde_json::from_slice(&fs::read(run_fail.join("verify-a1.input.json")).unwrap())
                .unwrap();
        let prompt_sent = verify_input["prompt"].as_str().unwrap_or("");
        assert!(
            prompt_sent.contains("lint-error-xyz"),
            "verify LLM prompt missing failure diagnostic: {prompt_sent}"
        );
    });
}
