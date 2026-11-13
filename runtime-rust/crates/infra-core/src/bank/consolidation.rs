//! Completion-series consolidation. Historical event images remain the evidence.
use super::{BankService, codec, promotion, publication::PublicationFiles, validation};
use crate::{BrainRepository, CreateRecord, Result, RuntimeError, UpdateRecord, domain, fs};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn text<'a>(record: &'a Value, key: &str) -> &'a str {
    record[key].as_str().unwrap_or("")
}
fn words(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn increment(value: &mut Value, key: &str, n: usize) {
    value[key] = json!(value[key].as_u64().unwrap_or(0) + n as u64);
}
fn clip(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.into();
    }
    let mut chars = value
        .chars()
        .take(limit.saturating_sub(1).max(1))
        .collect::<String>();
    if let Some((byte, _)) = chars.char_indices().rev().find(|(_, c)| *c == ' ')
        && chars[..byte].chars().count() > limit / 2
    {
        chars.truncate(byte);
    }
    while chars.chars().last().is_some_and(|c| " ,;:".contains(c)) {
        chars.pop();
    }
    format!("{chars}…")
}
fn first_sentence(value: &str) -> String {
    let value = words(value);
    let mut end = value.len();
    let mut previous = ' ';
    for (position, current) in value.char_indices() {
        if current.is_whitespace() && ".!?".contains(previous) {
            end = position;
            break;
        }
        previous = current;
    }
    domain::rstrip(&value[..end].chars().take(600).collect::<String>()).into()
}
fn completion_subject(value: &str) -> Option<String> {
    let value = words(value);
    if !value.to_lowercase().starts_with("completed ") {
        return None;
    }
    let (subject, _) = value["completed ".len()..].split_once(": ")?;
    let subject = subject.trim();
    if subject.is_empty() {
        None
    } else {
        Some(subject.into())
    }
}
fn event_subject(record: &Value) -> Result<String> {
    let mut title = words(text(record, "title"));
    if title.to_lowercase().starts_with("completed ") {
        title = title["completed ".len()..].into();
    }
    if let Some((_, tail)) = title.split_once(": ") {
        title = tail.into();
    }
    title = title.trim_matches([' ', ':', '-']).into();
    if title.split_whitespace().count() < 4
        || ["completed", "verification"]
            .iter()
            .any(|p| title.to_lowercase().starts_with(p))
        || title.to_lowercase().contains("auto-provisioned")
        || regex::Regex::new(r"^[A-Z]{2,10}-\d+\b")
            .map_err(|_| RuntimeError::invalid("Ticket expression"))?
            .is_match(&title)
    {
        Ok(String::new())
    } else {
        Ok(title)
    }
}
fn verification_parts(record: &Value) -> Result<Option<(usize, String)>> {
    // The original grammar ends at the next Markdown heading, including a heading on its own line.
    let goal = text(record, "goal");
    let pattern = regex::Regex::new(r"(?is)Verification:\s*(.+?)(?:\s+##|\s*$)")
        .map_err(|_| RuntimeError::invalid("Verification expression"))?;
    Ok(pattern.captures(goal).map(|capture| {
        (
            capture.get(0).map(|m| m.start()).unwrap_or(0),
            words(capture.get(1).map(|m| m.as_str()).unwrap_or("")),
        )
    }))
}
fn verification(record: &Value) -> Result<String> {
    Ok(verification_parts(record)?
        .map(|(_, s)| s)
        .unwrap_or_default())
}
fn outcome(record: &Value) -> Result<String> {
    let goal = text(record, "goal");
    let value = words(
        &goal[..verification_parts(record)?
            .map(|(i, _)| i)
            .unwrap_or(goal.len())],
    );
    let mut normalized = value.as_str();
    while let Some(rest) = normalized.strip_suffix('.') {
        normalized = rest;
    }
    Ok(
        if ["completed", "complete", "done"].contains(&normalized.to_lowercase().as_str()) {
            String::new()
        } else {
            value
        },
    )
}
fn event_line(record: &Value) -> Result<String> {
    let title = words(text(record, "title"));
    let subject = if completion_subject(&title).is_some() {
        title
            .split_once(": ")
            .map(|(_, tail)| tail)
            .unwrap_or(&title)
    } else {
        &title
    };
    let subject = subject.trim_matches([' ', ':', '-']);
    let subject = if subject.is_empty() {
        let external = text(record, "external_id");
        if external.is_empty() {
            "event"
        } else {
            external
        }
    } else {
        subject
    };
    let mut parts = vec![format!("{}:", clip(subject, 160))];
    let result = outcome(record)?;
    if !result.is_empty() {
        parts.push(result);
    }
    let checked = verification(record)?;
    if !checked.is_empty() {
        parts.push(format!("Verification: {checked}"));
    }
    Ok(format!(
        "- {} [event {}]",
        clip(&parts.join(" "), 2000),
        text(record, "id")
    ))
}
fn entry_pattern() -> Result<regex::Regex> {
    regex::Regex::new(
        r"^- .*\[event ([0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})\]$",
    )
    .map_err(|_| RuntimeError::invalid("Event-line expression"))
}
fn covered(record: Option<&Value>) -> Result<Vec<(String, String)>> {
    let pattern = entry_pattern()?;
    Ok(record
        .into_iter()
        .flat_map(|record| text(record, "progress").lines())
        .filter_map(|line| {
            pattern
                .captures(line)
                .map(|capture| (capture[1].into(), line.into()))
        })
        .collect())
}
fn progress(
    ticket: &str,
    previous: &str,
    lines: Vec<String>,
    related: &[String],
) -> Result<String> {
    let pattern = entry_pattern()?;
    let head = regex::Regex::new(r"^Consolidated \d+ completed events? for \S+\.$")
        .map_err(|_| RuntimeError::invalid("Consolidation heading"))?;
    let mut kept = Vec::new();
    let mut entries = Vec::new();
    for line in previous.lines() {
        if pattern.is_match(line) {
            entries.push(line.into());
        } else if !line.trim().is_empty()
            && !head.is_match(line)
            && !line.starts_with("Related findings: ")
        {
            kept.push(line.to_string());
        }
    }
    entries.extend(lines);
    let mut out = vec![format!(
        "Consolidated {} completed event{} for {ticket}.",
        entries.len(),
        if entries.len() == 1 { "" } else { "s" }
    )];
    out.extend(kept);
    out.extend(entries);
    if !related.is_empty() {
        out.push(format!(
            "Related findings: {}.",
            related
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(out.join("\n"))
}
fn ticket_ids(value: &str) -> Result<Vec<String>> {
    const STOP: &str = "AES AGPL ANSI ASCII BCP BSD CET CP CRC CSS CVE CVSS CWE DES DIN DNS DSA ECDSA ECMA EN EPL ES FIPS GDPR GMT GPL GPT HMAC HTML HTTP HTTPS IEC IEEE IP IPV ISO JSON KOI LATIN LGPL MD MPEG MPL NIST OAUTH OWASP PCI PER PHP PSR RFC RSA SHA SOC SSL TCP TLS UCS UDP UTC UTF UUID WCAG WIN XML YAML";
    let pattern = regex::Regex::new(r"\b([A-Z]{2,10})-(\d{1,6})\b")
        .map_err(|_| RuntimeError::invalid("Ticket expression"))?;
    let mut result = Vec::new();
    for capture in pattern.captures_iter(value) {
        let m = capture
            .get(0)
            .ok_or_else(|| RuntimeError::invalid("Ticket capture"))?;
        if STOP.split_whitespace().any(|word| word == &capture[1])
            || value[m.end()..]
                .strip_prefix('.')
                .and_then(|tail| tail.chars().next())
                .is_some_and(|c| c.is_numeric())
        {
            continue;
        }
        let id = m.as_str().to_string();
        if !result.contains(&id) {
            result.push(id);
        }
    }
    Ok(result)
}
fn event_tickets(record: &Value) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for key in ["title", "external_id"] {
        for id in ticket_ids(text(record, key))? {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    Ok(out)
}
fn source_image(service: &BankService, records: &[Value]) -> Option<Vec<Value>> {
    if records.is_empty() {
        return None;
    }
    let mut pins = BTreeMap::<String, String>::new();
    for record in records {
        if verification(record).ok()?.is_empty()
            || record["sources"].as_array()?.is_empty()
            || !domain::sources_are_fresh(&service.context.roots, record).ok()?
        {
            return None;
        }
        for pin in record["source_fingerprints"].as_array()? {
            let path = pin["path"].as_str()?;
            let digest = pin["sha256"].as_str()?;
            if pins.get(path).is_some_and(|old| old != digest) {
                return None;
            }
            pins.insert(path.into(), digest.into());
        }
    }
    Some(
        pins.into_iter()
            .map(|(path, sha256)| json!({"path":path,"sha256":sha256}))
            .collect(),
    )
}
fn merge_images(first: &[Value], second: &[Value]) -> Option<Vec<Value>> {
    let mut pins = BTreeMap::<String, String>::new();
    for row in first.iter().chain(second) {
        let path = row["path"].as_str()?;
        let digest = row["sha256"].as_str()?;
        if pins.get(path).is_some_and(|old| old != digest) {
            return None;
        }
        pins.insert(path.into(), digest.into());
    }
    Some(
        pins.into_iter()
            .map(|(path, sha256)| json!({"path":path,"sha256":sha256}))
            .collect(),
    )
}
fn local_sources(service: &BankService, records: &[Value]) -> Vec<String> {
    let mut sources = Vec::new();
    for record in records {
        for source in record["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if source.is_empty()
                || source.starts_with("http://")
                || source.starts_with("https://")
                || sources.iter().any(|s| s == source)
            {
                continue;
            }
            if domain::source_fingerprints(&service.context.roots, &[source.into()]).is_ok() {
                sources.push(source.into());
            }
        }
    }
    sources
}
fn union_strings(records: &[Value], key: &str) -> Vec<String> {
    domain::unique_strings(
        records
            .iter()
            .flat_map(|record| record[key].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from),
    )
}
fn index_paths(brain: &BrainRepository) -> [std::path::PathBuf; 2] {
    [
        brain.roots().brain_root().join("indexes/active.json"),
        brain.roots().brain_root().join("indexes/archive.json"),
    ]
}
fn current_successor(
    service: &BankService,
    record: &Value,
    ledger: &promotion::Ledger,
) -> Result<Option<String>> {
    let Some(proposal) = ledger.latest.get(text(record, "id")) else {
        return Ok(None);
    };
    if proposal["source_records"][0]["revision"] != record["revision"] {
        return Ok(None);
    }
    let id = text(proposal, "destination_memory_id");
    let result = (|| -> Result<bool> {
        let mut files = PublicationFiles::new(&service.context.roots.state)?;
        let chunk = codec::find(service, id, &mut files)?;
        if chunk.metadata["id"].as_str() != Some(id) || chunk.metadata["status"] != "active" {
            return Ok(false);
        }
        validation::metadata(service, &chunk.path, &chunk.metadata, false)?;
        let config = service.config()?;
        if !config.value["allowed_privacy"]
            .as_array()
            .is_some_and(|items| {
                items.contains(chunk.metadata.get("privacy").unwrap_or(&json!("team")))
            })
            || !config.value["allowed_authority"]
                .as_array()
                .is_some_and(|items| {
                    items.contains(
                        chunk
                            .metadata
                            .get("authority")
                            .unwrap_or(&json!("observed")),
                    )
                })
        {
            return Ok(false);
        }
        let rendered = String::from_utf8(codec::render_chunk(&chunk.metadata, &chunk.body)?)
            .map_err(|_| RuntimeError::invalid("Chunk UTF-8"))?;
        Ok(domain::secret_label(&rendered)?.is_none()&&!promotion::instruction_like(text(&chunk.metadata,"title"),&rendered)?&&domain::citation_problem(&service.context.roots,&json!({"sources":chunk.metadata["sources"],"source_fingerprints":chunk.metadata["source_digests"]}))?.is_none())
    })();
    Ok(if result.unwrap_or(false) {
        Some(id.into())
    } else {
        None
    })
}
fn retire_events(service: &BankService, retiring: &[(Value, String, Vec<Value>)]) -> Result<usize> {
    let brain = service.brain()?;
    let _guard = brain.lock()?;
    let paths = retiring
        .iter()
        .flat_map(|(_, _, members)| members)
        .map(|r| {
            brain
                .find(text(r, "id"), None, false)
                .map(|image| image.path)
        })
        .collect::<Result<Vec<_>>>()?;
    let snapshot = fs::snapshot(paths.into_iter().chain(index_paths(&brain)))?;
    let result = (|| -> Result<usize> {
        let ledger = promotion::ledger(service)?;
        for (volume, memory, _) in retiring {
            let current = brain.find(text(volume, "id"), None, true)?.record;
            if current["revision"] != volume["revision"]
                || current_successor(service, &current, &ledger)?.as_deref() != Some(memory)
            {
                return Err(RuntimeError::conflict(
                    "Consolidated successor became unavailable; events stay recorded",
                ));
            }
        }
        let mut count = 0;
        for (volume, memory, members) in retiring {
            for event in members {
                brain.update(
                    text(event, "id"),
                    UpdateRecord {
                        expected_revision: event["revision"].as_u64(),
                        actor: service.context.actor.clone(),
                        transition_to: Some("superseded".into()),
                        reason: format!(
                            "Consolidated into {}, carried by {memory}",
                            text(volume, "external_id")
                        ),
                        ..UpdateRecord::default()
                    },
                )?;
                count += 1;
            }
        }
        Ok(count)
    })();
    if result.is_err() {
        fs::restore(&snapshot)?;
    }
    result
}
#[expect(
    clippy::too_many_arguments,
    reason = "Consolidation extension binds the prior summary, event proof, reporting and apply/verification controls"
)]
fn extend(
    service: &BankService,
    summary: &Value,
    ticket: &str,
    events: &[Value],
    related: &[String],
    entry: &mut Value,
    report: &mut Value,
    apply: bool,
    verify_enabled: bool,
) -> Result<()> {
    entry["finding"] = summary["external_id"].clone();
    let verified = summary["authority"] == "verified";
    if !summary["authorized_owners"]
        .as_array()
        .is_some_and(|owners| {
            owners
                .iter()
                .any(|owner| owner.as_str() == Some(service.context.actor.as_str()))
        })
    {
        entry["state"] = "skipped".into();
        entry["reason"] = format!(
            "the finding belongs to {}; the events stay recorded",
            text(summary, "owner")
        )
        .into();
        return Ok(());
    }
    let mut checked = source_image(service, events);
    if verified
        && !(verify_enabled
            && checked.is_some()
            && domain::sources_are_fresh(&service.context.roots, summary).unwrap_or(false))
    {
        entry["state"] = "skipped".into();
        entry["reason"] =
            "the verified finding takes only verified events; they stay recorded".into();
        return Ok(());
    }
    let rendered = progress(
        ticket,
        text(summary, "progress"),
        events.iter().map(event_line).collect::<Result<_>>()?,
        related,
    )?;
    let sources = local_sources(service, events);
    let mut carried = rendered.lines().skip(1).collect::<Vec<_>>();
    if carried
        .last()
        .is_some_and(|line| line.starts_with("Related findings: "))
    {
        carried.pop();
    }
    let pattern = entry_pattern()?;
    let mut harden = !verified
        && verify_enabled
        && (!summary["sources"].as_array().is_none_or(Vec::is_empty) || !sources.is_empty())
        && !carried.is_empty()
        && carried
            .iter()
            .all(|line| pattern.is_match(line) && line.contains(" Verification: "));
    let brain = service.brain()?;
    if harden {
        let previous = covered(Some(summary))?
            .into_iter()
            .map(|(id, _)| {
                brain
                    .find(&id, Some("event"), true)
                    .map(|image| image.record)
            })
            .collect::<Result<Vec<_>>>();
        checked = previous.ok().and_then(|mut old| {
            old.extend_from_slice(events);
            source_image(service, &old)
        });
        harden = checked.is_some();
    } else if verified {
        checked = summary["source_fingerprints"]
            .as_array()
            .zip(checked.as_ref())
            .and_then(|(first, second)| merge_images(first, second));
        if checked.is_none() {
            entry["state"] = "skipped".into();
            entry["reason"] =
                "consolidated verification images disagree; events stay recorded".into();
            return Ok(());
        }
    }
    entry["state"] = "planned-update".into();
    entry["adds"] = json!(events.len());
    if !apply {
        return Ok(());
    }
    let _guard = brain.lock()?;
    let path = brain
        .find(text(summary, "id"), Some("finding"), false)?
        .path;
    let snapshot = fs::snapshot([path].into_iter().chain(index_paths(&brain)))?;
    let result = brain.update(
        text(summary, "id"),
        UpdateRecord {
            expected_revision: summary["revision"].as_u64(),
            progress: Some(rendered),
            files: union_strings(events, "files"),
            sources,
            actor: service.context.actor.clone(),
            authority: if harden {
                Some("verified".into())
            } else {
                None
            },
            attestation: if harden || verified {
                Some("agent".into())
            } else {
                None
            },
            reverify: verified,
            checked_source_fingerprints: if harden || verified { checked } else { None },
            reason: format!("Added {} completed event(s) of {ticket}", events.len()),
            ..UpdateRecord::default()
        },
    );
    if let Err(error) = result {
        fs::restore(&snapshot)?;
        entry["state"] = "blocked".into();
        entry["reason"] = error.to_string().into();
        increment(report, "blocked", 1);
        return Ok(());
    }
    entry["state"] = "updated".into();
    entry["verified"] = json!(verified || harden);
    increment(report, "updated", 1);
    if harden {
        increment(report, "verified", 1);
    }
    Ok(())
}
#[expect(
    clippy::too_many_arguments,
    reason = "Consolidation creation carries event evidence, ticket relations, mutable report and explicit apply/verification policy"
)]
fn create(
    service: &BankService,
    ticket: &str,
    events: &[Value],
    related: &[String],
    entry: &mut Value,
    report: &mut Value,
    apply: bool,
    verify_enabled: bool,
) -> Result<()> {
    let mut ordered = events.to_vec();
    ordered.sort_by_key(|event| text(event, "created_at").to_string());
    let latest = ordered
        .last()
        .ok_or_else(|| RuntimeError::invalid("No consolidation events"))?;
    let mut goal = String::new();
    for event in ordered.iter().rev() {
        goal = event_subject(event)?;
        if !goal.is_empty() {
            break;
        }
    }
    if goal.is_empty() {
        goal = first_sentence(text(latest, "goal"));
    }
    if goal.is_empty() {
        goal = first_sentence(text(latest, "title"));
    }
    if goal.is_empty() {
        goal = ticket.into();
    }
    let mut title = format!("{ticket}: {goal}");
    if title.chars().count() > 160 {
        title = format!(
            "{}...",
            domain::rstrip(&title.chars().take(157).collect::<String>())
        );
    }
    let external = format!("CONSOLIDATED-{ticket}");
    let files = union_strings(&ordered, "files");
    let sources = local_sources(service, &ordered);
    let rendered = progress(
        ticket,
        "",
        ordered.iter().map(event_line).collect::<Result<_>>()?,
        related,
    )?;
    let checked = source_image(service, &ordered);
    let verify = verify_enabled && checked.is_some() && !sources.is_empty();
    entry["state"] = "planned".into();
    entry["finding"] = external.clone().into();
    entry["title"] = title.clone().into();
    entry["sources"] = json!(sources.len());
    entry["adds"] = json!(events.len());
    if !apply {
        return Ok(());
    }
    let brain = service.brain()?;
    let _guard = brain.lock()?;
    let snapshot = fs::snapshot(index_paths(&brain))?;
    let mut created = None::<String>;
    let result = (|| -> Result<()> {
        let record = brain.create(CreateRecord {
            record_type: "finding".into(),
            external_id: external,
            title,
            goal: Some(goal),
            files,
            sources,
            owner: service.context.actor.clone(),
            ..CreateRecord::default()
        })?;
        created = record["id"].as_str().map(String::from);
        let updated = brain.update(
            text(&record, "id"),
            UpdateRecord {
                expected_revision: record["revision"].as_u64(),
                progress: Some(rendered),
                actor: service.context.actor.clone(),
                transition_to: Some("resolved".into()),
                reason: format!("Consolidated {} completed events", events.len()),
                ..UpdateRecord::default()
            },
        )?;
        if verify {
            brain.update(
                text(&record, "id"),
                UpdateRecord {
                    expected_revision: updated["revision"].as_u64(),
                    actor: service.context.actor.clone(),
                    authority: Some("verified".into()),
                    attestation: Some("agent".into()),
                    checked_source_fingerprints: checked,
                    reason: "Every consolidated event recorded verification".into(),
                    ..UpdateRecord::default()
                },
            )?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        if let Some(id) = created {
            let _ = brain.rollback_created(&id);
        }
        fs::restore(&snapshot)?;
        entry["state"] = "blocked".into();
        entry["reason"] = error.to_string().into();
        increment(report, "blocked", 1);
        return Ok(());
    }
    entry["state"] = "created".into();
    entry["verified"] = json!(verify);
    increment(report, "created", 1);
    if verify {
        increment(report, "verified", 1);
    }
    Ok(())
}
pub fn consolidate(service: &BankService, apply: bool, wanted: Option<&str>) -> Result<Value> {
    let brain = service.brain()?;
    let verify_enabled = service
        .config()?
        .enabled("automatic_consolidation_verification");
    let wanted = wanted
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_uppercase);
    let active = brain.records(false)?;
    let mut summaries = BTreeMap::<String, Value>::new();
    let mut groups = BTreeMap::<String, Vec<Value>>::new();
    let mut foreign = BTreeMap::<String, usize>::new();
    for image in active {
        let record = image.record;
        if record["type"] == "finding" {
            if let Some(ticket) = text(&record, "external_id").strip_prefix("CONSOLIDATED-") {
                summaries.insert(ticket.into(), record);
            }
            continue;
        }
        if record["type"] != "event"
            || record["status"] != "recorded"
            || !["public", "team"].contains(&text(&record, "privacy"))
            || !text(&record, "title").to_lowercase().contains("completed")
        {
            continue;
        }
        let tickets = event_tickets(&record)?;
        let Some(ticket) = tickets.first() else {
            continue;
        };
        if wanted.as_ref().is_some_and(|w| w != ticket) {
            continue;
        }
        if record["authorized_owners"]
            .as_array()
            .is_some_and(|owners| {
                owners
                    .iter()
                    .any(|o| o.as_str() == Some(service.context.actor.as_str()))
            })
        {
            groups.entry(ticket.clone()).or_default().push(record);
        } else {
            *foreign.entry(ticket.clone()).or_default() += 1;
        }
    }
    let mut report = json!({"applied":apply,"tickets":[],"created":0,"updated":0,"existing":0,"verified":0,"superseded":0,"blocked":0});
    let mut finding_index = None::<(BTreeMap<String, Vec<Value>>, BTreeMap<String, Vec<Value>>)>;
    let mut ledger = None::<promotion::Ledger>;
    let tickets = groups
        .keys()
        .chain(foreign.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for ticket in tickets {
        let mut events = groups.remove(&ticket).unwrap_or_default();
        events.sort_by_key(|r| (text(r, "created_at").to_string(), text(r, "id").to_string()));
        let mut entry = json!({"ticket":ticket,"events":events.len(),"ids":events.iter().map(|r|r["id"].clone()).collect::<Vec<_>>()});
        if let Some(n) = foreign.get(&ticket) {
            entry["foreign"] = json!(n);
        }
        let process = (|| -> Result<()> {
            if events.is_empty() {
                entry["state"] = "skipped".into();
                entry["reason"] =
                    "the completed events belong to another owner, who consolidates them".into();
                return Ok(());
            }
            let summary = summaries.get(&ticket);
            let old = covered(summary)?;
            let uncovered = events
                .iter()
                .any(|r| !old.iter().any(|(id, _)| id == text(r, "id")));
            if summary.is_some_and(|s| !uncovered && s["authority"] != "verified") {
                entry["state"] = "unchanged".into();
                entry["finding"] = summary.unwrap()["external_id"].clone();
                entry["reason"] =
                    "the events stay recorded until the finding is verified and promoted".into();
                return Ok(());
            }
            if (summary.is_none() || uncovered) && finding_index.is_none() {
                let mut archived = BTreeMap::<String, Vec<Value>>::new();
                let mut related = BTreeMap::<String, Vec<Value>>::new();
                for image in brain.records(true)? {
                    let r = image.record;
                    if r["type"] != "finding" {
                        continue;
                    }
                    if let Some(ticket) = text(&r, "external_id").strip_prefix("CONSOLIDATED-") {
                        if image.archived {
                            archived.entry(ticket.into()).or_default().push(r);
                        }
                    } else {
                        for ticket in ticket_ids(
                            &format!("{} {}", text(&r, "external_id"), text(&r, "title"))
                                .to_uppercase(),
                        )? {
                            related.entry(ticket).or_default().push(r.clone());
                        }
                    }
                }
                finding_index = Some((archived, related));
            }
            let mut volumes = summary.into_iter().cloned().collect::<Vec<_>>();
            if (summary.is_none() || uncovered)
                && let Some((archived, _)) = &finding_index
            {
                volumes.extend(archived.get(&ticket).into_iter().flatten().cloned());
            }
            let mut taken = BTreeSet::new();
            let mut retiring = Vec::new();
            for volume in volumes {
                let carried = covered(Some(&volume))?;
                taken.extend(carried.iter().map(|(id, _)| id.clone()));
                let members = events
                    .iter()
                    .filter(|r| carried.iter().any(|(id, _)| id == text(r, "id")))
                    .cloned()
                    .collect::<Vec<_>>();
                if members.is_empty() || volume["authority"] != "verified" {
                    continue;
                }
                if ledger.is_none() {
                    ledger = Some(promotion::ledger(service)?);
                }
                if let Some(memory) = current_successor(service, &volume, ledger.as_ref().unwrap())?
                {
                    retiring.push((volume, memory, members));
                }
            }
            if !retiring.is_empty() {
                entry["supersede"] = json!(
                    retiring
                        .iter()
                        .map(|(_, _, members)| members.len())
                        .sum::<usize>()
                );
                if apply {
                    match retire_events(service, &retiring) {
                        Ok(n) => {
                            entry["superseded"] = json!(n);
                            increment(&mut report, "superseded", n);
                        }
                        Err(error) => {
                            entry["state"] = "blocked".into();
                            entry["reason"] = error.to_string().into();
                            increment(&mut report, "blocked", 1);
                            return Ok(());
                        }
                    }
                }
            }
            let remaining = events
                .iter()
                .filter(|r| !taken.contains(text(r, "id")))
                .cloned()
                .collect::<Vec<_>>();
            if remaining.is_empty() {
                if !retiring.is_empty() {
                    entry["state"] = if apply {
                        "superseded"
                    } else {
                        "planned-supersede"
                    }
                    .into();
                } else {
                    entry["state"] = "unchanged".into();
                    entry["finding"] = summary
                        .map(|s| s["external_id"].clone())
                        .unwrap_or(Value::Null);
                    entry["reason"] =
                        "the events stay recorded until the finding's revision is promoted".into();
                }
                return Ok(());
            }
            let related = finding_index
                .as_ref()
                .and_then(|(_, related)| related.get(&ticket))
                .cloned()
                .unwrap_or_default();
            let names = related
                .iter()
                .map(|r| format!("{} ({})", text(r, "external_id"), text(r, "id")))
                .collect::<Vec<_>>();
            if let Some(summary) = summary {
                return extend(
                    service,
                    summary,
                    &ticket,
                    &remaining,
                    &names,
                    &mut entry,
                    &mut report,
                    apply,
                    verify_enabled,
                );
            }
            if let Some(resolved) = related.iter().find(|r| r["status"] == "resolved") {
                entry["state"] = "existing".into();
                entry["finding"] = resolved["external_id"].clone();
                entry["reason"] =
                    "the ticket has a resolved finding; the events stay recorded".into();
                increment(&mut report, "existing", 1);
                return Ok(());
            }
            if remaining.len() < 2 {
                entry["state"] = "skipped".into();
                entry["reason"] = "fewer than two completed events".into();
                return Ok(());
            }
            create(
                service,
                &ticket,
                &remaining,
                &names,
                &mut entry,
                &mut report,
                apply,
                verify_enabled,
            )
        })();
        process?;
        report["tickets"]
            .as_array_mut()
            .ok_or_else(|| RuntimeError::invalid("Consolidation report"))?
            .push(entry);
    }
    Ok(report)
}
