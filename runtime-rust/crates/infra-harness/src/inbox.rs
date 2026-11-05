//! Durable, explicitly scoped follow-ups. A local assignment is not proof of
//! provider receipt, and interrupted assignments are never replayed here.
use crate::{
    HarnessError, Result,
    store::{self, SessionStore},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

const SPECIAL: &[&str] = &[
    "brain",
    "sdd",
    "fleet",
    "clash",
    "creator",
    "system_run",
    "system_discovery",
];
const FROZEN: &[&str] = &[
    "id",
    "provider",
    "native_session_id",
    "project_id",
    "project_path",
    "workspace",
    "branch",
    "git_common_dir",
    "workflow",
    "mode",
    "model",
    "thinking_effort",
    "model_routing",
    "agents_enabled",
    "agent_count",
    "project_context",
    "budgets",
    "budget_revision",
];

pub(crate) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_inbox(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            request_id TEXT NOT NULL,
            prompt TEXT NOT NULL,
            prompt_hash TEXT NOT NULL,
            source_generation TEXT NOT NULL,
            authority TEXT NOT NULL,
            scope_stamp TEXT NOT NULL,
            source_ready INTEGER NOT NULL DEFAULT 0 CHECK(source_ready IN (0,1)),
            delivery_generation TEXT,
            status TEXT NOT NULL CHECK(status IN ('pending','assigned','completed','blocked','cancelled','uncertain')),
            reason TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(session_id,request_id)
        );
        CREATE INDEX IF NOT EXISTS session_inbox_page ON session_inbox(session_id,id);
        CREATE INDEX IF NOT EXISTS session_inbox_delivery ON session_inbox(session_id,delivery_generation);
        CREATE UNIQUE INDEX IF NOT EXISTS session_inbox_one_pending ON session_inbox(session_id) WHERE status='pending';",
    )?;
    db.execute_batch("CREATE INDEX IF NOT EXISTS session_inbox_active ON session_inbox(status,id) WHERE status IN ('pending','assigned');")?;
    Ok(())
}

pub(crate) fn recover(db: &Connection) -> Result<()> {
    db.execute(
        "UPDATE session_inbox SET status=CASE status WHEN 'pending' THEN 'blocked' ELSE 'uncertain' END,
         source_ready=0,reason='Server restarted; review this follow-up before continuing.',updated_at=?1
         WHERE status IN ('pending','assigned')",
        [store::now()],
    )?;
    Ok(())
}

/// The caller invokes this inside its session-cancellation transaction.
pub(crate) fn cancel_session(db: &Connection, sid: &str) -> Result<()> {
    db.execute(
        "UPDATE session_inbox SET status=CASE status WHEN 'pending' THEN 'cancelled' ELSE 'uncertain' END,
         source_ready=0,reason='Session cancellation requested.',updated_at=?1
         WHERE session_id=?2 AND status IN ('pending','assigned')",
        params![store::now(), sid],
    )?;
    Ok(())
}

pub(crate) fn authority(session: &Value) -> Result<Value> {
    if session["workflow"] != "native"
        || session["agents_enabled"] != false
        || SPECIAL.iter().any(|key| !session[*key].is_null())
        || !["claude", "codex", "cursor"].contains(&session["provider"].as_str().unwrap_or(""))
        || !["plan", "edit"].contains(&session["mode"].as_str().unwrap_or(""))
        || session["native_session_id"].as_str().is_none_or(|id| {
            id.trim().is_empty()
                || id.len() > 4096
                || id.starts_with('-')
                || id.chars().any(char::is_control)
        })
    {
        return Err(HarnessError::conflict(
            "Inbox follow-ups require an ordinary native session without additional agents or specialized workflows and a known native session identity.",
        ));
    }
    for key in ["id", "project_id", "project_path"] {
        if session[key]
            .as_str()
            .is_none_or(|value| value.trim().is_empty() || value.contains('\0'))
        {
            return Err(HarnessError::conflict(
                "The session binding is unavailable.",
            ));
        }
    }
    let mut frozen = json!({});
    for key in FROZEN.iter().chain(SPECIAL.iter()) {
        frozen[*key] = session[*key].clone();
    }
    if frozen.to_string().len() > 256 * 1024 {
        return Err(HarnessError::conflict(
            "The frozen inbox authority exceeds its byte limit.",
        ));
    }
    Ok(frozen)
}

