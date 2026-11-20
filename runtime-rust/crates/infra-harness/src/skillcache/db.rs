//! SkillCache tables in the session database.
//!
//! They live in `sessions.sqlite3` with every other Harness domain. Nothing
//! is written to a project's `memory-bank/` or `project-brain/`.
use crate::{HarnessError, Result, store};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

/// A trace nothing was compiled from is dropped after this many days.
pub const TRACE_DAYS: i64 = 30;
/// A session keeps this many traces nothing was compiled from.
pub const TRACES_PER_SESSION: usize = 10;

pub(crate) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS skillcache_traces(
            generation TEXT PRIMARY KEY, session_id TEXT NOT NULL, provider TEXT NOT NULL,
            model TEXT, request TEXT NOT NULL, skill_refs TEXT NOT NULL, steps TEXT NOT NULL,
            usage TEXT, redaction TEXT NOT NULL, trace_sha256 TEXT NOT NULL, created_at TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS skillcache_traces_session ON skillcache_traces(session_id,created_at);
        CREATE TABLE IF NOT EXISTS skillcache_capsules(
            capsule_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, skill_name TEXT NOT NULL,
            skill_path TEXT NOT NULL, mu_name TEXT NOT NULL, mu_description TEXT NOT NULL,
            form TEXT NOT NULL CHECK(form IN ('hybrid','textual')),
            status TEXT NOT NULL CHECK(status IN ('compiling','active','stale','needs_recompile','failed','retired')),
            revision INTEGER NOT NULL, capsule_sha256 TEXT NOT NULL, tree_sha256 TEXT NOT NULL,
            skill_files TEXT NOT NULL, env_fingerprint TEXT NOT NULL, schema_version INTEGER NOT NULL,
            compile_rules_version TEXT NOT NULL, source_session_id TEXT NOT NULL,
            source_generation TEXT NOT NULL, compile_id TEXT NOT NULL, artifact_dir TEXT NOT NULL,
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_used_at TEXT,
            package_sha256 TEXT NOT NULL DEFAULT '', capabilities TEXT NOT NULL DEFAULT '{}',
            engine_digest TEXT NOT NULL DEFAULT '', eval_arm TEXT, runs_reset_rowid INTEGER NOT NULL DEFAULT 0, reason TEXT);
        CREATE INDEX IF NOT EXISTS skillcache_capsules_skill ON skillcache_capsules(project_id,skill_path,status);
        CREATE VIRTUAL TABLE IF NOT EXISTS skillcache_capsules_fts USING fts5(
            capsule_id UNINDEXED, project_id UNINDEXED, skill_name, mu_name, mu_description,
            tokenize='porter unicode61');
        CREATE TABLE IF NOT EXISTS skillcache_gates(
            capsule_id TEXT NOT NULL, round INTEGER NOT NULL, gate TEXT NOT NULL,
            passed INTEGER NOT NULL, report TEXT NOT NULL, created_at TEXT NOT NULL,
            PRIMARY KEY(capsule_id, round, gate));
        CREATE TABLE IF NOT EXISTS skillcache_compiles(
            compile_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, source_session_id TEXT NOT NULL,
            source_generation TEXT NOT NULL,
            status TEXT NOT NULL CHECK(status IN ('queued','running','completed','failed','interrupted')),
            provider TEXT NOT NULL, model TEXT, rounds INTEGER NOT NULL DEFAULT 0, outcome TEXT,
            usage TEXT, receipt TEXT, started_at TEXT NOT NULL, finished_at TEXT,
            capsule_id TEXT, eval_arm TEXT);
        CREATE TABLE IF NOT EXISTS skillcache_runs(
            generation TEXT PRIMARY KEY, session_id TEXT NOT NULL, capsule_id TEXT NOT NULL,
            provider TEXT NOT NULL, model TEXT, outcome TEXT NOT NULL, steps TEXT NOT NULL,
            deopts INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS skillcache_runs_capsule ON skillcache_runs(capsule_id,created_at);",
    )?;
    for (table, column, definition) in [
        (
            "skillcache_capsules",
            "engine_digest",
            "TEXT NOT NULL DEFAULT ''",
        ),
        ("skillcache_capsules", "eval_arm", "TEXT"),
        (
            "skillcache_capsules",
            "runs_reset_rowid",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("skillcache_compiles", "eval_arm", "TEXT"),
    ] {
        let cols = {
            let mut q = db.prepare(&format!("PRAGMA table_info({table})"))?;
            q.query_map([], |r| r.get::<_, String>(1))?
                .collect::<std::result::Result<std::collections::BTreeSet<_>, _>>()?
        };
        if !cols.contains(column) {
            db.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))?;
        }
    }
    let needs_retired_check = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='skillcache_capsules'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some_and(|sql| !sql.contains("'retired'"));
    if needs_retired_check {
        db.execute_batch(&format!(
            "DROP TABLE IF EXISTS skillcache_capsules_new;
             CREATE TABLE skillcache_capsules_new(
                 capsule_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, skill_name TEXT NOT NULL,
                 skill_path TEXT NOT NULL, mu_name TEXT NOT NULL, mu_description TEXT NOT NULL,
                 form TEXT NOT NULL CHECK(form IN ('hybrid','textual')),
                 status TEXT NOT NULL CHECK(status IN ('compiling','active','stale','needs_recompile','failed','retired')),
                 revision INTEGER NOT NULL, capsule_sha256 TEXT NOT NULL, tree_sha256 TEXT NOT NULL,
                 skill_files TEXT NOT NULL, env_fingerprint TEXT NOT NULL, schema_version INTEGER NOT NULL,
                 compile_rules_version TEXT NOT NULL, source_session_id TEXT NOT NULL,
                 source_generation TEXT NOT NULL, compile_id TEXT NOT NULL, artifact_dir TEXT NOT NULL,
                 created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_used_at TEXT,
                 package_sha256 TEXT NOT NULL DEFAULT '', capabilities TEXT NOT NULL DEFAULT '{{}}',
                 engine_digest TEXT NOT NULL DEFAULT '', eval_arm TEXT, runs_reset_rowid INTEGER NOT NULL DEFAULT 0, reason TEXT);
             INSERT INTO skillcache_capsules_new({CAPSULE_COLUMNS},runs_reset_rowid)
             SELECT {CAPSULE_COLUMNS},runs_reset_rowid FROM skillcache_capsules;
             DROP TABLE skillcache_capsules;
             ALTER TABLE skillcache_capsules_new RENAME TO skillcache_capsules;
             CREATE INDEX IF NOT EXISTS skillcache_capsules_skill ON skillcache_capsules(project_id,skill_path,status);"
        ))?;
    }
    Ok(())
}

