use super::{
    export::{self, PublicExport, RetainDraft},
    recall::{Catalogue, Scope, SourceBinding},
    screen::{PublicPolicy, active},
    wire,
};
use crate::strict;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
pub const PROFILE: &str = "hindsight-ingestion-journal-v1";
pub const QUIESCENT: &str = "caller-attested-operation-quiescent";
pub const VISIBLE: &str = "caller-attested-full-visibility-and-quiescence";
const SCHEMA: [&str; 4] = [
    "CREATE TABLE journal (singleton INTEGER PRIMARY KEY CHECK(singleton=1), profile TEXT NOT NULL, journal_id TEXT NOT NULL, team TEXT NOT NULL, project TEXT NOT NULL, head INTEGER NOT NULL CHECK(head>=0 AND head<=16))",
    "CREATE TABLE generations (revision INTEGER PRIMARY KEY CHECK(revision>=1 AND revision<=16), generation TEXT NOT NULL UNIQUE, manifest BLOB NOT NULL CHECK(length(manifest)<=65536), digest TEXT NOT NULL CHECK(length(digest)=64))",
    "CREATE TABLE intents (identity TEXT PRIMARY KEY NOT NULL, revision INTEGER NOT NULL REFERENCES generations(revision), ordinal INTEGER NOT NULL CHECK(ordinal>=0 AND ordinal<32), binding BLOB NOT NULL CHECK(length(binding)<=2048), logical TEXT NOT NULL, body BLOB NOT NULL CHECK(length(body)>0 AND length(body)<=262144), digest TEXT NOT NULL CHECK(length(digest)=64), UNIQUE(revision,ordinal))",
    "CREATE TABLE attempts (nonce TEXT PRIMARY KEY NOT NULL, intent TEXT NOT NULL UNIQUE REFERENCES intents(identity), outcome TEXT NOT NULL CHECK(outcome IN ('recorded','success','definite_no_write','settled_unknown')))",
];
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub revision: usize,
    pub generation: Uuid,
    pub manifest: Vec<u8>,
    pub intent_ids: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub journal_id: Uuid,
    pub nonce: Uuid,
    pub intent_id: String,
    pub revision: usize,
    pub body_sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    pub journal_id: Uuid,
    pub nonce: Uuid,
    pub epoch: Uuid,
    pub revision: usize,
    pub generation: Uuid,
    pub manifest_sha256: String,
    pub attempts_sha256: String,
    pub expected: Vec<SourceBinding>,
}
#[derive(Clone, Debug)]
pub struct DocumentObservation {
    pub binding: SourceBinding,
    pub state: String,
}
fn binding_image(v: &Value, scope: &Scope) -> Result<SourceBinding> {
    strict::keys(
        v,
        &["document_id", "path", "source_sha256"],
        &["document_id", "path", "source_sha256"],
    )?;
    let b = SourceBinding {
        scope: scope.clone(),
        document_id: v["document_id"].as_str().context("corrupt")?.into(),
        path: v["path"].as_str().context("corrupt")?.into(),
        source_sha256: v["source_sha256"].as_str().context("corrupt")?.into(),
    };
    b.validate_integrity()?;
    Ok(b)
}
fn intent_id(journal: Uuid, generation: Uuid, d: &RetainDraft) -> Result<String> {
    Ok(format!(
        "intent-{}",
        strict::sha(&wire::value(&json!([
            PROFILE,
            journal.to_string(),
            generation.to_string(),
            d.binding.document_id,
            strict::sha(&d.body)
        ]))?)
    ))
}
fn descriptor(journal: Uuid, generation: Uuid, d: &RetainDraft) -> Result<Vec<u8>> {
    wire::object(vec![
        ("document_id", wire::value(&d.binding.document_id)?),
        ("path", wire::value(&d.binding.path)?),
        ("source_sha256", wire::value(&d.binding.source_sha256)?),
        ("source_id", wire::value(&d.source_id)?),
        ("body_sha256", wire::value(&strict::sha(&d.body))?),
        ("body_bytes", wire::value(&d.body.len())?),
        (
            "intent_id",
            wire::value(&intent_id(journal, generation, d)?)?,
        ),
    ])
}
struct Loaded {
    head: usize,
    generations: BTreeMap<usize, (Uuid, Vec<u8>, Value)>,
    owned: BTreeMap<String, (usize, RetainDraft)>,
    attempts: Vec<(Uuid, String, String)>,
    size: usize,
}
fn load(db: &Connection, scope: &Scope, journal: Uuid) -> Result<Loaded> {
    let mut tables =
        db.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
    let names = tables
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (name, expected) in ["journal", "generations", "intents", "attempts"]
        .iter()
        .zip(SCHEMA)
    {
        let sql: String = db.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?",
            [name],
            |r| r.get(0),
        )?;
        let normalize = |s: &str| {
            s.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        };
        if normalize(&sql) != normalize(expected) {
            bail!("schema");
        }
    }
    if names != ["attempts", "generations", "intents", "journal"]
        || db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('trigger','view')",
            [],
            |r| r.get::<_, usize>(0),
        )? != 0
    {
        bail!("schema");
    }
    let rows = db.query_row("SELECT COUNT(*) FROM journal", [], |r| r.get::<_, usize>(0))?;
    if rows != 1 {
        bail!("corrupt");
    }
    let meta = db.query_row(
        "SELECT singleton,profile,journal_id,team,project,head FROM journal",
        [],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, usize>(5)?,
            ))
        },
    )?;
    if meta.0 != 1
        || meta.1 != PROFILE
        || meta.2 != journal.to_string()
        || meta.3 != scope.team_id.to_string()
        || meta.4 != scope.project_id.to_string()
        || meta.5 > 16
    {
        bail!("corrupt");
    }
    let mut generations = BTreeMap::new();
    let mut generation_ids = BTreeSet::new();
    let mut size = 0;
    let mut statement = db
        .prepare("SELECT revision,generation,manifest,digest FROM generations ORDER BY revision")?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, usize>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (revision, generation, raw, digest) in rows {
        let id = strict::uuid(&generation)?;
        let v = strict::decode(&raw, 65536, 16, 4096)?;
        strict::keys(
            &v,
            &[
                "profile",
                "journal_id",
                "scope",
                "generation",
                "revision",
                "export_sha256",
                "current",
                "previous",
            ],
            &[
                "profile",
                "journal_id",
                "scope",
                "generation",
                "revision",
                "export_sha256",
                "current",
                "previous",
            ],
        )?;
        if revision != generations.len() + 1
            || !generation_ids.insert(id)
            || strict::sha(&raw) != digest
            || v["profile"] != PROFILE
            || v["journal_id"] != journal.to_string()
            || v["scope"] != scope.key()
            || v["generation"] != generation
            || v["revision"].as_u64() != Some(revision as u64)
            || v["current"].as_array().is_none_or(|a| a.len() > 32)
            || v["previous"].as_array().is_none_or(|a| a.len() > 32)
        {
            bail!("corrupt");
        }
        for image in v["previous"].as_array().unwrap() {
            binding_image(image, scope)?;
        }
        size += raw.len();
        generations.insert(revision, (id, raw, v));
    }
    if generations.len() != meta.5 {
        bail!("corrupt");
    }
    let mut owned = BTreeMap::new();
    let mut by_revision = BTreeMap::<usize, Vec<RetainDraft>>::new();
    let mut stmt=db.prepare("SELECT identity,revision,ordinal,binding,logical,body,digest FROM intents ORDER BY revision,ordinal")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, usize>(1)?,
                r.get::<_, usize>(2)?,
                r.get::<_, Vec<u8>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Vec<u8>>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if rows.len() > 512 {
        bail!("capacity");
    }
    for (identity, revision, ordinal, binding, logical, body, digest) in rows {
        let (generation, _, v) = generations.get(&revision).context("corrupt")?;
        let b = binding_image(&strict::decode(&binding, 2048, 8, 64)?, scope)?;
        let d = RetainDraft {
            binding: b,
            source_id: logical,
            body,
        };
        d.text()?;
        let group = by_revision.entry(revision).or_default();
        if ordinal != group.len()
            || ordinal >= 32
            || strict::sha(&d.body) != digest
            || identity != intent_id(journal, *generation, &d)?
            || v["current"].get(ordinal)
                != Some(&strict::decode(
                    &descriptor(journal, *generation, &d)?,
                    4096,
                    8,
                    128,
                )?)
        {
            bail!("corrupt");
        }
        size += binding.len() + d.body.len();
        group.push(d.clone());
        if owned.insert(identity, (revision, d)).is_some() {
            bail!("corrupt");
        }
    }
    for (revision, (_, _, v)) in &generations {
        let drafts = by_revision.get(revision).cloned().unwrap_or_default();
        if drafts.len() != v["current"].as_array().unwrap().len() {
            bail!("corrupt");
        }
        let catalogue = Catalogue::new(
            scope.clone(),
            drafts.iter().map(|d| d.binding.clone()).collect(),
        )?;
        if v["export_sha256"] != strict::sha(&export::packet(&catalogue, &drafts)?) {
            bail!("corrupt");
        }
    }
    let mut stmt = db.prepare("SELECT nonce,intent,outcome FROM attempts ORDER BY nonce")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if rows.len() > 512 || size > 8388608 {
        bail!("capacity");
    }
    let mut attempts = vec![];
    let mut pending = BTreeSet::new();
    let mut intents = BTreeSet::new();
    for (nonce, id, outcome) in rows {
        let nonce = strict::uuid(&nonce)?;
        let d = &owned.get(&id).context("corrupt")?.1;
        if ![
            "recorded",
            "success",
            "definite_no_write",
            "settled_unknown",
        ]
        .contains(&outcome.as_str())
            || !intents.insert(id.clone())
            || outcome == "recorded" && !pending.insert(d.source_id.clone())
        {
            bail!("corrupt");
        }
        attempts.push((nonce, id, outcome));
    }
    Ok(Loaded {
        head: meta.5,
        generations,
        owned,
        attempts,
        size,
    })
}
pub struct Journal {
    connection: Connection,
    pub scope: Scope,
    pub journal_id: Uuid,
    epoch: Uuid,
    issued: BTreeMap<Uuid, Challenge>,
    used: BTreeSet<Uuid>,
}
impl Journal {
    fn new(connection: Connection, scope: Scope, id: &str) -> Result<Self> {
        if !connection.is_autocommit() {
            bail!("profile");
        }
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "trusted_schema", false)?;
        Ok(Self {
            connection,
            scope,
            journal_id: strict::uuid(id)?,
            epoch: Uuid::new_v4(),
            issued: BTreeMap::new(),
            used: BTreeSet::new(),
        })
    }
    pub fn create_new(connection: Connection, scope: Scope, id: &str) -> Result<Self> {
        let mut j = Self::new(connection, scope, id)?;
        let tx = j
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| {
            r.get::<_, usize>(0)
        })? != 0
        {
            bail!("schema");
        }
        for sql in SCHEMA {
            tx.execute_batch(sql)?;
        }
        tx.execute(
            "INSERT INTO journal VALUES (1,?,?,?,?,0)",
            params![
                PROFILE,
                j.journal_id.to_string(),
                j.scope.team_id.to_string(),
                j.scope.project_id.to_string()
            ],
        )?;
        load(&tx, &j.scope, j.journal_id)?;
        tx.commit()?;
        Ok(j)
    }
    pub fn open_existing(connection: Connection, scope: Scope, id: &str) -> Result<Self> {
        let mut j = Self::new(connection, scope, id)?;
        let tx = j
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        load(&tx, &j.scope, j.journal_id)?;
        tx.commit()?;
        Ok(j)
    }
    pub fn prepare<P: PublicPolicy>(
        &mut self,
        selection: &PublicExport,
        generation: &str,
        expected_revision: usize,
        previous: Option<&Catalogue>,
        policy: &P,
        a: &mut dyn FnMut() -> bool,
    ) -> Result<Prepared> {
        let generation = strict::uuid(generation)?;
        if expected_revision > 16 || selection.catalogue.scope != self.scope {
            bail!("input");
        }
        selection.validate(policy, a)?;
        let previous = previous
            .map(|c| {
                if c.scope != self.scope
                    || c.sources
                        .iter()
                        .map(|b| &b.path)
                        .collect::<BTreeSet<_>>()
                        .len()
                        != c.sources.len()
                {
                    bail!("input");
                }
                for b in &c.sources {
                    b.validate_integrity()?;
                }
                Ok(c.sources.clone())
            })
            .transpose()?
            .unwrap_or_default();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        active(a)?;
        let state = load(&tx, &self.scope, self.journal_id)?;
        active(a)?;
        let replay = state.head > 0 && state.generations[&state.head].0 == generation;
        let revision = if replay { state.head } else { state.head + 1 };
        let current = selection
            .retain_drafts
            .iter()
            .map(|d| descriptor(self.journal_id, generation, d))
            .collect::<Result<Vec<_>>>()?;
        let old = previous
            .iter()
            .map(SourceBinding::image_wire)
            .collect::<Result<Vec<_>>>()?;
        let manifest = wire::object(vec![
            ("profile", wire::value(PROFILE)?),
            ("journal_id", wire::value(&self.journal_id.to_string())?),
            ("scope", wire::value(&self.scope.key())?),
            ("generation", wire::value(&generation.to_string())?),
            ("revision", wire::value(&revision)?),
            (
                "export_sha256",
                wire::value(&strict::sha(&selection.serialized))?,
            ),
            ("current", wire::array(current)),
            ("previous", wire::array(old)),
        ])?;
        if manifest.len() > 65536 {
            bail!("capacity");
        }
        if replay {
            if expected_revision != state.head || manifest != state.generations[&state.head].1 {
                bail!("conflict");
            }
        } else {
            let added = manifest.len()
                + selection
                    .retain_drafts
                    .iter()
                    .map(|d| {
                        d.body.len()
                            + d.binding
                                .image_wire()
                                .map(|b| b.len())
                                .unwrap_or(usize::MAX)
                    })
                    .sum::<usize>();
            if expected_revision != state.head
                || state
                    .generations
                    .values()
                    .any(|(id, _, _)| *id == generation)
            {
                bail!("conflict");
            }
            if revision > 16
                || state.owned.len() + selection.retain_drafts.len() > 512
                || state.size.checked_add(added).is_none_or(|n| n > 8388608)
            {
                bail!("capacity");
            }
            tx.execute(
                "INSERT INTO generations VALUES (?,?,?,?)",
                params![
                    revision,
                    generation.to_string(),
                    &manifest,
                    strict::sha(&manifest)
                ],
            )?;
            for (ordinal, d) in selection.retain_drafts.iter().enumerate() {
                tx.execute(
                    "INSERT INTO intents VALUES (?,?,?,?,?,?,?)",
                    params![
                        intent_id(self.journal_id, generation, d)?,
                        revision,
                        ordinal,
                        d.binding.image_wire()?,
                        d.source_id,
                        &d.body,
                        strict::sha(&d.body)
                    ],
                )?;
            }
            tx.execute("UPDATE journal SET head=? WHERE singleton=1", [revision])?;
        }
        let result = Prepared {
            revision,
            generation,
            manifest,
            intent_ids: selection
                .retain_drafts
                .iter()
                .map(|d| intent_id(self.journal_id, generation, d))
                .collect::<Result<_>>()?,
        };
        active(a)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn current(&mut self) -> Result<Option<Prepared>> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let s = load(&tx, &self.scope, self.journal_id)?;
        let result = if s.head == 0 {
            None
        } else {
            let (generation, raw, v) = &s.generations[&s.head];
            Some(Prepared {
                revision: s.head,
                generation: *generation,
                manifest: raw.clone(),
                intent_ids: v["current"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v["intent_id"].as_str().unwrap().into())
                    .collect(),
            })
        };
        tx.commit()?;
        Ok(result)
    }
    pub fn request_body<P: PublicPolicy>(
        &mut self,
        id: &str,
        policy: &P,
        a: &mut dyn FnMut() -> bool,
    ) -> Result<Vec<u8>> {
        if id.len() != 71 {
            bail!("input");
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        active(a)?;
        let s = load(&tx, &self.scope, self.journal_id)?;
        let d = &s.owned.get(id).context("input")?.1;
        d.validate(policy, a)?;
        let body = d.body.clone();
        active(a)?;
        tx.commit()?;
        Ok(body)
    }
    pub fn reserve_attempt<P: PublicPolicy>(
        &mut self,
        id: &str,
        nonce: &str,
        expected_revision: usize,
        policy: &P,
        a: &mut dyn FnMut() -> bool,
    ) -> Result<Ticket> {
        let nonce = strict::uuid(nonce)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        active(a)?;
        let s = load(&tx, &self.scope, self.journal_id)?;
        let (revision, d) = s.owned.get(id).context("input")?;
        if s.head != expected_revision || *revision != s.head {
            bail!("conflict");
        }
        if s.attempts.iter().any(|(n, i, _)| *n == nonce || i == id) {
            bail!("admitted");
        }
        if s.attempts
            .iter()
            .any(|(_, i, o)| o == "recorded" && s.owned[i].1.source_id == d.source_id)
        {
            bail!("pending");
        }
        if s.attempts.len() >= 512 {
            bail!("capacity");
        }
        d.validate(policy, a)?;
        tx.execute(
            "INSERT INTO attempts VALUES (?,?,?)",
            params![nonce.to_string(), id, "recorded"],
        )?;
        let ticket = Ticket {
            journal_id: self.journal_id,
            nonce,
            intent_id: id.into(),
            revision: *revision,
            body_sha256: strict::sha(&d.body),
        };
        active(a)?;
        tx.commit()?;
        Ok(ticket)
    }
    pub fn attest_terminal(&mut self, ticket: &Ticket, outcome: &str, profile: &str) -> Result<()> {
        if profile != QUIESCENT
            || !["success", "definite_no_write", "settled_unknown"].contains(&outcome)
            || ticket.journal_id != self.journal_id
        {
            bail!("settlement");
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let s = load(&tx, &self.scope, self.journal_id)?;
        let (revision, d) = s.owned.get(&ticket.intent_id).context("ticket")?;
        let found = s
            .attempts
            .iter()
            .find(|(n, _, _)| *n == ticket.nonce)
            .context("ticket")?;
        if found.1 != ticket.intent_id
            || *revision != ticket.revision
            || strict::sha(&d.body) != ticket.body_sha256
        {
            bail!("ticket");
        }
        if found.2 != "recorded" && found.2 != outcome {
            bail!("conflict");
        }
        tx.execute(
            "UPDATE attempts SET outcome=? WHERE nonce=?",
            params![outcome, ticket.nonce.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
    fn image_challenge(
        journal_id: Uuid,
        epoch: Uuid,
        scope: &Scope,
        nonce: Uuid,
        s: &Loaded,
    ) -> Result<Challenge> {
        if s.head == 0 {
            bail!("challenge");
        }
        let (generation, raw, v) = &s.generations[&s.head];
        let mut expected = BTreeMap::new();
        for image in v["current"]
            .as_array()
            .unwrap()
            .iter()
            .chain(v["previous"].as_array().unwrap())
        {
            let b = binding_image(
                &json!({"document_id":image["document_id"],"path":image["path"],"source_sha256":image["source_sha256"]}),
                scope,
            )?;
            expected.insert(b.document_id.clone(), b);
        }
        for (_, id, _) in &s.attempts {
            let b = s.owned[id].1.binding.clone();
            expected.insert(b.document_id.clone(), b);
        }
        if expected.len() > 64 {
            bail!("capacity");
        }
        let frontier = s
            .attempts
            .iter()
            .map(|(n, i, o)| {
                wire::object(vec![
                    ("nonce", wire::value(&n.to_string())?),
                    ("intent_id", wire::value(i)?),
                    ("outcome", wire::value(o)?),
                ])
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Challenge {
            journal_id,
            nonce,
            epoch,
            revision: s.head,
            generation: *generation,
            manifest_sha256: strict::sha(raw),
            attempts_sha256: strict::sha(&wire::array(frontier)),
            expected: expected.into_values().collect(),
        })
    }
    pub fn challenge(&mut self, nonce: &str) -> Result<Challenge> {
        let nonce = strict::uuid(nonce)?;
        if self.issued.contains_key(&nonce)
            || self.used.contains(&nonce)
            || self.issued.len() + self.used.len() >= 32
        {
            bail!("challenge");
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.scope, self.journal_id)?;
        let c = Self::image_challenge(self.journal_id, self.epoch, &self.scope, nonce, &state)?;
        tx.commit()?;
        self.issued.insert(nonce, c.clone());
        Ok(c)
    }
    pub fn evaluate(
        &mut self,
        c: &Challenge,
        observations: &[DocumentObservation],
        profile: Option<&str>,
    ) -> Result<Value> {
        if self.issued.remove(&c.nonce).as_ref() != Some(c) {
            bail!("challenge");
        }
        self.used.insert(c.nonce);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let s = load(&tx, &self.scope, self.journal_id)?;
        if *c != Self::image_challenge(self.journal_id, self.epoch, &self.scope, c.nonce, &s)? {
            bail!("challenge");
        }
        let observed = observations
            .iter()
            .map(|o| (&o.binding.document_id, o))
            .collect::<BTreeMap<_, _>>();
        if observations.len() != c.expected.len() || observed.len() != observations.len() {
            bail!("observation");
        }
        for b in &c.expected {
            let o = observed.get(&b.document_id).context("observation")?;
            if o.binding != *b
                || !["observed_not_found", "source_match", "unknown", "conflict"]
                    .contains(&o.state.as_str())
            {
                bail!("observation");
            }
        }
        let current = s.generations[&s.head].2["current"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v["document_id"].as_str())
            .collect::<BTreeSet<_>>();
        let missing = observations
            .iter()
            .filter(|o| {
                o.state == "observed_not_found" && current.contains(o.binding.document_id.as_str())
            })
            .count();
        let obsolete = observations
            .iter()
            .filter(|o| {
                o.state == "source_match" && !current.contains(o.binding.document_id.as_str())
            })
            .count();
        let unknown = observations.iter().filter(|o| o.state == "unknown").count();
        let conflict = observations
            .iter()
            .filter(|o| o.state == "conflict")
            .count();
        let pending = s
            .attempts
            .iter()
            .filter(|(_, _, o)| o == "recorded")
            .count();
        let decision = if unknown + conflict + pending > 0 || profile != Some(VISIBLE) {
            "incomplete"
        } else if obsolete > 0 {
            "cleanup_required"
        } else if missing > 0 {
            "missing_source_images"
        } else {
            "supplied_selection_current"
        };
        let result = json!({"state":decision,"expected_count":observations.len(),"missing_current":missing,"obsolete_present":obsolete,"unknown":unknown,"conflict":conflict,"pending_attempts":pending,"authorized_mutations":0,"actual_remote_noop":null,"executed_transport_calls":0,"physical_sqlite_io_bytes":null,"provider_tokens":null,"remote_usd":null,"total_cost_complete":false});
        tx.commit()?;
        Ok(result)
    }
    pub fn diagnostics(&mut self) -> Result<Value> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let s = load(&tx, &self.scope, self.journal_id)?;
        let result = json!({"profile":PROFILE,"revision":s.head,"generations":s.head,"intents":s.owned.len(),"recorded_attempts":s.attempts.len(),"pending_attempts":s.attempts.iter().filter(|(_,_,o)|o=="recorded").count(),"persisted_payload_bytes":s.size,"physical_sqlite_io_bytes":null,"executed_transport_calls":0,"provider_tokens":null,"remote_usd":null,"total_cost_complete":false});
        tx.commit()?;
        Ok(result)
    }
}
pub fn observe_document(
    binding: SourceBinding,
    status: u16,
    body: &[u8],
    eof: bool,
) -> Result<DocumentObservation> {
    binding.validate_integrity()?;
    let state = (|| -> Result<&str> {
        if body.len() > 262144 || !eof {
            return Ok("unknown");
        }
        if status == 404 {
            return Ok("observed_not_found");
        }
        if status != 200 {
            return Ok("unknown");
        }
        let v = strict::decode(body, 262144, 16, 4096)?;
        if ["id", "bank_id", "original_text"]
            .iter()
            .any(|k| !v[*k].is_string())
            || !v["document_metadata"].is_object()
            || !v["retain_params"].is_object()
            || v["tags"]
                .as_array()
                .is_none_or(|a| a.iter().any(|x| !x.is_string()))
        {
            return Ok("unknown");
        }
        let wanted = json!({
            "harness_scope": binding.scope.key(),
            "source_sha256": binding.source_sha256,
            "source_id": format!(
                "logical-{}",
                strict::sha(&wire::value(&[binding.scope.key(), binding.path.clone()])?)
            ),
            "export_profile": export::PROFILE
        });
        let meta = v["document_metadata"].as_object().unwrap();
        let Some(params) = v["retain_params"]["metadata"].as_object() else {
            return Ok("unknown");
        };
        if wanted
            .as_object()
            .unwrap()
            .keys()
            .any(|k| !meta.contains_key(k) || !params.contains_key(k))
        {
            return Ok("unknown");
        }
        if v["id"] != binding.document_id
            || v["bank_id"] != binding.scope.bank_id()
            || strict::sha(v["original_text"].as_str().unwrap().as_bytes()) != binding.source_sha256
            || v["tags"] != json!([binding.scope.tag()])
            || wanted
                .as_object()
                .unwrap()
                .iter()
                .any(|(k, v)| meta[k] != *v || params[k] != *v)
        {
            return Ok("conflict");
        }
        Ok("source_match")
    })()
    .unwrap_or("unknown");
    Ok(DocumentObservation {
        binding,
        state: state.into(),
    })
}