pub(crate) fn validate_request(id: &str, prompt: &str) -> Result<String> {
    if id.is_empty()
        || id.len() > 128
        || [".", ".."].contains(&id)
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(HarnessError::invalid(
            "Request ID must contain 1–128 ASCII letters, digits, hyphens, underscores or dots.",
        ));
    }
    let normalized = prompt.trim();
    if normalized.is_empty()
        || prompt.len() > 32000
        || prompt.contains('\0')
        || normalized.starts_with('/')
    {
        return Err(HarnessError::invalid(
            "Enter a follow-up of 1–32000 UTF-8 bytes without a leading slash command.",
        ));
    }
    Ok(normalized.into())
}

fn scope(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 16384 || value.contains('\0') {
        return Err(HarnessError::invalid("Invalid inbox scope stamp."));
    }
    Ok(())
}

struct Record {
    id: i64,
    sid: String,
    request: String,
    prompt: String,
    hash: String,
    source: String,
    authority: String,
    scope: String,
    ready: bool,
    delivery: Option<String>,
    status: String,
    reason: Option<String>,
    created: String,
    updated: String,
}
const COLUMNS: &str = "id,session_id,request_id,prompt,prompt_hash,source_generation,authority,scope_stamp,source_ready,delivery_generation,status,reason,created_at,updated_at";

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        id: row.get(0)?,
        sid: row.get(1)?,
        request: row.get(2)?,
        prompt: row.get(3)?,
        hash: row.get(4)?,
        source: row.get(5)?,
        authority: row.get(6)?,
        scope: row.get(7)?,
        ready: row.get(8)?,
        delivery: row.get(9)?,
        status: row.get(10)?,
        reason: row.get(11)?,
        created: row.get(12)?,
        updated: row.get(13)?,
    })
}
impl Record {
    fn receipt(&self) -> Value {
        json!({"id":self.id,"session_id":self.sid,"request_id":self.request,
            "prompt":self.prompt,"source_generation":self.source,
            "delivery_generation":self.delivery,"status":self.status,"reason":self.reason,
            "created_at":self.created,"updated_at":self.updated})
    }
    fn frozen(&self) -> Result<Value> {
        crate::strict_json::parse(self.authority.as_bytes(), 256 * 1024)
    }
}
fn record(db: &Connection, sid: &str, request: &str) -> Result<Option<Record>> {
    Ok(db
        .query_row(
            &format!("SELECT {COLUMNS} FROM session_inbox WHERE session_id=?1 AND request_id=?2"),
            params![sid, request],
            decode,
        )
        .optional()?)
}
fn existing(db: &Connection, sid: &str, id: &str, prompt: &str) -> Result<Option<Value>> {
    let Some(row) = record(db, sid, id)? else {
        return Ok(None);
    };
    if row.hash != store::sha(prompt.as_bytes()) || row.prompt != prompt {
        return Err(HarnessError::conflict(
            "This request ID already names a different follow-up.",
        ));
    }
    Ok(Some(row.receipt()))
}
fn generation(db: &Connection, sid: &str) -> Result<String> {
    Ok(db.query_row(
        "SELECT generation FROM harness_generations WHERE session_id=?1",
        [sid],
        |row| row.get(0),
    )?)
}
fn block(db: &Connection, sid: &str, request: &str, reason: &str) -> Result<()> {
    db.execute(
        "UPDATE session_inbox SET status='blocked',source_ready=0,reason=?1,updated_at=?2 WHERE session_id=?3 AND request_id=?4 AND status='pending'",
        params![reason, store::now(), sid, request],
    )?;
    Ok(())
}

pub(crate) struct InboxTurn {
    pub session: Value,
    pub prompt: String,
    pub generation: String,
}

impl SessionStore {
    pub(crate) fn inbox_existing(
        &self,
        sid: &str,
        id: &str,
        prompt: &str,
    ) -> Result<Option<Value>> {
        let prompt = validate_request(id, prompt)?;
        let db = self.connection()?;
        existing(&db, sid, id, &prompt)
    }