/// Runs once when the store opens.
pub(crate) fn recover(db: &Connection) -> Result<()> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(TRACE_DAYS))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    // A trace a capsule was compiled from stays as long as that capsule does.
    db.execute(
        "DELETE FROM skillcache_traces WHERE created_at<?1
           AND generation NOT IN (SELECT source_generation FROM skillcache_capsules)",
        [cutoff],
    )?;
    // Nothing that was running survives a restart, and nothing is retried.
    db.execute(
        "UPDATE skillcache_compiles SET status='interrupted',finished_at=?1,
           outcome='Server restarted; the compilation was interrupted.'
         WHERE status IN ('queued','running')",
        [store::now()],
    )?;
    db.execute(
        "UPDATE skillcache_capsules SET status='failed',updated_at=?1,
           reason='Server restarted; the compilation was interrupted.'
         WHERE status='compiling'",
        [store::now()],
    )?;
    Ok(())
}

pub(crate) struct TraceRow<'a> {
    pub generation: &'a str,
    pub session_id: &'a str,
    pub provider: &'a str,
    pub model: Option<&'a str>,
    pub request: &'a str,
    pub skill_refs: &'a Value,
    pub steps: &'a Value,
    pub redaction: &'a Value,
}

/// Stores one turn's trace and drops the session's oldest beyond its limit.
/// Returns the digest of the stored steps.
pub(crate) fn save_trace(db: &Connection, row: &TraceRow<'_>) -> Result<String> {
    let steps = row.steps.to_string();
    let digest = store::sha(steps.as_bytes());
    db.execute(
        "INSERT INTO skillcache_traces(generation,session_id,provider,model,request,skill_refs,steps,usage,redaction,trace_sha256,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,NULL,?8,?9,?10)
         ON CONFLICT(generation) DO UPDATE SET request=excluded.request,skill_refs=excluded.skill_refs,
           steps=excluded.steps,redaction=excluded.redaction,trace_sha256=excluded.trace_sha256,created_at=excluded.created_at",
        params![
            row.generation,
            row.session_id,
            row.provider,
            row.model,
            row.request,
            row.skill_refs.to_string(),
            steps,
            row.redaction.to_string(),
            digest,
            store::now()
        ],
    )?;
    db.execute(
        "DELETE FROM skillcache_traces WHERE session_id=?1
           AND generation NOT IN (SELECT source_generation FROM skillcache_capsules)
           AND generation NOT IN
           (SELECT generation FROM skillcache_traces WHERE session_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT ?2)",
        params![row.session_id, TRACES_PER_SESSION as i64],
    )?;
    Ok(digest)
}

/// The launches whose traces are stored: what keeps a snapshot alive.
pub(crate) fn trace_generations(db: &Connection) -> Result<Vec<String>> {
    let mut query = db.prepare("SELECT generation FROM skillcache_traces")?;
    let rows = query.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn trace_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "generation": row.get::<_, String>(0)?,
        "session_id": row.get::<_, String>(1)?,
        "provider": row.get::<_, String>(2)?,
        "model": row.get::<_, Option<String>>(3)?,
        "request": row.get::<_, String>(4)?,
        "skill_refs": decoded(row.get(5)?),
        "steps": decoded(row.get(6)?),
        "usage": decoded(row.get(7)?),
        "redaction": decoded(row.get(8)?),
        "trace_sha256": row.get::<_, String>(9)?,
        "created_at": row.get::<_, String>(10)?,
    }))
}

/// One trace in full, or `None`.
pub(crate) fn trace(db: &Connection, generation: &str) -> Result<Option<Value>> {
    Ok(db
        .query_row(
            "SELECT generation,session_id,provider,model,request,skill_refs,steps,usage,redaction,trace_sha256,created_at
             FROM skillcache_traces WHERE generation=?1",
            [generation],
            trace_from_row,
        )
        .optional()?)
}

