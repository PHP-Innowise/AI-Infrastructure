//! SkillCache tables in the session database.
//!
//! They live in `sessions.sqlite3` with every other Harness domain. Nothing
//! is written to a project's `memory-bank/` or `project-brain/`.
use crate::{HarnessError, Result, store};
use infra_skillcache::candidate::Candidate;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Filesystem/IR verification race window (2 seconds in nanoseconds).
pub(crate) const RACY_NS: u128 = 2_000_000_000;

/// Current wall-clock time in Unix nanoseconds for RACY_NS manifest guards.
pub(crate) fn current_unix_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

/// Persisted artifact dependency manifest bound to a compiled SkillCache capsule.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ArtifactDependencyManifest {
    #[serde(default)]
    pub skill_paths: Vec<String>,
    #[serde(default)]
    pub watched_globs: Vec<String>,
    #[serde(default)]
    pub manifest_files: Vec<String>,
    #[serde(default)]
    pub ast_selectors: Vec<String>,
    #[serde(default)]
    pub route_selectors: Vec<String>,
    #[serde(default)]
    pub table_selectors: Vec<String>,
    #[serde(default)]
    pub caller_files: Vec<String>,
    #[serde(default)]
    pub ir_revision: u64,
    #[serde(default)]
    pub verified_ns: u128,
    #[serde(default)]
    pub path_hashes: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeltaMatch {
    None,
    ExactSkill,
    Dependency,
}

pub(crate) fn normalize_rel_path(path: &str) -> String {
    let normalized = path.trim().replace('\\', "/");
    let stripped = normalized.trim_start_matches("./").trim_end_matches('/');
    stripped.to_string()
}

pub(crate) fn path_matches(candidate: &str, needle: &str) -> bool {
    let sp = normalize_rel_path(candidate);
    let needle = normalize_rel_path(needle);
    if sp.is_empty() || needle.is_empty() {
        return false;
    }
    sp == needle
        || sp.starts_with(&format!("{needle}/"))
        || needle.starts_with(&format!("{sp}/"))
        || sp.ends_with(&format!("/{needle}"))
        || sp.contains(&format!("/{needle}/"))
}

pub(crate) fn glob_matches(pattern: &str, path: &str) -> bool {
    let pat = normalize_rel_path(pattern);
    let target = normalize_rel_path(path);
    if pat.is_empty() || target.is_empty() {
        return false;
    }
    if !pat.contains(['*', '?']) {
        return path_matches(&pat, &target);
    }
    fn inner(p: &[u8], s: &[u8]) -> bool {
        if p.is_empty() {
            return s.is_empty();
        }
        if p.starts_with(b"**") {
            let next_p = if p.get(2) == Some(&b'/') {
                &p[3..]
            } else {
                &p[2..]
            };
            if inner(next_p, s) {
                return true;
            }
            return !s.is_empty() && inner(p, &s[1..]);
        }
        if p[0] == b'*' {
            if inner(&p[1..], s) {
                return true;
            }
            return !s.is_empty() && s[0] != b'/' && inner(p, &s[1..]);
        }
        if p[0] == b'?' {
            return !s.is_empty() && s[0] != b'/' && inner(&p[1..], &s[1..]);
        }
        !s.is_empty() && p[0] == s[0] && inner(&p[1..], &s[1..])
    }
    inner(pat.as_bytes(), target.as_bytes())
}

impl ArtifactDependencyManifest {
    pub(crate) fn from_value(value: &Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }

    pub(crate) fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }

    pub(crate) fn evaluate_delta(
        &self,
        changed_paths: &[String],
        caller_files: &[String],
        ir_revision: u64,
        now_ns: u128,
    ) -> DeltaMatch {
        if changed_paths.is_empty() && caller_files.is_empty() {
            return DeltaMatch::None;
        }
        for changed in changed_paths {
            if self
                .skill_paths
                .iter()
                .any(|skill| path_matches(skill, changed))
            {
                return DeltaMatch::ExactSkill;
            }
        }
        let dep_changed = changed_paths.iter().any(|changed| {
            self.watched_globs
                .iter()
                .any(|glob| glob_matches(glob, changed))
                || self
                    .manifest_files
                    .iter()
                    .any(|file| path_matches(file, changed))
                || self
                    .ast_selectors
                    .iter()
                    .any(|sel| path_matches(sel, changed))
                || self
                    .route_selectors
                    .iter()
                    .any(|sel| path_matches(sel, changed))
                || self
                    .table_selectors
                    .iter()
                    .any(|sel| path_matches(sel, changed))
        });
        let caller_changed = caller_files.iter().any(|caller| {
            self.caller_files
                .iter()
                .any(|known| path_matches(known, caller))
                || self
                    .watched_globs
                    .iter()
                    .any(|glob| glob_matches(glob, caller))
        });
        if !dep_changed && !caller_changed {
            return DeltaMatch::None;
        }
        if self.ir_revision != 0
            && ir_revision != 0
            && ir_revision <= self.ir_revision
            && now_ns.saturating_sub(self.verified_ns) >= RACY_NS
        {
            return DeltaMatch::None;
        }
        DeltaMatch::Dependency
    }
}

/// A trace nothing was compiled from is dropped after this many days.
pub const TRACE_DAYS: i64 = 30;
/// A session keeps this many traces nothing was compiled from.
pub const TRACES_PER_SESSION: usize = 10;
/// Maximum number of capsules retained per project before LRU eviction runs.
pub const CAPSULES_PER_PROJECT: usize = 128;

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
            engine_digest TEXT NOT NULL DEFAULT '', eval_arm TEXT, runs_reset_rowid INTEGER NOT NULL DEFAULT 0,
            dependency_manifest TEXT NOT NULL DEFAULT '{}', capsule_kind TEXT NOT NULL DEFAULT 'skill',
            tool_chain_tree_sha TEXT NOT NULL DEFAULT '', reason TEXT,
            needs_trace INTEGER NOT NULL DEFAULT 0, needs_trace_after_rowid INTEGER NOT NULL DEFAULT 0,
            is_read_only INTEGER, git_common_dir TEXT NOT NULL DEFAULT '');
        CREATE INDEX IF NOT EXISTS skillcache_capsules_skill ON skillcache_capsules(project_id,skill_path,status);
        CREATE TABLE IF NOT EXISTS skillcache_pending_requalify(
            capsule_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            prior_status TEXT NOT NULL DEFAULT 'active',
            reason TEXT,
            attempts INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS skillcache_pending_requalify_order
            ON skillcache_pending_requalify(created_at);
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
        CREATE INDEX IF NOT EXISTS skillcache_runs_capsule ON skillcache_runs(capsule_id,created_at);
        CREATE TABLE IF NOT EXISTS skillcache_templates(
            template_id TEXT PRIMARY KEY,
            edition TEXT NOT NULL,
            skill_name TEXT NOT NULL,
            skill_tree_sha256 TEXT NOT NULL,
            skill_semantic_sha TEXT NOT NULL DEFAULT '',
            form TEXT NOT NULL CHECK(form IN ('hybrid','textual')),
            mu_name TEXT NOT NULL,
            mu_description TEXT NOT NULL,
            capsule_json TEXT NOT NULL,
            files_json TEXT NOT NULL,
            markdown TEXT,
            examples_json TEXT,
            portability_sha256 TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(edition, skill_name, skill_tree_sha256));
        CREATE INDEX IF NOT EXISTS skillcache_templates_lookup
            ON skillcache_templates(edition, skill_name, skill_tree_sha256);",
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
        (
            "skillcache_capsules",
            "dependency_manifest",
            "TEXT NOT NULL DEFAULT '{}'",
        ),
        (
            "skillcache_capsules",
            "capsule_kind",
            "TEXT NOT NULL DEFAULT 'skill'",
        ),
        (
            "skillcache_capsules",
            "tool_chain_tree_sha",
            "TEXT NOT NULL DEFAULT ''",
        ),
        (
            "skillcache_capsules",
            "needs_trace",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "skillcache_capsules",
            "needs_trace_after_rowid",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("skillcache_capsules", "is_read_only", "INTEGER"),
        (
            "skillcache_capsules",
            "git_common_dir",
            "TEXT NOT NULL DEFAULT ''",
        ),
        ("skillcache_compiles", "eval_arm", "TEXT"),
        (
            "skillcache_templates",
            "skill_semantic_sha",
            "TEXT NOT NULL DEFAULT ''",
        ),
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
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS skillcache_templates_semantic
             ON skillcache_templates(edition, skill_name, skill_semantic_sha);
         CREATE INDEX IF NOT EXISTS skillcache_capsules_common_dir
             ON skillcache_capsules(git_common_dir, status);",
    )?;
    let needs_retired_check = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='skillcache_capsules'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some_and(|sql| !sql.contains("'retired'"));
    if needs_retired_check {
        // One transaction: an interruption between dropping the old table and
        // renaming the new one must not leave the project without its
        // capsules.
        let rebuild = db.unchecked_transaction()?;
        rebuild.execute_batch(&format!(
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
                 engine_digest TEXT NOT NULL DEFAULT '', eval_arm TEXT, runs_reset_rowid INTEGER NOT NULL DEFAULT 0,
                 dependency_manifest TEXT NOT NULL DEFAULT '{{}}', capsule_kind TEXT NOT NULL DEFAULT 'skill',
                 tool_chain_tree_sha TEXT NOT NULL DEFAULT '', reason TEXT,
                 needs_trace INTEGER NOT NULL DEFAULT 0, needs_trace_after_rowid INTEGER NOT NULL DEFAULT 0,
                 is_read_only INTEGER, git_common_dir TEXT NOT NULL DEFAULT '');
             INSERT INTO skillcache_capsules_new({CAPSULE_COLUMNS},runs_reset_rowid)
             SELECT {CAPSULE_COLUMNS},runs_reset_rowid FROM skillcache_capsules;
             DROP TABLE skillcache_capsules;
             ALTER TABLE skillcache_capsules_new RENAME TO skillcache_capsules;
             CREATE INDEX IF NOT EXISTS skillcache_capsules_skill ON skillcache_capsules(project_id,skill_path,status);
             CREATE INDEX IF NOT EXISTS skillcache_capsules_common_dir ON skillcache_capsules(git_common_dir,status);"
        ))?;
        rebuild.commit()?;
    }
    Ok(())
}

