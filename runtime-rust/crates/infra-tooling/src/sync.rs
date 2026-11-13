use crate::{common, file_modes::FileModes, manifests::CoreManifest};
use infra_core::{Result, fs as safe};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};
pub fn core_sync(root: &Path, write: bool, chain: bool) -> Result<Value> {
    let manifest = CoreManifest::load(root)?;
    let mut globs = common::Globs::default();
    let canon = root.join(&manifest.canonical);
    let source_modes = FileModes::read(&canon)?;
    let all = common::walk(&canon)?;
    let mut canonical = BTreeSet::new();
    let mut retired_canonical = Vec::new();
    for name in all {
        if manifest.retired_with_globs(&name, &mut globs)? {
            retired_canonical.push(name);
        } else if manifest.core_with_globs(&name, &mut globs)? {
            canonical.insert(name);
        }
    }
    let mut changes = Vec::new();
    for edition in manifest
        .editions
        .values()
        .filter(|edition| *edition != &manifest.canonical)
    {
        let base = root.join(edition);
        if !base.is_dir() {
            continue;
        }
        let mut modes = FileModes::read(&base)?;
        let mut existing = BTreeSet::new();
        for name in common::walk(&base)? {
            if manifest.core_with_globs(&name, &mut globs)?
                || manifest.retired_with_globs(&name, &mut globs)?
            {
                existing.insert(name);
            }
        }
        for name in &canonical {
            let bytes = safe::read_regular(&common::under(&canon, name)?)?;
            let destination = common::under(&base, name)?;
            let actual = if destination.is_file() {
                Some(safe::read_regular(&destination)?)
            } else {
                None
            };
            let executable = source_modes.executable(name)?;
            let mode_diff = actual.is_some()
                && executable.is_some_and(|wanted| {
                    modes
                        .executable(name)
                        .ok()
                        .flatten()
                        .is_some_and(|actual| actual != wanted)
                });
            if actual.as_deref() != Some(bytes.as_slice()) || mode_diff {
                let action = if actual.is_none() {
                    "add"
                } else if actual.as_deref() != Some(bytes.as_slice()) {
                    "update"
                } else {
                    "mode"
                };
                changes.push(json!({"edition":edition,"path":name,"action":action}));
                if write {
                    common::write(&base, name, &bytes)?;
                    if let Some(value) = executable {
                        modes.set(name, value)?;
                    }
                }
            }
        }
        for name in existing.difference(&canonical) {
            changes.push(json!({"edition":edition,"path":name,"action":"remove"}));
            if write {
                std::fs::remove_file(common::under(&base, name)?)?;
            }
        }
        if write {
            modes.flush()?;
        }
    }
    let mut chained = Vec::new();
    if write && chain {
        chained.push(asset_sync(root, true)?);
        for edition in common::EDITIONS {
            if root.join(edition).is_dir() {
                chained.push(crate::mirrors::process(root, edition, true)?);
            }
        }
    }
    Ok(
        json!({"canonical":manifest.canonical,"write":write,"changes":changes,"chained":chained,"retired_canonical":retired_canonical,"passed":retired_canonical.is_empty()&&(write||changes.is_empty())}),
    )
}
pub fn asset_sync(root: &Path, write: bool) -> Result<Value> {
    let canon = root.join("PHP Core");
    let asset = root.join("Infrastructure-Creator/.agents/skills/memory-seed/assets");
    let includes = [
        "memory-bank/MCP.md",
        "memory-bank/config/**",
        "memory-bank/templates/**",
        "project-brain/PROTOCOL.md",
        "project-brain/README.md",
        "project-brain/.gitignore",
        "project-brain/schemas/**",
        "project-brain/templates/**",
        "project-brain/config/providers.json",
        "project-brain/config/telemetry.json",
        "project-brain/indexes/*.json",
        "project-brain/**/.gitkeep",
    ];
    let mut changes = Vec::new();
    let mut expected = BTreeSet::new();
    let findings: Vec<String> = Vec::new();
    let modes = FileModes::read(&canon)?;
    let mut asset_modes = FileModes::read(&asset)?;
    let mut globs = common::Globs::default();
    for path in common::walk(&canon)? {
        let mut selected = false;
        for pattern in &includes {
            if globs.matches(pattern, &path, true)? {
                selected = true;
                break;
            }
        }
        if !selected
            || path.contains("/tests/")
            || path.starts_with("project-brain/local/") && !path.ends_with("/.gitkeep")
        {
            continue;
        }
        let relative = if let Some(rest) = path.strip_prefix("memory-bank/") {
            rest.to_string()
        } else {
            path.clone()
        };
        let source = match path.as_str() {
            "project-brain/indexes/active.json" => "project-brain/.install/active.json",
            "project-brain/indexes/archive.json" => "project-brain/.install/archive.json",
            other => other,
        };
        expected.insert(relative.clone());
        let bytes = safe::read_regular(&common::under(&canon, source)?)?;
        let destination = common::under(&asset, &relative)?;
        let actual = if destination.is_file() {
            Some(safe::read_regular(&destination)?)
        } else {
            None
        };
        let executable = modes.executable(source)?;
        let mode_diff = actual.is_some()
            && executable.is_some_and(|want| {
                asset_modes
                    .executable(&relative)
                    .ok()
                    .flatten()
                    .is_some_and(|have| have != want)
            });
        if actual.as_deref() != Some(bytes.as_slice()) || mode_diff {
            changes.push(json!({"path":relative,"action":"write"}));
            if write {
                common::write(&asset, &relative, &bytes)?;
                if let Some(mode) = executable {
                    asset_modes.set(&relative, mode)?;
                }
            }
        }
    }
    // The only materialized config substitutions are framework and canonical
    // tool edition. Other keys and values stay the canonical runtime defaults.
    let mut template = common::read_json(&canon.join("project-brain/config/runtime.json"))?;
    template["framework"] = "{{TARGET_FRAMEWORK}}".into();
    template["canonical_edition"] = "{{CANONICAL_EDITION}}".into();
    let template_relative = "project-brain/config/runtime.json.template";
    expected.insert(template_relative.into());
    let template_path = asset.join(template_relative);
    let actual = if template_path.is_file() {
        Some(common::read_json(&template_path)?)
    } else {
        None
    };
    if actual.as_ref() != Some(&template) {
        changes.push(json!({"path":template_relative,"action":"write"}));
        if write {
            common::write(&asset, template_relative, &common::json_bytes(&template)?)?;
        }
    }
    // The same explicit versioned resource is compiled by infra-generator.
    // Reading this source path avoids a library dependency cycle and never loads
    // a retired Python module to discover its contract.
    let contract_source =
        root.join("runtime-rust/crates/infra-generator/resources/native-runtime-contract.json");
    let contract_bytes = safe::read_regular(&contract_source)?;
    let contract_value: Value = serde_json::from_slice(&contract_bytes)?;
    if contract_value["native_engine"]["unix_path"] != "memory-bank/bin/infra-engine"
        || contract_value["native_engine"]["windows_path"] != "memory-bank/bin/infra-engine.exe"
        || contract_value["native_engine"]["capability_profile"] != "infra-engine-v1"
    {
        return Err(infra_core::RuntimeError::invalid(
            "Trusted generator contract has no current native engine contract",
        ));
    }
    let contract_name = "runtime-contract.json";
    expected.insert(contract_name.into());
    let contract = asset.join(contract_name);
    let current = if contract.is_file() {
        Some(safe::read_regular(&contract)?)
    } else {
        None
    };
    if current.as_deref() != Some(contract_bytes.as_slice()) {
        changes.push(json!({"path":contract_name,"action":"write"}));
        if write {
            common::write(&asset, contract_name, &contract_bytes)?;
        }
    }
    // Native release artifacts are admitted independently by their digest
    // manifest, never copied from an implicit build-output scan.
    for name in [
        "native-artifacts.json",
        "bin/infra-engine",
        "bin/infra-engine.exe",
    ] {
        if asset.join(name).is_file() {
            expected.insert(name.into());
        }
    }
    for relative in common::walk(&asset)? {
        if expected.contains(&relative) || common::ignored(&relative) {
            continue;
        }
        changes.push(json!({"path":relative,"action":"remove"}));
        if write {
            std::fs::remove_file(common::under(&asset, &relative)?)?;
        }
    }
    if write {
        asset_modes.flush()?;
    }
    Ok(
        json!({"write":write,"changes":changes,"findings":findings,"passed":findings.is_empty()&&(write||changes.is_empty())}),
    )
}