/// Newest trace recorded for `project_id` referencing `skill_name` (or `skill_path`).
pub(crate) fn latest_trace_for_skill(
    db: &Connection,
    project_id: &str,
    skill_name: &str,
) -> Result<Option<Value>> {
    Ok(db
        .query_row(
            "SELECT t.generation,t.session_id,t.provider,t.model,t.request,t.skill_refs,t.steps,t.usage,t.redaction,t.trace_sha256,t.created_at
             FROM skillcache_traces t
             JOIN sessions s ON s.id=t.session_id
             WHERE s.project_id=?1
               AND EXISTS(
                   SELECT 1 FROM json_each(t.skill_refs)
                   WHERE json_extract(value,'$.name')=?2 OR json_extract(value,'$.path')=?2
               )
             ORDER BY t.created_at DESC,t.rowid DESC
             LIMIT 1",
            params![project_id, skill_name],
            trace_from_row,
        )
        .optional()?)
}

pub(crate) fn delete_trace(db: &Connection, generation: &str) -> Result<bool> {
    Ok(db.execute(
        "DELETE FROM skillcache_traces WHERE generation=?1",
        [generation],
    )? > 0)
}

const CAPSULE_COLUMNS: &str = "capsule_id,project_id,skill_name,skill_path,mu_name,mu_description,form,status,revision,capsule_sha256,tree_sha256,skill_files,env_fingerprint,schema_version,compile_rules_version,source_session_id,source_generation,compile_id,artifact_dir,created_at,updated_at,last_used_at,package_sha256,capabilities,engine_digest,eval_arm,reason";

fn capsule_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "capsule_id": row.get::<_, String>(0)?,
        "project_id": row.get::<_, String>(1)?,
        "skill_name": row.get::<_, String>(2)?,
        "skill_path": row.get::<_, String>(3)?,
        "mu": {"name": row.get::<_, String>(4)?, "description": row.get::<_, String>(5)?},
        "form": row.get::<_, String>(6)?,
        "status": row.get::<_, String>(7)?,
        "revision": row.get::<_, i64>(8)?,
        "capsule_sha256": row.get::<_, String>(9)?,
        "tree_sha256": row.get::<_, String>(10)?,
        "skill_files": decoded(row.get(11)?),
        "env_fingerprint": row.get::<_, String>(12)?,
        "schema_version": row.get::<_, i64>(13)?,
        "compile_rules_version": row.get::<_, String>(14)?,
        "source_session_id": row.get::<_, String>(15)?,
        "source_generation": row.get::<_, String>(16)?,
        "compile_id": row.get::<_, String>(17)?,
        "artifact_dir": row.get::<_, String>(18)?,
        "created_at": row.get::<_, String>(19)?,
        "updated_at": row.get::<_, String>(20)?,
        "last_used_at": row.get::<_, Option<String>>(21)?,
        "package_sha256": row.get::<_, String>(22)?,
        "capabilities": decoded(row.get(23)?),
        "engine_digest": row.get::<_, String>(24)?,
        "eval_arm": row.get::<_, Option<String>>(25)?,
        "reason": row.get::<_, Option<String>>(26)?,
    }))
}

/// A capsule that is being compiled: a row the gate reports can refer to
/// before there is anything to mount.
#[expect(
    clippy::too_many_arguments,
    reason = "One capsule row opens with its session, turn, skill and optional eval arm."
)]
pub(crate) fn begin_capsule(
    db: &Connection,
    capsule_id: &str,
    compile_id: &str,
    project_id: &str,
    session_id: &str,
    generation: &str,
    skill: &Value,
    eval_arm: Option<&str>,
) -> Result<()> {
    let now = store::now();
    db.execute(
        "INSERT INTO skillcache_capsules(capsule_id,project_id,skill_name,skill_path,mu_name,mu_description,form,status,revision,capsule_sha256,tree_sha256,skill_files,env_fingerprint,schema_version,compile_rules_version,source_session_id,source_generation,compile_id,artifact_dir,eval_arm,created_at,updated_at)
         VALUES(?1,?2,?3,?4,'','','hybrid','compiling',1,'',?5,'{}','',?6,?7,?8,?9,?10,'',?11,?12,?12)",
        params![
            capsule_id,
            project_id,
            skill["name"].as_str().unwrap_or(""),
            skill["path"].as_str().unwrap_or(""),
            skill["tree_sha256"].as_str().unwrap_or(""),
            infra_skillcache::SCHEMA_VERSION as i64,
            infra_skillcache::compile_rules::version(),
            session_id,
            generation,
            compile_id,
            eval_arm,
            now
        ],
    )?;
    Ok(())
}

/// What a qualified capsule is stored with.
pub(crate) struct Qualified<'a> {
    pub capsule_id: &'a str,
    pub mu_name: &'a str,
    pub mu_description: &'a str,
    pub form: &'a str,
    pub capsule_sha256: &'a str,
    pub package_sha256: &'a str,
    pub skill_files: &'a Value,
    pub env_fingerprint: &'a str,
    pub engine_digest: &'a str,
    pub artifact_dir: &'a str,
    pub capabilities: &'a Value,
}