fn table_exists(db: &Connection, name: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |row| row.get::<_, i64>(0),
    )? != 0)
}

fn session_pinned_capsule_ids(db: &Connection) -> Result<BTreeSet<String>> {
    if !table_exists(db, "sessions")? {
        return Ok(BTreeSet::new());
    }
    let mut query = db.prepare(
        "SELECT DISTINCT json_extract(skillcache, '$.capsule_id')
         FROM sessions
         WHERE skillcache IS NOT NULL AND json_extract(skillcache, '$.capsule_id') IS NOT NULL",
    )?;
    let rows = query.query_map([], |row| row.get::<_, Option<String>>(0))?;
    let mut pinned = BTreeSet::new();
    for row in rows {
        if let Some(id) = row?
            && !id.is_empty()
        {
            pinned.insert(id);
        }
    }
    Ok(pinned)
}

fn is_plain_id_component(segment: &str) -> bool {
    !segment.is_empty() && !matches!(segment, "." | "..") && !segment.contains(['/', '\\', '\0'])
}

pub(crate) fn state_dir_from_connection(db: &Connection) -> Option<PathBuf> {
    let raw = db.path().filter(|p| !p.is_empty() && *p != ":memory:")?;
    Path::new(raw).parent().map(Path::to_path_buf)
}

pub(crate) fn cleanup_deleted_capsule_artifacts(
    state_dir: Option<&Path>,
    deleted: &[(String, String, String)],
) {
    let Some(state) = state_dir else {
        return;
    };
    let capsules_root = state.join("skillcache/capsules");
    let qual_root = state.join("skillcache/qualification");
    for (project_id, capsule_id, artifact_dir) in deleted {
        if is_plain_id_component(project_id) && is_plain_id_component(capsule_id) {
            let cap_dir = capsules_root.join(project_id).join(capsule_id);
            if cap_dir.exists() {
                infra_skillcache::package::release(&cap_dir);
                let _ = std::fs::remove_dir_all(&cap_dir);
            }
        }
        if !artifact_dir.is_empty() {
            let art_path = Path::new(artifact_dir);
            if art_path.starts_with(&capsules_root) && art_path.exists() {
                let target = art_path
                    .parent()
                    .filter(|p| p.starts_with(&capsules_root) && *p != capsules_root)
                    .unwrap_or(art_path);
                infra_skillcache::package::release(target);
                let _ = std::fs::remove_dir_all(target);
            }
        }
        if is_plain_id_component(capsule_id) {
            let qual_dir = qual_root.join(capsule_id);
            if qual_dir.exists() {
                infra_skillcache::package::release(&qual_dir);
                let _ = std::fs::remove_dir_all(&qual_dir);
            }
        }
    }
}

struct PrunedCapsuleMeta {
    capsule_id: String,
    project_id: String,
    kind: String,
    skill_name: String,
    skill_path: String,
    mu_name: String,
    eval_arm: Option<String>,
    artifact_dir: String,
}

fn capsule_superseded_by_active(
    candidate: &PrunedCapsuleMeta,
    active: &[PrunedCapsuleMeta],
) -> bool {
    active.iter().any(|act| {
        act.project_id == candidate.project_id
            && act.kind == candidate.kind
            && act.eval_arm == candidate.eval_arm
            && if candidate.kind == "tool_chain" {
                !candidate.mu_name.is_empty() && act.mu_name == candidate.mu_name
            } else {
                (!candidate.skill_path.is_empty() && act.skill_path == candidate.skill_path)
                    || (canonical_mirror_skill(&act.skill_path).is_some()
                        && canonical_mirror_skill(&act.skill_path)
                            == canonical_mirror_skill(&candidate.skill_path)
                        && (candidate.skill_name.is_empty()
                            || act.skill_name == candidate.skill_name))
            }
    })
}

