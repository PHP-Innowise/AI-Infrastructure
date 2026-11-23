use crate::common::*;
use infra_core::{Result, RuntimeError, fs};
use serde::{Deserialize, Serialize};
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
        let mut row = object!({"path":relative,"classification":classification,"reason":reason});
        if classification == "safe-update"
            && let (Some(expected), Some(proposal)) = (expected, proposal)
            && manifest
                .get("validation_stamps")
                .and_then(|stamps| stamps.get(&relative))
                .and_then(|stamp| stamp.get("sha256"))
                .and_then(Value::as_str)
                .is_some_and(|stamped| {
                    stamped == text(expected)
                        && fs::read_regular(proposal)
                            .ok()
                            .is_some_and(|bytes| sha(&bytes) == stamped)
                })
        {
            row["verified_unchanged"] = true.into();
        }
        output.push(row);
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
            let next_hash = sha(&bytes);
            let prev_hash = manifest["files"][name].as_str().map(str::to_owned);
            manifest["files"][name] = next_hash.clone().into();
            if prev_hash.as_deref() != Some(next_hash.as_str())
                && let Some(stamps) = manifest
                    .get_mut("validation_stamps")
                    .and_then(Value::as_object_mut)
            {
                stamps.remove(name);
            }
            updated += 1;
        } else {
            if let Some(stamps) = manifest
                .get_mut("validation_stamps")
                .and_then(Value::as_object_mut)
            {
                stamps.remove(name);
            }
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

/// Records per-artifact validation stamps inside `.infra-manifest.json` so subsequent
/// `infra-update` runs can skip unchanged verified slices.
pub fn stamp_validated_artifacts(
    manifest: &mut Value,
    stamps: &BTreeMap<String, String>,
    slice_tag: Option<&str>,
) -> Result<()> {
    let files = manifest["files"]
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Manifest files must be an object"))?
        .clone();
    let mut existing = manifest
        .get("validation_stamps")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    existing.retain(|path, _| files.contains_key(path));
    let verified_at = infra_core::domain::utc_now();
    for (rel, digest) in stamps {
        let rel = normalize(rel)?;
        if !files.contains_key(&rel) {
            return Err(RuntimeError::policy(format!(
                "validation stamp references non-manifest path: {rel}"
            )));
        }
        if !hex(digest, 64) {
            return Err(RuntimeError::invalid(format!(
                "validation stamp sha256 invalid: {rel}"
            )));
        }
        let mut entry = object!({
            "sha256": digest,
            "verified_at": &verified_at,
        });
        if let Some(tag) = slice_tag.filter(|t| !t.trim().is_empty()) {
            entry["slice"] = tag.into();
        }
        existing.insert(rel, entry);
    }
    manifest["validation_stamps"] = Value::Object(existing);
    Ok(())
}

/// Stamps every manifest-owned artifact in `<target>/.infra-manifest.json` after
/// `validate-generated` passes, avoiding rewrites if matching stamps already exist.
pub fn stamp_manifest_if_needed(target: &Path, slice_tag: Option<&str>) -> Result<bool> {
    let mut manifest = load_manifest(target)?;
    let files = strings_map(&manifest["files"])?;
    if files.is_empty() {
        return Ok(false);
    }
    let normalized_tag = slice_tag.filter(|t| !t.trim().is_empty());
    let already_stamped = manifest
        .get("validation_stamps")
        .and_then(Value::as_object)
        .is_some_and(|existing| {
            existing.len() == files.len()
                && files.iter().all(|(rel, digest)| {
                    existing.get(rel).is_some_and(|entry| {
                        entry.get("sha256").and_then(Value::as_str) == Some(digest.as_str())
                            && entry
                                .get("verified_at")
                                .and_then(Value::as_str)
                                .is_some_and(|ts| !ts.trim().is_empty())
                            && normalized_tag.is_none_or(|tag| {
                                entry.get("slice").and_then(Value::as_str) == Some(tag)
                            })
                    })
                })
        });
    if already_stamped {
        return Ok(false);
    }
    stamp_validated_artifacts(&mut manifest, &files, normalized_tag)?;
    write_manifest(target, &manifest)?;
    Ok(true)
}

/// Per-surface dependency manifest stored in `.infra-manifest.json` so `infra-update`
/// can skip unchanged skill/agent batches pre-generation and always invalidate dependent
/// slices when watched source globs or composer/package manifests change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtifactDependencyManifest {
    pub skill_paths: Vec<String>,
    pub watched_globs: Vec<String>,
    pub manifest_files: Vec<String>,
    pub path_hashes: BTreeMap<String, String>,
}

