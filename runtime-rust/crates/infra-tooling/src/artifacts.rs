//! Explicit native artifacts; never infer a release from a build directory.
use crate::common;
use infra_core::{Result, RuntimeError, fs as safe};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
const MAX_ENGINE: u64 = 512 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub target: String,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub executable: bool,
    pub capabilities: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub engine: String,
    pub version: String,
    pub artifacts: Vec<Artifact>,
}
impl Artifact {
    pub fn capture(root: &Path, path: &str, target: &str, capabilities: &[String]) -> Result<Self> {
        if target.is_empty()
            || !target
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(RuntimeError::invalid("Invalid native target"));
        }
        let source = common::under(root, path)?;
        let image = std::fs::symlink_metadata(&source)?;
        if safe::linked(&image) || !image.is_file() || image.len() > MAX_ENGINE {
            return Err(RuntimeError::policy(
                "Native artifact must be a bounded regular file",
            ));
        }
        let bytes = safe::read_regular(&source)?;
        let mut declared = capabilities.to_vec();
        declared.sort();
        declared.dedup();
        if declared.is_empty() || declared.iter().any(|s| s.is_empty()) {
            return Err(RuntimeError::invalid(
                "Native artifact requires real capability metadata",
            ));
        }
        Ok(Self {
            target: target.into(),
            path: path.into(),
            sha256: common::sha256(&bytes),
            bytes: bytes.len() as u64,
            executable: true,
            capabilities: declared,
        })
    }
    pub fn verify(&self, root: &Path) -> Result<PathBuf> {
        common::relative(&self.path)?;
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.bytes > MAX_ENGINE
            || !self.executable
        {
            return Err(RuntimeError::invalid("Malformed native artifact metadata"));
        }
        let path = common::under(root, &self.path)?;
        let data = safe::read_regular(&path)?;
        if data.len() as u64 != self.bytes || common::sha256(&data) != self.sha256 {
            return Err(RuntimeError::conflict(
                "Native artifact differs from its release manifest",
            ));
        }
        Ok(path)
    }
}
impl ReleaseManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let value: Self = serde_json::from_slice(&safe::read_regular(path)?)?;
        if value.schema_version != 1
            || value.engine != "infra-engine"
            || value.version.is_empty()
            || value.artifacts.is_empty()
        {
            return Err(RuntimeError::invalid("Unsupported native release manifest"));
        }
        let mut targets = BTreeSet::new();
        for artifact in &value.artifacts {
            if !targets.insert(&artifact.target) {
                return Err(RuntimeError::invalid("Duplicate native release target"));
            }
        }
        Ok(value)
    }
    pub fn select(&self, target: &str, required: &[String]) -> Result<&Artifact> {
        let artifact = self
            .artifacts
            .iter()
            .find(|artifact| artifact.target == target)
            .ok_or_else(|| {
                RuntimeError::unavailable("Release has no engine for the selected target")
            })?;
        if required
            .iter()
            .any(|capability| !artifact.capabilities.contains(capability))
        {
            return Err(RuntimeError::unavailable(
                "Native engine lacks a required production capability",
            ));
        }
        Ok(artifact)
    }
}
pub fn current_engine(target: &str, capabilities: &[String]) -> Result<(PathBuf, Artifact)> {
    let executable = std::env::current_exe()?;
    let parent = executable
        .parent()
        .ok_or_else(|| RuntimeError::invalid("Native executable has no parent"))?;
    let name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| RuntimeError::invalid("Native executable name is not UTF-8"))?;
    Ok((
        executable.clone(),
        Artifact::capture(parent, name, target, capabilities)?,
    ))
}

