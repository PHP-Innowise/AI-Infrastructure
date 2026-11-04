//! Private maintenance packets: current bytes are evidence, semantic decisions stay agent supplied.
use crate::executable;
use crate::{Result, artifact, facts};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
pub const PROFILE: &str = "memory-maintenance-evidence-v1";
fn context(root: &Path, owner: &str) -> Result<infra_core::RuntimeContext> {
    infra_core::RuntimeContext::new(infra_core::Roots::installed(root)?, owner, "governed")
}
fn seal(mut v: Value) -> Result<Value> {
    v["sha256"] = artifact::digest(&v)?.into();
    Ok(v)
}
fn check(v: &Value) -> Result<()> {
    let mut body = v.clone();
    let sha = body
        .as_object_mut()
        .ok_or_else(|| artifact::invalid("sealed object required"))?
        .remove("sha256")
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| artifact::invalid("evidence seal unavailable"))?;
    if artifact::digest(&body)? != sha {
        return Err(artifact::invalid("evidence image changed"));
    }
    Ok(())
}
pub fn snapshot(root: &Path) -> Result<Vec<artifact::ImageFile>> {
    artifact::image(root, artifact::ImageKind::Public, None)
}
pub fn snapshot_digest(root: &Path) -> Result<String> {
    artifact::image_digest(&snapshot(root)?, false)
}
pub fn review_packet(audit: &Value, digest: &str, limit: usize) -> Result<Value> {
    if !(1..=10000).contains(&limit) || !audit.is_object() {
        return Err(artifact::invalid("invalid review limit or audit"));
    }
    let mut entries = BTreeMap::<String, BTreeSet<String>>::new();
    let mut sources = BTreeMap::<String, Vec<Value>>::new();
    let mut dates = BTreeMap::<String, BTreeSet<String>>::new();
    let mut ids = BTreeMap::<String, String>::new();
    let valid_chunk = |p: &str| -> Result<()> {
        facts::safe_name(p, true)?;
        facts::public(p)?;
        if !p.starts_with("memory-bank/chunks/") || !p.ends_with(".md") {
            return Err(artifact::invalid("invalid audit chunk"));
        }
        Ok(())
    };
    for row in audit["source_changed"].as_array().into_iter().flatten() {
        let p = row["chunk"]
            .as_str()
            .ok_or_else(|| artifact::invalid("audit chunk required"))?;
        valid_chunk(p)?;
        let state = row["state"]
            .as_str()
            .filter(|s| ["changed", "missing"].contains(s))
            .ok_or_else(|| artifact::invalid("invalid source state"))?;
        entries.entry(p.into()).or_default().insert(state.into());
        let mut detail = json!({"state":state,"source_withheld":true});
        if let Some(source) = row["source"].as_str()
            && facts::safe_name(source, false).is_ok()
            && facts::public(source).is_ok()
        {
            detail["source"] = source.into();
            detail["source_withheld"] = false.into();
        }
        for key in ["expected_sha256", "current_sha256"] {
            if row[key].as_str().is_some_and(|s| artifact::hex(s, 64)) {
                detail[key] = row[key].clone();
            }
        }
        sources.entry(p.into()).or_default().push(detail);
        if let Some(id) = row["id"].as_str().filter(|s| {
            s.starts_with("MEM-") && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        }) {
            ids.insert(p.into(), id.into());
        }
    }
    for row in audit["overdue_review"].as_array().into_iter().flatten() {
        let p = row["chunk"]
            .as_str()
            .ok_or_else(|| artifact::invalid("audit chunk required"))?;
        valid_chunk(p)?;
        entries
            .entry(p.into())
            .or_default()
            .insert("overdue".into());
        if let Some(date) = row["review_after"]
            .as_str()
            .filter(|s| s.len() == 10 && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok())
        {
            dates.entry(p.into()).or_default().insert(date.into());
        }
    }
    for (key, reason) in [
        ("undigested", "undigested"),
        ("instruction_like", "instruction-like"),
    ] {
        let rows = if key == "instruction_like" {
            &audit["provenance"][key]
        } else {
            &audit[key]
        };
        for p in rows
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            valid_chunk(p)?;
            entries.entry(p.into()).or_default().insert(reason.into());
        }
    }
    for row in audit["duplicates"].as_array().into_iter().flatten() {
        let mut pair = vec![
            row["left"].as_str().unwrap_or("").to_string(),
            row["right"].as_str().unwrap_or("").to_string(),
        ];
        pair.sort();
        if pair.iter().any(|s| {
            !s.starts_with("MEM-")
                || s.len() < 5
                || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        }) {
            return Err(artifact::invalid("invalid duplicate IDs"));
        }
        facts::public(&serde_json::to_string(&pair)?)?;
        entries
            .entry(format!("pair:{}", pair.join("|")))
            .or_default()
            .insert("duplicate".into());
    }
    let priority = |r: &str| {
        [
            "instruction-like",
            "missing",
            "changed",
            "overdue",
            "undigested",
            "duplicate",
        ]
        .iter()
        .position(|s| *s == r)
        .unwrap_or(6)
    };
    let mut entries = entries.into_iter().collect::<Vec<_>>();
    entries.sort_by_key(|(p, reasons)| {
        (
            reasons.iter().map(|r| priority(r)).min().unwrap_or(6),
            p.clone(),
        )
    });
    let total = entries.len();
    let mut queue = vec![];
    for (p, reasons) in entries.into_iter().take(limit) {
        let mut reasons = reasons.into_iter().collect::<Vec<_>>();
        reasons.sort_by_key(|r| priority(r));
        let mut sources = sources.remove(&p).unwrap_or_default();
        sources.sort_by_key(|v| {
            (
                v["source"].as_str().unwrap_or("").to_string(),
                v["state"].as_str().unwrap_or("").to_string(),
            )
        });
        queue.push(json!({"chunk":if p.starts_with("pair:"){Value::Null}else{json!(p)},"id":ids.get(&p),"duplicate_ids":p.strip_prefix("pair:").map(|s|s.split('|').collect::<Vec<_>>()).unwrap_or_default(),"sources":sources,"review_dates":dates.get(&p).cloned().unwrap_or_default(),"reasons":reasons,"review_required":true,"required_evidence":"Read the claim and current canonical sources; verify applicability before mutation.","allowed_actions":if reasons.iter().any(|r|r=="instruction-like"){vec!["investigate","quarantine"]}else{vec!["investigate","reverify-after-check","replace","retire"]}}));
    }
    Ok(
        json!({"schema":"memory-freshness-plan-v1","snapshot_digest":digest,"queue":queue,"limit":limit,"total_candidates":total,"returned":total.min(limit),"truncated":total>limit,"remaining":total.saturating_sub(limit),"read_only":true,"attestation":"not-performed","audit_limit_applies":false,"audit_chunks":audit["chunks"]}),
    )
}
pub fn audit(root: &Path) -> Result<Value> {
    let c = context(root, "local")?;
    let mut audit = infra_core::BankService::new(c.clone()).audit(true)?;
    for row in audit["source_changed"].as_array_mut().into_iter().flatten() {
        let run = (|| -> Result<()> {
            let p = row["chunk"]
                .as_str()
                .ok_or_else(|| artifact::invalid("chunk required"))?;
            let (raw, _) =
                artifact::read_file(&root.join(facts::safe_name(p, true)?), artifact::MAX_FILE)?;
            let (meta, _) = infra_core::bank::codec::parse_raw(&raw)?;
            let source = row["source"]
                .as_str()
                .ok_or_else(|| artifact::invalid("source required"))?
                .to_string();
            facts::safe_name(&source, false)?;
            row["expected_sha256"] = meta["source_digests"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|pin| pin["path"] == source.as_str())
                .map(|p| p["sha256"].clone())
                .unwrap_or(Value::Null);
            row["current_sha256"] = json!(
                infra_core::domain::source_fingerprints(&c.roots, &[source])?
                    .first()
                    .map(|r| r["sha256"].clone())
            );
            Ok(())
        })();
        let _ = run;
    }
    Ok(audit)
}
fn selection(chunks: Option<&[String]>, limit: usize) -> Result<()> {
    if !(1..=50).contains(&limit) {
        return Err(artifact::invalid("invalid evidence limit"));
    }
    if let Some(chunks) = chunks {
        if chunks.is_empty()
            || chunks.len() > limit
            || chunks.iter().collect::<BTreeSet<_>>().len() != chunks.len()
        {
            return Err(artifact::invalid("invalid explicit selection"));
        }
        for p in chunks {
            facts::safe_name(p, true)?;
            facts::public(p)?;
            if !p.starts_with("memory-bank/chunks/")
                || p.split('/').count() != 3
                || !p.ends_with(".md")
            {
                return Err(artifact::invalid("invalid selected chunk"));
            }
        }
    }
    Ok(())
}
pub fn applicable(meta: &Value) -> bool {
    let today = chrono::Local::now().date_naive();
    for key in ["valid_from", "valid_to", "review_after"] {
        if meta.get(key).is_none() {
            if key == "review_after" {
                return false;
            }
            continue;
        }
        if key == "valid_to" && meta[key].is_null() {
            continue;
        }
        let Some(raw) = meta[key].as_str() else {
            return false;
        };
        if raw.len() != 10 {
            return false;
        }
        let Ok(date) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") else {
            return false;
        };
        if key == "valid_from" && date > today || key != "valid_from" && date < today {
            return false;
        }
    }
    true
}
fn frontier(
    root: &Path,
    row: &Value,
    chunk: bool,
    results: &infra_core::ResultService,
) -> Result<bool> {
    Ok((!chunk
        || artifact::sha(&facts::safe_read(
            root,
            row["chunk"].as_str().unwrap_or(""),
            true,
            65536,
        )?) == row["chunk_sha256"])
        && artifact::sha(&facts::safe_read(
            root,
            row["record_path"].as_str().unwrap_or(""),
            true,
            65536,
        )?) == row["record_sha256"]
        && json!(results.checked_fingerprints(&strings(&row["record_sources"])?)?)
            == row["source_fingerprints"])
}
fn strings(v: &Value) -> Result<Vec<String>> {
    v.as_array()
        .ok_or_else(|| artifact::invalid("string list required"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(String::from)
                .ok_or_else(|| artifact::invalid("string list required"))
        })
        .collect()
}
pub fn evidence(
    root: &Path,
    chunks: Option<&[String]>,
    limit: usize,
    owner: &str,
) -> Result<Value> {
    selection(chunks, limit)?;
    facts::public(owner)?;
    let c = context(root, owner)?;
    let results = infra_core::ResultService::new(c.clone());
    let config = infra_core::RuntimeConfig::load(&c.roots)?;
    let digest = snapshot_digest(root)?;
    let (plan, total, truncated) = if let Some(chunks) = chunks {
        (
            chunks
                .iter()
                .map(|p| json!({"chunk":p,"reasons":["explicit-selection"]}))
                .collect::<Vec<_>>(),
            chunks.len(),
            false,
        )
    } else {
        let plan = review_packet(&audit(root)?, &digest, limit)?;
        (
            plan["queue"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| json!({"chunk":r["chunk"],"reasons":r["reasons"]}))
                .collect(),
            plan["total_candidates"].as_u64().unwrap_or(0) as usize,
            plan["truncated"] == true,
        )
    };
    let mut items = vec![];
    let mut total_source_bytes = 0;
    let origin_expression = regex::Regex::new(
        r"^project-brain/(?:dynamic/(?:findings|decisions)|archive/finding)/[0-9a-f-]{36}\.md$",
    )
    .ok();
    for candidate in plan {
        let mut row = candidate.clone();
        row["state"] = "held".into();
        row["reason"] = "evidence-unavailable-or-unsupported".into();
        row["sources"] = json!([]);
        let ready = (|| -> Result<Value> {
            let name = candidate["chunk"]
                .as_str()
                .ok_or_else(|| artifact::invalid("unresolved duplicate"))?;
            let raw = facts::safe_read(root, name, true, 65536)?;
            facts::public(
                std::str::from_utf8(&raw).map_err(|_| artifact::invalid("chunk encoding"))?,
            )?;
            let (meta, body) = infra_core::bank::codec::parse_raw(&raw)?;
            if meta["status"] != "active"
                || !["public", "team"].contains(&meta["privacy"].as_str().unwrap_or("team"))
                || !config.value["allowed_privacy"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|v| v.as_str() == Some(meta["privacy"].as_str().unwrap_or("team")))
                })
                || !config.value["allowed_authority"]
                    .as_array()
                    .is_some_and(|a| {
                        a.iter().any(|v| {
                            v.as_str() == Some(meta["authority"].as_str().unwrap_or("observed"))
                        })
                    })
                || infra_core::bank::promotion::instruction_like(
                    meta["title"].as_str().unwrap_or(""),
                    &body,
                )?
            {
                return Err(artifact::invalid("chunk not admitted"));
            }
            let origin = |p: &str| origin_expression.as_ref().is_some_and(|r| r.is_match(p));
            let links = meta["source_digests"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| p["path"].as_str().is_some_and(origin))
                .collect::<Vec<_>>();
            if links.len() != 1 {
                return Err(artifact::invalid("unsupported provenance"));
            }
            let record_path = links[0]["path"].as_str().unwrap();
            let record_raw = facts::safe_read(root, record_path, true, 65536)?;
            facts::public(
                std::str::from_utf8(&record_raw)
                    .map_err(|_| artifact::invalid("record encoding"))?,
            )?;
            if facts::neutral_hash(&record_raw) != links[0]["sha256"] {
                return Err(artifact::invalid("record image changed"));
            }
            let (record, _) = infra_core::domain::parse_markdown_record(
                std::str::from_utf8(&record_raw)
                    .map_err(|_| artifact::invalid("record encoding"))?,
            )?;
            let archived = record_path.starts_with("project-brain/archive/");
            infra_core::domain::validate_record(&record, archived)?;
            if archived {
                let records = infra_core::BrainRepository::new(c.clone())?
                    .inventory(true)?
                    .records
                    .into_iter()
                    .filter(|r| r.record["id"] == record["id"])
                    .collect::<Vec<_>>();
                if records.len() != 1 || records[0].path != root.join(record_path) {
                    return Err(artifact::invalid("archive placement unavailable"));
                }
            }
            if !["finding", "decision"].contains(&record["type"].as_str().unwrap_or(""))
                || record["authority"] != "verified"
                || infra_core::domain::record_attestation(&record) != "agent"
                || !record["superseded_by"].is_null()
                || !record["conflicts"].as_array().is_some_and(Vec::is_empty)
                || !(record["type"] == "finding" && record["status"] == "resolved"
                    || record["type"] == "decision" && record["status"] == "accepted")
                || !(record["owner"] == owner
                    || config
                        .strings("owners")?
                        .iter()
                        .any(|o| o == "*" || record["owner"] == o.as_str()))
                || !config.value["allowed_privacy"]
                    .as_array()
                    .is_some_and(|a| a.contains(&record["privacy"]))
                || !["public", "team"].contains(&record["privacy"].as_str().unwrap_or(""))
                || !config.value["allowed_authority"]
                    .as_array()
                    .is_some_and(|a| a.contains(&record["authority"]))
            {
                return Err(artifact::invalid("record not admitted"));
            }
            let pins = results.checked_fingerprints(&strings(&record["sources"])?)?;
            let expected = pins
                .iter()
                .filter_map(|p| p["path"].as_str().map(String::from))
                .chain([record_path.into()])
                .collect::<BTreeSet<_>>();
            let chunk_pins = infra_core::results::canonical_image(
                meta["source_digests"]
                    .as_array()
                    .ok_or_else(|| artifact::invalid("chunk pins required"))?,
            )?;
            let paths = strings(&meta["sources"])?
                .into_iter()
                .map(|s| s.split('#').next().unwrap_or(&s).to_string())
                .collect::<BTreeSet<_>>();
            if paths != expected
                || chunk_pins
                    .iter()
                    .filter_map(|p| p["path"].as_str().map(String::from))
                    .collect::<BTreeSet<_>>()
                    != expected
            {
                return Err(artifact::invalid("unsupported chunk source coverage"));
            }
            let mut sources = vec![];
            let mut bytes = 0;
            for pin in &pins {
                let path = pin["path"].as_str().unwrap();
                let raw = facts::safe_read(root, path, false, 262144)?;
                let text =
                    std::str::from_utf8(&raw).map_err(|_| artifact::invalid("source encoding"))?;
                facts::public(text)?;
                if text.contains('\0') || facts::neutral_hash(&raw) != pin["sha256"] {
                    return Err(artifact::invalid("source image changed"));
                }
                bytes += raw.len();
                if total_source_bytes + bytes > 524288 {
                    return Err(artifact::invalid("source budget"));
                }
                sources.push(json!({"path":pin["path"],"sha256":pin["sha256"],"raw_sha256":artifact::sha(&raw),"text":text}));
            }
            let consequence = record["progress"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| record["goal"].as_str())
                .unwrap_or("");
            if consequence.is_empty()
                || consequence.chars().count() > 1000
                || consequence != crate::scoring::flatten(consequence)
            {
                return Err(artifact::invalid("unsupported claim rendering"));
            }
            let chunk_fresh = chunk_pins.iter().all(|pin| {
                pin["path"]
                    .as_str()
                    .and_then(|p| facts::safe_read(root, p, true, artifact::MAX_FILE).ok())
                    .is_some_and(|raw| facts::neutral_hash(&raw) == pin["sha256"])
            });
            total_source_bytes += bytes;
            let mut row = candidate.clone();
            for(k,v)in json!({"state":"ready","reason":null,"memory_id":meta["id"],"chunk_sha256":artifact::sha(&raw),"body":body,"title":meta["title"],"record_path":record_path,"record_sha256":artifact::sha(&record_raw),"record_id":record["id"],"expected_revision":record["revision"],"type":record["type"],"record_title":record["title"],"consequence":consequence,"record_sources":record["sources"],"sources":sources,"source_fingerprints":pins,"previous_source_fingerprints":record["source_fingerprints"],"source_image_matches_record":json!(pins)==json!(infra_core::results::canonical_image(record["source_fingerprints"].as_array().ok_or_else(||artifact::invalid("source pins required"))?)?),"chunk_sources_fresh":chunk_fresh,"currently_applicable":applicable(&meta)}).as_object().unwrap(){row[k]=v.clone();}
            if archived {
                row["origin_placement"] = "archive".into();
                row["maintenance_capabilities"] = json!({"confirm":"unchanged-evidence-only","correct":"unchanged-consequence-and-evidence-only","retire":"selected-fresh-successor","defer":"supported","origin_record_mutable":false});
            }
            Ok(row)
        })();
        if let Ok(ready) = ready {
            row = ready;
        }
        items.push(seal(row)?);
    }
    let packet = seal(
        json!({"schema":PROFILE,"snapshot_digest":digest,"owner":owner,"selection":chunks,"limit":limit,"total_candidates":total,"truncated":truncated,"remaining":total.saturating_sub(items.len()),"items":items,"read_only":true,"attestation":"not-performed","source_evidence":"captured-current-bytes","semantic_verification":"not-performed"}),
    )?;
    if serde_json::to_vec(&packet)?.len() > 1048576 {
        return Err(artifact::invalid("evidence packet byte limit"));
    }
    Ok(packet)
}
fn prepare_copy(root: &Path, target: &Path) -> Result<String> {
    let image = snapshot(root)?;
    let before = artifact::image_digest(&image, false)?;
    artifact::copy_image(&image, target)?;
    Ok(before)
}
pub fn plan(root: &Path, limit: usize) -> Result<Value> {
    let work = artifact::Scratch::new(&std::env::temp_dir(), "memory-freshness")?;
    let copy = work.0.join("snapshot");
    let before = prepare_copy(root, &copy)?;
    let result = review_packet(&audit(&copy)?, &before, limit)?;
    if snapshot_digest(root)? != before {
        return Err(artifact::invalid("input changed during audit"));
    }
    Ok(result)
}
pub fn capture(root: &Path, chunks: Option<&[String]>, limit: usize, owner: &str) -> Result<Value> {
    let work = artifact::Scratch::new(&std::env::temp_dir(), "memory-evidence")?;
    let copy = work.0.join("snapshot");
    let before = prepare_copy(root, &copy)?;
    let packet = evidence(&copy, chunks, limit, owner)?;
    if snapshot_digest(root)? != before {
        return Err(artifact::invalid("input changed during capture"));
    }
    Ok(packet)
}
fn verify_decision(
    root: &Path,
    row: &Value,
    d: &Value,
    results: &infra_core::ResultService,
) -> Result<String> {
    let kind = d["action"].as_str().unwrap_or("");
    let mut keys = vec![
        "chunk",
        "row_sha256",
        "action",
        "attestation",
        "reason",
        "evidence",
    ];
    if kind == "correct" {
        keys.push("consequence");
    }
    if kind == "retire" {
        keys.extend(["successor", "valid_to"]);
    }
    if !["confirm", "correct", "retire"].contains(&kind)
        || !artifact::exact(d, &keys)
        || d["row_sha256"] != row["sha256"]
        || d["attestation"] != "agent"
        || !d["reason"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.chars().count() <= 1000)
    {
        return Err(artifact::invalid("invalid checked decision"));
    }
    facts::public(d["reason"].as_str().unwrap())?;
    let sources = row["sources"]
        .as_array()
        .ok_or_else(|| artifact::invalid("captured sources unavailable"))?;
    let citations = d["evidence"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 20)
        .ok_or_else(|| artifact::invalid("missing checked evidence"))?;
    let mut covered = BTreeSet::new();
    for citation in citations {
        if !artifact::exact(citation, &["path", "sha256", "quote"]) {
            return Err(artifact::invalid("invalid checked evidence"));
        }
        let source = sources
            .iter()
            .find(|s| s["path"] == citation["path"])
            .ok_or_else(|| artifact::invalid("unknown evidence"))?;
        let quote = citation["quote"]
            .as_str()
            .filter(|q| {
                !q.trim().is_empty()
                    && q.chars().count() <= 2000
                    && source["text"].as_str().is_some_and(|s| s.contains(*q))
            })
            .ok_or_else(|| artifact::invalid("unchecked evidence"))?;
        let _ = quote;
        if source["sha256"] != citation["sha256"] {
            return Err(artifact::invalid("unchecked evidence"));
        }
        covered.insert(citation["path"].as_str().unwrap());
    }
    if covered
        != sources
            .iter()
            .filter_map(|s| s["path"].as_str())
            .collect::<BTreeSet<_>>()
        || !frontier(root, row, true, results)?
    {
        return Err(artifact::invalid("selected frontier changed"));
    }
    Ok(kind.into())
}
pub fn replay(
    source: &Path,
    root: &Path,
    packet: &Value,
    decisions: &Value,
    export: &Path,
) -> Result<Value> {
    check(packet)?;
    if packet["schema"] != PROFILE
        || packet["items"]
            .as_array()
            .is_none_or(|a| a.is_empty() || a.len() > 50)
        || decisions.as_array().is_none_or(|a| a.len() > 50)
        || serde_json::to_vec(packet)?.len() > 1048576
        || serde_json::to_vec(decisions)?.len() > 131072
    {
        return Err(artifact::invalid("invalid bounded maintenance input"));
    }
    for row in packet["items"].as_array().unwrap() {
        check(row)?;
    }
    if export.exists() || artifact::overlaps(root, export)? || artifact::overlaps(source, export)? {
        return Err(artifact::invalid(
            "export must be new outside input and tooling",
        ));
    }
    let work = artifact::Scratch::new(&std::env::temp_dir(), "memory-maintenance")?;
    let copy = work.0.join("snapshot");
    let before = prepare_copy(root, &copy)?;
    if packet["snapshot_digest"] != before {
        return Err(artifact::invalid(
            "input image changed since evidence capture",
        ));
    }
    let chunks = packet["selection"].as_array().map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect::<Vec<_>>()
    });
    let current = evidence(
        &copy,
        chunks.as_deref(),
        packet["limit"].as_u64().unwrap_or(0) as usize,
        packet["owner"].as_str().unwrap_or(""),
    )?;
    if &current != packet {
        return Err(artifact::invalid("evidence image changed before replay"));
    }
    let owner = packet["owner"].as_str().unwrap();
    let c = context(&copy, owner)?;
    let results = infra_core::ResultService::new(c.clone());
    let bank = infra_core::BankService::new(c.clone());
    let brain = infra_core::BrainRepository::new(c.clone())?;
    let schema = copy.join("project-brain/schemas");
    if schema.exists() {
        std::fs::remove_dir_all(&schema)?;
    }
    crate::bench_driver::copy_tree(
        &source.join("PHP Core/project-brain/schemas"),
        &schema,
        &BTreeSet::new(),
    )?;
    let configuration = copy.join("project-brain/config/runtime.json");
    let mut flags = if configuration.exists() {
        artifact::read_json(&configuration, artifact::MAX_FILE)?
    } else {
        json!({})
    };
    let defaults = infra_core::RuntimeConfig::load(&c.roots)?.value;
    let mut overrides = json!({});
    for key in [
        "automatic_promotion",
        "automatic_consolidation",
        "automatic_consolidation_verification",
        "automatic_compaction",
    ] {
        overrides[key] = json!({"input":flags.get(key).unwrap_or(&defaults[key]),"copy":false});
        flags[key] = false.into();
    }
    overrides["automatic_promotion"]["targeted_selected_records_only"] = true.into();
    artifact::write_json(&configuration, &flags)?;
    let offered = packet["items"].as_array().unwrap();
    let mut chosen = BTreeMap::<String, Vec<&Value>>::new();
    for decision in decisions.as_array().unwrap() {
        let name = decision["chunk"]
            .as_str()
            .ok_or_else(|| artifact::invalid("invalid maintenance decision"))?;
        if !offered.iter().any(|r| r["chunk"] == name) {
            return Err(artifact::invalid("decision outside selected evidence"));
        }
        chosen.entry(name.into()).or_default().push(decision);
    }
    let mut task = None::<Value>;
    let mut actions = vec![];
    for row in offered {
        let mut action = json!({"chunk":row["chunk"],"state":"held","reason":"decision-or-evidence-unavailable"});
        let mut effect = false;
        let run = (|| -> Result<()> {
            let decisions = chosen.get(row["chunk"].as_str().unwrap_or(""));
            if row["state"] != "ready" || decisions.is_none_or(|d| d.len() != 1) {
                return Ok(());
            }
            let d = decisions.unwrap()[0];
            if d["action"] == "defer"
                && artifact::exact(d, &["chunk", "row_sha256", "action", "reason"])
            {
                if d["row_sha256"] != row["sha256"]
                    || !d["reason"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty() && s.chars().count() <= 1000)
                {
                    return Err(artifact::invalid("invalid defer"));
                }
                facts::public(d["reason"].as_str().unwrap())?;
                action["state"] = "deferred".into();
                action["reason"] = "explicit-defer".into();
                return Ok(());
            }
            let kind = verify_decision(&copy, row, d, &results)?;
            if kind == "retire" {
                let successor = offered
                    .iter()
                    .find(|r| r["memory_id"] == d["successor"])
                    .filter(|s| {
                        s["state"] == "ready"
                            && s["chunk"] != row["chunk"]
                            && s["chunk_sources_fresh"] == true
                            && s["source_image_matches_record"] == true
                            && s["currently_applicable"] == true
                    })
                    .ok_or_else(|| artifact::invalid("fresh selected successor required"))?;
                if !frontier(&copy, successor, true, &results)? {
                    return Err(artifact::invalid("retirement evidence changed"));
                }
                let (old, old_body) = infra_core::bank::codec::parse_raw(&facts::safe_read(
                    &copy,
                    row["chunk"].as_str().unwrap(),
                    true,
                    65536,
                )?)?;
                let (new, new_body) = infra_core::bank::codec::parse_raw(&facts::safe_read(
                    &copy,
                    successor["chunk"].as_str().unwrap(),
                    true,
                    65536,
                )?)?;
                if !applicable(&new) {
                    return Err(artifact::invalid("successor is not applicable"));
                }
                effect = true;
                let result = bank.retire(
                    row["memory_id"].as_str().unwrap(),
                    d["valid_to"]
                        .as_str()
                        .ok_or_else(|| artifact::invalid("retirement date required"))?,
                    successor["memory_id"].as_str(),
                    d["reason"].as_str(),
                )?;
                let (after_old, after_old_body) = infra_core::bank::codec::parse_raw(
                    &facts::safe_read(&copy, row["chunk"].as_str().unwrap(), true, 65536)?,
                )?;
                let (after_new, after_new_body) = infra_core::bank::codec::parse_raw(
                    &facts::safe_read(&copy, successor["chunk"].as_str().unwrap(), true, 65536)?,
                )?;
                let mut expected_old = old;
                expected_old["status"] = "superseded".into();
                expected_old["superseded_by"] = successor["memory_id"].clone();
                expected_old["valid_to"] = d["valid_to"].clone();
                let mut expected_new = new;
                let previous = expected_new["supersedes"]
                    .as_array_mut()
                    .ok_or_else(|| artifact::invalid("successor lineage missing"))?;
                if !previous.contains(&row["memory_id"]) {
                    previous.push(row["memory_id"].clone());
                }
                if after_old != expected_old
                    || after_new != expected_new
                    || after_old_body != old_body
                    || after_new_body != new_body
                    || !applicable(&after_new)
                    || !frontier(&copy, row, false, &results)?
                    || !frontier(&copy, successor, false, &results)?
                {
                    return Err(artifact::invalid(
                        "retirement evidence changed during operation",
                    ));
                }
                action = json!({"chunk":row["chunk"],"state":"applied","reason":null,"result":result,"attestation":"agent"});
                return Ok(());
            }
            let consequence = if kind == "correct" {
                d["consequence"].as_str().unwrap_or("")
            } else {
                row["consequence"].as_str().unwrap_or("")
            };
            if consequence.is_empty()
                || consequence.chars().count() > 1000
                || consequence != crate::scoring::flatten(consequence)
            {
                return Err(artifact::invalid("unsupported claim rendering"));
            }
            facts::public(consequence)?;
            let archived = row["origin_placement"] == "archive";
            let unchanged = consequence == row["consequence"]
                && row["chunk_sources_fresh"] == true
                && row["source_image_matches_record"] == true;
            if archived && !unchanged {
                action["reason"] = "archived-origin-revision-required".into();
                return Ok(());
            }
            if unchanged {
                let (meta, body) = infra_core::bank::codec::parse_raw(&facts::safe_read(
                    &copy,
                    row["chunk"].as_str().unwrap(),
                    true,
                    65536,
                )?)?;
                if meta["review_after"]
                    .as_str()
                    .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
                    .is_some_and(|d| d < chrono::Local::now().date_naive())
                {
                    if !frontier(&copy, row, true, &results)? {
                        return Err(artifact::invalid("renewal frontier changed"));
                    }
                    let start = chrono::Utc::now().date_naive();
                    effect = true;
                    let renewal = bank.reverify(row["memory_id"].as_str().unwrap(), None)?;
                    let finish = chrono::Utc::now().date_naive();
                    let (after, after_body) = infra_core::bank::codec::parse_raw(
                        &facts::safe_read(&copy, row["chunk"].as_str().unwrap(), true, 65536)?,
                    )?;
                    let dates = [start, finish];
                    let legal = dates.iter().any(|date| {
                        let mut expected = meta.clone();
                        expected["last_verified"] = date.to_string().into();
                        expected["review_after"] =
                            (*date + chrono::Duration::days(365)).to_string().into();
                        expected["source_digests"] = after["source_digests"].clone();
                        expected == after
                    });
                    let old_pins = infra_core::results::canonical_image(
                        meta["source_digests"]
                            .as_array()
                            .ok_or_else(|| artifact::invalid("old pins required"))?,
                    )?;
                    let new_pins = infra_core::results::canonical_image(
                        after["source_digests"]
                            .as_array()
                            .ok_or_else(|| artifact::invalid("new pins required"))?,
                    )?;
                    if after_body != body
                        || !legal
                        || finish < start
                        || finish > start + chrono::Duration::days(1)
                        || old_pins != new_pins
                        || !frontier(&copy, row, false, &results)?
                        || renewal["id"] != row["memory_id"]
                        || renewal["chunk"] != row["chunk"]
                        || ["last_verified", "review_after", "source_digests"]
                            .iter()
                            .any(|k| renewal[*k] != after[*k])
                    {
                        return Err(artifact::invalid("renewal evidence changed during check"));
                    }
                    action = json!({"chunk":row["chunk"],"state":"applied","reason":null,"renewal":renewal,"attestation":"agent"});
                    return Ok(());
                }
                if archived {
                    if !frontier(&copy, row, true, &results)? {
                        return Err(artifact::invalid("archive evidence changed"));
                    }
                    action = json!({"chunk":row["chunk"],"state":"applied","reason":null,"unchanged":true,"record_id":row["record_id"],"attestation":"agent"});
                    return Ok(());
                }
            }
            effect = true;
            if task.is_none() {
                task = Some(brain.create_task(
                    &format!(
                        "memory-maintenance/{}",
                        &packet["sha256"].as_str().unwrap()[..16]
                    ),
                    "Apply explicitly checked memory maintenance on a private copy",
                    vec![],
                    vec![],
                    "team",
                )?);
            }
            let payload = json!({"verified":true,"learnings":[{"type":row["type"],"title":row["record_title"],"consequence":consequence,"sources":row["record_sources"],"record_id":row["record_id"],"expected_revision":row["expected_revision"],"source_fingerprints":row["source_fingerprints"]}]});
            let saved = results.record(infra_core::RecordResult {
                task_id: task.as_ref().unwrap()["id"].as_str().unwrap().into(),
                result_id: format!("review-{}", &row["sha256"].as_str().unwrap()[..48]),
                revision: infra_core::Revision::Auto,
                data: payload,
                reason: d["reason"].as_str().unwrap().into(),
                attestation: "agent".into(),
                prepared: None,
                replay_only: false,
            })?;
            if saved["records"].as_array().is_none_or(|a| {
                a.iter()
                    .any(|r| !["updated", "existing"].contains(&r["state"].as_str().unwrap_or("")))
            }) {
                return Err(artifact::invalid("canonical checked save held"));
            }
            flags["automatic_promotion"] = true.into();
            artifact::write_json(&configuration, &flags)?;
            let promotion = bank.auto_promote(
                1,
                Some(1),
                Some(&[row["record_id"].as_str().unwrap().to_string()]),
            );
            flags["automatic_promotion"] = false.into();
            artifact::write_json(&configuration, &flags)?;
            let promotion = promotion?;
            if promotion["failed"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
                || promotion["blocked"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
                || promotion["retry_unavailable"] == true
            {
                action = json!({"chunk":row["chunk"],"state":"failed","reason":"targeted-promotion-incomplete","record_saved":true});
            } else {
                action = json!({"chunk":row["chunk"],"state":"applied","reason":null,"record_id":row["record_id"],"promotion":promotion,"attestation":"agent"});
            }
            Ok(())
        })();
        if run.is_err() {
            action["state"] = if effect { "failed" } else { "held" }.into();
            action["reason"] = "checked-operation-refused".into();
            action["copy_effects_may_have_applied"] = effect.into();
        }
        actions.push(action);
    }
    let audit_after = review_packet(
        &audit(&copy)?,
        &snapshot_digest(&copy)?,
        packet["limit"].as_u64().unwrap_or(0) as usize,
    )?;
    let errors = bank.validate()?;
    let mut schemas = BTreeMap::<String, String>::new();
    for entry in std::fs::read_dir(&schema)? {
        let entry = entry?;
        if entry.path().extension().is_some_and(|s| s == "json") {
            schemas.insert(
                entry.file_name().to_string_lossy().into(),
                artifact::sha(&artifact::read_file(&entry.path(), artifact::MAX_FILE)?.0),
            );
        }
    }
    if snapshot_digest(root)? != before {
        return Err(artifact::invalid("input changed during checked replay"));
    }
    let image = snapshot(&copy)?;
    artifact::copy_image(&image, export)?;
    let success = !actions.is_empty()
        && actions
            .iter()
            .all(|a| ["applied", "deferred"].contains(&a["state"].as_str().unwrap_or("")))
        && errors.is_empty();
    Ok(
        json!({"schema":"memory-maintenance-replay-v1","input_digest":packet["snapshot_digest"],"packet_sha256":packet["sha256"],"actions":actions,"audit_after":audit_after,"validation_errors":errors.len(),"copy_overrides":overrides,"tooling_overrides":{"schemas":"trusted-canonical-copy","runtime_sha256":executable::digest(&std::env::current_exe()?)?,"schema_sha256":artifact::digest(&json!(schemas))?},"targeted_promotion_only":true,"attestation":"agent-supplied-decisions","semantic_verification":"not-independently-established","remaining":packet["remaining"],"success":success,"export_digest":artifact::image_digest(&image,false)?,"input_unchanged":true,"copy_only":true}),
    )
}