const DEFAULT_PACKAGE_MANIFESTS: &[&str] = &[
    "composer.json",
    "package.json",
    "tsconfig.json",
    "pyproject.toml",
    "go.mod",
    "Cargo.toml",
];

fn walk_watched_target_files(
    target: &Path,
    watched_globs: &[String],
) -> Result<BTreeMap<String, String>> {
    const MAX_WATCHED_FILES: usize = 4096;
    let mut found = BTreeMap::new();
    if watched_globs.is_empty() {
        return Ok(found);
    }
    let mut pending = vec![target.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let mut entries = std::fs::read_dir(&dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path)?;
            if fs::linked(&meta) {
                if meta.is_file() || meta.is_dir() {
                    let rel = relative(target, &path)?;
                    if watched_globs
                        .iter()
                        .any(|g| crate::coverage::matches(g, "file", &rel).unwrap_or(false))
                    {
                        return Err(RuntimeError::policy(format!(
                            "watched dependency path contains symlink: {rel}"
                        )));
                    }
                }
                continue;
            }
            let rel = relative(target, &path)?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if meta.is_dir() {
                if matches!(
                    name_str.as_ref(),
                    ".git" | "vendor" | "node_modules" | "target"
                ) {
                    continue;
                }
                pending.push(path);
            } else if meta.is_file()
                && watched_globs
                    .iter()
                    .any(|g| crate::coverage::matches(g, "file", &rel).unwrap_or(false))
            {
                if found.len() >= MAX_WATCHED_FILES {
                    return Err(RuntimeError::policy(
                        "watched surface exceeds 4096 matched dependency files",
                    ));
                }
                let bytes = fs::read_regular(&path)?;
                found.insert(rel, sha(&bytes));
            }
        }
    }
    Ok(found)
}

fn hash_dependency_surface(
    target: &Path,
    dep: &ArtifactDependencyManifest,
) -> Result<BTreeMap<String, String>> {
    let target = resolve_target(target)?;
    let mut hashes = walk_watched_target_files(&target, &dep.watched_globs)?;
    for mf in &dep.manifest_files {
        let rel = normalize(mf)?;
        let path = confined(&target, &rel)?;
        if path.is_file() {
            let bytes = fs::read_regular(&path)?;
            hashes.insert(rel, sha(&bytes));
        } else if path.exists() {
            return Err(RuntimeError::policy(format!(
                "manifest dependency is not a regular file: {rel}"
            )));
        } else {
            hashes.insert(rel, "__absent__".into());
        }
    }
    Ok(hashes)
}

/// Derives per-skill/agent [`ArtifactDependencyManifest`] records from a `skill-generation-plan.json`
/// and manifest-owned `files`, binding each batch to its watched source globs and package manifests.
pub fn build_dependency_manifests_from_plan(
    target: &Path,
    plan: &Value,
    manifest: &Value,
) -> Result<BTreeMap<String, ArtifactDependencyManifest>> {
    let target = resolve_target(target)?;
    let owned_files = strings_map(&manifest["files"])?;
    let mut detected_manifests: Vec<String> = DEFAULT_PACKAGE_MANIFESTS
        .iter()
        .filter(|name| target.join(name).is_file())
        .map(|name| (*name).to_string())
        .collect();
    for core_pkg in ["composer.json", "package.json"] {
        if !detected_manifests.iter().any(|m| m == core_pkg) {
            detected_manifests.push(core_pkg.into());
        }
    }
    detected_manifests.sort();
    detected_manifests.dedup();

    let mut out = BTreeMap::new();
    for skill in plan["skills"].as_array().into_iter().flatten() {
        let name = text(&skill["name"]).trim();
        if name.is_empty() {
            continue;
        }
        let mut skill_paths: Vec<String> = owned_files
            .keys()
            .filter(|rel| {
                rel.contains(&format!("/skills/{name}/"))
                    || rel.ends_with(&format!("/agents/{name}.md"))
                    || rel.ends_with(&format!("/agents/{name}-agent.md"))
                    || rel.ends_with(&format!("/commands/{name}.md"))
            })
            .cloned()
            .collect();
        skill_paths.sort();
        skill_paths.dedup();

        let mut watched_globs = BTreeSet::new();
        for p in texts(&skill["writes"]) {
            watched_globs.insert(p);
        }
        for row in skill["ownership"].as_array().into_iter().flatten() {
            for p in texts(&row["paths"]) {
                watched_globs.insert(p);
            }
        }
        for row in skill["path_contracts"].as_array().into_iter().flatten() {
            if nonempty(&row["path"]) {
                watched_globs.insert(text(&row["path"]).trim().to_string());
            }
        }

        let dep = ArtifactDependencyManifest {
            skill_paths,
            watched_globs: watched_globs.into_iter().collect(),
            manifest_files: detected_manifests.clone(),
            path_hashes: BTreeMap::new(),
        };
        out.insert(name.to_string(), dep);
    }
    Ok(out)
}