pub(crate) fn activate_capsule(db: &Connection, capsule: &Qualified<'_>) -> Result<()> {
    let changed = db.execute(
        "UPDATE skillcache_capsules SET mu_name=?2,mu_description=?3,form=?4,status='active',capsule_sha256=?5,
           package_sha256=?6,skill_files=?7,env_fingerprint=?8,engine_digest=?9,artifact_dir=?10,capabilities=?11,reason=NULL,updated_at=?12
         WHERE capsule_id=?1",
        params![
            capsule.capsule_id,
            capsule.mu_name,
            capsule.mu_description,
            capsule.form,
            capsule.capsule_sha256,
            capsule.package_sha256,
            capsule.skill_files.to_string(),
            capsule.env_fingerprint,
            capsule.engine_digest,
            capsule.artifact_dir,
            capsule.capabilities.to_string(),
            store::now()
        ],
    )?;
    if changed == 0 {
        return Err(HarnessError::missing("Capsule not found."));
    }
    db.execute(
        "DELETE FROM skillcache_capsules_fts WHERE capsule_id=?1",
        [capsule.capsule_id],
    )?;
    db.execute(
        "INSERT INTO skillcache_capsules_fts(capsule_id,project_id,skill_name,mu_name,mu_description)
         SELECT capsule_id,project_id,skill_name,mu_name,mu_description FROM skillcache_capsules WHERE capsule_id=?1",
        [capsule.capsule_id],
    )?;
    Ok(())
}

/// Moves a capsule to `status` with the reason a person is shown.
pub(crate) fn set_capsule_status(
    db: &Connection,
    capsule_id: &str,
    status: &str,
    reason: Option<&str>,
) -> Result<()> {
    let changed = db.execute(
        "UPDATE skillcache_capsules SET status=?2,reason=?3,updated_at=?4 WHERE capsule_id=?1",
        params![capsule_id, status, reason, store::now()],
    )?;
    if changed == 0 {
        return Err(HarnessError::missing("Capsule not found."));
    }
    Ok(())
}

/// Records fresh identities after a requalification.
pub(crate) fn refresh_capsule(
    db: &Connection,
    capsule_id: &str,
    skill_files: &Value,
    tree_sha256: &str,
    env_fingerprint: &str,
    engine_digest: &str,
) -> Result<()> {
    db.execute(
        "UPDATE skillcache_capsules SET skill_files=?2,tree_sha256=?3,env_fingerprint=?4,engine_digest=?5,
           schema_version=?6,compile_rules_version=?7,status='active',reason=NULL,
           runs_reset_rowid=COALESCE((SELECT MAX(rowid) FROM skillcache_runs WHERE capsule_id=?1),0),
           updated_at=?8
         WHERE capsule_id=?1",
        params![
            capsule_id,
            skill_files.to_string(),
            tree_sha256,
            env_fingerprint,
            engine_digest,
            infra_skillcache::SCHEMA_VERSION as i64,
            infra_skillcache::compile_rules::version(),
            store::now()
        ],
    )?;
    Ok(())
}

pub(crate) fn capsule(db: &Connection, capsule_id: &str) -> Result<Option<Value>> {
    Ok(db
        .query_row(
            &format!("SELECT {CAPSULE_COLUMNS} FROM skillcache_capsules WHERE capsule_id=?1"),
            [capsule_id],
            capsule_row,
        )
        .optional()?)
}

