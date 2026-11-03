//! Explicit factual projections, separate from prompt-time conflict presentation.
use crate::{Result, artifact};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub fn public(text: &str) -> Result<()> {
    if infra_core::domain::sensitive_label(text)?.is_some() {
        return Err(artifact::invalid("sensitive pilot input refused"));
    }
    Ok(())
}
pub fn instant(value: &str) -> Result<DateTime<Utc>> {
    let at = DateTime::parse_from_rfc3339(value)
        .map_err(|_| artifact::invalid("aware ISO instant required"))?
        .with_timezone(&Utc);
    if at.year() < 1 || at.year() > 9999 {
        return Err(artifact::invalid("timestamp outside supported range"));
    }
    Ok(at)
}
use chrono::Datelike;
pub fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, false)
}
fn timestamp(value: &Value) -> Result<DateTime<Utc>> {
    if value.is_null() || value == "" {
        return Ok(Utc.with_ymd_and_hms(1, 1, 1, 0, 0, 0).single().unwrap());
    }
    let s = value
        .as_str()
        .ok_or_else(|| artifact::invalid("timestamp must be ISO date or aware instant"))?;
    if s.len() == 10 {
        return day(s, false);
    }
    instant(s)
}
pub fn day(value: &str, ceiling: bool) -> Result<DateTime<Utc>> {
    let day = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| artifact::invalid("invalid day timestamp"))?;
    if value.len() != 10 {
        return Err(artifact::invalid("day precision required"));
    }
    let day = if ceiling {
        day.succ_opt()
            .ok_or_else(|| artifact::invalid("day overflow"))?
    } else {
        day
    };
    Ok(Utc.from_utc_datetime(
        &day.and_hms_opt(0, 0, 0)
            .ok_or_else(|| artifact::invalid("day time"))?,
    ))
}
fn version(row: &Value) -> Option<String> {
    for key in ["serial", "version", "revision"] {
        if let Value::Number(n) = &row[key] {
            let s = n.to_string();
            if !s.contains(['.', 'e', 'E']) {
                return Some(s);
            }
        }
    }
    None
}
fn integer_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let na = a.starts_with('-');
    let nb = b.starts_with('-');
    if na != nb {
        return if na {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        };
    }
    let x = a.trim_start_matches('-').trim_start_matches('0');
    let y = b.trim_start_matches('-').trim_start_matches('0');
    let c = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
    if na { c.reverse() } else { c }
}
fn rank_cmp(a: &Value, b: &Value) -> Result<std::cmp::Ordering> {
    let av = version(a);
    let bv = version(b);
    let ordering = av
        .is_some()
        .cmp(&bv.is_some())
        .then_with(|| match (&av, &bv) {
            (Some(a), Some(b)) => integer_cmp(a, b),
            _ => std::cmp::Ordering::Equal,
        });
    Ok(if ordering.is_eq() {
        timestamp(
            a.get("updated_at")
                .or_else(|| a.get("at"))
                .unwrap_or(&Value::Null),
        )?
        .cmp(&timestamp(
            b.get("updated_at")
                .or_else(|| b.get("at"))
                .unwrap_or(&Value::Null),
        )?)
    } else {
        ordering
    })
}
pub fn resolve_facts(document: &Value) -> Result<Value> {
    let rows = document
        .as_array()
        .or_else(|| document["facts"].as_array())
        .ok_or_else(|| artifact::invalid("facts must be list"))?;
    let mut seen = BTreeSet::new();
    let mut groups = BTreeMap::<String, Vec<&Value>>::new();
    for row in rows {
        let id = row["id"]
            .as_str()
            .filter(|s| !s.is_empty() && seen.insert(s.to_string()))
            .ok_or_else(|| artifact::invalid("invalid or duplicate fact ID"))?;
        let key = row["fact"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| artifact::invalid("missing fact key"))?;
        let _ = id;
        timestamp(
            row.get("updated_at")
                .or_else(|| row.get("at"))
                .unwrap_or(&Value::Null),
        )?;
        groups.entry(key.trim().into()).or_default().push(row);
    }
    let mut out = vec![];
    for (key, rows) in groups {
        let mut best = rows[0];
        for row in &rows[1..] {
            if rank_cmp(row, best)?.is_gt() {
                best = row;
            }
        }
        let winners = rows
            .iter()
            .filter_map(|row| rank_cmp(row, best).ok().filter(|o| o.is_eq()).map(|_| *row))
            .collect::<Vec<_>>();
        let mut superseded = rows
            .iter()
            .filter(|row| !winners.contains(row))
            .filter_map(|row| row["id"].as_str())
            .collect::<Vec<_>>();
        superseded.sort();
        let mut ids = rows
            .iter()
            .filter_map(|row| row["id"].as_str())
            .collect::<Vec<_>>();
        ids.sort();
        out.push(json!({"fact":key,"winner":if winners.len()==1{winners[0]["id"].clone()}else{Value::Null},"ambiguous":winners.len()>1,"superseded":superseded,"candidates":ids}));
    }
    Ok(json!({"groups":out}))
}
pub fn pointer<'a>(document: &'a Value, pointer: &str) -> Result<&'a Value> {
    if pointer.is_empty() {
        return Ok(document);
    }
    if !pointer.starts_with('/') {
        return Err(artifact::invalid("invalid JSON Pointer"));
    }
    let mut value = document;
    for token in pointer[1..].split('/') {
        let raw = token.as_bytes();
        for (i, b) in raw.iter().enumerate() {
            if *b == b'~' && !raw.get(i + 1).is_some_and(|b| *b == b'0' || *b == b'1') {
                return Err(artifact::invalid("invalid JSON Pointer escape"));
            }
        }
        let token = token.replace("~1", "/").replace("~0", "~");
        value = if let Some(a) = value.as_array() {
            if token.is_empty()
                || token != "0" && token.starts_with('0')
                || !token.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(artifact::invalid("invalid array index"));
            }
            a.get(
                token
                    .parse::<usize>()
                    .map_err(|_| artifact::invalid("array index overflow"))?,
            )
            .ok_or_else(|| artifact::invalid("array index unavailable"))?
        } else {
            value
                .as_object()
                .and_then(|m| m.get(&token))
                .ok_or_else(|| artifact::invalid("JSON Pointer unavailable"))?
        };
    }
    Ok(value)
}
pub fn safe_name(name: &str, memory: bool) -> Result<PathBuf> {
    let p = artifact::relative(name)?;
    if name.contains(['\\', ':', '\0'])
        || name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(artifact::invalid("unsafe pilot path"));
    }
    if !memory && infra_core::domain::source_path_problem(name).is_some() {
        return Err(artifact::invalid("private source refused"));
    }
    if memory {
        let lower = name.to_lowercase();
        if lower.ends_with(".db")
            || lower.ends_with(".sqlite")
            || lower.split('/').any(|p| {
                p.starts_with(".env")
                    || [".ssh", ".aws", ".kube", "local", "secrets", "credentials"].contains(&p)
            })
        {
            return Err(artifact::invalid("private memory path"));
        }
    }
    Ok(p)
}
pub fn safe_read(root: &Path, name: &str, memory: bool, limit: usize) -> Result<Vec<u8>> {
    artifact::read_file(&root.join(safe_name(name, memory)?), limit).map(|r| r.0)
}
pub struct Reader {
    pub root: PathBuf,
    images: BTreeMap<(String, bool), Vec<u8>>,
    total: usize,
    origins: BTreeMap<String, Value>,
}
impl Reader {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            images: BTreeMap::new(),
            total: 0,
            origins: BTreeMap::new(),
        }
    }
    pub fn read(&mut self, name: &str, memory: bool) -> Result<Vec<u8>> {
        let key = (name.into(), memory);
        if let Some(raw) = self.images.get(&key) {
            return Ok(raw.clone());
        }
        if self.images.len() >= 1000 {
            return Err(artifact::invalid("adapter read limit"));
        }
        let raw = safe_read(
            &self.root,
            name,
            memory,
            artifact::MAX_FILE.min(artifact::MAX_TREE.saturating_sub(self.total)),
        )?;
        self.total += raw.len();
        self.images.insert(key, raw.clone());
        Ok(raw)
    }
    pub fn document(&mut self, name: &str) -> Result<(Value, String)> {
        let raw = self.read(name, true)?;
        let text =
            std::str::from_utf8(&raw).map_err(|_| artifact::invalid("frontmatter is not UTF-8"))?;
        public(text)?;
        let (meta, body) = infra_core::bank::codec::parse_raw(&raw)?;
        if infra_core::bank::promotion::instruction_like(
            meta["title"].as_str().unwrap_or(""),
            &body,
        )? {
            return Err(artifact::invalid("instruction-like source"));
        }
        Ok((meta, body))
    }
    pub fn pins(
        &mut self,
        sources: &Value,
        pins: &Value,
        origins: bool,
    ) -> Result<(Vec<Value>, bool)> {
        let names = sources
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 100)
            .ok_or_else(|| artifact::invalid("complete sources required"))?;
        let expected = infra_core::results::canonical_image(
            pins.as_array()
                .ok_or_else(|| artifact::invalid("complete source pins required"))?,
        )?;
        let paths = names
            .iter()
            .map(|s| {
                s.as_str()
                    .map(|s| s.split('#').next().unwrap_or(s).to_string())
                    .ok_or_else(|| artifact::invalid("invalid source"))
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let actual = expected
            .iter()
            .filter_map(|r| r["path"].as_str().map(String::from))
            .collect::<BTreeSet<_>>();
        if expected.is_empty() || paths != actual {
            return Err(artifact::invalid("complete original source pins required"));
        }
        let mut stale = false;
        for row in &expected {
            let name = row["path"].as_str().unwrap();
            let raw = self.read(name, origins && brain_path(name))?;
            let hash = neutral_hash(&raw);
            stale |= row["sha256"] != hash;
        }
        Ok((expected, stale))
    }
    pub fn origin(&mut self, name: &str) -> Result<Value> {
        if let Some(v) = self.origins.get(name) {
            return Ok(v.clone());
        }
        if !brain_path(name) {
            return Err(artifact::invalid("unsupported Brain source path"));
        }
        let (meta, body) = self.document(name)?;
        infra_core::domain::validate_record(&meta, false)?;
        let expected = format!(
            "project-brain/dynamic/{}/{}.md",
            match meta["type"].as_str() {
                Some("finding") => "findings",
                Some("bug") => "bugs",
                Some("incident") => "incidents",
                Some("decision") => "decisions",
                _ => return Err(artifact::invalid("origin type")),
            },
            meta["id"].as_str().unwrap_or("")
        );
        let config = admission(&self.root)?;
        let admitted = |key: &str, v: &Value| config[key].as_array().is_some_and(|a| a.contains(v));
        let terminal = match meta["type"].as_str() {
            Some("decision") => meta["status"] == "accepted",
            _ => meta["status"] == "resolved",
        };
        if expected != name
            || !["public", "team"].contains(&meta["privacy"].as_str().unwrap_or(""))
            || !admitted("allowed_privacy", &meta["privacy"])
            || meta["authority"] != "verified"
            || !admitted("allowed_authority", &meta["authority"])
            || !terminal
            || !meta["superseded_by"].is_null()
        {
            return Err(artifact::invalid("origin not admitted"));
        }
        let attestation = infra_core::domain::record_attestation(&meta);
        if !["agent", "person"].contains(&attestation) {
            return Err(artifact::invalid("unknown attestation"));
        }
        let created = instant(meta["created_at"].as_str().unwrap_or(""))?;
        let updated = instant(meta["updated_at"].as_str().unwrap_or(""))?;
        if created > updated
            || meta["transitions"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|r| {
                    !instant(r["at"].as_str().unwrap_or(""))
                        .is_ok_and(|at| at >= created && at <= updated)
                })
        {
            return Err(artifact::invalid("inconsistent recorded time"));
        }
        let (pins, stale) = self.pins(&meta["sources"], &meta["source_fingerprints"], false)?;
        let provenance = json!({"record_id":meta["id"],"authority":meta["authority"],"attestation":attestation,"privacy":meta["privacy"],"recorded_at":iso(updated),"recorded_at_precision":"instant","valid_from":iso(updated),"valid_to":null,"lineage":format!("brain:{}",meta["id"].as_str().unwrap_or("")),"revision":meta["revision"],"conflicts":meta["conflicts"],"supersedes":meta["supersedes"],"superseded_by":meta["superseded_by"],"source_pins":pins,"reason":if stale{"stale"}else{"eligible"},"_metadata":meta,"_body":body});
        self.origins.insert(name.into(), provenance.clone());
        Ok(provenance)
    }
    pub fn frontmatter(&mut self, selector: &Value) -> Result<Value> {
        let name = selector["source"].as_str().unwrap();
        let kind = selector["source_kind"].as_str().unwrap();
        let (mut provenance, meta, body) = if kind == "brain-frontmatter" {
            let mut p = self.origin(name)?;
            let meta = p.as_object_mut().unwrap().remove("_metadata").unwrap();
            let body = p
                .as_object_mut()
                .unwrap()
                .remove("_body")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string();
            (p, meta, body)
        } else {
            if !name.starts_with("memory-bank/chunks/")
                || name.split('/').count() != 3
                || infra_core::bank::codec::filename_id(
                    Path::new(name)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(""),
                )?
                .is_none()
            {
                return Err(artifact::invalid("unsupported bank source"));
            }
            let (meta, body) = self.document(name)?;
            let context = infra_core::RuntimeContext::new(
                infra_core::Roots::installed(&self.root)?,
                "local",
                "governed",
            )?;
            infra_core::bank::validation::metadata(
                &infra_core::BankService::new(context),
                name,
                &meta,
                false,
            )?;
            if meta["status"] != "active" || !meta["superseded_by"].is_null() {
                return Err(artifact::invalid("inactive bank source"));
            }
            let (pins, mut stale) = self.pins(&meta["sources"], &meta["source_digests"], true)?;
            let mut origins = vec![];
            for pin in &pins {
                if brain_path(pin["path"].as_str().unwrap_or("")) {
                    origins.push(self.origin(pin["path"].as_str().unwrap())?);
                }
            }
            if origins.is_empty() {
                return Err(artifact::invalid("verified bank origins required"));
            }
            for origin in &origins {
                for pin in origin["source_pins"].as_array().into_iter().flatten() {
                    if !pins.iter().any(|p| p == pin) {
                        return Err(artifact::invalid("incomplete bank original image"));
                    }
                }
                stale |= origin["reason"] == "stale";
            }
            let mut recorded = day(meta["created"].as_str().unwrap_or(""), true)?
                .max(day(meta["last_verified"].as_str().unwrap_or(""), true)?);
            for origin in &origins {
                recorded = recorded.max(instant(origin["recorded_at"].as_str().unwrap_or(""))?);
            }
            let mut end = day(meta["review_after"].as_str().unwrap_or(""), true)?;
            if let Some(s) = meta["valid_to"].as_str() {
                end = end.min(day(s, true)?);
            }
            let conflicts = origins
                .iter()
                .flat_map(|o| o["conflicts"].as_array().into_iter().flatten().cloned())
                .collect::<Vec<_>>();
            let mut scope = meta["scope"].as_array().cloned().unwrap_or_default();
            scope.sort_by_key(Value::to_string);
            let provenance = json!({"record_id":meta["id"],"authority":"verified","attestation":if origins.iter().any(|o|o["attestation"]=="agent"){"agent"}else{"person"},"privacy":if origins.iter().any(|o|o["privacy"]=="team"){"team"}else{"public"},"recorded_at":iso(recorded),"recorded_at_precision":"day","valid_from":iso(day(meta["valid_from"].as_str().or_else(||meta["created"].as_str()).unwrap_or(""),false)?),"valid_to":iso(end),"lineage":null,"revision":null,"canonical_scope":scope,"conflicts":conflicts.into_iter().filter_map(|v|v.as_str().map(String::from)).collect::<BTreeSet<_>>(),"supersedes":meta["supersedes"],"superseded_by":meta["superseded_by"],"source_pins":pins,"origin_records":origins.iter().map(|o|{let mut p=json!({});for k in ["record_id","revision","authority","attestation","privacy","recorded_at","conflicts","supersedes","superseded_by"]{p[k]=o[k].clone();}p}).collect::<Vec<_>>(),"reason":if stale{"stale"}else{"eligible"}});
            (provenance, meta, body)
        };
        let mut document = meta;
        document["body"] = body.into();
        let claim = pointer(
            &document,
            selector["pointer"]
                .as_str()
                .ok_or_else(|| artifact::invalid("pointer required"))?,
        )?;
        scalar(claim)?;
        provenance["claim"] = claim.clone();
        provenance["source"] = name.into();
        provenance["source_kind"] = kind.into();
        provenance["source_digest"] = artifact::sha(&self.read(name, true)?).into();
        Ok(provenance)
    }
}
fn brain_path(name: &str) -> bool {
    regex::Regex::new(
        r"^project-brain/dynamic/(findings|bugs|incidents|decisions)/[0-9a-f-]{36}\.md$",
    )
    .is_ok_and(|r| r.is_match(name))
}
pub fn neutral_hash(raw: &[u8]) -> String {
    let mut neutral = vec![];
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\r' {
            neutral.push(b'\n');
            i += 1;
            if raw.get(i) == Some(&b'\n') {
                i += 1;
            }
        } else {
            neutral.push(raw[i]);
            i += 1;
        }
    }
    artifact::sha(&neutral)
}
pub fn admission(root: &Path) -> Result<Value> {
    let mut config =
        json!({"allowed_privacy":["public","team"],"allowed_authority":["observed","verified"]});
    let path = root.join("project-brain/config/runtime.json");
    if path.exists() {
        let supplied = artifact::read_json(&path, artifact::MAX_FILE)?;
        for (k, v) in supplied
            .as_object()
            .ok_or_else(|| artifact::invalid("invalid admission config"))?
        {
            config[k] = v.clone();
        }
    }
    for key in ["allowed_privacy", "allowed_authority"] {
        if !config[key]
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string))
        {
            return Err(artifact::invalid("invalid admission config"));
        }
    }
    Ok(config)
}
fn scalar(v: &Value) -> Result<()> {
    if v.is_object() || v.is_array() || v.as_str().is_some_and(|s| s.chars().count() > 2000) {
        return Err(artifact::invalid("bounded scalar claim required"));
    }
    public(&serde_json::to_string(v)?)
}
pub fn collect(root: &Path, selectors: &Value, as_of: &str) -> Result<Vec<Value>> {
    let moment = instant(as_of)?;
    let selectors = selectors
        .as_array()
        .filter(|a| a.len() <= 1000)
        .ok_or_else(|| artifact::invalid("invalid selector set"))?;
    let mut seen = BTreeSet::new();
    let mut reader = Reader::new(root);
    let mut out = vec![];
    for selector in selectors {
        let kind = selector["source_kind"].as_str().unwrap_or("json");
        if !["json", "brain-frontmatter", "bank-frontmatter"].contains(&kind) {
            return Err(artifact::invalid("unsupported source kind"));
        }
        let mut required = BTreeSet::from([
            "id",
            "namespace",
            "subject",
            "predicate",
            "source",
            "pointer",
        ]);
        let mut optional = BTreeSet::from(["qualifiers", "expected_sha256", "source_kind"]);
        if kind == "json" {
            required.insert("recorded_at_pointer");
            optional.extend([
                "lineage_pointer",
                "revision_pointer",
                "conflicts_pointer",
                "valid_from_pointer",
                "valid_to_pointer",
            ]);
        }
        let m = selector
            .as_object()
            .ok_or_else(|| artifact::invalid("invalid selector"))?;
        if required.iter().any(|k| !m.contains_key(*k))
            || m.keys()
                .any(|k| !required.contains(k.as_str()) && !optional.contains(k.as_str()))
            || required
                .iter()
                .any(|k| *k != "pointer" && !selector[*k].as_str().is_some_and(|s| !s.is_empty()))
            || !seen.insert(selector["id"].as_str().unwrap().to_string())
        {
            return Err(artifact::invalid("invalid or duplicate selector"));
        }
        public(&serde_json::to_string(selector)?)?;
        let qualifiers = selector
            .get("qualifiers")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !qualifiers
            .as_object()
            .is_some_and(|m| m.values().all(Value::is_string))
        {
            return Err(artifact::invalid("qualifiers must be string pairs"));
        }
        let mut row = json!({"id":selector["id"],"namespace":selector["namespace"],"subject":selector["subject"],"predicate":selector["predicate"],"qualifiers":qualifiers,"reason":"unclassified","claim":null});
        let result = (|| -> Result<Value> {
            let mut fields = if kind != "json" {
                reader.frontmatter(selector)?
            } else {
                let name = selector["source"].as_str().unwrap();
                if !name.ends_with(".json") {
                    return Err(artifact::invalid("JSON source required"));
                }
                let raw = reader.read(name, false)?;
                let doc = artifact::strict(&raw)?;
                let claim = pointer(
                    &doc,
                    selector["pointer"]
                        .as_str()
                        .ok_or_else(|| artifact::invalid("pointer required"))?,
                )?;
                scalar(claim)?;
                let recorded = instant(
                    pointer(&doc, selector["recorded_at_pointer"].as_str().unwrap())?
                        .as_str()
                        .ok_or_else(|| artifact::invalid("recorded timestamp"))?,
                )?;
                let mut value = json!({"source":name,"source_digest":artifact::sha(&raw),"recorded_at":iso(recorded),"claim":claim,"authority":"canonical-json","lineage":null,"revision":null,"conflicts":[],"reason":"eligible"});
                for key in ["lineage", "revision", "conflicts"] {
                    if let Some(p) = selector[format!("{key}_pointer")].as_str() {
                        value[key] = pointer(&doc, p)?.clone();
                    }
                }
                if !value["lineage"].is_null() && !value["lineage"].is_string()
                    || !value["revision"].is_null() && value["revision"].as_u64().is_none()
                    || !value["conflicts"]
                        .as_array()
                        .is_some_and(|a| a.iter().all(Value::is_string))
                {
                    return Err(artifact::invalid("invalid lineage evidence"));
                }
                public(&serde_json::to_string(&json!([
                    value["lineage"],
                    value["conflicts"]
                ]))?)?;
                let begin = if let Some(p) = selector["valid_from_pointer"].as_str() {
                    instant(
                        pointer(&doc, p)?
                            .as_str()
                            .ok_or_else(|| artifact::invalid("validity start"))?,
                    )?
                } else {
                    recorded
                };
                let end = if let Some(p) = selector["valid_to_pointer"].as_str() {
                    pointer(&doc, p)?.as_str().map(instant).transpose()?
                } else {
                    None
                };
                if end.is_some_and(|e| e <= begin) {
                    return Err(artifact::invalid("invalid validity range"));
                }
                value["valid_from"] = iso(begin).into();
                value["valid_to"] = json!(end.map(iso));
                value
            };
            if selector.get("expected_sha256").is_some()
                && selector["expected_sha256"] != fields["source_digest"]
            {
                fields["reason"] = "stale".into();
            } else if fields["reason"] == "eligible" {
                let recorded = instant(fields["recorded_at"].as_str().unwrap())?;
                let begin = instant(fields["valid_from"].as_str().unwrap())?;
                let end = fields["valid_to"].as_str().map(instant).transpose()?;
                if recorded > moment || begin > moment {
                    fields["reason"] = "future".into();
                } else if end.is_some_and(|e| moment >= e) {
                    fields["reason"] = "expired".into();
                }
            }
            Ok(fields)
        })();
        match result {
            Ok(fields) => {
                for (k, v) in fields.as_object().unwrap() {
                    row[k] = v.clone();
                }
            }
            Err(_) => {
                if kind != "json" {
                    row["source_kind"] = kind.into();
                }
            }
        }
        out.push(row);
    }
    out.sort_by_key(|r| r["id"].as_str().unwrap_or("").to_string());
    Ok(out)
}
pub fn resolve_current(candidates: &[Value], namespace: &str, as_of: &str) -> Result<Value> {
    let moment = instant(as_of)?;
    let mut groups = BTreeMap::<String, Vec<Value>>::new();
    for row in candidates.iter().filter(|r| r["namespace"] == namespace) {
        let mut row = row.clone();
        if ["eligible", "future", "expired"].contains(&row["reason"].as_str().unwrap_or("")) {
            let begin = instant(row["valid_from"].as_str().unwrap_or(""))?;
            let recorded = instant(row["recorded_at"].as_str().unwrap_or(""))?;
            let end = row["valid_to"].as_str().map(instant).transpose()?;
            row["reason"] = if recorded > moment || begin > moment {
                "future"
            } else if end.is_some_and(|e| moment >= e) {
                "expired"
            } else {
                "eligible"
            }
            .into();
        }
        let key = infra_core::domain::json::canonical(&json!([
            row["subject"],
            row["predicate"],
            row["qualifiers"],
            row.get("canonical_scope")
                .cloned()
                .unwrap_or_else(|| json!([]))
        ]))?;
        groups.entry(key).or_default().push(row);
    }
    let mut facts = vec![];
    for (_, mut rows) in groups {
        rows.sort_by_key(|r| r["id"].as_str().unwrap_or("").to_string());
        let mut eligible = rows
            .iter()
            .filter(|r| r["reason"] == "eligible")
            .collect::<Vec<_>>();
        let conflict = eligible
            .iter()
            .any(|r| r["conflicts"].as_array().is_some_and(|a| !a.is_empty()));
        let canonical = eligible
            .iter()
            .copied()
            .filter(|r| r["authority"] == "canonical-json")
            .collect::<Vec<_>>();
        if !canonical.is_empty() {
            eligible = canonical;
        }
        if !eligible.is_empty()
            && eligible[0]["lineage"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
            && eligible
                .iter()
                .all(|r| r["lineage"] == eligible[0]["lineage"])
            && eligible.iter().all(|r| r["revision"].as_u64().is_some())
            && !conflict
        {
            let max = eligible
                .iter()
                .filter_map(|r| r["revision"].as_u64())
                .max()
                .unwrap();
            eligible.retain(|r| r["revision"] == max);
        }
        let claims = eligible
            .iter()
            .map(|r| artifact::digest(&r["claim"]))
            .collect::<Result<BTreeSet<_>>>()?;
        let state = if conflict || claims.len() > 1 {
            "ambiguous"
        } else if rows.iter().any(|r| r["reason"] == "stale") {
            "stale"
        } else if rows.iter().any(|r| r["reason"] == "unclassified") {
            "unclassified"
        } else if !eligible.is_empty() {
            "current"
        } else {
            "unclassified"
        };
        let mut fact = json!({"namespace":namespace,"subject":rows[0]["subject"],"predicate":rows[0]["predicate"],"qualifiers":rows[0]["qualifiers"],"state":state,"claim":if state=="current"{eligible[0]["claim"].clone()}else{Value::Null},"current_ids":if state=="current"{eligible.iter().map(|r|r["id"].clone()).collect::<Vec<_>>()}else{vec![]},"history":rows});
        if let Some(scope) = fact["history"][0]["canonical_scope"]
            .as_array()
            .filter(|a| !a.is_empty())
        {
            fact["canonical_scope"] = json!(scope);
        }
        facts.push(fact);
    }
    let mut result = json!({"schema":"memory-current-facts-v1","as_of":iso(moment),"facts":facts,"derived":true,"runtime_integrated":false});
    if candidates.iter().any(|r| {
        r["namespace"] == namespace
            && ["brain-frontmatter", "bank-frontmatter"]
                .contains(&r["source_kind"].as_str().unwrap_or(""))
    }) {
        result["adapter_limits"] = json!({"max_file_bytes":artifact::MAX_FILE,"max_total_read_bytes":artifact::MAX_TREE,"max_unique_reads":1000,"max_sources_per_record":100,"max_claim_characters":2000});
    }
    Ok(result)
}
pub fn project(root: &Path, selectors: &Value, as_of: &str) -> Result<Value> {
    let candidates = collect(root, selectors, as_of)?;
    let namespaces = candidates
        .iter()
        .filter_map(|r| r["namespace"].as_str())
        .collect::<BTreeSet<_>>();
    Ok(
        json!({"schema":"memory-current-facts-set-v1","projections":namespaces.into_iter().map(|name|resolve_current(&candidates,name,as_of)).collect::<Result<Vec<_>>>()?}),
    )
}