/// Records per-surface [`ArtifactDependencyManifest`] metadata and live target digests into
/// `.infra-manifest.json` (`dependency_manifests`).
pub fn record_dependency_manifests(
    manifest: &mut Value,
    target: &Path,
    dependencies: &BTreeMap<String, ArtifactDependencyManifest>,
) -> Result<()> {
    let files = manifest["files"]
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Manifest files must be an object"))?
        .clone();
    let mut encoded = serde_json::Map::new();
    for (surface, dep) in dependencies {
        if surface.trim().is_empty() {
            return Err(RuntimeError::invalid(
                "Dependency manifest surface name must not be empty",
            ));
        }
        let mut normalized = ArtifactDependencyManifest {
            skill_paths: dep
                .skill_paths
                .iter()
                .map(|p| normalize(p))
                .collect::<Result<Vec<_>>>()?,
            watched_globs: dep.watched_globs.clone(),
            manifest_files: dep
                .manifest_files
                .iter()
                .map(|p| normalize(p))
                .collect::<Result<Vec<_>>>()?,
            path_hashes: BTreeMap::new(),
        };
        for sp in &normalized.skill_paths {
            if !files.contains_key(sp) {
                return Err(RuntimeError::policy(format!(
                    "dependency manifest references non-manifest skill path: {sp}"
                )));
            }
        }
        normalized.skill_paths.sort();
        normalized.skill_paths.dedup();
        normalized.watched_globs.sort();
        normalized.watched_globs.dedup();
        normalized.manifest_files.sort();
        normalized.manifest_files.dedup();
        normalized.path_hashes = hash_dependency_surface(target, &normalized)?;
        encoded.insert(surface.clone(), serde_json::to_value(&normalized)?);
    }
    manifest["dependency_manifests"] = Value::Object(encoded);
    Ok(())
}

