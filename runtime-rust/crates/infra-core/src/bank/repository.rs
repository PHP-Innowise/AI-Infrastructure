//! Complete governed-repository inspection; no mutation is performed here.

use super::{BankService, promotion};
use crate::{
    BrainRepository, Result, RuntimeError,
    domain::{self, json},
    fs,
};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn listed(root: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    if !fs::admit_directory(root)? {
        return Ok(Vec::new());
    }
    let mut paths = std::fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.extension().is_some_and(|suffix| suffix == extension));
    paths.sort();
    Ok(paths)
}
fn relative(brain: &BrainRepository, path: &Path) -> String {
    path.strip_prefix(&brain.roots().state)
        .ok()
        .and_then(Path::to_str)
        .map(|s| s.replace('\\', "/"))
        .unwrap_or_else(|| path.to_string_lossy().into())
}
pub fn index_data(brain: &BrainRepository) -> Result<[Value; 2]> {
    let mut groups = [Vec::new(), Vec::new()];
    for image in brain.records(true)? {
        let path = relative(brain, &image.path);
        groups[usize::from(image.archived)].push(object!({"id":image.record["id"],"type":image.record["type"],"status":image.record["status"],"revision":image.record["revision"],"path":path}));
    }
    for group in &mut groups {
        group.sort_by_key(|row| {
            (
                row["type"].to_string(),
                row["id"].to_string(),
                row["path"].to_string(),
            )
        });
    }
    let [active, archive] = groups;
    Ok([object!(active), object!(archive)])
}
pub fn index_errors(service: &BankService) -> Result<Vec<String>> {
    const REMEDY: &str = "It is derived from the records: the next Brain write rebuilds it - a checkpoint, \u{0060}update\u{0060}, \u{0060}record-result\u{0060} or \u{0060}compact\u{0060}.";
    let brain = service.brain()?;
    let expected = match index_data(&brain) {
        Ok(data) => data,
        Err(error) => {
            return Ok(vec![format!(
                "{}: {error}",
                relative(&brain, &brain.roots().brain_root().join("indexes"))
            )]);
        }
    };
    let mut errors = Vec::new();
    for (name, wanted) in ["active", "archive"].into_iter().zip(expected) {
        let path = brain
            .roots()
            .brain_root()
            .join(format!("indexes/{name}.json"));
        let shown = relative(&brain, &path);
        let raw = match fs::read_text(&path) {
            Ok(raw) => raw,
            Err(error) => {
                errors.push(format!(
                    "{shown}: deterministic index unreadable ({error}). {REMEDY}"
                ));
                continue;
            }
        };
        match json::parse(&raw){
            Ok(actual)if actual!=wanted=>errors.push(format!("{shown}: deterministic index is stale or non-canonical. {REMEDY}")),
            Ok(_)=>{},
            Err(error)=>errors.push(format!("{shown}: deterministic index is not valid JSON ({error}); a merge can leave conflict markers in it. {REMEDY}"))
        }
    }
    Ok(errors)
}
fn stored_secret(record: &Value) -> Result<Option<&'static str>> {
    let mut fields = ["title", "goal", "progress", "auto_checkpoint"]
        .into_iter()
        .filter_map(|key| record[key].as_str())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    fields.extend(
        record["next_steps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|s| !s.is_empty()),
    );
    fields.extend(
        record["transitions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| row["reason"].as_str())
            .filter(|s| !s.is_empty()),
    );
    domain::secret_label(&fields.join("\n"))
}
fn check_handoffs(service: &BankService, errors: &mut Vec<String>) -> Result<()> {
    let brain = service.brain()?;
    for archived in [false, true] {
        let folder = brain.roots().brain_root().join(if archived {
            "archive/handoffs"
        } else {
            "control/handoffs"
        });
        for path in listed(&folder, "md")? {
            let result = (|| -> Result<()> {
                let (handoff, _) = domain::parse_markdown_record(&fs::read_text(&path)?)?;
                domain::validate_schema_file(brain.roots(), "handoff.schema.json", &handoff)?;
                let task = brain
                    .find(text(&handoff, "task_id"), Some("task"), archived)?
                    .record;
                domain::validate_handoff(&handoff, Some(&task))?;
                if archived && handoff["status"] != "closed" {
                    return Err(RuntimeError::invalid("archived handoff must be closed"));
                }
                Ok(())
            })();
            if let Err(error) = result {
                errors.push(format!("{}: {error}", path.display()));
            }
        }
    }
    Ok(())
}
fn message_problem(line: &str, last: bool) -> &'static str {
    if ["<<<<<<< ", "=======", ">>>>>>> "]
        .iter()
        .any(|prefix| line.starts_with(prefix))
    {
        "holds an unresolved Git merge conflict"
    } else if last {
        "is cut short: an append stopped part-way, as on a full disk; remove that line"
    } else {
        "is not valid JSON"
    }
}
fn check_messages(service: &BankService, errors: &mut Vec<String>) -> Result<()> {
    let brain = service.brain()?;
    for path in listed(
        &brain.roots().brain_root().join("control/messages"),
        "jsonl",
    )? {
        let result = (|| -> Result<()> {
            let id = path
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or_else(|| RuntimeError::invalid("message journal name is not UTF-8"))?;
            if !domain::is_uuid4(id) {
                return Err(RuntimeError::invalid(
                    "message journal name must be a task UUID",
                ));
            }
            brain.find(id, Some("task"), true)?;
            let raw = fs::read_text(&path)?;
            let mut lines = raw.split('\n').collect::<Vec<_>>();
            if lines.last() == Some(&"") {
                lines.pop();
            }
            let mut previous = 0u64;
            for (index, line) in lines.iter().enumerate() {
                let number = index + 1;
                if line.trim().is_empty() {
                    return Err(RuntimeError::invalid(format!(
                        "line {number}: blank line in message journal"
                    )));
                }
                let entry = json::parse(line).map_err(|_| {
                    RuntimeError::invalid(format!(
                        "line {number} {}",
                        message_problem(line, index + 1 == lines.len() && !raw.ends_with('\n'))
                    ))
                })?;
                crate::brain::validate_message(&entry)?;
                domain::validate_schema_file(brain.roots(), "message.schema.json", &entry)?;
                if entry["task_id"].as_str() != Some(id) {
                    return Err(RuntimeError::invalid(format!(
                        "line {number}: task_id does not match the journal"
                    )));
                }
                let seq = entry["seq"]
                    .as_u64()
                    .ok_or_else(|| RuntimeError::invalid("Message sequence must be an integer"))?;
                if previous.checked_add(1) != Some(seq) {
                    return Err(RuntimeError::invalid(format!(
                        "line {number}: seq must increase by exactly one"
                    )));
                }
                previous = seq;
            }
            Ok(())
        })();
        if let Err(error) = result {
            errors.push(format!("{}: {error}", path.display()));
        }
    }
    Ok(())
}
fn exact(value: &Value, names: &BTreeSet<&str>) -> bool {
    value.as_object().is_some_and(|map| {
        map.len() == names.len() && map.keys().all(|name| names.contains(name.as_str()))
    })
}
fn check_manifests(service: &BankService, errors: &mut Vec<String>) -> Result<()> {
    let brain = service.brain()?;
    for path in listed(
        &brain
            .roots()
            .brain_root()
            .join("control/retrieval-manifests"),
        "json",
    )? {
        let result = (|| -> Result<()> {
            let manifest = json::parse(&fs::read_text(&path)?)?;
            domain::validate_schema_file(
                brain.roots(),
                "retrieval-manifest.schema.json",
                &manifest,
            )?;
            let version = manifest["schema_version"].as_u64().ok_or_else(|| {
                RuntimeError::invalid("retrieval manifest does not match strict schema")
            })?;
            let mut keys = [
                "schema_version",
                "id",
                "created_at",
                "query",
                "task_id",
                "task_revision",
                "filters",
                "selected",
                "excluded",
                "token_estimates",
                "provider",
                "escalation_reason",
            ]
            .into_iter()
            .collect::<BTreeSet<_>>();
            if (2..=4).contains(&version) {
                keys.extend(["query_source", "phase_seconds", "gate"]);
            }
            if (3..=4).contains(&version) {
                keys.extend(["host", "entry_point", "local_episode_count"]);
            }
            if version == 4 {
                keys.insert("source_links");
                if manifest.get("source_hints").is_some() {
                    keys.insert("source_hints");
                }
            }
            if !(1..=4).contains(&version) || !exact(&manifest, &keys) {
                return Err(RuntimeError::invalid(
                    "retrieval manifest does not match strict schema",
                ));
            }
            if !domain::is_uuid4(text(&manifest, "id"))
                || !domain::is_uuid4(text(&manifest, "task_id"))
            {
                return Err(RuntimeError::invalid(
                    "retrieval manifest IDs must be UUIDv4",
                ));
            }
            if path.file_stem().and_then(|name| name.to_str()) != manifest["id"].as_str() {
                return Err(RuntimeError::invalid(
                    "retrieval manifest ID does not match filename",
                ));
            }
            let mut token_keys = [
                "policy", "handoff", "durable", "dynamic", "evidence", "total", "target", "hard",
            ]
            .into_iter()
            .collect::<BTreeSet<_>>();
            if version >= 3 {
                token_keys.insert("local_episodes");
            }
            if !exact(&manifest["token_estimates"], &token_keys) {
                return Err(RuntimeError::invalid(
                    "retrieval manifest token estimates do not match strict schema",
                ));
            }
            let estimates = &manifest["token_estimates"];
            if estimates["total"].as_u64().unwrap_or(0)
                > estimates["hard"].as_u64().unwrap_or(12000)
            {
                return Err(RuntimeError::invalid(
                    "retrieval manifest exceeds hard token ceiling",
                ));
            }
            Ok(())
        })();
        if let Err(error) = result {
            errors.push(format!("{}: {error}", path.display()));
        }
    }
    Ok(())
}
fn check_promotions(service: &BankService, errors: &mut Vec<String>) -> Result<()> {
    for path in listed(
        &service
            .context
            .roots
            .brain_root()
            .join("control/promotions"),
        "json",
    )? {
        let result = (|| -> Result<()> {
            let proposal = json::parse(&fs::read_text(&path)?)?;
            let id = path
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or_else(|| RuntimeError::invalid("Promotion filename is not UTF-8"))?;
            promotion::validate(service, &proposal, Some(id))?;
            if proposal.get("publication").is_some() {
                promotion::validate_publication(&proposal)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            errors.push(format!("{}: {error}", path.display()));
        }
    }
    Ok(())
}
pub fn repository_errors(service: &BankService, freshness: bool) -> Result<Vec<String>> {
    let brain = service.brain()?;
    let inventory = brain.inventory(true)?;
    let mut errors = Vec::new();
    let mut seen = BTreeSet::new();
    let mut records = BTreeMap::<String, (PathBuf, Value)>::new();
    for image in inventory.records {
        let record = image.record;
        let result = (|| -> Result<()> {
            domain::validate_schema_file(brain.roots(), "dynamic-record.schema.json", &record)?;
            domain::validate_record(&record, image.archived)?;
            let id = text(&record, "id");
            if !seen.insert(id.to_string()) {
                return Err(RuntimeError::invalid(format!("duplicate UUID: {id}")));
            }
            let expected = if image.archived {
                brain
                    .roots()
                    .brain_root()
                    .join("archive")
                    .join(text(&record, "type"))
                    .join(format!("{id}.md"))
            } else {
                brain.dynamic_path(&record)?
            };
            if image.path != expected {
                return Err(RuntimeError::invalid(format!(
                    "record path must be {}",
                    relative(&brain, &expected)
                )));
            }
            if freshness && !domain::sources_are_fresh(brain.roots(), &record)? {
                return Err(RuntimeError::invalid("source fingerprint is stale"));
            }
            if let Some(secret) = stored_secret(&record)? {
                return Err(RuntimeError::policy(format!(
                    "record contains a possible {secret}; remove it from the record and rotate the credential, since Git history keeps it"
                )));
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                records.insert(text(&record, "id").into(), (image.path, record));
            }
            Err(error) => errors.push(format!("{}: {error}", image.path.display())),
        }
    }
    errors.extend(
        inventory
            .problems
            .iter()
            .map(|problem| format!("{}: {}", problem.path.display(), problem.error)),
    );
    let present = records
        .keys()
        .map(String::as_str)
        .chain(
            inventory
                .problems
                .iter()
                .filter_map(|problem| problem.id.as_deref()),
        )
        .collect::<BTreeSet<_>>();
    for (id, (path, record)) in &records {
        for relationship in ["conflicts", "supersedes"]
            .into_iter()
            .flat_map(|key| record[key].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
        {
            if !present.contains(relationship) {
                errors.push(format!(
                    "{}: relationship points to missing {relationship}",
                    path.display()
                ));
            }
        }
        if let Some(replacement) = record["superseded_by"].as_str() {
            if !present.contains(replacement) {
                errors.push(format!(
                    "{}: superseded_by points to missing {replacement}",
                    path.display()
                ));
            }
            if records.get(replacement).is_some_and(|(_, other)| {
                !other["supersedes"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(id)))
            }) {
                errors.push(format!(
                    "{}: supersession backlink is missing from {replacement}",
                    path.display()
                ));
            }
        }
    }
    check_handoffs(service, &mut errors)?;
    check_messages(service, &mut errors)?;
    check_manifests(service, &mut errors)?;
    check_promotions(service, &mut errors)?;
    errors.extend(index_errors(service)?);
    Ok(errors)
}

#[cfg(test)]
mod manifest_hint_tests {
    use super::*;
    use crate::{
        OpenOptions, Roots, RuntimeContext, Store,
        retrieval::{self, RetrievalRequest},
    };
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> Result<(Fixture, BankService, Store, String)> {
        let root = std::env::temp_dir().join(format!("infra-manifest-hints-{}", domain::uuid4()));
        std::fs::create_dir_all(root.join("project-brain/schemas"))?;
        let fixture = Fixture(root.clone());
        let canonical =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../PHP Core/project-brain/schemas");
        for name in [
            "dynamic-record.schema.json",
            "handoff.schema.json",
            "retrieval-manifest.schema.json",
        ] {
            std::fs::copy(
                canonical.join(name),
                root.join("project-brain/schemas").join(name),
            )?;
        }
        let context = RuntimeContext::new(Roots::installed(&root)?, "local", "governed")?;
        let service = BankService::new(context);
        let task = service.brain()?.create_task(
            "HINT-SCHEMA",
            "Preserve native manifest compatibility",
            vec![],
            vec![],
            "team",
        )?;
        let store = Store::open(
            root.join("memory-bank/local/context.db"),
            OpenOptions::default(),
        )?;
        Ok((
            fixture,
            service,
            store,
            task["id"].as_str().unwrap().to_string(),
        ))
    }
    #[test]
    fn emitted_governed_hint_stats_are_valid_repository_evidence() -> Result<()> {
        let (fixture, service, mut store, task) = fixture()?;
        for hints in [None, Some(object!({"schema_version":1,"references":[]}))] {
            let expected = hints.is_some();
            let capsule = retrieval::retrieve(
                &mut store,
                &service.context,
                &RetrievalRequest {
                    query: "manifest compatibility".into(),
                    task_id: Some(task.clone()),
                    gate: Some("off".into()),
                    refresh_index: false,
                    source_hints: hints,
                    ..Default::default()
                },
            )?;
            let manifest_file = fixture.0.join(capsule.0["manifest"].as_str().unwrap());
            let mut manifest = json::parse(&fs::read_text(&manifest_file)?)?;
            assert_eq!(manifest.get("source_hints").is_some(), expected);
            let mut errors = vec![];
            check_manifests(&service, &mut errors)?;
            assert!(errors.is_empty(), "{errors:?}");
            manifest["unknown_member"] = true.into();
            fs::atomic_json(&manifest_file, &manifest)?;
            let mut errors = vec![];
            check_manifests(&service, &mut errors)?;
            assert!(!errors.is_empty(), "unknown metadata must remain refused");
            std::fs::remove_file(manifest_file)?;
        }
        Ok(())
    }
}
