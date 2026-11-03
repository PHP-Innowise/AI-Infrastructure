//! Native compatibility checks use only disposable, explicitly seeded SQLite files.
//! A separate Store connection attaches the legacy file without initializing it.
#[path = "support/executable.rs"]
mod executable;
use infra_core::{Roots, RuntimeContext, Store};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

const LEGACY_WORKING: &str = r#"
CREATE TABLE legacy.working_tasks(
    task_id TEXT PRIMARY KEY,
    goal TEXT NOT NULL,
    progress TEXT NOT NULL,
    next_steps TEXT NOT NULL,
    files TEXT NOT NULL,
    sources TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
INSERT INTO legacy.working_tasks VALUES(
    'TASK-LEGACY', 'Predates checkpoints.', 'Manual narrative.',
    '["Keep review gate."]', '["src/A.php"]', '["specs/legacy.md"]',
    '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'
);
CREATE TABLE legacy.turn_deltas(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    files TEXT NOT NULL
);
INSERT INTO legacy.turn_deltas VALUES
    (41, 'TASK-LEGACY', '2026-01-01T10:00:00+00:00', '["src/B.php"]'),
    (42, 'TASK-OTHER', '2026-01-01T10:01:00+00:00', '["src/Other.php"]'),
    (43, 'TASK-LEGACY', '2026-01-01T10:02:00+00:00', '["src/C.php"]');
"#;

const LEGACY_DOCUMENTS: &str = r#"
CREATE VIRTUAL TABLE legacy.documents USING fts5(
    path UNINDEXED, kind UNINDEXED, title, content, tokenize='unicode61'
);
INSERT INTO legacy.documents VALUES(
    'ghost.md', 'overview', 'Ghost', 'ghostneedle'
);
CREATE TABLE legacy.document_source_state(
    path TEXT PRIMARY KEY, mtime_ns INTEGER NOT NULL, size INTEGER NOT NULL
);
INSERT INTO legacy.document_source_state VALUES('ghost.md', 17, 11);
"#;

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    database: PathBuf,
    helper_database: PathBuf,
    binary: PathBuf,
    spool: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra-memory-legacy-{}",
            infra_core::domain::uuid4()
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let project = root.join("project with spaces");
        let database = root.join("legacy state").join("context.db");
        let helper_database = root.join("inspection.db");
        let spool = root.join("spool");
        fs::create_dir_all(project.join("project-brain/config")).unwrap();
        fs::create_dir_all(project.join("memory-bank/bin")).unwrap();
        fs::create_dir_all(database.parent().unwrap()).unwrap();
        fs::create_dir(&spool).unwrap();
        fs::write(
            project.join("project-brain/config/runtime.json"),
            br#"{"mode":"lightweight"}"#,
        )
        .unwrap();
        let binary = project.join(if cfg!(windows) {
            "memory-bank/bin/infra-engine.exe"
        } else {
            "memory-bank/bin/infra-engine"
        });
        executable::copy(env!("CARGO_BIN_EXE_infra-engine"), &binary);
        let git = Command::new("git")
            .args(["-c", "init.templateDir=", "init", "-q"])
            .arg(&project)
            .env_remove("GIT_DIR")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_CONFIG")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_CONFIG_COUNT")
            .output()
            .unwrap();
        assert!(
            git.status.success(),
            "{}",
            String::from_utf8_lossy(&git.stderr)
        );
        Self {
            root,
            project,
            database,
            helper_database,
            binary,
            spool,
        }
    }

    fn attached<T>(&self, operation: impl FnOnce(&Store) -> T) -> T {
        let helper = Store::open(&self.helper_database, Default::default()).unwrap();
        helper
            .conn
            .execute(
                "ATTACH DATABASE ?1 AS legacy",
                [self.database.to_str().unwrap()],
            )
            .unwrap();
        let result = operation(&helper);
        // No helper connection may keep the target open during native calls.
        helper.conn.execute_batch("DETACH DATABASE legacy").unwrap();
        result
    }

    fn seed(&self, sql: &str) {
        self.attached(|helper| helper.conn.execute_batch(sql).unwrap());
    }

    fn run(&self, arguments: &[&str]) -> Output {
        Command::new(&self.binary)
            .arg("memory")
            .arg("--root")
            .arg(&self.project)
            .arg("--db")
            .arg(&self.database)
            .args(["--mode", "lightweight"])
            .args(arguments)
            .current_dir(&self.project)
            .env("TMPDIR", &self.spool)
            .env("CODEX_HOME", self.root.join("codex-home"))
            .env("HOME", self.root.join("home"))
            .env_remove("CLAUDE_CODE_TMPDIR")
            .env_remove("INFRA_SOURCE_ROOT")
            .env_remove("ACCELERATOR_HOME")
            .env_remove("ACCELERATOR_STATE_DIR")
            .env_remove("ACCELERATOR_PROJECT_DIR")
            .env_remove("PROJECT_BRAIN_OWNER")
            .env_remove("PROJECT_BRAIN_MODE")
            .env_remove("CONTEXT_QUEUE_DRAIN")
            .env_remove("CONTEXT_HOOK_BUDGET")
            .env_remove("GIT_DIR")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_CONFIG")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_CONFIG_COUNT")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn success(&self, arguments: &[&str]) -> Value {
        let output = self.run(arguments);
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
            "legacy-compat-test",
            "lightweight",
        )
        .unwrap()
    }

    fn columns(&self, table: &str) -> Vec<String> {
        assert!(["working_tasks", "documents"].contains(&table));
        self.attached(|helper| {
            let mut statement = helper
                .conn
                .prepare(&format!("PRAGMA legacy.table_info({table})"))
                .unwrap();
            statement
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        })
    }

    fn deltas(&self) -> Vec<Vec<String>> {
        self.attached(|helper| {
            text_rows(
                helper,
                "SELECT CAST(id AS TEXT),task_id,created_at,files \
                 FROM legacy.turn_deltas ORDER BY id",
                4,
            )
        })
    }

    fn schema_snapshot(&self) -> Vec<Vec<String>> {
        self.attached(|helper| {
            text_rows(
                helper,
                "SELECT type,name,coalesce(sql,'') FROM legacy.sqlite_master \
                 ORDER BY type,name",
                3,
            )
        })
    }

    fn unfinished_snapshot(&self) -> Value {
        self.attached(|helper| {
            json!({
                "working": text_rows(helper,
                    "SELECT task_id,goal,progress,next_steps,files,sources,created_at,updated_at \
                     FROM legacy.working_tasks ORDER BY task_id", 8),
                "turns": text_rows(helper,
                    "SELECT CAST(id AS TEXT),task_id,created_at,files \
                     FROM legacy.turn_deltas ORDER BY id", 4),
                "sequence": text_rows(helper,
                    "SELECT name,CAST(seq AS TEXT) FROM legacy.sqlite_sequence ORDER BY name", 2),
            })
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text_rows(helper: &Store, sql: &str, width: usize) -> Vec<Vec<String>> {
    let mut statement = helper.conn.prepare(sql).unwrap();
    statement
        .query_map([], |row| {
            (0..width)
                .map(|column| row.get::<_, String>(column))
                .collect::<std::result::Result<Vec<_>, _>>()
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn legacy_working_tasks_keep_manual_state_and_buffered_turns_through_migration_and_flush() {
    let fixture = Fixture::new();
    fixture.seed(LEGACY_WORKING);
    assert!(
        !fixture
            .columns("working_tasks")
            .iter()
            .any(|name| name == "auto_checkpoint")
    );
    let unfinished = fixture.unfinished_snapshot();
    let deltas = fixture.deltas();

    let task = fixture.success(&["get", "--task-id", "TASK-LEGACY", "--json"]);
    assert_eq!(task["goal"], "Predates checkpoints.");
    assert_eq!(task["progress"], "Manual narrative.");
    assert_eq!(task["auto_checkpoint"], "");
    assert_eq!(task["next_steps"], json!(["Keep review gate."]));
    assert_eq!(task["files"], json!(["src/A.php"]));
    assert_eq!(task["sources"], json!(["specs/legacy.md"]));
    assert_eq!(task["created_at"], "2026-01-01T00:00:00+00:00");
    assert_eq!(task["updated_at"], "2026-01-01T00:00:00+00:00");
    assert_eq!(fixture.unfinished_snapshot(), unfinished);
    assert_eq!(fixture.deltas(), deltas);

    {
        let mut store = Store::open(&fixture.database, Default::default()).unwrap();
        let flushed = store
            .flush_turns(&fixture.context(), "TASK-LEGACY", 20)
            .unwrap()
            .unwrap();
        assert_eq!(flushed["files_omitted"], 0);
        assert_eq!(flushed["provisioned"], false);
        let task = store.working("TASK-LEGACY").unwrap().unwrap();
        assert_eq!(task.progress, "Manual narrative.");
        assert_eq!(task.next_steps, ["Keep review gate."]);
        assert_eq!(task.sources, ["specs/legacy.md"]);
        assert_eq!(task.files, ["src/A.php", "src/B.php", "src/C.php"]);
        assert_eq!(task.created_at, "2026-01-01T00:00:00+00:00");
        assert_eq!(
            task.auto_checkpoint,
            "Auto-checkpoint: 2 turn(s) since 2026-01-01 10:00 UTC, 2 file(s) touched."
        );
        assert!(store.pending_turns("TASK-LEGACY").unwrap().is_empty());
        let remaining = store.pending_turns("TASK-OTHER").unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, 42);
    }
    assert_eq!(fixture.deltas(), vec![deltas[1].clone()]);
    {
        let mut store = Store::open(&fixture.database, Default::default()).unwrap();
        assert_eq!(
            store
                .append_turn("TASK-OTHER", &["src/D.php".into()])
                .unwrap(),
            44
        );
    }
    assert_eq!(fixture.deltas()[1][0], "44");
}

#[test]
fn legacy_episode_migration_preserves_rowids_and_indexes_all_evidence_fields() {
    // FTS5 rejects tabs/newlines inside these module arguments; exercise
    // multiple ASCII spaces without assuming all SQL whitespace is accepted.
    for (label, tokenizer, separator) in [
        ("old tokenizer", "unicode61", " "),
        (
            "current tokenizer with two spaces",
            "porter unicode61",
            "  ",
        ),
        (
            "current tokenizer with four spaces",
            "porter unicode61",
            "    ",
        ),
    ] {
        let fixture = Fixture::new();
        fixture.seed(LEGACY_WORKING);
        fixture.seed(LEGACY_DOCUMENTS);
        fixture.seed(&format!(
            r#"
CREATE VIRTUAL TABLE legacy.episodes USING fts5(
    summary,outcome,files{separator}UNINDEXED,verification{separator}UNINDEXED,
    sources{separator}UNINDEXED,created_at UNINDEXED,tokenize='{tokenizer}'
);
INSERT INTO legacy.episodes(rowid,summary,outcome,files,verification,sources,created_at)
VALUES(7,'Legacy task','Keep outcome','["src/legacyneedle.php"]',
       '["proofneedle"]','["specs/sourceneedle.md"]','2026-07-27T00:00:00+00:00');
"#
        ));
        let unfinished = fixture.unfinished_snapshot();
        let episode_before = fixture.attached(|helper| {
            text_rows(
                helper,
                "SELECT CAST(rowid AS TEXT),summary,outcome,files,verification,sources,created_at \
             FROM legacy.episodes ORDER BY rowid",
                7,
            )
        });

        // Actual native dispatch opens and migrates the selected legacy file.
        for needle in ["legacyneedle", "proofneedle", "sourceneedle"] {
            let result = fixture.success(&["search", needle, "--layer", "episodic", "--json"]);
            let episodes = result["episodes"].as_array().unwrap();
            assert_eq!(episodes.len(), 1, "{label}: evidence token {needle}");
            assert_eq!(episodes[0]["id"], 7, "{label}: stable episode rowid");
        }
        assert_eq!(
            fixture.columns("documents"),
            ["path", "layer", "kind", "title", "summary", "content"]
        );
        fixture.attached(|helper| {
            assert_eq!(text_rows(helper,
                "SELECT CAST(rowid AS TEXT),summary,outcome,files,verification,sources,created_at \
                 FROM legacy.episodes ORDER BY rowid", 7), episode_before, "{label}");
            let ghosts: i64 = helper.conn.query_row(
                "SELECT COUNT(*) FROM legacy.documents WHERE path='ghost.md'", [],
                |row| row.get(0)).unwrap();
            assert_eq!(ghosts, 0, "{label}: stale derived document must be rebuilt");
            let sql: String = helper.conn.query_row(
                "SELECT sql FROM legacy.sqlite_master WHERE name='episodes'", [],
                |row| row.get(0)).unwrap();
            let normalized = sql.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
            assert!(normalized.contains("porter unicode61"), "{label}");
            for field in ["files", "verification", "sources"] {
                assert!(!normalized.contains(&format!("{field} unindexed")), "{label}: {field}");
            }
        });
        assert_eq!(fixture.unfinished_snapshot(), unfinished, "{label}");
    }
}

#[test]
fn noncorrupt_incompatible_episode_schema_rolls_back_without_quarantining_unfinished_state() {
    let fixture = Fixture::new();
    fixture.seed(LEGACY_WORKING);
    fixture.seed(LEGACY_DOCUMENTS);
    fixture.seed(
        r#"
CREATE VIRTUAL TABLE legacy.episodes USING fts5(
    summary,outcome,files,verification,created_at UNINDEXED,tokenize='unicode61'
);
INSERT INTO legacy.episodes(rowid,summary,outcome,files,verification,created_at)
VALUES(7,'Legacy task','Keep outcome','[]','[]','2026-07-27T00:00:00+00:00');
"#,
    );
    let schema = fixture.schema_snapshot();
    let unfinished = fixture.unfinished_snapshot();
    let retained = fixture.attached(|helper| {
        assert_eq!(
            text_rows(helper, "PRAGMA legacy.integrity_check", 1),
            vec![vec!["ok"]]
        );
        json!({
            "episodes":text_rows(helper,
                "SELECT CAST(rowid AS TEXT),summary,outcome,files,verification,created_at \
                 FROM legacy.episodes ORDER BY rowid", 6),
            "documents":text_rows(helper,
                "SELECT path,kind,title,content FROM legacy.documents ORDER BY path", 4),
            "source_state":text_rows(helper,
                "SELECT path,CAST(mtime_ns AS TEXT),CAST(size AS TEXT) \
                 FROM legacy.document_source_state ORDER BY path", 3),
        })
    });

    // Index enables corruption recovery, so this catches accidental schema-error quarantine.
    let output = fixture.run(&["index", "--json"]);
    assert!(
        !output.status.success(),
        "Incompatible schema must be refused"
    );
    assert_eq!(fixture.schema_snapshot(), schema);
    assert_eq!(fixture.unfinished_snapshot(), unfinished);
    fixture.attached(|helper| {
        assert_eq!(
            text_rows(helper, "PRAGMA legacy.integrity_check", 1),
            vec![vec!["ok"]]
        );
        assert_eq!(
            json!({
                "episodes":text_rows(helper,
                    "SELECT CAST(rowid AS TEXT),summary,outcome,files,verification,created_at \
                     FROM legacy.episodes ORDER BY rowid", 6),
                "documents":text_rows(helper,
                    "SELECT path,kind,title,content FROM legacy.documents ORDER BY path", 4),
                "source_state":text_rows(helper,
                    "SELECT path,CAST(mtime_ns AS TEXT),CAST(size AS TEXT) \
                     FROM legacy.document_source_state ORDER BY path", 3),
            }),
            retained
        );
    });
    assert!(
        !fixture
            .columns("working_tasks")
            .iter()
            .any(|name| name == "auto_checkpoint")
    );
    assert!(fixture.database.is_file());
    for entry in fs::read_dir(fixture.database.parent().unwrap()).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(
            !name.to_string_lossy().starts_with("context-damaged-"),
            "A non-corrupt incompatible schema must not be renamed as damaged"
        );
    }
}

#[test]
fn native_get_refuses_private_legacy_working_data_without_erasure_and_clear_recovers() {
    let fixture = Fixture::new();
    fixture.seed(LEGACY_WORKING);
    let private = "Customer email: person@example.test";
    fixture.attached(|helper| {
        helper
            .conn
            .execute(
                "UPDATE legacy.working_tasks SET progress=?1 WHERE task_id='TASK-LEGACY'",
                [private],
            )
            .unwrap();
    });
    let unfinished = fixture.unfinished_snapshot();
    let turns = fixture.deltas();

    let output = fixture.run(&["get", "--task-id", "TASK-LEGACY", "--json"]);
    assert_eq!(
        fixture.unfinished_snapshot(),
        unfinished,
        "A privacy refusal must preserve the legacy operator state and pending turns"
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(private),
        "Private legacy task text leaked to stdout"
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains(private),
        "Private legacy task text leaked to stderr"
    );
    assert!(
        !output.status.success(),
        "Private legacy state must be refused"
    );

    fixture.success(&["clear", "--task-id", "TASK-LEGACY", "--json"]);
    fixture.attached(|helper| {
        let count: i64 = helper
            .conn
            .query_row(
                "SELECT COUNT(*) FROM legacy.working_tasks WHERE task_id='TASK-LEGACY'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    });
    assert_eq!(
        fixture.deltas(),
        turns,
        "Explicit clear removes the task, without silently discarding buffered turns"
    );
}