/// Evaluates per-surface [`ArtifactDependencyManifest`] entries in `.infra-manifest.json`
/// before generation/staging so `infra-update` can skip unchanged skill/agent batches while
/// always invalidating dependent slices when watched files or composer/package manifests change.
pub fn partition_pre_generation_batches(
    target: &Path,
    manifest: &Value,
    changed_paths: Option<&[String]>,
) -> Result<Value> {
    let target = resolve_target(target)?;
    let files = strings_map(&manifest["files"])?;
    let stamps = manifest.get("validation_stamps").and_then(Value::as_object);
    let mut skipped_batches = Vec::new();
    let mut pending_batches = Vec::new();
    let mut skipped_skill_paths = BTreeSet::new();
    let mut invalidated = BTreeMap::<String, String>::new();

    let Some(deps) = manifest
        .get("dependency_manifests")
        .and_then(Value::as_object)
    else {
        return Ok(object!({
            "skipped_batches": skipped_batches,
            "pending_batches": pending_batches,
            "skipped_skill_paths": [],
            "invalidated": invalidated,
        }));
    };

    let normalized_changed: Option<Vec<String>> = changed_paths.map(|paths| {
        paths
            .iter()
            .filter_map(|p| normalize(p).ok())
            .collect::<Vec<_>>()
    });

    for (surface, raw_dep) in deps {
        let Ok(dep) = serde_json::from_value::<ArtifactDependencyManifest>(raw_dep.clone()) else {
            pending_batches.push(surface.clone());
            invalidated.insert(surface.clone(), "invalid-dependency-manifest".into());
            continue;
        };
        if dep.skill_paths.is_empty() {
            pending_batches.push(surface.clone());
            invalidated.insert(surface.clone(), "no-skill-paths".into());
            continue;
        }
        let artifacts_valid = dep.skill_paths.iter().all(|rel| {
            let Some(expected) = files.get(rel) else {
                return false;
            };
            let Some(stamped) = stamps
                .and_then(|map| map.get(rel))
                .and_then(|entry| entry.get("sha256"))
                .and_then(Value::as_str)
            else {
                return false;
            };
            if stamped != expected.as_str() {
                return false;
            }
            confined(&target, rel)
                .and_then(|path| fs::read_regular(&path))
                .is_ok_and(|bytes| sha(&bytes) == *expected)
        });
        if !artifacts_valid {
            pending_batches.push(surface.clone());
            invalidated.insert(surface.clone(), "artifact-stamp-missing-or-drifted".into());
            continue;
        }

        if let Some(ref changed) = normalized_changed {
            if let Some(hit) = changed.iter().find(|ch| {
                dep.manifest_files.iter().any(|mf| {
                    mf == *ch || crate::coverage::matches(mf, "file", ch).unwrap_or(false)
                })
            }) {
                pending_batches.push(surface.clone());
                invalidated.insert(surface.clone(), format!("manifest-changed:{hit}"));
                continue;
            }
            if let Some(hit) = changed.iter().find(|ch| {
                dep.skill_paths.contains(ch)
                    || dep.watched_globs.iter().any(|g| {
                        crate::coverage::matches(g, "file", ch).unwrap_or(false)
                            || crate::coverage::matches(g, "tree", ch).unwrap_or(false)
                    })
            }) {
                pending_batches.push(surface.clone());
                invalidated.insert(surface.clone(), format!("watched-path-changed:{hit}"));
                continue;
            }
        }

        let live_hashes = match hash_dependency_surface(&target, &dep) {
            Ok(h) => h,
            Err(err) => {
                pending_batches.push(surface.clone());
                invalidated.insert(surface.clone(), format!("dependency-unreadable:{err}"));
                continue;
            }
        };
        let manifest_drift = dep.manifest_files.iter().find(|mf| {
            live_hashes.get(*mf) != dep.path_hashes.get(*mf) || !dep.path_hashes.contains_key(*mf)
        });
        if let Some(mf) = manifest_drift {
            pending_batches.push(surface.clone());
            invalidated.insert(surface.clone(), format!("manifest-drifted:{mf}"));
            continue;
        }
        if live_hashes != dep.path_hashes {
            pending_batches.push(surface.clone());
            invalidated.insert(surface.clone(), "watched-surface-drifted".into());
            continue;
        }

        skipped_batches.push(surface.clone());
        skipped_skill_paths.extend(dep.skill_paths);
    }

    Ok(object!({
        "skipped_batches": skipped_batches,
        "pending_batches": pending_batches,
        "skipped_skill_paths": skipped_skill_paths.into_iter().collect::<Vec<_>>(),
        "invalidated": invalidated,
    }))
}

pub(crate) fn dependency_invalidated_skill_paths(
    target: &Path,
    manifest: &Value,
) -> Result<BTreeSet<String>> {
    let Some(deps) = manifest
        .get("dependency_manifests")
        .and_then(Value::as_object)
    else {
        return Ok(BTreeSet::new());
    };
    let partition = partition_pre_generation_batches(target, manifest, None)?;
    let pending: BTreeSet<String> = texts(&partition["pending_batches"]).into_iter().collect();
    let mut blocked = BTreeSet::new();
    for surface in pending {
        if let Some(raw) = deps.get(&surface)
            && let Ok(dep) = serde_json::from_value::<ArtifactDependencyManifest>(raw.clone())
        {
            blocked.extend(dep.skill_paths);
        }
    }
    Ok(blocked)
}

