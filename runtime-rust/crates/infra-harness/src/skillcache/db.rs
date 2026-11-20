//! SkillCache tables in the session database.
//!
//! They live in `sessions.sqlite3` with every other Harness domain. Nothing
//! is written to a project's `memory-bank/` or `project-brain/`.
use crate::{Result, store};
use rusqlite::{Connection, params};
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
        CREATE INDEX IF NOT EXISTS skillcache_traces_session ON skillcache_traces(session_id,created_at);",
    )?;
    Ok(())
}

/// Runs once when the store opens.
pub(crate) fn recover(db: &Connection) -> Result<()> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(TRACE_DAYS))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    db.execute(
        "DELETE FROM skillcache_traces WHERE created_at<?1",
        [cutoff],
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
        "DELETE FROM skillcache_traces WHERE session_id=?1 AND generation NOT IN
           (SELECT generation FROM skillcache_traces WHERE session_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT ?2)",
        params![row.session_id, TRACES_PER_SESSION as i64],
    )?;
    Ok(digest)
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
}