/// A project's capsules, newest first, optionally narrowed.
pub(crate) fn capsules(
    db: &Connection,
    project_id: &str,
    skill: Option<&str>,
    status: Option<&str>,
    form: Option<&str>,
    include_retired: bool,
) -> Result<Vec<Value>> {
    let mut query = db.prepare(&format!(
        "SELECT {CAPSULE_COLUMNS} FROM skillcache_capsules
         WHERE project_id=?1 AND eval_arm IS NULL
           AND (?2 IS NULL OR skill_name=?2 OR skill_path=?2)
           AND (?3 IS NULL OR status=?3)
           AND (?3 IS NOT NULL OR ?5 != 0 OR status != 'retired')
           AND (?4 IS NULL OR form=?4)
         ORDER BY created_at DESC,rowid DESC"
    ))?;
    let rows = query.query_map(
        params![project_id, skill, status, form, i64::from(include_retired)],
        capsule_row,
    )?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// The active capsules compiled from the skill at `skill_path` (excluding
/// evaluation-only ablation capsules).
pub(crate) fn active_capsules(
    db: &Connection,
    project_id: &str,
    skill_path: &str,
) -> Result<Vec<Value>> {
    active_capsules_for_eval(db, project_id, skill_path, None)
}

/// The active capsules compiled for `eval_arm` (or normal non-eval capsules
/// when `eval_arm` is `None`).
pub(crate) fn active_capsules_for_eval(
    db: &Connection,
    project_id: &str,
    skill_path: &str,
    eval_arm: Option<&str>,
) -> Result<Vec<Value>> {
    let mut query = db.prepare(&format!(
        "SELECT {CAPSULE_COLUMNS} FROM skillcache_capsules
         WHERE project_id=?1 AND skill_path=?2 AND status='active'
           AND ((?3 IS NULL AND eval_arm IS NULL) OR eval_arm=?3)
         ORDER BY created_at DESC,rowid DESC"
    ))?;
    let rows = query.query_map(params![project_id, skill_path, eval_arm], capsule_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub(crate) fn touch_capsule(db: &Connection, capsule_id: &str) -> Result<()> {
    db.execute(
        "UPDATE skillcache_capsules SET last_used_at=?2 WHERE capsule_id=?1",
        params![capsule_id, store::now()],
    )?;
    Ok(())
}

/// One warm turn: which capsule ran, how it ended and how often a step fell
/// back to the agent. Its tokens and cost are the launch's own, read by
/// `generation`; they are not copied here.
pub(crate) struct Run<'a> {
    pub generation: &'a str,
    pub session_id: &'a str,
    pub capsule_id: &'a str,
    pub provider: &'a str,
    pub model: Option<&'a str>,
    pub outcome: &'a str,
    pub steps: &'a Value,
    pub deopts: i64,
}

pub(crate) fn record_run(db: &Connection, run: &Run<'_>) -> Result<()> {
    db.execute(
        "INSERT INTO skillcache_runs(generation,session_id,capsule_id,provider,model,outcome,steps,deopts,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(generation) DO UPDATE SET outcome=excluded.outcome,steps=excluded.steps,deopts=excluded.deopts",
        params![
            run.generation,
            run.session_id,
            run.capsule_id,
            run.provider,
            run.model,
            run.outcome,
            run.steps.to_string(),
            run.deopts,
            store::now()
        ],
    )?;
    Ok(())
}

/// A session's warm turns, newest first.
pub(crate) fn session_runs(db: &Connection, session_id: &str) -> Result<Vec<Value>> {
    let mut query = db.prepare(
        "SELECT generation,capsule_id,provider,model,outcome,steps,deopts,created_at
         FROM skillcache_runs WHERE session_id=?1 ORDER BY created_at DESC,rowid DESC",
    )?;
    let rows = query.query_map([session_id], |row| {
        Ok(json!({
            "generation": row.get::<_, String>(0)?,
            "capsule_id": row.get::<_, String>(1)?,
            "provider": row.get::<_, String>(2)?,
            "model": row.get::<_, Option<String>>(3)?,
            "outcome": row.get::<_, String>(4)?,
            "steps": decoded(row.get(5)?),
            "deopts": row.get::<_, i64>(6)?,
            "created_at": row.get::<_, String>(7)?,
        }))
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn map_capsule_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "generation": row.get::<_, String>(0)?,
        "session_id": row.get::<_, String>(1)?,
        "provider": row.get::<_, String>(2)?,
        "model": row.get::<_, Option<String>>(3)?,
        "outcome": row.get::<_, String>(4)?,
        "deopts": row.get::<_, i64>(5)?,
        "created_at": row.get::<_, String>(6)?,
    }))
}

/// A capsule's warm turns, newest first.
pub(crate) fn capsule_runs(db: &Connection, capsule_id: &str, limit: i64) -> Result<Vec<Value>> {
    let mut query = db.prepare(
        "SELECT generation,session_id,provider,model,outcome,deopts,created_at
         FROM skillcache_runs WHERE capsule_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT ?2",
    )?;
    let rows = query.query_map(params![capsule_id, limit], map_capsule_run)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// A capsule's warm turns since `since` (and strictly after the most recent
/// requalification), newest first.
pub(crate) fn capsule_runs_since(
    db: &Connection,
    capsule_id: &str,
    since: &str,
    limit: i64,
) -> Result<Vec<Value>> {
    let mut query = db.prepare(
        "SELECT generation,session_id,provider,model,outcome,deopts,created_at
         FROM skillcache_runs
         WHERE capsule_id=?1
           AND created_at>=?2
           AND rowid>COALESCE((SELECT runs_reset_rowid FROM skillcache_capsules WHERE capsule_id=?1),0)
         ORDER BY created_at DESC,rowid DESC LIMIT ?3",
    )?;
    let rows = query.query_map(params![capsule_id, since, limit], map_capsule_run)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Removes a capsule's rows. Its files are the caller's to remove.
pub(crate) fn delete_capsule(db: &Connection, capsule_id: &str) -> Result<bool> {
    db.execute(
        "DELETE FROM skillcache_capsules_fts WHERE capsule_id=?1",
        [capsule_id],
    )?;
    db.execute(
        "DELETE FROM skillcache_gates WHERE capsule_id=?1",
        [capsule_id],
    )?;
    db.execute(
        "DELETE FROM skillcache_runs WHERE capsule_id=?1",
        [capsule_id],
    )?;
    Ok(db.execute(
        "DELETE FROM skillcache_capsules WHERE capsule_id=?1",
        [capsule_id],
    )? > 0)
}

pub(crate) fn record_gate(
    db: &Connection,
    capsule_id: &str,
    round: i64,
    gate: &str,
    passed: bool,
    report: &Value,
) -> Result<()> {
    db.execute(
        "INSERT INTO skillcache_gates(capsule_id,round,gate,passed,report,created_at) VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(capsule_id,round,gate) DO UPDATE SET passed=excluded.passed,report=excluded.report,created_at=excluded.created_at",
        params![capsule_id, round, gate, i64::from(passed), report.to_string(), store::now()],
    )?;
    Ok(())
}

pub(crate) fn gates(db: &Connection, capsule_id: &str) -> Result<Vec<Value>> {
    let mut query = db.prepare(
        "SELECT round,gate,passed,report,created_at FROM skillcache_gates WHERE capsule_id=?1 ORDER BY round,rowid",
    )?;
    let rows = query.query_map([capsule_id], |row| {
        Ok(json!({
            "round": row.get::<_, i64>(0)?,
            "gate": row.get::<_, String>(1)?,
            "passed": row.get::<_, i64>(2)? != 0,
            "report": decoded(row.get(3)?),
            "created_at": row.get::<_, String>(4)?,
        }))
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub(crate) fn next_round(db: &Connection, capsule_id: &str) -> Result<i64> {
    Ok(db.query_row(
        "SELECT COALESCE(MAX(round),0)+1 FROM skillcache_gates WHERE capsule_id=?1",
        [capsule_id],
        |row| row.get(0),
    )?)
}

#[expect(
    clippy::too_many_arguments,
    reason = "One row of the compile journal is written from its eight identities."
)]
pub(crate) fn begin_compile(
    db: &Connection,
    compile_id: &str,
    capsule_id: &str,
    project_id: &str,
    session_id: &str,
    generation: &str,
    provider: &str,
    model: Option<&str>,
    eval_arm: Option<&str>,
) -> Result<()> {
    db.execute(
        "INSERT INTO skillcache_compiles(compile_id,project_id,source_session_id,source_generation,status,provider,model,started_at,capsule_id,eval_arm)
         VALUES(?1,?2,?3,?4,'queued',?5,?6,?7,?8,?9)",
        params![compile_id, project_id, session_id, generation, provider, model, store::now(), capsule_id, eval_arm],
    )?;
    Ok(())
}

pub(crate) fn compile_running(db: &Connection, compile_id: &str) -> Result<()> {
    db.execute(
        "UPDATE skillcache_compiles SET status='running' WHERE compile_id=?1 AND status='queued'",
        [compile_id],
    )?;
    Ok(())
}

pub(crate) fn finish_compile(
    db: &Connection,
    compile_id: &str,
    status: &str,
    rounds: i64,
    outcome: &str,
    usage: &Value,
    receipt: &Value,
) -> Result<()> {
    db.execute(
        "UPDATE skillcache_compiles SET status=?2,rounds=?3,outcome=?4,usage=?5,receipt=?6,finished_at=?7 WHERE compile_id=?1",
        params![
            compile_id,
            status,
            rounds,
            outcome,
            usage.to_string(),
            receipt.to_string(),
            store::now()
        ],
    )?;
    Ok(())
}

fn compile_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "compile_id": row.get::<_, String>(0)?,
        "project_id": row.get::<_, String>(1)?,
        "source_session_id": row.get::<_, String>(2)?,
        "source_generation": row.get::<_, String>(3)?,
        "status": row.get::<_, String>(4)?,
        "provider": row.get::<_, String>(5)?,
        "model": row.get::<_, Option<String>>(6)?,
        "rounds": row.get::<_, i64>(7)?,
        "outcome": row.get::<_, Option<String>>(8)?,
        "usage": decoded(row.get(9)?),
        "receipt": decoded(row.get(10)?),
        "started_at": row.get::<_, String>(11)?,
        "finished_at": row.get::<_, Option<String>>(12)?,
        "capsule_id": row.get::<_, Option<String>>(13)?,
        "eval_arm": row.get::<_, Option<String>>(14)?,
    }))
}
const COMPILE_COLUMNS: &str = "compile_id,project_id,source_session_id,source_generation,status,provider,model,rounds,outcome,usage,receipt,started_at,finished_at,capsule_id,eval_arm";

