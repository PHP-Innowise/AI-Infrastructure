use crate::{
    common::*,
    ownership::{self, MANIFEST},
};
use infra_core::bank::publication::PublicationFiles;
use infra_core::{Result, RuntimeError, fs};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
const LIMIT: usize = 512 * 1024 * 1024;
const JOURNAL_LIMIT: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum FileState {
    Missing,
    File { sha256: String, mode: u32 },
    Symlink { target: String, sha256: String },
}
#[derive(Clone)]
struct Image {
    bytes: Option<Vec<u8>>,
    mode: Option<u32>,
    identity: Value,
    state: Value,
}
pub fn mode(metadata: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        if metadata.permissions().readonly() {
            0o444
        } else {
            0o644
        }
    }
}
pub fn set_mode(path: &Path, mode: u32) -> Result<()> {
    let path = fs::absolute(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| RuntimeError::policy("Mode path has no parent"))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| RuntimeError::policy("Mode name is not UTF-8"))?;
    PublicationFiles::new(parent)?.set_mode(name, mode)
}
fn normalized(values: &[String]) -> Result<Vec<String>> {
    let paths = values
        .iter()
        .map(|value| ownership::normalize(value))
        .collect::<Result<Vec<_>>>()?;
    if paths.iter().collect::<BTreeSet<_>>().len() != paths.len() {
        return Err(RuntimeError::invalid("Publication paths must be unique"));
    }
    Ok(paths)
}
fn observe(files: &mut PublicationFiles, name: &str, watch: bool) -> Result<Image> {
    ownership::normalize(name)?;
    if watch {
        let mut state = files.watch_image(name, LIMIT)?;
        let identity = state
            .as_object_mut()
            .ok_or_else(|| RuntimeError::invalid("Invalid native observation"))?
            .remove("identity")
            .unwrap_or(Value::Null);
        return Ok(Image {
            bytes: None,
            mode: state["mode"].as_u64().map(|value| value as u32),
            identity,
            state,
        });
    }
    let before = files.image_identity(name)?;
    let bytes = files.read(name, LIMIT)?;
    let mode = files.read_mode(name)?;
    let after = files.image_identity(name)?;
    if before != after {
        return Err(RuntimeError::conflict(
            "Publication image changed while capturing baseline",
        ));
    }
    let state = match &bytes {
        Some(bytes) => object!({"state":"file","sha256":sha(bytes),"mode":mode}),
        None => object!({"state":"missing"}),
    };
    Ok(Image {
        bytes,
        mode,
        identity: object!(after),
        state,
    })
}
pub fn file_state(root: &Path, relative: &str, allow_final_symlink: bool) -> Result<Value> {
    let root = ownership::resolve_target(root)?;
    observe(
        &mut PublicationFiles::new(&root)?,
        relative,
        allow_final_symlink,
    )
    .map(|image| image.state)
}
pub fn planned_paths(path: &Path) -> Result<Vec<String>> {
    let mut paths = ownership::read_plan(path)?;
    if !paths.iter().any(|path| path == MANIFEST) {
        paths.push(MANIFEST.into());
    }
    Ok(paths)
}
pub fn removal_paths(path: Option<&Path>) -> Result<Vec<String>> {
    let paths = path
        .map(ownership::read_plan)
        .transpose()?
        .unwrap_or_default();
    if paths.iter().any(|path| path == MANIFEST) {
        return Err(RuntimeError::policy(
            "Removal plan must not contain manifest",
        ));
    }
    Ok(paths)
}
fn disjoint(paths: &[String], removals: &[String], watch: &[String]) -> Result<()> {
    normalized(paths)?;
    normalized(removals)?;
    normalized(watch)?;
    let published = paths.iter().chain(removals).collect::<BTreeSet<_>>();
    if paths.iter().any(|path| removals.contains(path))
        || watch.iter().any(|path| published.contains(path))
    {
        return Err(RuntimeError::policy(
            "Publication, removal and baseline-only plans must not overlap",
        ));
    }
    Ok(())
}
pub fn build_snapshot(target: &Path, paths: &[String], watch: &[String]) -> Result<Value> {
    disjoint(paths, &[], watch)?;
    let target = ownership::resolve_target(target)?;
    let mut files = PublicationFiles::new(&target)?;
    let mut states = BTreeMap::new();
    let mut identities = BTreeMap::new();
    for name in paths.iter().chain(watch) {
        let image = observe(&mut files, name, watch.contains(name))?;
        states.insert(name.clone(), image.state);
        identities.insert(name.clone(), image.identity);
    }
    files.check()?;
    Ok(
        object!({"schema_version":1,"target":path_text(&target)?,"files":states,"baseline_only":watch.iter().cloned().collect::<BTreeSet<_>>(),"custody":{"root":files.root_identity(),"directories":files.directory_identities()?,"images":identities}}),
    )
}
fn verify_with(
    files: &mut PublicationFiles,
    target: &Path,
    paths: &[String],
    snapshot: &Value,
    watch: &[String],
) -> Result<BTreeMap<String, Image>> {
    if snapshot["target"] != path_text(&ownership::resolve_target(target)?)?
        || snapshot["schema_version"] != 1
    {
        return Err(RuntimeError::conflict("Baseline target or version differs"));
    }
    if texts(&snapshot["baseline_only"])
        .into_iter()
        .collect::<BTreeSet<_>>()
        != watch.iter().cloned().collect()
    {
        return Err(RuntimeError::conflict("Baseline-only membership differs"));
    }
    let expected = snapshot["files"]
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Baseline files must be an object"))?;
    let names = paths.iter().chain(watch).cloned().collect::<BTreeSet<_>>();
    if expected.keys().cloned().collect::<BTreeSet<_>>() != names {
        return Err(RuntimeError::conflict("Baseline membership differs"));
    }
    if snapshot["custody"]["root"] != object!(files.root_identity()) {
        return Err(RuntimeError::conflict(
            "Baseline directory identity is absent or changed; take a fresh native snapshot",
        ));
    }
    let directories: BTreeMap<String, (u64, u64, u32)> =
        serde_json::from_value(snapshot["custody"]["directories"].clone())
            .map_err(|_| RuntimeError::invalid("Baseline directory custody malformed"))?;
    files.check_directory_identities(&directories)?;
    let mut result = BTreeMap::new();
    for name in names {
        let image = observe(files, &name, watch.contains(&name))?;
        if image.state != expected[&name] || image.identity != snapshot["custody"]["images"][&name]
        {
            return Err(RuntimeError::conflict(format!(
                "Target changed after collision review: {name}"
            )));
        }
        result.insert(name, image);
    }
    files.check()?;
    Ok(result)
}
pub fn verify_baseline(
    target: &Path,
    paths: &[String],
    snapshot: &Value,
    watch: &[String],
) -> Result<()> {
    verify_with(
        &mut PublicationFiles::new(&ownership::resolve_target(target)?)?,
        target,
        paths,
        snapshot,
        watch,
    )
    .map(|_| ())
}
fn members_bytes(raw: &[u8], label: &str) -> Result<BTreeSet<String>> {
    let content = std::str::from_utf8(raw)
        .map_err(|_| RuntimeError::invalid(format!("{label} manifest is not UTF-8")))?;
    let value = infra_core::domain::json::parse(content)?;
    value["files"]
        .as_object()
        .ok_or_else(|| RuntimeError::invalid(format!("{label} manifest membership malformed")))?
        .keys()
        .map(|name| ownership::normalize(name))
        .collect()
}
fn members(files: &mut PublicationFiles, label: &str) -> Result<BTreeSet<String>> {
    members_bytes(
        &files
            .read(MANIFEST, JOURNAL_LIMIT)?
            .ok_or_else(|| RuntimeError::invalid(format!("{label} manifest missing")))?,
        label,
    )
}
fn ownership_with(
    target: &mut PublicationFiles,
    staging: &mut PublicationFiles,
    paths: &[String],
    removals: &[String],
) -> Result<()> {
    let staged = members(staging, "Staged")?;
    for name in paths {
        if name == MANIFEST {
            continue;
        }
        if name.contains("__pycache__") || name.ends_with(".pyc") {
            return Err(RuntimeError::policy(
                "Publication plan contains runtime cache artifacts",
            ));
        }
        if ownership::ownable(name) {
            if !staged.contains(name) {
                return Err(RuntimeError::policy(format!(
                    "Publication path absent from staged manifest: {name}"
                )));
            }
        } else if target.read(name, LIMIT)?.is_some() {
            return Err(RuntimeError::policy(format!(
                "Publication would overwrite team-owned runtime state: {name}"
            )));
        }
    }
    if !removals.is_empty() {
        let owned = members(target, "Target")?;
        for name in removals {
            if !ownership::ownable(name) || !owned.contains(name) {
                return Err(RuntimeError::policy(format!(
                    "Removal is not target-manifest owned: {name}"
                )));
            }
        }
    }
    Ok(())
}
pub fn verify_plan_ownership(
    target: &Path,
    staging: &Path,
    paths: &[String],
    removals: &[String],
) -> Result<()> {
    ownership_with(
        &mut PublicationFiles::new(&ownership::resolve_target(target)?)?,
        &mut PublicationFiles::new(&ownership::resolve_target(staging)?)?,
        paths,
        removals,
    )
}
pub fn verify_staging(staging: &Path, paths: &[String]) -> Result<()> {
    let mut files = PublicationFiles::new(&ownership::resolve_target(staging)?)?;
    for name in normalized(paths)? {
        if observe(&mut files, &name, false)?.bytes.is_none() {
            return Err(RuntimeError::invalid(format!(
                "Staged file missing: {name}"
            )));
        }
    }
    files.check()
}
pub fn atomic_copy(source: &Path, destination: &Path) -> Result<()> {
    let source = fs::absolute(source)?;
    let destination = fs::absolute(destination)?;
    let source_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| RuntimeError::policy("Source has no UTF-8 filename"))?;
    let destination_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| RuntimeError::policy("Destination has no UTF-8 filename"))?;
    let mut staged = PublicationFiles::new(
        source
            .parent()
            .ok_or_else(|| RuntimeError::policy("Source has no parent"))?,
    )?;
    let source = observe(&mut staged, source_name, false)?;
    let raw = source
        .bytes
        .as_deref()
        .ok_or_else(|| RuntimeError::not_found("Copy source missing"))?;
    let mut target = PublicationFiles::new(
        destination
            .parent()
            .ok_or_else(|| RuntimeError::policy("Destination has no parent"))?,
    )?;
    let before = observe(&mut target, destination_name, false)?;
    target.write_expected_mode_checked(
        destination_name,
        raw,
        before.bytes.as_deref(),
        before.mode,
        source
            .mode
            .ok_or_else(|| RuntimeError::invalid("Copy source has no mode"))?,
        LIMIT,
    )?;
    staged.check()?;
    target.check()
}
fn journal_load(files: &mut PublicationFiles) -> Result<Value> {
    let raw = files
        .read("journal.json", JOURNAL_LIMIT)?
        .ok_or_else(|| RuntimeError::not_found("Rollback journal missing"))?;
    let value = infra_core::domain::json::parse(
        std::str::from_utf8(&raw).map_err(|_| RuntimeError::invalid("Journal is not UTF-8"))?,
    )?;
    if !value.is_object() {
        return Err(RuntimeError::invalid("Journal must be an object"));
    }
    Ok(value)
}
fn journal_save(files: &mut PublicationFiles, value: &Value) -> Result<()> {
    let bytes = (infra_core::domain::json::sorted_pretty(value)? + "\n").into_bytes();
    let before = files.read("journal.json", JOURNAL_LIMIT)?;
    files.write_expected_limit("journal.json", &bytes, before.as_deref(), JOURNAL_LIMIT)
}
fn create_journal(journal: &Path) -> Result<PublicationFiles> {
    let journal = fs::absolute(journal)?;
    let parent = journal
        .parent()
        .ok_or_else(|| RuntimeError::policy("Journal has no parent"))?;
    let name = journal
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| RuntimeError::policy("Invalid journal name"))?;
    let mut custody = PublicationFiles::new(parent)?;
    custody.create_directory(name)?;
    infra_core::security::secure_private_dir(&journal)?;
    let mut files = PublicationFiles::new(&journal)?;
    files.create_directory("backups")?;
    custody.check()?;
    Ok(files)
}
fn prepare_with(
    target: &mut PublicationFiles,
    target_path: &Path,
    paths: &[String],
    snapshot: &Value,
    journal: &mut PublicationFiles,
) -> Result<Value> {
    let mut baseline = BTreeMap::new();
    let mut total = 0usize;
    for name in paths {
        let image = observe(target, name, false)?;
        if image.state != snapshot["files"][name]
            || image.identity != snapshot["custody"]["images"][name]
        {
            return Err(RuntimeError::conflict(format!(
                "Target changed while preparing backups: {name}"
            )));
        }
        if let Some(raw) = image.bytes {
            total = total
                .checked_add(raw.len())
                .ok_or_else(|| RuntimeError::invalid("Backup size overflow"))?;
            if total > LIMIT {
                return Err(RuntimeError::invalid("Publication backups exceed 512 MiB"));
            }
            journal.write_expected_mode_checked(
                &format!("backups/{name}"),
                &raw,
                None,
                None,
                0o600,
                LIMIT,
            )?;
        }
        baseline.insert(name.clone(), image.state);
    }
    let value = object!({"schema_version":2,"target":path_text(target_path)?,"paths":paths,"baseline":baseline,"snapshot":snapshot,"created_dirs":[],"created_directory_identities":{},"effects":{},"target_directories":target.directory_identities()?,"status":"prepared"});
    journal_save(journal, &value)?;
    Ok(value)
}
pub fn prepare_journal(
    target: &Path,
    paths: &[String],
    journal: &Path,
    snapshot: &Value,
) -> Result<Value> {
    let target = ownership::resolve_target(target)?;
    let journal = ownership::resolve_target(journal)?;
    if target.starts_with(&journal) || journal.starts_with(&target) {
        return Err(RuntimeError::policy(
            "Rollback journal must be outside target",
        ));
    }
    let mut files = PublicationFiles::new(&target)?;
    verify_with(
        &mut files,
        &target,
        paths,
        snapshot,
        &texts(&snapshot["baseline_only"]),
    )?;
    let mut journal = create_journal(&journal)?;
    prepare_with(&mut files, &target, paths, snapshot, &mut journal)
}
fn prepared_matches(image: &Image, prepared: &Value) -> bool {
    let observed = image.identity.as_array();
    let wanted = prepared["identity"].as_array();
    match (observed, wanted) {
        (Some(observed), Some(wanted)) => {
            observed.len() >= 6 && wanted.len() >= 6 && observed[..6] == wanted[..6]
        }
        _ => false,
    }
}
fn check_watches(target: &mut PublicationFiles, snapshot: &Value) -> Result<()> {
    for name in texts(&snapshot["baseline_only"]) {
        let image = observe(target, &name, true)?;
        if image.state != snapshot["files"][&name]
            || image.identity != snapshot["custody"]["images"][&name]
        {
            return Err(RuntimeError::conflict(format!(
                "Baseline-only path changed: {name}"
            )));
        }
    }
    Ok(())
}
fn merge_prepared(metadata: &mut Value, name: &str, prepared: &Value) -> Result<()> {
    metadata["effects"][name]["prepared"] = prepared.clone();
    metadata["target_directories"] = prepared["directories"].clone();
    if let Some(created) = prepared["created"].as_object() {
        for (name, identity) in created {
            metadata["created_directory_identities"][name] = identity.clone();
        }
    }
    metadata["created_dirs"] = object!(
        metadata["created_directory_identities"]
            .as_object()
            .map(|value| value.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    );
    Ok(())
}
fn restore_with(
    target_path: &Path,
    target: &mut PublicationFiles,
    journal: &mut PublicationFiles,
    mut metadata: Value,
) -> Result<Value> {
    if metadata["schema_version"] != 2
        || metadata["target"] != path_text(target_path)?
        || metadata["snapshot"]["custody"]["root"] != object!(target.root_identity())
    {
        return Err(RuntimeError::policy(
            "Rollback target identity or journal version differs",
        ));
    }
    if metadata["status"] == "rolled-back" {
        return Ok(metadata);
    }
    let paths = normalized(&texts(&metadata["paths"]))?;
    if !metadata["paths"].is_array()
        || metadata["baseline"].as_object().is_none()
        || metadata["effects"].as_object().is_none()
    {
        return Err(RuntimeError::invalid(
            "Rollback journal membership malformed",
        ));
    }
    let mut conflicts = BTreeSet::new();
    let directories: BTreeMap<String, (u64, u64, u32)> =
        serde_json::from_value(metadata["target_directories"].clone())
            .map_err(|_| RuntimeError::invalid("Rollback directory custody malformed"))?;
    if target.check_directory_identities(&directories).is_err() {
        metadata["status"] = "rolled-back-with-conflicts".into();
        metadata["conflicts"] = object!(paths);
        journal_save(journal, &metadata)?;
        return Err(RuntimeError::conflict(
            "Rollback preserved files because target directory identities changed",
        ));
    }
    for name in paths.iter().rev() {
        let Some(baseline) = metadata["baseline"].get(name).cloned() else {
            return Err(RuntimeError::invalid("Rollback baseline missing"));
        };
        let effect = metadata["effects"]
            .get(name)
            .cloned()
            .unwrap_or(Value::Null);
        let current = match observe(target, name, false) {
            Ok(image) => image,
            Err(_) => {
                conflicts.insert(name.clone());
                continue;
            }
        };
        let baseline_identity = &metadata["snapshot"]["custody"]["images"][name];
        let already_restored = effect["restored_identity"].is_array()
            && current.identity == effect["restored_identity"]
            && current.state == baseline;
        if current.state == baseline && (current.identity == *baseline_identity || already_restored)
        {
            continue;
        }
        if effect.is_null() || current.state != effect["after"] {
            conflicts.insert(name.clone());
            continue;
        }
        if current.state["state"] == "file"
            && current.identity != effect["after_identity"]
            && !prepared_matches(&current, &effect["prepared"])
        {
            conflicts.insert(name.clone());
            continue;
        }
        if current.state["state"] == "missing" {
            let captured = effect["prepared"]["captured"] == true
                || effect["prepared"]["quarantine"]
                    .as_str()
                    .is_some_and(|temporary| {
                        observe(target, temporary, false).is_ok_and(|image| {
                            image.state == baseline && prepared_matches(&image, &effect["prepared"])
                        })
                    });
            if effect["after"]["state"] != "missing" || !captured {
                conflicts.insert(name.clone());
                continue;
            }
        }
        let result = match text(&baseline["state"]) {
            "missing" => {
                if let Some(raw) = current.bytes.as_deref() {
                    target.unlink_expected_limit(name, raw, LIMIT)
                } else {
                    Ok(())
                }
            }
            "file" => {
                let raw = journal
                    .read(&format!("backups/{name}"), LIMIT)?
                    .ok_or_else(|| RuntimeError::conflict("Rollback backup missing"))?;
                if sha(&raw) != text(&baseline["sha256"]) {
                    return Err(RuntimeError::conflict("Rollback backup digest differs"));
                }
                let mode = baseline["mode"]
                    .as_u64()
                    .filter(|value| *value <= 0o777)
                    .ok_or_else(|| RuntimeError::invalid("Rollback mode invalid"))?
                    as u32;
                target.write_expected_mode_checked(
                    name,
                    &raw,
                    current.bytes.as_deref(),
                    current.mode,
                    mode,
                    LIMIT,
                )
            }
            _ => return Err(RuntimeError::invalid("Unsupported rollback baseline state")),
        };
        if result.is_err() {
            conflicts.insert(name.clone());
            continue;
        }
        metadata["effects"][name]["restored_identity"] = observe(target, name, false)?.identity;
        journal_save(journal, &metadata)?;
    }
    // Temporary/quarantined names are deleted only when their saved identity and content match.
    for name in &paths {
        let effect = &metadata["effects"][name];
        let prepared = &effect["prepared"];
        for key in ["temporary", "quarantine"] {
            if let Some(temporary) = prepared[key].as_str() {
                ownership::normalize(temporary)?;
                if let Ok(image) = observe(target, temporary, false)
                    && let Some(raw) = image.bytes.as_deref()
                {
                    let expected_state = if key == "temporary" {
                        &effect["after"]
                    } else {
                        &metadata["baseline"][name]
                    };
                    if image.state == *expected_state && prepared_matches(&image, prepared) {
                        if target.unlink_expected_limit(temporary, raw, LIMIT).is_err() {
                            conflicts.insert(temporary.into());
                        }
                    } else {
                        conflicts.insert(temporary.into());
                    }
                }
            }
        }
    }
    let mut created: Vec<_> = metadata["created_directory_identities"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    created.sort_by_key(|(name, _)| std::cmp::Reverse(name.split('/').count()));
    for (name, identity) in created {
        let expected: (u64, u64, u32) = serde_json::from_value(identity)
            .map_err(|_| RuntimeError::invalid("Created directory identity malformed"))?;
        if target.remove_empty_directory(&name, expected).is_err() {
            conflicts.insert(name);
        } else {
            if let Some(directories) = metadata["target_directories"].as_object_mut() {
                directories.remove(&name);
            }
            if let Some(created) = metadata["created_directory_identities"].as_object_mut() {
                created.remove(&name);
            }
            journal_save(journal, &metadata)?;
        }
    }
    metadata["status"] = if conflicts.is_empty() {
        "rolled-back"
    } else {
        "rolled-back-with-conflicts"
    }
    .into();
    metadata["conflicts"] = object!(conflicts);
    journal_save(journal, &metadata)?;
    if !conflicts.is_empty() {
        return Err(RuntimeError::conflict(format!(
            "Rollback preserved third-party changes; inspect journal conflicts: {}",
            conflicts.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(metadata)
}
pub fn restore(target: &Path, journal: &Path, provided: Option<Value>) -> Result<Value> {
    let target = ownership::resolve_target(target)?;
    let mut files = PublicationFiles::new(&target)?;
    let mut journal = PublicationFiles::new(&ownership::resolve_target(journal)?)?;
    let stored = journal_load(&mut journal)?;
    let metadata = match provided {
        Some(value) if value == stored => value,
        Some(_) => {
            return Err(RuntimeError::conflict(
                "Supplied rollback metadata differs from durable journal",
            ));
        }
        None => stored,
    };
    restore_with(&target, &mut files, &mut journal, metadata)
}
pub fn publish(
    target: &Path,
    staging: &Path,
    paths: &[String],
    snapshot: &Value,
    journal: &Path,
    removals: &[String],
    watch: &[String],
) -> Result<Value> {
    disjoint(paths, removals, watch)?;
    let target_path = ownership::resolve_target(target)?;
    let staging_path = ownership::resolve_target(staging)?;
    let journal_path = ownership::resolve_target(journal)?;
    if target_path.starts_with(&staging_path)
        || staging_path.starts_with(&target_path)
        || journal_path.starts_with(&target_path)
        || journal_path.starts_with(&staging_path)
        || target_path.starts_with(&journal_path)
        || staging_path.starts_with(&journal_path)
    {
        return Err(RuntimeError::policy(
            "Target, staging and rollback journal must be disjoint",
        ));
    }
    let mut paths = normalized(paths)?;
    if !paths.iter().any(|path| path == MANIFEST) {
        paths.push(MANIFEST.into());
    }
    let all = paths.iter().chain(removals).cloned().collect::<Vec<_>>();
    let mut target = PublicationFiles::new(&target_path)?;
    let mut staged = PublicationFiles::new(&staging_path)?;
    ownership_with(&mut target, &mut staged, &paths, removals)?;
    let baseline = verify_with(&mut target, &target_path, &all, snapshot, watch)?;
    let mut source = BTreeMap::new();
    let mut total = 0usize;
    for name in &paths {
        let image = observe(&mut staged, name, false)?;
        let raw = image
            .bytes
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid(format!("Staged image missing: {name}")))?;
        total = total
            .checked_add(raw.len())
            .ok_or_else(|| RuntimeError::invalid("Staged size overflow"))?;
        if total > LIMIT {
            return Err(RuntimeError::invalid("Staged publication exceeds 512 MiB"));
        }
        source.insert(name.clone(), image);
    }
    // Re-run ownership against the same retained staging namespace after capturing all source images.
    ownership_with(&mut target, &mut staged, &paths, removals)?;
    for (name, image) in &source {
        let current = observe(&mut staged, name, false)?;
        if current.identity != image.identity || current.state != image.state {
            return Err(RuntimeError::conflict(
                "Staging changed while capturing publication",
            ));
        }
    }
    let mut journal = create_journal(&journal_path)?;
    let mut metadata = prepare_with(&mut target, &target_path, &all, snapshot, &mut journal)?;
    metadata["status"] = "publishing".into();
    journal_save(&mut journal, &metadata)?;
    let action = (|| -> Result<()> {
        let order = paths
            .iter()
            .filter(|name| name.as_str() != MANIFEST)
            .cloned()
            .chain(removals.iter().cloned())
            .chain([MANIFEST.to_string()])
            .collect::<Vec<_>>();
        for name in order {
            check_watches(&mut target, snapshot)?;
            let before = baseline
                .get(&name)
                .ok_or_else(|| RuntimeError::invalid("Effect baseline absent"))?;
            let current = observe(&mut target, &name, false)?;
            if current.state != before.state || current.identity != before.identity {
                return Err(RuntimeError::conflict(format!(
                    "Target effect changed before publication: {name}"
                )));
            }
            let removing = removals.contains(&name);
            let after = if removing {
                object!({"state":"missing"})
            } else {
                source[&name].state.clone()
            };
            if !removing {
                let current_source = observe(&mut staged, &name, false)?;
                if current_source.identity != source[&name].identity
                    || current_source.state != source[&name].state
                {
                    return Err(RuntimeError::conflict(format!(
                        "Staged effect changed: {name}"
                    )));
                }
            }
            metadata["effects"][&name] =
                object!({"before":before.state,"after":after,"prepared":null,"committed":false});
            journal_save(&mut journal, &metadata)?;
            if removing {
                if let Some(raw) = before.bytes.as_deref() {
                    target.remove_receipted(
                        &name,
                        raw,
                        before
                            .mode
                            .ok_or_else(|| RuntimeError::invalid("Removal mode absent"))?,
                        LIMIT,
                        |prepared| {
                            merge_prepared(&mut metadata, &name, prepared)?;
                            journal_save(&mut journal, &metadata)
                        },
                    )?;
                }
            } else {
                let image = &source[&name];
                target.write_receipted(
                    &name,
                    image
                        .bytes
                        .as_deref()
                        .ok_or_else(|| RuntimeError::invalid("Staged bytes absent"))?,
                    before.bytes.as_deref(),
                    before.mode,
                    image
                        .mode
                        .ok_or_else(|| RuntimeError::invalid("Staged mode absent"))?,
                    LIMIT,
                    |prepared| {
                        merge_prepared(&mut metadata, &name, prepared)?;
                        journal_save(&mut journal, &metadata)
                    },
                )?;
            }
            let after = observe(&mut target, &name, false)?;
            if after.state != metadata["effects"][&name]["after"] {
                return Err(RuntimeError::conflict(
                    "Published effect changed before receipt",
                ));
            }
            metadata["effects"][&name]["committed"] = true.into();
            metadata["effects"][&name]["after_identity"] = after.identity;
            journal_save(&mut journal, &metadata)?;
        }
        check_watches(&mut target, snapshot)?;
        staged.check()?;
        target.check()?;
        Ok(())
    })();
    if let Err(error) = action {
        match restore_with(&target_path, &mut target, &mut journal, metadata) {
            Ok(_) => return Err(error),
            Err(rollback) => {
                return Err(RuntimeError::conflict(format!(
                    "Publication stopped: {error}; {rollback}"
                )));
            }
        }
    }
    metadata["published"] = object!(
        all.iter()
            .map(|name| Ok((name.clone(), observe(&mut target, name, false)?.state)))
            .collect::<Result<BTreeMap<_, _>>>()?
    );
    metadata["status"] = "published-pending-verification".into();
    journal_save(&mut journal, &metadata)?;
    Ok(metadata)
}