fn enforce_project_capsule_cap_collect(
    db: &Connection,
    project_id: &str,
    max_capsules: usize,
    pinned: &BTreeSet<String>,
    deleted: &mut Vec<(String, String, String)>,
) -> Result<usize> {
    let mut query = db.prepare(
        "SELECT capsule_id, status, artifact_dir
         FROM skillcache_capsules
         WHERE project_id=?1
         ORDER BY COALESCE(last_used_at, updated_at, created_at) ASC, rowid ASC",
    )?;
    let rows = query
        .query_map([project_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if rows.len() <= max_capsules {
        return Ok(0);
    }
    let excess = rows.len() - max_capsules;
    let mut picked: Vec<(String, String)> = Vec::with_capacity(excess);
    // Evict inactive (`failed`/`retired`/`stale`/`needs_recompile`) capsules first,
    // preserving pinned and active capsules whenever possible.
    for (id, status, artifact_dir) in &rows {
        if picked.len() >= excess {
            break;
        }
        if status != "compiling" && status != "active" && !pinned.contains(id) {
            picked.push((id.clone(), artifact_dir.clone()));
        }
    }
    if picked.len() < excess {
        for (id, status, artifact_dir) in &rows {
            if picked.len() >= excess {
                break;
            }
            if status == "active" && !pinned.contains(id) {
                picked.push((id.clone(), artifact_dir.clone()));
            }
        }
    }
    let mut removed = 0usize;
    for (id, artifact_dir) in picked {
        if delete_capsule(db, &id)? {
            deleted.push((project_id.to_owned(), id, artifact_dir));
            removed += 1;
        }
    }
    Ok(removed)
}

/// Enforces `max_capsules` on `project_id` using `last_used_at` LRU eviction
/// (preserving session-pinned, compiling, and active capsules before older ones).
pub(crate) fn enforce_project_capsule_cap(
    db: &Connection,
    project_id: &str,
    max_capsules: usize,
) -> Result<usize> {
    let pinned = session_pinned_capsule_ids(db)?;
    let mut deleted = Vec::new();
    let count =
        enforce_project_capsule_cap_collect(db, project_id, max_capsules, &pinned, &mut deleted)?;
    if !deleted.is_empty() {
        let state_dir = state_dir_from_connection(db);
        cleanup_deleted_capsule_artifacts(state_dir.as_deref(), &deleted);
    }
    Ok(count)
}

/// Prunes expired traces (`created_at < cutoff`) and per-session excess traces
/// no longer pinned by any non-failed, non-retired capsule.
pub(crate) fn prune_unpinned_traces(db: &Connection, cutoff: &str) -> Result<usize> {
    let mut removed = db.execute(
        "DELETE FROM skillcache_traces WHERE created_at<?1
           AND generation NOT IN (
               SELECT source_generation FROM skillcache_capsules
               WHERE source_generation != '' AND status NOT IN ('failed', 'retired')
           )",
        [cutoff],
    )?;
    let over_cap_sessions = {
        let mut q = db.prepare(
            "SELECT session_id FROM skillcache_traces GROUP BY session_id HAVING COUNT(*) > ?1",
        )?;
        q.query_map([TRACES_PER_SESSION as i64], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for sid in over_cap_sessions {
        removed += db.execute(
            "DELETE FROM skillcache_traces WHERE session_id=?1
               AND generation NOT IN (
                   SELECT source_generation FROM skillcache_capsules
                   WHERE source_generation != '' AND status NOT IN ('failed', 'retired')
               )
               AND generation NOT IN (
                   SELECT generation FROM skillcache_traces
                   WHERE session_id=?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2
               )",
            params![sid, TRACES_PER_SESSION as i64],
        )?;
    }
    Ok(removed)
}

/// Runs once when the store opens.
pub(crate) fn recover(db: &Connection) -> Result<()> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(TRACE_DAYS))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
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
    let pinned = session_pinned_capsule_ids(db)?;
    let active_capsules = {
        let mut q = db.prepare(
            "SELECT capsule_id,project_id,capsule_kind,skill_name,skill_path,mu_name,eval_arm,artifact_dir
             FROM skillcache_capsules WHERE status='active'",
        )?;
        q.query_map([], |row| {
            Ok(PrunedCapsuleMeta {
                capsule_id: row.get(0)?,
                project_id: row.get(1)?,
                kind: row.get(2)?,
                skill_name: row.get(3)?,
                skill_path: row.get(4)?,
                mu_name: row.get(5)?,
                eval_arm: row.get(6)?,
                artifact_dir: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let old_non_active = {
        let mut q = db.prepare(
            "SELECT capsule_id,project_id,capsule_kind,skill_name,skill_path,mu_name,eval_arm,artifact_dir,status
             FROM skillcache_capsules
             WHERE updated_at<?1 AND status IN ('failed','retired','stale','needs_recompile')",
        )?;
        q.query_map([&cutoff], |row| {
            Ok((
                PrunedCapsuleMeta {
                    capsule_id: row.get(0)?,
                    project_id: row.get(1)?,
                    kind: row.get(2)?,
                    skill_name: row.get(3)?,
                    skill_path: row.get(4)?,
                    mu_name: row.get(5)?,
                    eval_arm: row.get(6)?,
                    artifact_dir: row.get(7)?,
                },
                row.get::<_, String>(8)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let mut deleted = Vec::new();
    for (meta, status) in old_non_active {
        let should_prune = match status.as_str() {
            "failed" => true,
            "retired" => !pinned.contains(&meta.capsule_id),
            "stale" | "needs_recompile" => {
                !pinned.contains(&meta.capsule_id)
                    && capsule_superseded_by_active(&meta, &active_capsules)
            }
            _ => false,
        };
        if should_prune && delete_capsule(db, &meta.capsule_id)? {
            deleted.push((meta.project_id, meta.capsule_id, meta.artifact_dir));
        }
    }
    db.execute(
        "DELETE FROM skillcache_compiles
         WHERE status IN ('failed','interrupted')
           AND COALESCE(finished_at, started_at) < ?1
           AND (capsule_id IS NULL OR capsule_id NOT IN (
               SELECT capsule_id FROM skillcache_capsules WHERE status IN ('active','compiling')
           ))",
        [&cutoff],
    )?;
    let over_cap_projects = {
        let mut q = db.prepare(
            "SELECT project_id FROM skillcache_capsules GROUP BY project_id HAVING COUNT(*)>?1",
        )?;
        q.query_map([CAPSULES_PER_PROJECT as i64], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for project_id in over_cap_projects {
        enforce_project_capsule_cap_collect(
            db,
            &project_id,
            CAPSULES_PER_PROJECT,
            &pinned,
            &mut deleted,
        )?;
    }
    prune_unpinned_traces(db, &cutoff)?;
    if let Some(state_dir) = state_dir_from_connection(db) {
        cleanup_deleted_capsule_artifacts(Some(&state_dir), &deleted);
        if let Ok(kept) = trace_generations(db) {
            super::snapshot::collect(&state_dir, &kept);
        }
    }
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
    pub usage: &'a Value,
    pub redaction: &'a Value,
}

/// Stores one turn's trace and drops the session's oldest beyond its limit.
/// Returns the digest of the stored steps.
pub(crate) fn save_trace(db: &Connection, row: &TraceRow<'_>) -> Result<String> {
    let steps = row.steps.to_string();
    let digest = store::sha(steps.as_bytes());
    let usage = (!row.usage.is_null()).then(|| row.usage.to_string());
    db.execute(
        "INSERT INTO skillcache_traces(generation,session_id,provider,model,request,skill_refs,steps,usage,redaction,trace_sha256,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(generation) DO UPDATE SET request=excluded.request,skill_refs=excluded.skill_refs,
           steps=excluded.steps,usage=excluded.usage,redaction=excluded.redaction,trace_sha256=excluded.trace_sha256,created_at=excluded.created_at",
        params![
            row.generation,
            row.session_id,
            row.provider,
            row.model,
            row.request,
            row.skill_refs.to_string(),
            steps,
            usage,
            row.redaction.to_string(),
            digest,
            store::now()
        ],
    )?;
    db.execute(
        "DELETE FROM skillcache_traces WHERE session_id=?1
           AND generation NOT IN (
               SELECT source_generation FROM skillcache_capsules
               WHERE source_generation != '' AND status NOT IN ('failed', 'retired')
           )
           AND generation NOT IN
           (SELECT generation FROM skillcache_traces WHERE session_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT ?2)",
        params![row.session_id, TRACES_PER_SESSION as i64],
    )?;
    Ok(digest)
}

/// The launches whose traces are stored or currently in-flight: what keeps a snapshot alive.
pub(crate) fn trace_generations(db: &Connection) -> Result<Vec<String>> {
    let mut query = db.prepare("SELECT generation FROM skillcache_traces")?;
    let rows = query.query_map([], |row| row.get::<_, String>(0))?;
    let mut generations: BTreeSet<String> =
        rows.collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let has_active_launch_tables: bool = db
        .query_row(
            "SELECT COUNT(*)=2 FROM sqlite_master WHERE type='table' AND name IN ('launches','sessions')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if has_active_launch_tables {
        let mut active = db.prepare(
            "SELECT l.id FROM launches l
             JOIN sessions s ON s.id = l.session_id
             WHERE l.status IN ('queued','running')
               AND s.status IN ('queued','running')",
        )?;
        for generation in active
            .query_map([], |row| row.get::<_, String>(0))?
            .flatten()
        {
            generations.insert(generation);
        }
    }
    Ok(generations.into_iter().collect())
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

const CAPSULE_COLUMNS: &str = "capsule_id,project_id,skill_name,skill_path,mu_name,mu_description,form,status,revision,capsule_sha256,tree_sha256,skill_files,env_fingerprint,schema_version,compile_rules_version,source_session_id,source_generation,compile_id,artifact_dir,created_at,updated_at,last_used_at,package_sha256,capabilities,engine_digest,eval_arm,reason,dependency_manifest,capsule_kind,tool_chain_tree_sha,needs_trace,needs_trace_after_rowid,is_read_only,git_common_dir";

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
        "dependency_manifest": decoded(row.get(27)?),
        "kind": row.get::<_, String>(28)?,
        "tool_chain_tree_sha": row.get::<_, String>(29)?,
        "needs_trace": row.get::<_, i64>(30)? != 0,
        "needs_trace_after_rowid": row.get::<_, i64>(31)?,
        "is_read_only": row.get::<_, Option<i64>>(32)?.map(|v| v != 0),
        "git_common_dir": row.get::<_, String>(33)?,
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
           needs_trace=0,needs_trace_after_rowid=0,
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

/// Marks a deopted capsule as needing a fresh cold observed trace before recompiling.
pub(crate) fn mark_capsule_needs_trace(db: &Connection, capsule_id: &str) -> Result<()> {
    let changed = db.execute(
        "UPDATE skillcache_capsules
         SET needs_trace=1,
             needs_trace_after_rowid=CASE
                 WHEN needs_trace=0 THEN COALESCE((SELECT MAX(rowid) FROM skillcache_traces),0)
                 ELSE needs_trace_after_rowid
             END,
             updated_at=?2
         WHERE capsule_id=?1",
        params![capsule_id, store::now()],
    )?;
    if changed == 0 {
        return Err(HarnessError::missing("Capsule not found."));
    }
    Ok(())
}

/// Clears the `needs_trace` flag once a fresh trace has been queued or requalified.
pub(crate) fn clear_capsule_needs_trace(db: &Connection, capsule_id: &str) -> Result<()> {
    db.execute(
        "UPDATE skillcache_capsules SET needs_trace=0,needs_trace_after_rowid=0 WHERE capsule_id=?1",
        [capsule_id],
    )?;
    Ok(())
}

/// Lists `needs_recompile` capsules in `project_id` waiting for a fresh observed trace.
pub(crate) fn capsules_needing_trace(db: &Connection, project_id: &str) -> Result<Vec<Value>> {
    let mut query = db.prepare(&format!(
        "SELECT {CAPSULE_COLUMNS} FROM skillcache_capsules
         WHERE project_id=?1 AND status='needs_recompile' AND needs_trace != 0"
    ))?;
    let rows = query.query_map([project_id], capsule_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Finds a freshly observed trace recorded strictly after `capsule` was flagged `needs_trace`.
pub(crate) fn find_fresh_trace_after(
    db: &Connection,
    project_id: &str,
    capsule: &Value,
) -> Result<Option<Value>> {
    let after_rowid = capsule["needs_trace_after_rowid"].as_i64().unwrap_or(0);
    let old_gen = capsule["source_generation"].as_str().unwrap_or("");
    let is_tool_chain = capsule["kind"] == "tool_chain";
    let skill_path = capsule["skill_path"].as_str().unwrap_or("");
    let skill_name = capsule["skill_name"].as_str().unwrap_or("");
    let mut query = db.prepare(
        "SELECT t.generation,t.session_id,t.provider,t.model,t.request,t.skill_refs,t.steps,t.usage,t.redaction,t.trace_sha256,t.created_at
         FROM skillcache_traces t
         JOIN sessions s ON s.id=t.session_id
         WHERE s.project_id=?1 AND t.rowid>?2 AND t.generation!=?3
         ORDER BY t.rowid DESC
         LIMIT 64",
    )?;
    let rows = query.query_map(params![project_id, after_rowid, old_gen], trace_from_row)?;
    for row in rows {
        let candidate = row?;
        if is_tool_chain {
            if candidate["redaction"]["trace_kind"] == "tool_chain" {
                return Ok(Some(candidate));
            }
            continue;
        }
        let matches = candidate["skill_refs"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|sref| {
                let sp = sref["path"].as_str().unwrap_or("");
                let sn = sref["name"].as_str().unwrap_or("");
                sp == skill_path
                    || (!skill_name.is_empty() && sn == skill_name)
                    || (canonical_mirror_skill(sp).is_some()
                        && canonical_mirror_skill(sp) == canonical_mirror_skill(skill_path))
            });
        if matches {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
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

/// A project's capsules, newest first, optionally narrowed. The capsules of
/// an evaluation arm are listed only when asked for: they are not what a
/// person manages, but they must be findable to be deleted.
pub(crate) fn capsules(
    db: &Connection,
    project_id: &str,
    skill: Option<&str>,
    status: Option<&str>,
    form: Option<&str>,
    include_retired: bool,
    include_eval: bool,
) -> Result<Vec<Value>> {
    let mut query = db.prepare(&format!(
        "SELECT {CAPSULE_COLUMNS} FROM skillcache_capsules
         WHERE project_id=?1 AND (?6 != 0 OR eval_arm IS NULL)
           AND (?2 IS NULL OR skill_name=?2 OR skill_path=?2)
           AND (?3 IS NULL OR status=?3)
           AND (?3 IS NOT NULL OR ?5 != 0 OR status != 'retired')
           AND (?4 IS NULL OR form=?4)
         ORDER BY created_at DESC,rowid DESC"
    ))?;
    let rows = query.query_map(
        params![
            project_id,
            skill,
            status,
            form,
            i64::from(include_retired),
            i64::from(include_eval)
        ],
        capsule_row,
    )?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// The active capsules compiled from the skill at `skill_path` (excluding
/// evaluation-only ablation capsules).
#[cfg(test)]
pub(crate) fn active_capsules(
    db: &Connection,
    project_id: &str,
    skill_path: &str,
) -> Result<Vec<Value>> {
    active_capsules_for_eval(db, project_id, skill_path, None)
}

/// Records the dependency manifest bound to a compiled capsule.
pub(crate) fn set_dependency_manifest(
    db: &Connection,
    capsule_id: &str,
    manifest: &ArtifactDependencyManifest,
) -> Result<()> {
    let mut stored_manifest = manifest.clone();
    if let Some(cap) = capsule(db, capsule_id)?
        && let Some(generation_id) = cap["source_generation"].as_str().filter(|s| !s.is_empty())
        && let Some(source_trace) = trace(db, generation_id)?
        && let Some(obj) = source_trace["redaction"]["tool_chain_manifest_hashes"].as_object()
    {
        for (path, hash) in obj {
            if let Some(hash_str) = hash.as_str() {
                stored_manifest
                    .path_hashes
                    .entry(path.clone())
                    .or_insert_with(|| hash_str.to_owned());
            }
        }
    }
    let changed = db.execute(
        "UPDATE skillcache_capsules SET dependency_manifest=?2,updated_at=?3 WHERE capsule_id=?1",
        params![
            capsule_id,
            stored_manifest.to_value().to_string(),
            store::now()
        ],
    )?;
    if changed == 0 {
        return Err(HarnessError::missing("Capsule not found."));
    }
    Ok(())
}

/// Records kind (`skill` | `tool_chain` | `template`) and workspace tree digest for cross-worktree sharing.
pub(crate) fn set_capsule_kind_meta(
    db: &Connection,
    capsule_id: &str,
    kind: &str,
    tool_chain_tree_sha: &str,
) -> Result<()> {
    let changed = db.execute(
        "UPDATE skillcache_capsules SET capsule_kind=?2,tool_chain_tree_sha=?3,updated_at=?4 WHERE capsule_id=?1",
        params![capsule_id, kind, tool_chain_tree_sha, store::now()],
    )?;
    if changed == 0 {
        return Err(HarnessError::missing("Capsule not found."));
    }
    Ok(())
}

/// Persists whether every program in the compiled capsule is declared read-only.
pub(crate) fn set_capsule_read_only(
    db: &Connection,
    capsule_id: &str,
    read_only: bool,
) -> Result<()> {
    let changed = db.execute(
        "UPDATE skillcache_capsules SET is_read_only=?2 WHERE capsule_id=?1",
        params![capsule_id, i64::from(read_only)],
    )?;
    if changed == 0 {
        return Err(HarnessError::missing("Capsule not found."));
    }
    Ok(())
}

/// Returns the attached accelerator edition recorded in `project_accelerators`, if any.
pub(crate) fn project_accelerator_edition(
    db: &Connection,
    project_id: &str,
) -> Result<Option<String>> {
    match db
        .query_row(
            "SELECT edition FROM project_accelerators WHERE project_id=?1",
            [project_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
    {
        Ok(found) => Ok(found.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())),
        Err(rusqlite::Error::SqliteFailure(_, Some(msg))) if msg.contains("no such table") => {
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

/// Resolves the canonical Git common directory for `workspace`, supporting main
/// checkouts, linked worktrees, and subdirectories inside a repository.
pub(crate) fn canonical_git_common_dir(workspace: &Path) -> Option<PathBuf> {
    let ws_canon = infra_core::fs::canonicalize(workspace).ok()?;
    for ancestor in ws_canon.ancestors() {
        let dot_git = ancestor.join(".git");
        let Ok(meta) = std::fs::symlink_metadata(&dot_git) else {
            continue;
        };
        if meta.is_dir() {
            return infra_core::fs::canonicalize(&dot_git).ok();
        }
        if meta.is_file()
            && let Ok(raw) = std::fs::read_to_string(&dot_git)
            && let Some(rel) = raw.trim().strip_prefix("gitdir:").map(str::trim)
        {
            let git_dir = if Path::new(rel).is_absolute() {
                PathBuf::from(rel)
            } else {
                ancestor.join(rel)
            };
            let common = if let Ok(common_raw) = std::fs::read_to_string(git_dir.join("commondir"))
            {
                let trimmed = common_raw.trim();
                let p = Path::new(trimmed);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    git_dir.join(p)
                }
            } else {
                git_dir
            };
            return infra_core::fs::canonicalize(&common).ok();
        }
    }
    infra_core::git::GitFrame::discover(&ws_canon)
        .ok()
        .flatten()
        .and_then(|frame| frame.common_dir().ok())
        .and_then(|p| infra_core::fs::canonicalize(&p).ok())
}

/// Returns other registered `project_id`s that share the same canonical Git
/// common directory (or canonical workspace root) as `workspace`.
pub(crate) fn sibling_worktree_projects(
    db: &Connection,
    project_id: &str,
    workspace: &Path,
) -> Result<Vec<String>> {
    if !table_exists(db, "registered_projects")? {
        return Ok(Vec::new());
    }
    let target_common = canonical_git_common_dir(workspace);
    let target_root = infra_core::fs::canonicalize(workspace).ok();
    if target_common.is_none() && target_root.is_none() {
        return Ok(Vec::new());
    }
    let mut query = db.prepare("SELECT id, path FROM registered_projects WHERE id!=?1")?;
    let rows = query.query_map([project_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut siblings = Vec::new();
    for row in rows {
        let (other_id, other_path) = row?;
        let other = Path::new(&other_path);
        let common_matches = target_common.is_some()
            && canonical_git_common_dir(other).as_ref() == target_common.as_ref();
        let root_matches = target_root.is_some()
            && infra_core::fs::canonicalize(other).ok().as_ref() == target_root.as_ref();
        if common_matches || root_matches {
            siblings.push(other_id);
        }
    }
    Ok(siblings)
}

/// Persists the canonical Git common directory for cross-worktree capsule sharing.
pub(crate) fn set_capsule_git_common_dir(
    db: &Connection,
    capsule_id: &str,
    git_common_dir: Option<&Path>,
) -> Result<()> {
    let value = git_common_dir
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let changed = db.execute(
        "UPDATE skillcache_capsules SET git_common_dir=?2 WHERE capsule_id=?1",
        params![capsule_id, value],
    )?;
    if changed == 0 {
        return Err(HarnessError::missing("Capsule not found."));
    }
    Ok(())
}

/// All active `ToolChain` capsules in `project_id` matching `eval_arm`, filtered in-memory
/// by dependency manifest / touched-path hashes.
pub(crate) fn active_toolchain_capsules_for_project(
    db: &Connection,
    project_id: &str,
    eval_arm: Option<&str>,
) -> Result<Vec<Value>> {
    let mut query = db.prepare(&format!(
        "SELECT {CAPSULE_COLUMNS} FROM skillcache_capsules
         WHERE project_id=?1 AND capsule_kind='tool_chain' AND status='active'
           AND ((?2 IS NULL AND eval_arm IS NULL) OR eval_arm=?2)
         ORDER BY created_at DESC,rowid DESC"
    ))?;
    let rows = query.query_map(params![project_id, eval_arm], capsule_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Active `ToolChain` capsules in `project_id` first, followed by sibling git
/// worktrees sharing canonical `git_common_dir`.
pub(crate) fn active_toolchain_capsules_for_worktrees(
    db: &Connection,
    project_id: &str,
    sibling_ids: &[String],
    common_dir: Option<&Path>,
    eval_arm: Option<&str>,
) -> Result<Vec<Value>> {
    let mut out = active_toolchain_capsules_for_project(db, project_id, eval_arm)?;
    let common_str = common_dir
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if sibling_ids.is_empty() && common_str.is_empty() {
        return Ok(out);
    }
    let sibling_set: BTreeSet<&str> = sibling_ids.iter().map(String::as_str).collect();
    let mut query = db.prepare(&format!(
        "SELECT {CAPSULE_COLUMNS} FROM skillcache_capsules
         WHERE project_id!=?1 AND capsule_kind='tool_chain' AND status='active'
           AND ((?2 IS NULL AND eval_arm IS NULL) OR eval_arm=?2)
         ORDER BY created_at DESC,rowid DESC"
    ))?;
    let rows = query.query_map(params![project_id, eval_arm], capsule_row)?;
    for row in rows {
        let cap = row?;
        let cap_project = cap["project_id"].as_str().unwrap_or("");
        let cap_common = cap["git_common_dir"].as_str().unwrap_or("");
        if sibling_set.contains(cap_project) || (!common_str.is_empty() && cap_common == common_str)
        {
            out.push(cap);
        }
    }
    Ok(out)
}

const TEMPLATE_COLUMNS: &str = "template_id,edition,skill_name,skill_tree_sha256,skill_semantic_sha,form,mu_name,mu_description,capsule_json,files_json,markdown,examples_json,portability_sha256,created_at,updated_at";

/// Stores or updates a portable edition template indexed by `(edition, skill_name, skill_tree_sha256)`.
#[cfg(test)]
pub(crate) fn upsert_template(
    db: &Connection,
    edition: &str,
    skill_name: &str,
    skill_tree_sha256: &str,
    candidate: &Candidate,
) -> Result<String> {
    upsert_template_with_semantic(db, edition, skill_name, skill_tree_sha256, "", candidate)
}

/// Stores or updates a portable edition template along with its normalized `skill_semantic_sha`.
pub(crate) fn upsert_template_with_semantic(
    db: &Connection,
    edition: &str,
    skill_name: &str,
    skill_tree_sha256: &str,
    skill_semantic_sha: &str,
    candidate: &Candidate,
) -> Result<String> {
    let files_json = Value::Array(
        candidate
            .files
            .iter()
            .filter_map(|(path, bytes)| {
                std::str::from_utf8(bytes)
                    .ok()
                    .map(|content| json!({"path": path, "content": content}))
            })
            .collect(),
    );
    let examples_json = candidate
        .examples
        .as_ref()
        .and_then(|e| serde_json::to_string(&e.to_value()).ok());
    let capsule_json = String::from_utf8_lossy(&candidate.capsule_bytes).into_owned();
    let portability_sha256 =
        infra_core::query::sha256(format!("{capsule_json}\n{files_json}").as_bytes());
    let template_id = format!("tpl-{}", &portability_sha256[..16]);
    let now = store::now();
    db.execute(
        "INSERT INTO skillcache_templates(
            template_id,edition,skill_name,skill_tree_sha256,skill_semantic_sha,form,mu_name,mu_description,
            capsule_json,files_json,markdown,examples_json,portability_sha256,created_at,updated_at
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14)
         ON CONFLICT(edition,skill_name,skill_tree_sha256) DO UPDATE SET
            skill_semantic_sha=CASE
                WHEN excluded.skill_semantic_sha != '' THEN excluded.skill_semantic_sha
                ELSE skillcache_templates.skill_semantic_sha
            END,
            form=excluded.form,mu_name=excluded.mu_name,mu_description=excluded.mu_description,
            capsule_json=excluded.capsule_json,files_json=excluded.files_json,markdown=excluded.markdown,
            examples_json=excluded.examples_json,portability_sha256=excluded.portability_sha256,updated_at=excluded.updated_at",
        params![
            template_id,
            edition,
            skill_name,
            skill_tree_sha256,
            skill_semantic_sha,
            candidate.capsule.form.as_str(),
            candidate.capsule.mu.name,
            candidate.capsule.mu.description,
            capsule_json,
            files_json.to_string(),
            candidate.markdown,
            examples_json,
            portability_sha256,
            now
        ],
    )?;
    Ok(template_id)
}

fn template_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "template_id": row.get::<_, String>(0)?,
        "edition": row.get::<_, String>(1)?,
        "skill_name": row.get::<_, String>(2)?,
        "skill_tree_sha256": row.get::<_, String>(3)?,
        "skill_semantic_sha": row.get::<_, String>(4)?,
        "form": row.get::<_, String>(5)?,
        "mu": {"name": row.get::<_, String>(6)?, "description": row.get::<_, String>(7)?},
        "capsule_json": row.get::<_, String>(8)?,
        "files_json": decoded(row.get(9)?),
        "markdown": row.get::<_, Option<String>>(10)?,
        "examples_json": row.get::<_, Option<String>>(11)?,
        "portability_sha256": row.get::<_, String>(12)?,
        "created_at": row.get::<_, String>(13)?,
        "updated_at": row.get::<_, String>(14)?,
    }))
}

/// Looks up portable edition templates matching `(edition, skill_name, skill_tree_sha256)`.
#[cfg_attr(not(test), expect(dead_code))]
pub(crate) fn find_templates(
    db: &Connection,
    edition: &str,
    skill_name: &str,
    skill_tree_sha256: &str,
) -> Result<Vec<Value>> {
    find_templates_with_semantic(db, edition, skill_name, skill_tree_sha256, "")
}

/// Looks up portable edition templates matching exact `skill_tree_sha256` first,
/// then normalized `skill_semantic_sha` when non-empty.
pub(crate) fn find_templates_with_semantic(
    db: &Connection,
    edition: &str,
    skill_name: &str,
    skill_tree_sha256: &str,
    skill_semantic_sha: &str,
) -> Result<Vec<Value>> {
    let mut query = db.prepare(&format!(
        "SELECT {TEMPLATE_COLUMNS}
         FROM skillcache_templates
         WHERE edition=?1 AND skill_name=?2
           AND (skill_tree_sha256=?3 OR (?4 != '' AND skill_semantic_sha=?4))
         ORDER BY (skill_tree_sha256=?3) DESC, updated_at DESC"
    ))?;
    let rows = query.query_map(
        params![edition, skill_name, skill_tree_sha256, skill_semantic_sha],
        template_row,
    )?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Lists all portable templates indexed for `edition`.
pub(crate) fn edition_templates(db: &Connection, edition: &str) -> Result<Vec<Value>> {
    let mut query = db.prepare(&format!(
        "SELECT {TEMPLATE_COLUMNS}
         FROM skillcache_templates
         WHERE edition=?1
         ORDER BY updated_at DESC"
    ))?;
    let rows = query.query_map([edition], template_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Lists all active/needs_recompile capsules of `project_id` for IR delta matching.
pub(crate) fn active_or_needs_recompile_capsules(
    db: &Connection,
    project_id: &str,
) -> Result<Vec<Value>> {
    let mut query = db.prepare(&format!(
        "SELECT {CAPSULE_COLUMNS} FROM skillcache_capsules
         WHERE project_id=?1 AND status IN ('active','needs_recompile')"
    ))?;
    let rows = query.query_map([project_id], capsule_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Canonical `(scope, relative_subpath)` for tool-mirror skills across
/// `.claude/skills/`, `.agents/skills/`, and `.cursor/skills/` (workspace or `~/`).
pub(crate) fn canonical_mirror_skill(skill_path: &str) -> Option<(&'static str, String)> {
    let normalized = normalize_rel_path(skill_path);
    let (scope, rest) = match normalized.strip_prefix("~/") {
        Some(tail) => ("~", tail),
        None => ("ws", normalized.as_str()),
    };
    for prefix in [".claude/skills/", ".agents/skills/", ".cursor/skills/"] {
        if let Some(sub) = rest.strip_prefix(prefix)
            && !sub.is_empty()
        {
            return Some((scope, sub.to_owned()));
        }
    }
    None
}

/// Whether `capsule` matches `skill_path` directly or via canonical tool-mirror
/// identity (`skill_name` + canonical subpath + `tree_sha256` content digest).
pub(crate) fn capsule_matches_skill_or_mirror(
    capsule: &Value,
    skill_path: &str,
    skill_name: &str,
    tree_sha256: &str,
) -> bool {
    let cap_path = capsule["skill_path"].as_str().unwrap_or("");
    if cap_path == skill_path {
        return true;
    }
    if tree_sha256.is_empty() || capsule["tree_sha256"].as_str() != Some(tree_sha256) {
        return false;
    }
    if !skill_name.is_empty() && capsule["skill_name"].as_str() != Some(skill_name) {
        return false;
    }
    matches!(
        (canonical_mirror_skill(cap_path), canonical_mirror_skill(skill_path)),
        (Some(a), Some(b)) if a == b
    )
}

/// The active capsules compiled for `eval_arm` (or normal non-eval capsules
/// when `eval_arm` is `None`).
#[cfg(test)]
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

/// Active capsules matching `skill_path` or an identical tool mirror
/// (`.claude`/`.agents`/`.cursor`) with matching canonical skill name and `tree_sha256`.
pub(crate) fn active_capsules_for_skill_or_mirror(
    db: &Connection,
    project_id: &str,
    skill_path: &str,
    skill_name: &str,
    tree_sha256: &str,
    eval_arm: Option<&str>,
) -> Result<Vec<Value>> {
    let mut query = db.prepare(&format!(
        "SELECT {CAPSULE_COLUMNS} FROM skillcache_capsules
         WHERE project_id=?1 AND status='active'
           AND ((?3 IS NULL AND eval_arm IS NULL) OR eval_arm=?3)
           AND (
               skill_path=?2
               OR (?4 != '' AND tree_sha256=?4 AND (?5 = '' OR skill_name=?5))
           )
         ORDER BY (skill_path=?2) DESC, created_at DESC, rowid DESC"
    ))?;
    let rows = query.query_map(
        params![project_id, skill_path, eval_arm, tree_sha256, skill_name],
        capsule_row,
    )?;
    let mut matched = Vec::new();
    for row in rows {
        let cap = row?;
        if capsule_matches_skill_or_mirror(&cap, skill_path, skill_name, tree_sha256) {
            matched.push(cap);
        }
    }
    Ok(matched)
}

/// Active capsules in `project_id` first, followed by verified mirror/skill
/// capsules in sibling git worktrees sharing canonical `git_common_dir` and
/// exact `tree_sha256`.
#[expect(
    clippy::too_many_arguments,
    reason = "Worktree skill lookup passes project, siblings, common_dir, mirror path, name, digest, and eval arm."
)]
pub(crate) fn active_capsules_for_skill_or_mirror_in_worktrees(
    db: &Connection,
    project_id: &str,
    sibling_ids: &[String],
    common_dir: Option<&Path>,
    skill_path: &str,
    skill_name: &str,
    tree_sha256: &str,
    eval_arm: Option<&str>,
) -> Result<Vec<Value>> {
    let mut matched = active_capsules_for_skill_or_mirror(
        db,
        project_id,
        skill_path,
        skill_name,
        tree_sha256,
        eval_arm,
    )?;
    let common_str = common_dir
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if (sibling_ids.is_empty() && common_str.is_empty()) || tree_sha256.is_empty() {
        return Ok(matched);
    }
    let sibling_set: BTreeSet<&str> = sibling_ids.iter().map(String::as_str).collect();
    let mut query = db.prepare(&format!(
        "SELECT {CAPSULE_COLUMNS} FROM skillcache_capsules
         WHERE project_id!=?1 AND status='active'
           AND tree_sha256=?2
           AND (?3 = '' OR skill_name=?3)
           AND ((?4 IS NULL AND eval_arm IS NULL) OR eval_arm=?4)
         ORDER BY (skill_path=?5) DESC, created_at DESC, rowid DESC"
    ))?;
    let rows = query.query_map(
        params![project_id, tree_sha256, skill_name, eval_arm, skill_path],
        capsule_row,
    )?;
    for row in rows {
        let cap = row?;
        let cap_project = cap["project_id"].as_str().unwrap_or("");
        let cap_common = cap["git_common_dir"].as_str().unwrap_or("");
        if !(sibling_set.contains(cap_project)
            || (!common_str.is_empty() && cap_common == common_str))
        {
            continue;
        }
        let cap_path = cap["skill_path"].as_str().unwrap_or("");
        let path_matches = cap_path == skill_path
            || matches!(
                (canonical_mirror_skill(cap_path), canonical_mirror_skill(skill_path)),
                (Some(a), Some(b)) if a == b
            );
        if path_matches {
            matched.push(cap);
        }
    }
    Ok(matched)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingRequalify {
    pub capsule_id: String,
    pub project_id: String,
    pub prior_status: String,
    pub reason: Option<String>,
    pub attempts: i64,
}

/// Persists a pending background requalification in SQLite so `WorkQueue` saturation
/// or restarts never drop requalification work.
pub(crate) fn enqueue_pending_requalify(
    db: &Connection,
    project_id: &str,
    capsule_id: &str,
    prior_status: &str,
    reason: Option<&str>,
) -> Result<()> {
    let now = store::now();
    db.execute(
        "INSERT INTO skillcache_pending_requalify(
            capsule_id,project_id,prior_status,reason,attempts,created_at,updated_at
         ) VALUES(?1,?2,?3,?4,0,?5,?5)
         ON CONFLICT(capsule_id) DO UPDATE SET
            project_id=excluded.project_id,
            prior_status=CASE
                WHEN skillcache_pending_requalify.prior_status='needs_recompile' THEN 'needs_recompile'
                ELSE excluded.prior_status
            END,
            reason=COALESCE(excluded.reason, skillcache_pending_requalify.reason),
            updated_at=excluded.updated_at",
        params![capsule_id, project_id, prior_status, reason, now],
    )?;
    Ok(())
}

pub(crate) fn list_pending_requalify(db: &Connection, limit: i64) -> Result<Vec<PendingRequalify>> {
    let mut query = db.prepare(
        "SELECT capsule_id,project_id,prior_status,reason,attempts
         FROM skillcache_pending_requalify
         ORDER BY created_at ASC,rowid ASC
         LIMIT ?1",
    )?;
    let rows = query.query_map([limit], |row| {
        Ok(PendingRequalify {
            capsule_id: row.get(0)?,
            project_id: row.get(1)?,
            prior_status: row.get(2)?,
            reason: row.get(3)?,
            attempts: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub(crate) fn record_pending_requalify_attempt(
    db: &Connection,
    capsule_id: &str,
    error: &str,
) -> Result<i64> {
    db.execute(
        "UPDATE skillcache_pending_requalify
         SET attempts=attempts+1,last_error=?2,updated_at=?3
         WHERE capsule_id=?1",
        params![capsule_id, error, store::now()],
    )?;
    Ok(db
        .query_row(
            "SELECT attempts FROM skillcache_pending_requalify WHERE capsule_id=?1",
            [capsule_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

pub(crate) fn delete_pending_requalify(db: &Connection, capsule_id: &str) -> Result<()> {
    db.execute(
        "DELETE FROM skillcache_pending_requalify WHERE capsule_id=?1",
        [capsule_id],
    )?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn pending_requalify_count(db: &Connection) -> Result<i64> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM skillcache_pending_requalify",
        [],
        |row| row.get(0),
    )?)
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
    db.execute(
        "DELETE FROM skillcache_pending_requalify WHERE capsule_id=?1",
        [capsule_id],
    )?;
    db.execute(
        "DELETE FROM skillcache_compiles WHERE capsule_id=?1",
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
                usage: &Value::Null,
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
        assert!(compile(&db, "done").unwrap().is_none());
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
            capsules(&db, "project", None, None, None, false, false)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            capsules(&db, "project", None, None, None, true, false)
                .unwrap()
                .len(),
            2
        );
        // An evaluation arm's capsule is listed only when asked for.
        let with_eval = capsules(&db, "project", None, None, None, true, true).unwrap();
        assert_eq!(with_eval.len(), 3);
        assert!(
            with_eval
                .iter()
                .any(|capsule| capsule["eval_arm"] == "skillcache_agent")
        );
        assert_eq!(
            capsules(&db, "project", None, Some("retired"), None, false, false)
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

    #[test]
    fn dependency_manifest_racy_ns_and_template_index_round_trip() {
        let now = current_unix_ns();
        let manifest = ArtifactDependencyManifest {
            skill_paths: vec![".claude/skills/deploy/SKILL.md".into()],
            watched_globs: vec!["src/routes/**".into()],
            manifest_files: vec!["composer.json".into()],
            ast_selectors: vec!["src/Controllers/AuthController.php".into()],
            route_selectors: vec!["routes/api.php".into()],
            table_selectors: vec!["database/migrations/**".into()],
            caller_files: vec!["app/Http/Kernel.php".into()],
            path_hashes: BTreeMap::new(),
            ir_revision: 10,
            verified_ns: now,
        };
        assert_eq!(
            manifest.evaluate_delta(
                &[".claude/skills/deploy/SKILL.md".into()],
                &[],
                10,
                now + RACY_NS + 100
            ),
            DeltaMatch::ExactSkill
        );
        // Same/older IR revision outside the 2s RACY_NS window is already verified.
        assert_eq!(
            manifest.evaluate_delta(&["routes/api.php".into()], &[], 10, now + RACY_NS + 1),
            DeltaMatch::None
        );
        // Within RACY_NS window, matching changes must be treated as drifted conservatively.
        assert_eq!(
            manifest.evaluate_delta(&["routes/api.php".into()], &[], 10, now + 1000),
            DeltaMatch::Dependency
        );
        // Newer IR revision triggers dependency staleness even after RACY_NS.
        assert_eq!(
            manifest.evaluate_delta(
                &[],
                &["app/Http/Kernel.php".into()],
                11,
                now + RACY_NS + 500
            ),
            DeltaMatch::Dependency
        );
        assert_eq!(
            manifest.evaluate_delta(&["README.md".into()], &[], 11, now + 500),
            DeltaMatch::None
        );

        let db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        let answer = json!({
            "form": "hybrid",
            "capsule": {
                "mu": {"name": "php-lint", "description": "Check PHP syntax"},
                "source": {"skills": [{"name": "php-lint", "path": ".claude/skills/php-lint/SKILL.md"}]},
                "capabilities": {"network": "none", "runtimes": ["python3"]},
                "bindings": [],
                "programs": [{
                    "id": "lint",
                    "entry": ["python3", "programs/lint/main.py"],
                    "inputs": [],
                    "outputs": [],
                    "check": ["python3", "programs/lint/main.py", "--check"],
                    "timeout_seconds": 30,
                    "read_only": true
                }],
                "steps": [{"kind": "program", "program": "lint"}],
                "verification": []
            },
            "files": [{"path": "programs/lint/main.py", "content": "print('ok')\n"}],
            "examples": {"bindings": {}, "steps": {}}
        });
        let known = vec![infra_skillcache::candidate::KnownSkill {
            name: "php-lint".into(),
            path: ".claude/skills/php-lint/SKILL.md".into(),
            tree_sha256: "e".repeat(64),
        }];
        let cand = infra_skillcache::candidate::from_answer(
            &answer,
            &known,
            infra_skillcache::Form::Hybrid,
        )
        .unwrap();
        let tpl = infra_skillcache::candidate::as_portable_template(&cand, "Laravel");
        upsert_template(&db, "Laravel", "php-lint", &"e".repeat(64), &tpl).unwrap();
        let found = find_templates(&db, "Laravel", "php-lint", &"e".repeat(64)).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["edition"], "Laravel");
        assert_eq!(found[0]["mu"]["name"], "php-lint");
    }

    #[test]
    fn pending_requalify_needs_trace_and_mirror_dedup_work() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("proj");
        std::fs::create_dir_all(&ws).unwrap();
        let store = crate::store::SessionStore::open(&dir.path().join("state")).unwrap();
        let project_id = store.register(&ws).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let session = store
            .create(
                &json!({"project_id": project_id, "provider": "codex"}),
                "Run pdf skill",
                &[],
            )
            .unwrap();
        let sid = session["id"].as_str().unwrap();
        let db = store.connection().unwrap();

        let tree_sha = "d".repeat(64);
        save_trace(
            &db,
            &TraceRow {
                generation: "g-old",
                session_id: sid,
                provider: "codex",
                model: None,
                request: "Run pdf skill",
                skill_refs: &json!([{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md", "tree_sha256": tree_sha}]),
                steps: &json!([{"kind": "command", "command": "php artisan --version"}]),
                usage: &json!({}),
                redaction: &json!({"tool_chain_manifest_hashes": {"composer.json": "h123"}}),
            },
        )
        .unwrap();

        let skill_ref = json!({
            "name": "pdf",
            "path": ".claude/skills/pdf/SKILL.md",
            "tree_sha256": tree_sha
        });
        begin_capsule(
            &db,
            "c1",
            "cmp1",
            &project_id,
            sid,
            "g-old",
            &skill_ref,
            None,
        )
        .unwrap();
        activate_capsule(
            &db,
            &Qualified {
                capsule_id: "c1",
                mu_name: "pdf-run",
                mu_description: "Run pdf workflow",
                form: "hybrid",
                capsule_sha256: "cap-sha",
                package_sha256: "pkg-sha",
                skill_files: &json!({}),
                env_fingerprint: "env",
                engine_digest: "eng",
                artifact_dir: "/tmp/c1",
                capabilities: &json!({"network": "none"}),
            },
        )
        .unwrap();

        let manifest = ArtifactDependencyManifest {
            skill_paths: vec![".claude/skills/pdf/SKILL.md".into()],
            watched_globs: Vec::new(),
            manifest_files: vec!["composer.json".into()],
            ast_selectors: Vec::new(),
            route_selectors: Vec::new(),
            table_selectors: Vec::new(),
            caller_files: Vec::new(),
            path_hashes: BTreeMap::new(),
            ir_revision: 1,
            verified_ns: 1,
        };
        set_dependency_manifest(&db, "c1", &manifest).unwrap();
        let stored_cap = capsule(&db, "c1").unwrap().unwrap();
        assert_eq!(
            stored_cap["dependency_manifest"]["path_hashes"]["composer.json"],
            "h123"
        );

        assert_eq!(
            canonical_mirror_skill(".claude/skills/pdf/SKILL.md"),
            Some(("ws", "pdf/SKILL.md".into()))
        );
        assert_eq!(
            canonical_mirror_skill(".agents/skills/pdf/SKILL.md"),
            Some(("ws", "pdf/SKILL.md".into()))
        );
        assert_eq!(
            canonical_mirror_skill(".cursor/skills/pdf/SKILL.md"),
            Some(("ws", "pdf/SKILL.md".into()))
        );
        assert_eq!(
            canonical_mirror_skill("~/.claude/skills/pdf/SKILL.md"),
            Some(("~", "pdf/SKILL.md".into()))
        );
        assert_eq!(
            active_capsules_for_skill_or_mirror(
                &db,
                &project_id,
                ".agents/skills/pdf/SKILL.md",
                "pdf",
                &tree_sha,
                None,
            )
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            active_capsules_for_skill_or_mirror(
                &db,
                &project_id,
                ".cursor/skills/pdf/SKILL.md",
                "pdf",
                &tree_sha,
                None,
            )
            .unwrap()
            .len(),
            1
        );
        assert!(
            active_capsules_for_skill_or_mirror(
                &db,
                &project_id,
                ".cursor/skills/pdf/SKILL.md",
                "pdf",
                &"f".repeat(64),
                None,
            )
            .unwrap()
            .is_empty()
        );

        set_capsule_status(&db, "c1", "needs_recompile", Some("Warm deopt")).unwrap();
        mark_capsule_needs_trace(&db, "c1").unwrap();
        assert_eq!(capsules_needing_trace(&db, &project_id).unwrap().len(), 1);
        let deopted_cap = capsule(&db, "c1").unwrap().unwrap();
        assert!(
            find_fresh_trace_after(&db, &project_id, &deopted_cap)
                .unwrap()
                .is_none()
        );

        save_trace(
            &db,
            &TraceRow {
                generation: "g-fresh",
                session_id: sid,
                provider: "codex",
                model: None,
                request: "Run pdf skill",
                skill_refs: &json!([{"name": "pdf", "path": ".agents/skills/pdf/SKILL.md", "tree_sha256": tree_sha}]),
                steps: &json!([{"kind": "command", "command": "php artisan --version"}]),
                usage: &json!({}),
                redaction: &json!({"admissible": true}),
            },
        )
        .unwrap();
        let fresh = find_fresh_trace_after(&db, &project_id, &deopted_cap)
            .unwrap()
            .expect("fresh observed trace after deopt must be discovered");
        assert_eq!(fresh["generation"], "g-fresh");
        clear_capsule_needs_trace(&db, "c1").unwrap();
        assert!(capsules_needing_trace(&db, &project_id).unwrap().is_empty());

        enqueue_pending_requalify(&db, &project_id, "c1", "needs_recompile", Some("Deopted"))
            .unwrap();
        enqueue_pending_requalify(&db, &project_id, "c1", "stale", Some("IR delta")).unwrap();
        let pending = list_pending_requalify(&db, 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].prior_status, "needs_recompile");
        assert_eq!(
            record_pending_requalify_attempt(&db, "c1", "busy").unwrap(),
            1
        );
        assert_eq!(pending_requalify_count(&db).unwrap(), 1);
        delete_pending_requalify(&db, "c1").unwrap();
        assert_eq!(pending_requalify_count(&db).unwrap(), 0);
    }

    #[test]
    fn recover_prunes_old_compiles_retired_superseded_and_unpins_snapshots_and_lru_evicts() {
        let dir = crate::skillcache::test_support::scratch();
        let state = dir.path().join("state");
        let ws = dir.path().join("proj");
        std::fs::create_dir_all(&ws).unwrap();

        let store = crate::store::SessionStore::open(&state).unwrap();
        let project_id = store.register(&ws).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let session = store
            .create(
                &json!({"project_id": project_id, "provider": "claude"}),
                "gc",
                &[],
            )
            .unwrap();
        let sid = session["id"].as_str().unwrap().to_owned();
        let pinned_session = store
            .create(
                &json!({
                    "project_id": project_id,
                    "provider": "claude",
                    "skillcache": {"mode": "auto", "capsule_id": "cap-pinned-retired"}
                }),
                "pinned",
                &[],
            )
            .unwrap();
        let _pinned_sid = pinned_session["id"].as_str().unwrap().to_owned();

        let old_ts = (chrono::Utc::now() - chrono::Duration::days(TRACE_DAYS + 5))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let mk_snapshot = |generation: &str| {
            let p = state.join("skillcache/snapshots").join(generation);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("blob.bin"), b"snapshot-bytes").unwrap();
            p
        };
        let mk_artifacts = |capsule_id: &str| {
            let art = state
                .join("skillcache/capsules")
                .join(&project_id)
                .join(capsule_id)
                .join("1");
            std::fs::create_dir_all(art.join("warm")).unwrap();
            std::fs::write(art.join("warm/capsule.json"), b"{}").unwrap();
            let qual = state
                .join("skillcache/qualification")
                .join(capsule_id)
                .join("1");
            std::fs::create_dir_all(&qual).unwrap();
            std::fs::write(qual.join("examples.json"), b"{}").unwrap();
            (art, qual)
        };

        {
            let db = store.connection().unwrap();
            let skill_pdf =
                json!({"name": "pdf", "path": ".claude/skills/pdf/SKILL.md", "tree_sha256": "a"});
            let skill_solo =
                json!({"name": "solo", "path": ".claude/skills/solo/SKILL.md", "tree_sha256": "b"});

            // 1. Old failed capsule + compile + snapshot -> pruned & snapshot deleted.
            let (art_fail, qual_fail) = mk_artifacts("cap-fail-old");
            let snap_fail = mk_snapshot("gen-fail-old");
            keep(&db, "gen-fail-old", &sid);
            begin_capsule(
                &db,
                "cap-fail-old",
                "cmp-fail-old",
                &project_id,
                &sid,
                "gen-fail-old",
                &skill_pdf,
                None,
            )
            .unwrap();
            begin_compile(
                &db,
                "cmp-fail-old",
                "cap-fail-old",
                &project_id,
                &sid,
                "gen-fail-old",
                "claude",
                None,
                None,
            )
            .unwrap();
            finish_compile(
                &db,
                "cmp-fail-old",
                "failed",
                1,
                "boom",
                &json!({}),
                &json!({}),
            )
            .unwrap();
            db.execute(
                "UPDATE skillcache_capsules SET status='failed', artifact_dir=?2, updated_at=?3 WHERE capsule_id=?1",
                params!["cap-fail-old", art_fail.to_string_lossy().as_ref(), &old_ts],
            ).unwrap();
            db.execute(
                "UPDATE skillcache_compiles SET finished_at=?2 WHERE compile_id=?1",
                params!["cmp-fail-old", &old_ts],
            )
            .unwrap();

            // 2. Old interrupted compile without surviving capsule -> pruned.
            begin_compile(
                &db,
                "cmp-int-old",
                "cap-missing",
                &project_id,
                &sid,
                "gen-fail-old",
                "claude",
                None,
                None,
            )
            .unwrap();
            db.execute(
                "UPDATE skillcache_compiles SET status='interrupted', started_at=?2, finished_at=?2 WHERE compile_id=?1",
                params!["cmp-int-old", &old_ts],
            ).unwrap();

            // 3. Old retired capsule -> pruned & snapshot deleted.
            let (art_ret, _) = mk_artifacts("cap-ret-old");
            let snap_ret = mk_snapshot("gen-ret-old");
            keep(&db, "gen-ret-old", &sid);
            begin_capsule(
                &db,
                "cap-ret-old",
                "cmp-ret-old",
                &project_id,
                &sid,
                "gen-ret-old",
                &skill_pdf,
                None,
            )
            .unwrap();
            db.execute(
                "UPDATE skillcache_capsules SET status='retired', artifact_dir=?2, updated_at=?3 WHERE capsule_id=?1",
                params!["cap-ret-old", art_ret.to_string_lossy().as_ref(), &old_ts],
            ).unwrap();

            // 4. Recently retired capsule (< TRACE_DAYS) with an OLD trace (> TRACE_DAYS) -> capsule kept, snapshot unpinned & deleted!
            let snap_ret_recent = mk_snapshot("gen-ret-recent");
            keep(&db, "gen-ret-recent", &sid);
            begin_capsule(
                &db,
                "cap-ret-recent",
                "cmp-ret-recent",
                &project_id,
                &sid,
                "gen-ret-recent",
                &skill_pdf,
                None,
            )
            .unwrap();
            set_capsule_status(&db, "cap-ret-recent", "retired", Some("retired today")).unwrap();

            // 5. Session-pinned old retired capsule -> preserved.
            begin_capsule(
                &db,
                "cap-pinned-retired",
                "cmp-pin-ret",
                &project_id,
                &sid,
                "gen-pin-ret",
                &skill_pdf,
                None,
            )
            .unwrap();
            db.execute(
                "UPDATE skillcache_capsules SET status='retired', updated_at=?2 WHERE capsule_id=?1",
                params!["cap-pinned-retired", &old_ts],
            ).unwrap();

            // 6. Old active replacement + old superseded stale capsule + old non-superseded stale capsule.
            let snap_active = mk_snapshot("gen-active-old");
            keep(&db, "gen-active-old", &sid);
            begin_capsule(
                &db,
                "cap-active-old",
                "cmp-act-old",
                &project_id,
                &sid,
                "gen-active-old",
                &skill_pdf,
                None,
            )
            .unwrap();
            activate_capsule(
                &db,
                &Qualified {
                    capsule_id: "cap-active-old",
                    mu_name: "pdf",
                    mu_description: "d",
                    form: "textual",
                    capsule_sha256: "a",
                    package_sha256: "p",
                    skill_files: &json!({}),
                    env_fingerprint: "e",
                    engine_digest: "d",
                    artifact_dir: "/tmp/act",
                    capabilities: &json!({}),
                },
            )
            .unwrap();

            let (art_sup, _) = mk_artifacts("cap-superseded");
            let snap_sup = mk_snapshot("gen-superseded");
            keep(&db, "gen-superseded", &sid);
            begin_capsule(
                &db,
                "cap-superseded",
                "cmp-sup",
                &project_id,
                &sid,
                "gen-superseded",
                &skill_pdf,
                None,
            )
            .unwrap();
            db.execute(
                "UPDATE skillcache_capsules SET status='stale', artifact_dir=?2, updated_at=?3 WHERE capsule_id=?1",
                params!["cap-superseded", art_sup.to_string_lossy().as_ref(), &old_ts],
            ).unwrap();

            let snap_solo = mk_snapshot("gen-solo-stale");
            keep(&db, "gen-solo-stale", &sid);
            begin_capsule(
                &db,
                "cap-solo-stale",
                "cmp-solo",
                &project_id,
                &sid,
                "gen-solo-stale",
                &skill_solo,
                None,
            )
            .unwrap();
            db.execute(
                "UPDATE skillcache_capsules SET status='stale', updated_at=?2 WHERE capsule_id=?1",
                params!["cap-solo-stale", &old_ts],
            )
            .unwrap();

            db.execute("UPDATE skillcache_traces SET created_at=?1", [&old_ts])
                .unwrap();

            recover(&db).unwrap();

            assert!(capsule(&db, "cap-fail-old").unwrap().is_none());
            assert!(compile(&db, "cmp-fail-old").unwrap().is_none());
            assert!(compile(&db, "cmp-int-old").unwrap().is_none());
            assert!(capsule(&db, "cap-ret-old").unwrap().is_none());
            assert!(capsule(&db, "cap-superseded").unwrap().is_none());
            assert!(capsule(&db, "cap-ret-recent").unwrap().is_some());
            assert!(capsule(&db, "cap-pinned-retired").unwrap().is_some());
            assert!(capsule(&db, "cap-active-old").unwrap().is_some());
            assert!(capsule(&db, "cap-solo-stale").unwrap().is_some());

            assert!(
                !snap_fail.exists(),
                "failed snapshot must be unpinned and deleted"
            );
            assert!(
                !snap_ret.exists(),
                "pruned retired snapshot must be deleted"
            );
            assert!(
                !snap_ret_recent.exists(),
                "retired capsule must not pin >TRACE_DAYS snapshot"
            );
            assert!(!snap_sup.exists(), "superseded snapshot must be deleted");
            assert!(
                snap_active.exists(),
                "active capsule snapshot must stay pinned"
            );
            assert!(
                snap_solo.exists(),
                "un-superseded stale capsule snapshot stays"
            );
            assert!(
                !art_fail.parent().unwrap().exists(),
                "pruned artifact dir must be removed"
            );
            assert!(
                !qual_fail.parent().unwrap().exists(),
                "pruned qualification dir must be removed"
            );
        }

        // Now test per-project LRU eviction by `last_used_at`, preserving pinned & active capsules.
        {
            let db = store.connection().unwrap();
            let skill =
                json!({"name": "k", "path": ".claude/skills/k/SKILL.md", "tree_sha256": "c"});
            for i in 0..8 {
                let id = format!("lru-stale-{i}");
                begin_capsule(
                    &db,
                    &id,
                    &format!("cmp-s-{i}"),
                    &project_id,
                    &sid,
                    "gen-active-old",
                    &skill,
                    None,
                )
                .unwrap();
                db.execute(
                    "UPDATE skillcache_capsules SET status='stale', last_used_at=?2 WHERE capsule_id=?1",
                    params![&id, format!("2026-01-{:02}T00:00:00Z", i + 1)],
                ).unwrap();
            }
            for i in 0..5 {
                let id = format!("lru-act-{i}");
                begin_capsule(
                    &db,
                    &id,
                    &format!("cmp-a-{i}"),
                    &project_id,
                    &sid,
                    "gen-active-old",
                    &skill,
                    None,
                )
                .unwrap();
                db.execute(
                    "UPDATE skillcache_capsules SET status='active', last_used_at=?2 WHERE capsule_id=?1",
                    params![&id, format!("2026-02-{:02}T00:00:00Z", i + 1)],
                ).unwrap();
            }
            // 4 existing (`cap-ret-recent`, `cap-pinned-retired`, `cap-active-old`, `cap-solo-stale`)
            // + 8 `lru-stale-*` + 5 `lru-act-*` = 17 total (6 active + 1 pinned + 10 non-active).
            // Cap=8 evicts 9 oldest non-active first, preserving pinned and all active capsules.
            let evicted1 = enforce_project_capsule_cap(&db, &project_id, 8).unwrap();
            assert_eq!(evicted1, 9);
            assert!(
                capsule(&db, "cap-pinned-retired").unwrap().is_some(),
                "pinned capsule must be preserved"
            );
            for i in 0..5 {
                assert!(
                    capsule(&db, &format!("lru-act-{i}")).unwrap().is_some(),
                    "active lru-act-{i} preserved"
                );
            }
            // Cap=4 evicts remaining non-active (`cap-ret-recent`) and oldest 3 active (`lru-act-0..2`),
            // preserving `cap-pinned-retired`, `lru-act-3`, `lru-act-4`, and `cap-active-old`.
            let _ = enforce_project_capsule_cap(&db, &project_id, 4).unwrap();
            let remaining = capsules(&db, &project_id, None, None, None, true, true).unwrap();
            assert_eq!(remaining.len(), 4);
            assert!(
                capsule(&db, "cap-pinned-retired").unwrap().is_some(),
                "pinned stays even under tighter cap"
            );
            assert!(
                capsule(&db, "lru-act-4").unwrap().is_some(),
                "most recently used active stays"
            );
            assert!(
                capsule(&db, "lru-act-0").unwrap().is_none(),
                "oldest last_used_at active evicted"
            );
        }
    }

    #[test]
    fn worktree_sibling_discovery_and_semantic_template_matching_work() {
        let dir = crate::skillcache::test_support::scratch();
        let repo = dir.path().join("repo");
        let wt2 = dir.path().join("wt2");
        let other = dir.path().join("other");
        for p in [&repo, &other] {
            std::fs::create_dir_all(p).unwrap();
            std::fs::write(p.join("README.md"), "hi\n").unwrap();
            let st = std::process::Command::new("git")
                .arg("-C")
                .arg(p)
                .args([
                    "-c",
                    "user.name=T",
                    "-c",
                    "user.email=t@e.invalid",
                    "init",
                    "-q",
                ])
                .status()
                .unwrap();
            assert!(st.success());
            let st = std::process::Command::new("git")
                .arg("-C")
                .arg(p)
                .args([
                    "-c",
                    "user.name=T",
                    "-c",
                    "user.email=t@e.invalid",
                    "add",
                    ".",
                ])
                .status()
                .unwrap();
            assert!(st.success());
            let st = std::process::Command::new("git")
                .arg("-C")
                .arg(p)
                .args([
                    "-c",
                    "user.name=T",
                    "-c",
                    "user.email=t@e.invalid",
                    "commit",
                    "-q",
                    "-m",
                    "init",
                ])
                .status()
                .unwrap();
            assert!(st.success());
        }
        let st = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "add", "-q", wt2.to_str().unwrap()])
            .status()
            .unwrap();
        assert!(st.success());

        let store = crate::store::SessionStore::open(&dir.path().join("state")).unwrap();
        let p1 = store.register(&repo).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let p2 = store.register(&wt2).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let p3 = store.register(&other).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let db = store.connection().unwrap();

        assert_eq!(
            canonical_git_common_dir(&repo),
            canonical_git_common_dir(&wt2)
        );
        assert_ne!(
            canonical_git_common_dir(&repo),
            canonical_git_common_dir(&other)
        );
        let sibs_p2 = sibling_worktree_projects(&db, &p2, &wt2).unwrap();
        assert_eq!(sibs_p2, vec![p1.clone()]);
        assert!(
            sibling_worktree_projects(&db, &p3, &other)
                .unwrap()
                .is_empty()
        );

        let answer = json!({
            "form": "textual",
            "capsule": {
                "mu": {"name": "php-lint", "description": "Check PHP syntax"},
                "source": {"skills": [{"name": "php-lint", "path": ".claude/skills/php-lint/SKILL.md"}]}
            },
            "markdown": "# Lint\n\n1. Run `php -l`.\n"
        });
        let known = vec![infra_skillcache::candidate::KnownSkill {
            name: "php-lint".into(),
            path: ".claude/skills/php-lint/SKILL.md".into(),
            tree_sha256: "a".repeat(64),
        }];
        let cand = infra_skillcache::candidate::from_answer(
            &answer,
            &known,
            infra_skillcache::Form::Textual,
        )
        .unwrap();
        let tpl = infra_skillcache::candidate::as_portable_template(&cand, "Laravel");
        upsert_template_with_semantic(
            &db,
            "Laravel",
            "php-lint",
            &"a".repeat(64),
            &"s".repeat(64),
            &tpl,
        )
        .unwrap();

        // Semantic lookup matches when exact tree differs; empty semantic hash does NOT falsely match.
        let by_sem = find_templates_with_semantic(
            &db,
            "Laravel",
            "php-lint",
            &"b".repeat(64),
            &"s".repeat(64),
        )
        .unwrap();
        assert_eq!(by_sem.len(), 1);
        assert_eq!(by_sem[0]["skill_semantic_sha"], "s".repeat(64));
        assert!(
            find_templates_with_semantic(&db, "Laravel", "php-lint", &"b".repeat(64), "")
                .unwrap()
                .is_empty()
        );
    }
}
