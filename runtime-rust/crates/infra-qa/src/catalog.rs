//! Versioned catalog snapshots and non-overwriting evidence destination admission.
use crate::{common::*, evidence};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const LEGACY_PATH: &str = "qa/tc-ai/cases.json";
pub const NATIVE_PATH: &str = "qa/tc-ai/cases.native.json";
pub const NATIVE_EVIDENCE: &str = "docs/qa/evidence-native";

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub source: String,
    pub sha256: String,
    pub version: String,
    pub cases: Vec<Value>,
    pub document: Value,
}
impl Snapshot {
    pub fn read(root: &Path, path: &Path) -> Result<Self> {
        let raw = fs::read(path).with_context(|| format!("catalog {}", path.display()))?;
        let value: Value = serde_json::from_slice(&raw).context("invalid case catalog JSON")?;
        let version = schema_version(&value)?.to_owned();
        let cases = evidence::catalog(&value)?;
        let path = fs::canonicalize(path)?;
        let root = fs::canonicalize(root)?;
        let source = path
            .strip_prefix(root)
            .ok()
            .and_then(|relative| relative.to_str())
            .map(|relative| relative.replace('\\', "/"))
            .unwrap_or_else(|| "{external-catalog}".to_owned());
        Ok(Self {
            source,
            sha256: sha(&raw),
            version,
            cases,
            document: value,
        })
    }
    pub fn case(&self, run: &str) -> Result<&Value> {
        self.cases
            .iter()
            .find(|case| case["run_id"] == run)
            .with_context(|| format!("run {run} absent from {}", self.source))
    }
}
pub fn for_version(root: &Path, version: &str) -> Result<Snapshot> {
    let expected = match version {
        LEGACY_VERSION => {
            let primary = Snapshot::read(root, &root.join(LEGACY_PATH))?;
            if primary.version == LEGACY_VERSION {
                return Ok(primary);
            }
            // Preserve the earlier explicit migration layout without modifying it.
            root.join("qa/tc-ai/cases.v1.json")
        }
        NATIVE_VERSION => root.join(NATIVE_PATH),
        _ => bail!("unsupported artifact schema_version"),
    };
    let snapshot = Snapshot::read(root, &expected)?;
    if snapshot.version != version {
        bail!("catalog version differs from evidence version");
    }
    Ok(snapshot)
}
pub fn evidence_version(path: &Path) -> Result<String> {
    let run = read_json(&path.join("run.json"))?;
    Ok(schema_version(&run)?.to_owned())
}
/// Check the whole selected batch before any command or output directory creation.
/// Each run performs the same admission again immediately before publication.
pub fn preflight(output: &Path, cases: &[Value]) -> Result<()> {
    let allowed = codex();
    for case in cases {
        let run = case["run_id"].as_str().context("run ID")?;
        if !allowed.iter().any(|id| id == run) {
            bail!("unknown Run ID");
        }
        let destination = output.join(run);
        match fs::symlink_metadata(&destination) {
            Ok(_) => bail!(
                "evidence directory exists; refusing overwrite: {}",
                destination.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
pub fn native_path(root: &Path) -> PathBuf {
    root.join(NATIVE_PATH)
}

#[cfg(test)]
mod tests;
