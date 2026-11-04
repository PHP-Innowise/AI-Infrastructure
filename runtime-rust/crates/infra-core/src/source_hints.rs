//! Optional source hints select only existing indexed knowledge and never
//! create public authority records from arbitrary files.
use crate::{
    Result, RuntimeConfig, RuntimeContext, RuntimeError,
    domain::{self, json},
    index::{DOCUMENT_SELECT, document_from_row},
    observability::Trace,
    query::{self, Candidate},
    source_policy::{self, CurrentSourcePolicy},
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::collections::BTreeSet;
pub const PROFILE: &str = "source-hints-v1";
pub const LIMIT: usize = 8;
pub const BYTES: usize = 8192;
pub const FILE_BYTES: usize = 131072;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reference {
    pub path: String,
    pub source_sha256: String,
}
fn exact(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|map| map.len() == keys.len() && keys.iter().all(|key| map.contains_key(*key)))
}
pub fn normalize(value: &Value) -> (Vec<Reference>, &'static str) {
    let operation = (|| -> Result<Vec<Reference>> {
        let parsed;
        if let Some(text) = value.as_str() {
            if text.len() > BYTES || text.chars().count() > BYTES {
                return Err(RuntimeError::invalid("Hint input bound"));
            }
            parsed = crate::strict_json::parse(text.as_bytes(), BYTES, 128, 4096)?;
        } else {
            parsed = value.clone();
        }
        if !exact(&parsed, &["schema_version", "references"])
            || parsed["schema_version"].as_u64() != Some(1)
        {
            return Err(RuntimeError::invalid("Hint profile invalid"));
        }
        let entries = parsed["references"]
            .as_array()
            .filter(|entries| entries.len() <= LIMIT)
            .ok_or_else(|| RuntimeError::invalid("Hint references invalid"))?;
        static CONTROLS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let controls = CONTROLS
            .get_or_init(|| regex::Regex::new(r"\p{C}").expect("constant hint path grammar"));
        let mut clean = vec![];
        for entry in entries {
            if !exact(entry, &["path", "source_sha256"]) {
                return Err(RuntimeError::invalid("Hint reference invalid"));
            }
            let name = entry["path"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Hint path invalid"))?;
            let digest = entry["source_sha256"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Hint digest invalid"))?;
            if name.is_empty()
                || name.len() > 512
                || name.chars().count() > 512
                || name.starts_with('/')
                || name.contains(['\\', ':', '#'])
                || controls.is_match(name)
                || name
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
                || domain::source_path_problem(&format!("hints/{name}")).is_some()
                || !crate::observability::hex(digest)
            {
                return Err(RuntimeError::invalid("Hint path or digest invalid"));
            }
            clean.push(Reference {
                path: name.into(),
                source_sha256: digest.into(),
            });
        }
        let normalized = object!({"schema_version":1,"references":clean});
        if json::serialize(&normalized, false, false, false, false)?.len() > BYTES {
            return Err(RuntimeError::invalid("Hint normalized byte bound"));
        }
        serde_json::from_value(normalized["references"].clone()).map_err(Into::into)
    })();
    match operation {
        Ok(references) => (references, "ready"),
        Err(_) => (vec![], "invalid"),
    }
}
pub fn schema_supported(context: &RuntimeContext) -> bool {
    let operation = (|| -> Result<bool> {
        let mut path = context
            .roots
            .brain_root()
            .join("schemas/retrieval-manifest.schema.json");
        if !path.is_file() {
            path = context
                .roots
                .tooling
                .join("project-brain/schemas/retrieval-manifest.schema.json");
        }
        let schema = json::parse(&crate::fs::read_text(&path)?)?;
        let properties = &schema["properties"];
        if properties["selected"]["items"]["properties"]["selection"]["enum"]
            .as_array()
            .is_none_or(|values| !values.iter().any(|value| value == "source-hint"))
            || properties["source_hints"]["properties"]["profile"]["const"] != PROFILE
        {
            return Ok(false);
        }
        for state in ["ready", "invalid", "unsupported"] {
            for count in [0, LIMIT] {
                domain::validate_schema_value(
                    &object!({"profile":PROFILE,"state":state,"supplied":count,"admitted":count,"delivered":count}),
                    &properties["source_hints"],
                )?;
            }
        }
        domain::validate_schema_value(
            &object!({"path":"specs/source.md","category":"evidence","estimated_tokens":1,"source_hash":"a".repeat(64),"selection":"source-hint","score":0.0,"rank":null}),
            &properties["selected"]["items"],
        )?;
        Ok(true)
    })();
    operation.unwrap_or(false)
}
pub fn candidates(
    connection: &Connection,
    context: &RuntimeContext,
    config: &RuntimeConfig,
    references: &[Reference],
    seen: &mut BTreeSet<String>,
    policy: &mut CurrentSourcePolicy,
    mut trace: Option<&mut Trace>,
) -> Result<Vec<Candidate>> {
    let mut candidates = vec![];
    for reference in references {
        let name = &reference.path;
        if seen.contains(name) {
            continue;
        }
        // Exclude large cached bodies in SQL before hydrating even one byte.
        let metadata_select =
            DOCUMENT_SELECT.replace("d.summary,d.content", "'' AS summary,'' AS content");
        let mut statement=connection.prepare(&format!("SELECT {metadata_select} FROM documents d JOIN document_metadata m ON m.path=d.path WHERE d.path=?1 LIMIT 2"))?;
        let mut rows = statement
            .query_map([name], document_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if rows.len() != 1 {
            continue;
        }
        let metadata = rows.remove(0);
        if metadata.layer != "semantic"
            || metadata.category == "policy"
            || !metadata.conflicts.is_empty()
            || crate::source_links::incoming_conflict(connection, metadata.record_id.as_deref())?
        {
            continue;
        }
        if source_policy::raw_hint_read(context, name, &reference.source_sha256)?.is_none() {
            continue;
        }
        let mut statement=connection.prepare(&format!("SELECT {DOCUMENT_SELECT} FROM documents d JOIN document_metadata m ON m.path=d.path WHERE d.path=?1 AND length(CAST(d.content AS BLOB))<=?2 LIMIT 2"))?;
        let mut rows = statement
            .query_map(params![name, FILE_BYTES], document_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if rows.len() != 1 {
            continue;
        }
        let document = rows.remove(0);
        if document.layer != "semantic"
            || document.category == "policy"
            || !document.conflicts.is_empty()
            || crate::source_links::incoming_conflict(connection, document.record_id.as_deref())?
        {
            continue;
        }
        if policy
            .image_source_hash(name)
            .is_some_and(|digest| digest != document.source_hash)
        {
            continue;
        }
        if policy.image(name).is_some_and(|image| {
            image.content.len() > FILE_BYTES || image.content.chars().count() > FILE_BYTES
        }) {
            continue;
        }
        let mut item = Candidate::linked(document, "source-hint");
        item.hint_raw_sha256 = Some(reference.source_sha256.clone());
        item.match_strength = None;
        item.adjusted_score = 0.0;
        item.score = 0.0;
        item.rank = None;
        if let Some(image) = policy.image(name) {
            item.held_image = Some(image.clone());
            item.held_source_hash = policy.image_source_hash(name).map(String::from);
        }
        let (eligible, _) = source_policy::filter_in_scope(
            context,
            config,
            connection,
            policy,
            vec![item],
            false,
            trace.as_deref_mut(),
        )?;
        for mut item in eligible {
            item.snippet = query::body_excerpt(&item.image().content, 320, &[], false);
            item.estimated_tokens =
                query::estimated_tokens(&format!("{}{}", item.document.title, item.snippet));
            seen.insert(item.document.path.clone());
            candidates.push(item);
        }
    }
    Ok(candidates)
}
