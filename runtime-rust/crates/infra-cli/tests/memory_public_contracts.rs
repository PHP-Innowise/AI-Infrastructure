//! Public compatibility runs the real installed-style native engine in disposable roots.
//! No old Python, providers, trust writes, or parent environment mutation.
#[path = "support/executable.rs"]
mod executable;
use infra_core::{BrainRepository, CreateRecord, Roots, RuntimeContext, Store, UpdateRecord};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    spool: PathBuf,
    binary: PathBuf,
}
impl Fixture {
    fn new(automatic_promotion: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra-memory-public-{}",
            infra_core::domain::uuid4()
        ));
        fs::create_dir(&root).unwrap();
        let root = infra_core::fs::canonicalize(root).unwrap();
        let project = root.join("installed project with spaces");
        let spool = root.join("spool");
        for name in [
            "project-brain/config",
            "memory-bank/bin",
            "memory-bank/chunks",
            ".agents/skills",
            "specs",
        ] {
            fs::create_dir_all(project.join(name)).unwrap();
        }
        fs::create_dir(&spool).unwrap();
        fs::write(
            project.join("project-brain/config/runtime.json"),
            serde_json::to_vec(&json!({
                "mode":"governed","framework":"generic","canonical_edition":".agents",
                "automatic_promotion":automatic_promotion,"telemetry_enabled":false
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            project.join("AGENTS.md"),
            "# Project policy\nVerify cited sources.\n",
        )
        .unwrap();
        fs::write(
            project.join("specs/source.md"),
            "# Cobalt metadata boundary\nCobalt retries stop after the third guarded write.\n",
        )
        .unwrap();
        fs::write(project.join("memory-bank/README.md"), "# Memory Bank\n").unwrap();
        fs::write(project.join("memory-bank/INDEX.md"), "# Memory Index\n").unwrap();
        let binary = project.join(if cfg!(windows) {
            "memory-bank/bin/infra-engine.exe"
        } else {
            "memory-bank/bin/infra-engine"
        });
        executable::copy(env!("CARGO_BIN_EXE_infra-engine"), &binary);
        let fixture = Self {
            root,
            project,
            spool,
            binary,
        };
        fixture.git(&["-c", "init.templateDir=", "init", "-q"]);
        fixture.git(&[
            "add",
            "--",
            "AGENTS.md",
            "specs/source.md",
            "project-brain/config/runtime.json",
        ]);
        fixture
    }
    fn isolate(&self, command: &mut Command) {
        command
            .current_dir(&self.project)
            .env("HOME", self.root.join("home"))
            .env("CODEX_HOME", self.root.join("codex-home"))
            .env("TMPDIR", &self.spool)
            .env("CONTEXT_NATIVE_RESULT_REQUEST", "every-turn")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root.join("empty-gitconfig"));
        for key in [
            "ACCELERATOR_HOME",
            "ACCELERATOR_STATE_DIR",
            "ACCELERATOR_PROJECT_DIR",
            "INFRA_SOURCE_ROOT",
            "PROJECT_BRAIN_OWNER",
            "PROJECT_BRAIN_MODE",
            "CLAUDE_CODE_TMPDIR",
            "CONTEXT_QUEUE_DRAIN",
            "CONTEXT_HOOK_BUDGET",
            "CONTEXT_MEMORY_RECOVERY",
            "CONTEXT_CAPSULE_DELIVERED",
            "GIT_DIR",
            "GIT_COMMON_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_CONFIG",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_COUNT",
        ] {
            command.env_remove(key);
        }
    }
    fn git(&self, arguments: &[&str]) {
        let mut command = Command::new("git");
        self.isolate(&mut command);
        let output = command.args(arguments).output().unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn execute(&self, group: &str, arguments: &[&str], bytes: &[u8]) -> Output {
        let mut command = Command::new(&self.binary);
        self.isolate(&mut command);
        let mut child = command
            .arg(group)
            .arg("--root")
            .arg(&self.project)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        {
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(bytes).unwrap();
        }
        child.wait_with_output().unwrap()
    }
    fn run(&self, arguments: &[&str], input: Option<&Value>) -> Output {
        self.execute(
            "memory",
            arguments,
            &input
                .map(|v| serde_json::to_vec(v).unwrap())
                .unwrap_or_default(),
        )
    }
    fn success(&self, arguments: &[&str], input: Option<&Value>) -> Value {
        let output = self.run(arguments, input);
        assert!(
            output.status.success(),
            "{arguments:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn context(&self) -> RuntimeContext {
        RuntimeContext::new(
            Roots::installed(&self.project).unwrap(),
            "local",
            "governed",
        )
        .unwrap()
    }
    fn brain(&self) -> BrainRepository {
        BrainRepository::new(self.context()).unwrap()
    }
    fn start(&self, id: &str) -> Value {
        self.success(
            &[
                "start",
                "--task-id",
                id,
                "--goal",
                "Verify the cobalt boundary",
                "--json",
            ],
            None,
        )
    }
    fn native_state(&self) -> (PathBuf, Value) {
        let directory = self.project.join("project-brain/local/native-results");
        let paths = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        assert_eq!(paths.len(), 1);
        let state = serde_json::from_slice(&fs::read(&paths[0]).unwrap()).unwrap();
        (paths[0].clone(), state)
    }
    fn spool_directory(&self) -> PathBuf {
        #[cfg(unix)]
        let uid = {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(&self.root).unwrap().uid().to_string()
        };
        #[cfg(not(unix))]
        let uid = "user".to_string();
        self.spool
            .join(format!("ai-accelerator-native-spool-{uid}"))
    }
    fn rpc(&self, calls: &[(&str, Value)]) -> BTreeMap<u64, Value> {
        let mut requests = vec![
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"native-contract-test","version":"1"}
            }}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        ];
        for (index, (name, arguments)) in calls.iter().enumerate() {
            requests.push(json!({"jsonrpc":"2.0","id":index+2,"method":"tools/call",
                "params":{"name":name,"arguments":arguments}}));
        }
        let mut input = Vec::new();
        for request in requests {
            serde_json::to_writer(&mut input, &request).unwrap();
            input.push(b'\n');
        }
        let output = self.execute("mcp", &["--host", "codex"], &input);
        assert!(
            output.status.success(),
            "MCP stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .map(|value| (value["id"].as_u64().unwrap(), value))
            .collect()
    }
    fn chunks(&self) -> Vec<PathBuf> {
        fs::read_dir(self.project.join("memory-bank/chunks"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
            .collect()
    }
    fn finding(&self, id: &str, title: &str) -> Value {
        self.success(
            &[
                "brain-create",
                "finding",
                "--external-id",
                id,
                "--title",
                title,
                "--source",
                "specs/source.md",
                "--json",
            ],
            None,
        )
    }
    fn verify(&self, id: &str) {
        self.success(
            &[
                "brain-update",
                "--record-id",
                id,
                "--revision",
                "auto",
                "--authority",
                "verified",
                "--reason",
                "Verified after re-reading the cited source",
                "--json",
            ],
            None,
        );
    }
    fn resolve(&self, id: &str) -> Value {
        self.success(
            &[
                "brain-update",
                "--record-id",
                id,
                "--revision",
                "auto",
                "--progress",
                "Cobalt retries stop after the third guarded write.",
                "--transition",
                "resolved",
                "--reason",
                "Resolved after verification",
                "--json",
            ],
            None,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn next_prompt_and_continuation_stop_drain_pending_native_save_exactly_once() {
    for (operation, attempt) in [("native-prompt", 0), ("native-stop", 1)] {
        let fixture = Fixture::new(false);
        fixture.start("TASK-SPOOL");
        let prompt = json!({"hook_event_name":"UserPromptSubmit","session_id":"native-contract-a","prompt":"Inspect cobalt boundaries."});
        let stopped = json!({"hook_event_name":"Stop","session_id":"native-contract-a","stop_hook_active":false});
        fixture.success(
            &[
                "native-prompt",
                "--task-id",
                "TASK-SPOOL",
                "--host",
                "codex",
                "--json",
            ],
            Some(&prompt),
        );
        let requested = fixture.success(
            &[
                "native-stop",
                "--task-id",
                "TASK-SPOOL",
                "--host",
                "codex",
                "--json",
            ],
            Some(&stopped),
        );
        assert_eq!(requested["decision"], "block");
        let (path, state) = fixture.native_state();
        assert_eq!(state["phase"], "requested");
        let key = path.file_stem().unwrap().to_str().unwrap();
        let directory = fixture.spool_directory();
        fs::create_dir_all(&directory).unwrap();
        // Seed the exact persisted spool wire, including an interrupted attempt,
        // to isolate public hook drain wiring from platform permission mechanics.
        let pending = directory.join(if attempt == 0 {
            format!("{key}.json")
        } else {
            format!("{key}.attempt{attempt}.json")
        });
        fs::write(
            &pending,
            serde_json::to_vec(&json!({
                "profile":"native-result-spool-v1","token":state["token"],"skip":false,
                "created":chrono::Utc::now().timestamp(),
                "data":{"progress":"The spooled observation was saved once.","verified":true,
                    "capture_decision":"no_reusable_knowledge","learnings":[]}
            }))
            .unwrap(),
        )
        .unwrap();
        let payload = if operation == "native-stop" {
            json!({"hook_event_name":"Stop","session_id":"native-contract-a","stop_hook_active":true})
        } else {
            prompt
        };
        fixture.success(
            &[
                operation,
                "--task-id",
                "TASK-SPOOL",
                "--host",
                "codex",
                "--json",
            ],
            Some(&payload),
        );
        let saved = fixture.success(&["get", "--task-id", "TASK-SPOOL", "--json"], None);
        assert_eq!(saved["progress"], "The spooled observation was saved once.");
        assert!(!pending.exists());
        assert!(fs::read_dir(&directory).unwrap().all(|entry| {
            entry
                .unwrap()
                .path()
                .extension()
                .is_none_or(|extension| extension != "json")
        }));
        fixture.success(
            &[
                operation,
                "--task-id",
                "TASK-SPOOL",
                "--host",
                "codex",
                "--json",
            ],
            Some(&payload),
        );
        let replayed = fixture.success(&["get", "--task-id", "TASK-SPOOL", "--json"], None);
        assert_eq!(
            replayed["revision"], saved["revision"],
            "a second hook must not apply the result twice"
        );
        assert_eq!(replayed["progress"], saved["progress"]);
    }
}

#[test]
fn mcp_returns_canonical_status_and_refresh_without_bypassing_scope_guards() {
    let fixture = Fixture::new(false);
    fixture.start("TASK-MCP");
    let finding = fixture.finding("MCP-COBALT", "Cobalt metadata boundary");
    fixture.success(
        &[
            "brain-update",
            "--record-id",
            finding["id"].as_str().unwrap(),
            "--revision",
            "auto",
            "--progress",
            "Cobalt retries stop after the third guarded write.",
            "--json",
        ],
        None,
    );
    fixture
        .brain()
        .create(CreateRecord {
            external_id: "TASK-PRIVATE".into(),
            title: "PRIVATE-SCOPE-CANARY".into(),
            privacy: "private".into(),
            ..CreateRecord::default()
        })
        .unwrap();
    fixture.start("TASK-CLOSED");
    fixture.success(
        &[
            "complete",
            "--task-id",
            "TASK-CLOSED",
            "--revision",
            "1",
            "--outcome",
            "Verified the bounded behavior",
            "--json",
        ],
        None,
    );
    let replies = fixture.rpc(&[
        ("memory_status", json!({})),
        ("memory_retrieve", json!({"task_id":"TASK-MCP","query":"Cobalt metadata boundary","paths":["specs/source.md"]})),
        ("memory_retrieve", json!({"task_id":"TASK-PRIVATE","query":"PRIVATE-SCOPE-CANARY"})),
        ("memory_checkpoint", json!({"task_id":"TASK-CLOSED"})),
        ("memory_status", json!({"unexpected":true})),
        ("memory_retrieve", json!({"task_id":"TASK-MCP","query":"Cobalt metadata boundary","paths":["../outside.md"]})),
    ]);
    let status = &replies[&2]["result"];
    assert_eq!(status["isError"], false);
    let status = &status["structuredContent"];
    assert!(status["working"].is_u64());
    assert!(status["documents"].is_u64());
    assert!(status["episodes"].is_u64());
    assert_eq!(status["authority"], "project-brain");
    assert_eq!(status["workspace"]["layout"], "installed");
    assert_eq!(
        Path::new(status["workspace"]["project"].as_str().unwrap()),
        fixture.project.as_path()
    );
    assert!(status["automatic_memory"].is_object());
    assert!(status["consolidation"].is_object());
    let retrieved = &replies[&3]["result"];
    assert_eq!(retrieved["isError"], false);
    let result = &retrieved["structuredContent"];
    assert!(result["capsule"].is_object());
    assert!(result["capsule_text"].as_str().unwrap().contains("Cobalt"));
    assert_eq!(result["mode"], "governed");
    assert!(result["warnings"].is_array());
    assert!(result["phases"].is_object());
    // The reply carries capsule_text to the model, so its render is recorded
    // before the answer and the funnel is observed; with no session nothing
    // was withheld as a repeat, which stays unobserved rather than zero.
    let funnel = &result["funnel_diagnosis"];
    assert_eq!(funnel["observation_state"], "observed", "{funnel}");
    assert_eq!(
        funnel["boundaries"]["selected_post_pack"]["state"], "exact",
        "{funnel}"
    );
    assert_eq!(
        funnel["boundaries"]["repeat_withheld"]["state"], "unknown",
        "{funnel}"
    );
    let manifest = result["capsule"]["manifest"].as_str().unwrap();
    let trace_name = Path::new(manifest).file_name().unwrap();
    let trace: Value = serde_json::from_slice(
        &fs::read(
            fixture
                .project
                .join("memory-bank/local/retrieval-traces")
                .join(trace_name),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(trace["render"]["form"], "capsule_text", "{trace}");
    assert!(!result.to_string().contains("PRIVATE-SCOPE-CANARY"));
    for id in 4..=7 {
        assert_eq!(
            replies[&id]["result"]["isError"], true,
            "admission failed for request {id}"
        );
        assert!(!replies[&id].to_string().contains("PRIVATE-SCOPE-CANARY"));
    }
}

/// Memory MCP's checkpoint is the command line's `rebind` and `turn --flush`:
/// the files changed since the last turn reach the task although no hook ever
/// ran, and the reply carries the flush report, as the retired server's did.
#[test]
fn mcp_checkpoint_records_the_working_tree_as_a_flushed_turn() {
    let fixture = Fixture::new(false);
    completion_git_commit(&fixture, "Baseline");
    fs::write(
        fixture.project.join("specs/source.md"),
        "# Cobalt metadata boundary\nCobalt retries stop after the fourth guarded write.\n",
    )
    .unwrap();
    let replies = fixture.rpc(&[
        (
            "memory_checkpoint",
            json!({"task_id":"TASK-MCP-CHECKPOINT","goal":"Checkpoint over MCP"}),
        ),
        (
            "memory_checkpoint",
            json!({"task_id":"TASK-MCP-CHECKPOINT"}),
        ),
        ("memory_checkpoint", json!({"task_id":"TASK-MCP-NO-GOAL"})),
    ]);
    // The server tells a client where the record of its work comes from: a
    // Stop hook's native-result save, else memory_record_result.
    let instructions = replies[&1]["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains(
            "memory_record_result before finishing authorized work only where no Stop hook asks for the native-result save"
        ),
        "{instructions}"
    );
    let first = &replies[&2]["result"];
    assert_eq!(first["isError"], false, "{first}");
    let first = &first["structuredContent"];
    let report = &first["checkpoint"];
    assert_eq!(report["flushed"], true, "{first}");
    assert_eq!(report["delta_id"], 1, "{first}");
    assert!(
        report["files"].as_u64().is_some_and(|files| files >= 1),
        "{first}"
    );
    assert_eq!(report["pending"], 0, "{first}");
    assert_eq!(first["revision"], 2, "{first}");
    assert_eq!(report["revision"], first["revision"], "{first}");
    let second = &replies[&3]["result"]["structuredContent"];
    assert_eq!(second["checkpoint"]["flushed"], true, "{second}");
    assert_eq!(second["revision"], 3, "{second}");
    assert_eq!(
        replies[&4]["result"]["isError"], true,
        "a new task needs a goal"
    );

    let view = fixture.success(&["get", "--task-id", "TASK-MCP-CHECKPOINT", "--json"], None);
    assert_eq!(view["revision"], 3, "{view}");
    assert!(
        view["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "specs/source.md"),
        "{view}"
    );
    assert!(
        view["auto_checkpoint"]
            .as_str()
            .unwrap()
            .starts_with("Auto-checkpoint: 1 turn(s) since "),
        "{view}"
    );
}

#[test]
fn mcp_refuses_a_degraded_refresh_with_a_real_read_only_capsule_and_preserves_state() {
    fn rows(holder: &Store, sql: &str) -> Vec<Vec<String>> {
        let mut statement = holder.conn.prepare(sql).unwrap();
        let width = statement.column_count();
        let mut rows = statement
            .query_map([], |row| {
                (0..width)
                    .map(|column| Ok(format!("{:?}", row.get_ref(column)?)))
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        rows.sort_unstable();
        rows
    }
    fn database_snapshot(holder: &Store) -> BTreeMap<String, String> {
        let digest =
            |rows: Vec<Vec<String>>| infra_core::query::sha256(&serde_json::to_vec(&rows).unwrap());
        let mut snapshot = BTreeMap::from([(
            "[schema]".to_string(),
            digest(rows(
                holder,
                "SELECT type,name,tbl_name,rootpage,sql FROM main.sqlite_master",
            )),
        )]);
        let tables = {
            let mut statement = holder.conn.prepare("PRAGMA main.table_list").unwrap();
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, bool>(4)?,
                    ))
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };
        for (schema, table, kind, without_rowid) in tables {
            if schema != "main" || kind == "view" {
                continue;
            }
            let quoted = table.replace('"', "\"\"");
            let fields = if without_rowid { "*" } else { "rowid,*" };
            snapshot.insert(
                table,
                digest(rows(
                    holder,
                    &format!("SELECT {fields} FROM main.\"{quoted}\""),
                )),
            );
        }
        snapshot
    }
    fn unchanged(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>, stage: &str) {
        let mut changed = before
            .keys()
            .chain(after.keys())
            .filter(|table| before.get(*table) != after.get(*table))
            .map(|table| format!("{table}: {:?} -> {:?}", before.get(table), after.get(table)))
            .collect::<Vec<_>>();
        changed.sort();
        changed.dedup();
        assert!(
            changed.is_empty(),
            "SQLite state changed after {stage}: {changed:?}"
        );
    }

    let fixture = Fixture::new(false);
    let task = fixture.start("TASK-MCP-DEGRADED");
    let uuid = task["task_uuid"].as_str().unwrap();
    fixture.success(&["index", "--json"], None);
    let task_before = fixture.brain().get(uuid).unwrap();
    let database = infra_core::storage::default_database(&fixture.context().roots).unwrap();
    let holder = Store::open(&database, Default::default()).unwrap();
    // A RESERVED writer blocks a second writer while still admitting readers.
    // The holder writes no rows and stays alive through both native children.
    holder.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(!holder.conn.is_autocommit());
    // Never open/read/close another database FD while SQLite owns these locks:
    // POSIX close() would cancel this process's locks behind SQLite's back.
    // Hash the full schema and typed rows, including rowids and FTS shadow
    // tables, through the same SQLite connection. Failures show hashes only.
    let database_before = database_snapshot(&holder);

    let cli = fixture.run(
        &[
            "refresh",
            "--task-id",
            uuid,
            "--host",
            "codex",
            "--query",
            "Cobalt metadata boundary",
            "--gate",
            "off",
            "--ephemeral",
            "--capsule-profile",
            "full",
            "--json",
        ],
        None,
    );
    let degraded: Value = serde_json::from_slice(&cli.stdout).unwrap_or_else(|error| {
        panic!(
            "The fixture must reach refresh fallback, not an earlier CLI guard: {error}; stderr: {}",
            String::from_utf8_lossy(&cli.stderr)
        )
    });
    assert_eq!(cli.status.code(), Some(1));
    assert_eq!(degraded["startup_profile"], "read-only-working-v1");
    assert_eq!(degraded["persisted"], false);
    assert!(degraded["capsule"].is_object());
    assert!(degraded["capsule"]["working"].is_object());
    for layer in ["procedural", "semantic", "episodic"] {
        assert_eq!(degraded[layer], "failed");
    }
    assert!(!degraded["warnings"].as_array().unwrap().is_empty());
    unchanged(
        &database_before,
        &database_snapshot(&holder),
        "native CLI fallback",
    );

    let replies = fixture.rpc(&[(
        "memory_retrieve",
        json!({"task_id":uuid,"query":"Cobalt metadata boundary"}),
    )]);
    assert!(!holder.conn.is_autocommit());
    let database_after_mcp = database_snapshot(&holder);
    holder.conn.execute_batch("ROLLBACK").unwrap();
    let database_after_rollback = database_snapshot(&holder);
    drop(holder);
    unchanged(
        &database_before,
        &database_after_mcp,
        "MCP child completion",
    );
    unchanged(
        &database_before,
        &database_after_rollback,
        "holder rollback",
    );
    assert!(
        fixture.brain().get(uuid).unwrap() == task_before,
        "The failed refresh must preserve the complete authoritative task"
    );

    let result = &replies[&2]["result"];
    assert_eq!(
        result["isError"], true,
        "MCP must reject the same degraded refresh that the canonical CLI rejects"
    );
    assert!(result.get("structuredContent").is_none());
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Memory operation failed")
    );
}

#[test]
fn resolving_verified_record_promotes_only_that_record_before_a_turn_boundary() {
    let fixture = Fixture::new(true);
    let brain = fixture.brain();
    let unrelated = brain
        .create(CreateRecord {
            record_type: "finding".into(),
            external_id: "UNRELATED-READY".into(),
            title: "An unrelated retry observation".into(),
            sources: vec!["specs/source.md".into()],
            authority: "verified".into(),
            attestation: Some("agent".into()),
            ..CreateRecord::default()
        })
        .unwrap();
    brain
        .update(
            unrelated["id"].as_str().unwrap(),
            UpdateRecord {
                expected_revision: Some(1),
                transition_to: Some("resolved".into()),
                progress: Some(
                    "The unrelated observation has a different durable consequence.".into(),
                ),
                ..UpdateRecord::default()
            },
        )
        .unwrap();
    let target = fixture.finding("PROMOTE-NOW", "Cobalt metadata boundary");
    let id = target["id"].as_str().unwrap();
    fixture.verify(id);
    let resolved = fixture.resolve(id);
    assert_eq!(resolved["status"], "resolved");
    let promoted = resolved["promotion"]["promoted"].as_array().unwrap();
    assert_eq!(promoted.len(), 1);
    assert_eq!(promoted[0]["record_id"], target["id"]);
    let chunks = fixture.chunks();
    assert_eq!(
        chunks.len(),
        1,
        "unrelated ready records must not be promoted as a side effect"
    );
    let chunk = fs::read_to_string(&chunks[0]).unwrap();
    assert!(chunk.contains(id));
    assert!(!chunk.contains(unrelated["id"].as_str().unwrap()));
    let repeated = fixture.success(
        &[
            "brain-update",
            "--record-id",
            id,
            "--revision",
            "auto",
            "--progress",
            "Cobalt retries stop after the third guarded write.",
            "--json",
        ],
        None,
    );
    assert!(
        repeated["promotion"]["promoted"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.chunks().len(), 1);
}

/// Records are read active first and archived after, as the retired runtime
/// read them: validation names what an agent works with before the archive.
#[test]
fn brain_records_are_read_active_before_archived() {
    let fixture = Fixture::new(false);
    let archived = fixture.finding("ORDER-ARCHIVED", "Cobalt metadata boundary");
    let archived = archived["id"].as_str().unwrap().to_owned();
    fixture.verify(&archived);
    fixture.resolve(&archived);
    fixture.success(&["compact", "--json"], None);
    fixture.finding("ORDER-ACTIVE", "Cobalt retry ceiling");
    let inventory = fixture.brain().inventory(true).unwrap();
    let paths = inventory
        .records
        .iter()
        .map(|image| image.path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();
    let first_archived = paths
        .iter()
        .position(|path| path.contains("/project-brain/archive/"))
        .expect("the resolved finding was archived");
    assert!(
        paths[..first_archived]
            .iter()
            .any(|path| path.contains("/project-brain/dynamic/")),
        "{paths:?}"
    );
    assert!(
        paths[first_archived..]
            .iter()
            .all(|path| path.contains("/project-brain/archive/")),
        "{paths:?}"
    );
}

/// One automatic promotion run makes at most five new attempts, as every run
/// of the retired runtime did; the rest waits for the next run.
#[test]
fn automatic_promotion_makes_five_attempts_a_run() {
    let fixture = Fixture::new(false);
    let topics = [
        (
            "Invoice numbers restart for each tenant",
            "Numbering restarts per tenant ledger at year start.",
        ),
        (
            "Refund dates follow the original invoice",
            "Refund documents copy the invoice issue date verbatim.",
        ),
        (
            "Credit notes reuse the refund rule",
            "Credit memos inherit refund timing semantics unchanged.",
        ),
        (
            "Tax reports group by calendar quarter",
            "Quarterly VAT summaries aggregate lines by quarter boundaries.",
        ),
        (
            "Currency rounding happens per line",
            "Monetary amounts round half-up on each individual line item.",
        ),
        (
            "Archived invoices stay immutable",
            "Closed ledgers reject edits; corrections need cancellation documents.",
        ),
        (
            "Discount codes expire at midnight UTC",
            "Promotional vouchers lapse when the coordinated clock passes midnight.",
        ),
    ];
    for (index, (title, progress)) in topics.iter().enumerate() {
        let created = fixture.success(
            &[
                "brain-create",
                "finding",
                "--external-id",
                &format!("FIVE-{index}"),
                "--title",
                title,
                "--source",
                "specs/source.md",
                "--json",
            ],
            None,
        );
        let id = created["id"].as_str().unwrap();
        fixture.verify(id);
        fixture.success(
            &[
                "brain-update",
                "--record-id",
                id,
                "--revision",
                "auto",
                "--progress",
                progress,
                "--transition",
                "resolved",
                "--reason",
                "Resolved after verification",
                "--json",
            ],
            None,
        );
    }
    let config = fixture.project.join("project-brain/config/runtime.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    value["automatic_promotion"] = json!(true);
    fs::write(&config, serde_json::to_vec(&value).unwrap()).unwrap();

    let first = fixture.success(&["promote-auto", "--json"], None);
    assert_eq!(first["promoted"].as_array().unwrap().len(), 5, "{first}");
    let second = fixture.success(&["promote-auto", "--json"], None);
    assert_eq!(second["promoted"].as_array().unwrap().len(), 2, "{second}");
    assert_eq!(fixture.chunks().len(), 7);
}

#[test]
fn promotion_remains_optional_and_its_failure_does_not_undo_resolution() {
    let disabled = Fixture::new(false);
    let target = disabled.finding("PROMOTE-DISABLED", "Cobalt metadata boundary");
    let id = target["id"].as_str().unwrap();
    disabled.verify(id);
    let resolved = disabled.resolve(id);
    assert_eq!(resolved["status"], "resolved");
    assert_eq!(resolved["promotion"]["enabled"], false);
    assert!(disabled.chunks().is_empty());

    let broken = Fixture::new(true);
    let target = broken.finding("PROMOTE-FAILED", "Cobalt metadata boundary");
    let id = target["id"].as_str().unwrap();
    broken.verify(id);
    fs::remove_dir(broken.project.join("memory-bank/chunks")).unwrap();
    fs::write(
        broken.project.join("memory-bank/chunks"),
        "not a directory\n",
    )
    .unwrap();
    let resolved = broken.resolve(id);
    assert_eq!(resolved["status"], "resolved");
    assert!(
        !resolved["promotion"]["failed"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(broken.brain().get(id).unwrap()["status"], "resolved");
}

#[test]
fn legacy_update_allows_omitted_revision_but_retains_explicit_cas_and_strict_brain_updates() {
    let fixture = Fixture::new(false);
    let task = fixture.start("TASK-OPTIONAL");
    let updated = fixture.success(
        &[
            "update",
            "--task-id",
            "TASK-OPTIONAL",
            "--progress",
            "Legacy caller supplied no revision.",
            "--json",
        ],
        None,
    );
    assert_eq!(updated["revision"], task["revision"].as_u64().unwrap() + 1);
    let stale = fixture.run(
        &[
            "update",
            "--task-id",
            "TASK-OPTIONAL",
            "--revision",
            "1",
            "--progress",
            "Stale writer must not replace state.",
            "--json",
        ],
        None,
    );
    assert!(!stale.status.success());
    let current = fixture.success(&["get", "--task-id", "TASK-OPTIONAL", "--json"], None);
    assert_eq!(current["progress"], updated["progress"]);
    let automatic = fixture.success(
        &[
            "update",
            "--task-id",
            "TASK-OPTIONAL",
            "--revision",
            "auto",
            "--progress",
            "Locked automatic revision.",
            "--json",
        ],
        None,
    );
    assert_eq!(
        automatic["revision"],
        updated["revision"].as_u64().unwrap() + 1
    );
    let strict = fixture.run(
        &[
            "brain-update",
            "--record-id",
            task["task_uuid"].as_str().unwrap(),
            "--progress",
            "Brain operations still require explicit revision.",
            "--json",
        ],
        None,
    );
    assert!(!strict.status.success());
    let final_state = fixture.success(&["get", "--task-id", "TASK-OPTIONAL", "--json"], None);
    assert_eq!(final_state["revision"], automatic["revision"]);
    assert_eq!(final_state["progress"], automatic["progress"]);
}

fn public_fixture_store(fixture: &Fixture) -> Store {
    let database = infra_core::storage::default_database(&fixture.context().roots).unwrap();
    Store::open(database, Default::default()).unwrap()
}

fn promoted_source_finding(fixture: &Fixture) -> String {
    let config = fixture.project.join("project-brain/config/runtime.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    value["capsule_profile"] = "recall".into();
    fs::write(config, serde_json::to_vec(&value).unwrap()).unwrap();
    fixture.start("TASK-FRESHNESS");
    let saved = fixture.success(
        &[
            "record-result",
            "--task-id",
            "TASK-FRESHNESS",
            "--result-id",
            "source-finding",
            "--revision",
            "auto",
            "--input",
            "-",
            "--json",
        ],
        Some(&json!({
            "verified":true,
            "learnings":[{
                "type":"finding", "title":"Cobalt guarded retry limit",
                "consequence":"Cobalt retries stop after the third guarded write.",
                "sources":["specs/source.md"]
            }]
        })),
    );
    assert_eq!(saved["promotion"]["promoted"].as_array().unwrap().len(), 1);
    fixture.success(&["index", "--json"], None);
    let chunks = fixture.chunks();
    assert_eq!(chunks.len(), 1);
    chunks[0]
        .strip_prefix(&fixture.project)
        .unwrap()
        .to_str()
        .unwrap()
        .replace('\\', "/")
}

#[test]
fn promoted_memory_marks_changed_citations_in_retrieve_and_recall_refresh() {
    let fixture = Fixture::new(true);
    let chunk = promoted_source_finding(&fixture);
    let arguments = [
        "retrieve",
        "Cobalt guarded retry limit",
        "--task-id",
        "TASK-FRESHNESS",
        "--json",
    ];
    let before = fixture.success(&arguments, None);
    let selected = before["selected"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["path"] == chunk)
        .unwrap();
    assert!(selected.get("source_changed").is_none());
    fs::write(
        fixture.project.join("specs/source.md"),
        "# Cobalt metadata boundary\nCobalt retries now stop after the fifth guarded write.\n",
    )
    .unwrap();
    let after = fixture.success(&arguments, None);
    let selected = after["selected"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["path"] == chunk)
        .unwrap();
    assert_eq!(selected["source_changed"], json!(["specs/source.md"]));
    let refreshed = fixture.success(
        &[
            "refresh",
            "--host",
            "codex",
            "--query",
            "Cobalt guarded retry limit",
            "--task-id",
            "TASK-FRESHNESS",
            "--ephemeral",
            "--json",
        ],
        None,
    );
    let text = refreshed["capsule_text"].as_str().unwrap();
    assert!(text.contains(&chunk), "{text}");
    assert!(text.contains("third guarded write"), "{text}");
    assert!(
        text.contains("cited file changed: specs/source.md; check it before relying on this"),
        "{text}"
    );
}

/// Commands a person or an agent runs by hand report what they did, as the
/// retired runtime's did: a standalone episode, a rebind of a bound task, a
/// task view with its status and authority, an empty mailbox, and a
/// lightweight task shown without a pending-checkpoint line.
#[test]
fn hand_run_commands_report_what_they_did() {
    let fixture = Fixture::new(true);
    let recorded = fixture.success(
        &[
            "record",
            "--summary",
            "Investigated the cobalt retries",
            "--outcome",
            "Retries stop after the third guarded write",
            "--file",
            "specs/source.md",
            "--json",
        ],
        None,
    );
    assert!(
        recorded["episode_id"].as_i64().is_some_and(|id| id > 0),
        "{recorded}"
    );
    let found = fixture.success(
        &["search", "cobalt retries", "--layer", "episodic", "--json"],
        None,
    );
    assert!(
        found["episodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|episode| episode["summary"] == "Investigated the cobalt retries"),
        "{found}"
    );

    fixture.start("TASK-REBIND");
    let view = fixture.success(&["get", "--task-id", "TASK-REBIND", "--json"], None);
    assert_eq!(view["status"], "active", "{view}");
    assert_eq!(view["authority"], "project-brain", "{view}");
    assert!(view.get("phase").is_some_and(Value::is_null), "{view}");
    // A task never checkpointed has no automatic summary yet, as before.
    assert!(
        view.get("auto_checkpoint").is_some_and(Value::is_null),
        "{view}"
    );
    let output = fixture.run(&["rebind", "--task-id", "TASK-REBIND"], None);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && text.starts_with("Working task already bound: TASK-REBIND -> "),
        "{text}"
    );
    let output = fixture.run(
        &["msg-read", "--task-id", "TASK-REBIND", "--for", "planner"],
        None,
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "No messages.\n");

    fixture.success(
        &[
            "--mode",
            "lightweight",
            "start",
            "--task-id",
            "LOCAL-1",
            "--goal",
            "Local note",
            "--json",
        ],
        None,
    );
    let output = fixture.run(
        &[
            "--mode",
            "lightweight",
            "retrieve",
            "local note",
            "--task-id",
            "LOCAL-1",
        ],
        None,
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success()
            && text.starts_with("working: LOCAL-1 — Local note")
            && !text.contains("warming:"),
        "{text}"
    );
}

/// Cursor's rule is re-sent with every request, and its hook answers in JSON
/// only. As in the retired runtime, a bare acknowledgement keeps the topic it
/// answers instead of emptying the rule, and a previous result that was not
/// saved reaches the agent through the rule's last-turn line.
#[test]
fn cursor_keeps_its_topic_and_hears_an_unsaved_result() {
    let fixture = Fixture::new(false);
    fixture.start("TASK-CURSOR");
    let refresh = |prompt: &str| {
        fixture.success(
            &[
                "refresh",
                "--json",
                "--host",
                "cursor",
                "--delivery-profile",
                "cursor-rule",
                "--capsule-profile",
                "full",
                "--task-id",
                "TASK-CURSOR",
                "--ephemeral",
                "--sanitize",
                "--query",
                prompt,
            ],
            None,
        )
    };
    let text = |value: &Value| value["capsule_text"].as_str().unwrap_or("").to_owned();
    let first = refresh("Where do cobalt retries stop at the metadata boundary?");
    assert!(text(&first).contains("specs/source.md"), "{first}");
    for reply in ["yes", "давай"] {
        let value = refresh(reply);
        assert!(text(&value).contains("specs/source.md"), "{reply}: {value}");
        // The topic asks with the task's goal as it stands, undistilled.
        assert_eq!(
            value["capsule"]["query"], "Verify the cobalt boundary",
            "{value}"
        );
        assert_eq!(value["capsule"]["query_source"], "task", "{value}");
    }

    let project = fixture.project.to_string_lossy().into_owned();
    let prompt = |generation: &str| {
        json!({"hook_event_name":"beforeSubmitPrompt","conversation_id":"cursor-contract",
            "generation_id":generation,"prompt":"Inspect cobalt boundaries.","workspace_roots":[project]})
    };
    let native = |operation: &str, payload: &Value| {
        fixture.success(
            &[
                operation,
                "--task-id",
                "TASK-CURSOR",
                "--host",
                "cursor",
                "--json",
            ],
            Some(payload),
        )
    };
    native("native-prompt", &prompt("g1"));
    native(
        "native-stop",
        &json!({"hook_event_name":"stop","conversation_id":"cursor-contract","generation_id":"g1",
            "status":"completed","loop_count":0,"workspace_roots":[project]}),
    );
    // The next prompt is the internal continuation the request asked for;
    // the one after it finds the result still unsaved.
    let continued = native("native-prompt", &prompt("g2"));
    assert!(continued.get("warning").is_none(), "{continued}");
    let warned = native("native-prompt", &prompt("g3"));
    assert!(warned["warning"].is_string(), "{warned}");
    let value = refresh("yes");
    assert!(
        text(&value).contains("Last turn: native memory result not saved completely"),
        "{value}"
    );
}

/// A hook without a prompt asks with the task's own words, as the retired
/// hook did: a file adds its stem, never "src" or "php", and an
/// auto-provisioned goal, the branch slug re-cased, adds nothing.
#[test]
fn a_hook_asks_with_the_tasks_words_and_file_stems() {
    let fixture = Fixture::new(false);
    fixture.success(
        &[
            "start",
            "--task-id",
            "TASK-STEMS",
            "--goal",
            "Cobalt retry boundary",
            "--file",
            "specs/source.md",
            "--json",
        ],
        None,
    );
    let value = fixture.success(&["hook-context", "--task-id", "TASK-STEMS", "--json"], None);
    assert_eq!(value["query_source"], "task", "{value}");
    assert_eq!(
        value["query"], "TASK STEMS Cobalt retry boundary source",
        "{value}"
    );

    completion_git_commit(&fixture, "Baseline");
    fs::write(fixture.project.join("specs/notes.md"), "# Notes\n").unwrap();
    fixture.success(&["turn", "--task-id", "AUTO-7", "--flush", "--json"], None);
    let task = fixture.success(&["get", "--task-id", "AUTO-7", "--json"], None);
    assert!(
        task["goal"].as_str().unwrap().contains("auto-provisioned"),
        "{task}"
    );
    let value = fixture.success(&["hook-context", "--task-id", "AUTO-7", "--json"], None);
    let query = value["query"].as_str().unwrap();
    assert!(!query.contains("provisioned"), "{query}");
    assert!(query.starts_with("AUTO 7"), "{query}");
}

/// A branch with no task yet is never searched by its name, as in the
/// retired runtime: without a change hook-context has no context (exit 3, on
/// which Cursor retires a stale rule), and with one it gives the warming
/// notice alone.
#[test]
fn hook_context_without_a_task_never_searches_the_branch_name() {
    let fixture = Fixture::new(false);
    // An installation ignores its engine and local state, as here.
    fs::write(
        fixture.project.join(".gitignore"),
        "/memory-bank/bin/\n/memory-bank/local/\n",
    )
    .unwrap();
    fixture.git(&[
        "add",
        "--",
        ".gitignore",
        "memory-bank/README.md",
        "memory-bank/INDEX.md",
    ]);
    completion_git_commit(&fixture, "Baseline");
    let output = fixture.run(
        &[
            "hook-context",
            "--task-id",
            "TASK-NONE",
            "--delivery-profile",
            "cursor-rule",
        ],
        None,
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());

    fs::write(fixture.project.join("specs/notes.md"), "# Notes\n").unwrap();
    let value = fixture.success(&["hook-context", "--task-id", "TASK-NONE", "--json"], None);
    assert_eq!(value["kind"], "warming", "{value}");
    assert_eq!(value["semantic"], json!([]), "{value}");
    assert!(value.get("query").is_none(), "{value}");
    assert_eq!(value["changed_files"], 1, "{value}");
}

/// The prepared-result path the Harness drives works from the command line
/// too: a prepared save is recorded under its scope, and a replay names that
/// scope, which the retired CLI required and the native one could not take.
#[test]
fn replay_result_takes_the_prepared_scope() {
    let fixture = Fixture::new(false);
    fixture.start("TASK-PREPARED");
    let input = fixture.root.join("prepared.json");
    fs::write(
        &input,
        serde_json::to_vec(&json!({
            "progress":"Read specs/source.md: cobalt retries stop after the third write.",
            "verified":true,
            "learnings":[]
        }))
        .unwrap(),
    )
    .unwrap();
    let input = input.to_string_lossy().into_owned();
    let common = [
        "--task-id",
        "TASK-PREPARED",
        "--result-id",
        "prepared-1",
        "--revision",
        "1",
        "--input",
        input.as_str(),
        "--reason",
        "Prepared by a test harness",
    ];
    let prepared = fixture.success(
        &[&["prepare-result"][..], &common, &["--json"]].concat(),
        None,
    );
    let scope = prepared["scope"]
        .as_str()
        .expect("a prepared scope")
        .to_owned();
    let with_scope = |operation: &'static str| {
        [
            &[operation][..],
            &common,
            &["--prepared-scope", scope.as_str(), "--json"],
        ]
        .concat()
    };
    let recorded = fixture.success(&with_scope("record-result"), None);
    assert_eq!(recorded["replayed"], false, "{recorded}");
    let replayed = fixture.success(&with_scope("replay-result"), None);
    assert_eq!(replayed["replayed"], true, "{replayed}");
    let missing = fixture.run(&[&["replay-result"][..], &common].concat(), None);
    assert!(
        !missing.status.success(),
        "a replay without its scope is refused"
    );
}

/// No handoff reaches a capsule, as in the retired runtime: another task's
/// open handoff repeated that task's record, which is admitted on its own.
#[test]
fn another_tasks_handoff_is_not_admitted_to_a_capsule() {
    let fixture = Fixture::new(false);
    fixture.success(
        &[
            "start",
            "--task-id",
            "TASK-OTHER",
            "--goal",
            "Zirconium ledger reconciliation",
            "--json",
        ],
        None,
    );
    fixture.start("TASK-WORK");
    let handoffs = fixture.project.join("project-brain/control/handoffs");
    assert!(
        fs::read_dir(&handoffs).unwrap().count() >= 2,
        "each task writes its handoff"
    );
    let value = fixture.success(
        &[
            "refresh",
            "--json",
            "--task-id",
            "TASK-WORK",
            "--host",
            "codex",
            "--gate",
            "off",
            "--ephemeral",
            "--capsule-profile",
            "full",
            "--query",
            "Zirconium ledger reconciliation",
        ],
        None,
    );
    let text = value["capsule_text"].as_str().unwrap();
    assert!(text.contains("project-brain/dynamic/tasks/"), "{text}");
    assert!(!text.contains("project-brain/control/handoffs/"), "{text}");
    // An explicit search stays broad, as the retired one was: it lists the
    // open handoff a capsule leaves out.
    let found = fixture.success(
        &["search", "Zirconium ledger reconciliation", "--json"],
        None,
    );
    assert_eq!(found["query"], "Zirconium ledger reconciliation", "{found}");
    assert!(
        found["documents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|document| document["kind"] == "brain-handoff"),
        "{found}"
    );
}

/// The operator reports list a refresh's phases in the order it runs them,
/// as the retired reports did, not in the sorted order of a JSON object.
#[test]
fn health_and_retrieval_report_list_phases_in_pipeline_order() {
    let fixture = Fixture::new(false);
    fixture.start("TASK-PHASES");
    for extra in [&["--ephemeral"][..], &[][..]] {
        let mut arguments = vec![
            "refresh",
            "--json",
            "--task-id",
            "TASK-PHASES",
            "--host",
            "codex",
            "--gate",
            "off",
            "--query",
            "cobalt metadata boundary",
        ];
        arguments.extend_from_slice(extra);
        fixture.success(&arguments, None);
    }
    for operation in ["health", "retrieval-report"] {
        let output = fixture.run(&[operation], None);
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(output.status.success(), "{text}");
        let phases = text
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("phase "))
            .map(|line| line.split(':').next().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            phases,
            ["stat", "index", "retrieval"],
            "{operation}: {text}"
        );
    }
}

/// A record whose cited file changed or went away is intact: validation lists
/// it with what retrieval does about it, as the retired runtime did, and
/// still passes.
#[test]
fn validate_lists_records_whose_citations_moved_without_failing() {
    let fixture = Fixture::new(true);
    promoted_source_finding(&fixture);
    let clean = fixture.success(&["validate", "--json"], None);
    assert_eq!(clean["valid"], true, "{clean}");
    assert_eq!(clean["stale"], json!([]), "{clean}");
    let source = fixture.project.join("specs/source.md");
    fs::write(
        &source,
        "# Cobalt metadata boundary\nCobalt retries now stop after the fifth guarded write.\n",
    )
    .unwrap();
    let changed = fixture.success(&["validate", "--json"], None);
    assert_eq!(changed["valid"], true, "{changed}");
    let states = changed["stale_states"].as_array().unwrap();
    assert!(
        states.iter().all(|row| row["state"] == "changed")
            && states.iter().any(|row| {
                row["path"]
                    .as_str()
                    .unwrap()
                    .replace('\\', "/")
                    .contains("/project-brain/dynamic/findings/")
            }),
        "{changed}"
    );
    assert_eq!(changed["stale"].as_array().unwrap().len(), states.len());
    let output = fixture.run(&["validate"], None);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{text}");
    assert!(
        text.starts_with("Project Brain validation passed.\n")
            && text.contains(": a cited source changed after the record was written; retrieval serves it marked for checking\n"),
        "{text}"
    );
    // A deleted citation also breaks the durable chunk promoted from the
    // finding, which validation reports as an error; the record is listed.
    fs::remove_file(&source).unwrap();
    let output = fixture.run(&["validate", "--json"], None);
    let missing: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        missing["stale_states"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["state"] == "source-missing"),
        "{missing}"
    );
    let output = fixture.run(&["validate"], None);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains(": a cited source no longer exists; retrieval leaves the record out\n"),
        "{text}"
    );
}

#[test]
fn legacy_memory_index_repairs_fingerprints_incrementally_and_preserves_pending_turns() {
    let fixture = Fixture::new(true);
    let chunk = promoted_source_finding(&fixture);
    public_pending_turn(&fixture, "TASK-FRESHNESS");
    let store = public_fixture_store(&fixture);
    let pending = serde_json::to_value(store.pending_turns("TASK-FRESHNESS").unwrap()).unwrap();
    assert!(!pending.as_array().unwrap().is_empty());
    store
        .conn
        .execute(
            "UPDATE document_metadata SET source_fingerprints='[]' WHERE path=?1",
            [&chunk],
        )
        .unwrap();
    store
        .conn
        .execute(
            "DELETE FROM index_state WHERE key='index-document-format'",
            [],
        )
        .unwrap();
    drop(store);
    // No source or chunk edit and no explicit full rebuild: routine indexing
    // must repair metadata produced by an older engine.
    fixture.success(&["index", "--incremental", "--json"], None);
    let store = public_fixture_store(&fixture);
    let raw: String = store
        .conn
        .query_row(
            "SELECT source_fingerprints FROM document_metadata WHERE path=?1",
            [&chunk],
            |row| row.get(0),
        )
        .unwrap();
    let fingerprints: Value = serde_json::from_str(&raw).unwrap();
    let source = fingerprints
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"] == "specs/source.md")
        .expect("incremental refresh must restore the canonical citation digest");
    assert_eq!(
        source["sha256"],
        infra_core::query::sha256(&fs::read(fixture.project.join("specs/source.md")).unwrap())
    );
    assert_eq!(
        pending,
        serde_json::to_value(store.pending_turns("TASK-FRESHNESS").unwrap()).unwrap()
    );
    let generation = store.state().unwrap()["index-generation"].clone();
    let rowid: i64 = store
        .conn
        .query_row(
            "SELECT rowid FROM documents WHERE path=?1",
            [&chunk],
            |row| row.get(0),
        )
        .unwrap();
    drop(store);
    fixture.success(&["index", "--incremental", "--json"], None);
    let store = public_fixture_store(&fixture);
    assert_eq!(generation, store.state().unwrap()["index-generation"]);
    assert_eq!(
        rowid,
        store
            .conn
            .query_row(
                "SELECT rowid FROM documents WHERE path=?1",
                [&chunk],
                |row| row.get::<_, i64>(0)
            )
            .unwrap()
    );
    assert_eq!(
        pending,
        serde_json::to_value(store.pending_turns("TASK-FRESHNESS").unwrap()).unwrap()
    );
}

fn public_pending_turn(fixture: &Fixture, task_id: &str) -> i64 {
    public_fixture_store(fixture)
        .append_turn(task_id, &["specs/source.md".into()])
        .unwrap()
}
fn public_record_progress(fixture: &Fixture, task_id: &str) -> Value {
    fixture.success(
        &[
            "record-result",
            "--task-id",
            task_id,
            "--result-id",
            "checkpoint-observation",
            "--revision",
            "1",
            "--input",
            "-",
            "--json",
        ],
        Some(&json!({
            "progress":"Manual observation that must survive the checkpoint.",
            "next_steps":["Keep the explicit review step."],
            "verified":false,
            "learnings":[]
        })),
    )
}
fn assert_authoritative_flush(fixture: &Fixture, task_id: &str, before: &Value) {
    let old_binding = public_fixture_store(fixture)
        .binding(task_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        old_binding.revision, 1,
        "the fixture must expose a stale derived binding"
    );
    assert!(before["revision"].as_u64().unwrap() > old_binding.revision);
    let report = fixture.success(&["turn", "--task-id", task_id, "--flush", "--json"], None);
    assert_eq!(report["flushed"], true);
    let after = fixture.success(&["get", "--task-id", task_id, "--json"], None);
    assert_eq!(after["revision"], before["revision"].as_u64().unwrap() + 1);
    assert_eq!(after["progress"], before["progress"]);
    assert_eq!(after["next_steps"], before["next_steps"]);
    assert!(
        after["auto_checkpoint"]
            .as_str()
            .unwrap()
            .starts_with("Auto-checkpoint:")
    );
    let store = public_fixture_store(fixture);
    assert!(store.pending_turns(task_id).unwrap().is_empty());
    assert_eq!(
        store.binding(task_id).unwrap().unwrap().revision,
        after["revision"].as_u64().unwrap()
    );
    drop(store);
    let stale = fixture.run(
        &[
            "update",
            "--task-id",
            task_id,
            "--revision",
            "1",
            "--progress",
            "An explicit stale writer must still be refused.",
            "--json",
        ],
        None,
    );
    assert!(!stale.status.success());
    let still = fixture.success(&["get", "--task-id", task_id, "--json"], None);
    assert_eq!(still["revision"], after["revision"]);
    assert_eq!(still["progress"], after["progress"]);
}

#[test]
fn result_save_then_turn_flush_uses_current_authoritative_revision() {
    let fixture = Fixture::new(false);
    fixture.start("TASK-RESULT-CHECKPOINT");
    public_record_progress(&fixture, "TASK-RESULT-CHECKPOINT");
    let before = fixture.success(
        &["get", "--task-id", "TASK-RESULT-CHECKPOINT", "--json"],
        None,
    );
    public_pending_turn(&fixture, "TASK-RESULT-CHECKPOINT");
    assert_authoritative_flush(&fixture, "TASK-RESULT-CHECKPOINT", &before);
}

#[test]
fn direct_brain_update_then_turn_flush_preserves_manual_fields() {
    let fixture = Fixture::new(false);
    let task = fixture.start("TASK-BRAIN-CHECKPOINT");
    fixture.success(
        &[
            "brain-update",
            "--record-id",
            task["task_uuid"].as_str().unwrap(),
            "--revision",
            "auto",
            "--progress",
            "The operator retained this manual narrative.",
            "--next-step",
            "Keep the explicit review step.",
            "--json",
        ],
        None,
    );
    let before = fixture.success(
        &["get", "--task-id", "TASK-BRAIN-CHECKPOINT", "--json"],
        None,
    );
    public_pending_turn(&fixture, "TASK-BRAIN-CHECKPOINT");
    assert_authoritative_flush(&fixture, "TASK-BRAIN-CHECKPOINT", &before);
}

#[test]
fn checkpoint_invalid_authoritative_task_preserves_buffer_and_other_records() {
    let fixture = Fixture::new(false);
    let task = fixture.start("TASK-INVALID-CHECKPOINT");
    let other = fixture.start("TASK-UNRELATED-CHECKPOINT");
    let other_path = fixture
        .brain()
        .find(other["task_uuid"].as_str().unwrap(), Some("task"), false)
        .unwrap()
        .path;
    let other_bytes = fs::read(&other_path).unwrap();
    let path = fixture
        .brain()
        .find(task["task_uuid"].as_str().unwrap(), Some("task"), false)
        .unwrap()
        .path;
    let invalid: &[u8] = b"---\n{}\n---\n";
    fs::write(&path, invalid).unwrap();
    let pending = public_pending_turn(&fixture, "TASK-INVALID-CHECKPOINT");
    let output = fixture.run(
        &[
            "turn",
            "--task-id",
            "TASK-INVALID-CHECKPOINT",
            "--flush",
            "--json",
        ],
        None,
    );
    assert!(!output.status.success());
    assert_eq!(fs::read(&path).unwrap().as_slice(), invalid);
    assert_eq!(fs::read(&other_path).unwrap(), other_bytes);
    let store = public_fixture_store(&fixture);
    assert!(
        store
            .pending_turns("TASK-INVALID-CHECKPOINT")
            .unwrap()
            .iter()
            .any(|turn| turn.id == pending)
    );
    assert_eq!(
        store
            .binding("TASK-INVALID-CHECKPOINT")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert_eq!(
        store
            .binding("TASK-UNRELATED-CHECKPOINT")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
}

#[test]
fn checkpoint_binding_write_failure_restores_current_brain_image_and_retains_buffer() {
    let fixture = Fixture::new(false);
    let task = fixture.start("TASK-BINDING-FAILURE");
    public_record_progress(&fixture, "TASK-BINDING-FAILURE");
    let path = fixture
        .brain()
        .find(task["task_uuid"].as_str().unwrap(), Some("task"), false)
        .unwrap()
        .path;
    let before = fs::read(&path).unwrap();
    let pending = public_pending_turn(&fixture, "TASK-BINDING-FAILURE");
    {
        let store = public_fixture_store(&fixture);
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse_fixture_binding BEFORE UPDATE ON task_bindings \
             WHEN OLD.external_id='TASK-BINDING-FAILURE' \
             BEGIN SELECT RAISE(ABORT,'fixture binding write refused'); END;",
            )
            .unwrap();
    }
    let output = fixture.run(
        &[
            "turn",
            "--task-id",
            "TASK-BINDING-FAILURE",
            "--flush",
            "--json",
        ],
        None,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Local SQLite operation failed"),
        "the test must reach the binding callback, not fail early on its stale cached revision: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "rollback must restore the authoritative revision that existed before this flush"
    );
    let store = public_fixture_store(&fixture);
    assert!(
        store
            .pending_turns("TASK-BINDING-FAILURE")
            .unwrap()
            .iter()
            .any(|turn| turn.id == pending)
    );
    assert_eq!(
        store
            .binding("TASK-BINDING-FAILURE")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
}

#[test]
fn reviewed_manual_promotion_requires_independent_approval_and_applies_once() {
    let fixture = Fixture::new(false);
    let finding = fixture.finding("PROMOTE-REVIEWED", "Cobalt reviewed boundary");
    let source_id = finding["id"].as_str().unwrap();
    fixture.verify(source_id);
    let resolved = fixture.resolve(source_id);
    assert_eq!(resolved["status"], "resolved");
    assert_eq!(resolved["authority"], "verified");
    assert_eq!(resolved["promotion"]["enabled"], false);
    assert!(
        fixture.chunks().is_empty(),
        "disabled automation published a chunk"
    );

    let source_path = fixture.brain().find(source_id, None, false).unwrap().path;
    let content = "Cobalt retries stop after the third guarded write.";
    let proposal = fixture.success(
        &[
            "promote-propose",
            "--source-id",
            source_id,
            "--title",
            "Cobalt reviewed boundary",
            "--content",
            content,
            "--json",
        ],
        None,
    );
    let promotion_id = proposal["id"].as_str().unwrap();
    assert_eq!(proposal["status"], "proposed");
    assert_eq!(proposal["review_mode"], "human");
    assert_eq!(proposal["proposer"], "local");
    assert!(proposal["reviewer"].is_null());
    assert_eq!(proposal["source_records"][0]["id"], finding["id"]);
    assert_eq!(
        proposal["source_records"][0]["revision"],
        resolved["revision"]
    );
    assert!(fixture.chunks().is_empty(), "proposing is not publication");
    let proposal_path = fixture.project.join(format!(
        "project-brain/control/promotions/{promotion_id}.json"
    ));
    let index_path = fixture.project.join("memory-bank/INDEX.md");
    let persisted = || {
        let mut image = BTreeMap::new();
        for path in [&source_path, &proposal_path, &index_path] {
            image.insert(path.clone(), fs::read(path).unwrap());
        }
        for path in fixture.chunks() {
            image.insert(path.clone(), fs::read(path).unwrap());
        }
        image
    };
    let refuse = |args: &[&str], message: &str| {
        let output = fixture.run(args, None);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{args:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let proposed_image = persisted();
    refuse(
        &[
            "promote-review",
            "--promotion-id",
            promotion_id,
            "--reviewer",
            "local",
            "--json",
        ],
        "independent",
    );
    assert_eq!(
        persisted(),
        proposed_image,
        "self-review changed governed artifacts"
    );
    refuse(
        &["promote-apply", "--promotion-id", promotion_id, "--json"],
        "approved review",
    );
    assert_eq!(
        persisted(),
        proposed_image,
        "unapproved apply changed governed artifacts"
    );

    let reviewed = fixture.success(
        &[
            "promote-review",
            "--promotion-id",
            promotion_id,
            "--reviewer",
            "independent-reviewer",
            "--json",
        ],
        None,
    );
    assert_eq!(reviewed["status"], "reviewed");
    assert_eq!(reviewed["review_mode"], "human");
    assert_eq!(reviewed["reviewer"], "independent-reviewer");
    assert_eq!(reviewed["outcome"], "approved");
    assert_eq!(
        reviewed["revision"].as_u64(),
        Some(proposal["revision"].as_u64().unwrap() + 1)
    );
    assert!(
        fixture.chunks().is_empty(),
        "approval alone published a chunk"
    );
    let day_before = chrono::Utc::now().format("%Y%m%d").to_string();
    let applied = fixture.success(
        &["promote-apply", "--promotion-id", promotion_id, "--json"],
        None,
    );
    let day_after = chrono::Utc::now().format("%Y%m%d").to_string();
    assert_eq!(applied["status"], "applied");
    assert_eq!(applied["review_mode"], "human");
    assert_eq!(applied["reviewer"], "independent-reviewer");
    assert_eq!(applied["outcome"], "approved");
    assert_eq!(applied["destination_revision"], 1);
    let memory_id = applied["destination_memory_id"].as_str().unwrap();
    let compact_source_id = source_id.replace('-', "");
    let suffix = &compact_source_id[..8];
    assert!(
        memory_id == format!("MEM-{day_before}-{suffix}")
            || memory_id == format!("MEM-{day_after}-{suffix}"),
        "reviewed promotion allocated an unexpected memory identity: {memory_id}"
    );
    let chunks = fixture.chunks();
    assert_eq!(
        chunks.len(),
        1,
        "one reviewed proposal must publish one chunk"
    );
    assert_eq!(
        chunks[0],
        fixture.project.join(format!(
            "memory-bank/chunks/{memory_id}-cobalt-reviewed-boundary.md"
        ))
    );
    let chunk = fs::read_to_string(&chunks[0]).unwrap();
    let (metadata, body) = infra_core::domain::parse_markdown_record(&chunk).unwrap();
    assert_eq!(metadata["id"], memory_id);
    assert_eq!(metadata["title"], "Cobalt reviewed boundary");
    assert_eq!(metadata["status"], "active");
    assert_eq!(metadata["type"], "domain");
    let tags = metadata["tags"].as_array().unwrap();
    for tag in ["project-brain", "promoted"] {
        assert!(tags.iter().any(|value| value == tag));
    }
    assert!(!tags.iter().any(|value| value == "auto-promoted"));
    let sources = metadata["sources"].as_array().unwrap();
    let source_relative = source_path
        .strip_prefix(&fixture.project)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    assert!(sources.iter().any(|value| value == &source_relative));
    assert!(sources.iter().any(|value| value == "specs/source.md"));
    assert_eq!(
        body.trim(),
        format!("# Cobalt reviewed boundary\n\n{content}")
    );
    assert!(
        fs::read_to_string(&index_path)
            .unwrap()
            .contains(&format!("| {memory_id} |"))
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&proposal_path).unwrap()).unwrap(),
        applied
    );

    let applied_image = persisted();
    refuse(
        &["promote-apply", "--promotion-id", promotion_id, "--json"],
        "approved review",
    );
    assert_eq!(
        persisted(),
        applied_image,
        "repeated apply duplicated or changed the publication"
    );
    refuse(
        &[
            "promote-review",
            "--promotion-id",
            promotion_id,
            "--reviewer",
            "independent-reviewer",
            "--json",
        ],
        "Only proposed",
    );
    assert_eq!(
        persisted(),
        applied_image,
        "repeated review changed the applied publication"
    );
    assert_eq!(fixture.chunks().len(), 1);
}

fn completion_task_image(fixture: &Fixture, id: &str) -> Value {
    let store = public_fixture_store(fixture);
    json!({
        "working":store.working(id).unwrap(),
        "binding":store.binding(id).unwrap(),
        "pending":store.pending_turns(id).unwrap()
    })
}

fn completion_episodes(fixture: &Fixture) -> Vec<Value> {
    let store = public_fixture_store(fixture);
    let mut statement = store.conn.prepare(
        "SELECT rowid,summary,outcome,files,verification,sources,created_at FROM episodes ORDER BY rowid",
    ).unwrap();
    statement
        .query_map([], |row| {
            Ok(json!({
                "id":row.get::<_,i64>(0)?,
                "summary":row.get::<_,String>(1)?,
                "outcome":row.get::<_,String>(2)?,
                "files":row.get::<_,String>(3)?,
                "verification":row.get::<_,String>(4)?,
                "sources":row.get::<_,String>(5)?,
                "created_at":row.get::<_,String>(6)?
            }))
        })
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

fn completion_source_image(fixture: &Fixture) -> BTreeMap<String, Vec<u8>> {
    [
        "AGENTS.md",
        "specs/source.md",
        "specs/completion-feature.md",
        "project-brain/config/runtime.json",
    ]
    .into_iter()
    .filter_map(|name| {
        let path = fixture.project.join(name);
        path.exists()
            .then(|| (name.into(), fs::read(path).unwrap()))
    })
    .collect()
}

fn completion_git_commit(fixture: &Fixture, message: &str) {
    fixture.git(&[
        "-c",
        "user.name=Native completion fixture",
        "-c",
        "user.email=native-completion@invalid.example",
        "commit",
        "-q",
        "-m",
        message,
    ]);
}

fn completion_git_main(fixture: &Fixture) {
    fixture.git(&["symbolic-ref", "HEAD", "refs/heads/main"]);
    completion_git_commit(fixture, "Initialize bounded completion proof");
}

fn completion_spawn(fixture: &Fixture, arguments: &[&str]) -> std::process::Child {
    let mut command = Command::new(&fixture.binary);
    fixture.isolate(&mut command);
    command
        .arg("memory")
        .arg("--root")
        .arg(&fixture.project)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().unwrap()
}

fn completion_collect(mut children: [std::process::Child; 2]) -> [Output; 2] {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let mut complete = true;
        for child in &mut children {
            complete &= child.try_wait().unwrap().is_some();
        }
        if complete {
            break;
        }
        if std::time::Instant::now() >= deadline {
            for child in &mut children {
                if child.try_wait().unwrap().is_none() {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
            panic!("Public completion children exceeded the private fixture deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    children.map(|child| child.wait_with_output().unwrap())
}

#[test]
fn turn_reports_merge_candidate_without_completing_authority_or_losing_pending() {
    let fixture = Fixture::new(false);
    let config_path = fixture.project.join("project-brain/config/runtime.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["automatic_completion"] = true.into();
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    fixture.git(&["add", "--", "project-brain/config/runtime.json"]);
    completion_git_main(&fixture);
    let id = "feature/completion-authority";
    fixture.git(&["checkout", "-q", "-b", id]);
    let task = fixture.start(id);
    let uuid = task["task_uuid"].as_str().unwrap();
    fs::write(
        fixture.project.join("specs/completion-feature.md"),
        "# Completion evidence\nThe feature commit is observable before its no-ff merge.\n",
    )
    .unwrap();
    fixture.git(&["add", "--", "specs/completion-feature.md"]);
    completion_git_commit(&fixture, "Add bounded feature proof");

    let unmerged = fixture.success(
        &[
            "turn",
            "--task-id",
            "TASK-MERGE-OBSERVER",
            "--flush-after",
            "1000",
            "--json",
        ],
        None,
    );
    assert_eq!(unmerged["completion_candidates"], json!([]));

    fixture.git(&["checkout", "-q", "main"]);
    fixture.git(&[
        "-c",
        "user.name=Native completion fixture",
        "-c",
        "user.email=native-completion@invalid.example",
        "merge",
        "-q",
        "--no-ff",
        id,
        "-m",
        "Merge bounded feature proof",
    ]);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        // An unrelated native ref must not invalidate the valid task's evidence.
        let reference =
            std::ffi::OsString::from_vec(b"refs/heads/unrelated-non-utf8-\xff".to_vec());
        let mut command = Command::new("git");
        fixture.isolate(&mut command);
        let output = command
            .arg("update-ref")
            .arg(reference)
            .arg("HEAD")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Git refused the private raw-byte ref fixture: {:?}",
            output.stderr
        );
    }
    public_pending_turn(&fixture, id);
    let authority = fixture.brain().get_task(uuid).unwrap();
    assert_eq!(authority["status"], "active");
    let local = completion_task_image(&fixture, id);
    let sources = completion_source_image(&fixture);
    let episodes = completion_episodes(&fixture);
    assert!(episodes.is_empty());

    let report = fixture.success(
        &[
            "turn",
            "--task-id",
            "TASK-MERGE-OBSERVER",
            "--flush-after",
            "1000",
            "--json",
        ],
        None,
    );
    let candidates = report["completion_candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1, "{report}");
    assert_eq!(candidates[0]["task_id"], id);
    assert_eq!(candidates[0]["task_uuid"], uuid);
    assert_eq!(candidates[0]["revision"], authority["revision"]);
    assert_eq!(candidates[0]["target"], "refs/heads/main");
    assert_eq!(fixture.brain().get_task(uuid).unwrap(), authority);
    assert_eq!(completion_task_image(&fixture, id), local);
    assert_eq!(completion_episodes(&fixture), episodes);
    assert!(completion_source_image(&fixture) == sources);
    assert!(fixture.chunks().is_empty());

    // The target of origin's HEAD names the default branch before main does.
    fixture.git(&["update-ref", "refs/remotes/origin/trunk", "main"]);
    fixture.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/trunk",
    ]);
    let report = fixture.success(
        &[
            "turn",
            "--task-id",
            "TASK-MERGE-OBSERVER",
            "--flush-after",
            "1000",
            "--json",
        ],
        None,
    );
    let candidates = report["completion_candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1, "{report}");
    assert_eq!(candidates[0]["target"], "refs/remotes/origin/trunk");
}

#[test]
fn governed_completion_requires_an_explicit_numeric_revision_and_preserves_refused_state() {
    let fixture = Fixture::new(false);
    let id = "TASK-EXPLICIT-COMPLETION";
    let task = fixture.start(id);
    let uuid = task["task_uuid"].as_str().unwrap();
    public_pending_turn(&fixture, id);
    let before = fixture.brain().get_task(uuid).unwrap();
    let local = completion_task_image(&fixture, id);
    let sources = completion_source_image(&fixture);
    let episodes = completion_episodes(&fixture);
    for revision in [None, Some("auto")] {
        let mut arguments = vec![
            "complete",
            "--task-id",
            id,
            "--outcome",
            "Missing pinned authority must not close this task.",
            "--json",
        ];
        if let Some(revision) = revision {
            arguments.extend(["--revision", revision]);
        }
        let output = fixture.run(&arguments, None);
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("current numeric --revision"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fixture.brain().get_task(uuid).unwrap(), before);
        assert_eq!(completion_task_image(&fixture, id), local);
        assert_eq!(completion_episodes(&fixture), episodes);
        assert!(completion_source_image(&fixture) == sources);
    }
    let stale_revision = (before["revision"].as_u64().unwrap() + 1).to_string();
    let stale = fixture.run(
        &[
            "complete",
            "--task-id",
            id,
            "--revision",
            &stale_revision,
            "--outcome",
            "A stale explicit number cannot close the task.",
            "--json",
        ],
        None,
    );
    assert_eq!(stale.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&stale.stderr).contains("Stale record revision"));
    assert_eq!(fixture.brain().get_task(uuid).unwrap(), before);
    assert_eq!(completion_task_image(&fixture, id), local);
    assert_eq!(completion_episodes(&fixture), episodes);
    assert!(completion_source_image(&fixture) == sources);
    assert!(fixture.chunks().is_empty());
}

#[test]
fn concurrent_completion_cas_creates_one_episode_and_preserves_pending_and_other_authority() {
    let fixture = Fixture::new(false);
    let id = "TASK-COMPLETE-RACE";
    let task = fixture.start(id);
    let uuid = task["task_uuid"].as_str().unwrap();
    fixture.success(
        &[
            "update",
            "--task-id",
            id,
            "--revision",
            "1",
            "--progress",
            "Manual progress before explicit completion.",
            "--next-step",
            "Retain the reviewer follow-up.",
            "--file",
            "specs/source.md",
            "--source",
            "specs/source.md",
            "--json",
        ],
        None,
    );
    let other_id = "TASK-COMPLETION-UNRELATED";
    let other = fixture.start(other_id);
    public_pending_turn(&fixture, id);
    public_pending_turn(&fixture, other_id);
    let brain = fixture.brain();
    let before = brain.get_task(uuid).unwrap();
    let other_before = brain
        .get_task(other["task_uuid"].as_str().unwrap())
        .unwrap();
    let local = completion_task_image(&fixture, id);
    let other_local = completion_task_image(&fixture, other_id);
    let sources = completion_source_image(&fixture);
    assert!(completion_episodes(&fixture).is_empty());
    let revision = before["revision"].as_u64().unwrap().to_string();
    let first_outcome = "First independent completion was verified.";
    let second_outcome = "Second independent completion was verified.";
    // Both separate engines exist before authority is released. Scheduling may
    // still let the second read an absent binding, which is canonical refusal.
    let guard = brain.lock().unwrap();
    let children = [
        completion_spawn(
            &fixture,
            &[
                "complete",
                "--task-id",
                id,
                "--revision",
                &revision,
                "--outcome",
                first_outcome,
                "--verification",
                "Native fixture checks passed",
                "--json",
            ],
        ),
        completion_spawn(
            &fixture,
            &[
                "complete",
                "--task-id",
                id,
                "--revision",
                &revision,
                "--outcome",
                second_outcome,
                "--verification",
                "Native fixture checks passed",
                "--json",
            ],
        ),
    ];
    drop(guard);
    let outputs = completion_collect(children);
    let mut codes = outputs
        .iter()
        .map(|output| output.status.code().unwrap())
        .collect::<Vec<_>>();
    codes.sort_unstable();
    assert_eq!(
        codes,
        vec![0, 1],
        "stderr first: {}; second: {}",
        String::from_utf8_lossy(&outputs[0].stderr),
        String::from_utf8_lossy(&outputs[1].stderr)
    );
    let winner = outputs
        .iter()
        .position(|output| output.status.success())
        .unwrap();
    let loser = &outputs[1 - winner];
    let refusal = String::from_utf8_lossy(&loser.stderr);
    assert!(
        refusal.contains("Stale record revision") || refusal.contains("Working task not found"),
        "{refusal}"
    );
    let result: Value = serde_json::from_slice(&outputs[winner].stdout).unwrap();
    let outcome = [first_outcome, second_outcome][winner];
    let after = brain.get_task(uuid).unwrap();
    assert_eq!(after["status"], "completed");
    assert_eq!(after["revision"], before["revision"].as_u64().unwrap() + 1);
    assert_eq!(result["revision"], after["revision"]);
    assert_eq!(result["task_uuid"], uuid);
    let transitions = after["transitions"].as_array().unwrap();
    assert_eq!(
        transitions.len(),
        before["transitions"].as_array().unwrap().len() + 1
    );
    assert_eq!(transitions.last().unwrap()["from"], "active");
    assert_eq!(transitions.last().unwrap()["to"], "completed");
    for field in [
        "goal",
        "next_steps",
        "files",
        "sources",
        "owner",
        "authorized_owners",
        "privacy",
    ] {
        assert_eq!(after[field], before[field], "{field}");
    }
    let episodes = completion_episodes(&fixture);
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0]["id"], result["episode_id"]);
    assert_eq!(episodes[0]["outcome"], outcome);
    assert_eq!(
        serde_json::from_str::<Value>(episodes[0]["files"].as_str().unwrap()).unwrap(),
        json!(["specs/source.md"])
    );
    assert_eq!(
        serde_json::from_str::<Value>(episodes[0]["verification"].as_str().unwrap()).unwrap(),
        json!(["Native fixture checks passed"])
    );
    assert_eq!(
        serde_json::from_str::<Value>(episodes[0]["sources"].as_str().unwrap()).unwrap(),
        json!(["specs/source.md"])
    );
    let after_local = completion_task_image(&fixture, id);
    assert!(after_local["working"].is_null());
    assert!(after_local["binding"].is_null());
    assert_eq!(after_local["pending"], local["pending"]);
    assert_eq!(
        brain
            .get_task(other["task_uuid"].as_str().unwrap())
            .unwrap(),
        other_before
    );
    assert_eq!(completion_task_image(&fixture, other_id), other_local);
    assert!(completion_source_image(&fixture) == sources);
    assert!(fixture.chunks().is_empty());

    // The Brain CAS itself rejects the old revision even if a public loser
    // saw the already-removed local alias before reaching its CAS boundary.
    let stale = brain
        .close_task(
            uuid,
            "A duplicate closure must not land.",
            &[],
            Some(before["revision"].as_u64().unwrap()),
        )
        .unwrap_err();
    assert_eq!(stale.kind, infra_core::ErrorKind::Conflict);
    assert_eq!(brain.get_task(uuid).unwrap(), after);
    assert_eq!(completion_episodes(&fixture), episodes);
    assert_eq!(
        completion_task_image(&fixture, id)["pending"],
        local["pending"]
    );
}

#[test]
fn completion_candidate_ignores_a_never_diverged_branch_even_after_checkout_to_main() {
    let fixture = Fixture::new(false);
    completion_git_main(&fixture);
    let id = "feature/never-diverged-completion";
    fixture.git(&["branch", id]);
    let task = fixture.start(id);
    let uuid = task["task_uuid"].as_str().unwrap();
    public_pending_turn(&fixture, id);
    let authority = fixture.brain().get_task(uuid).unwrap();
    let local = completion_task_image(&fixture, id);
    let sources = completion_source_image(&fixture);
    let episodes = completion_episodes(&fixture);
    let report = fixture.success(
        &[
            "turn",
            "--task-id",
            "TASK-FRESH-OBSERVER",
            "--flush-after",
            "1000",
            "--json",
        ],
        None,
    );
    assert_eq!(
        report["completion_candidates"],
        json!([]),
        "An equal-tip feature/default ref proves no merge: {report}"
    );
    assert_eq!(fixture.brain().get_task(uuid).unwrap(), authority);
    assert_eq!(completion_task_image(&fixture, id), local);
    assert_eq!(completion_episodes(&fixture), episodes);
    assert!(completion_source_image(&fixture) == sources);
}

#[test]
fn the_message_journal_keeps_the_retired_line_format() {
    let fixture = Fixture::new(false);
    let task = fixture.start("TASK-MESSAGES");
    let body = "Grüße\u{2028}aus Köln";
    let sent = fixture.success(
        &[
            "msg-send",
            "--task-id",
            "TASK-MESSAGES",
            "--from",
            "planner",
            "--to",
            "implementer",
            "--type",
            "handoff",
            "--body",
            body,
            "--json",
        ],
        None,
    );
    let journal = fs::read_to_string(fixture.project.join(format!(
        "project-brain/control/messages/{}.jsonl",
        task["task_uuid"].as_str().unwrap()
    )))
    .unwrap();
    // `json.dumps(record, ensure_ascii=False, sort_keys=True)`, with the line
    // separators escaped: a migrated task's journal keeps one format.
    let expected = format!(
        "{{\"body\": \"Grüße\\u2028aus Köln\", \"created_at\": \"{}\", \"from_actor\": \"planner\", \"id\": \"{}\", \"refs\": [], \"schema_version\": 1, \"seq\": 1, \"task_id\": \"{}\", \"task_revision\": {}, \"to_actor\": \"implementer\", \"type\": \"handoff\"}}\n",
        sent["created_at"].as_str().unwrap(),
        sent["id"].as_str().unwrap(),
        task["task_uuid"].as_str().unwrap(),
        sent["task_revision"],
    );
    assert_eq!(journal, expected);
    let read = fixture.success(
        &[
            "msg-read",
            "--task-id",
            "TASK-MESSAGES",
            "--for",
            "implementer",
            "--json",
        ],
        None,
    );
    assert_eq!(read[0]["body"], body, "{read}");
}

/// An explicit request asks with its plain words, each once and at most 32,
/// as the retired runtime asked: a long request's tail beyond them is not
/// searched, and the capsule records the words it searched with.
#[test]
fn an_explicit_request_asks_with_its_first_thirty_two_words() {
    let fixture = Fixture::new(false);
    fixture.start("TASK-WORDS");
    fixture.success(&["index", "--json"], None);
    let asked = fixture.success(
        &[
            "retrieve",
            "--task-id",
            "TASK-WORDS",
            "--json",
            "Cobalt retries?",
        ],
        None,
    );
    assert_eq!(asked["query"], "Cobalt retries", "{asked}");
    assert!(
        asked["selected"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["path"] == "specs/source.md"),
        "{asked}"
    );
    let filler = (0..32)
        .map(|index| format!("zzfill{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let long = fixture.success(
        &[
            "retrieve",
            "--task-id",
            "TASK-WORDS",
            "--json",
            &format!("{filler} {filler} cobalt retries"),
        ],
        None,
    );
    assert_eq!(long["query"], filler, "{long}");
    assert!(
        !long["selected"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["path"] == "specs/source.md"),
        "{long}"
    );
}

/// A task's declared phase reaches its capsule, as it did before: the
/// working state carries the phase and the record's times, and the render
/// shows the phase under the task line.
#[test]
fn a_declared_phase_reaches_the_capsule() {
    let fixture = Fixture::new(false);
    fixture.start("TASK-PHASE");
    fixture.success(
        &[
            "update",
            "--task-id",
            "TASK-PHASE",
            "--phase",
            "implementation",
            "--json",
        ],
        None,
    );
    fixture.success(&["index", "--json"], None);
    let capsule = fixture.success(
        &[
            "retrieve",
            "--task-id",
            "TASK-PHASE",
            "--json",
            "cobalt retries",
        ],
        None,
    );
    let working = &capsule["working"];
    assert_eq!(working["phase"], "implementation", "{capsule}");
    assert!(
        working["created_at"].is_string() && working["updated_at"].is_string(),
        "{capsule}"
    );
    let output = fixture.run(
        &["retrieve", "--task-id", "TASK-PHASE", "cobalt retries"],
        None,
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let lines = text.lines().take(2).collect::<Vec<_>>();
    assert_eq!(
        lines,
        [
            "working: TASK-PHASE — Verify the cobalt boundary",
            "phase: implementation"
        ],
        "{text}"
    );
}

/// A task's files become file edges as the retired index made them: each is
/// screened (no directory, build output, lockfile or runtime state) and the
/// survivors are checked against Git's index at once, a staged file counting.
/// A directory listed by a task used to link every tracked file under it.
#[test]
fn a_tasks_file_edges_are_screened_and_checked_against_the_index() {
    let fixture = Fixture::new(false);
    for (path, text) in [
        ("src/Tracked.php", "<?php\n"),
        ("src/Staged.php", "<?php\n"),
        ("src/Untracked.php", "<?php\n"),
        ("src/Nested/Inner.php", "<?php\n"),
        ("vendor/acme/Lib.php", "<?php\n"),
        ("composer.lock", "{}\n"),
    ] {
        let file = fixture.project.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }
    fixture.git(&[
        "add",
        "--",
        "src/Tracked.php",
        "src/Staged.php",
        "src/Nested/Inner.php",
        "vendor/acme/Lib.php",
        "composer.lock",
        "memory-bank/README.md",
    ]);
    let task = fixture.start("TASK-EDGES");
    let mut arguments = vec!["update", "--task-id", "TASK-EDGES", "--json"];
    for file in [
        "src/",
        "./src/Tracked.php",
        "src/Staged.php",
        "src/Untracked.php",
        "vendor/acme/Lib.php",
        "composer.lock",
        "memory-bank/README.md",
    ] {
        arguments.extend(["--file", file]);
    }
    fixture.success(&arguments, None);
    fixture.success(&["index", "--json"], None);
    let record = format!(
        "project-brain/dynamic/tasks/{}.md",
        task["task_uuid"].as_str().unwrap()
    );
    let linked = || {
        let store = public_fixture_store(&fixture);
        let mut statement = store
            .conn
            .prepare("SELECT ref_path FROM document_links WHERE path=?1 AND ref_kind='file' ORDER BY ref_path")
            .unwrap();
        statement
            .query_map([&record], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(linked(), ["src/Staged.php", "src/Tracked.php"]);
    // A file Git starts tracking gains its edge at the next refresh, though
    // no record changed: the incremental index compares the links too.
    fixture.git(&["add", "--", "src/Untracked.php"]);
    fixture.success(
        &[
            "refresh",
            "--json",
            "--task-id",
            "TASK-EDGES",
            "--query",
            "cobalt retries",
        ],
        None,
    );
    assert_eq!(
        linked(),
        ["src/Staged.php", "src/Tracked.php", "src/Untracked.php"]
    );
}

/// The refresh report names every layer, an empty one as zero, and a capsule
/// that searched nothing has the retired shape: no categories, its token
/// estimates by layer.
#[test]
fn an_unsearched_capsule_and_the_layer_counts_keep_the_retired_shape() {
    let fixture = Fixture::new(false);
    fixture.start("TASK-SHAPE");
    let report = fixture.success(
        &[
            "refresh",
            "--json",
            "--task-id",
            "TASK-SHAPE",
            "--capsule-profile",
            "navigation",
            "--query",
            "cobalt retries",
        ],
        None,
    );
    assert_eq!(report["counts"]["episodic"], 0, "{report}");
    let capsule = &report["capsule"];
    assert_eq!(capsule["categories"], json!({}), "{report}");
    assert_eq!(
        capsule["token_estimates"],
        json!({"procedural":0,"semantic":0,"episodic":0,"local_episodes":0,"total":0}),
        "{report}"
    );
}

/// A refresh that built no capsule says there was nothing to observe, in its
/// answer and its health record, as the retired refresh did: `health` then
/// counts it unavailable, not as a record whose diagnosis is missing.
#[test]
fn a_refresh_without_a_capsule_records_nothing_to_observe() {
    let fixture = Fixture::new(false);
    let report = fixture.success(&["refresh", "--json"], None);
    assert!(report["capsule"].is_null(), "{report}");
    assert_eq!(
        report["funnel_diagnosis"]["observation_state"], "unavailable",
        "{report}"
    );
    assert_eq!(
        report["ordinary_receipt"],
        json!({"schema_version":1,"observation_state":"unavailable","plans_present":null,"returned_status":"unknown"}),
        "{report}"
    );
    let health = fixture.success(&["health", "--json"], None);
    assert_eq!(
        health["funnel_diagnosis"]["availability"],
        json!({"observed":0,"unavailable":1,"missing":0,"rejected":0}),
        "{health}"
    );
}

/// A refused result says what to correct, as the retired refusals said it:
/// the fields a learning lacks or must not have (`verified` belongs to the
/// result), the attestation, the kind of data it must not carry, two learnings
/// to merge, and text that is no JSON at all.
#[test]
fn a_refused_result_says_what_to_correct() {
    let fixture = Fixture::new(false);
    fixture.start("TASK-REFUSED");
    let refusal = |input: &[u8]| {
        let output = fixture.execute(
            "memory",
            &[
                "record-result",
                "--task-id",
                "TASK-REFUSED",
                "--result-id",
                "refused-1",
                "--revision",
                "auto",
                "--input",
                "-",
                "--json",
            ],
            input,
        );
        assert!(!output.status.success());
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    let learning = json!({"type":"finding","title":"Cobalt retries","consequence":"Stop after three.","sources":["specs/source.md"]});
    for (result, said) in [
        (
            json!({"progress":"p","learnings":[{"type":"finding","title":"t","consequence":"c","sources":["specs/source.md"],"verified":true}]}),
            "Each learning needs type, title, consequence and sources; unexpected: verified (verified is a top-level boolean, never a learning field)",
        ),
        (
            json!({"progress":"p","learnings":[{"title":"t","sources":["specs/source.md"]}]}),
            "Each learning needs type, title, consequence and sources; missing: consequence, type",
        ),
        (
            json!({"progress":"p","learnings":[learning]}),
            "Attest that each learning was checked against its sources with verified=true",
        ),
        (
            json!({"progress":"The key is AKIAABCDEFGHIJKLMNOP","verified":true,"learnings":[]}),
            "progress contains a possible AWS access key; replace it with a sanitized summary - nothing was stored",
        ),
        (
            json!({"progress":"p","verified":true,"learnings":[learning, {"type":"finding","title":"cobalt  RETRIES","consequence":"Another.","sources":["specs/source.md"]}]}),
            "Conflicting learning identities in one result: two findings titled 'cobalt  RETRIES' cite the same sources; merge them",
        ),
    ] {
        let stderr = refusal(&serde_json::to_vec(&result).unwrap());
        assert!(stderr.contains(said), "{said:?} in {stderr}");
    }
    let stderr = refusal(b"{not json");
    assert!(
        stderr.contains("The result must be a JSON object"),
        "{stderr}"
    );
}
