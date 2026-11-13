use crate::common::*;
use infra_core::{Result, RuntimeError, fs};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub const MANIFEST: &str = ".infra-manifest.json";
pub const STATE_EXCLUDES: &[&str] = &[
    "memory-bank/chunks/",
    "memory-bank/INDEX.md",
    "memory-bank/.memory-counter",
    "memory-bank/local/",
    "project-brain/indexes/",
    "project-brain/local/",
    "project-brain/archive/",
    "project-brain/control/",
    "project-brain/dynamic/",
];

pub fn resolve_target(path: &Path) -> Result<PathBuf> {
    let expanded = if let Some(value) = path.to_str().and_then(|value| value.strip_prefix("~/")) {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or_else(|| RuntimeError::invalid("Cannot expand target home directory"))?;
        PathBuf::from(home).join(value)
    } else {
        path.to_path_buf()
    };
    let absolute = fs::absolute(&expanded)?;
    let mut current = PathBuf::new();
    for part in absolute.components() {
        current.push(part.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if fs::linked(&metadata) => {
                return Err(RuntimeError::policy(format!(
                    "target path contains a symlink: {}",
                    path.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    fs::canonical_or_missing(&absolute)
}
pub fn normalize(value: &str) -> Result<String> {
    let reserved = expression(r"(?i)^(CON|CONIN\$|CONOUT\$|PRN|AUX|NUL|COM[1-9¹²³]|LPT[1-9¹²³])$")?;
    let invalid = value.is_empty()
        || value.starts_with('/')
        || value.contains('\\')
        || value.chars().any(|character| {
            character < ' ' || character == '\u{7f}' || ":<>\"|?*".contains(character)
        })
        || value.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with('.')
                || part.ends_with(' ')
                || reserved.is_match(trailing(part.split('.').next().unwrap_or(""), &[' ']))
        });
    if invalid {
        return Err(RuntimeError::policy(format!(
            "write-plan path must use safe canonical target-relative POSIX spelling: {value}"
        )));
    }
    Ok(value.into())
}
pub fn confined(root: &Path, value: &str) -> Result<PathBuf> {
    let root = resolve_target(root)?;
    let value = normalize(value)?;
    let path = root.join(value);
    fs::admit_descendant(&root, &path)?;
    if !fs::canonical_or_missing(&path)?.starts_with(&root) {
        return Err(RuntimeError::policy("Target path escapes root"));
    }
    Ok(path)
}
pub fn ownable(value: &str) -> bool {
    value != MANIFEST
        && !value.contains("__pycache__")
        && !value.ends_with(".pyc")
        && !STATE_EXCLUDES.iter().any(|excluded| {
            value == excluded.strip_suffix('/').unwrap_or(excluded) || value.starts_with(excluded)
        })
}
pub fn read_plan(path: &Path) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut result = vec![];
    for line in fs::read_text(path)?.lines() {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        let relative = normalize(line)?;
        if seen.insert(relative.clone()) {
            result.push(relative);
        }
    }
    if result.is_empty() {
        return Err(RuntimeError::invalid(format!(
            "write plan is empty: {}",
            path.display()
        )));
    }
    Ok(result)
}
pub fn hash_plan(target: &Path, plan: &[String]) -> Result<BTreeMap<String, String>> {
    plan.iter()
        .map(|relative| {
            if !ownable(relative) {
                return Err(RuntimeError::policy(format!(
                    "write plan contains non-ownable runtime state: {relative}"
                )));
            }
            let bytes = fs::read_regular(&confined(target, relative)?)?;
            Ok((relative.clone(), sha(&bytes)))
        })
        .collect()
}
pub fn hash_sources(
    staging: &Path,
    plan: &[String],
    target: &Path,
    sources: &Value,
    decisions: &Value,
) -> Result<BTreeMap<String, String>> {
    let sources = sources
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Source map must be an object"))?;
    if sources.keys().any(|key| !plan.contains(key)) {
        return Err(RuntimeError::policy(
            "Source map contains paths outside write plan",
        ));
    }
    let mut output = BTreeMap::new();
    for relative in plan {
        if !ownable(relative) {
            return Err(RuntimeError::policy(format!(
                "write plan contains non-ownable runtime state: {relative}"
            )));
        }
        let source = sources
            .get(relative)
            .and_then(Value::as_str)
            .unwrap_or("staging");
        let decision = &decisions[relative];
        let root = match source {
            "staging" => {
                if decision["decision"] == "kept" {
                    return Err(RuntimeError::policy(
                        "Kept decision must hash final target bytes",
                    ));
                }
                staging
            }
            "target" => {
                if decision["decision"] != "kept" {
                    return Err(RuntimeError::policy(
                        "Target final-content source requires a kept decision",
                    ));
                }
                target
            }
            _ => {
                return Err(RuntimeError::invalid(format!(
                    "Invalid final source: {source}"
                )));
            }
        };
        output.insert(
            relative.clone(),
            sha(&fs::read_regular(&confined(root, relative)?)?),
        );
    }
    Ok(output)
}
pub fn validate_decisions(decisions: &Value, files: &BTreeMap<String, String>) -> Result<()> {
    let entries = decisions
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("decisions must be an object"))?;
    let structured = [
        "origin",
        "strategy",
        "proposal_sha256",
        "resolved_sha256",
        "requirements",
    ];
    for (relative, entry) in entries {
        if !files.contains_key(relative) {
            return Err(RuntimeError::invalid(
                "decisions contain paths outside manifest membership",
            ));
        }
        if !entry.is_object()
            || !matches!(text(&entry["decision"]), "kept" | "merged")
            || !matching(r"^TASK-\d+$", text(&entry["task"]))
            || !hex(text(&entry["rejected_sha256"]), 64)
        {
            return Err(RuntimeError::invalid(format!(
                "Invalid legacy decision: {relative}"
            )));
        }
        if structured.iter().any(|key| entry.get(key).is_some()) {
            if structured.iter().any(|key| entry.get(key).is_none())
                || !matches!(
                    text(&entry["origin"]),
                    "generated" | "preexisting-team" | "shared"
                )
                || !matches!(
                    text(&entry["strategy"]),
                    "keep" | "append-requirements" | "manual-merge"
                )
                || entry["proposal_sha256"] != entry["rejected_sha256"]
                || !hex(text(&entry["proposal_sha256"]), 64)
                || !hex(text(&entry["resolved_sha256"]), 64)
                || text(&entry["resolved_sha256"]) != files[relative]
                || !string_list(&entry["requirements"], true)
            {
                return Err(RuntimeError::invalid(format!(
                    "Invalid structured decision: {relative}"
                )));
            }
            let requirements = texts(&entry["requirements"]);
            if requirements
                != requirements
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
            {
                return Err(RuntimeError::invalid(
                    "Decision requirements must be sorted and unique",
                ));
            }
            let strategy = text(&entry["strategy"]);
            if (strategy == "keep") != (entry["decision"] == "kept") {
                return Err(RuntimeError::invalid(
                    "Decision strategy contradicts decision",
                ));
            }
            let origin = text(&entry["origin"]);
            if strategy == "append-requirements" {
                if origin != "shared" || requirements.is_empty() {
                    return Err(RuntimeError::invalid(
                        "append-requirements requires shared origin and at least one requirement",
                    ));
                }
            } else if origin == "shared" {
                if strategy != "keep" || requirements.is_empty() {
                    return Err(RuntimeError::invalid(
                        "Shared origin requires keep/append strategy and requirements",
                    ));
                }
            } else if !requirements.is_empty() {
                return Err(RuntimeError::invalid(
                    "Requirements are only valid with shared origin",
                ));
            }
        }
    }
    Ok(())
}
#[derive(Clone, Debug)]
pub struct ManifestRequest {
    pub generator: String,
    pub version: String,
    pub task: String,
    pub profile: String,
    pub editions: Vec<String>,
    pub mode: String,
    pub decisions: Value,
    pub generated_at: Option<String>,
}
pub fn build_manifest(
    target: &Path,
    plan: &[String],
    request: &ManifestRequest,
    hashes: Option<BTreeMap<String, String>>,
) -> Result<Value> {
    if !matches!(request.mode.as_str(), "full" | "merge")
        || !matching(r"^\d+\.\d+\.\d+$", &request.version)
        || !matching(r"^TASK-\d+$", &request.task)
    {
        return Err(RuntimeError::invalid(
            "Invalid manifest mode, generator version or task",
        ));
    }
    let plan = plan
        .iter()
        .map(|value| normalize(value))
        .collect::<Result<Vec<_>>>()?;
    let files = match hashes {
        Some(files) => {
            if files.keys().cloned().collect::<BTreeSet<_>>()
                != plan.iter().cloned().collect::<BTreeSet<_>>()
                || files.values().any(|value| !hex(value, 64))
            {
                return Err(RuntimeError::invalid(
                    "file_hashes must exactly match write plan",
                ));
            }
            files
        }
        None => hash_plan(target, &plan)?,
    };
    if request.decisions.is_object() {
        validate_decisions(&request.decisions, &files)?;
    }
    let mut manifest = object!({"manifest_version":1,"generator":request.generator,"generator_version":request.version,"task":request.task,"profile":request.profile,"generated_at":request.generated_at.clone().unwrap_or_else(infra_core::domain::utc_now),"editions":request.editions,"mode":request.mode,"files":files});
    if request
        .decisions
        .as_object()
        .is_some_and(|value| !value.is_empty())
    {
        manifest["decisions"] = request.decisions.clone();
    }
    Ok(manifest)
}
pub fn write_manifest(target: &Path, manifest: &Value) -> Result<PathBuf> {
    let path = confined(target, MANIFEST)?;
    write_json(&path, manifest)?;
    Ok(path)
}
pub fn load_manifest(target: &Path) -> Result<Value> {
    let path = confined(target, MANIFEST)?;
    if !path.is_file() {
        return Err(RuntimeError::policy(
            ".infra-manifest.json missing; legacy target ownership is unknown, refusing update",
        ));
    }
    read_object(&path)
}
pub fn staged_files(staging: &Path) -> Result<BTreeMap<String, PathBuf>> {
    walk_files(staging)?
        .into_iter()
        .map(|path| Ok((relative(staging, &path)?, path)))
        .collect()
}
pub fn classify_update(target: &Path, staging: &Path, manifest: &Value) -> Result<Vec<Value>> {
    let files = manifest["files"]
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Manifest files must be an object"))?;
    let decisions = manifest.get("decisions").cloned().unwrap_or(object!({}));
    if !decisions.is_object() {
        return Err(RuntimeError::invalid(
            "Manifest decisions must be an object",
        ));
    }
    if decisions
        .as_object()
        .is_some_and(|values| values.values().any(|value| !value.is_object()))
    {
        return Err(RuntimeError::invalid(
            "Manifest decision entry must be an object",
        ));
    }
    let mut staged = staged_files(staging)?;
    staged.remove(MANIFEST);
    let paths = files
        .keys()
        .chain(staged.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut output = vec![];
    for relative in paths {
        let destination = confined(target, &relative)?;
        let expected = files.get(&relative);
        let proposal = staged.get(&relative);
        let decision = decisions.get(&relative);
        let (classification, reason) = if let Some(expected) = expected {
            if proposal.is_none() {
                if !destination.is_file() {
                    ("requires-decision", "tracked-file-deleted".into())
                } else if sha(&fs::read_regular(&destination)?) == text(expected) {
                    ("obsolete", "no-longer-generated".into())
                } else {
                    ("requires-decision", "tracked-file-modified".into())
                }
            } else if !destination.is_file() {
                ("requires-decision", "tracked-file-deleted".into())
            } else if sha(&fs::read_regular(&destination)?) != text(expected) {
                ("requires-decision", "tracked-file-modified".into())
            } else if let (Some(decision), Some(proposal)) = (decision, proposal) {
                let proposal = sha(&fs::read_regular(proposal)?);
                if proposal
                    == text(
                        decision
                            .get("proposal_sha256")
                            .unwrap_or(&decision["rejected_sha256"]),
                    )
                {
                    (
                        "standing-decision-honored",
                        text(decision.get("strategy").unwrap_or(&decision["decision"])).to_string(),
                    )
                } else {
                    ("requires-decision", "generator-output-changed".into())
                }
            } else {
                ("safe-update", "manifest-hash-unchanged".into())
            }
        } else if destination.exists() {
            ("requires-decision", "new-file-collision".into())
        } else {
            ("new-file", "new-generated-path".into())
        };
        output.push(object!({"path":relative,"classification":classification,"reason":reason}));
    }
    Ok(output)
}

/// Rehash exactly the existing owned membership after explicit content repairs.
/// Missing files retain their prior hash and are returned for escalation.
pub fn refresh_manifest(target: &Path) -> Result<Value> {
    let target = resolve_target(target)?;
    let mut custody = infra_core::bank::publication::PublicationFiles::new(&target)?;
    let raw = custody
        .read(MANIFEST, 16 * 1024 * 1024)?
        .ok_or_else(|| RuntimeError::not_found("Manifest missing"))?;
    let mode = custody
        .read_mode(MANIFEST)?
        .ok_or_else(|| RuntimeError::invalid("Manifest mode unavailable"))?;
    let mut manifest = infra_core::domain::json::parse(
        std::str::from_utf8(&raw).map_err(|_| RuntimeError::invalid("Manifest is not UTF-8"))?,
    )?;
    let names = manifest["files"]
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Manifest files must be an object"))?
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let mut missing = vec![];
    let mut updated = 0usize;
    let mut identities = BTreeMap::new();
    for name in &names {
        normalize(name)?;
        if !ownable(name) {
            return Err(RuntimeError::policy(format!(
                "Manifest includes non-ownable membership: {name}"
            )));
        }
        if !hex(text(&manifest["files"][name]), 64) {
            return Err(RuntimeError::invalid(format!(
                "Manifest hash invalid: {name}"
            )));
        }
        let before = custody.image_identity(name)?;
        let bytes = custody.read(name, 512 * 1024 * 1024)?;
        if custody.image_identity(name)? != before {
            return Err(RuntimeError::conflict(
                "Owned image changed while refreshing manifest",
            ));
        }
        identities.insert(name.clone(), before);
        if let Some(bytes) = bytes {
            manifest["files"][name] = sha(&bytes).into();
            updated += 1;
        } else {
            missing.push(name.clone());
        }
    }
    for (name, identity) in identities {
        if custody.image_identity(&name)? != identity {
            return Err(RuntimeError::conflict(format!(
                "Owned image changed before manifest refresh commit: {name}"
            )));
        }
    }
    manifest["generated_at"] = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
        .into();
    let output = (infra_core::domain::json::serialize(&manifest, false, true, true, false)? + "\n")
        .into_bytes();
    custody.write_expected_mode_checked(
        MANIFEST,
        &output,
        Some(&raw),
        Some(mode),
        mode,
        16 * 1024 * 1024,
    )?;
    custody.check()?;
    Ok(
        object!({"manifest":target.join(MANIFEST),"listed_count":names.len(),"updated_count":updated,"missing":missing,"membership_preserved":true}),
    )
}
