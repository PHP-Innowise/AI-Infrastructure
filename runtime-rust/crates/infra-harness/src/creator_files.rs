//! Creator's bounded reviewed images; receipts bind both bytes and directory custody.
use crate::{HarnessError, Result, store};
use infra_core::bank::publication::PublicationFiles;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub const TASK: &str = "tasks/TASK-001";
pub const REPORTS: [&str; 7] = [
    "infra-scan-project-profile.md",
    "skill-generation-plan.json",
    "skill-plan-quality-report.json",
    "infra-scan-rejection-report.md",
    "harness-questions.md",
    "infra-generate-report.md",
    "infra-validate-review.json",
];
pub const PLANS: [&str; 3] = [
    "infra-generate-publication-plan.txt",
    "infra-generate-removal-plan.txt",
    "infra-generate-watch-plan.txt",
];
pub const LIMIT: usize = 64 * 1024 * 1024;
pub fn read(root: &Path, name: &str, required: bool, limit: usize) -> Result<Option<Vec<u8>>> {
    let mut files = PublicationFiles::new(root)?;
    let value = files.read(name, limit)?;
    if required && value.is_none() {
        return Err(HarnessError::missing(format!(
            "Required Creator file is missing: {name}"
        )));
    }
    Ok(value)
}
pub fn object(root: &Path, name: &str, required: bool) -> Result<Option<Value>> {
    read(root, name, required, LIMIT)?
        .map(|bytes| {
            let v = crate::strict_json::parse(&bytes, LIMIT)?;
            if !v.is_object() {
                return Err(HarnessError::invalid("Creator state must be an object."));
            }
            Ok(v)
        })
        .transpose()
}
pub fn write(root: &Path, name: &str, value: &Value) -> Result<()> {
    let mut files = PublicationFiles::new(root)?;
    let old = files.read(name, LIMIT)?;
    let bytes = serde_json::to_vec(value)?;
    let mode = files.read_mode(name)?;
    files.write_expected_mode_checked(name, &bytes, old.as_deref(), mode, 0o600, LIMIT)?;
    Ok(())
}
pub fn plan(task: &Path, name: &str, optional: bool) -> Result<Vec<String>> {
    let bytes = read(task, name, !optional, 1024 * 1024)?.unwrap_or_default();
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| HarnessError::invalid("Publication plans must use UTF-8 text."))?;
    let paths = text
        .lines()
        .filter(|p| !p.trim().is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    if paths.len() > 5000 || paths.iter().any(|p| !seen.insert(p.clone())) {
        return Err(HarnessError::invalid(
            "Publication plans must contain distinct bounded paths.",
        ));
    }
    for p in &paths {
        infra_generator::ownership::normalize(p)?;
    }
    Ok(paths)
}
pub fn allowed(p: &str) -> bool {
    [
        "AGENTS.md",
        "CLAUDE.md",
        "DOD.md",
        "GOLDEN-PRINCIPLES.md",
        "STABILIZATION.md",
        ".gitignore",
        ".infra-manifest.json",
        "spec-desc.md",
    ]
    .contains(&p)
        || [
            ".agents",
            ".claude",
            ".codex",
            ".cursor",
            "memory-bank",
            "project-brain",
            "specs",
            "tasks",
        ]
        .contains(&p.split('/').next().unwrap_or(""))
}
pub fn hashes(directory: &Path) -> Result<Value> {
    let task = directory.join("agent").join(TASK);
    let mut value = json!({});
    for name in &REPORTS[..3] {
        value[*name] = store::sha(&read(&task, name, true, LIMIT)?.unwrap()).into();
    }
    Ok(value)
}
pub fn identity(path: &Path) -> Result<Value> {
    Ok(serde_json::to_value(
        PublicationFiles::new(path)?.root_identity(),
    )?)
}
pub fn check_identity(run: &Value) -> Result<()> {
    if identity(Path::new(run["target"].as_str().ok_or_else(|| {
        HarnessError::invalid("Creator target is missing.")
    })?))?
        != run["identity"]
    {
        return Err(HarnessError::conflict("Target directory changed."));
    }
    Ok(())
}
fn metadata(files: &mut PublicationFiles, path: &str, bytes: Option<&[u8]>) -> Result<Value> {
    match bytes {
        None => Ok(Value::Null),
        Some(b) => {
            let identity = files.image_identity(path)?;
            #[cfg(unix)]
            let mode = identity.map(|v| v.2 & 0o7777);
            #[cfg(not(unix))]
            let mode = files.read_mode(path)?;
            Ok(json!({"hash":store::sha(b),"bytes":b.len(),"mode":mode,"identity":identity}))
        }
    }
}
pub fn diff(before: &[u8], after: &[u8], path: &str, budget: usize) -> (String, bool) {
    if before == after {
        return (String::new(), false);
    }
    let (left, right) = (std::str::from_utf8(before), std::str::from_utf8(after));
    if left.is_err() || right.is_err() {
        return (format!("Binary files differ: {path}\n"), false);
    }
    let mut result = format!("--- a/{path}\n+++ b/{path}\n");
    let old = left.unwrap().lines().collect::<Vec<_>>();
    let new = right.unwrap().lines().collect::<Vec<_>>();
    let mut start = 0;
    while start < old.len().min(new.len()) && old[start] == new[start] {
        start += 1;
    }
    let mut end = 0;
    while end
        < old
            .len()
            .saturating_sub(start)
            .min(new.len().saturating_sub(start))
        && old[old.len() - 1 - end] == new[new.len() - 1 - end]
    {
        end += 1;
    }
    result.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        start + 1,
        old.len() - start - end,
        start + 1,
        new.len() - start - end
    ));
    for line in &old[start..old.len() - end] {
        result.push('-');
        result.push_str(line);
        result.push('\n');
        if result.len() > budget {
            break;
        }
    }
    if result.len() <= budget {
        for line in &new[start..new.len() - end] {
            result.push('+');
            result.push_str(line);
            result.push('\n');
            if result.len() > budget {
                break;
            }
        }
    }
    let truncated = result.len() > budget;
    if truncated {
        let mut at = budget.min(result.len());
        while !result.is_char_boundary(at) {
            at -= 1;
        }
        result.truncate(at);
    }
    (result, truncated)
}
pub fn preview(run: &Value, directory: &Path) -> Result<Value> {
    if run["status"] != "preview" {
        return Err(HarnessError::conflict(
            "The generated bundle has not passed verification.",
        ));
    }
    check_identity(run)?;
    let task = directory.join("agent").join(TASK);
    let staging = task.join("infra-generate-staging");
    let target = PathBuf::from(run["target"].as_str().unwrap_or(""));
    let mut paths = plan(&task, PLANS[0], false)?;
    if !paths.iter().any(|v| v == ".infra-manifest.json") {
        paths.push(".infra-manifest.json".into());
    }
    let removals = plan(&task, PLANS[1], true)?;
    let watched = plan(&task, PLANS[2], true)?;
    let all = paths.iter().chain(&removals).collect::<BTreeSet<_>>();
    if paths.iter().any(|p| removals.contains(p)) || watched.iter().any(|p| all.contains(p)) {
        return Err(HarnessError::invalid(
            "Publication, removal and watch plans overlap.",
        ));
    }
    if !all.iter().all(|p| allowed(p)) {
        return Err(HarnessError::forbidden(
            "Creator output cannot overwrite application source files.",
        ));
    }
    let mut oldfiles = PublicationFiles::new(&target)?;
    let mut newfiles = PublicationFiles::new(&staging)?;
    let existing = oldfiles
        .read(".infra-manifest.json", 4 * 1024 * 1024)?
        .map(|b| crate::strict_json::parse(&b, 4 * 1024 * 1024))
        .transpose()?
        .unwrap_or(json!({}));
    if existing.get("files").is_some_and(|v| !v.is_object()) {
        return Err(HarnessError::invalid(
            "Existing manifest files must be an object.",
        ));
    }
    let mut before = json!({});
    let mut payloads = json!({});
    let mut entries = vec![];
    let mut total = 0usize;
    let mut diff_budget = 256000usize;
    for path in paths.iter().chain(&removals).chain(&watched) {
        let old = oldfiles.read_checked(path, LIMIT, |bytes| {
            if total.saturating_add(bytes as usize) > LIMIT {
                return Err(infra_core::RuntimeError::invalid(
                    "Creator preview exceeds 64 MiB.",
                ));
            }
            Ok(())
        })?;
        total += old.as_ref().map(Vec::len).unwrap_or(0);
        let new = if paths.contains(path) {
            let bytes = newfiles
                .read_checked(path, LIMIT, |bytes| {
                    if total.saturating_add(bytes as usize) > LIMIT {
                        return Err(infra_core::RuntimeError::invalid(
                            "Creator preview exceeds 64 MiB.",
                        ));
                    }
                    Ok(())
                })?
                .ok_or_else(|| HarnessError::missing(format!("Staged file missing: {path}")))?;
            total += bytes.len();
            Some(bytes)
        } else {
            None
        };
        before[path] = metadata(&mut oldfiles, path, old.as_deref())?;
        if let Some(bytes) = new.as_deref() {
            payloads[path] = metadata(&mut newfiles, path, Some(bytes))?;
            if payloads[path]["mode"]
                .as_u64()
                .is_some_and(|mode| mode & 0o7000 != 0)
            {
                return Err(HarnessError::forbidden(
                    "Generated files cannot use special permission bits.",
                ));
            }
        }
        if watched.contains(path) {
            continue;
        }
        let (equal, decision) = {
            let equal = old.is_some()
                && new.is_some()
                && before[path]["hash"] == payloads[path]["hash"]
                && before[path]["mode"] == payloads[path]["mode"];
            let owned = &existing["files"][path];
            let digest = owned["sha256"].as_str().or_else(|| owned.as_str());
            let decision = !equal
                && path != ".infra-manifest.json"
                && (old.is_some() || !owned.is_null())
                && (run["operation"] != "update"
                    || old.is_none()
                    || digest != before[path]["hash"].as_str());
            (equal, decision)
        };
        let (d, truncated) = diff(
            old.as_deref().unwrap_or_default(),
            new.as_deref().unwrap_or_default(),
            path,
            diff_budget,
        );
        diff_budget = diff_budget.saturating_sub(d.len());
        entries.push(json!({"path":path,"action":if equal{"unchanged"}else if new.is_none(){"remove"}else if old.is_some(){"replace"}else{"add"},"requires_decision":decision,"diff":d,"diff_truncated":truncated,"before_bytes":old.as_ref().map(Vec::len).unwrap_or(0),"after_bytes":new.as_ref().map(Vec::len).unwrap_or(0)}));
    }
    let mut result = json!({"run_id":run["id"],"identity":run["identity"],"before":before,"payloads":payloads,"directories":oldfiles.directory_identities()?,"staging_identity":newfiles.root_identity(),"staging_directories":newfiles.directory_identities()?,"profile":hashes(directory)?,"paths":paths,"removals":removals,"watched":watched});
    let canonical = infra_core::domain::json::canonical(&result)?;
    result["preview_id"] = store::sha(canonical.as_bytes()).into();
    let conflict =
        infra_generator::publication::verify_plan_ownership(&target, &staging, &paths, &removals)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
    result["files"] = entries.into();
    result["can_apply"] = conflict.is_empty().into();
    result["conflict"] = conflict.into();
    Ok(result)
}
pub use crate::creator_payload::{CreatorEngine, admit_engine};
pub fn copy_generator(source: &Path, destination: &Path, engine: &mut CreatorEngine) -> Result<()> {
    crate::creator_payload::copy(source, destination, engine)
}
pub fn texts(value: &Value) -> Result<Vec<String>> {
    value
        .as_array()
        .ok_or_else(|| HarnessError::invalid("Expected a path array."))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| HarnessError::invalid("Expected a path string."))
        })
        .collect()
}
pub fn custody(root: &Path, directories: &Value) -> Result<()> {
    let expected: BTreeMap<String, (u64, u64, u32)> = serde_json::from_value(directories.clone())?;
    let mut files = PublicationFiles::new(root)?;
    files.check_directory_identities(&expected)?;
    Ok(())
}

