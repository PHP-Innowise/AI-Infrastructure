//! Draft parsing, source admission and truthful memory-use reporting.
use crate::{HarnessError, Result};
use infra_core::{self as core, domain::json as wire};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
pub const TEXT_LIMIT: usize = 200_000;
pub const AUTOMATIC_REASON: &str =
    "Saved automatically when a Harness run completed; agent-attested, not reviewed by a person";
const REQUEST: &str = r#"Project Brain memory draft: this session is linked to a Brain task. End your final reply with one fenced block whose info string is memory-draft, holding JSON: {"progress": "where the task stands now, one or two sentences", "next_steps": ["the next action"], "learnings": [{"type": "finding" or "decision", "title": "one line", "consequence": "the reusable rule or decision, one or two sentences", "sources": ["project-relative file that proves it"]}], "used_memory": ["retrieved capsule path whose claim you checked and used"]}. Use at most 3 next steps and at most 3 learnings. A learning is durable, reusable knowledge you verified in a project file during this run, not progress and not a guess; use [] when there is none. Never include secrets, personal data, logs or transcripts. Normal saves need no learning revision. For a deliberate correction with an inspected record, you may include its positive integer expected_revision. For an inspected existing claim, also supply record_id and the complete checked source_fingerprints from memory-bank/bin/infra-engine memory source-fingerprints, preserving its title/type/source set; never duplicate it. List at most 10 used_memory paths; [] is valid. This is your report of use, not proof of reading. "#;
pub fn instruction() -> String {
    format!(
        "{REQUEST}The Harness saves the draft to project memory as written, with no review, so leave out anything you did not verify."
    )
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}
fn text(value: &Value, label: &str, limit: usize) -> Result<String> {
    let text = value
        .as_str()
        .ok_or_else(|| HarnessError::invalid(format!("Invalid {label}.")))?;
    if text.trim().is_empty()
        || text.chars().count() > limit
        || text.chars().any(|character| {
            character == '\0'
                || character == '\u{7f}'
                || character < ' ' && !['\n', '\t'].contains(&character)
        })
    {
        return Err(HarnessError::invalid(format!("Invalid {label}.")));
    }
    Ok(text.trim().into())
}
fn clip(value: &Value, limit: usize) -> String {
    value
        .as_str()
        .map(|value| core::query::chars(&core::results::collapse(value), limit))
        .unwrap_or_default()
}
fn plain(value: &Value, limit: usize) -> String {
    value
        .as_str()
        .map(|value| {
            core::query::chars(
                &core::results::collapse(
                    &value
                        .chars()
                        .map(|character| {
                            if character > '\u{1f}' && character != '\u{7f}' {
                                character
                            } else {
                                ' '
                            }
                        })
                        .collect::<String>(),
                ),
                limit,
            )
        })
        .unwrap_or_default()
}
fn source(value: &Value) -> Result<String> {
    let source = text(value, "learning source", 1024)?;
    crate::knowledge::relative(source.split('#').next().unwrap_or(""))?;
    Ok(source)
}
fn has_reference(value: &Value) -> bool {
    value.get("record_id").is_some() || value.get("source_fingerprints").is_some()
}
fn reference(item: &Value, sources: &[String]) -> Result<Value> {
    if !has_reference(item) {
        return Ok(json!({}));
    }
    let id = item["record_id"]
        .as_str()
        .filter(|id| core::domain::is_uuid4(id))
        .ok_or_else(|| {
            HarnessError::invalid(
                "Existing knowledge needs record_id and its original expected_revision.",
            )
        })?;
    let revision = item["expected_revision"]
        .as_u64()
        .filter(|revision| *revision > 0)
        .ok_or_else(|| {
            HarnessError::invalid(
                "Existing knowledge needs record_id and its original expected_revision.",
            )
        })?;
    let pins = item["source_fingerprints"]
        .as_array()
        .filter(|pins| (1..=10).contains(&pins.len()))
        .ok_or_else(|| {
            HarnessError::invalid("Existing knowledge needs its checked source_fingerprints.")
        })?;
    let mut image = BTreeMap::new();
    for pin in pins {
        if pin.as_object().is_none_or(|fields| fields.len() != 2)
            || !pin["sha256"].as_str().is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
        {
            return Err(HarnessError::invalid("Invalid checked source image."));
        }
        let path = source(&pin["path"])?;
        let digest = pin["sha256"].as_str().unwrap_or("");
        if path.contains('#') || image.get(&path).is_some_and(|previous| previous != digest) {
            return Err(HarnessError::invalid("Invalid checked source image."));
        }
        image.insert(path, digest.to_string());
    }
    if !(1..=10).contains(&sources.len())
        || image.keys().cloned().collect::<BTreeSet<_>>()
            != sources
                .iter()
                .map(|source| source.split('#').next().unwrap_or("").to_string())
                .collect()
    {
        return Err(HarnessError::invalid(
            "An existing claim needs its complete checked source image.",
        ));
    }
    Ok(
        json!({"record_id":id,"expected_revision":revision,"source_fingerprints":image.into_iter().map(|(path,sha256)|json!({"path":path,"sha256":sha256})).collect::<Vec<_>>()}),
    )
}
fn merge(target: &mut Value, source: Value) {
    if let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) {
        target.extend(source.clone());
    }
}
fn capture(data: &Value, learnings: &[Value]) -> Result<Value> {
    let Some(value) = data.get("capture_decision") else {
        return Ok(json!({}));
    };
    let decision=value.as_str().filter(|value|["selected","no_reusable_knowledge","insufficiently_checked"].contains(value)).ok_or_else(||HarnessError::invalid("Use selected, no_reusable_knowledge or insufficiently_checked for capture_decision."))?;
    if (decision == "selected") != !learnings.is_empty() {
        return Err(HarnessError::invalid(
            "selected needs a learning; other capture decisions need empty learnings.",
        ));
    }
    Ok(json!({"capture_decision":decision}))
}
pub fn parse(text: &str) -> Option<Value> {
    if text.chars().count() > TEXT_LIMIT {
        return None;
    }
    let grave = char::from(96);
    let pattern = regex::Regex::new(&format!(
        r"(?s){grave}{grave}{grave}memory-draft[^\S\n]*\n(.*?)\n[^\S\n]*{grave}{grave}{grave}"
    ))
    .ok()?;
    let raw = pattern.captures_iter(text).last()?.get(1)?.as_str();
    let data = wire::parse(raw).ok()?;
    if !data.is_object() {
        return None;
    }
    let mut learnings = Vec::new();
    for item in data["learnings"].as_array().into_iter().flatten() {
        if !item.is_object() || !matches!(item["type"].as_str(), Some("finding" | "decision")) {
            continue;
        }
        let sources = item["sources"].as_array().cloned().unwrap_or_default();
        let mut learning = json!({"type":item["type"],"title":clip(&item["title"],200),"consequence":clip(&item["consequence"],1000),"sources":sources.iter().filter_map(Value::as_str).filter(|source|!source.trim().is_empty()).take(10).map(|source|source.trim()).collect::<Vec<_>>()});
        if has_reference(item) {
            let checked = (|| -> Result<()> {
                if !(1..=10).contains(&sources.len())
                    || sources.iter().any(|source| {
                        !source
                            .as_str()
                            .is_some_and(|source| !source.trim().is_empty())
                    })
                {
                    return Err(HarnessError::invalid("Invalid existing learning sources."));
                }
                let sources = core::domain::unique_strings(
                    sources.iter().map(source).collect::<Result<Vec<_>>>()?,
                );
                learning["sources"] = json!(sources);
                for (field, limit) in [("title", 200), ("consequence", 1000)] {
                    let value = self::text(&item[field], field, limit)?;
                    learning[field] = json!(value);
                }
                merge(&mut learning, reference(item, &sources)?);
                Ok(())
            })();
            if checked.is_err() {
                continue;
            }
        }
        if let Some(revision) = item["expected_revision"]
            .as_u64()
            .filter(|revision| *revision > 0)
        {
            learning["expected_revision"] = json!(revision);
        }
        if learning["title"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
            && learning["consequence"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
        {
            learnings.push(learning);
        }
    }
    learnings.truncate(3);
    let mut result = json!({"progress":clip(&data["progress"],1000),"next_steps":data["next_steps"].as_array().into_iter().flatten().map(|step|clip(step,300)).filter(|step|!step.is_empty()).take(3).collect::<Vec<_>>(),"learnings":learnings});
    let selection = capture(&data, &learnings).ok()?;
    merge(&mut result, selection);
    if data.get("used_memory").is_some() {
        result["used_memory"] = json!(
            core::domain::unique_strings(
                data["used_memory"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|value| !value.trim().is_empty() && value.chars().count() <= 1024)
                    .map(|value| value.trim().to_string())
            )
            .into_iter()
            .take(10)
            .collect::<Vec<_>>()
        );
    }
    Some(result)
}
pub fn submission(data: &Value) -> Result<Value> {
    let fields = [
        "progress",
        "next_steps",
        "learnings",
        "verified",
        "used_memory",
        "capture_decision",
    ];
    if data
        .as_object()
        .is_none_or(|object| object.keys().any(|key| !fields.contains(&key.as_str())))
    {
        return Err(HarnessError::invalid("Invalid memory draft."));
    }
    let progress = match data.get("progress") {
        None => String::new(),
        Some(Value::String(value)) if value.chars().count() <= 1000 => {
            if value.trim().is_empty() {
                String::new()
            } else {
                text(&json!(value), "progress", 4000)?
            }
        }
        _ => {
            return Err(HarnessError::invalid(
                "Write progress in at most 1000 characters.",
            ));
        }
    };
    let steps = match data.get("next_steps") {
        None => vec![],
        Some(Value::Array(items)) if items.len() <= 3 => items
            .iter()
            .map(|value| text(value, "next step", 1200))
            .collect::<Result<Vec<_>>>()?,
        _ => return Err(HarnessError::invalid("Keep at most 3 next steps.")),
    };
    if steps.iter().any(|step| step.chars().count() > 300) {
        return Err(HarnessError::invalid(
            "Write each next step in at most 300 characters.",
        ));
    }
    let items = match data.get("learnings") {
        None => vec![],
        Some(Value::Array(items)) if items.len() <= 3 => items.clone(),
        _ => return Err(HarnessError::invalid("Keep at most 3 learnings.")),
    };
    let mut kept = Vec::new();
    for item in items {
        let required = ["type", "title", "consequence", "sources"];
        let optional = ["expected_revision", "record_id", "source_fingerprints"];
        if item.as_object().is_none_or(|fields| {
            required.iter().any(|field| !fields.contains_key(*field))
                || fields.keys().any(|field| {
                    !required.contains(&field.as_str()) && !optional.contains(&field.as_str())
                })
        }) || !matches!(item["type"].as_str(), Some("finding" | "decision"))
        {
            return Err(HarnessError::invalid(
                "Each learning needs a type, a title, a consequence and sources.",
            ));
        }
        let title = text(&item["title"], "learning title", 800)?;
        let consequence = text(&item["consequence"], "learning consequence", 4000)?;
        if title.chars().count() > 200 || consequence.chars().count() > 1000 {
            return Err(HarnessError::invalid(
                "Keep a learning's title within 200 and its consequence within 1000 characters.",
            ));
        }
        let sources = item["sources"]
            .as_array()
            .filter(|sources| (1..=10).contains(&sources.len()))
            .ok_or_else(|| {
                HarnessError::invalid("Cite 1 to 10 project files for each learning.")
            })?;
        let sources =
            core::domain::unique_strings(sources.iter().map(source).collect::<Result<Vec<_>>>()?);
        let mut learning =
            json!({"type":item["type"],"title":title,"consequence":consequence,"sources":sources});
        if item.get("expected_revision").is_some() {
            let revision = item["expected_revision"]
                .as_u64()
                .filter(|revision| *revision > 0)
                .ok_or_else(|| {
                    HarnessError::invalid(
                        "Learning expected_revision must be a positive original revision.",
                    )
                })?;
            learning["expected_revision"] = json!(revision);
        }
        merge(&mut learning, reference(&item, &sources)?);
        kept.push(learning);
    }
    if !kept.is_empty() && data["verified"] != true {
        return Err(HarnessError::invalid(
            "Confirm that you checked each kept learning against its sources.",
        ));
    }
    let selection = capture(data, &kept)?;
    if progress.is_empty() && steps.is_empty() && kept.is_empty() {
        return Err(HarnessError::invalid(
            "Nothing to save: write progress, a next step or a learning.",
        ));
    }
    let mut draft = json!({"progress":progress,"next_steps":steps,"learnings":kept});
    merge(&mut draft, selection);
    Ok(draft)
}
fn unusable(root: &Path, source: &str) -> Option<String> {
    if let Some(reason) = core::domain::source_path_problem(source) {
        return Some(format!(
            "{reason}: {}",
            source.split('#').next().unwrap_or("")
        ));
    }
    let path = source.split('#').next().unwrap_or("");
    let checked = (|| -> core::Result<()> {
        let mut files = core::bank::publication::PublicationFiles::new(root)?;
        files
            .image_identity(path)?
            .ok_or_else(|| core::RuntimeError::not_found("Source not found"))?;
        Ok(())
    })();
    checked
        .err()
        .map(|_| format!("Source not found in the session workspace: {path}"))
}
pub fn check_sources(root: &Path, draft: &Value) -> Result<()> {
    for item in draft["learnings"].as_array().into_iter().flatten() {
        for source in strings(&item["sources"]) {
            if let Some(reason) = unusable(root, &source) {
                return Err(HarnessError::invalid(reason));
            }
        }
    }
    Ok(())
}
pub fn usable(root: &Path, draft: &Value) -> Result<(Value, Vec<Value>)> {
    let original = draft["learnings"].as_array().cloned().unwrap_or_default();
    let mut selection = capture(draft, &original)?;
    let mut prepared = Vec::new();
    let mut consequences = BTreeMap::<String, BTreeSet<String>>::new();
    for item in original {
        let mut sources = Vec::new();
        for raw in item["sources"].as_array().into_iter().flatten() {
            if let Ok(source) = source(raw)
                && unusable(root, &source).is_none()
                && !sources.contains(&source)
            {
                sources.push(source);
            }
        }
        let mut learning = json!({"type":item["type"],"title":plain(&item["title"],200),"consequence":plain(&item["consequence"],1000),"sources":sources});
        let mut valid = true;
        if has_reference(&item) {
            let checked = (|| -> Result<()> {
                let original = core::domain::unique_strings(
                    item["sources"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(source)
                        .collect::<Result<Vec<_>>>()?,
                );
                if original.len() != sources.len() {
                    return Err(HarnessError::invalid("A checked source is unavailable."));
                }
                merge(&mut learning, reference(&item, &sources)?);
                for (field, limit) in [("title", 200), ("consequence", 1000)] {
                    let value = text(&item[field], field, limit * 4)?;
                    if value.chars().count() > limit {
                        return Err(HarnessError::invalid("Invalid existing claim text."));
                    }
                    learning[field] = json!(value);
                }
                Ok(())
            })();
            valid = checked.is_ok();
        }
        if item.get("expected_revision").is_some() {
            if let Some(revision) = item["expected_revision"]
                .as_u64()
                .filter(|revision| *revision > 0)
            {
                learning["expected_revision"] = json!(revision);
            } else {
                valid = false;
            }
        }
        let identity = if valid
            && matches!(learning["type"].as_str(), Some("finding" | "decision"))
            && learning["title"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
            && learning["consequence"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
            && !sources.is_empty()
        {
            let mut sorted = sources.clone();
            sorted.sort();
            let key = if learning["record_id"].is_string() {
                json!(["record", learning["record_id"]])
            } else {
                json!([
                    learning["type"],
                    core::query::folded(learning["title"].as_str().unwrap_or("")),
                    sorted
                ])
            };
            Some(wire::canonical(&key)?)
        } else {
            None
        };
        if let Some(identity) = &identity {
            let signature = if learning["record_id"].is_string() {
                wire::python_default(&learning, true, true, true)?
            } else {
                learning["consequence"].as_str().unwrap_or("").into()
            };
            consequences
                .entry(identity.clone())
                .or_default()
                .insert(signature);
        }
        prepared.push((learning, identity));
    }
    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = BTreeSet::new();
    for (learning, identity) in prepared {
        let reason = if let Some(identity) = identity {
            if consequences
                .get(&identity)
                .is_some_and(|values| values.len() > 1)
            {
                Some("conflict")
            } else if !seen.insert(identity) {
                Some("repeated")
            } else {
                None
            }
        } else {
            Some("unsourced")
        };
        if let Some(reason) = reason {
            skipped.push(json!({"title":learning["title"].as_str().filter(|text|!text.is_empty()).unwrap_or("an untitled learning"),"reason":reason}));
        } else {
            kept.push(learning);
        }
    }
    kept.truncate(3);
    if selection["capture_decision"] == "selected" && kept.is_empty() {
        selection = json!({});
    }
    let mut output = json!({"progress":plain(&draft["progress"],1000),"next_steps":draft["next_steps"].as_array().into_iter().flatten().map(|value|plain(value,300)).filter(|value|!value.is_empty()).take(3).collect::<Vec<_>>(),"verified":!kept.is_empty(),"learnings":kept});
    merge(&mut output, selection);
    Ok((output, skipped))
}
pub fn delivered(capsule: &Value) -> BTreeSet<String> {
    ["procedural", "semantic", "episodic", "selected"]
        .into_iter()
        .flat_map(|layer| capsule[layer].as_array().into_iter().flatten())
        .filter_map(|item| item["path"].as_str().map(String::from))
        .collect()
}
pub fn watch(capsule: &Value) -> BTreeMap<String, String> {
    let trees = [
        ".agents/skills/",
        ".claude/skills/",
        ".cursor/skills/",
        ".codex/skills/",
    ];
    let mut output = BTreeMap::new();
    for path in delivered(capsule) {
        output.entry(path.clone()).or_insert(path.clone());
        let found = trees.iter().find_map(|tree| {
            if let Some(inside) = path.strip_prefix(tree) {
                Some((String::new(), inside))
            } else {
                path.find(&format!("/{tree}")).map(|index| {
                    (
                        path[..index + 1].to_string(),
                        &path[index + 1 + tree.len()..],
                    )
                })
            }
        });
        if let Some((prefix, inside)) = found {
            for tree in trees {
                output
                    .entry(format!("{prefix}{tree}{inside}"))
                    .or_insert(path.clone());
            }
            if let Some((name, "SKILL.md")) = inside.split_once('/')
                && !name.is_empty()
            {
                output
                    .entry(format!("skill:{name}"))
                    .or_insert(path.clone());
            }
        }
    }
    output
}
pub fn usage(draft: &Value, capsule: &Value, read: Option<&Value>) -> Value {
    let paths = delivered(capsule);
    let (observation, opened) = match read {
        None => ("unrecorded", None),
        Some(read) => match (read["observation"].as_str(), read["opened"].as_u64()) {
            (Some(observation @ ("observed" | "lower_bound")), Some(opened)) => {
                (observation, Some(opened.min(paths.len() as u64)))
            }
            _ => ("unknown", None),
        },
    };
    let unseen = read
        .into_iter()
        .flat_map(|read| read["unseen"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .filter(|name| ["shell", "helpers", "outside"].contains(name))
        .collect::<Vec<_>>();
    let reported = strings(&draft["used_memory"])
        .into_iter()
        .filter(|path| paths.contains(path))
        .collect::<BTreeSet<_>>();
    json!({"delivered":paths.len(),"opened":opened,"observation":observation,"unseen":unseen,"reported_used":reported,"attestation":"agent-reported","reported":draft.get("used_memory").is_some()})
}
pub fn use_notice(provenance: &Value) -> String {
    let names = strings(&provenance["unseen"])
        .into_iter()
        .filter_map(|name| match name.as_str() {
            "shell" => Some("shell commands"),
            "helpers" => Some("helper agents"),
            "outside" => Some("reads outside the project"),
            _ => None,
        })
        .collect::<Vec<_>>();
    let unseen = if names.is_empty() {
        "some reads".into()
    } else {
        names.join(" and ")
    };
    let opened = provenance["opened"].as_u64().unwrap_or(0);
    let seen = match provenance["observation"].as_str() {
        Some("observed") => format!("{opened} opened by the agent"),
        Some("lower_bound") => format!(
            "{} ({unseen} not inspected)",
            if opened > 0 {
                format!("at least {opened} opened by the agent")
            } else {
                "none opened with a read tool".into()
            }
        ),
        Some("unknown") => format!("opening unknown (no read reported; {unseen} not inspected)"),
        _ => "opening not recorded".into(),
    };
    format!(
        "Memory use: {} delivered, {seen}{}. Neither proves the claim was used.",
        provenance["delivered"].as_u64().unwrap_or(0),
        if provenance["reported"] == true {
            format!(
                ", {} reported used",
                provenance["reported_used"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(0)
            )
        } else {
            String::new()
        }
    )
}
pub fn summary(state: &str, result: Option<&Value>) -> String {
    if state == "missing" {
        return "This run left no memory draft, so nothing was saved to project memory.".into();
    }
    if state == "unreadable" {
        return "This run's memory draft could not be read, so nothing was saved to project memory.".into();
    }
    let empty = json!({});
    let result = result.unwrap_or(&empty);
    let saved = &result["saved"];
    let records = saved["records"].as_array().cloned().unwrap_or_default();
    let named = |record: &Value| {
        format!(
            "{} “{}”",
            record["type"].as_str().unwrap_or("None"),
            record["title"].as_str().unwrap_or("None")
        )
    };
    let mut parts = Vec::new();
    if saved["task"].is_object() {
        parts.push("the task's progress and next steps".into());
    }
    parts.extend(
        records
            .iter()
            .filter(|record| {
                record["state"].is_null()
                    || matches!(
                        record["state"].as_str(),
                        Some("created" | "completed" | "updated")
                    )
            })
            .map(named),
    );
    let mut output = if !parts.is_empty() {
        format!("Saved to project memory: {}.", parts.join("; "))
    } else if result["ok"] == true {
        "Nothing new to save to project memory from this run.".into()
    } else {
        "Project memory could not complete this save.".into()
    };
    let already = records
        .iter()
        .filter(|record| matches!(record["state"].as_str(), Some("existing" | "archived")))
        .map(named)
        .collect::<Vec<_>>();
    if !already.is_empty() {
        output.push_str(&format!(
            " Already in project memory: {}.",
            already.join("; ")
        ));
    }
    for record in records.iter().filter(|record| {
        matches!(
            record["state"].as_str(),
            Some("differs" | "archived-differs")
        )
    }) {
        let name = named(record);
        match record["reason"].as_str() {
            Some("record-not-found") => output.push_str(&format!(
                " Not saved: {name} refers to a record that no longer exists (record-not-found)."
            )),
            Some(reason) => output.push_str(&format!(
                " Left unchanged: {name}{} ({reason}).",
                record["revision"]
                    .as_u64()
                    .map(|revision| format!(" at revision {revision}"))
                    .unwrap_or_default()
            )),
            None => output.push_str(&format!(
                " Left unchanged: {name}; this run did not replace it."
            )),
        }
    }
    let removed = records
        .iter()
        .filter(|record| record["state"] == "missing")
        .map(named)
        .collect::<Vec<_>>();
    if !removed.is_empty() {
        output.push_str(&format!(
            " Removed from project memory since this run saved it: {}.",
            removed.join("; ")
        ));
    }
    let skipped = result["skipped"].as_array().cloned().unwrap_or_default();
    for (reason, template) in [
        ("repeated", "duplicate learning(s) within this draft."),
        (
            "conflict",
            "conflicting learning(s); no claim from those groups was saved.",
        ),
        ("unsourced", "learning(s) citing no file in the workspace."),
    ] {
        let count = skipped
            .iter()
            .filter(|item| item["reason"] == reason)
            .count();
        if count > 0 {
            output.push_str(&format!(" Left out {count} {template}"));
        }
    }
    let promotion = &saved["promotion"];
    if promotion.is_object() {
        if let Some(error) = promotion["error"].as_str() {
            output.push_str(&format!(" Not promoted yet: {error}"));
        } else if promotion["enabled"] == false {
            output.push_str(
                " Automatic promotion is off for this project, so it stays in Project Brain.",
            );
        }
        let promoted = promotion["promoted"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["memory_id"].as_str())
            .collect::<Vec<_>>();
        if !promoted.is_empty() {
            output.push_str(&format!(
                " Promoted to the Memory Bank as {}.",
                promoted.join(", ")
            ));
        }
        let held = promotion["blocked"].as_array().map(Vec::len).unwrap_or(0)
            + promotion["failed"].as_array().map(Vec::len).unwrap_or(0);
        if held > 0 {
            output.push_str(&format!(
                " {held} held back from the Memory Bank; they stay in Project Brain."
            ));
        }
    }
    if result["ok"] != true {
        output.push_str(&format!(
            " Stopped: {}",
            result["error"]
                .as_str()
                .unwrap_or("the runtime did not finish.")
        ));
        if !parts.is_empty() {
            output.push_str(" What is listed was saved.");
        }
    }
    output
}

#[cfg(test)]
mod native_guidance_tests {
    #[test]
    fn source_fingerprints_instruction_uses_the_installed_native_command() {
        let output = super::instruction();
        assert!(output.contains("memory-bank/bin/infra-engine memory source-fingerprints"));
        assert!(!output.contains("context.py"));
    }
}