    pub(crate) fn inbox_accept(
        &self,
        sid: &str,
        id: &str,
        prompt: &str,
        expected_generation: &str,
        frozen: &Value,
        scope_stamp: &str,
    ) -> Result<Value> {
        let prompt = validate_request(id, prompt)?;
        let mut db = self.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(receipt) = existing(&tx, sid, id, &prompt)? {
            return Ok(receipt);
        }
        scope(scope_stamp)?;
        let session = store::get_session(&tx, sid)?;
        if session["status"] != "running"
            || generation(&tx, sid)? != expected_generation
            || authority(&session)? != *frozen
        {
            return Err(HarnessError::conflict(
                "The active session binding changed before accepting this follow-up.",
            ));
        }
        let pending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_inbox WHERE session_id=?1 AND status='pending')",
            [sid],
            |row| row.get(0),
        )?;
        let total: i64 = tx.query_row(
            "SELECT count(*) FROM session_inbox WHERE status IN ('pending','assigned')",
            [],
            |row| row.get(0),
        )?;
        if pending || total >= 64 {
            return Err(HarnessError::conflict("The follow-up inbox is full."));
        }
        let at = store::now();
        tx.execute(
            "INSERT INTO session_inbox(session_id,request_id,prompt,prompt_hash,source_generation,authority,scope_stamp,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8,?8)",
            params![sid,id,prompt,store::sha(prompt.as_bytes()),expected_generation,frozen.to_string(),scope_stamp,at],
        )?;
        let receipt = record(&tx, sid, id)?
            .ok_or_else(|| HarnessError::missing("Inbox receipt is missing."))?
            .receipt();
        tx.commit()?;
        Ok(receipt)
    }

    pub(crate) fn inbox_list(&self, sid: &str, before: Option<i64>) -> Result<Value> {
        if before.is_some_and(|cursor| cursor <= 0) {
            return Err(HarnessError::invalid("Invalid inbox cursor."));
        }
        let db = self.connection()?;
        store::get_session(&db, sid)?;
        let mut stmt = db.prepare(&format!(
            "SELECT {COLUMNS} FROM session_inbox WHERE session_id=?1 AND (?2 IS NULL OR id<?2) ORDER BY id DESC LIMIT 101"
        ))?;
        let rows = stmt
            .query_map(params![sid, before], decode)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = rows.len() > 100;
        let items = rows
            .iter()
            .take(100)
            .map(Record::receipt)
            .collect::<Vec<_>>();
        let next = more.then(|| rows[99].id);
        Ok(json!({"items":items,"next_before":next}))
    }

    pub(crate) fn inbox_pending(&self) -> Result<Vec<Value>> {
        let db = self.connection()?;
        let mut stmt = db.prepare(&format!(
            "SELECT {COLUMNS} FROM session_inbox WHERE status='pending' ORDER BY id LIMIT 64"
        ))?;
        let rows = stmt
            .query_map([], decode)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|row| {
                let mut value = row.receipt();
                value["authority"] = row.frozen()?;
                value["scope_stamp"] = row.scope.into();
                value["source_ready"] = row.ready.into();
                Ok(value)
            })
            .collect()
    }

    pub(crate) fn inbox_block(&self, sid: &str, id: &str, reason: &str) -> Result<()> {
        let db = self.connection()?;
        block(&db, sid, id, reason)
    }

    pub(crate) fn inbox_native_guard(&self, sid: &str, generation: &str) -> Result<Option<String>> {
        let db = self.connection()?;
        let mut stmt = db.prepare(
            "SELECT authority FROM session_inbox WHERE session_id=?1 AND ((status='pending' AND source_generation=?2) OR (status='assigned' AND delivery_generation=?2))",
        )?;
        let rows = stmt.query_map(params![sid, generation], |row| row.get::<_, String>(0))?;
        let mut expected: Option<String> = None;
        for raw in rows {
            let raw = raw?;
            let frozen = crate::strict_json::parse(raw.as_bytes(), 256 * 1024)?;
            let identity = frozen["native_session_id"]
                .as_str()
                .filter(|value| {
                    !value.trim().is_empty() && value.len() <= 4096 && !value.contains('\0')
                })
                .ok_or_else(|| HarnessError::conflict("Inbox native identity is unavailable."))?;
            if expected.as_deref().is_some_and(|value| value != identity) {
                return Err(HarnessError::conflict(
                    "Inbox requests disagree about the native session identity.",
                ));
            }
            expected = Some(identity.into());
        }
        Ok(expected)
    }

    pub(crate) fn inbox_owned(&self, sid: &str, generation: &str) -> Result<bool> {
        let db = self.connection()?;
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_inbox WHERE session_id=?1 AND delivery_generation=?2 AND status='assigned')",
            params![sid,generation], |row| row.get(0),
        )?)
    }

    pub(crate) fn inbox_verify_execution(
        &self,
        sid: &str,
        delivery: &str,
        scope_stamp: &str,
    ) -> Result<()> {
        let db = self.connection()?;
        let raw: Option<(String, String)> = db.query_row(
            "SELECT authority,scope_stamp FROM session_inbox WHERE session_id=?1 AND delivery_generation=?2 AND status='assigned'",
            params![sid,delivery], |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional()?;
        let (frozen, scope) = raw
            .ok_or_else(|| HarnessError::conflict("The inbox assignment is no longer current."))?;
        let frozen = crate::strict_json::parse(frozen.as_bytes(), 256 * 1024)?;
        let session = store::get_session(&db, sid)?;
        if generation(&db, sid)? != delivery
            || authority(&session)? != frozen
            || scope_stamp != scope
        {
            return Err(HarnessError::conflict(
                "The assigned inbox authority or execution scope changed.",
            ));
        }
        Ok(())
    }

    pub(crate) fn inbox_settle(
        &self,
        sid: &str,
        completed_generation: &str,
        success: bool,
        native_proof: Option<&str>,
    ) -> Result<()> {
        let mut db = self.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let session = store::get_session(&tx, sid)?;
        let binding = authority(&session).ok();
        let current = generation(&tx, sid)?;
        let completed =
            success && current == completed_generation && session["status"] == "completed";
        let rows = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {COLUMNS} FROM session_inbox WHERE session_id=?1 AND ((status='assigned' AND delivery_generation=?2) OR (status='pending' AND source_generation=?2))"
            ))?;
            stmt.query_map(params![sid, completed_generation], decode)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for row in rows {
            let frozen = row.frozen()?;
            let proved = completed
                && binding.as_ref() == Some(&frozen)
                && native_proof.is_some_and(|id| frozen["native_session_id"].as_str() == Some(id));
            if row.status == "assigned" {
                tx.execute(
                    "UPDATE session_inbox SET status=?1,reason=?2,updated_at=?3 WHERE id=?4 AND status='assigned'",
                    params![if proved { "completed" } else { "uncertain" },
                        (!proved).then_some("The assigned turn has no matching successful native completion proof."),store::now(),row.id],
                )?;
            } else if proved {
                tx.execute("UPDATE session_inbox SET source_ready=1,updated_at=?1 WHERE id=?2 AND status='pending'", params![store::now(),row.id])?;
            } else {
                block(
                    &tx,
                    sid,
                    &row.request,
                    "The source turn did not complete with its original native identity and authority.",
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn inbox_claim(
        &self,
        sid: &str,
        id: &str,
        scope_stamp: &str,
        frozen: &Value,
    ) -> Result<Option<InboxTurn>> {
        let mut db = self.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(row) = record(&tx, sid, id)? else {
            return Ok(None);
        };
        if row.status != "pending" || !row.ready {
            return Ok(None);
        }
        let session = store::get_session(&tx, sid)?;
        let saved = row.frozen()?;
        if session["status"] != "completed"
            || generation(&tx, sid)? != row.source
            || authority(&session).ok().as_ref() != Some(&saved)
            || *frozen != saved
            || scope_stamp != row.scope
        {
            block(
                &tx,
                sid,
                id,
                "The source generation, session authority or execution scope changed.",
            )?;
            tx.commit()?;
            return Ok(None);
        }
        let queued: i64 = tx.query_row(
            "SELECT count(*) FROM sessions WHERE status='queued'",
            [],
            |r| r.get(0),
        )?;
        if queued >= 16 {
            return Ok(None);
        }
        let next = store::hexid();
        let at = store::now();
        tx.execute(
            "UPDATE sessions SET status='queued',updated_at=?1 WHERE id=?2",
            params![at, sid],
        )?;
        tx.execute(
            "UPDATE harness_generations SET generation=?1 WHERE session_id=?2",
            params![next, sid],
        )?;
        tx.execute(
            "INSERT INTO events(session_id,data) VALUES(?1,?2)",
            params![sid,json!({"kind":"user","text":row.prompt,"attachments":[],"inbox_request_id":id,"launch_id":next,"at":at}).to_string()],
        )?;
        tx.execute(
            "UPDATE session_inbox SET status='assigned',delivery_generation=?1,updated_at=?2 WHERE id=?3 AND status='pending'",
            params![next,at,row.id],
        )?;
        let session = store::get_session(&tx, sid)?;
        tx.commit()?;
        Ok(Some(InboxTurn {
            session,
            prompt: row.prompt,
            generation: next,
        }))
    }

    pub(crate) fn inbox_notification_failed(
        &self,
        sid: &str,
        delivered_generation: &str,
        reason: &str,
    ) -> Result<()> {
        let mut db = self.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE session_inbox SET status='uncertain',reason=?1,updated_at=?2 WHERE session_id=?3 AND delivery_generation=?4 AND status='assigned'",
            params![reason,store::now(),sid,delivered_generation],
        )?;
        if changed != 0 {
            tx.execute(
                "UPDATE sessions SET status='interrupted',updated_at=?1 WHERE id=?2 AND status='queued' AND EXISTS(SELECT 1 FROM harness_generations WHERE session_id=?2 AND generation=?3)",
                params![store::now(),sid,delivered_generation],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn inbox_cancel(&self, sid: &str, id: &str) -> Result<Value> {
        let mut db = self.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = record(&tx, sid, id)?
            .ok_or_else(|| HarnessError::missing("Inbox receipt is missing."))?;
        if row.status == "assigned" {
            return Err(HarnessError::conflict(
                "This follow-up was assigned; cancel the session instead.",
            ));
        }
        if ["pending", "blocked"].contains(&row.status.as_str()) {
            tx.execute(
                "UPDATE session_inbox SET status='cancelled',source_ready=0,reason='Follow-up cancelled.',updated_at=?1 WHERE id=?2",
                params![store::now(),row.id],
            )?;
        }
        let result = record(&tx, sid, id)?
            .ok_or_else(|| HarnessError::missing("Inbox receipt is missing."))?
            .receipt();
        tx.commit()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, SessionStore, Value) {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&directory.path().join("state")).unwrap();
        {
            let db = store.connection().unwrap();
            initialize(&db).unwrap();
            db.execute("INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,native_session_id,created_at,updated_at) VALUES('s','Example','p','/project','codex','plan','native',0,'running','native-1','now','now')", []).unwrap();
            db.execute(
                "INSERT INTO harness_generations(session_id,generation) VALUES('s','source-1')",
                [],
            )
            .unwrap();
        }
        let frozen = authority(&store.get("s").unwrap()).unwrap();
        (directory, store, frozen)
    }
    fn accept(store: &SessionStore, frozen: &Value, id: &str) -> Value {
        store
            .inbox_accept(
                "s",
                id,
                " Continue the task. ",
                "source-1",
                frozen,
                "scope-1",
            )
            .unwrap()
    }
    fn source_completed(store: &SessionStore) {
        store
            .connection()
            .unwrap()
            .execute("UPDATE sessions SET status='completed' WHERE id='s'", [])
            .unwrap();
        store
            .inbox_settle("s", "source-1", true, Some("native-1"))
            .unwrap();
    }
    fn event_count(store: &SessionStore) -> i64 {
        store
            .connection()
            .unwrap()
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn actual_store_reopen_preserves_input_without_replaying_pending_or_assigned() {
        for assigned in [false, true] {
            let (directory, store, frozen) = fixture();
            let accepted = accept(&store, &frozen, "request");
            if assigned {
                source_completed(&store);
                store
                    .inbox_claim("s", "request", "scope-1", &frozen)
                    .unwrap()
                    .unwrap();
            }
            let last_generation = store.generation("s").unwrap();
            let events = event_count(&store);
            drop(store);
            let reopened = SessionStore::open(&directory.path().join("state")).unwrap();
            let receipt = &reopened.inbox_list("s", None).unwrap()["items"][0];
            assert_eq!(receipt["id"], accepted["id"]);
            assert_eq!(receipt["prompt"], "Continue the task.");
            assert_eq!(
                receipt["status"],
                if assigned { "uncertain" } else { "blocked" }
            );
            assert_eq!(reopened.get("s").unwrap()["status"], "interrupted");
            assert_eq!(reopened.generation("s").unwrap(), last_generation);
            assert_eq!(event_count(&reopened), events + 1); // One restart status event, no user replay.
            assert!(reopened.inbox_pending().unwrap().is_empty());
        }
    }

    #[test]
    fn duplicate_precedes_state_capacity_and_conflicting_payload_is_refused() {
        let (_dir, store, frozen) = fixture();
        let receipt = accept(&store, &frozen, "req-1");
        assert_eq!(
            store
                .inbox_existing("s", "req-1", "Continue the task.")
                .unwrap(),
            Some(receipt.clone())
        );
        store
            .connection()
            .unwrap()
            .execute(
                "UPDATE sessions SET status='cancelled',native_session_id='other' WHERE id='s'",
                [],
            )
            .unwrap();
        assert_eq!(accept(&store, &frozen, "req-1"), receipt);
        assert_eq!(
            store
                .inbox_existing("s", "req-1", "A different task.")
                .unwrap_err()
                .status,
            409
        );
        assert!(store.inbox_existing("s", "req-1", "/command").is_err());
        assert!(!receipt.as_object().unwrap().contains_key("authority"));
    }

    #[test]
    fn one_pending_is_enforced_and_terminal_receipts_do_not_take_capacity() {
        let (_dir, store, frozen) = fixture();
        for index in 0..105 {
            let request = format!("req-{index}");
            accept(&store, &frozen, &request);
            assert_eq!(
                store
                    .inbox_accept("s", "another", "Next.", "source-1", &frozen, "scope-1")
                    .unwrap_err()
                    .status,
                409
            );
            assert_eq!(
                store.inbox_cancel("s", &request).unwrap()["status"],
                "cancelled"
            );
        }
        accept(&store, &frozen, "last");
        assert_eq!(store.inbox_pending().unwrap().len(), 1);
        let first = store.inbox_list("s", None).unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 100);
        let second = store
            .inbox_list("s", first["next_before"].as_i64())
            .unwrap();
        assert_eq!(second["items"].as_array().unwrap().len(), 6);
        assert!(second["next_before"].is_null());
    }

    #[test]
    fn claim_assigns_one_event_and_generation_atomically_and_cannot_repeat() {
        let (_dir, store, frozen) = fixture();
        accept(&store, &frozen, "request");
        assert!(
            store
                .inbox_claim("s", "request", "scope-1", &frozen)
                .unwrap()
                .is_none()
        );
        source_completed(&store);
        let turn = store
            .inbox_claim("s", "request", "scope-1", &frozen)
            .unwrap()
            .unwrap();
        assert_eq!(turn.prompt, "Continue the task.");
        assert_eq!(turn.session["status"], "queued");
        assert_ne!(turn.generation, "source-1");
        assert_eq!(event_count(&store), 1);
        assert!(
            store
                .inbox_claim("s", "request", "scope-1", &frozen)
                .unwrap()
                .is_none()
        );
        assert_eq!(event_count(&store), 1);
        let db = store.connection().unwrap();
        let raw: String = db
            .query_row("SELECT data FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&raw).unwrap()["inbox_request_id"],
            "request"
        );
    }

    #[test]
    fn claim_rolls_back_generation_event_and_session_when_receipt_write_fails() {
        let (_dir, store, frozen) = fixture();
        accept(&store, &frozen, "request");
        source_completed(&store);
        store.connection().unwrap().execute_batch("CREATE TRIGGER refuse_assignment BEFORE UPDATE OF status ON session_inbox WHEN NEW.status='assigned' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        assert!(
            store
                .inbox_claim("s", "request", "scope-1", &frozen)
                .is_err()
        );
        assert_eq!(store.generation("s").unwrap(), "source-1");
        assert_eq!(store.get("s").unwrap()["status"], "completed");
        assert_eq!(event_count(&store), 0);
        assert_eq!(store.inbox_pending().unwrap().len(), 1);
    }

    #[test]
    fn queue_capacity_does_not_consume_ready_pending_request() {
        let (_dir, store, frozen) = fixture();
        accept(&store, &frozen, "request");
        source_completed(&store);
        {
            let db = store.connection().unwrap();
            for index in 0..16 {
                db.execute("INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at) VALUES(?1,'Queued','p','/project','codex','plan','native',0,'queued','now','now')", [format!("q-{index}")]).unwrap();
            }
        }
        assert!(
            store
                .inbox_claim("s", "request", "scope-1", &frozen)
                .unwrap()
                .is_none()
        );
        assert_eq!(store.inbox_pending().unwrap()[0]["source_ready"], true);
        assert_eq!(event_count(&store), 0);
        assert_eq!(store.generation("s").unwrap(), "source-1");
    }

    #[test]
    fn global_active_capacity_does_not_reject_duplicate_receipt() {
        let (_dir, store, frozen) = fixture();
        let receipt = accept(&store, &frozen, "request");
        {
            let db = store.connection().unwrap();
            for index in 0..63 {
                db.execute("INSERT INTO session_inbox(session_id,request_id,prompt,prompt_hash,source_generation,authority,scope_stamp,status,created_at,updated_at) VALUES(?1,'request','Other','hash','source','{}','scope','assigned','now','now')", [format!("other-{index}")]).unwrap();
            }
        }
        assert_eq!(accept(&store, &frozen, "request"), receipt);
        store.inbox_cancel("s", "request").unwrap();
        {
            let db = store.connection().unwrap();
            db.execute("INSERT INTO session_inbox(session_id,request_id,prompt,prompt_hash,source_generation,authority,scope_stamp,status,created_at,updated_at) VALUES('extra','request','Other','hash','source','{}','scope','assigned','now','now')", []).unwrap();
        }
        assert_eq!(
            store
                .inbox_accept("s", "new-request", "Next.", "source-1", &frozen, "scope-1")
                .unwrap_err()
                .status,
            409
        );
        store
            .connection()
            .unwrap()
            .execute(
                "UPDATE session_inbox SET status='completed' WHERE session_id='extra'",
                [],
            )
            .unwrap();
        accept(&store, &frozen, "new-request");
        assert_eq!(store.inbox_pending().unwrap().len(), 1);
    }

    #[test]
    fn changed_scope_or_binding_blocks_instead_of_assigning() {
        for changed_scope in [false, true] {
            let (_dir, store, frozen) = fixture();
            accept(&store, &frozen, "request");
            source_completed(&store);
            if !changed_scope {
                store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE sessions SET model='different-model' WHERE id='s'",
                        [],
                    )
                    .unwrap();
            }
            assert!(
                store
                    .inbox_claim(
                        "s",
                        "request",
                        if changed_scope { "scope-2" } else { "scope-1" },
                        &frozen
                    )
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                store.inbox_list("s", None).unwrap()["items"][0]["status"],
                "blocked"
            );
            assert_eq!(event_count(&store), 0);
        }
    }

    #[test]
    fn missing_or_mismatched_native_proof_never_makes_source_ready() {
        for proof in [None, Some("other-native-id")] {
            let (_dir, store, frozen) = fixture();
            accept(&store, &frozen, "request");
            store
                .connection()
                .unwrap()
                .execute("UPDATE sessions SET status='completed' WHERE id='s'", [])
                .unwrap();
            store.inbox_settle("s", "source-1", true, proof).unwrap();
            assert!(store.inbox_pending().unwrap().is_empty());
            assert_eq!(
                store.inbox_list("s", None).unwrap()["items"][0]["status"],
                "blocked"
            );
        }
    }

    #[test]
    fn assigned_completion_requires_matching_native_proof_and_current_completion() {
        for valid in [false, true] {
            let (_dir, store, frozen) = fixture();
            accept(&store, &frozen, "request");
            source_completed(&store);
            let turn = store
                .inbox_claim("s", "request", "scope-1", &frozen)
                .unwrap()
                .unwrap();
            assert_eq!(store.inbox_cancel("s", "request").unwrap_err().status, 409);
            store
                .connection()
                .unwrap()
                .execute("UPDATE sessions SET status='completed' WHERE id='s'", [])
                .unwrap();
            store
                .inbox_settle(
                    "s",
                    &turn.generation,
                    true,
                    Some(if valid { "native-1" } else { "wrong" }),
                )
                .unwrap();
            assert_eq!(
                store.inbox_list("s", None).unwrap()["items"][0]["status"],
                if valid { "completed" } else { "uncertain" }
            );
        }
    }

    #[test]
    fn native_guard_tracks_source_and_assignment_and_refuses_conflicting_or_corrupt_bindings() {
        let (_dir, store, frozen) = fixture();
        accept(&store, &frozen, "request");
        assert_eq!(
            store
                .inbox_native_guard("s", "source-1")
                .unwrap()
                .as_deref(),
            Some("native-1")
        );
        assert!(!store.inbox_owned("s", "source-1").unwrap());
        source_completed(&store);
        let turn = store
            .inbox_claim("s", "request", "scope-1", &frozen)
            .unwrap()
            .unwrap();
        assert!(store.inbox_owned("s", &turn.generation).unwrap());
        assert_eq!(
            store
                .inbox_native_guard("s", &turn.generation)
                .unwrap()
                .as_deref(),
            Some("native-1")
        );
        let mut different = frozen.clone();
        different["native_session_id"] = "different-native".into();
        {
            let db = store.connection().unwrap();
            db.execute(
                "INSERT INTO session_inbox(session_id,request_id,prompt,prompt_hash,source_generation,authority,scope_stamp,status,created_at,updated_at) VALUES('s','next','Next','hash',?1,?2,'scope-1','pending','now','now')",
                params![turn.generation,different.to_string()],
            ).unwrap();
        }
        assert!(store.inbox_native_guard("s", &turn.generation).is_err());
        store
            .connection()
            .unwrap()
            .execute(
                "UPDATE session_inbox SET authority='{' WHERE request_id='next'",
                [],
            )
            .unwrap();
        assert!(store.inbox_native_guard("s", &turn.generation).is_err());
    }

    #[test]
    fn cancellation_and_restart_never_replay_assigned_work() {
        let (_dir, store, frozen) = fixture();
        accept(&store, &frozen, "request");
        {
            let mut db = store.connection().unwrap();
            let tx = db.transaction().unwrap();
            cancel_session(&tx, "s").unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(
            store.inbox_list("s", None).unwrap()["items"][0]["status"],
            "cancelled"
        );
        accept(&store, &frozen, "pending-after-cancel");
        recover(&store.connection().unwrap()).unwrap();
        assert_eq!(
            store.inbox_list("s", None).unwrap()["items"][0]["status"],
            "blocked"
        );
        accept(&store, &frozen, "assigned-before-restart");
        source_completed(&store);
        let turn = store
            .inbox_claim("s", "assigned-before-restart", "scope-1", &frozen)
            .unwrap()
            .unwrap();
        recover(&store.connection().unwrap()).unwrap();
        assert_eq!(
            store.inbox_list("s", None).unwrap()["items"][0]["status"],
            "uncertain"
        );
        assert!(
            store
                .inbox_claim("s", "assigned-before-restart", "scope-1", &frozen)
                .unwrap()
                .is_none()
        );
        store
            .inbox_notification_failed("s", &turn.generation, "late notification failure")
            .unwrap();
        assert_eq!(event_count(&store), 1);
    }

    #[test]
    fn notification_failure_preserves_assignment_receipt_and_interrupts_matching_queue() {
        let (_dir, store, frozen) = fixture();
        accept(&store, &frozen, "request");
        source_completed(&store);
        let turn = store
            .inbox_claim("s", "request", "scope-1", &frozen)
            .unwrap()
            .unwrap();
        store
            .inbox_notification_failed("s", &turn.generation, "Queue is stopping.")
            .unwrap();
        assert_eq!(store.get("s").unwrap()["status"], "interrupted");
        let receipt = &store.inbox_list("s", None).unwrap()["items"][0];
        assert_eq!(receipt["status"], "uncertain");
        assert_eq!(receipt["delivery_generation"], turn.generation);
        assert_eq!(event_count(&store), 1);
    }

    #[test]
    fn session_cancellation_shares_the_callers_transaction_and_preserves_uncertainty() {
        let (_dir, store, frozen) = fixture();
        accept(&store, &frozen, "request");
        source_completed(&store);
        let turn = store
            .inbox_claim("s", "request", "scope-1", &frozen)
            .unwrap()
            .unwrap();
        {
            let mut db = store.connection().unwrap();
            let tx = db.transaction().unwrap();
            cancel_session(&tx, "s").unwrap();
            tx.rollback().unwrap();
        }
        assert!(store.inbox_owned("s", &turn.generation).unwrap());
        {
            let mut db = store.connection().unwrap();
            let tx = db.transaction().unwrap();
            cancel_session(&tx, "s").unwrap();
            tx.commit().unwrap();
        }
        assert!(!store.inbox_owned("s", &turn.generation).unwrap());
        assert_eq!(
            store.inbox_list("s", None).unwrap()["items"][0]["status"],
            "uncertain"
        );
        store
            .inbox_settle("s", &turn.generation, true, Some("native-1"))
            .unwrap();
        assert_eq!(
            store.inbox_list("s", None).unwrap()["items"][0]["status"],
            "uncertain"
        );
    }

    #[test]
    fn authority_and_request_validation_refuse_unscoped_or_specialized_inputs() {
        let (_dir, store, _) = fixture();
        let session = store.get("s").unwrap();
        for key in SPECIAL {
            let mut changed = session.clone();
            changed[*key] = json!({"enabled":true});
            assert!(authority(&changed).is_err(), "{key}");
        }
        for (id, prompt) in [
            ("bad/id", "Task"),
            (".", "Task"),
            ("..", "Task"),
            ("valid", " /command"),
            ("valid", "\0"),
            ("é", "Task"),
        ] {
            assert!(validate_request(id, prompt).is_err());
        }
        assert!(validate_request("valid", &"é".repeat(16001)).is_err());
        assert_eq!(validate_request("valid-_.1", " Task ").unwrap(), "Task");
    }
}