pub fn validation_copy(target: &Path, base: &Path) -> Result<tempfile::TempDir> {
    let directory = tempfile::Builder::new()
        .prefix("creator-validation-")
        .tempdir_in(base)?;
    infra_host::util::private_directory(directory.path())?;
    let mut source = PublicationFiles::new(target)?;
    let (names, omitted) = source.walk_files(64, 100000, |path| {
        allowed(path.split('/').next().unwrap_or(""))
            && !["memory-bank/local", "project-brain/local"].contains(&path)
    })?;
    if omitted > 0 || names.len() > 50000 {
        return Err(HarnessError::invalid(
            "Accelerator validation copy exceeds its entry bound.",
        ));
    }
    let mut output = PublicationFiles::new(directory.path())?;
    let mut total = 0usize;
    for name in names {
        if !allowed(&name)
            || name.starts_with("memory-bank/local/")
            || name.starts_with("project-brain/local/")
        {
            continue;
        }
        let bytes = source
            .read_checked(&name, 256 * 1024 * 1024, |size| {
                if total.saturating_add(size as usize) > 256 * 1024 * 1024 {
                    return Err(infra_core::RuntimeError::invalid(
                        "Accelerator validation copy exceeds 256 MiB.",
                    ));
                }
                Ok(())
            })?
            .ok_or_else(|| HarnessError::conflict("Accelerator validation input disappeared."))?;
        total += bytes.len();
        let mode = source
            .read_mode(&name)?
            .ok_or_else(|| HarnessError::conflict("Validation input mode disappeared."))?;
        output.write_expected_mode_checked(
            &name,
            &bytes,
            None,
            None,
            if cfg!(windows) { 0o600 } else { mode },
            256 * 1024 * 1024,
        )?;
    }
    source.check()?;
    output.check()?;
    Ok(directory)
}