/// Returns owned relative paths whose on-disk target hash, staged proposal hash, and
/// `.infra-manifest.json` validation stamp all match (and whose declared dependency manifest
/// has not drifted), allowing `infra-update` to skip them.
pub fn unchanged_verified_slices(
    target: &Path,
    staging: &Path,
    manifest: &Value,
) -> Result<Vec<String>> {
    let invalidated = dependency_invalidated_skill_paths(target, manifest)?;
    let entries = classify_update(target, staging, manifest)?;
    Ok(entries
        .into_iter()
        .filter(|entry| entry["verified_unchanged"] == true)
        .filter_map(|entry| entry["path"].as_str().map(str::to_owned))
        .filter(|path| !invalidated.contains(path))
        .collect())
}

#[cfg(test)]
mod manifest_stamp_tests {
    use super::*;

    #[test]
    fn manifest_validation_stamps_skip_unchanged_slices_and_invalidate_on_drift() {
        let dir = std::env::temp_dir().join(format!(
            "infra-manifest-stamps-{}",
            infra_core::domain::uuid4()
        ));
        let target = dir.join("target");
        let staging = dir.join("staging");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        let a_bytes = b"# Policy unchanged\n";
        let b_bytes_v1 = b"# Changed skill v1\n";
        let b_bytes_v2 = b"# Changed skill v2\n";
        std::fs::write(target.join("AGENTS.md"), a_bytes).unwrap();
        std::fs::write(staging.join("AGENTS.md"), a_bytes).unwrap();
        std::fs::write(target.join("SKILL.md"), b_bytes_v1).unwrap();
        std::fs::write(staging.join("SKILL.md"), b_bytes_v2).unwrap();

        let plan = vec!["AGENTS.md".to_owned(), "SKILL.md".to_owned()];
        let mut manifest = build_manifest(
            &target,
            &plan,
            &ManifestRequest {
                generator: "test".into(),
                version: "1.0.0".into(),
                task: "TASK-1".into(),
                profile: "php-core".into(),
                editions: vec!["codex".into()],
                mode: "merge".into(),
                decisions: object!({}),
                generated_at: None,
            },
            None,
        )
        .unwrap();

        let stamps = BTreeMap::from([
            ("AGENTS.md".to_owned(), sha(a_bytes)),
            ("SKILL.md".to_owned(), sha(b_bytes_v1)),
        ]);
        stamp_validated_artifacts(&mut manifest, &stamps, Some("batch-1")).unwrap();
        write_manifest(&target, &manifest).unwrap();

        let unchanged = unchanged_verified_slices(&target, &staging, &manifest).unwrap();
        assert_eq!(unchanged, vec!["AGENTS.md".to_owned()]);

        // Editing AGENTS.md on disk + refreshing manifest prunes stale stamp cleanly.
        std::fs::write(target.join("AGENTS.md"), b"# Policy edited\n").unwrap();
        refresh_manifest(&target).unwrap();
        let refreshed = load_manifest(&target).unwrap();
        assert!(
            refreshed["validation_stamps"].get("AGENTS.md").is_none(),
            "modified artifact stamp must be pruned on refresh"
        );
        let unchanged_after = unchanged_verified_slices(&target, &staging, &refreshed).unwrap();
        assert!(unchanged_after.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dependency_manifests_skip_unchanged_batches_pre_gen_and_invalidate_on_manifest_or_glob_drift()
     {
        let dir = std::env::temp_dir().join(format!(
            "infra-dep-manifests-{}",
            infra_core::domain::uuid4()
        ));
        let target = dir.join("target");
        let staging = dir.join("staging");
        std::fs::create_dir_all(target.join(".codex/skills/orders")).unwrap();
        std::fs::create_dir_all(target.join(".codex/skills/billing")).unwrap();
        std::fs::create_dir_all(target.join("src/Orders")).unwrap();
        std::fs::create_dir_all(target.join("src/Billing")).unwrap();
        std::fs::create_dir_all(staging.join(".codex/skills/orders")).unwrap();
        std::fs::create_dir_all(staging.join(".codex/skills/billing")).unwrap();

        std::fs::write(target.join("composer.json"), r#"{"name":"acme/shop"}"#).unwrap();
        std::fs::write(target.join("package.json"), r#"{"name":"shop-ui"}"#).unwrap();
        std::fs::write(target.join("src/Orders/Order.php"), "<?php\n").unwrap();
        std::fs::write(target.join("src/Billing/Invoice.php"), "<?php\n").unwrap();
        let orders_skill = b"# Orders skill\n";
        let billing_skill = b"# Billing skill\n";
        std::fs::write(target.join(".codex/skills/orders/SKILL.md"), orders_skill).unwrap();
        std::fs::write(target.join(".codex/skills/billing/SKILL.md"), billing_skill).unwrap();
        std::fs::write(staging.join(".codex/skills/orders/SKILL.md"), orders_skill).unwrap();
        std::fs::write(
            staging.join(".codex/skills/billing/SKILL.md"),
            billing_skill,
        )
        .unwrap();

        let owned = vec![
            ".codex/skills/orders/SKILL.md".to_owned(),
            ".codex/skills/billing/SKILL.md".to_owned(),
        ];
        let mut manifest = build_manifest(
            &target,
            &owned,
            &ManifestRequest {
                generator: "Infrastructure-Creator".into(),
                version: "1.0.0".into(),
                task: "TASK-10".into(),
                profile: "php-core".into(),
                editions: vec!["codex".into()],
                mode: "merge".into(),
                decisions: object!({}),
                generated_at: None,
            },
            None,
        )
        .unwrap();
        let stamps = BTreeMap::from([
            (
                ".codex/skills/orders/SKILL.md".to_owned(),
                sha(orders_skill),
            ),
            (
                ".codex/skills/billing/SKILL.md".to_owned(),
                sha(billing_skill),
            ),
        ]);
        stamp_validated_artifacts(&mut manifest, &stamps, Some("batch")).unwrap();

        let deps = BTreeMap::from([
            (
                "orders".to_owned(),
                ArtifactDependencyManifest {
                    skill_paths: vec![".codex/skills/orders/SKILL.md".into()],
                    watched_globs: vec!["src/Orders/**".into()],
                    manifest_files: vec!["composer.json".into()],
                    path_hashes: BTreeMap::new(),
                },
            ),
            (
                "billing".to_owned(),
                ArtifactDependencyManifest {
                    skill_paths: vec![".codex/skills/billing/SKILL.md".into()],
                    watched_globs: vec!["src/Billing/**".into()],
                    manifest_files: vec!["package.json".into()],
                    path_hashes: BTreeMap::new(),
                },
            ),
        ]);
        record_dependency_manifests(&mut manifest, &target, &deps).unwrap();

        // Both batches are skippable pre-generation when nothing changed.
        let clean = partition_pre_generation_batches(&target, &manifest, None).unwrap();
        assert_eq!(clean["skipped_batches"], object!(["billing", "orders"]));
        assert_eq!(clean["pending_batches"], object!([]));

        // Watched-glob change via explicit changed_paths invalidates only orders.
        let orders_changed = partition_pre_generation_batches(
            &target,
            &manifest,
            Some(&["src/Orders/Order.php".to_string()]),
        )
        .unwrap();
        assert_eq!(orders_changed["skipped_batches"], object!(["billing"]));
        assert_eq!(orders_changed["pending_batches"], object!(["orders"]));

        // Composer manifest change on disk invalidates composer-dependent orders batch pre-gen AND post-staging.
        std::fs::write(
            target.join("composer.json"),
            r#"{"name":"acme/shop","require":{"php":"^8.3"}}"#,
        )
        .unwrap();
        let composer_drift = partition_pre_generation_batches(&target, &manifest, None).unwrap();
        assert_eq!(composer_drift["skipped_batches"], object!(["billing"]));
        assert_eq!(composer_drift["pending_batches"], object!(["orders"]));

        let post_staging =
            crate::coverage::partition_update_slices(&target, &staging, &manifest).unwrap();
        assert_eq!(
            post_staging["skipped_unchanged"],
            object!([".codex/skills/billing/SKILL.md"])
        );
        assert_eq!(
            post_staging["pending_slices"],
            object!([".codex/skills/orders/SKILL.md"])
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
