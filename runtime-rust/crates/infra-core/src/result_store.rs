//! Shared result fences and disposable receipts, implemented without a foreign runtime.
use crate::{
    BrainRepository, ErrorKind, Result, RuntimeContext, RuntimeError,
    domain::{is_uuid4, json},
    fs,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

pub const PREPARE_PROFILE: &str = "harness-memory-request-v1";
pub const FRONTIER_PROFILE: &str = "memory-result-frontier-v2";
pub const PROOF_PROFILE: &str = "memory-result-proof-v1";
pub const ATTEMPT_LIMIT: usize = 4096;
pub fn sha(bytes: &[u8]) -> String {
    crate::query::sha256(bytes)
}
pub fn legacy_digest(value: &Value, sort: bool, ascii: bool) -> Result<String> {
    Ok(sha(
        json::python_default(value, sort, ascii, true)?.as_bytes()
    ))
}
pub fn fail(code: &str) -> RuntimeError {
    RuntimeError::coded(ErrorKind::Conflict, code, code)
}
pub fn hex(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn exact(value: &Value, names: &[&str]) -> bool {
    value.as_object().is_some_and(|map| {
        map.len() == names.len() && names.iter().all(|key| map.contains_key(*key))
    })
}

pub fn read_bounded(path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || fs::linked(&metadata) || metadata.len() > limit as u64 {
        return Err(RuntimeError::policy(
            "Evidence must be a bounded regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(RuntimeError::policy("Linked evidence file is refused"));
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let handle = options.open(path)?;
    let before = handle.metadata()?;
    if !same_image(&metadata, &before) {
        return Err(RuntimeError::conflict("Evidence changed before capture"));
    }
    let mut raw = Vec::new();
    handle.take(limit as u64 + 1).read_to_end(&mut raw)?;
    let after = std::fs::symlink_metadata(path)?;
    if raw.len() > limit || raw.len() as u64 != metadata.len() || !same_image(&metadata, &after) {
        return Err(RuntimeError::conflict("Evidence changed during capture"));
    }
    Ok(Some(raw))
}
fn same_image(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    if left.len() != right.len()
        || left.modified().ok() != right.modified().ok()
        || left.is_file() != right.is_file()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev()
            && left.ino() == right.ino()
            && left.mode() == right.mode()
            && left.nlink() == right.nlink()
            && left.ctime() == right.ctime()
            && left.ctime_nsec() == right.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        left.created().ok() == right.created().ok() && !fs::linked(right)
    }
}
pub fn read_json(path: &Path, limit: usize) -> Result<Option<Value>> {
    read_bounded(path, limit)?
        .map(|raw| {
            let text = std::str::from_utf8(&raw)
                .map_err(|_| RuntimeError::invalid("Evidence is not UTF-8"))?;
            json::parse(text)
        })
        .transpose()
}
/// Custody proofs and replay controls require unique decoded members and
/// finite values; unrelated legacy metadata continues to use read_json.
pub fn read_strict_json(path: &Path, limit: usize) -> Result<Option<Value>> {
    read_bounded(path, limit)?
        .map(|raw| crate::strict_json::parse(&raw, limit, 512, limit.max(1)))
        .transpose()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub task_id: String,
    pub result_id: String,
    pub digest: String,
    pub task_revision: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stage {
    pub record_id: String,
    pub revision: u64,
    pub sha256: String,
    pub state: String,
    pub done: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attempt {
    pub profile: String,
    pub task_id: String,
    pub result_id: String,
    pub digest: String,
    pub owner: String,
    pub attestation: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stages: BTreeMap<String, Stage>,
    pub domain_sha256: Option<String>,
    pub task_revision: Option<u64>,
}
impl Attempt {
    #[expect(
        clippy::too_many_arguments,
        reason = "Result frontier construction seals task/result digests, owner attestation, revision and three independent proof hashes"
    )]
    pub fn new(
        task: &str,
        result: &str,
        digest: &str,
        owner: &str,
        attestation: &str,
        revision: u64,
        plan: String,
        scope: String,
        sources: String,
        domain: Option<String>,
    ) -> Self {
        Self {
            profile: FRONTIER_PROFILE.into(),
            task_id: task.into(),
            result_id: result.into(),
            digest: digest.into(),
            owner: owner.into(),
            attestation: attestation.into(),
            state: "started".into(),
            original_revision: Some(revision),
            plan_sha256: Some(plan),
            scope_sha256: Some(scope),
            source_sha256: Some(sources),
            stages: BTreeMap::new(),
            domain_sha256: domain,
            task_revision: None,
        }
    }
    pub fn value(&self) -> Result<Value> {
        let mut value = serde_json::to_value(self)?;
        if self.profile == FRONTIER_PROFILE {
            value["stages"] = serde_json::to_value(&self.stages)?;
        }
        Ok(value)
    }
}
#[derive(Clone)]
pub struct EvidenceStore {
    pub context: RuntimeContext,
}
impl EvidenceStore {
    pub fn new(context: RuntimeContext) -> Self {
        Self { context }
    }
    pub fn receipt_path(&self, task: &str, result: &str) -> PathBuf {
        self.context
            .roots
            .brain_root()
            .join("local/results")
            .join(format!(
                "{}.json",
                sha(format!("{task}\0{result}").as_bytes())
            ))
    }
    pub fn attempt_path(&self, task: &str, result: &str, digest: &str) -> PathBuf {
        self.context
            .roots
            .brain_root()
            .join("control/result-attempts")
            .join(task)
            .join(format!(
                "{}.json",
                sha(format!("{task}\0{result}\0{digest}").as_bytes())
            ))
    }
    fn admit(&self, path: &Path) -> Result<()> {
        fs::admit_descendant(&self.context.roots.state, path)
    }
    pub fn receipt(
        &self,
        task: &str,
        result: &str,
        digest: &str,
        aliases: &[String],
    ) -> Result<(Option<Receipt>, bool)> {
        let path = self.receipt_path(task, result);
        self.admit(&path)?;
        let present = match std::fs::symlink_metadata(&path) {
            Ok(_) => true,
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        };
        let value = read_strict_json(&path, ATTEMPT_LIMIT).ok().flatten();
        let parsed = value
            .as_ref()
            .filter(|value| exact(value, &["task_id", "result_id", "digest", "task_revision"]))
            .and_then(|value| serde_json::from_value::<Receipt>(value.clone()).ok());
        if let Some(receipt) = parsed {
            if (receipt.task_id == task || aliases.contains(&receipt.task_id))
                && receipt.result_id == result
                && receipt.digest == digest
                && receipt.task_revision.is_none_or(|revision| revision > 0)
            {
                return Ok((Some(receipt), present));
            }
            if receipt.result_id == result
                && (receipt.task_id == task || aliases.contains(&receipt.task_id))
                && receipt.digest != digest
            {
                return Err(fail("memory-result-v1:result_id_reused"));
            }
        }
        Ok((None, present))
    }
    pub fn attempt(&self, task: &str, result: &str, digest: &str) -> Result<Option<Attempt>> {
        let path = self.attempt_path(task, result, digest);
        self.admit(&path)?;
        let Some(raw) = read_bounded(&path, ATTEMPT_LIMIT)
            .map_err(|_| fail("memory-result-v1:attempt_unavailable"))?
        else {
            return Ok(None);
        };
        let value: Value = crate::results::strict_input(&raw)
            .map_err(|_| fail("memory-result-v1:attempt_unavailable"))?;
        let profile = value["profile"].as_str().unwrap_or("");
        let proof = profile == PROOF_PROFILE;
        let frontier = profile == FRONTIER_PROFILE;
        let fields = if proof {
            vec![
                "profile",
                "task_id",
                "result_id",
                "digest",
                "owner",
                "attestation",
                "state",
            ]
        } else if frontier {
            vec![
                "profile",
                "task_id",
                "result_id",
                "digest",
                "original_revision",
                "owner",
                "attestation",
                "plan_sha256",
                "state",
                "task_revision",
                "scope_sha256",
                "source_sha256",
                "domain_sha256",
                "stages",
            ]
        } else {
            vec![
                "profile",
                "task_id",
                "result_id",
                "digest",
                "original_revision",
                "owner",
                "attestation",
                "plan_sha256",
                "state",
                "task_revision",
            ]
        };
        if !exact(&value, &fields)
            || (!proof && !frontier && profile != "memory-result-attempt-v1")
            || !is_uuid4(value["task_id"].as_str().unwrap_or(""))
            || !hex(value["digest"].as_str().unwrap_or(""), 64)
            || !matches!(value["attestation"].as_str(), Some("agent" | "person"))
            || !matches!(value["state"].as_str(), Some("started" | "complete"))
        {
            return Err(fail("memory-result-v1:attempt_unavailable"));
        }
        let mut expanded = value.clone();
        if proof {
            expanded["domain_sha256"] = Value::Null;
            expanded["task_revision"] = Value::Null;
            expanded["stages"] = object!({});
        } else if !frontier {
            expanded["domain_sha256"] = Value::Null;
            expanded["stages"] = object!({});
        }
        let attempt: Attempt = serde_json::from_value(expanded)
            .map_err(|_| fail("memory-result-v1:attempt_unavailable"))?;
        if attempt.task_id != task
            || attempt.result_id != result
            || attempt.digest != digest
            || attempt.owner.is_empty()
            || attempt.owner.chars().count() > 200
            || (!proof
                && attempt
                    .original_revision
                    .is_none_or(|revision| revision == 0))
            || proof && attempt.state != "complete"
            || attempt.stages.len() > 4
        {
            return Err(fail("memory-result-v1:attempt_unavailable"));
        }
        for (key, stage) in &attempt.stages {
            if (key != "task" && (!key.starts_with("learning-") || !hex(&key[9..], 24)))
                || !is_uuid4(&stage.record_id)
                || stage.revision == 0
                || !hex(&stage.sha256, 64)
                || !matches!(
                    stage.state.as_str(),
                    "created" | "completed" | "updated" | "task"
                )
                || (key == "task" && (stage.state != "task" || !stage.done))
                || (key != "task" && stage.state == "task")
            {
                return Err(fail("memory-result-v1:attempt_unavailable"));
            }
        }
        if frontier
            && (attempt
                .scope_sha256
                .as_deref()
                .is_none_or(|value| !hex(value, 64))
                || attempt
                    .source_sha256
                    .as_deref()
                    .is_none_or(|value| !hex(value, 64))
                || attempt
                    .domain_sha256
                    .as_deref()
                    .is_some_and(|value| !hex(value, 64)))
        {
            return Err(fail("memory-result-v1:attempt_unavailable"));
        }
        Ok(Some(attempt))
    }
    pub fn write_attempt(&self, attempt: &Attempt) -> Result<()> {
        let path = self.attempt_path(&attempt.task_id, &attempt.result_id, &attempt.digest);
        self.admit(&path)?;
        fs::atomic_json(&path, &attempt.value()?)
    }
    pub fn checkpoint(
        &self,
        attempt: &mut Attempt,
        key: &str,
        record: &Value,
        state: &str,
        done: bool,
    ) -> Result<()> {
        if attempt.profile != FRONTIER_PROFILE {
            return Ok(());
        }
        let id = record["id"].as_str().unwrap_or("");
        let image = self.record_image(id);
        if let Some(image) = image {
            attempt.stages.insert(
                key.into(),
                Stage {
                    record_id: id.into(),
                    revision: record["revision"].as_u64().unwrap_or(0),
                    sha256: image,
                    state: state.into(),
                    done,
                },
            );
            attempt.domain_sha256 = self.domain_image();
        } else {
            attempt.domain_sha256 = None;
        }
        if key == "task" {
            attempt.task_revision = record["revision"].as_u64();
        }
        self.write_attempt(attempt)
    }
    pub fn complete(&self, attempt: &Attempt) -> Result<()> {
        let path = self.attempt_path(&attempt.task_id, &attempt.result_id, &attempt.digest);
        self.admit(&path)?;
        fs::atomic_json(
            &path,
            &object!({"profile":PROOF_PROFILE,"task_id":attempt.task_id,"result_id":attempt.result_id,"digest":attempt.digest,"owner":attempt.owner,"attestation":attempt.attestation,"state":"complete"}),
        )?;
        self.prune(&path);
        Ok(())
    }
    pub fn write_receipt(&self, receipt: &Receipt) -> Result<()> {
        let path = self.receipt_path(&receipt.task_id, &receipt.result_id);
        self.admit(&path)?;
        fs::atomic_json(&path, &serde_json::to_value(receipt)?)
    }
    pub fn record_image(&self, id: &str) -> Option<String> {
        let brain = BrainRepository::new(self.context.clone()).ok()?;
        let image = brain.find(id, None, true).ok()?;
        read_bounded(&image.path, 4 * 1024 * 1024)
            .ok()
            .flatten()
            .map(|raw| sha(&raw))
    }
    fn learning_image_path(&self, id: &str) -> PathBuf {
        self.context
            .roots
            .brain_root()
            .join("control/result-learning-images")
            .join(format!("{}.json", sha(id.as_bytes())))
    }
    pub fn owns_image(&self, id: &str, record: &Value, owner: &str) -> bool {
        let path = self.learning_image_path(id);
        if self.admit(&path).is_err() {
            return false;
        }
        let Some(proof) = read_strict_json(&path, ATTEMPT_LIMIT).ok().flatten() else {
            return false;
        };
        exact(
            &proof,
            &["profile", "record_id", "revision", "sha256", "owner"],
        ) && proof["profile"] == "native-learning-image-v1"
            && proof["record_id"] == record["id"]
            && proof["revision"] == record["revision"]
            && proof["owner"] == owner
            && self
                .record_image(record["id"].as_str().unwrap_or(""))
                .as_deref()
                == proof["sha256"].as_str()
    }
    pub fn remember_image(&self, id: &str, record: &Value, owner: &str) {
        if let Some(image) = self.record_image(record["id"].as_str().unwrap_or("")) {
            let path = self.learning_image_path(id);
            if self.admit(&path).is_ok() {
                let _ = fs::atomic_json(
                    &path,
                    &object!({"profile":"native-learning-image-v1","record_id":record["id"],"revision":record["revision"],"sha256":image,"owner":owner}),
                );
            }
        }
    }
    pub fn domain_image(&self) -> Option<String> {
        fn scan(
            root: &Path,
            path: &Path,
            depth: usize,
            nodes: &mut usize,
            total: &mut usize,
            rows: &mut Vec<Value>,
        ) -> Result<()> {
            if !fs::admit_directory(path)? {
                return Ok(());
            }
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                *nodes += 1;
                if *nodes > 4096 || depth >= 32 {
                    return Err(RuntimeError::unavailable("Frontier budget exhausted"));
                }
                let metadata = std::fs::symlink_metadata(entry.path())?;
                if fs::linked(&metadata) {
                    return Err(RuntimeError::policy("Linked frontier is unavailable"));
                }
                if metadata.is_dir() {
                    scan(root, &entry.path(), depth + 1, nodes, total, rows)?;
                } else {
                    *total = total.saturating_add(metadata.len() as usize);
                    if *total > 32 * 1024 * 1024 {
                        return Err(RuntimeError::unavailable("Frontier byte budget exhausted"));
                    }
                    let raw = read_bounded(&entry.path(), 32 * 1024 * 1024)?
                        .ok_or_else(|| RuntimeError::unavailable("Frontier disappeared"))?;
                    rows.push(object!([
                        entry
                            .path()
                            .strip_prefix(root)
                            .map_err(|_| RuntimeError::policy("Frontier escapes root"))?
                            .to_str()
                            .ok_or_else(|| RuntimeError::invalid("Frontier path is not UTF-8"))?,
                        sha(&raw)
                    ]));
                }
            }
            Ok(())
        }
        let root = self.context.roots.brain_root();
        let (mut rows, mut nodes, mut total) = (vec![], 0, 0);
        for name in ["dynamic", "indexes", "control/handoffs"] {
            scan(
                &root,
                &root.join(name),
                0,
                &mut nodes,
                &mut total,
                &mut rows,
            )
            .ok()?;
        }
        rows.sort_by(|left, right| left[0].as_str().cmp(&right[0].as_str()));
        legacy_digest(&object!(rows), true, true).ok()
    }
    fn prune(&self, path: &Path) {
        let Some(parent) = path.parent() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(parent) else {
            return;
        };
        let mut proofs = Vec::new();
        for entry in entries.flatten() {
            let candidate = entry.path();
            if candidate == path {
                continue;
            }
            let Some(name) = candidate.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };
            if !hex(name, 64) {
                continue;
            }
            if let Ok(Some(value)) = read_strict_json(&candidate, ATTEMPT_LIMIT)
                && value["state"] == "complete"
            {
                proofs.push((
                    entry.metadata().and_then(|value| value.modified()).ok(),
                    candidate,
                ));
            }
        }
        proofs.sort();
        let remove = proofs.len().saturating_sub(63);
        for (_, path) in proofs.into_iter().take(remove) {
            let _ = std::fs::remove_file(path);
        }
    }
}
