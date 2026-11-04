use super::{
    BankService, codec, maintenance,
    publication::{self, IMAGE_LIMIT, PLAN_LIMIT, PublicationFiles},
    retirement::strict,
    validation,
};
use crate::{
    ErrorKind, Result, RuntimeError,
    domain::{self, json},
};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const KEYS: &[&str] = &[
    "schema_version",
    "id",
    "status",
    "revision",
    "proposer",
    "reviewer",
    "review_mode",
    "reviewed_at",
    "created_at",
    "updated_at",
    "source_records",
    "title",
    "content",
    "destination_memory_id",
    "destination_revision",
    "conflicts",
    "outcome",
];
pub fn verification_image(record: &Value) -> Result<String> {
    let checks = record["transitions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["from"] == "observed" && row["to"] == "verified")
        .cloned()
        .collect::<Vec<_>>();
    publication::digest(&object!(checks))
}
fn path(id: &str) -> Result<String> {
    if !domain::is_uuid4(id) {
        return Err(RuntimeError::invalid("Promotion ID must be UUIDv4"));
    }
    Ok(format!("project-brain/control/promotions/{id}.json"))
}
fn raw_json(bytes: &[u8]) -> Result<Value> {
    json::parse(
        std::str::from_utf8(bytes).map_err(|_| RuntimeError::invalid("Promotion is not UTF-8"))?,
    )
}
fn write(
    files: &mut PublicationFiles,
    id: &str,
    value: &Value,
    expected: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let bytes = format!("{}\n", json::sorted_pretty(value)?).into_bytes();
    files.write_expected(&path(id)?, &bytes, expected)?;
    Ok(bytes)
}

pub fn validate(service: &BankService, proposal: &Value, expected: Option<&str>) -> Result<()> {
    let o = proposal
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Promotion must be an object"))?;
    if KEYS.iter().any(|key| !o.contains_key(*key))
        || o.keys().any(|key| {
            !KEYS.contains(&key.as_str())
                && !["source_verifications", "publication"].contains(&key.as_str())
        })
    {
        return Err(RuntimeError::invalid(
            "Promotion does not match strict schema",
        ));
    }
    domain::validate_schema_file(&service.context.roots, "promotion.schema.json", proposal)?;
    let id = proposal["id"].as_str().unwrap_or("");
    if !domain::is_uuid4(id)
        || expected.is_some_and(|expected| expected != id)
        || proposal["schema_version"].as_u64() != Some(1)
        || !proposal["revision"]
            .as_u64()
            .is_some_and(|revision| revision > 0)
    {
        return Err(RuntimeError::invalid(
            "Invalid promotion identity/schema/revision",
        ));
    }
    let status = proposal["status"].as_str().unwrap_or("");
    let automatic = proposal["review_mode"] == "automatic";
    if !["proposed", "reviewed", "rejected", "applied"].contains(&status)
        || !["human", "automatic"].contains(&proposal["review_mode"].as_str().unwrap_or(""))
    {
        return Err(RuntimeError::invalid("Invalid promotion status or mode"));
    }
    if automatic && !proposal["reviewer"].is_null()
        || !automatic
            && ["reviewed", "applied"].contains(&status)
            && proposal["reviewer"].as_str().is_none_or(|s| s.is_empty())
    {
        return Err(RuntimeError::invalid("Promotion review provenance differs"));
    }
    let outcome = proposal["outcome"].as_str();
    if status == "proposed" && !proposal["outcome"].is_null()
        || status == "rejected" && outcome != Some("rejected")
        || ["reviewed", "applied"].contains(&status)
            && outcome
                != Some(if automatic {
                    "approved-without-review"
                } else {
                    "approved"
                })
            && !(status == "applied" && outcome == Some("promoted"))
    {
        return Err(RuntimeError::invalid("Promotion review outcome is invalid"));
    }
    for key in ["proposer", "title", "content", "created_at", "updated_at"] {
        if proposal[key]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
        {
            return Err(RuntimeError::invalid("Promotion required text is empty"));
        }
    }
    let sources = proposal["source_records"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| RuntimeError::invalid("Promotion requires source records"))?;
    for source in sources {
        strict(source, &["id", "type", "path", "revision"])?;
        if !domain::is_uuid4(source["id"].as_str().unwrap_or(""))
            || !domain::RECORD_TYPES.contains(&source["type"].as_str().unwrap_or(""))
            || source["path"].as_str().is_none_or(|s| s.is_empty())
            || !source["revision"].as_u64().is_some_and(|n| n > 0)
        {
            return Err(RuntimeError::invalid("Invalid promotion source binding"));
        }
    }
    if let Some(checks) = proposal.get("source_verifications") {
        let checks = checks
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("Invalid promotion verification images"))?;
        let mut seen = BTreeSet::new();
        for row in checks {
            strict(row, &["id", "sha256"])?;
            let id = row["id"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Invalid verification ID"))?;
            if !seen.insert(id) || !hash(row["sha256"].as_str().unwrap_or("")) {
                return Err(RuntimeError::invalid(
                    "Invalid promotion verification image",
                ));
            }
        }
        if seen
            != sources
                .iter()
                .filter_map(|row| row["id"].as_str())
                .collect()
        {
            return Err(RuntimeError::invalid(
                "Incomplete promotion verification images",
            ));
        }
    }
    Ok(())
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn list(service: &BankService, strict_mode: bool) -> Result<Vec<Value>> {
    let root = service
        .context
        .roots
        .state
        .join("project-brain/control/promotions");
    if !crate::fs::admit_directory(&root)? {
        return Ok(vec![]);
    }
    let mut entries = std::fs::read_dir(root)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut output = Vec::new();
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    for entry in entries {
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        let result = (|| -> Result<Value> {
            let id = entry
                .path()
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_string();
            let raw = files
                .read(&path(&id)?, 2 * PLAN_LIMIT)?
                .ok_or_else(|| RuntimeError::unavailable("Promotion unavailable"))?;
            let value = raw_json(&raw)?;
            if strict_mode {
                let mut base = value.clone();
                base.as_object_mut()
                    .ok_or_else(|| RuntimeError::invalid("Promotion must be an object"))?
                    .remove("publication");
                validate(service, &base, Some(&id))?;
                if value.get("publication").is_some() && value["status"] != "reviewed" {
                    return Err(RuntimeError::invalid(
                        "Publication requires a reviewed proposal",
                    ));
                }
            }
            Ok(value)
        })();
        match result {
            Ok(value) => output.push(value),
            Err(error) if strict_mode => return Err(error),
            Err(_) => {}
        }
    }
    Ok(output)
}
#[derive(Default)]
pub struct Ledger {
    pub held: BTreeSet<String>,
    pub latest: BTreeMap<String, Value>,
}
pub fn ledger(service: &BankService) -> Result<Ledger> {
    let mut ledger = Ledger::default();
    for proposal in list(service, false)? {
        if proposal["status"] == "rejected" {
            continue;
        }
        let sources = proposal["source_records"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| {
                row.is_object()
                    && row["id"].is_string()
                    && (row["revision"].is_i64() || row["revision"].is_u64())
            })
            .cloned()
            .collect::<Vec<_>>();
        if proposal["status"] != "applied" {
            if proposal["review_mode"] != "automatic" {
                for source in sources {
                    ledger
                        .held
                        .insert(source["id"].as_str().unwrap_or("").to_owned());
                }
            }
            continue;
        }
        if sources.len() != 1 {
            for source in sources {
                ledger
                    .held
                    .insert(source["id"].as_str().unwrap_or("").to_owned());
            }
            continue;
        }
        let id = sources[0]["id"].as_str().unwrap_or("");
        let revision = sources[0]["revision"]
            .as_i64()
            .map(i128::from)
            .or_else(|| sources[0]["revision"].as_u64().map(i128::from))
            .unwrap_or(0);
        let previous = ledger
            .latest
            .get(id)
            .map(|p| &p["source_records"][0]["revision"])
            .and_then(|r| {
                r.as_i64()
                    .map(i128::from)
                    .or_else(|| r.as_u64().map(i128::from))
            });
        if previous.is_none_or(|old| revision > old) {
            ledger.latest.insert(id.to_owned(), proposal);
        }
    }
    Ok(ledger)
}

pub fn propose(
    service: &BankService,
    ids: &[String],
    title: &str,
    content: &str,
    automatic: bool,
) -> Result<Value> {
    if ids.is_empty() {
        return Err(RuntimeError::invalid("Promotion requires source records"));
    }
    let _guard = service.brain()?.lock()?;
    let brain = service.brain()?;
    let mut sources = Vec::new();
    let mut checks = Vec::new();
    for id in ids {
        let image = brain.find(id, None, true)?;
        sources.push(object!({"id":image.record["id"],"type":image.record["type"],"path":relative(service,&image.path)?,"revision":image.record["revision"]}));
        checks.push(object!({"id":image.record["id"],"sha256":verification_image(&image.record)?}));
    }
    let timestamp = domain::utc_now();
    let proposal = object!({"schema_version":1,"id":domain::uuid4(),"status":"proposed","revision":1,"proposer":service.context.actor,"reviewer":null,"review_mode":if automatic{"automatic"}else{"human"},"reviewed_at":null,"created_at":timestamp,"updated_at":timestamp,"source_records":sources,"title":title,"content":content,"source_verifications":checks,"destination_memory_id":null,"destination_revision":null,"conflicts":[],"outcome":null});
    validate(service, &proposal, None)?;
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    write(
        &mut files,
        proposal["id"].as_str().unwrap_or(""),
        &proposal,
        None,
    )?;
    Ok(proposal)
}
pub fn review(
    service: &BankService,
    id: &str,
    reviewer: Option<&str>,
    approve: bool,
) -> Result<Value> {
    let _guard = service.brain()?.lock()?;
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let raw = files
        .read(&path(id)?, 2 * PLAN_LIMIT)?
        .ok_or_else(|| RuntimeError::not_found("Promotion not found"))?;
    let mut value = raw_json(&raw)?;
    validate(service, &value, Some(id))?;
    if value["status"] != "proposed" {
        return Err(RuntimeError::policy(
            "Only proposed promotions may be reviewed",
        ));
    }
    let automatic = value["review_mode"] == "automatic";
    if automatic && reviewer.is_some() || !automatic && reviewer.is_none() {
        return Err(RuntimeError::policy(
            "Human and automatic review boundaries cannot be interchanged",
        ));
    }
    if reviewer.is_some_and(|reviewer| value["proposer"].as_str() == Some(reviewer)) {
        return Err(RuntimeError::policy(
            "Promotion reviewer must be independent from proposer",
        ));
    }
    value["status"] = if approve { "reviewed" } else { "rejected" }.into();
    value["reviewer"] = object!(reviewer);
    value["reviewed_at"] = domain::utc_now().into();
    value["updated_at"] = domain::utc_now().into();
    value["revision"] = (value["revision"].as_u64().unwrap_or(0) + 1).into();
    value["outcome"] = if !approve {
        "rejected"
    } else if automatic {
        "approved-without-review"
    } else {
        "approved"
    }
    .into();
    validate(service, &value, Some(id))?;
    write(&mut files, id, &value, Some(&raw))?;
    Ok(value)
}
fn python_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
}
fn rstrip_period(text: &str) -> &str {
    let mut end = text.len();
    while end > 0 && text.as_bytes()[end - 1] == b'.' {
        end -= 1;
    }
    &text[..end]
}
pub fn content(record: &Value) -> Option<String> {
    let title = record["title"]
        .as_str()
        .unwrap_or("")
        .trim_matches(python_space);
    let progress = record["progress"]
        .as_str()
        .unwrap_or("")
        .split(python_space)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if progress.is_empty()
        || crate::query::folded(rstrip_period(&progress))
            == crate::query::folded(rstrip_period(title))
    {
        return None;
    }
    let sources = record["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| !s.trim_matches(python_space).is_empty())
        .collect::<Vec<_>>();
    Some(if sources.is_empty() {
        progress
    } else {
        format!(
            "{progress}\n\n## Sources\n{}",
            sources
                .iter()
                .map(|source| format!("- {source}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}
fn instruction_patterns() -> Vec<String> {
    vec![
        r"\bignore\s+(?:all\s+)?(?:previous|prior|above)\s+instructions?\b".into(),
        r"\bdisregard\s+(?:all\s+)?(?:previous|prior|above|your)\s+instructions?\b".into(),
        r"\bforget\s+(?:all\s+)?(?:previous|prior|above|your)\s+instructions?\b".into(),
        r"\bsystem\s+prompt\b".into(),
        format!(r"<\|(?:im_start|im_{}|system|assistant)\|>", "end"),
    ]
}
pub fn instruction_like(title: &str, text: &str) -> Result<bool> {
    let joined = [title, text]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    for pattern in instruction_patterns() {
        if domain::policy_pattern_matches(&pattern, 34, &joined)? {
            return Ok(true);
        }
    }
    Ok(false)
}
pub fn provenance_policy_fingerprint() -> String {
    publication::sha256(
        instruction_patterns()
            .iter()
            .map(|pattern| format!("34:{pattern}"))
            .collect::<Vec<_>>()
            .join("\n")
            .as_bytes(),
    )
}
fn eligibility(
    service: &BankService,
    record: &Value,
    allow_changed_sources: bool,
) -> Result<Option<String>> {
    let config = service.config()?;
    eligibility_with_config(service, record, allow_changed_sources, &config)
}
fn eligibility_with_config(
    service: &BankService,
    record: &Value,
    allow_changed_sources: bool,
    config: &crate::RuntimeConfig,
) -> Result<Option<String>> {
    let kind = record["type"].as_str().unwrap_or("");
    let status = record["status"].as_str().unwrap_or("");
    let wanted = match kind {
        "finding" | "bug" => "resolved",
        "incident" => "closed",
        "decision" => "accepted",
        _ => return Ok(Some(format!("{kind} records are not promotable"))),
    };
    if status != wanted {
        return Ok(Some(format!("{kind} status {status} is not promotable")));
    }
    if record["authority"] != "verified" {
        return Ok(Some(format!(
            "authority is {}, not verified",
            record["authority"].as_str().unwrap_or("")
        )));
    }
    let privacy = record["privacy"].as_str().unwrap_or("");
    if !config
        .strings("allowed_privacy")?
        .iter()
        .any(|p| p == privacy)
    {
        return Ok(Some(format!(
            "privacy {privacy} is not allowed for retrieval"
        )));
    }
    let Some(body) = content(record) else {
        return Ok(Some(
            "record carries no content beyond its own title".into(),
        ));
    };
    if let Some(found) = domain::sensitive_label(&format!(
        "{}\n{body}",
        record["title"].as_str().unwrap_or("")
    ))? {
        return Ok(Some(format!("record content contains {found}")));
    }
    if domain::sources_are_fresh(&service.context.roots, record)? {
        return Ok(None);
    }
    if allow_changed_sources && domain::citation_problem(&service.context.roots, record)?.is_none()
    {
        return Ok(None);
    }
    let mut stale = Vec::new();
    for row in record["source_fingerprints"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let one = object!({"sources":[row["path"]],"source_fingerprints":[row]});
        if !domain::sources_are_fresh(&service.context.roots, &one)?
            && let Some(path) = row["path"].as_str()
        {
            stale.push(path.to_owned());
        }
    }
    if stale.is_empty() {
        stale = codec::strings(record, "sources", true)?
    }
    Ok(Some(format!(
        "cited source changed since the record was written: {}",
        stale.join(", ")
    )))
}
pub fn provenance_problems(record: &Value, body: Option<&str>) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    if let Some(source) = record["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|s| s.starts_with("https://") || s.starts_with("http://"))
    {
        problems.push(format!("external source requires review: {source}"))
    }
    if instruction_like(record["title"].as_str().unwrap_or(""), body.unwrap_or(""))? {
        problems.push("instruction-like content requires review".into())
    }
    Ok(problems)
}
fn provenance(record: &Value, body: &str) -> Result<Option<String>> {
    let problems = provenance_problems(record, Some(body))?;
    Ok((!problems.is_empty()).then(|| problems.join("; ")))
}
fn relative(service: &BankService, path: &Path) -> Result<String> {
    path.strip_prefix(&service.context.roots.state)
        .ok()
        .and_then(|path| path.to_str())
        .map(|s| s.replace('\\', "/"))
        .ok_or_else(|| RuntimeError::policy("Promotion source path escapes state"))
}
fn source_images(
    service: &BankService,
    proposal: &Value,
    files: &mut PublicationFiles,
) -> Result<(Vec<String>, BTreeMap<String, String>, bool)> {
    let mut inherited = BTreeSet::new();
    let mut digests = BTreeMap::new();
    let mut agent = false;
    for source in proposal["source_records"]
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("Invalid promotion sources"))?
    {
        let image = service
            .brain()?
            .find(source["id"].as_str().unwrap_or(""), None, true)?;
        let record = &image.record;
        if record["revision"] != source["revision"]
            || record["type"] != source["type"]
            || relative(service, &image.path)? != source["path"].as_str().unwrap_or("")
        {
            return Err(RuntimeError::conflict(
                "Promotion source record binding changed",
            ));
        }
        if let Some(reason) = eligibility(service, record, proposal["review_mode"] == "automatic")?
        {
            return Err(RuntimeError::policy(reason));
        }
        if proposal["review_mode"] == "automatic"
            && let Some(reason) = provenance(record, proposal["content"].as_str().unwrap_or(""))?
        {
            return Err(RuntimeError::policy(reason));
        }
        let path = source["path"].as_str().unwrap_or("");
        let raw = files
            .read(path, 4 * IMAGE_LIMIT)?
            .ok_or_else(|| RuntimeError::unavailable("Promotion source image missing"))?;
        inherited.insert(path.into());
        digests.insert(path.into(), memory_kernel_probe::fingerprint_bytes(&raw));
        for cited in codec::strings(record, "sources", true)? {
            if service.context.roots.resolve(&cited).is_file() {
                inherited.insert(cited);
            }
        }
        for row in record["source_fingerprints"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let (Some(path), Some(hash)) = (row["path"].as_str(), row["sha256"].as_str()) {
                digests.insert(path.into(), hash.into());
            }
        }
        agent |= domain::record_attestation(record) == "agent";
    }
    Ok((inherited.into_iter().collect(), digests, agent))
}

#[derive(Clone)]
struct Plan {
    destination: String,
    after: Vec<u8>,
    predecessor: Option<(String, Vec<u8>, Vec<u8>)>,
    sources: Vec<Value>,
    metadata: Value,
}
fn plan(proposal: &Value) -> Result<Plan> {
    let p = &proposal["publication"];
    strict(
        p,
        &[
            "profile",
            "phase",
            "original_updated_at",
            "proposal_sha256",
            "destination",
            "predecessor",
            "sources",
            "sha256",
        ],
    )?;
    publication::verify_seal(p)?;
    let mut base = proposal.clone();
    base.as_object_mut()
        .ok_or_else(|| RuntimeError::invalid("Promotion must be an object"))?
        .remove("publication");
    if p["profile"] != "promotion-publication-v1"
        || !["publish", "rollback"].contains(&p["phase"].as_str().unwrap_or(""))
        || proposal["status"] != "reviewed"
        || p["proposal_sha256"].as_str() != Some(publication::digest(&base)?.as_str())
        || json::serialize(p, false, false, false, false)?.len() > PLAN_LIMIT
    {
        return Err(RuntimeError::invalid(
            "Invalid promotion publication binding",
        ));
    }
    strict(&p["destination"], &["path", "after"])?;
    let destination = p["destination"]["path"]
        .as_str()
        .ok_or_else(|| RuntimeError::invalid("Invalid destination"))?
        .to_string();
    let after = publication::unblob(&p["destination"]["after"])?;
    let (metadata, body) = codec::parse_raw(&after)?;
    strict(
        &metadata,
        &[
            "id",
            "title",
            "type",
            "status",
            "scope",
            "tags",
            "created",
            "last_verified",
            "review_after",
            "sources",
            "supersedes",
            "superseded_by",
            "valid_from",
            "valid_to",
            "source_digests",
        ],
    )?;
    let title = proposal["title"].as_str().unwrap_or("");
    let heading = format!("# {title}");
    let content = proposal["content"].as_str().unwrap_or("").trim();
    let expected = if content.starts_with(&heading) {
        content.to_string()
    } else {
        format!("{heading}\n\n{content}")
    };
    let source = &proposal["source_records"][0];
    let uuid = source["id"].as_str().unwrap_or("");
    let revision = source["revision"].as_u64().unwrap_or(0);
    let suffix = if p["predecessor"].is_null() {
        uuid.replace('-', "").chars().take(8).collect::<String>()
    } else {
        publication::sha256(format!("{uuid}#{revision}").as_bytes())[..8].into()
    };
    let created = codec::date(metadata["created"].as_str().unwrap_or(""))?;
    let frozen =
        chrono::DateTime::parse_from_rfc3339(proposal["updated_at"].as_str().unwrap_or(""))
            .map_err(|_| {
                RuntimeError::invalid("Publication freeze requires an offset timestamp")
            })?;
    chrono::DateTime::parse_from_rfc3339(p["original_updated_at"].as_str().unwrap_or(""))
        .map_err(|_| RuntimeError::invalid("Publication origin requires an offset timestamp"))?;
    let id = format!("MEM-{}-{suffix}", created.format("%Y%m%d"));
    if metadata["id"] != id
        || created != frozen.with_timezone(&chrono::Utc).date_naive()
        || destination != format!("memory-bank/chunks/{id}-{}.md", codec::slug(title)?)
        || metadata["title"] != proposal["title"]
        || metadata["status"] != "active"
        || body.trim() != expected.trim()
        || metadata["last_verified"] != metadata["created"]
        || metadata["valid_from"] != metadata["created"]
        || metadata["review_after"].as_str()
            != Some((created + chrono::Duration::days(365)).to_string().as_str())
        || !metadata["valid_to"].is_null()
        || !metadata["superseded_by"].is_null()
    {
        return Err(RuntimeError::invalid(
            "Publication destination identity, dates, or content differ",
        ));
    }
    let predecessor = if p["predecessor"].is_null() {
        if metadata["supersedes"] != object!([]) {
            return Err(RuntimeError::invalid("Publication predecessor is missing"));
        }
        None
    } else {
        if proposal["review_mode"] != "automatic"
            || proposal["source_records"].as_array().map(Vec::len) != Some(1)
        {
            return Err(RuntimeError::invalid(
                "Only a single-source automatic revision may supersede",
            ));
        }
        let previous = &p["predecessor"];
        strict(previous, &["path", "before", "after"])?;
        let name = previous["path"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Invalid predecessor path"))?;
        if Path::new(name).parent() != Some(Path::new("memory-bank/chunks"))
            || name == destination
            || codec::filename_id(
                Path::new(name)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(""),
            )?
            .is_none()
        {
            return Err(RuntimeError::policy("Invalid predecessor path"));
        }
        let before = publication::unblob(&previous["before"])?;
        let after = publication::unblob(&previous["after"])?;
        let (mut expected, old_body) = codec::parse_raw(&before)?;
        let old_id = expected["id"].clone();
        if expected["status"] != "active" {
            return Err(RuntimeError::invalid("Predecessor was not active"));
        }
        expected["status"] = "superseded".into();
        expected["superseded_by"] = id.clone().into();
        expected["valid_to"] = metadata["valid_from"].clone();
        let (actual, new_body) = codec::parse_raw(&after)?;
        if actual != expected
            || old_body.trim() != new_body.trim()
            || metadata["supersedes"] != object!([old_id])
        {
            return Err(RuntimeError::invalid(
                "Publication changes unrelated predecessor knowledge",
            ));
        }
        Some((name.into(), before, after))
    };
    let sources = p["sources"]
        .as_array()
        .filter(|rows| !rows.is_empty() && rows.len() <= 100)
        .ok_or_else(|| RuntimeError::invalid("Invalid publication source images"))?
        .clone();
    let expected_sources = proposal["source_records"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row["id"].as_str()?, row["path"].as_str()?)))
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for row in &sources {
        strict(row, &["id", "path", "sha256"])?;
        let id = row["id"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Invalid source ID"))?;
        let path = row["path"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Invalid source path"))?;
        if !expected_sources.contains(&(id, path))
            || !seen.insert(id)
            || !hash(row["sha256"].as_str().unwrap_or(""))
        {
            return Err(RuntimeError::invalid(
                "Publication source image binding differs",
            ));
        }
    }
    if sources.len() != expected_sources.len() {
        return Err(RuntimeError::invalid("Incomplete source custody"));
    }
    Ok(Plan {
        destination,
        after,
        predecessor,
        sources,
        metadata,
    })
}
fn stored(files: &mut PublicationFiles, id: &str, proposal: &Value) -> Result<Vec<u8>> {
    let raw = files
        .read(&path(id)?, 2 * PLAN_LIMIT)?
        .ok_or_else(|| RuntimeError::unavailable("Publication checkpoint unavailable"))?;
    if raw_json(&raw)? != *proposal {
        return Err(RuntimeError::conflict("Publication checkpoint changed"));
    }
    Ok(raw)
}
type PromotionImageStates = (Option<Vec<u8>>, Option<Vec<u8>>);
fn states(files: &mut PublicationFiles, p: &Plan) -> Result<PromotionImageStates> {
    let destination = files.read(&p.destination, IMAGE_LIMIT)?;
    if destination.as_deref().is_some_and(|raw| raw != p.after) {
        return Err(RuntimeError::conflict(
            "Publication destination externally edited",
        ));
    }
    let previous = match &p.predecessor {
        Some((name, before, after)) => {
            let raw = files.read(name, IMAGE_LIMIT)?;
            if raw.as_deref() != Some(before.as_slice()) && raw.as_deref() != Some(after.as_slice())
            {
                return Err(RuntimeError::conflict(
                    "Publication predecessor externally edited",
                ));
            }
            raw
        }
        None => None,
    };
    files.read("memory-bank/INDEX.md", PLAN_LIMIT)?;
    Ok((destination, previous))
}
fn rollback(service: &BankService, proposal: &Value) -> Result<Value> {
    let p = plan(proposal)?;
    let id = proposal["id"].as_str().unwrap_or("");
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let mut raw = stored(&mut files, id, proposal)?;
    let (current, previous) = states(&mut files, &p)?;
    let mut proposal = proposal.clone();
    if proposal["publication"]["phase"] != "rollback" {
        proposal["publication"]["phase"] = "rollback".into();
        publication::seal(&mut proposal["publication"])?;
        raw = write(&mut files, id, &proposal, Some(&raw))?;
    }
    if let Some((name, before, after)) = &p.predecessor
        && previous.as_deref() != Some(before.as_slice())
    {
        files.write_expected(name, before, Some(after))?;
    }
    if current.is_some() {
        states(&mut files, &p)?;
        files.unlink_expected(&p.destination, &p.after)?;
    }
    let rendered = codec::render_index(service)?.0;
    let index = files.read("memory-bank/INDEX.md", PLAN_LIMIT)?;
    if index.as_deref() != Some(rendered.as_bytes()) {
        files.write_expected(
            "memory-bank/INDEX.md",
            rendered.as_bytes(),
            index.as_deref(),
        )?;
    }
    let (current, previous) = states(&mut files, &p)?;
    if current.is_some()
        || p.predecessor
            .as_ref()
            .is_some_and(|(_, before, _)| previous.as_deref() != Some(before.as_slice()))
    {
        return Err(RuntimeError::conflict("Publication rollback is incomplete"));
    }
    let mut original = proposal.clone();
    original["updated_at"] = proposal["publication"]["original_updated_at"].clone();
    original
        .as_object_mut()
        .ok_or_else(|| RuntimeError::invalid("Invalid proposal"))?
        .remove("publication");
    stored(&mut files, id, &proposal)?;
    write(&mut files, id, &original, Some(&raw))?;
    Ok(original)
}

fn resume(service: &BankService, proposal: &Value) -> Result<Value> {
    let p = plan(proposal)?;
    let id = proposal["id"].as_str().unwrap_or("");
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let raw = stored(&mut files, id, proposal)?;
    let (current, previous) = states(&mut files, &p)?;
    for source in &p.sources {
        let bytes = files
            .read(source["path"].as_str().unwrap_or(""), 4 * IMAGE_LIMIT)?
            .ok_or_else(|| RuntimeError::unavailable("Publication source is missing"))?;
        if publication::sha256(&bytes) != source["sha256"].as_str().unwrap_or("") {
            return Err(RuntimeError::conflict(
                "Same-revision publication source image was edited; custody retained",
            ));
        }
    }
    let source_state = source_images(service, proposal, &mut files);
    let (inherited, digests, agent) = match source_state {
        Ok(state) => state,
        Err(error) => {
            rollback(service, proposal)?;
            return Err(error);
        }
    };
    let mut tags = vec!["project-brain", "promoted"];
    if proposal["review_mode"] == "automatic" {
        tags.push("auto-promoted");
    }
    if agent {
        tags.push("agent-attested");
    }
    let paths = inherited
        .iter()
        .map(|source| source.split('#').next().unwrap_or(source))
        .collect::<BTreeSet<_>>();
    let expected = paths
        .iter()
        .map(|path| {
            digests
                .get(*path)
                .map(|hash| object!({"path":path,"sha256":hash}))
                .ok_or_else(|| RuntimeError::invalid("Publication source image is undigested"))
        })
        .collect::<Result<Vec<_>>>()?;
    let kind = match proposal["source_records"][0]["type"].as_str() {
        Some("finding") => "domain",
        Some("bug") => "constraint",
        Some("incident") => "operations",
        _ => "decision",
    };
    let mut actual = p.metadata["source_digests"]
        .as_array()
        .cloned()
        .ok_or_else(|| RuntimeError::invalid("Invalid publication source digests"))?;
    actual.sort_by_key(|row| row["path"].as_str().unwrap_or("").to_string());
    if p.metadata["sources"] != object!(inherited)
        || actual != expected
        || p.metadata["tags"] != object!(tags)
        || p.metadata["scope"] != object!(["application"])
        || p.metadata["type"] != kind
    {
        return Err(RuntimeError::policy(
            "Publication provenance differs from checked records; no writes started",
        ));
    }
    if proposal["publication"]["phase"] == "rollback" {
        rollback(service, proposal)?;
        return apply(service, id);
    }
    let automatic = proposal["review_mode"] == "automatic";
    let ledger = ledger(service)?;
    let mut admission = None;
    if automatic {
        for source in proposal["source_records"].as_array().into_iter().flatten() {
            let source_id = source["id"].as_str().unwrap_or("");
            if ledger.held.contains(source_id)
                || ledger.latest.get(source_id).is_some_and(|winner| {
                    winner["id"] != proposal["id"]
                        && winner["source_records"][0]["revision"]
                            .as_u64()
                            .unwrap_or(0)
                            >= source["revision"].as_u64().unwrap_or(0)
                })
            {
                admission = Some(RuntimeError::policy(
                    "Publication source is held or already promoted",
                ));
            }
        }
        let own = std::iter::once(p.metadata["id"].as_str().unwrap_or("").to_string())
            .chain(
                p.predecessor
                    .as_ref()
                    .and_then(|(_, before, _)| codec::parse_raw(before).ok())
                    .and_then(|(metadata, _)| metadata["id"].as_str().map(String::from)),
            )
            .collect::<BTreeSet<_>>();
        let others = maintenance::active_texts(service)?
            .into_iter()
            .filter(|(id, _)| !own.contains(id))
            .collect::<Vec<_>>();
        if maintenance::near_duplicate(
            &format!(
                "{}\n{}",
                proposal["title"].as_str().unwrap_or(""),
                proposal["content"].as_str().unwrap_or("")
            ),
            &others,
            maintenance::duplicate_ratio(&service.config()?.value),
        )
        .is_some()
        {
            admission = Some(RuntimeError::policy("Publication is a near-duplicate"));
        }
    }
    if let Some(error) = admission {
        rollback(service, proposal)?;
        return Err(error);
    }
    let before_errors = validation::validate_bank(service)?;
    let mut applied = proposal.clone();
    applied
        .as_object_mut()
        .ok_or_else(|| RuntimeError::invalid("Invalid proposal"))?
        .remove("publication");
    applied["status"] = "applied".into();
    applied["destination_memory_id"] = p.metadata["id"].clone();
    applied["destination_revision"] = 1.into();
    applied["updated_at"] = domain::utc_now().into();
    applied["revision"] = (proposal["revision"].as_u64().unwrap_or(0) + 1).into();
    let result = (|| -> Result<()> {
        if current.is_none() {
            files.write_expected(&p.destination, &p.after, None)?;
        }
        if let Some((name, before, after)) = &p.predecessor
            && previous.as_deref() == Some(before.as_slice())
        {
            files.write_expected(name, after, Some(before))?;
        }
        let rendered = codec::render_index(service)?.0;
        let old = files.read("memory-bank/INDEX.md", PLAN_LIMIT)?;
        if old.as_deref() != Some(rendered.as_bytes()) {
            files.write_expected("memory-bank/INDEX.md", rendered.as_bytes(), old.as_deref())?;
        }
        let errors = validation::write_errors(
            service,
            &before_errors,
            std::slice::from_ref(&p.destination),
        )?;
        if !errors.is_empty() {
            return Err(RuntimeError::invalid(format!(
                "Promoted Bank publication failed validation: {}",
                errors.join("; ")
            )));
        }
        validate(service, &applied, Some(id))?;
        states(&mut files, &p)?;
        stored(&mut files, id, proposal)?;
        write(&mut files, id, &applied, Some(&raw))?;
        Ok(())
    })();
    if let Err(error) = result {
        let current = files
            .read(&path(id)?, 2 * PLAN_LIMIT)?
            .map(|raw| raw_json(&raw))
            .transpose()?;
        if current.as_ref() != Some(&applied) {
            rollback(service, proposal)?;
        }
        return Err(error);
    }
    Ok(applied)
}

pub fn apply(service: &BankService, id: &str) -> Result<Value> {
    let _guard = service.brain()?.lock()?;
    list(service, true)?;
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let raw = files
        .read(&path(id)?, 2 * PLAN_LIMIT)?
        .ok_or_else(|| RuntimeError::not_found("Promotion not found"))?;
    let mut proposal = raw_json(&raw)?;
    validate(service, &proposal, Some(id))?;
    if proposal["status"] != "reviewed" {
        return Err(RuntimeError::policy(
            "Promotion requires an approved review before apply",
        ));
    }
    if proposal.get("publication").is_some() {
        return resume(service, &proposal);
    }
    let (inherited, digests, agent) = source_images(service, &proposal, &mut files)?;
    let source = &proposal["source_records"][0];
    let source_id = source["id"].as_str().unwrap_or("");
    let automatic = proposal["review_mode"] == "automatic";
    let mut predecessor = None;
    let mut suffix = source_id
        .replace('-', "")
        .chars()
        .take(8)
        .collect::<String>();
    if automatic
        && proposal["source_records"].as_array().map(Vec::len) == Some(1)
        && let Some(previous) = ledger(service)?.latest.get(source_id)
        && previous["id"] != proposal["id"]
    {
        if previous["source_records"][0]["revision"]
            .as_u64()
            .unwrap_or(0)
            >= source["revision"].as_u64().unwrap_or(0)
        {
            return Err(RuntimeError::policy(
                "Promotion source was already promoted",
            ));
        }
        let chunk = codec::find(
            service,
            previous["destination_memory_id"].as_str().unwrap_or(""),
            &mut files,
        )?;
        let heading = format!("# {}", previous["title"].as_str().unwrap_or(""));
        let old_content = previous["content"].as_str().unwrap_or("").trim();
        let expected = if old_content.starts_with(&heading) {
            old_content.into()
        } else {
            format!("{heading}\n\n{old_content}")
        };
        if chunk.metadata["status"] != "active"
            || chunk.metadata["title"] != previous["title"]
            || chunk.body.trim() != expected.trim()
        {
            return Err(RuntimeError::policy(
                "Previously promoted chunk was edited or retired; review replacement explicitly",
            ));
        }
        suffix = publication::sha256(
            format!("{source_id}#{}", source["revision"].as_u64().unwrap_or(0)).as_bytes(),
        )[..8]
            .into();
        predecessor = Some(chunk);
    }
    let instant = domain::clock::now();
    let frozen = instant.to_rfc3339_opts(chrono::SecondsFormat::Micros, false);
    let today = instant.date_naive();
    let memory_id = format!("MEM-{}-{suffix}", today.format("%Y%m%d"));
    let title = proposal["title"].as_str().unwrap_or("");
    let destination = format!("memory-bank/chunks/{memory_id}-{}.md", codec::slug(title)?);
    if files.read(&destination, IMAGE_LIMIT)?.is_some() {
        return Err(RuntimeError::conflict(
            "Promotion destination already exists",
        ));
    }
    if automatic {
        let others = maintenance::active_texts(service)?
            .into_iter()
            .filter(|(id, _)| {
                predecessor
                    .as_ref()
                    .is_none_or(|chunk| chunk.metadata["id"].as_str() != Some(id))
            })
            .collect::<Vec<_>>();
        if let Some(duplicate) = maintenance::near_duplicate(
            &format!("{title}\n{}", proposal["content"].as_str().unwrap_or("")),
            &others,
            maintenance::duplicate_ratio(&service.config()?.value),
        ) {
            return Err(RuntimeError::policy(format!(
                "Near-duplicate of {duplicate}; merge or supersede first"
            )));
        }
    }
    let mut tags = vec!["project-brain", "promoted"];
    if automatic {
        tags.push("auto-promoted");
    }
    if agent {
        tags.push("agent-attested");
    }
    let paths = inherited
        .iter()
        .map(|source| source.split('#').next().unwrap_or(source))
        .collect::<BTreeSet<_>>();
    let source_digests = paths
        .into_iter()
        .map(|path| {
            digests
                .get(path)
                .map(|hash| object!({"path":path,"sha256":hash}))
                .ok_or_else(|| RuntimeError::invalid("Promotion source is undigested"))
        })
        .collect::<Result<Vec<_>>>()?;
    let kind = match source["type"].as_str() {
        Some("finding") => "domain",
        Some("bug") => "constraint",
        Some("incident") => "operations",
        _ => "decision",
    };
    let metadata = object!({"id":memory_id,"title":title,"type":kind,"status":"active","scope":["application"],"tags":tags,"created":today.to_string(),"last_verified":today.to_string(),"review_after":(today+chrono::Duration::days(365)).to_string(),"sources":inherited,"supersedes":predecessor.as_ref().map(|chunk|vec![chunk.metadata["id"].clone()]).unwrap_or_default(),"superseded_by":null,"valid_from":today.to_string(),"valid_to":null,"source_digests":source_digests});
    let heading = format!("# {title}");
    let content = proposal["content"].as_str().unwrap_or("").trim();
    let body = if content.starts_with(&heading) {
        content.to_string()
    } else {
        format!("{heading}\n\n{content}")
    };
    let chunk = domain::render_markdown_record(&metadata, &body)?.into_bytes();
    let original_updated = proposal["updated_at"].clone();
    proposal["updated_at"] = frozen.into();
    let mut plan = object!({"profile":"promotion-publication-v1","phase":"publish","original_updated_at":original_updated,"proposal_sha256":publication::digest(&proposal)?,"destination":{"path":destination,"after":publication::blob(&chunk)?},"predecessor":null,"sources":[]});
    for source in proposal["source_records"].as_array().into_iter().flatten() {
        let path = source["path"].as_str().unwrap_or("");
        let raw = files
            .read(path, 4 * IMAGE_LIMIT)?
            .ok_or_else(|| RuntimeError::unavailable("Promotion source image missing"))?;
        plan["sources"]
            .as_array_mut()
            .ok_or_else(|| RuntimeError::invalid("Invalid publication sources"))?
            .push(object!({"id":source["id"],"path":path,"sha256":publication::sha256(&raw)}));
    }
    if let Some(chunk) = predecessor {
        let mut after = chunk.metadata.clone();
        after["status"] = "superseded".into();
        after["superseded_by"] = memory_id.into();
        after["valid_to"] = today.to_string().into();
        plan["predecessor"] = object!({"path":chunk.path,"before":publication::blob(&chunk.raw)?,"after":publication::blob(&codec::render_chunk(&after,&chunk.body)?)?});
    }
    publication::seal(&mut plan)?;
    proposal["publication"] = plan;
    validate(service, &proposal, Some(id))?;
    self::plan(&proposal)?;
    write(&mut files, id, &proposal, Some(&raw))?;
    resume(service, &proposal)
}

fn canonical_image(rows: &Value) -> Result<BTreeMap<String, String>> {
    let mut image = BTreeMap::new();
    for row in rows
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("Invalid checked source image: expected a list"))?
    {
        strict(row, &["path", "sha256"])?;
        let path = row["path"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Invalid or changed checked source image"))?;
        let digest = row["sha256"]
            .as_str()
            .filter(|v| hash(v))
            .ok_or_else(|| RuntimeError::invalid("Invalid or changed checked source image"))?;
        if image.get(path).is_some_and(|previous| previous != digest) {
            return Err(RuntimeError::invalid(
                "Invalid or changed checked source image",
            ));
        }
        image.insert(path.to_owned(), digest.to_owned());
    }
    Ok(image)
}
fn checked_again(service: &BankService, record: &Value, previous: &Value) -> Result<bool> {
    let verified = record["transitions"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|row| row["from"] == "observed" && row["to"] == "verified");
    if !verified
        || !["agent", "person"].contains(&domain::record_attestation(record))
        || !domain::sources_are_fresh(&service.context.roots, record)?
    {
        return Ok(false);
    }
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let chunk = codec::find(
        service,
        previous["destination_memory_id"].as_str().unwrap_or(""),
        &mut files,
    )
    .map_err(|_| {
        RuntimeError::policy(
            "Previous promoted source image is unavailable; review its replacement explicitly",
        )
    })?;
    let images = (|| -> Result<_> {
        let checked = canonical_image(&record["source_fingerprints"])?;
        let records = previous["source_records"]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("Invalid source records"))?
            .iter()
            .map(|row| {
                row["path"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| RuntimeError::invalid("Invalid record source path"))
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let all = canonical_image(chunk.metadata.get("source_digests").unwrap_or(&object!([])))?;
        let old = all
            .iter()
            .filter(|(path, _)| !records.contains(*path))
            .map(|(p, h)| (p.clone(), h.clone()))
            .collect::<BTreeMap<_, _>>();
        Ok((checked, records, all, old))
    })()
    .map_err(|_| {
        RuntimeError::policy(
            "Previous promoted source image is invalid; review its replacement explicitly",
        )
    })?;
    let (checked, record_paths, all, old) = images;
    let expected = codec::strings(record, "sources", true)?
        .iter()
        .map(|s| s.split('#').next().unwrap_or(s).to_owned())
        .collect::<BTreeSet<_>>();
    if checked.is_empty()
        || checked.keys().cloned().collect::<BTreeSet<_>>() != expected
        || old.keys().cloned().collect::<BTreeSet<_>>() != expected
    {
        return Ok(false);
    }
    if checked != old {
        return Ok(true);
    }
    let previous_checks = previous["source_verifications"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row["id"].as_str()?, row["sha256"].as_str()?)))
        .collect::<BTreeMap<_, _>>();
    let Some(previous_check) = previous_checks.get(record["id"].as_str().unwrap_or("")) else {
        return Ok(false);
    };
    if verification_image(record)?.as_str() == *previous_check {
        return Ok(false);
    }
    let old_records = all
        .into_iter()
        .filter(|(path, _)| record_paths.contains(path))
        .collect::<BTreeMap<_, _>>();
    let current = domain::source_fingerprints(
        &service.context.roots,
        &record_paths.iter().cloned().collect::<Vec<_>>(),
    )?;
    Ok(
        old_records.keys().cloned().collect::<BTreeSet<_>>() == record_paths
            && old_records != canonical_image(&object!(current))?,
    )
}
fn promoted_source_ids(service: &BankService) -> Result<BTreeSet<String>> {
    let mut ids = BTreeSet::new();
    for proposal in list(service, false)? {
        if proposal["status"] == "rejected"
            || (proposal["status"] != "applied" && proposal["review_mode"] == "automatic")
        {
            continue;
        }
        for row in proposal["source_records"].as_array().into_iter().flatten() {
            if let Some(id) = row["id"].as_str() {
                ids.insert(id.to_owned());
            }
        }
    }
    Ok(ids)
}
fn stalled(service: &BankService) -> Result<Vec<Value>> {
    Ok(list(service, false)?
        .into_iter()
        .filter(|p| {
            p["review_mode"] == "automatic"
                && ["proposed", "reviewed"].contains(&p["status"].as_str().unwrap_or(""))
        })
        .collect())
}
fn promotable(
    service: &BankService,
    config: &crate::RuntimeConfig,
    allow_changed_sources: bool,
    only: Option<&BTreeSet<String>>,
) -> Result<(Vec<Value>, Vec<Value>)> {
    let already = promoted_source_ids(service)?;
    let ledger = ledger(service)?;
    let mut candidates = Vec::new();
    let mut blocked = Vec::new();
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    for image in service.brain()?.records(true)? {
        let record = image.record;
        let id = record["id"].as_str().unwrap_or("").to_owned();
        if only.is_some_and(|ids| !ids.contains(&id)) {
            continue;
        }
        let state = match record["type"].as_str() {
            Some("finding" | "bug") => "resolved",
            Some("incident") => "closed",
            Some("decision") => "accepted",
            _ => continue,
        };
        if record["status"] != state {
            continue;
        }
        let body = content(&record);
        let mut supersedes = Value::Null;
        if already.contains(&id) {
            let previous = ledger.latest.get(&id);
            let mut rechecked = false;
            if let Some(previous) = previous
                && body.as_deref() == previous["content"].as_str()
            {
                match checked_again(service, &record, previous) {
                    Ok(value) => rechecked = value,
                    Err(error) => {
                        blocked.push(object!({"record_id":id,"reason":error.to_string()}));
                        continue;
                    }
                }
            }
            if ledger.held.contains(&id)
                || previous.is_none()
                || body.is_none()
                || (body.as_deref() == previous.and_then(|p| p["content"].as_str()) && !rechecked)
                || record["revision"].as_u64().unwrap_or(0)
                    <= previous
                        .map(|p| p["source_records"][0]["revision"].as_u64().unwrap_or(0))
                        .unwrap_or(0)
            {
                continue;
            }
            let previous = previous.unwrap();
            let memory = previous["destination_memory_id"].as_str();
            let label = memory.map(str::to_owned).unwrap_or_else(|| "None".into());
            if previous["review_mode"] != "automatic" {
                blocked.push(object!({"record_id":id,"reason":format!("revised since {label} was promoted after a person's review; propose the revision for review")}));
                continue;
            }
            let status = memory
                .and_then(|memory| codec::find(service, memory, &mut files).ok())
                .and_then(|c| c.metadata["status"].as_str().map(str::to_owned));
            if status.as_deref() != Some("active") {
                blocked.push(object!({"record_id":id,"reason":format!("revised since it was promoted, and {label} is {}; promote the revision by hand if it still holds",status.as_deref().unwrap_or("missing"))}));
                continue;
            }
            supersedes = previous["destination_memory_id"].clone();
        }
        let provenance = provenance_problems(&record, body.as_deref())?;
        if !provenance.is_empty() {
            blocked.push(object!({"record_id":id,"reason":provenance.join("; ")}));
            continue;
        }
        if let Some(reason) =
            eligibility_with_config(service, &record, allow_changed_sources, config)?
        {
            blocked.push(object!({"record_id":id,"reason":reason}));
            continue;
        }
        candidates.push(object!({"record":record,"content":body,"supersedes":supersedes}));
    }
    Ok((candidates, blocked))
}
/// Read-only health admission: canonical promotable_records with current sources.
pub fn status_candidates(service: &BankService) -> Result<(Vec<Value>, Vec<Value>)> {
    let config = service.config()?;
    promotable(service, &config, false, None)
}
fn obsolete(
    service: &BankService,
    proposal: &Value,
    applied: &BTreeMap<String, Value>,
) -> Result<Option<String>> {
    let source = &proposal["source_records"][0];
    let id = source["id"].as_str().unwrap_or("");
    if ledger(service)?.held.contains(id) {
        return Ok(Some(
            "its record is held by a human review or synthesis".into(),
        ));
    }
    if let Some(winner) = applied.get(id)
        && winner["id"] != proposal["id"]
        && winner["source_records"][0]["revision"]
            .as_u64()
            .unwrap_or(0)
            >= source["revision"].as_u64().unwrap_or(0)
    {
        return Ok(Some(format!(
            "already promoted as {}",
            winner["destination_memory_id"].as_str().unwrap_or("None")
        )));
    }
    match service.brain()?.find(id, None, true) {
        Err(error) if error.kind == ErrorKind::NotFound => {
            Ok(Some("its record no longer exists".into()))
        }
        Err(_) => Ok(None),
        Ok(image) => {
            if image.record["revision"] != source["revision"] {
                return Ok(Some(format!(
                    "its record moved on to revision {}",
                    image.record["revision"]
                )));
            }
            let current = relative(service, &image.path)?;
            if source["path"].as_str() != Some(&current) {
                return Ok(Some(format!("its record moved to {current}")));
            }
            Ok(None)
        }
    }
}
fn withdraw(service: &BankService, id: &str, reason: &str) -> Result<Value> {
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let raw = files
        .read(&path(id)?, 2 * PLAN_LIMIT)?
        .ok_or_else(|| RuntimeError::not_found("Promotion not found"))?;
    let mut proposal = raw_json(&raw)?;
    validate(service, &proposal, Some(id))?;
    if proposal["review_mode"] != "automatic"
        || !["proposed", "reviewed"].contains(&proposal["status"].as_str().unwrap_or(""))
    {
        return Err(RuntimeError::policy(
            "Only an unapplied automatic promotion is withdrawn",
        ));
    }
    if proposal.get("publication").is_some() {
        proposal = rollback(service, &proposal)?;
    }
    let expected = stored(&mut files, id, &proposal)?;
    proposal["status"] = "rejected".into();
    proposal["outcome"] = "rejected".into();
    proposal["updated_at"] = domain::utc_now().into();
    proposal["revision"] = (proposal["revision"].as_u64().unwrap_or(0) + 1).into();
    let mut conflicts = codec::strings(&proposal, "conflicts", true)?;
    conflicts.push(format!("withdrawn: {reason}"));
    proposal["conflicts"] = object!(domain::unique_strings(conflicts));
    validate(service, &proposal, Some(id))?;
    write(&mut files, id, &proposal, Some(&expected))?;
    Ok(proposal)
}
fn retry_path(service: &BankService) -> Result<String> {
    let directory = service.context.roots.state.join("project-brain/local");
    for parent in [
        directory.clone(),
        service.context.roots.state.join("project-brain"),
    ] {
        match std::fs::symlink_metadata(&parent) {
            Ok(image) if !image.is_dir() || crate::fs::linked_at(&parent, &image)? => {
                return Err(RuntimeError::unavailable(
                    "Automatic promotion retry state is unavailable",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(RuntimeError::unavailable(
                    "Automatic promotion retry state is unavailable",
                ));
            }
        }
    }
    let key = format!(
        "project-brain/local/promotion-retry-{}.json",
        publication::sha256(service.context.actor.as_bytes())
    );
    let destination = service.context.roots.state.join(&key);
    match std::fs::symlink_metadata(&destination) {
        Ok(image) => {
            if !image.is_file()
                || crate::fs::linked_at(&destination, &image)?
                || crate::fs::native_image(&destination)?.is_none_or(|image| image.links != 1)
            {
                return Err(RuntimeError::unavailable(
                    "Automatic promotion retry state is unavailable",
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(RuntimeError::unavailable(
                "Automatic promotion retry state is unavailable",
            ));
        }
    }
    Ok(key)
}
fn retry_state(files: &mut PublicationFiles, path: &str) -> Option<Value> {
    let raw = files.read(path, 512).ok()??;
    let value = raw_json(&raw).ok()?;
    let map = value.as_object()?;
    if !((map.len() == 2 && map.contains_key("profile") && map.contains_key("next_attempt"))
        || (map.len() == 3
            && map.contains_key("profile")
            && map.contains_key("next_attempt")
            && map.contains_key("after_record")))
        || value["profile"] != "automatic-promotion-retry-v1"
        || value["next_attempt"].as_u64().is_none()
        || !(value["after_record"].is_null()
            || value["after_record"].as_str().is_some_and(domain::is_uuid4))
    {
        return None;
    }
    Some(value)
}
fn save_retry(
    service: &BankService,
    _files: &mut PublicationFiles,
    path: &str,
    value: &Value,
) -> Result<()> {
    if retry_path(service)? != path {
        return Err(RuntimeError::invalid(
            "Automatic promotion retry identity changed",
        ));
    }
    crate::fs::ensure_directory(
        &service.context.roots.state.join("project-brain/local"),
        true,
    )?;
    crate::fs::atomic_json(&service.context.roots.state.join(path), value)
}
fn empty_auto() -> Value {
    object!({"enabled":false,"promoted":[],"failed":[],"blocked":[],"closed":[],"skipped":0})
}
pub fn auto_promote(
    service: &BankService,
    limit: usize,
    max_attempts: Option<usize>,
    only: Option<&[String]>,
) -> Result<Value> {
    let config = service.config()?;
    if !config.enabled("automatic_promotion") {
        return Ok(empty_auto());
    }
    if max_attempts == Some(0) {
        return Err(RuntimeError::invalid(
            "Automatic promotion attempt limit must be positive",
        ));
    }
    let _guard = service.brain()?.lock()?;
    list(service, true)?;
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let marker = retry_path(service).ok();
    let previous = marker
        .as_ref()
        .and_then(|path| retry_state(&mut files, path))
        .unwrap_or(object!({}));
    let mut retry = object!({"profile":"automatic-promotion-retry-v1","next_attempt":domain::clock::proof_now().timestamp().saturating_add(30),"after_record":previous.get("after_record").cloned().unwrap_or(Value::Null)});
    let mut retry_file =
        marker.filter(|path| save_retry(service, &mut files, path, &retry).is_ok());
    if max_attempts.is_some() && retry_file.is_none() {
        let mut result = empty_auto();
        result["retry_unavailable"] = true.into();
        return Ok(result);
    }
    let mut attempts = 0usize;
    let mut deferred_stalled = 0usize;
    let mut applied = ledger(service)?.latest;
    let mut work_only = only.map(|ids| ids.iter().cloned().collect::<BTreeSet<_>>());
    let mut deferred_records = 0usize;
    let mut retry_blocked = Vec::new();
    let threshold = maintenance::duplicate_ratio(&config.value);
    if let Some(maximum) = max_attempts {
        let (fresh, mut initial_blocked) = promotable(service, &config, true, work_only.as_ref())?;
        let bank = maintenance::active_texts(service)?;
        let mut identities = BTreeSet::new();
        for candidate in fresh {
            let record = &candidate["record"];
            let replaces = candidate["supersedes"].as_str();
            let others = bank
                .iter()
                .filter(|(id, _)| Some(id.as_str()) != replaces)
                .cloned()
                .collect::<Vec<_>>();
            if let Some(duplicate) = maintenance::near_duplicate(
                &format!(
                    "{}\n{}",
                    record["title"].as_str().unwrap_or(""),
                    candidate["content"].as_str().unwrap_or("")
                ),
                &others,
                threshold,
            ) {
                initial_blocked.push(object!({"record_id":record["id"],"reason":format!("near-duplicate of {duplicate}; merge or supersede first")}));
            } else if let Some(id) = record["id"].as_str() {
                identities.insert(id.to_owned());
            }
        }
        retry_blocked = initial_blocked;
        for proposal in stalled(service)? {
            let id = proposal["source_records"][0]["id"].as_str().unwrap_or("");
            if work_only.as_ref().is_none_or(|ids| ids.contains(id)) {
                identities.insert(id.to_owned());
            }
        }
        let after = previous["after_record"].as_str().unwrap_or("");
        let ordered = identities
            .iter()
            .filter(|id| id.as_str() > after)
            .chain(identities.iter().filter(|id| id.as_str() <= after))
            .cloned()
            .collect::<Vec<_>>();
        let selected = ordered
            .iter()
            .take(maximum)
            .cloned()
            .collect::<BTreeSet<_>>();
        deferred_records = identities.len() - selected.len();
        if !selected.is_empty()
            && let Some(path) = &retry_file
        {
            retry["after_record"] = ordered[selected.len() - 1].clone().into();
            if save_retry(service, &mut files, path, &retry).is_err() {
                let mut result = empty_auto();
                result["retry_unavailable"] = true.into();
                return Ok(result);
            }
        }
        work_only = Some(selected);
    }
    let mut promoted = Vec::new();
    let mut failed = Vec::new();
    let mut closed = Vec::new();
    for proposal in stalled(service)? {
        let source = &proposal["source_records"][0];
        let source_id = source["id"].as_str().unwrap_or("");
        if work_only
            .as_ref()
            .is_some_and(|ids| !ids.contains(source_id))
        {
            continue;
        }
        if max_attempts.is_some_and(|max| attempts >= max) {
            deferred_stalled += 1;
            continue;
        }
        attempts += 1;
        let id = proposal["id"].as_str().unwrap_or("");
        let effect = (|| -> Result<Option<Value>> {
            if let Some(reason) = obsolete(service, &proposal, &applied)? {
                withdraw(service, id, &reason)?;
                closed.push(object!({"record_id":source_id,"promotion_id":id,"reason":reason}));
                return Ok(None);
            }
            if proposal["status"] == "proposed" {
                review(service, id, None, true)?;
            }
            Ok(Some(apply(service, id)?))
        })();
        match effect {
            Err(error) => failed.push(object!({"record_id":source_id,"reason":error.to_string()})),
            Ok(None) => {}
            Ok(Some(proposal)) => {
                applied.insert(source_id.to_owned(), proposal.clone());
                promoted.push(object!({"record_id":source_id,"type":source["type"],"memory_id":proposal["destination_memory_id"]}));
            }
        }
    }
    let pending = stalled(service)?
        .iter()
        .filter_map(|p| p["source_records"][0]["id"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let (candidates, blocked) = promotable(service, &config, true, work_only.as_ref())?;
    let mut unique = BTreeMap::<(String, String), Value>::new();
    let mut order = Vec::new();
    for row in retry_blocked.into_iter().chain(blocked) {
        let key = (
            row["record_id"].as_str().unwrap_or("").to_owned(),
            row["reason"].as_str().unwrap_or("").to_owned(),
        );
        if !unique.contains_key(&key) {
            order.push(key.clone());
        }
        unique.insert(key, row);
    }
    let mut blocked = order
        .into_iter()
        .map(|key| unique.remove(&key).unwrap())
        .collect::<Vec<_>>();
    let candidates = candidates
        .into_iter()
        .filter(|c| !pending.contains(c["record"]["id"].as_str().unwrap_or("")))
        .collect::<Vec<_>>();
    let mut existing = maintenance::active_texts(service)?;
    let mut admission = limit.min(candidates.len());
    if let Some(max) = max_attempts {
        admission = admission.min(max.saturating_sub(attempts))
    }
    let mut admitted = 0usize;
    let mut considered = 0usize;
    for candidate in &candidates {
        if admitted >= admission {
            break;
        }
        considered += 1;
        let record = &candidate["record"];
        let id = record["id"].as_str().unwrap_or("");
        let replaces = candidate["supersedes"].as_str();
        let others = existing
            .iter()
            .filter(|(memory, _)| Some(memory.as_str()) != replaces)
            .cloned()
            .collect::<Vec<_>>();
        let body = candidate["content"].as_str().unwrap_or("");
        let comparable = format!("{}\n{body}", record["title"].as_str().unwrap_or(""));
        if let Some(duplicate) = maintenance::near_duplicate(&comparable, &others, threshold) {
            blocked.push(object!({"record_id":id,"reason":format!("near-duplicate of {duplicate}; merge or supersede first")}));
            continue;
        }
        admitted += 1;
        let effect = (|| -> Result<Value> {
            let proposal = propose(
                service,
                &[id.to_owned()],
                record["title"].as_str().unwrap_or(""),
                body,
                true,
            )?;
            let proposal_id = proposal["id"].as_str().unwrap_or("");
            review(service, proposal_id, None, true)?;
            apply(service, proposal_id)
        })();
        match effect {
            Err(error) => failed.push(object!({"record_id":id,"reason":error.to_string()})),
            Ok(proposal) => {
                promoted.push(object!({"record_id":id,"type":record["type"],"memory_id":proposal["destination_memory_id"]}));
                existing = others;
                existing.push((
                    proposal["destination_memory_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                    comparable,
                ));
            }
        }
    }
    let skipped = candidates.len() - considered + deferred_stalled + deferred_records;
    if retry_file.is_some() && only.is_none() && failed.is_empty() && skipped == 0 {
        let removal = (|| -> Result<()> {
            let path = retry_path(service)?;
            if let Some(expected) = files.read(&path, 512)? {
                files.unlink_expected(&path, &expected)?;
            }
            Ok(())
        })();
        if removal.is_err() {
            retry_file = None
        }
    }
    let mut result = object!({"enabled":true,"promoted":promoted,"failed":failed,"blocked":blocked,"closed":closed,"skipped":skipped});
    if retry_file.is_none() && (!result["failed"].as_array().unwrap().is_empty() || skipped > 0) {
        result["retry_unavailable"] = true.into()
    }
    Ok(result)
}

/// Validate the optional frozen publication plan without publishing it.
pub fn validate_publication(proposal: &Value) -> Result<()> {
    plan(proposal).map(|_| ())
}
