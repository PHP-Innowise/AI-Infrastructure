use crate::{
    ErrorKind, Result, Roots, RuntimeContext, RuntimeError,
    domain::*,
    fs::{self, FileSnapshot, MutationLock},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateRecord {
    pub record_type: String,
    pub external_id: String,
    pub title: String,
    pub files: Vec<String>,
    pub sources: Vec<String>,
    pub owner: String,
    pub privacy: String,
    pub authority: String,
    pub confidence: f64,
    pub goal: Option<String>,
    pub conflicts: Vec<String>,
    pub attestation: Option<String>,
}
impl Default for CreateRecord {
    fn default() -> Self {
        Self {
            record_type: "task".into(),
            external_id: String::new(),
            title: String::new(),
            files: vec![],
            sources: vec![],
            owner: String::new(),
            privacy: "team".into(),
            authority: "observed".into(),
            confidence: 1.0,
            goal: None,
            conflicts: vec![],
            attestation: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateRecord {
    pub expected_revision: Option<u64>,
    pub progress: Option<String>,
    pub next_steps: Vec<String>,
    pub files: Vec<String>,
    pub sources: Vec<String>,
    pub actor: String,
    pub conflicts: Option<Vec<String>>,
    pub transition_to: Option<String>,
    pub phase: Option<String>,
    pub authority: Option<String>,
    pub auto_checkpoint: Option<String>,
    pub reason: String,
    pub attestation: Option<String>,
    pub reverify: bool,
    pub checked_source_fingerprints: Option<Vec<Value>>,
    pub allow_phase_regression: bool,
    pub replace_next_steps: bool,
}
impl Default for UpdateRecord {
    fn default() -> Self {
        Self {
            expected_revision: None,
            progress: None,
            next_steps: vec![],
            files: vec![],
            sources: vec![],
            actor: String::new(),
            conflicts: None,
            transition_to: None,
            phase: None,
            authority: None,
            auto_checkpoint: None,
            reason: "Task updated".into(),
            attestation: None,
            reverify: false,
            checked_source_fingerprints: None,
            allow_phase_regression: false,
            replace_next_steps: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageInput {
    pub from_actor: String,
    pub to_actor: String,
    pub message_type: String,
    pub body: String,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub capsule_digest: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RecordImage {
    pub path: PathBuf,
    pub record: Value,
    pub body: String,
    pub archived: bool,
}
#[derive(Clone, Debug)]
pub struct RecordProblem {
    pub path: PathBuf,
    pub error: String,
    pub record_type: Option<String>,
    pub id: Option<String>,
    pub external_id: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct RecordInventory {
    pub records: Vec<RecordImage>,
    pub problems: Vec<RecordProblem>,
}

#[derive(Clone, Debug)]
pub struct BrainRepository {
    pub context: RuntimeContext,
}

impl BrainRepository {
    pub fn new(context: RuntimeContext) -> Result<Self> {
        fs::admit_directory(&context.roots.brain_root())?;
        Ok(Self { context })
    }
    pub fn roots(&self) -> &Roots {
        &self.context.roots
    }
    pub fn lock(&self) -> Result<MutationLock> {
        MutationLock::acquire(&self.roots().state)
    }

    pub fn inventory(&self, include_archive: bool) -> Result<RecordInventory> {
        let (records, problems) = self.inventory_map(include_archive, |image, _raw| image)?;
        Ok(RecordInventory { records, problems })
    }

    /// Keep a consumer's projection of each admitted image without reading or
    /// parsing its source again. The public inventory needs only parsed data.
    pub(crate) fn inventory_map<T>(
        &self,
        include_archive: bool,
        mut map: impl FnMut(RecordImage, String) -> T,
    ) -> Result<(Vec<T>, Vec<RecordProblem>)> {
        let anchor = self.roots().brain_root();
        if !fs::admit_directory(&anchor)? {
            return Ok((Vec::new(), Vec::new()));
        }
        // Active records first and the archive after, each in path order, as
        // the retired runtime read them: validation and its stale list name
        // the records an agent works with before the archived ones.
        let mut candidates = Vec::new();
        collect_records(&anchor.join("dynamic"), &mut candidates)?;
        candidates.sort();
        if include_archive {
            let mut archived = Vec::new();
            collect_records(&anchor.join("archive"), &mut archived)?;
            archived.sort();
            candidates.extend(archived);
        }
        let mut records = Vec::new();
        let mut problems = Vec::new();
        for path in candidates {
            let archived = path.strip_prefix(&anchor).is_ok_and(|relative| {
                relative
                    .components()
                    .any(|part| part.as_os_str() == "archive")
            });
            let text = fs::read_text(&path);
            let parsed = text
                .as_ref()
                .map_err(|error| RuntimeError::invalid(error.to_string()))
                .and_then(|text| parse_markdown_record(text))
                .map_err(|error| {
                    // A half-merged record names the conflict, not the JSON
                    // error its markers cause.
                    if text.as_ref().is_ok_and(|text| {
                        text.lines().any(|line| line.starts_with("<<<<<<< "))
                    }) {
                        RuntimeError::invalid(
                            "unresolved Git merge conflict (<<<<<<< ... >>>>>>>); keep one side or merge both, without the markers",
                        )
                    } else {
                        error
                    }
                });
            if let Ok((metadata, _)) = &parsed
                && metadata["type"] == "handoff"
            {
                let expected = metadata["task_id"].as_str().unwrap_or("");
                let error = if path.parent() == Some(anchor.join("archive/handoffs").as_path())
                    && path.file_stem().and_then(|name| name.to_str()) == Some(expected)
                {
                    let checked = validate_handoff(metadata, None).and_then(|()| {
                        if metadata["status"] == "closed" {
                            Ok(())
                        } else {
                            Err(RuntimeError::invalid("archived handoff must be closed"))
                        }
                    });
                    match checked {
                        Ok(()) => continue,
                        Err(error) => format!("archived handoff invalid: {}", error.message),
                    }
                } else {
                    "unexpected handoff in authority path".into()
                };
                problems.push(RecordProblem {
                    path,
                    error,
                    record_type: None,
                    id: metadata["id"].as_str().map(String::from),
                    external_id: metadata
                        .get("external_id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .or(metadata["task_id"].as_str())
                        .map(String::from),
                });
                continue;
            }
            // A record that parsed but fails is named by the field the schema
            // rejects where it rejects one; the lifecycle check alone may not
            // name it.
            let parsed = parsed.and_then(|(record, body)| {
                let identified = RECORD_TYPES.contains(&record["type"].as_str().unwrap_or(""))
                    && record["id"].as_str().is_some_and(is_uuid4)
                    && record["external_id"]
                        .as_str()
                        .is_some_and(|id| !id.trim().is_empty());
                let checked = if identified {
                    validate_record(&record, archived)
                } else {
                    Err(RuntimeError::invalid("record identity unavailable"))
                };
                if let Err(error) = checked {
                    return Err(crate::domain::validate_schema_file(
                        self.roots(),
                        "dynamic-record.schema.json",
                        &record,
                    )
                    .err()
                    .unwrap_or(error));
                }
                Ok((record, body))
            });
            match parsed {
                Ok((record, body)) => records.push(map(
                    RecordImage {
                        path,
                        record,
                        body,
                        archived,
                    },
                    text?,
                )),
                Err(error) => {
                    let (kind, id, external) = text
                        .as_ref()
                        .ok()
                        .map(|text| claimed_identity(text))
                        .unwrap_or_default();
                    let id = id.or_else(|| {
                        path.file_stem()
                            .and_then(|name| name.to_str())
                            .filter(|name| is_uuid4(name))
                            .map(String::from)
                    });
                    problems.push(RecordProblem {
                        path,
                        error: error.to_string(),
                        record_type: kind,
                        id,
                        external_id: external,
                    });
                }
            }
        }
        Ok((records, problems))
    }

    pub fn records(&self, include_archive: bool) -> Result<Vec<RecordImage>> {
        Ok(self.inventory(include_archive)?.records)
    }

    pub fn find(
        &self,
        identifier: &str,
        record_type: Option<&str>,
        include_archive: bool,
    ) -> Result<RecordImage> {
        let mut matches = Vec::new();
        let (_, problems) = self.inventory_map(include_archive, |image, _raw| {
            if record_type.is_none_or(|kind| image.record["type"].as_str() == Some(kind))
                && (image.record["id"].as_str() == Some(identifier)
                    || image.record["external_id"].as_str() == Some(identifier))
            {
                matches.push(image);
            }
        })?;
        let claimants: Vec<_> = problems
            .iter()
            .filter(|problem| {
                (problem.id.as_deref() == Some(identifier)
                    || problem.external_id.as_deref() == Some(identifier))
                    && record_type.is_none_or(|kind| {
                        problem
                            .record_type
                            .as_deref()
                            .is_none_or(|actual| actual == kind)
                    })
            })
            .collect();
        if matches.len() + claimants.len() > 1 {
            return Err(RuntimeError::conflict(format!(
                "Project Brain record identifier is ambiguous: {identifier}"
            )));
        }
        if let Some(problem) = claimants.first() {
            return Err(RuntimeError::invalid(format!(
                "Project Brain record invalid: {}: {}",
                problem
                    .path
                    .strip_prefix(&self.roots().state)
                    .unwrap_or(&problem.path)
                    .display(),
                problem.error
            )));
        }
        matches.into_iter().next().ok_or_else(|| {
            // Which record, as the retired lookup said: an agent told only
            // "not found" cannot tell a typo from a task never started.
            RuntimeError::not_found(format!(
                "Project Brain {} not found: {identifier}",
                record_type.unwrap_or("record")
            ))
        })
    }

    pub fn get(&self, identifier: &str) -> Result<Value> {
        Ok(self.find(identifier, None, false)?.record)
    }
    pub fn get_task(&self, identifier: &str) -> Result<Value> {
        Ok(self.find(identifier, Some("task"), false)?.record)
    }

    pub fn create(&self, input: CreateRecord) -> Result<Value> {
        let initial = initial_state(&input.record_type)?;
        if input.attestation.as_deref().is_some_and(|actor| {
            input.authority != "verified" || !["agent", "person"].contains(&actor)
        }) {
            return Err(RuntimeError::invalid(
                "attestation says who checked a verified record: agent or person",
            ));
        }
        let mut introduced = vec![input.title.clone()];
        introduced.extend(input.goal.iter().cloned());
        introduced.extend(input.conflicts.clone());
        // Named as the retired guard named it: "Task record", "Finding record".
        let mut kind = input.record_type.chars();
        let label = kind
            .next()
            .map(|first| format!("{}{} record", first.to_uppercase(), kind.as_str()))
            .unwrap_or_else(|| "Record".into());
        guard_shared_text(&label, &introduced)?;
        let _guard = self.lock()?;
        match self.find(&input.external_id, None, false) {
            Err(error) if error.kind == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
            Ok(_) => {
                return Err(RuntimeError::conflict(format!(
                    "Project Brain record already exists: {}",
                    input.external_id
                )));
            }
        }
        let owner = if input.owner.is_empty() {
            self.context.actor.clone()
        } else {
            input.owner
        };
        let timestamp = utc_now();
        let mut transitions = vec![
            json!({"from":null,"to":initial,"at":timestamp,"actor":owner,"reason":format!("{} created", capitalize(&input.record_type))}),
        ];
        if let Some(attestation) = &input.attestation {
            transitions.push(json!({"from":"observed","to":"verified","at":timestamp,"actor":owner,"reason":format!("Created verified [attestation:{attestation}]")}));
        }
        let record = json!({
            "schema_version":1,"id":uuid4(),"type":input.record_type,"external_id":input.external_id,
            "title":input.title,"status":initial,"revision":1,"owner":owner,"authorized_owners":[owner],
            "privacy":input.privacy,"authority":input.authority,"confidence":input.confidence,
            "created_at":timestamp,"updated_at":timestamp,"transitions":transitions,
            "source_fingerprints":source_fingerprints(self.roots(), &input.sources)?,
            "conflicts":sorted(input.conflicts),"supersedes":[],"superseded_by":null,
            "goal":input.goal.filter(|goal| !goal.is_empty()).unwrap_or(input.title),"progress":"",
            "next_steps":[],"files":sorted(input.files),"sources":sorted(input.sources)
        });
        let path = self.dynamic_path(&record)?;
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err(RuntimeError::conflict("Project Brain UUID collision"));
        }
        self.write_record(&path, &record)?;
        Ok(record)
    }

    pub fn create_task(
        &self,
        external_id: &str,
        goal: &str,
        files: Vec<String>,
        sources: Vec<String>,
        privacy: &str,
    ) -> Result<Value> {
        self.create(CreateRecord {
            external_id: external_id.into(),
            title: goal.into(),
            goal: Some(goal.into()),
            files,
            sources,
            privacy: privacy.into(),
            ..CreateRecord::default()
        })
    }

    pub fn update(&self, identifier: &str, input: UpdateRecord) -> Result<Value> {
        let mut introduced = vec![input.reason.clone()];
        introduced.extend(input.progress.iter().cloned());
        introduced.extend(input.auto_checkpoint.iter().cloned());
        introduced.extend(input.next_steps.clone());
        if let Some(conflicts) = &input.conflicts {
            introduced.extend(conflicts.clone());
        }
        guard_shared_text("Record update", &introduced)?;
        let _guard = self.lock()?;
        let image = self.find(identifier, None, false)?;
        let mut record = image.record;
        let kind = record["type"].as_str().unwrap_or("").to_string();
        let actor = if input.actor.is_empty() {
            self.context.actor.as_str()
        } else {
            input.actor.as_str()
        };
        let revision = record["revision"]
            .as_u64()
            .ok_or_else(|| RuntimeError::invalid("Invalid record revision"))?;
        if let Some(expected) = input
            .expected_revision
            .filter(|expected| *expected != revision)
        {
            return Err(RuntimeError::conflict(format!(
                "Stale {kind} revision: expected {expected}, current {revision}"
            )));
        }
        if !strings(&record, "authorized_owners")?
            .iter()
            .any(|owner| owner == actor)
        {
            return Err(RuntimeError::new(
                ErrorKind::Unauthorized,
                format!("Owner is not authorized to mutate record: {actor}"),
            ));
        }
        if kind == "event"
            && (input.progress.is_some()
                || !input.next_steps.is_empty()
                || !input.files.is_empty()
                || !input.sources.is_empty()
                || input.authority.is_some()
                || input.replace_next_steps)
        {
            return Err(RuntimeError::policy(
                "Events are immutable; only lifecycle supersession is allowed",
            ));
        }
        if input.auto_checkpoint.is_some() && kind != "task" {
            return Err(RuntimeError::invalid(
                "only a task may record an auto checkpoint",
            ));
        }
        if input
            .attestation
            .as_deref()
            .is_some_and(|actor| !["agent", "person"].contains(&actor))
        {
            return Err(RuntimeError::invalid(
                "attestation must be one of: agent, person",
            ));
        }
        let checked = ["finding", "bug", "incident", "decision"].contains(&kind.as_str())
            && record["authority"] == "verified";
        if input.reverify
            && record["authority"] == "verified"
            && input.attestation.as_deref() == Some("agent")
            && record_attestation(&record) != "agent"
        {
            return Err(RuntimeError::policy(
                "A person or unknown actor verified this record's claim; only a person revises it (attestation person)",
            ));
        }
        let old_sources = strings(&record, "sources")?;
        let cited_now: Vec<_> = unique_strings(input.sources.clone())
            .into_iter()
            .filter(|source| !old_sources.contains(source))
            .collect();
        let all_sources = unique_strings(old_sources.into_iter().chain(input.sources.clone()));
        if let Some(expected) = &input.checked_source_fingerprints {
            if !(input.reverify || input.authority.as_deref() == Some("verified"))
                || input.attestation.is_none()
            {
                return Err(RuntimeError::invalid(
                    "Checked source image requires explicit re-verification",
                ));
            }
            let mut current = source_fingerprints(self.roots(), &all_sources)?;
            current.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
            current.dedup();
            if *expected != current {
                return Err(RuntimeError::conflict(
                    "Checked source image changed before re-verification",
                ));
            }
        }
        // The next agent must not be free to overwrite a person's check;
        // record-result holds the same revision.
        if checked
            && (input
                .progress
                .as_deref()
                .is_some_and(|progress| record["progress"].as_str() != Some(progress))
                || !cited_now.is_empty())
            && input.attestation.as_deref() != Some("person")
            && record_attestation(&record) == "person"
        {
            return Err(RuntimeError::policy(
                "A person verified this record's claim; only a person revises it (attestation person), or record the new finding as a record of its own",
            ));
        }
        if let Some(authority) = &input.authority {
            if record["authority"] != "observed" || authority != "verified" {
                return Err(RuntimeError::invalid(format!(
                    "Illegal authority transition: {} -> {authority}; only observed -> verified is allowed",
                    record["authority"].as_str().unwrap_or("")
                )));
            }
            let suffix = input
                .attestation
                .as_ref()
                .map(|actor| format!(" [attestation:{actor}]"))
                .unwrap_or_default();
            record["transitions"].as_array_mut().ok_or_else(|| RuntimeError::invalid("Invalid transitions"))?.push(json!({"from":"observed","to":"verified","at":utc_now(),"actor":actor,"reason":format!("{}{suffix}", input.reason)}));
            record["authority"] = authority.clone().into();
        }
        if input.reverify {
            if input.authority.is_some() || record["authority"] != "verified" {
                return Err(RuntimeError::invalid(
                    "Only a verified record's revised claim is verified again",
                ));
            }
            let attestation = input.attestation.as_deref().ok_or_else(|| {
                RuntimeError::invalid(
                    "Say who checked the revised claim: attestation agent or person",
                )
            })?;
            record["transitions"].as_array_mut().ok_or_else(|| RuntimeError::invalid("Invalid transitions"))?.push(json!({"from":"observed","to":"verified","at":utc_now(),"actor":actor,"reason":format!("{} [attestation:{attestation}]",input.reason)}));
        }
        if let Some(to) = &input.transition_to {
            let from = record["status"].as_str().unwrap_or("").to_string();
            if !transition_allowed(&kind, &from, to) {
                // Agents pass an evidence level for a status: 15 of the 383
                // calls Codex made in one client project in September 2026
                // were `--transition verified` or `observed`.
                let hint = if !AUTHORITIES.contains(&to.as_str()) {
                    String::new()
                } else if to == "verified" {
                    format!(
                        "; {to} is an evidence level, not a {kind} status: set it with brain-update --authority verified"
                    )
                } else {
                    format!(
                        "; {to} is an evidence level, not a {kind} status: a record starts observed, and only brain-update --authority verified changes it"
                    )
                };
                return Err(RuntimeError::invalid(format!(
                    "Illegal {kind} transition: {from} -> {to}{hint}"
                )));
            }
            record["transitions"]
                .as_array_mut()
                .ok_or_else(|| RuntimeError::invalid("Invalid transitions"))?
                .push(
                    json!({"from":from,"to":to,"at":utc_now(),"actor":actor,"reason":input.reason}),
                );
            record["status"] = to.clone().into();
        }
        if let Some(phase) = &input.phase {
            if kind != "task" {
                return Err(RuntimeError::invalid("only a task may declare a phase"));
            }
            let phase = normalize_phase(phase)?;
            if !input.allow_phase_regression {
                let current = record["phase"]
                    .as_str()
                    .and_then(|current| PHASES.iter().position(|phase| *phase == current));
                let next = PHASES.iter().position(|candidate| *candidate == phase);
                if let Some((current, next)) =
                    current.zip(next).filter(|(current, next)| next < current)
                {
                    return Err(RuntimeError::policy(format!(
                        "Phase may not move backward: {} -> {}; pass allow_phase_regression to state the regression is deliberate",
                        PHASES[current], PHASES[next]
                    )));
                }
            }
            record["phase"] = phase.into();
        }
        if let Some(progress) = input.progress {
            record["progress"] = progress.into();
        }
        if let Some(checkpoint) = input.auto_checkpoint {
            record["auto_checkpoint"] = checkpoint.into();
        }
        let steps = if input.replace_next_steps {
            input.next_steps
        } else {
            strings(&record, "next_steps")?
                .into_iter()
                .chain(input.next_steps)
                .collect()
        };
        record["next_steps"] = json!(unique_strings(steps));
        let touched = unique_strings(input.files);
        let mut files: Vec<_> = strings(&record, "files")?
            .into_iter()
            .filter(|file| !touched.contains(file))
            .collect();
        files.extend(touched);
        record["files"] = json!(files);
        record["sources"] = json!(all_sources);
        if let Some(conflicts) = input.conflicts {
            record["conflicts"] = json!(unique_strings(
                strings(&record, "conflicts")?.into_iter().chain(conflicts)
            ));
        }
        if checked && input.authority.is_none() && !input.reverify {
            let mut fingerprints = record["source_fingerprints"]
                .as_array()
                .cloned()
                .ok_or_else(|| RuntimeError::invalid("Invalid fingerprints"))?;
            let new_sources: Vec<_> = cited_now
                .into_iter()
                .filter(|source| {
                    !fingerprints
                        .iter()
                        .any(|row| row["path"].as_str() == source.split('#').next())
                })
                .collect();
            fingerprints.extend(source_fingerprints(self.roots(), &new_sources)?);
            fingerprints.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
            record["source_fingerprints"] = json!(fingerprints);
        } else {
            record["source_fingerprints"] = json!(match input.checked_source_fingerprints {
                Some(pinned) => pinned,
                None => source_fingerprints(self.roots(), &strings(&record, "sources")?)?,
            });
        }
        record["revision"] = revision
            .checked_add(1)
            .ok_or_else(|| RuntimeError::invalid("Record revision overflow"))?
            .into();
        record["updated_at"] = utc_now().into();
        self.write_record(&image.path, &record)?;
        Ok(record)
    }

    pub fn close_task(
        &self,
        identifier: &str,
        outcome: &str,
        verification: &[String],
        expected_revision: Option<u64>,
    ) -> Result<Value> {
        self.get_task(identifier)?;
        let progress = if verification.is_empty() {
            outcome.into()
        } else {
            format!("{outcome}\nVerification: {}", verification.join("; "))
        };
        self.update(
            identifier,
            UpdateRecord {
                expected_revision,
                progress: Some(progress),
                transition_to: Some("completed".into()),
                reason: "Task completed after verification".into(),
                ..UpdateRecord::default()
            },
        )
    }

    pub fn cancel_task(&self, identifier: &str) -> Result<Value> {
        self.get_task(identifier)?;
        self.update(
            identifier,
            UpdateRecord {
                transition_to: Some("cancelled".into()),
                reason: "Task abandoned".into(),
                ..UpdateRecord::default()
            },
        )
    }

    pub fn dynamic_path(&self, record: &Value) -> Result<PathBuf> {
        let plural = match record["type"].as_str() {
            Some("task") => "tasks",
            Some("finding") => "findings",
            Some("bug") => "bugs",
            Some("incident") => "incidents",
            Some("decision") => "decisions",
            Some("event") => "events",
            _ => return Err(RuntimeError::invalid("Invalid record type")),
        };
        let id = record["id"]
            .as_str()
            .filter(|id| is_uuid4(id))
            .ok_or_else(|| RuntimeError::invalid("Invalid record id"))?;
        Ok(self
            .roots()
            .brain_root()
            .join("dynamic")
            .join(plural)
            .join(format!("{id}.md")))
    }

    pub fn handoff_path(&self, id: &str) -> Result<PathBuf> {
        if !is_uuid4(id) {
            return Err(RuntimeError::invalid("Handoff task id must be UUIDv4"));
        }
        Ok(self
            .roots()
            .brain_root()
            .join("control/handoffs")
            .join(format!("{id}.md")))
    }

    pub fn handoff(&self, identifier: &str) -> Result<Value> {
        let task = self.get_task(identifier)?;
        let path = self.handoff_path(task["id"].as_str().unwrap_or(""))?;
        let (handoff, _) = parse_markdown_record(&fs::read_text(&path)?)?;
        validate_handoff(&handoff, Some(&task))?;
        Ok(handoff)
    }

    fn affected(&self, path: &Path, record: &Value) -> Result<Vec<PathBuf>> {
        let mut paths = vec![
            path.to_path_buf(),
            self.roots().brain_root().join("indexes/active.json"),
            self.roots().brain_root().join("indexes/archive.json"),
        ];
        if record["type"] == "task" {
            paths.push(self.handoff_path(record["id"].as_str().unwrap_or(""))?);
        }
        Ok(paths)
    }

    pub fn snapshot_record(&self, identifier: &str) -> Result<FileSnapshot> {
        let _guard = self.lock()?;
        let image = self.find(identifier, None, false)?;
        fs::snapshot(self.affected(&image.path, &image.record)?)
    }

    pub fn restore_record(&self, snapshot: &FileSnapshot) -> Result<()> {
        let _guard = self.lock()?;
        for path in snapshot.keys() {
            fs::admit_descendant(&self.roots().brain_root(), path)?;
        }
        fs::restore(snapshot)
    }

    pub fn rollback_created(&self, identifier: &str) -> Result<()> {
        let _guard = self.lock()?;
        let image = self.find(identifier, None, false)?;
        std::fs::remove_file(image.path)?;
        if image.record["type"] == "task" {
            let path = self.handoff_path(image.record["id"].as_str().unwrap_or(""))?;
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.rebuild_indexes()
    }

    pub fn rebuild_indexes(&self) -> Result<()> {
        let _guard = self.lock()?;
        let mut active = Vec::new();
        let mut archived = Vec::new();
        for image in self.records(true)? {
            let relative = image
                .path
                .strip_prefix(&self.roots().state)
                .map_err(|_| RuntimeError::policy("Record index path escapes state"))?
                .to_str()
                .ok_or_else(|| RuntimeError::invalid("Record path is not UTF-8"))?
                .replace('\\', "/");
            let item = json!({"id":image.record["id"],"type":image.record["type"],"status":image.record["status"],"revision":image.record["revision"],"path":relative});
            if image.archived {
                archived.push(item);
            } else {
                active.push(item);
            }
        }
        let key = |item: &Value| {
            (
                item["type"].to_string(),
                item["id"].to_string(),
                item["path"].to_string(),
            )
        };
        active.sort_by_key(&key);
        archived.sort_by_key(key);
        let base = self.roots().brain_root().join("indexes");
        fs::admit_descendant(&self.roots().brain_root(), &base)?;
        fs::atomic_json(&base.join("active.json"), &json!(active))?;
        fs::atomic_json(&base.join("archive.json"), &json!(archived))
    }

    fn write_record(&self, path: &Path, record: &Value) -> Result<()> {
        validate_record(record, false)?;
        validate_schema_file(self.roots(), "dynamic-record.schema.json", record)?;
        let paths = self.affected(path, record)?;
        for path in &paths {
            fs::admit_descendant(&self.roots().brain_root(), path)?;
        }
        let handoff = if record["type"] == "task" {
            let path = self.handoff_path(record["id"].as_str().unwrap_or(""))?;
            let previous = match std::fs::symlink_metadata(&path) {
                Ok(_) => Some(parse_markdown_record(&fs::read_text(&path)?)?.0),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            let handoff = make_handoff(record, previous.as_ref())?;
            validate_handoff(&handoff, Some(record))?;
            Some((path, handoff))
        } else {
            None
        };
        let before = fs::snapshot(paths)?;
        let result = (|| -> Result<()> {
            fs::atomic_write(
                path,
                render_markdown_record(record, &record_body(record)?)?.as_bytes(),
            )?;
            if let Some((path, handoff)) = handoff {
                fs::atomic_write(
                    &path,
                    render_markdown_record(&handoff, &handoff_body(&handoff)?)?.as_bytes(),
                )?;
            }
            self.rebuild_indexes()
        })();
        if let Err(error) = result {
            fs::restore(&before)?;
            return Err(error);
        }
        Ok(())
    }

    pub fn append_message(&self, identifier: &str, input: MessageInput) -> Result<Value> {
        guard_shared_text("Agent message", std::slice::from_ref(&input.body))?;
        let _guard = self.lock()?;
        let task = self.get_task(identifier)?;
        if is_terminal("task", task["status"].as_str().unwrap_or("")) {
            return Err(RuntimeError::policy("Messages require an active task"));
        }
        let path = self.message_path(task["id"].as_str().unwrap_or(""))?;
        fs::admit_descendant(&self.roots().brain_root(), &path)?;
        let old = match std::fs::symlink_metadata(&path) {
            Ok(_) => fs::read_text(&path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.into()),
        };
        let lines = journal_lines(&old);
        let separator = if !old.is_empty() && !old.ends_with('\n') {
            let last = lines.last().copied().unwrap_or("");
            serde_json::from_str::<Value>(last)
                .map_err(|_| RuntimeError::invalid("Message journal last line is cut short"))?;
            "\n"
        } else {
            ""
        };
        let mut message = json!({"schema_version":1,"id":uuid4(),"seq":1 + lines.iter().filter(|line| !line.trim().is_empty()).count(),"task_id":task["id"],"task_revision":task["revision"],"created_at":utc_now(),"from_actor":actor(&input.from_actor,false)?,"to_actor":actor(&input.to_actor,true)?,"type":input.message_type,"body":input.body,"refs":input.refs});
        if let Some(digest) = input.capsule_digest {
            message["capsule_digest"] = digest.into();
        }
        validate_message(&message)?;
        validate_schema_file(self.roots(), "message.schema.json", &message)?;
        // Written as the retired runtime wrote it (`json.dumps(sort_keys=True)`):
        // the journal is tracked, and a migrated task keeps one line format.
        let mut line = crate::domain::json::python_default(&message, true, false, false)?;
        for (character, escaped) in [
            ('\u{2028}', "\\u2028"),
            ('\u{2029}', "\\u2029"),
            ('\u{85}', "\\u0085"),
        ] {
            line = line.replace(character, escaped);
        }
        fs::append_whole_line(&path, format!("{separator}{line}\n").as_bytes())?;
        Ok(message)
    }

    pub fn read_messages(
        &self,
        identifier: &str,
        for_actor: Option<&str>,
        since_seq: u64,
        message_type: Option<&str>,
    ) -> Result<Vec<Value>> {
        let task = self.find(identifier, Some("task"), true)?.record;
        let path = self.message_path(task["id"].as_str().unwrap_or(""))?;
        fs::admit_descendant(&self.roots().brain_root(), &path)?;
        let text = match std::fs::symlink_metadata(&path) {
            Ok(_) => fs::read_text(&path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(error.into()),
        };
        let recipient = for_actor.map(|value| actor(value, false)).transpose()?;
        let mut selected = Vec::new();
        for line in journal_lines(&text) {
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(line).map_err(|_| {
                RuntimeError::invalid("Message journal has an invalid or interrupted line")
            })?;
            validate_message(&message)?;
            if message["seq"].as_u64().unwrap_or(0) <= since_seq {
                continue;
            }
            if message_type.is_some_and(|kind| message["type"].as_str() != Some(kind)) {
                continue;
            }
            if recipient.as_deref().is_some_and(|actor| {
                message["to_actor"].as_str() != Some(actor) && message["to_actor"] != "*"
            }) {
                continue;
            }
            selected.push(message);
        }
        Ok(selected)
    }

    fn message_path(&self, id: &str) -> Result<PathBuf> {
        if !is_uuid4(id) {
            return Err(RuntimeError::invalid("Message task id must be UUIDv4"));
        }
        Ok(self
            .roots()
            .brain_root()
            .join("control/messages")
            .join(format!("{id}.jsonl")))
    }
}

fn strings(value: &Value, key: &str) -> Result<Vec<String>> {
    value[key]
        .as_array()
        .ok_or_else(|| RuntimeError::invalid(format!("{key} must be a list")))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(String::from)
                .ok_or_else(|| RuntimeError::invalid(format!("{key} must hold strings")))
        })
        .collect()
}
fn sorted(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn collect_records(root: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    if !fs::admit_directory(root)? {
        return Ok(());
    }
    for attempt in 0..6 {
        let before = directory_identity(root)?;
        let mut files = Vec::new();
        let mut directories = Vec::new();
        let mut settled = true;
        let mut entries = std::fs::read_dir(root)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if entry.file_name().to_str().is_none() {
                return Err(RuntimeError::invalid(
                    "Project Brain source invalid-utf8-path",
                ));
            }
            let metadata = match std::fs::symlink_metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    settled = false;
                    break;
                }
                Err(error) => return Err(error.into()),
            };
            if fs::linked_at(&entry.path(), &metadata)? {
                return Err(RuntimeError::policy("Linked Project Brain inventory entry"));
            }
            if metadata.is_dir() {
                directories.push(entry.path());
            } else if entry.path().extension().is_some_and(|extension| {
                if cfg!(windows) {
                    extension.to_string_lossy().eq_ignore_ascii_case("md")
                } else {
                    extension == "md"
                }
            }) {
                if !metadata.is_file() {
                    return Err(RuntimeError::policy("Nonregular Project Brain record"));
                }
                files.push(entry.path());
            }
        }
        if settled && before == directory_identity(root)? {
            output.extend(files);
            for directory in directories {
                collect_records(&directory, output)?;
            }
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(2 * (attempt + 1)));
    }
    Err(RuntimeError::unavailable(
        "Project Brain inventory changed during traversal",
    ))
}

fn directory_identity(path: &Path) -> Result<fs::NativeIdentity> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || fs::linked_at(path, &metadata)? {
        return Err(RuntimeError::policy(
            "Project Brain inventory directory is unavailable",
        ));
    }
    fs::native_image(path)?
        .ok_or_else(|| RuntimeError::unavailable("Project Brain inventory directory disappeared"))
}

fn claimed_identity(text: &str) -> (Option<String>, Option<String>, Option<String>) {
    let mut versions = vec![text.to_string()];
    let mut ours = String::new();
    let mut theirs = String::new();
    let mut state = 0;
    let mut conflict = false;
    for line in text.split_inclusive('\n') {
        if line.starts_with("<<<<<<< ") {
            state = 1;
            conflict = true;
        } else if state == 1 && line.starts_with("||||||| ") {
            state = 2;
        } else if (state == 1 || state == 2)
            && line
                .strip_suffix('\n')
                .unwrap_or(line)
                .strip_suffix('\r')
                .unwrap_or(line.strip_suffix('\n').unwrap_or(line))
                == "======="
        {
            state = 3;
        } else if state == 3 && line.starts_with(">>>>>>> ") {
            state = 0;
        } else {
            if state == 0 || state == 1 {
                ours.push_str(line);
            }
            if state == 0 || state == 3 {
                theirs.push_str(line);
            }
        }
    }
    if conflict {
        versions.push(ours);
        versions.push(theirs);
    }
    for version in versions {
        if let Some(tail) = version.strip_prefix("---\n") {
            let raw = tail
                .split_once("\n---\n")
                .map(|(raw, _)| raw)
                .unwrap_or(tail);
            if let Ok(metadata) =
                crate::domain::json::identity_strings(raw, &["type", "id", "external_id"])
            {
                return (
                    metadata.get("type").cloned(),
                    metadata.get("id").cloned(),
                    metadata.get("external_id").cloned(),
                );
            }
        }
    }

    (None, None, None)
}

fn bullets(items: &[String], code: bool) -> String {
    if items.is_empty() {
        "- None".into()
    } else {
        items
            .iter()
            .map(|item| {
                if code {
                    format!("- `{item}`")
                } else {
                    format!("- {item}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn record_body(record: &Value) -> Result<String> {
    let progress = record["progress"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("Not started.");
    Ok(format!(
        "# {}\n\n## Goal\n{}\n\n## Progress\n{progress}\n\n## Next Steps\n{}\n\n## Files\n{}\n\n## Sources\n{}\n",
        record["title"].as_str().unwrap_or(""),
        record["goal"].as_str().unwrap_or(""),
        bullets(&strings(record, "next_steps")?, false),
        bullets(&strings(record, "files")?, true),
        bullets(&strings(record, "sources")?, true)
    ))
}

fn make_handoff(task: &Value, previous: Option<&Value>) -> Result<Value> {
    if let Some(previous) = previous
        && (!previous["id"].as_str().is_some_and(is_uuid4)
            || !previous["revision"].as_u64().is_some_and(|n| n > 0)
            || !previous["created_at"].as_str().is_some_and(valid_datetime))
    {
        return Err(RuntimeError::invalid(
            "Existing handoff identity/revision/creation timestamp is invalid",
        ));
    }
    let timestamp = utc_now();
    let revision = previous
        .and_then(|value| value["revision"].as_u64())
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| RuntimeError::invalid("Handoff revision overflow"))?;
    let mut handoff = json!({"schema_version":1,"id":previous.and_then(|value| value["id"].as_str()).map(String::from).unwrap_or_else(uuid4),"type":"handoff","task_id":task["id"],"task_revision":task["revision"],"revision":revision,"owner":task["owner"],"privacy":task["privacy"],"created_at":previous.and_then(|value| value.get("created_at")).cloned().unwrap_or(json!(timestamp)),"updated_at":timestamp,"objective":task["goal"],"current_state":render_current_state(task["progress"].as_str().unwrap_or(""), task["auto_checkpoint"].as_str()),"next_actions":task["next_steps"],"files":task["files"],"sources":task["sources"],"status":if is_terminal("task",task["status"].as_str().unwrap_or("")) { "closed" } else { "active" }});
    if let Some(phase) = task.get("phase") {
        handoff["phase"] = phase.clone();
    }
    Ok(handoff)
}

fn handoff_body(handoff: &Value) -> Result<String> {
    Ok(format!(
        "# Handoff for {}\n\n## Phase\n{}\n\n## Objective\n{}\n\n## Current State\n{}\n\n## Next Actions\n{}\n",
        handoff["task_id"].as_str().unwrap_or(""),
        handoff["phase"].as_str().unwrap_or("Not declared."),
        handoff["objective"].as_str().unwrap_or(""),
        handoff["current_state"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("Not started."),
        bullets(&strings(handoff, "next_actions")?, false)
    ))
}

pub fn actor(value: &str, broadcast: bool) -> Result<String> {
    let value = value.trim();
    if broadcast && value == "*" {
        return Ok(value.into());
    }
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte))
    {
        return Err(RuntimeError::invalid(
            "Actor must be a lowercase slug up to 64 characters",
        ));
    }
    Ok(value.into())
}

fn journal_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<_> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

pub fn validate_message(message: &Value) -> Result<()> {
    if !message.is_object()
        || message["schema_version"].as_u64() != Some(1)
        || !is_uuid4(message["id"].as_str().unwrap_or(""))
        || !is_uuid4(message["task_id"].as_str().unwrap_or(""))
    {
        return Err(RuntimeError::invalid("Invalid message schema or IDs"));
    }
    for key in ["seq", "task_revision"] {
        if !message[key].as_u64().is_some_and(|value| value > 0) {
            return Err(RuntimeError::invalid(
                "Message counters must be positive integers",
            ));
        }
    }
    if !valid_datetime(message["created_at"].as_str().unwrap_or("")) {
        return Err(RuntimeError::invalid("Invalid message timestamp"));
    }
    actor(message["from_actor"].as_str().unwrap_or(""), false)?;
    actor(message["to_actor"].as_str().unwrap_or(""), true)?;
    if !["finding", "question", "handoff", "dispatch", "completion"]
        .contains(&message["type"].as_str().unwrap_or(""))
    {
        return Err(RuntimeError::invalid("Invalid message type"));
    }
    let body = message["body"]
        .as_str()
        .filter(|body| !body.trim().is_empty())
        .ok_or_else(|| RuntimeError::invalid("Message body must not be empty"))?;
    if body.chars().count() > 8000 {
        return Err(RuntimeError::invalid(
            "Message body exceeds 8000 characters",
        ));
    }
    if strings(message, "refs")?
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(RuntimeError::invalid(
            "Message references must not be empty",
        ));
    }
    if let Some(digest) = message.get("capsule_digest") {
        let digest = digest.as_str().unwrap_or("");
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(RuntimeError::invalid("Invalid message capsule digest"));
        }
    }
    Ok(())
}