pub(crate) fn compile(db: &Connection, compile_id: &str) -> Result<Option<Value>> {
    Ok(db
        .query_row(
            &format!("SELECT {COMPILE_COLUMNS} FROM skillcache_compiles WHERE compile_id=?1"),
            [compile_id],
            compile_row,
        )
        .optional()?)
}

/// The compilations started from a session's turns, newest first.
pub(crate) fn session_compiles(db: &Connection, session_id: &str) -> Result<Vec<Value>> {
    let mut query = db.prepare(&format!(
        "SELECT {COMPILE_COLUMNS} FROM skillcache_compiles WHERE source_session_id=?1 ORDER BY started_at DESC,rowid DESC"
    ))?;
    let rows = query.query_map([session_id], compile_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Whether a compilation of this launch's trace is waiting or running.
pub(crate) fn compile_pending(db: &Connection, generation: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM skillcache_compiles WHERE source_generation=?1 AND status IN ('queued','running'))",
        [generation],
        |row| row.get::<_, i64>(0),
    )? != 0)
}

pub(crate) fn pending_compiles(db: &Connection) -> Result<i64> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM skillcache_compiles WHERE status IN ('queued','running')",
        [],
        |row| row.get(0),
    )?)
}

fn decoded(text: Option<String>) -> Value {
    text.and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

/// What a session's traces are, newest first, without their steps.
pub(crate) fn session_traces(db: &Connection, session_id: &str) -> Result<Vec<Value>> {
    let mut query = db.prepare(
        "SELECT generation,provider,model,skill_refs,redaction,trace_sha256,created_at,json_array_length(steps)
         FROM skillcache_traces WHERE session_id=?1 ORDER BY created_at DESC,rowid DESC",
    )?;
    let rows = query.query_map([session_id], |row| {
        Ok(json!({
            "generation": row.get::<_, String>(0)?,
            "provider": row.get::<_, String>(1)?,
            "model": row.get::<_, Option<String>>(2)?,
            "skill_refs": decoded(row.get(3)?),
            "redaction": decoded(row.get(4)?),
            "trace_sha256": row.get::<_, String>(5)?,
            "created_at": row.get::<_, String>(6)?,
            "steps": row.get::<_, i64>(7)?,
        }))
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        db
    }
    fn keep(db: &Connection, generation: &str, session: &str) -> String {
        save_trace(
            db,
            &TraceRow {
                generation,
                session_id: session,
                provider: "claude",
                model: None,
                request: "Summarise",
                skill_refs: &json!([{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md"}]),
                steps: &json!([{"index": 0, "kind": "tool", "tool": "Bash"}]),
                redaction: &json!({"redacted": 0}),
            },
        )
        .unwrap()
    }
    fn generations(db: &Connection, session: &str) -> Vec<String> {
        session_traces(db, session)
            .unwrap()
            .iter()
            .map(|trace| trace["generation"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn a_session_keeps_only_its_newest_traces() {
        let db = database();
        for index in 0..TRACES_PER_SESSION + 3 {
            keep(&db, &format!("a-{index:02}"), "session-a");
        }
        keep(&db, "b-00", "session-b");
        let kept = generations(&db, "session-a");
        assert_eq!(kept.len(), TRACES_PER_SESSION);
        assert_eq!(kept[0], format!("a-{:02}", TRACES_PER_SESSION + 2));
        assert!(!kept.contains(&"a-00".to_string()) && !kept.contains(&"a-02".to_string()));
        // Another session's traces are its own.
        assert_eq!(generations(&db, "session-b"), ["b-00"]);
        // The same generation again replaces its row: one turn, one trace.
        let digest = keep(&db, "b-00", "session-b");
        assert_eq!(generations(&db, "session-b"), ["b-00"]);
        let listed = &session_traces(&db, "session-b").unwrap()[0];
        assert_eq!(listed["trace_sha256"], digest.as_str());
        assert_eq!(listed["steps"], 1);
        assert!(
            listed.get("request").is_none(),
            "a listing carries no content"
        );
    }

    #[test]
    fn a_restart_interrupts_what_was_compiling_and_retries_nothing() {
        let db = database();
        let skill =
            json!({"name": "pdf", "path": ".claude/skills/pdf/SKILL.md", "tree_sha256": "a"});
        for (compile, capsule) in [
            ("c-queued", "k-queued"),
            ("c-running", "k-running"),
            ("c-done", "k-done"),
        ] {
            begin_capsule(
                &db, capsule, compile, "project", "session", "gen", &skill, None,
            )
            .unwrap();
            begin_compile(
                &db, compile, capsule, "project", "session", "gen", "claude", None, None,
            )
            .unwrap();
        }
        compile_running(&db, "c-running").unwrap();
        finish_compile(
            &db,
            "c-done",
            "completed",
            1,
            "Qualified.",
            &json!({}),
            &json!({}),
        )
        .unwrap();
        db.execute(
            "UPDATE skillcache_capsules SET status='active' WHERE capsule_id='k-done'",
            [],
        )
        .unwrap();
        assert!(compile_pending(&db, "gen").unwrap());
        assert_eq!(pending_compiles(&db).unwrap(), 2);
        recover(&db).unwrap();
        let status = |id: &str| {
            compile(&db, id).unwrap().unwrap()["status"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(status("c-queued"), "interrupted");
        assert_eq!(status("c-running"), "interrupted");
        assert_eq!(status("c-done"), "completed");
        let state = |id: &str| {
            capsule(&db, id).unwrap().unwrap()["status"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(state("k-queued"), "failed");
        assert_eq!(state("k-running"), "failed");
        assert_eq!(state("k-done"), "active");
        assert!(
            capsule(&db, "k-queued").unwrap().unwrap()["reason"]
                .as_str()
                .unwrap()
                .contains("restarted")
        );
        assert!(!compile_pending(&db, "gen").unwrap());
        // A trace a capsule was compiled from outlives its age and the
        // per-session limit.
        keep(&db, "gen", "session");
        let past = (chrono::Utc::now() - chrono::Duration::days(TRACE_DAYS + 5))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        db.execute("UPDATE skillcache_traces SET created_at=?1", [past])
            .unwrap();
        for index in 0..TRACES_PER_SESSION + 2 {
            keep(&db, &format!("later-{index:02}"), "session");
        }
        recover(&db).unwrap();
        assert!(trace(&db, "gen").unwrap().is_some());
        assert_eq!(generations(&db, "session").len(), TRACES_PER_SESSION + 1);
        // Gate rows are per capsule, round and gate.
        record_gate(&db, "k-done", 1, "gate1", true, &json!({"gate": "gate1"})).unwrap();
        record_gate(
            &db,
            "k-done",
            1,
            "gate1",
            false,
            &json!({"gate": "gate1", "again": true}),
        )
        .unwrap();
        assert_eq!(gates(&db, "k-done").unwrap().len(), 1);
        assert_eq!(gates(&db, "k-done").unwrap()[0]["passed"], false);
        assert_eq!(next_round(&db, "k-done").unwrap(), 2);
        assert!(delete_capsule(&db, "k-done").unwrap());
        assert!(gates(&db, "k-done").unwrap().is_empty());
        assert!(delete_trace(&db, "gen").unwrap());
    }

    #[test]
    fn traces_past_their_age_are_dropped_when_the_store_opens() {
        let db = database();
        keep(&db, "old", "session-a");
        keep(&db, "new", "session-a");
        let past = (chrono::Utc::now() - chrono::Duration::days(TRACE_DAYS + 1))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        db.execute(
            "UPDATE skillcache_traces SET created_at=?1 WHERE generation='old'",
            [past],
        )
        .unwrap();
        recover(&db).unwrap();
        assert_eq!(generations(&db, "session-a"), ["new"]);
        // Opening twice is harmless.
        initialize(&db).unwrap();
        recover(&db).unwrap();
        assert_eq!(generations(&db, "session-a"), ["new"]);
    }

    #[test]
    fn retired_eval_arm_and_requalify_run_reset_work() {
        let db = database();
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY, project_id TEXT NOT NULL);
             INSERT INTO sessions(id, project_id) VALUES('session-1', 'project');",
        )
        .unwrap();
        keep(&db, "gen-1", "session-1");
        assert_eq!(
            latest_trace_for_skill(&db, "project", "pdf")
                .unwrap()
                .unwrap()["generation"],
            "gen-1"
        );

        let skill =
            json!({"name": "pdf", "path": ".claude/skills/pdf/SKILL.md", "tree_sha256": "a"});
        for (cid, cap_id, eval_arm) in [
            ("comp-norm", "cap-norm", None),
            ("comp-ret", "cap-ret", None),
            ("comp-eval", "cap-eval", Some("skillcache_agent")),
        ] {
            begin_capsule(
                &db,
                cap_id,
                cid,
                "project",
                "session-1",
                "gen-1",
                &skill,
                eval_arm,
            )
            .unwrap();
            begin_compile(
                &db,
                cid,
                cap_id,
                "project",
                "session-1",
                "gen-1",
                "claude",
                None,
                eval_arm,
            )
            .unwrap();
            activate_capsule(
                &db,
                &Qualified {
                    capsule_id: cap_id,
                    mu_name: "mu",
                    mu_description: "desc",
                    form: "hybrid",
                    capsule_sha256: "sha",
                    package_sha256: "psha",
                    skill_files: &json!({}),
                    env_fingerprint: "env",
                    engine_digest: "eng",
                    artifact_dir: "/tmp/cap",
                    capabilities: &json!({}),
                },
            )
            .unwrap();
        }
        set_capsule_status(&db, "cap-ret", "retired", Some("Stop using")).unwrap();

        assert_eq!(
            active_capsules(&db, "project", ".claude/skills/pdf/SKILL.md")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            active_capsules_for_eval(
                &db,
                "project",
                ".claude/skills/pdf/SKILL.md",
                Some("skillcache_agent"),
            )
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            capsules(&db, "project", None, None, None, false)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            capsules(&db, "project", None, None, None, true)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            capsules(&db, "project", None, Some("retired"), None, false)
                .unwrap()
                .len(),
            1
        );

        // Two failed runs before requalify within the same second do not count
        // after refresh_capsule updates runs_reset_rowid.
        for g in ["r1", "r2"] {
            record_run(
                &db,
                &Run {
                    generation: g,
                    session_id: "session-1",
                    capsule_id: "cap-norm",
                    provider: "claude",
                    model: None,
                    outcome: "fallback",
                    steps: &json!([]),
                    deopts: 1,
                },
            )
            .unwrap();
        }
        refresh_capsule(&db, "cap-norm", &json!({}), "tree-2", "env-2", "eng-2").unwrap();
        let since = capsule(&db, "cap-norm").unwrap().unwrap()["updated_at"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            capsule_runs_since(&db, "cap-norm", &since, 10)
                .unwrap()
                .is_empty()
        );
        record_run(
            &db,
            &Run {
                generation: "r3",
                session_id: "session-1",
                capsule_id: "cap-norm",
                provider: "claude",
                model: None,
                outcome: "warm",
                steps: &json!([]),
                deopts: 0,
            },
        )
        .unwrap();
        assert_eq!(
            capsule_runs_since(&db, "cap-norm", &since, 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn legacy_capsule_table_migrates_to_allow_retired() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE skillcache_capsules(
                capsule_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, skill_name TEXT NOT NULL,
                skill_path TEXT NOT NULL, mu_name TEXT NOT NULL, mu_description TEXT NOT NULL,
                form TEXT NOT NULL CHECK(form IN ('hybrid','textual')),
                status TEXT NOT NULL CHECK(status IN ('compiling','active','stale','needs_recompile','failed')),
                revision INTEGER NOT NULL, capsule_sha256 TEXT NOT NULL, tree_sha256 TEXT NOT NULL,
                skill_files TEXT NOT NULL, env_fingerprint TEXT NOT NULL, schema_version INTEGER NOT NULL,
                compile_rules_version TEXT NOT NULL, source_session_id TEXT NOT NULL,
                source_generation TEXT NOT NULL, compile_id TEXT NOT NULL, artifact_dir TEXT NOT NULL,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_used_at TEXT,
                package_sha256 TEXT NOT NULL DEFAULT '', capabilities TEXT NOT NULL DEFAULT '{}',
                reason TEXT);
            INSERT INTO skillcache_capsules VALUES(
                'k1','p','pdf','.claude/skills/pdf/SKILL.md','mu','d','hybrid','active',
                1,'c','t','{}','e',1,'1','s','g','cid','/tmp','2026-01-01T00:00:00Z',
                '2026-01-01T00:00:00Z',NULL,'','{}',NULL);",
        )
        .unwrap();
        initialize(&db).unwrap();
        set_capsule_status(&db, "k1", "retired", Some("Retired")).unwrap();
        assert_eq!(capsule(&db, "k1").unwrap().unwrap()["status"], "retired");
    }
}