pub fn native_target() -> Result<String> {
    let architecture = if cfg!(target_arch = "x86") {
        "i686"
    } else {
        std::env::consts::ARCH
    };
    let platform = if cfg!(target_os = "linux") {
        if cfg!(target_env = "musl") {
            "unknown-linux-musl"
        } else {
            "unknown-linux-gnu"
        }
    } else if cfg!(target_os = "macos") {
        "apple-darwin"
    } else if cfg!(target_os = "windows") {
        if cfg!(target_env = "gnu") {
            "pc-windows-gnu"
        } else {
            "pc-windows-msvc"
        }
    } else {
        return Err(RuntimeError::unavailable(
            "Native artifact target is outside the supported platform set",
        ));
    };
    Ok(format!("{architecture}-{platform}"))
}
pub fn assemble(
    root: &Path,
    editions: &[String],
    include_generator: bool,
    write: bool,
) -> Result<serde_json::Value> {
    let installer = crate::installer::Installer::new(root)?;
    let descriptor = installer.engine_capabilities()?;
    let executable = std::env::current_exe()?;
    let bytes = safe::read_regular(&executable)?;
    if bytes.len() as u64 > MAX_ENGINE {
        return Err(RuntimeError::invalid(
            "Current native executable exceeds release bound",
        ));
    }
    let digest = common::sha256(&bytes);
    let target = native_target()?;
    let capabilities = descriptor["capabilities"]
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("Current engine capabilities are absent"))?;
    let filename = if cfg!(windows) {
        "infra-engine.exe"
    } else {
        "infra-engine"
    };
    let selected = if editions.is_empty() {
        common::PHP_EDITIONS
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    } else {
        editions.to_vec()
    };
    let mut destinations = Vec::new();
    for edition in selected {
        let edition = common::edition(&edition)?;
        if edition == "Infrastructure-Creator" {
            return Err(RuntimeError::invalid(
                "Generator artifacts use the explicit include_generator switch",
            ));
        }
        if !root.join(edition).join("AGENTS.md").is_file() {
            return Err(RuntimeError::not_found("Release edition is absent"));
        }
        destinations.push((
            root.join(edition),
            format!("memory-bank/bin/{filename}"),
            "memory-bank/runtime-capabilities.json".to_string(),
        ));
    }
    if include_generator {
        let generator = root.join("Infrastructure-Creator");
        if !generator.join("AGENTS.md").is_file() {
            return Err(RuntimeError::not_found("Generator source root is absent"));
        }
        destinations.push((
            generator,
            format!("memory-bank/bin/{filename}"),
            "memory-bank/runtime-capabilities.json".to_string(),
        ));
        let asset = root.join("Infrastructure-Creator/.agents/skills/memory-seed/assets");
        if !asset.is_dir() {
            return Err(RuntimeError::not_found("Generator asset root is absent"));
        }
        destinations.push((
            asset,
            format!("bin/{filename}"),
            "native-artifacts.json".to_string(),
        ));
    }
    let mut report = Vec::new();
    for (base, path, metadata) in destinations {
        // "source" names the sources the engine was compiled from; a Harness
        // update restarts only on an engine whose sources match the clone.
        let value = serde_json::json!({"schema_version":1,"engine":"infra-engine","capability_profile":"infra-engine-v1","version":descriptor["version"],"target":target,"path":path,"sha256":digest,"bytes":bytes.len(),"capabilities":capabilities,"source":descriptor["source"]});
        if write {
            let mut files = infra_core::bank::publication::PublicationFiles::new(&base)?;
            let previous = files.read(&path, MAX_ENGINE as usize)?;
            files.write_expected_mode(
                &path,
                &bytes,
                previous.as_deref(),
                0o755,
                MAX_ENGINE as usize,
            )?;
            let previous = files.read(&metadata, 1024 * 1024)?;
            files.write_expected_mode(
                &metadata,
                &common::json_bytes(&value)?,
                previous.as_deref(),
                0o644,
                1024 * 1024,
            )?;
        }
        report.push(
            serde_json::json!({"root":base,"artifact":value,"metadata":metadata,"written":write}),
        );
    }
    Ok(
        serde_json::json!({"engine":descriptor,"sha256":digest,"target":target,"artifacts":report,"write":write}),
    )
}
