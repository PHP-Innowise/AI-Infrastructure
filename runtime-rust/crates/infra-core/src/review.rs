//! Encounter-scoped claim review and external semantic-controller publication.
use crate::{
    ErrorKind, Result, RuntimeConfig, RuntimeContext, RuntimeError,
    domain::*,
    fs,
    native::{self, NativeService},
    result_store::{self, EvidenceStore},
    results::{self, RecordResult, ResultService, Revision},
};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub const REVIEW_PROFILE: &str = "pre-answer-memory-review-v1";
pub const REVIEW_REASON: &str =
    "Explicit memory review working stage; agent-attested, not human reviewed";
pub const SEMANTIC_PROFILE: &str = "memory-semantic-proposal-v1";
pub const PACKET_PROFILE: &str = "memory-semantic-packet-v1";
pub const VERDICT_PROFILE: &str = "memory-semantic-verdict-v1";
pub const AUTH_PROFILE: &str = "memory-semantic-controller-v1";
const LIMIT: usize = 1024 * 1024;
fn fail(code: &str) -> RuntimeError {
    RuntimeError::coded(ErrorKind::Policy, code, code)
}
fn array<'a>(value: &'a Value, code: &str) -> Result<&'a Vec<Value>> {
    value.as_array().ok_or_else(|| fail(code))
}
pub fn digest(value: &Value) -> Result<String> {
    Ok(result_store::sha(json::canonical(value)?.as_bytes()))
}
#[derive(Clone)]
pub struct ReviewService {
    pub context: RuntimeContext,
    pub results: ResultService,
}
impl ReviewService {
    pub fn new(context: RuntimeContext) -> Self {
        Self {
            results: ResultService::new(context.clone()),
            context,
        }
    }
    pub fn maintenance_mode(&self) -> Result<String> {
        Ok(std::env::var("CONTEXT_NATIVE_MEMORY_MAINTENANCE")
            .ok()
            .unwrap_or_else(|| {
                RuntimeConfig::load(&self.context.roots)
                    .ok()
                    .and_then(|config| {
                        config.value["native_memory_maintenance"]
                            .as_str()
                            .map(String::from)
                    })
                    .unwrap_or_else(|| "off".into())
            }))
    }
    fn enabled(&self) -> Result<bool> {
        Ok(self.context.mode == "governed"
            && matches!(
                self.maintenance_mode()?.as_str(),
                "review" | "review-verified"
            ))
    }
    fn encounter_path(&self, task: &str) -> Result<PathBuf> {
        let key = result_store::legacy_digest(
            &object!([
                NativeService::new(self.context.clone()).project(),
                self.context.actor,
                results::task_id(task)?
            ]),
            false,
            true,
        )?;
        let path = self
            .context
            .roots
            .brain_root()
            .join("local/native-maintenance")
            .join(format!("{key}.json"));
        fs::admit_descendant(&self.context.roots.state, &path)?;
        Ok(path)
    }
    fn plan_path(&self, task: &str, id: &str, suffix: &str) -> Result<PathBuf> {
        if !result_store::hex(id, 32) {
            return Err(fail("memory-review:invalid-plan"));
        }
        let parent = self
            .encounter_path(task)?
            .parent()
            .ok_or_else(|| fail("memory-review:unsafe-storage"))?
            .join("reviews");
        let path = parent.join(format!("{id}{suffix}.json"));
        fs::admit_descendant(&self.context.roots.state, &path)?;
        Ok(path)
    }
    fn read(&self, path: &Path, limit: usize) -> Result<Option<Value>> {
        let Some(raw) = result_store::read_bounded(path, limit)? else {
            return Ok(None);
        };
        let value = results::strict_input(&raw)?;
        json::canonical(&value)?;
        if !value.is_object() {
            return Err(fail("memory-review:invalid-json"));
        }
        Ok(Some(value))
    }
    fn write(&self, path: &Path, value: &Value, limit: usize) -> Result<()> {
        if json::sorted_pretty(value)?.len() > limit - 1 {
            return Err(fail("semantic-review:packet-too-large"));
        }
        fs::atomic_json(path, value)
    }
    fn maintenance_image(&self, name: &str) -> Result<(String, String)> {
        let pattern=regex::Regex::new(r"^(?:memory-bank/chunks/[^/]+|project-brain/dynamic/(?:findings|decisions)/[^/]+)\.md$").map_err(|_|RuntimeError::invalid("Invalid maintenance expression"))?;
        if name.len() > 1024
            || name.contains('\\')
            || name
                .split('/')
                .any(|part| part == ".." || part.is_empty() || part == ".")
            || !pattern.is_match(name)
        {
            return Err(fail("native-result:invalid-maintenance-source"));
        }
        let path = self.context.roots.resolve(name);
        fs::admit_descendant(&self.context.roots.state, &path)?;
        let raw = result_store::read_bounded(&path, 65536)?
            .ok_or_else(|| fail("native-result:invalid-maintenance-source"))?;
        let text = std::str::from_utf8(&raw)
            .map_err(|_| fail("native-result:invalid-maintenance-source"))?
            .replace("\r\n", "\n");
        Ok((text.clone(), result_store::sha(text.as_bytes())))
    }
    /// Images must be the exact admitted images of the returned capsule, not
    /// a second arbitrary filesystem scan or proof that the agent read them.
    pub fn observe_encounters(
        &self,
        capsule: &Value,
        task: &str,
        images: &BTreeMap<String, String>,
    ) -> Result<bool> {
        Ok(self.observe_inner(capsule, task, images).unwrap_or(false))
    }
    fn observe_inner(
        &self,
        capsule: &Value,
        task: &str,
        images: &BTreeMap<String, String>,
    ) -> Result<bool> {
        if !matches!(
            self.maintenance_mode()?.as_str(),
            "encountered" | "review" | "review-verified"
        ) || capsule["manifest"].is_null()
            || capsule["manifest_publication"] == "unconfirmed"
        {
            return Ok(false);
        }
        let paths = capsule["semantic"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| row["path"].as_str())
            .collect::<BTreeSet<_>>();
        let mut rows = Vec::new();
        for (name, image) in images {
            if !paths.contains(name.as_str()) {
                continue;
            }
            if let Ok((_, hash)) = self.maintenance_image(name)
                && result_store::sha(image.as_bytes()) == hash
            {
                rows.push(object!({"path":name,"sha256":hash}));
            }
            if rows.len() == 3 {
                break;
            }
        }
        if rows.is_empty() {
            return Ok(false);
        }
        let brain = self.results.brain()?;
        let _guard = brain.lock()?;
        let path = self.encounter_path(task)?;
        self.write(&path,&object!({"profile":"native-memory-encounters-v1","at":native::seconds(),"task_uuid":capsule["task_uuid"],"rows":rows}),4096)?;
        self.prune_local(
            path.parent()
                .ok_or_else(|| fail("memory-review:unsafe-storage"))?,
            128,
            &path,
        );
        Ok(true)
    }
    fn prune_local(&self, directory: &Path, keep: usize, current: &Path) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        let mut files = entries
            .flatten()
            .filter(|entry| {
                entry.path() != current
                    && entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                    && std::fs::symlink_metadata(entry.path()).is_ok_and(|metadata| {
                        metadata.is_file()
                            && !fs::linked_at(&entry.path(), &metadata).unwrap_or(true)
                    })
            })
            .collect::<Vec<_>>();
        files.sort_by_key(|entry| {
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
        });
        let excess = files.len().saturating_sub(keep.saturating_sub(1));
        for file in files.into_iter().take(excess) {
            let _ = std::fs::remove_file(file.path());
        }
    }
    pub fn candidates(&self, task: &str, uuid: &str) -> Result<Vec<Value>> {
        Ok(self.candidates_inner(task, uuid).unwrap_or_default())
    }
    fn candidates_inner(&self, task: &str, uuid: &str) -> Result<Vec<Value>> {
        if !matches!(
            self.maintenance_mode()?.as_str(),
            "encountered" | "review" | "review-verified"
        ) {
            return Ok(vec![]);
        }
        let Some(packet) = self.read(&self.encounter_path(task)?, 4096)? else {
            return Ok(vec![]);
        };
        let age = native::seconds() - packet["at"].as_f64().unwrap_or(f64::NEG_INFINITY);
        if !result_store::exact(&packet, &["profile", "at", "task_uuid", "rows"])
            || packet["profile"] != "native-memory-encounters-v1"
            || !(0.0..=3600.0).contains(&age)
            || !packet["task_uuid"].is_null() && packet["task_uuid"] != uuid
        {
            return Ok(vec![]);
        }
        let config = RuntimeConfig::load(&self.context.roots)?;
        let mut output = Vec::new();
        let mut seen = BTreeSet::new();
        for row in packet["rows"].as_array().into_iter().flatten().take(3) {
            let candidate = (|| -> Result<Value> {
                if !result_store::exact(row, &["path", "sha256"]) {
                    return Err(fail("memory-review:invalid-encounter"));
                }
                let mut path = row["path"]
                    .as_str()
                    .ok_or_else(|| fail("memory-review:invalid-encounter"))?
                    .to_string();
                let (mut content, mut hash) = self.maintenance_image(&path)?;
                if row["sha256"] != hash || sensitive_label(&content)?.is_some() {
                    return Err(fail("memory-review:encounter-changed"));
                }
                let (mut metadata, _) = parse_markdown_record(&content)?;
                if path.starts_with("memory-bank/") {
                    if metadata["status"] != "active"
                        || crate::index::memory_temporal_reason(&metadata).is_some()
                    {
                        return Err(fail("memory-review:encounter-not-current"));
                    }
                    let links = metadata["source_digests"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|pin| {
                            pin["path"].as_str().is_some_and(|path| {
                                path.starts_with("project-brain/dynamic/findings/")
                                    || path.starts_with("project-brain/dynamic/decisions/")
                            })
                        })
                        .collect::<Vec<_>>();
                    if links.len() != 1 {
                        return Err(fail("memory-review:ambiguous-origin"));
                    }
                    path = links[0]["path"].as_str().unwrap_or("").into();
                    (content, hash) = self.maintenance_image(&path)?;
                    if links[0]["sha256"] != hash {
                        return Err(fail("memory-review:origin-changed"));
                    }
                    (metadata, _) = parse_markdown_record(&content)?;
                }
                validate_record(&metadata, false)?;
                let id = metadata["id"].as_str().unwrap_or("");
                let kind = metadata["type"].as_str().unwrap_or("");
                let closed = if kind == "finding" {
                    "resolved"
                } else if kind == "decision" {
                    "accepted"
                } else {
                    return Err(fail("memory-review:unsupported-kind"));
                };
                if metadata["status"] != closed
                    || metadata["authority"] != "verified"
                    || record_attestation(&metadata) != "agent"
                    || !metadata["superseded_by"].is_null()
                    || !metadata["authorized_owners"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|value| value.as_str() == Some(self.context.actor.as_str()))
                    || !config
                        .strings("allowed_authority")?
                        .iter()
                        .any(|value| metadata["authority"].as_str() == Some(value.as_str()))
                    || !config
                        .strings("allowed_privacy")?
                        .iter()
                        .any(|value| metadata["privacy"].as_str() == Some(value.as_str()))
                    || !matches!(metadata["privacy"].as_str(), Some("public" | "team"))
                    || !config.strings("owners")?.iter().any(|owner| {
                        owner == "*" || metadata["owner"].as_str() == Some(owner.as_str())
                    })
                    || sensitive_label(&content)?.is_some()
                {
                    return Err(fail("memory-review:origin-not-admitted"));
                }
                if !seen.insert(id.to_string()) {
                    return Err(fail("memory-review:duplicate-origin"));
                }
                let sources = results::strings(&metadata["sources"], "sources", 10, 1024)?;
                let pins = self.results.checked_fingerprints(&sources)?;
                if pins
                    == results::canonical_image(
                        metadata["source_fingerprints"]
                            .as_array()
                            .map(Vec::as_slice)
                            .unwrap_or(&[]),
                    )?
                {
                    return Err(fail("memory-review:sources-unchanged"));
                }
                let title = results::text(&metadata["title"], "title", 160)?;
                let consequence = results::text(
                    metadata
                        .get("progress")
                        .filter(|value| value.as_str().is_some_and(|value| !value.is_empty()))
                        .unwrap_or(&metadata["goal"]),
                    "consequence",
                    1000,
                )?;
                let value = object!({"type":kind,"title":title,"consequence":consequence,"sources":sources,"record_id":id,"expected_revision":metadata["revision"],"_record_path":path,"_record_sha256":hash,"_source_image":pins});
                if json::python_default(&value, false, false, true)?
                    .chars()
                    .count()
                    > 3000
                {
                    return Err(fail("memory-review:candidate-too-large"));
                }
                Ok(value)
            })();
            if let Ok(candidate) = candidate {
                output.push(candidate);
            }
        }
        Ok(output)
    }
    pub fn notice(&self, task: &str) -> Result<Option<String>> {
        if !self.enabled()? {
            return Ok(None);
        }
        let record = self.results.task(task, false)?.record;
        if matches!(record["status"].as_str(), Some("completed" | "cancelled"))
            || self
                .read(&self.encounter_path(task)?, 4096)?
                .is_none_or(|value| value["rows"].as_array().is_none_or(Vec::is_empty))
        {
            return Ok(None);
        }
        let mode = if self.maintenance_mode()? == "review-verified" {
            "stages a proposal ONLY; independent verification/publication are performed by the external controller after the session. Native closing save uses learnings=[]; never claim publication"
        } else {
            "applies checked references; hashes alone are not verification"
        };
        Ok(Some(format!(
            "MEMORY_REVIEW_STAGE: after implementing and testing, BEFORE your final answer, run memory-review --task-id {} --json. Read every offered claim/current source and explicitly confirm, correct or defer every item. This command {mode}.",
            native::quote(task)
        )))
    }
    pub fn plan(&self, task: &str) -> Result<Value> {
        if !self.enabled()? {
            return Err(fail("memory-review:disabled"));
        }
        let task_id = results::task_id(task)?;
        let brain = self.results.brain()?;
        let _guard = brain.lock()?;
        let task = self.results.task(&task_id, false)?.record;
        if matches!(task["status"].as_str(), Some("completed" | "cancelled")) {
            return Err(fail("memory-review:task-closed"));
        }
        let candidates = self.candidates(&task_id, task["id"].as_str().unwrap_or(""))?;
        if candidates.is_empty() {
            return Ok(
                object!({"profile":REVIEW_PROFILE,"state":"unavailable","plan_id":null,"records":[],"reason":"No safely admitted changed-source encounter; no claim was verified."}),
            );
        }
        let id = uuid4().replace('-', "");
        let mut plan = object!({"profile":REVIEW_PROFILE,"plan_id":id,"owner":self.context.actor,"project":NativeService::new(self.context.clone()).project(),"task_id":task_id,"task_uuid":task["id"],"task_revision":task["revision"],"at":native::seconds(),"scope":self.results.scope(REVIEW_REASON,"agent")?,"adapter":native::adapter(),"checkout":native::checkout(&self.context)?,"records":candidates});
        if self.maintenance_mode()? == "review-verified" {
            plan["verification_required"] = true.into();
        }
        let destination = self.plan_path(&task_id, &id, "")?;
        self.write(&destination, &plan, 65536)?;
        self.prune_local(
            destination
                .parent()
                .ok_or_else(|| fail("memory-review:unsafe-storage"))?,
            256,
            &destination,
        );
        let records = plan["records"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|row| {
                let mut row = row.clone();
                if let Some(map) = row.as_object_mut() {
                    map.retain(|key, _| !key.starts_with('_'));
                }
                row
            })
            .collect::<Vec<_>>();
        Ok(
            object!({"profile":REVIEW_PROFILE,"state":"review-required","plan_id":id,"verification_required":plan["verification_required"]==true,"task_id":task_id,"task_uuid":task["id"],"records":records,"instructions":"Open every claim and necessary current source. Submit one confirm/correct/defer decision per record. Confirm/correct require top-level verified=true and checked source_fingerprints; correct also supplies consequence. Defer supplies reason. Preserve exact type/title/sources.","submit_command":format!("memory-review --task-id {} --input - --json",native::quote(&task_id)),"submit_schema":{"plan_id":id,"verified":true,"decisions":"one object per offered record"},"decision_fields":{"confirm":["record_id","action","source_fingerprints"],"correct":["record_id","action","source_fingerprints","consequence"],"defer":["record_id","action","reason"]}}),
        )
    }
    fn validate_plan(
        &self,
        plan: &Value,
        task: &str,
        id: &str,
        uuid: &str,
    ) -> Result<BTreeMap<String, Value>> {
        let mut fields = vec![
            "profile",
            "plan_id",
            "owner",
            "project",
            "task_id",
            "task_uuid",
            "task_revision",
            "at",
            "scope",
            "adapter",
            "checkout",
            "records",
        ];
        if plan["verification_required"] == true {
            fields.push("verification_required");
        }
        if !result_store::exact(plan, &fields)
            || plan["profile"] != REVIEW_PROFILE
            || plan["plan_id"] != id
            || plan["owner"] != self.context.actor
            || plan["project"] != NativeService::new(self.context.clone()).project()
            || plan["task_id"] != task
            || plan["task_uuid"] != uuid
            || !is_uuid4(uuid)
            || plan["task_revision"]
                .as_u64()
                .is_none_or(|revision| revision == 0)
            || plan["scope"] != self.results.scope(REVIEW_REASON, "agent")?
            || plan["adapter"] != native::adapter()
            || plan["checkout"] != native::checkout(&self.context)?
        {
            return Err(fail("memory-review:plan-unavailable-or-scope-changed"));
        }
        let rows = array(&plan["records"], "memory-review:plan-unavailable")?;
        if rows.is_empty() || rows.len() > 3 {
            return Err(fail("memory-review:plan-unavailable"));
        }
        let mut records = BTreeMap::new();
        for row in rows {
            if !result_store::exact(
                row,
                &[
                    "type",
                    "title",
                    "consequence",
                    "sources",
                    "record_id",
                    "expected_revision",
                    "_record_path",
                    "_record_sha256",
                    "_source_image",
                ],
            ) {
                return Err(fail("memory-review:plan-unavailable"));
            }
            let key = row["record_id"]
                .as_str()
                .filter(|id| is_uuid4(id))
                .ok_or_else(|| fail("memory-review:plan-unavailable"))?;
            if records.insert(key.into(), row.clone()).is_some() {
                return Err(fail("memory-review:plan-unavailable"));
            }
            let mut learning = row.clone();
            for key in ["_record_path", "_record_sha256", "_source_image"] {
                learning
                    .as_object_mut()
                    .ok_or_else(|| fail("memory-review:plan-unavailable"))?
                    .remove(key);
            }
            learning["source_fingerprints"] = row["_source_image"].clone();
            self.results
                .normalize(&object!({"verified":true,"learnings":[learning]}), false)?;
        }
        Ok(records)
    }
    pub fn apply(&self, task: &str, input: &Value) -> Result<Value> {
        if !self.enabled()? {
            return Err(fail("memory-review:disabled"));
        }
        let fields = input
            .as_object()
            .ok_or_else(|| fail("memory-review:invalid-decisions"))?;
        if !fields.contains_key("plan_id")
            || !fields.contains_key("decisions")
            || fields
                .keys()
                .any(|key| !["plan_id", "decisions", "verified"].contains(&key.as_str()))
            || fields
                .get("verified")
                .is_some_and(|value| !value.is_boolean())
        {
            return Err(fail("memory-review:invalid-decisions"));
        }
        let id = input["plan_id"]
            .as_str()
            .ok_or_else(|| fail("memory-review:invalid-plan"))?;
        let brain = self.results.brain()?;
        let _guard = brain.lock()?;
        let current = self.results.task(task, false)?.record;
        let plan = self
            .read(&self.plan_path(task, id, "")?, 65536)?
            .ok_or_else(|| fail("memory-review:plan-unavailable"))?;
        let offered = self.validate_plan(&plan, task, id, current["id"].as_str().unwrap_or(""))?;
        if self.maintenance_mode()? == "review-verified" && plan["verification_required"] != true {
            return Err(fail("memory-review:verification-required"));
        }
        let decisions = array(&input["decisions"], "memory-review:decide-every-record")?;
        if decisions.len() != offered.len() {
            return Err(fail("memory-review:decide-every-record"));
        }
        let (mut checked, mut deferred, mut seen) = (Vec::new(), Vec::new(), BTreeSet::new());
        for decision in decisions {
            let key = decision["record_id"]
                .as_str()
                .filter(|key| offered.contains_key(*key))
                .ok_or_else(|| fail("memory-review:invalid-or-duplicate-record"))?;
            if !seen.insert(key) {
                return Err(fail("memory-review:invalid-or-duplicate-record"));
            }
            let row = &offered[key];
            let action = decision["action"].as_str().unwrap_or("");
            if action == "defer" {
                if !result_store::exact(decision, &["record_id", "action", "reason"]) {
                    return Err(fail("memory-review:invalid-defer"));
                }
                deferred.push(object!({"record_id":key,"reason":results::text(&decision["reason"],"defer reason",300)?}));
                continue;
            }
            let required = if action == "correct" {
                vec!["record_id", "action", "source_fingerprints", "consequence"]
            } else {
                vec!["record_id", "action", "source_fingerprints"]
            };
            if !matches!(action, "confirm" | "correct")
                || !result_store::exact(decision, &required)
                || input["verified"] != true
            {
                return Err(fail("memory-review:checked-agent-attestation-required"));
            }
            let pins = results::canonical_image(array(
                &decision["source_fingerprints"],
                "memory-review:checked-image-does-not-match-plan",
            )?)?;
            if object!(pins) != row["_source_image"] {
                return Err(fail("memory-review:checked-image-does-not-match-plan"));
            }
            let mut learning = object!({"type":row["type"],"title":row["title"],"consequence":row["consequence"],"sources":row["sources"],"record_id":row["record_id"],"expected_revision":row["expected_revision"],"source_fingerprints":pins});
            if action == "correct" {
                learning["consequence"] =
                    results::text(&decision["consequence"], "consequence", 1000)?.into();
            }
            checked.push(learning);
        }
        let payload = object!({"progress":null,"verified":true,"learnings":checked});
        let incoming = result_store::legacy_digest(input, true, false)?;
        let destination = self.plan_path(task, id, "-result")?;
        let receipt = self.read(&destination, 65536)?;
        if receipt.as_ref().is_some_and(|receipt| {
            !result_store::exact(
                receipt,
                &["profile", "plan_id", "input_sha256", "result_id", "state"],
            ) || receipt["input_sha256"] != incoming
                || receipt["profile"] != REVIEW_PROFILE
                || receipt["plan_id"] != id
                || receipt["result_id"] != format!("review-{id}")
                || !matches!(
                    receipt["state"].as_str(),
                    Some("started" | "applied" | "held" | "deferred" | "proposed")
                )
        }) {
            return Err(fail(
                "memory-review:decisions-already-frozen-or-unavailable",
            ));
        }
        let normalized = if checked.is_empty() {
            None
        } else {
            Some(self.results.normalize(&payload, false)?)
        };
        let result_id = format!("review-{id}");
        let evidence = EvidenceStore::new(self.context.clone());
        let known = if let Some(normalized) = &normalized {
            evidence
                .attempt(
                    current["id"].as_str().unwrap_or(""),
                    &result_id,
                    &results::request_digest(current["id"].as_str().unwrap_or(""), normalized)?,
                )?
                .is_some_and(|proof| {
                    proof.owner == self.context.actor && proof.attestation == "agent"
                })
        } else {
            false
        };
        if !known
            && !receipt
                .as_ref()
                .is_some_and(|receipt| checked.is_empty() && receipt["state"] == "deferred")
        {
            let age = native::seconds() - plan["at"].as_f64().unwrap_or(f64::NEG_INFINITY);
            if !(0.0..=3600.0).contains(&age)
                || current["revision"] != plan["task_revision"]
                || matches!(current["status"].as_str(), Some("completed" | "cancelled"))
            {
                return Err(fail("memory-review:plan-expired-or-task-changed"));
            }
            for learning in &checked {
                let row = &offered[learning["record_id"].as_str().unwrap_or("")];
                if self
                    .maintenance_image(row["_record_path"].as_str().unwrap_or(""))?
                    .1
                    != row["_record_sha256"].as_str().unwrap_or("")
                    || object!(self.results.checked_fingerprints(&results::strings(
                        &row["sources"],
                        "sources",
                        10,
                        1024
                    )?)?)
                        != row["_source_image"]
                {
                    return Err(fail("memory-review:claim-or-source-changed"));
                }
            }
        }
        let mut receipt=receipt.unwrap_or_else(||object!({"profile":REVIEW_PROFILE,"plan_id":id,"input_sha256":incoming,"result_id":result_id,"state":"started"}));
        if receipt["state"] == "started" {
            self.write(&destination, &receipt, 65536)?;
        }
        let reply = if checked.is_empty() {
            if matches!(
                receipt["state"].as_str(),
                Some("applied" | "held" | "proposed")
            ) {
                return Err(fail("memory-review:receipt-state-does-not-match-decisions"));
            }
            object!({"profile":REVIEW_PROFILE,"state":"deferred","plan_id":id,"deferred":deferred,"records":[],"attestation":"not-performed","replayed":receipt["state"]=="deferred"})
        } else if plan["verification_required"] == true {
            self.propose_semantic(&plan, &payload, &incoming, &object!(deferred))?
        } else {
            if receipt["state"] == "deferred" {
                return Err(fail("memory-review:receipt-state-does-not-match-decisions"));
            }
            let saved = self.results.record(RecordResult {
                task_id: current["id"].as_str().unwrap_or("").into(),
                result_id,
                revision: Revision::Exact(plan["task_revision"].as_u64().unwrap_or(0)),
                data: payload,
                reason: REVIEW_REASON.into(),
                attestation: "agent".into(),
                prepared: plan["scope"].as_str().map(String::from),
                replay_only: false,
            })?;
            let held = saved["records"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|row| !matches!(row["state"].as_str(), Some("updated" | "existing")))
                || ["failed", "blocked"].iter().any(|field| {
                    saved["promotion"][field]
                        .as_array()
                        .is_some_and(|rows| !rows.is_empty())
                });
            object!({"profile":REVIEW_PROFILE,"state":if held{"held"}else{"applied"},"plan_id":id,"deferred":deferred,"attestation":"agent","replayed":saved["replayed"],"result":saved})
        };
        receipt["state"] = if reply["state"] == "proposal-staged" {
            "proposed".into()
        } else {
            reply["state"].clone()
        };
        self.write(&destination, &receipt, 65536)?;
        Ok(reply)
    }
    fn propose_semantic(
        &self,
        plan: &Value,
        payload: &Value,
        incoming: &str,
        deferred: &Value,
    ) -> Result<Value> {
        let normalized = self.results.normalize(payload, false)?;
        if normalized
            .learnings
            .iter()
            .any(|row| results::collapse(&row.consequence) != row.consequence)
        {
            return Err(fail("semantic-review:rendering-would-change-claim"));
        }
        let mut proposal = object!({"profile":SEMANTIC_PROFILE,"proposal_id":plan["plan_id"],"plan":plan,"input_sha256":incoming,"payload":{"progress":null,"verified":true,"learnings":normalized.learnings},"deferred":deferred});
        let hash = digest(&proposal)?;
        proposal["sha256"] = hash.clone().into();
        let path = self.plan_path(
            plan["task_id"].as_str().unwrap_or(""),
            plan["plan_id"].as_str().unwrap_or(""),
            "-proposal",
        )?;
        if let Some(previous) = self.read(&path, LIMIT)? {
            if previous != proposal {
                return Err(fail(
                    "semantic-review:proposal-already-frozen-or-unavailable",
                ));
            }
        } else {
            self.write(&path, &proposal, LIMIT)?;
        }
        Ok(
            object!({"profile":SEMANTIC_PROFILE,"state":"proposal-staged","proposal_id":plan["plan_id"],"proposal_sha256":hash,"applied":false,"verification_required":true,"attestation":"agent-proposal","deferred":deferred,"next":"External controller must independently verify before publication; native save uses learnings=[]."}),
        )
    }
    fn proposal(&self, task: &str, id: &str, expected: &str) -> Result<Value> {
        let proposal = self
            .read(&self.plan_path(task, id, "-proposal")?, LIMIT)?
            .ok_or_else(|| fail("semantic-review:proposal-unavailable"))?;
        if !result_store::exact(
            &proposal,
            &[
                "profile",
                "proposal_id",
                "plan",
                "input_sha256",
                "payload",
                "deferred",
                "sha256",
            ],
        ) || proposal["profile"] != SEMANTIC_PROFILE
            || proposal["proposal_id"] != id
            || proposal["sha256"] != expected
            || !result_store::hex(expected, 64)
        {
            return Err(fail("semantic-review:proposal-unavailable"));
        }
        let mut unsigned = proposal.clone();
        unsigned
            .as_object_mut()
            .ok_or_else(|| fail("semantic-review:proposal-unavailable"))?
            .remove("sha256");
        if digest(&unsigned)? != expected || proposal["plan"]["verification_required"] != true {
            return Err(fail("semantic-review:proposal-scope-changed"));
        }
        let offered = self.validate_plan(
            &proposal["plan"],
            task,
            id,
            proposal["plan"]["task_uuid"].as_str().unwrap_or(""),
        )?;
        if !result_store::exact(&proposal["payload"], &["progress", "verified", "learnings"])
            || !proposal["payload"]["progress"].is_null()
            || proposal["payload"]["verified"] != true
            || !result_store::hex(proposal["input_sha256"].as_str().unwrap_or(""), 64)
        {
            return Err(fail("semantic-review:invalid-proposal"));
        }
        let normalized = self.results.normalize(&proposal["payload"], false)?;
        let mut seen = BTreeSet::new();
        for learning in &normalized.learnings {
            let key = learning
                .record_id
                .as_deref()
                .filter(|id| offered.contains_key(*id))
                .ok_or_else(|| fail("semantic-review:foreign-or-duplicate-reference"))?;
            if !seen.insert(key.to_string()) {
                return Err(fail("semantic-review:foreign-or-duplicate-reference"));
            }
            let previous = &offered[key];
            let current = serde_json::to_value(learning)?;
            if ["type", "title", "sources", "expected_revision"]
                .iter()
                .any(|field| current[field] != previous[field])
                || current["source_fingerprints"] != previous["_source_image"]
                || results::collapse(&learning.consequence) != learning.consequence
            {
                return Err(fail("semantic-review:reference-changed"));
            }
        }
        for row in array(&proposal["deferred"], "semantic-review:invalid-deferral")? {
            let key = row["record_id"]
                .as_str()
                .filter(|id| offered.contains_key(*id))
                .ok_or_else(|| fail("semantic-review:invalid-deferral"))?;
            if !result_store::exact(row, &["record_id", "reason"]) || !seen.insert(key.to_string())
            {
                return Err(fail("semantic-review:invalid-deferral"));
            }
            results::text(&row["reason"], "defer reason", 300)?;
        }
        if seen != offered.keys().cloned().collect() || normalized.learnings.is_empty() {
            return Err(fail("semantic-review:incomplete-proposal"));
        }
        Ok(proposal)
    }
    fn current(&self, proposal: &Value, task: &Value) -> Result<()> {
        let plan = &proposal["plan"];
        let age = native::seconds() - plan["at"].as_f64().unwrap_or(f64::NEG_INFINITY);
        if task["id"] != plan["task_uuid"]
            || matches!(task["status"].as_str(), Some("completed" | "cancelled"))
        {
            return Err(fail("semantic-review:task-changed"));
        }
        if !(0.0..=3600.0).contains(&age) {
            return Err(fail("semantic-review:proposal-expired"));
        }
        for learning in array(
            &proposal["payload"]["learnings"],
            "semantic-review:invalid-proposal",
        )? {
            let row = array(&plan["records"], "semantic-review:invalid-proposal")?
                .iter()
                .find(|row| row["record_id"] == learning["record_id"])
                .ok_or_else(|| fail("semantic-review:foreign-reference"))?;
            if self
                .maintenance_image(row["_record_path"].as_str().unwrap_or(""))?
                .1
                != row["_record_sha256"].as_str().unwrap_or("")
                || object!(self.results.checked_fingerprints(&results::strings(
                    &row["sources"],
                    "sources",
                    10,
                    1024
                )?)?)
                    != learning["source_fingerprints"]
            {
                return Err(fail("semantic-review:record-or-source-changed"));
            }
        }
        Ok(())
    }
    fn sources(&self, proposal: &Value) -> Result<Value> {
        let mut pins = BTreeMap::new();
        for row in array(
            &proposal["payload"]["learnings"],
            "semantic-review:invalid-proposal",
        )? {
            for pin in array(
                &row["source_fingerprints"],
                "semantic-review:invalid-proposal",
            )? {
                pins.insert(
                    pin["path"].as_str().unwrap_or("").to_string(),
                    pin["sha256"].as_str().unwrap_or("").to_string(),
                );
            }
        }
        let (mut rows, mut total) = (Vec::new(), 0);
        for (path, expected) in pins {
            self.results.source(&path, true)?;
            let raw = result_store::read_bounded(&self.context.roots.resolve(&path), 262144)?
                .ok_or_else(|| fail("semantic-review:source-unavailable"))?;
            total += raw.len();
            if total > 524288 {
                return Err(fail("semantic-review:source-budget"));
            }
            let text = std::str::from_utf8(&raw)
                .map_err(|_| fail("semantic-review:source-unavailable"))?;
            if text.contains('\0')
                || sensitive_label(text)?.is_some()
                || memory_kernel_probe::fingerprint_bytes(&raw) != expected
            {
                return Err(fail("semantic-review:source-unavailable"));
            }
            rows.push(object!({"path":path,"sha256":expected,"raw_sha256":result_store::sha(&raw),"text":text}));
        }
        Ok(object!(rows))
    }
    pub fn semantic_export(&self, task: &str, id: &str, expected: &str) -> Result<Value> {
        if self.maintenance_mode()? != "review-verified" {
            return Err(fail("semantic-review:disabled"));
        }
        let brain = self.results.brain()?;
        let _guard = brain.lock()?;
        let proposal = self.proposal(task, id, expected)?;
        let current = self.results.task(task, false)?;
        self.current(&proposal, &current.record)?;
        let sources = self.sources(&proposal)?;
        let mut records = Vec::new();
        let offered = array(&proposal["plan"]["records"], "semantic-review:invalid-plan")?;
        let mut chunks = Vec::new();
        let directory = self.context.roots.state.join("memory-bank/chunks");
        if directory.is_dir() {
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                if entry.path().extension().and_then(|value| value.to_str()) == Some("md") {
                    chunks.push(format!(
                        "memory-bank/chunks/{}",
                        entry.file_name().to_string_lossy()
                    ));
                    if chunks.len() > 2000 {
                        return Err(fail("semantic-review:chunk-budget"));
                    }
                }
            }
            chunks.sort();
        }
        for learning in array(
            &proposal["payload"]["learnings"],
            "semantic-review:invalid-proposal",
        )? {
            let previous = offered
                .iter()
                .find(|row| row["record_id"] == learning["record_id"])
                .ok_or_else(|| fail("semantic-review:foreign-reference"))?;
            let mut memories = Vec::new();
            for chunk in &chunks {
                let (text, hash) = self.maintenance_image(chunk)?;
                let (metadata, body) = parse_markdown_record(&text)?;
                let links = metadata["sources"]
                    .as_array()
                    .filter(|rows| rows.len() <= 100 && rows.iter().all(Value::is_string))
                    .ok_or_else(|| fail("semantic-review:memory-unavailable"))?;
                if metadata["status"] == "active"
                    && links.iter().any(|value| value == &previous["_record_path"])
                {
                    let config = RuntimeConfig::load(&self.context.roots)?;
                    if !matches!(metadata["privacy"].as_str(), Some("public" | "team"))
                        || !config
                            .strings("allowed_privacy")?
                            .iter()
                            .any(|value| metadata["privacy"].as_str() == Some(value.as_str()))
                        || !config
                            .strings("allowed_authority")?
                            .iter()
                            .any(|value| metadata["authority"].as_str() == Some(value.as_str()))
                        || crate::index::memory_temporal_reason(&metadata).is_some()
                        || sensitive_label(&text)?.is_some()
                    {
                        return Err(fail("semantic-review:memory-not-admitted"));
                    }
                    memories.push(
                        object!({"id":metadata["id"],"path":chunk,"body":body,"sha256":hash}),
                    );
                }
            }
            records.push(object!({"record_id":learning["record_id"],"title":learning["title"],"old_consequence":previous["consequence"],"proposed_consequence":learning["consequence"],"old_clauses":clauses(previous["consequence"].as_str().unwrap_or(""),"old")?,"new_clauses":clauses(learning["consequence"].as_str().unwrap_or(""),"new")?,"sources":learning["sources"],"old_memories":memories}));
        }
        let mut packet = object!({"profile":PACKET_PROFILE,"proposal_id":id,"proposal_sha256":expected,"task_id":task,"task_uuid":current.record["id"],"owner":self.context.actor,"project":NativeService::new(self.context.clone()).project(),"task_revision":current.record["revision"],"task_sha256":result_store::sha(&fs::read_regular(&current.path)?),"source_snapshot_sha256":digest(&sources)?,"sources":sources,"records":records});
        packet["sha256"] = digest(&packet)?.into();
        if json::serialize(&packet, false, false, true, false)?.len() > LIMIT {
            return Err(fail("semantic-review:packet-too-large"));
        }
        Ok(packet)
    }
    pub fn controller_input(&self, path: &Path) -> Result<Value> {
        if !path.is_absolute() {
            return Err(fail("semantic-review:external-input-required"));
        }
        let resolved = std::fs::canonicalize(path)?;
        for root in [
            &self.context.roots.project,
            &self.context.roots.state,
            &self.context.roots.tooling,
        ] {
            if resolved.starts_with(fs::canonical_or_missing(root)?) {
                return Err(fail("semantic-review:external-input-required"));
            }
        }
        self.read(path, LIMIT)?
            .ok_or_else(|| fail("semantic-review:external-input-unavailable"))
    }
    pub fn semantic_apply(
        &self,
        task: &str,
        id: &str,
        expected: &str,
        packet: &Value,
        authorization: &Value,
    ) -> Result<Value> {
        if self.maintenance_mode()? != "review-verified" {
            return Err(fail("semantic-review:disabled"));
        }
        let brain = self.results.brain()?;
        let _guard = brain.lock()?;
        let proposal = self.proposal(task, id, expected)?;
        check_packet(packet, &proposal)?;
        if !result_store::exact(
            authorization,
            &[
                "profile",
                "packet_sha256",
                "primary_request",
                "reviewer_session",
                "verdict",
            ],
        ) || authorization["profile"] != AUTH_PROFILE
            || authorization["packet_sha256"] != packet["sha256"]
            || !result_store::hex(authorization["primary_request"].as_str().unwrap_or(""), 64)
            || authorization["reviewer_session"]
                .as_str()
                .is_none_or(|value| {
                    value.is_empty()
                        || value.len() > 160
                        || !value
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
                })
        {
            return Err(fail("semantic-review:controller-authorization-unavailable"));
        }
        let assessment = check_verdict(packet, &authorization["verdict"])?;
        let auth_hash = digest(authorization)?;
        let binding = object!({"profile":AUTH_PROFILE,"packet_sha256":packet["sha256"],"authorization_sha256":auth_hash});
        let destination = self.plan_path(task, id, "-authorization")?;
        let previous = self.read(&destination, 65536)?;
        if previous.as_ref().is_some_and(|value| value != &binding) {
            return Err(fail(
                "semantic-review:authorization-already-frozen-or-unavailable",
            ));
        }
        let normalized = self.results.normalize(&proposal["payload"], false)?;
        let uuid = proposal["plan"]["task_uuid"].as_str().unwrap_or("");
        let request = results::request_digest(uuid, &normalized)?;
        let result_id = format!("semantic-{}", &auth_hash[..48]);
        let evidence = EvidenceStore::new(self.context.clone());
        let (receipt, _) = evidence.receipt(uuid, &result_id, &request, &[])?;
        let proof = evidence.attempt(uuid, &result_id, &request)?;
        let known = receipt.is_some()
            || proof.as_ref().is_some_and(|proof| {
                proof.owner == self.context.actor && proof.attestation == "agent"
            });
        if known && previous.as_ref() != Some(&binding) {
            return Err(fail("semantic-review:authorization-unavailable"));
        }
        if !(receipt.is_some()
            || known
                && proof
                    .as_ref()
                    .is_some_and(|proof| proof.state == "complete"))
        {
            let native = NativeService::new(self.context.clone());
            let state = native
                .read_state(
                    authorization["primary_request"].as_str().unwrap_or(""),
                    None,
                )?
                .ok_or_else(|| fail("semantic-review:primary-unfinished-or-same-session"))?;
            if state.phase != "saved"
                || state.owner != self.context.actor
                || state.task_id != task
                || state.task_uuid.as_deref() != Some(uuid)
                || !native.proven(&state)?
                || state.session
                    == result_store::sha(
                        authorization["reviewer_session"]
                            .as_str()
                            .unwrap_or("")
                            .as_bytes(),
                    )
            {
                return Err(fail("semantic-review:primary-unfinished-or-same-session"));
            }
        }
        if assessment["publish"] != true {
            if previous.is_none() {
                self.write(&destination, &binding, LIMIT)?;
            }
            return Ok(
                object!({"profile":AUTH_PROFILE,"state":assessment["state"],"applied":false,"attestation":"not-performed"}),
            );
        }
        if !known {
            let task_image = self.results.task(task, false)?;
            self.current(&proposal, &task_image.record)?;
            if packet["task_uuid"] != task_image.record["id"]
                || packet["task_revision"] != task_image.record["revision"]
                || packet["task_sha256"] != result_store::sha(&fs::read_regular(&task_image.path)?)
                || self.sources(&proposal)? != packet["sources"]
                || self.semantic_export(task, id, expected)? != *packet
            {
                return Err(fail("semantic-review:publication-frontier-changed"));
            }
        }
        if previous.is_none() {
            self.write(&destination, &binding, LIMIT)?;
        }
        let result = self.results.record(RecordResult {
            task_id: uuid.into(),
            result_id: result_id.clone(),
            revision: Revision::Exact(packet["task_revision"].as_u64().unwrap_or(0)),
            data: proposal["payload"].clone(),
            reason: REVIEW_REASON.into(),
            attestation: "agent".into(),
            prepared: proposal["plan"]["scope"].as_str().map(String::from),
            replay_only: false,
        })?;
        let held = result["records"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|row| !matches!(row["state"].as_str(), Some("updated" | "existing")))
            || ["failed", "blocked"].iter().any(|field| {
                result["promotion"][field]
                    .as_array()
                    .is_some_and(|rows| !rows.is_empty())
            });
        Ok(
            object!({"profile":AUTH_PROFILE,"state":if held{"held"}else{"applied"},"applied":!held,"attestation":"agent","reviewer_session":authorization["reviewer_session"],"result_id":result_id,"proposal_sha256":expected,"packet_sha256":packet["sha256"],"result":result}),
        )
    }
}
pub fn clauses(text: &str, prefix: &str) -> Result<Value> {
    if text.is_empty() {
        return Err(fail("semantic-review:empty-claim"));
    }
    let mut starts = BTreeSet::from([0, text.len()]);
    let chars = text.char_indices().collect::<Vec<_>>();
    for (index, (offset, character)) in chars.iter().enumerate() {
        if *character == '\n' {
            starts.insert(offset + 1);
        }
        if matches!(*character, '.' | '!' | '?' | ';')
            && chars
                .get(index + 1)
                .is_some_and(|(_, next)| matches!(next, ' ' | '\t' | '\r' | '\n'))
        {
            starts.insert(offset + character.len_utf8());
        }
    }
    let starts = starts.into_iter().collect::<Vec<_>>();
    Ok(object!(starts.windows(2).enumerate().map(|(index,pair)|{let fragment=&text[pair[0]..pair[1]];object!({"id":format!("{prefix}-{index}"),"text":fragment,"sha256":result_store::sha(fragment.as_bytes())})}).collect::<Vec<_>>()))
}
pub fn check_verdict(packet: &Value, verdict: &Value) -> Result<Value> {
    if !result_store::exact(
        verdict,
        &[
            "profile",
            "packet_sha256",
            "proposal_sha256",
            "verdict",
            "records",
        ],
    ) || verdict["profile"] != VERDICT_PROFILE
        || verdict["packet_sha256"] != packet["sha256"]
        || verdict["proposal_sha256"] != packet["proposal_sha256"]
        || !matches!(
            verdict["verdict"].as_str(),
            Some("accept" | "reject" | "unresolved")
        )
    {
        return Err(fail("semantic-review:invalid-verdict"));
    }
    let offered = array(&packet["records"], "semantic-review:invalid-packet")?;
    let rows = array(&verdict["records"], "semantic-review:invalid-verdict")?;
    if rows.len() != offered.len() {
        return Err(fail("semantic-review:incomplete-coverage"));
    }
    let sources = array(&packet["sources"], "semantic-review:invalid-packet")?;
    let evidence = |value: &Value| -> Result<()> {
        let rows = array(value, "semantic-review:missing-evidence")?;
        if rows.is_empty() || rows.len() > 5 {
            return Err(fail("semantic-review:missing-evidence"));
        }
        for cite in rows {
            let source = sources
                .iter()
                .find(|row| row["path"] == cite["path"])
                .ok_or_else(|| fail("semantic-review:invalid-evidence"))?;
            let quote = cite["quote"]
                .as_str()
                .filter(|value| !value.is_empty() && value.chars().count() <= 500)
                .ok_or_else(|| fail("semantic-review:invalid-evidence"))?;
            if !result_store::exact(cite, &["path", "sha256", "quote"])
                || cite["sha256"] != source["sha256"]
                || !source["text"].as_str().unwrap_or("").contains(quote)
            {
                return Err(fail("semantic-review:invalid-evidence"));
            }
        }
        Ok(())
    };
    let mut seen = BTreeSet::new();
    let mut accepted = true;
    for row in rows {
        if !result_store::exact(
            row,
            &["record_id", "old_clauses", "new_clauses", "boundary_checks"],
        ) || !seen.insert(row["record_id"].as_str().unwrap_or(""))
        {
            return Err(fail("semantic-review:invalid-record-review"));
        }
        let record = offered
            .iter()
            .find(|record| record["record_id"] == row["record_id"])
            .ok_or_else(|| fail("semantic-review:invalid-record-review"))?;
        for key in ["old_clauses", "new_clauses"] {
            let clauses = array(&record[key], "semantic-review:invalid-packet")?;
            let decisions = array(&row[key], "semantic-review:incomplete-coverage")?;
            if clauses.len() != decisions.len() {
                return Err(fail("semantic-review:incomplete-coverage"));
            }
            let mut used = BTreeSet::new();
            for decision in decisions {
                let clause = clauses
                    .iter()
                    .find(|clause| clause["id"] == decision["id"])
                    .ok_or_else(|| fail("semantic-review:invalid-clause-review"))?;
                let status = decision["status"].as_str().unwrap_or("");
                if !result_store::exact(decision, &["id", "sha256", "status", "reason", "evidence"])
                    || !used.insert(decision["id"].as_str().unwrap_or(""))
                    || decision["sha256"] != clause["sha256"]
                    || !(if key == "old_clauses" {
                        [
                            "retained",
                            "changed-with-evidence",
                            "removed-with-evidence",
                            "unresolved",
                        ]
                        .contains(&status)
                    } else {
                        ["supported", "contradicted", "unresolved"].contains(&status)
                    })
                {
                    return Err(fail("semantic-review:invalid-clause-review"));
                }
                results::text(&decision["reason"], "clause reason", 500)?;
                evidence(&decision["evidence"])?;
                if matches!(status, "contradicted" | "unresolved") {
                    accepted = false;
                }
            }
        }
        let checks = array(
            &row["boundary_checks"],
            "semantic-review:missing-boundary-checks",
        )?;
        if checks.is_empty() || checks.len() > 10 {
            return Err(fail("semantic-review:missing-boundary-checks"));
        }
        for check in checks {
            if !result_store::exact(check, &["case", "status", "evidence"])
                || !matches!(
                    check["status"].as_str(),
                    Some("supported" | "contradicted" | "unresolved")
                )
            {
                return Err(fail("semantic-review:invalid-boundary-check"));
            }
            results::text(&check["case"], "boundary case", 500)?;
            evidence(&check["evidence"])?;
            accepted &= check["status"] == "supported";
        }
    }
    let state = if verdict["verdict"] == "accept" {
        if accepted { "accepted" } else { "unresolved" }
    } else {
        verdict["verdict"].as_str().unwrap_or("unresolved")
    };
    Ok(object!({"state":state,"publish":verdict["verdict"]=="accept"&&accepted}))
}
fn check_packet(packet: &Value, proposal: &Value) -> Result<()> {
    if !result_store::exact(
        packet,
        &[
            "profile",
            "proposal_id",
            "proposal_sha256",
            "task_id",
            "task_uuid",
            "owner",
            "project",
            "task_revision",
            "task_sha256",
            "sources",
            "source_snapshot_sha256",
            "records",
            "sha256",
        ],
    ) || packet["profile"] != PACKET_PROFILE
        || packet["proposal_id"] != proposal["plan"]["plan_id"]
        || packet["proposal_sha256"] != proposal["sha256"]
        || ["task_id", "task_uuid", "owner", "project"]
            .iter()
            .any(|key| packet[key] != proposal["plan"][key])
        || packet["task_revision"]
            .as_u64()
            .is_none_or(|revision| revision == 0)
        || !result_store::hex(packet["task_sha256"].as_str().unwrap_or(""), 64)
    {
        return Err(fail("semantic-review:packet-changed"));
    }
    let mut unsigned = packet.clone();
    unsigned
        .as_object_mut()
        .ok_or_else(|| fail("semantic-review:packet-changed"))?
        .remove("sha256");
    if packet["sha256"] != digest(&unsigned)?
        || packet["source_snapshot_sha256"] != digest(&packet["sources"])?
    {
        return Err(fail("semantic-review:packet-changed"));
    }
    let mut pins = BTreeMap::new();
    for row in array(
        &proposal["payload"]["learnings"],
        "semantic-review:invalid-proposal",
    )? {
        for pin in array(
            &row["source_fingerprints"],
            "semantic-review:invalid-proposal",
        )? {
            pins.insert(pin["path"].as_str().unwrap_or(""), pin["sha256"].clone());
        }
    }
    let mut seen = BTreeSet::new();
    for source in array(&packet["sources"], "semantic-review:invalid-packet-source")? {
        let path = source["path"]
            .as_str()
            .ok_or_else(|| fail("semantic-review:invalid-packet-source"))?;
        let text = source["text"]
            .as_str()
            .ok_or_else(|| fail("semantic-review:invalid-packet-source"))?;
        if !result_store::exact(source, &["path", "sha256", "raw_sha256", "text"])
            || pins.get(path) != Some(&source["sha256"])
            || !seen.insert(path)
            || result_store::sha(text.as_bytes()) != source["raw_sha256"].as_str().unwrap_or("")
            || memory_kernel_probe::fingerprint_bytes(text.as_bytes())
                != source["sha256"].as_str().unwrap_or("")
        {
            return Err(fail("semantic-review:invalid-packet-source"));
        }
    }
    if seen != pins.keys().copied().collect() {
        return Err(fail("semantic-review:incomplete-packet-source"));
    }
    let offered = array(
        &proposal["plan"]["records"],
        "semantic-review:invalid-proposal",
    )?;
    let proposed = array(
        &proposal["payload"]["learnings"],
        "semantic-review:invalid-proposal",
    )?;
    let mut seen = BTreeSet::new();
    for row in array(&packet["records"], "semantic-review:invalid-packet-record")? {
        let id = row["record_id"]
            .as_str()
            .ok_or_else(|| fail("semantic-review:invalid-packet-record"))?;
        let old = offered
            .iter()
            .find(|value| value["record_id"] == id)
            .ok_or_else(|| fail("semantic-review:invalid-packet-record"))?;
        let new = proposed
            .iter()
            .find(|value| value["record_id"] == id)
            .ok_or_else(|| fail("semantic-review:invalid-packet-record"))?;
        if !result_store::exact(
            row,
            &[
                "record_id",
                "title",
                "old_consequence",
                "proposed_consequence",
                "old_clauses",
                "new_clauses",
                "sources",
                "old_memories",
            ],
        ) || !seen.insert(id)
            || row["title"] != new["title"]
            || row["sources"] != new["sources"]
            || row["old_consequence"] != old["consequence"]
            || row["proposed_consequence"] != new["consequence"]
            || row["old_clauses"] != clauses(old["consequence"].as_str().unwrap_or(""), "old")?
            || row["new_clauses"] != clauses(new["consequence"].as_str().unwrap_or(""), "new")?
        {
            return Err(fail("semantic-review:packet-claim-changed"));
        }
        let memories = array(
            &row["old_memories"],
            "semantic-review:invalid-packet-memory",
        )?;
        if memories.len() > 2000
            || memories.iter().any(|memory| {
                !result_store::exact(memory, &["id", "path", "body", "sha256"])
                    || ["id", "path", "body", "sha256"]
                        .iter()
                        .any(|key| !memory[key].is_string())
                    || !result_store::hex(memory["sha256"].as_str().unwrap_or(""), 64)
            })
        {
            return Err(fail("semantic-review:invalid-packet-memory"));
        }
    }
    if seen
        != proposed
            .iter()
            .filter_map(|row| row["record_id"].as_str())
            .collect()
    {
        return Err(fail("semantic-review:incomplete-packet-records"));
    }
    Ok(())
}
