use crate::{common, file_modes::FileModes, manifests::CoreManifest, mirrors::check_receipts};
use infra_core::{Result, fs as safe};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

const ASSET_INCLUDES: &[&str] = &[
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

#[derive(Clone, Debug)]
pub struct SyncOptions {
    pub write: bool,
    pub chain: bool,
    pub use_cache: bool,
    pub cache_path: Option<PathBuf>,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            write: false,
            chain: true,
            use_cache: true,
            cache_path: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SyncOutcome {
    pub report: Value,
    pub cache_hit: bool,
}

fn compute_core_sync_hashes(
    root: &Path,
    manifest: &CoreManifest,
    globs: &mut common::Globs,
    store: &mut check_receipts::TreeReceiptStore,
    now_ns: i128,
    dirty: &mut bool,
) -> Option<(String, String, String)> {
    let manifest_path = root.join("install/core-manifest.json");
    let (manifest_sha, _) =
        check_receipts::hash_regular_file(&manifest_path, store, now_ns, dirty)?;
    let canon = root.join(&manifest.canonical);
    if !canon.is_dir() {
        return None;
    }
    let mut git_cues = Vec::new();
    let canon_cue = check_receipts::git_modes_cue(&canon, store, now_ns, dirty)?;
    git_cues.push(format!("{}:{canon_cue}", manifest.canonical));

    let mut canonical_parts = Vec::new();
    for name in common::walk(&canon).ok()? {
        if manifest.retired_with_globs(&name, globs).ok()? {
            return None;
        }
        if manifest.core_with_globs(&name, globs).ok()? {
            let file = common::under(&canon, &name).ok()?;
            let (sha, exec_bit) = check_receipts::hash_regular_file(&file, store, now_ns, dirty)?;
            canonical_parts.push(format!("{name}:{sha}:{exec_bit}"));
        }
    }

    let mut target_parts = Vec::new();
    for edition in manifest
        .editions
        .values()
        .filter(|edition| *edition != &manifest.canonical)
    {
        let base = root.join(edition);
        if !base.is_dir() {
            target_parts.push(format!("{edition}:absent_dir"));
            continue;
        }
        let ed_cue = check_receipts::git_modes_cue(&base, store, now_ns, dirty)?;
        git_cues.push(format!("{edition}:{ed_cue}"));
        for name in common::walk(&base).ok()? {
            if manifest.core_with_globs(&name, globs).ok()?
                || manifest.retired_with_globs(&name, globs).ok()?
            {
                let file = common::under(&base, &name).ok()?;
                let (sha, exec_bit) =
                    check_receipts::hash_regular_file(&file, store, now_ns, dirty)?;
                target_parts.push(format!("{edition}:{name}:{sha}:{exec_bit}"));
            }
        }
    }
    let rules_sha256 =
        common::sha256(format!("manifest:{manifest_sha}\ngit:{}", git_cues.join("|")).as_bytes());
    let canonical_tree_sha256 = common::sha256(canonical_parts.join("\n").as_bytes());
    let target_tree_sha256 = common::sha256(target_parts.join("\n").as_bytes());
    Some((canonical_tree_sha256, rules_sha256, target_tree_sha256))
}

fn compute_asset_sync_hashes(
    root: &Path,
    globs: &mut common::Globs,
    store: &mut check_receipts::TreeReceiptStore,
    now_ns: i128,
    dirty: &mut bool,
) -> Option<(String, String, String)> {
    let canon = root.join("PHP Core");
    let asset = root.join("Infrastructure-Creator/.agents/skills/memory-seed/assets");
    if !canon.is_dir() || !asset.is_dir() {
        return None;
    }
    let (runtime_sha, _) = check_receipts::hash_regular_file(
        &canon.join("project-brain/config/runtime.json"),
        store,
        now_ns,
        dirty,
    )?;
    let (contract_sha, _) = check_receipts::hash_regular_file(
        &root.join("runtime-rust/crates/infra-generator/resources/native-runtime-contract.json"),
        store,
        now_ns,
        dirty,
    )?;
    let canon_git = check_receipts::git_modes_cue(&canon, store, now_ns, dirty)?;
    let asset_git = check_receipts::git_modes_cue(&asset, store, now_ns, dirty)?;
    let rules_sha256 = common::sha256(
        format!(
            "asset-sync-v1\nincludes:{}\nruntime:{runtime_sha}\ncontract:{contract_sha}\ngit:{canon_git}|{asset_git}",
            ASSET_INCLUDES.join(",")
        )
        .as_bytes(),
    );

    let mut canonical_parts = Vec::new();
    for path in common::walk(&canon).ok()? {
        let mut selected = false;
        for pattern in ASSET_INCLUDES {
            if globs.matches(pattern, &path, true).ok()? {
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
        let relative = path.strip_prefix("memory-bank/").unwrap_or(&path);
        let source = match path.as_str() {
            "project-brain/indexes/active.json" => "project-brain/.install/active.json",
            "project-brain/indexes/archive.json" => "project-brain/.install/archive.json",
            other => other,
        };
        let file = common::under(&canon, source).ok()?;
        let (sha, exec_bit) = check_receipts::hash_regular_file(&file, store, now_ns, dirty)?;
        canonical_parts.push(format!("{relative}:{source}:{sha}:{exec_bit}"));
    }

    let mut target_parts = Vec::new();
    for relative in common::walk(&asset).ok()? {
        if common::ignored(&relative) {
            continue;
        }
        let file = common::under(&asset, &relative).ok()?;
        let (sha, exec_bit) = check_receipts::hash_regular_file(&file, store, now_ns, dirty)?;
        target_parts.push(format!("{relative}:{sha}:{exec_bit}"));
    }

    let canonical_tree_sha256 = common::sha256(canonical_parts.join("\n").as_bytes());
    let target_tree_sha256 = common::sha256(target_parts.join("\n").as_bytes());
    Some((canonical_tree_sha256, rules_sha256, target_tree_sha256))
}

pub fn core_sync(root: &Path, write: bool, chain: bool) -> Result<Value> {
    core_sync_with_options(
        root,
        SyncOptions {
            write,
            chain,
            use_cache: true,
            cache_path: None,
        },
    )
    .map(|outcome| outcome.report)
}

pub fn core_sync_with_options(root: &Path, options: SyncOptions) -> Result<SyncOutcome> {
    let write = options.write;
    let chain = options.chain;
    let manifest = CoreManifest::load(root)?;
    let mut globs = common::Globs::default();
    let effective_cache = !write && options.use_cache && !check_receipts::is_cache_bypassed();
    let cache_path = effective_cache.then(|| {
        options.cache_path.clone().unwrap_or_else(|| {
            check_receipts::default_receipt_path(root, "infra-sync-receipts.json")
        })
    });
    let mut store = cache_path
        .as_deref()
        .map(check_receipts::load_store)
        .unwrap_or_default();
    let mut dirty = false;
    let now_ns = check_receipts::now_unix_ns();
    let hashes = if effective_cache {
        compute_core_sync_hashes(root, &manifest, &mut globs, &mut store, now_ns, &mut dirty)
    } else {
        None
    };
    if effective_cache
        && let Some((ref canonical_sha, ref rules_sha, ref target_sha)) = hashes
        && store.matches_receipt("core-sync", canonical_sha, rules_sha, target_sha)
    {
        if dirty && let Some(path) = cache_path.as_deref() {
            check_receipts::save_store(path, &store);
        }
        return Ok(SyncOutcome {
            report: json!({"canonical":manifest.canonical,"write":false,"changes":[],"chained":[],"retired_canonical":[],"passed":true}),
            cache_hit: true,
        });
    }

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
    let passed = retired_canonical.is_empty() && (write || changes.is_empty());
    if !write
        && effective_cache
        && passed
        && let Some((canonical_sha, rules_sha, target_sha)) = hashes
    {
        store.insert_receipt("core-sync", canonical_sha, rules_sha, target_sha);
        dirty = true;
    }
    if effective_cache
        && dirty
        && let Some(path) = cache_path.as_deref()
    {
        check_receipts::save_store(path, &store);
    }
    Ok(SyncOutcome {
        report: json!({"canonical":manifest.canonical,"write":write,"changes":changes,"chained":chained,"retired_canonical":retired_canonical,"passed":passed}),
        cache_hit: false,
    })
}

pub fn asset_sync(root: &Path, write: bool) -> Result<Value> {
    asset_sync_with_options(
        root,
        SyncOptions {
            write,
            chain: false,
            use_cache: true,
            cache_path: None,
        },
    )
    .map(|outcome| outcome.report)
}

pub fn asset_sync_with_options(root: &Path, options: SyncOptions) -> Result<SyncOutcome> {
    let write = options.write;
    let mut globs = common::Globs::default();
    let effective_cache = !write && options.use_cache && !check_receipts::is_cache_bypassed();
    let cache_path = effective_cache.then(|| {
        options.cache_path.clone().unwrap_or_else(|| {
            check_receipts::default_receipt_path(root, "infra-sync-receipts.json")
        })
    });
    let mut store = cache_path
        .as_deref()
        .map(check_receipts::load_store)
        .unwrap_or_default();
    let mut dirty = false;
    let now_ns = check_receipts::now_unix_ns();
    let hashes = if effective_cache {
        compute_asset_sync_hashes(root, &mut globs, &mut store, now_ns, &mut dirty)
    } else {
        None
    };
    if effective_cache
        && let Some((ref canonical_sha, ref rules_sha, ref target_sha)) = hashes
        && store.matches_receipt("asset-sync", canonical_sha, rules_sha, target_sha)
    {
        if dirty && let Some(path) = cache_path.as_deref() {
            check_receipts::save_store(path, &store);
        }
        return Ok(SyncOutcome {
            report: json!({"write":false,"changes":[],"findings":[],"passed":true}),
            cache_hit: true,
        });
    }

    let canon = root.join("PHP Core");
    let asset = root.join("Infrastructure-Creator/.agents/skills/memory-seed/assets");
    let mut changes = Vec::new();
    let mut expected = BTreeSet::new();
    let findings: Vec<String> = Vec::new();
    let modes = FileModes::read(&canon)?;
    let mut asset_modes = FileModes::read(&asset)?;
    for path in common::walk(&canon)? {
        let mut selected = false;
        for pattern in ASSET_INCLUDES {
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
    let passed = findings.is_empty() && (write || changes.is_empty());
    if !write
        && effective_cache
        && passed
        && let Some((canonical_sha, rules_sha, target_sha)) = hashes
    {
        store.insert_receipt("asset-sync", canonical_sha, rules_sha, target_sha);
        dirty = true;
    }
    if effective_cache
        && dirty
        && let Some(path) = cache_path.as_deref()
    {
        check_receipts::save_store(path, &store);
    }
    Ok(SyncOutcome {
        report: json!({"write":write,"changes":changes,"findings":findings,"passed":passed}),
        cache_hit: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::temp::Temporary;

    fn write_file(root: &Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn core_sync_check_receipt_short_circuits_and_respects_write_bypass_and_drift() {
        let temp = Temporary::new("infra-core-sync-receipts").unwrap();
        let root = infra_core::fs::canonicalize(temp.path()).unwrap();
        let cache_dir = Temporary::new("infra-core-sync-cache").unwrap();
        let cache_file = cache_dir.path().join("sync-receipts.json");

        write_file(
            &root,
            "install/core-manifest.json",
            &common::json_bytes(&json!({
                "version": 1,
                "canonical": "PHP Core",
                "editions": {
                    "php-core": "PHP Core",
                    "laravel": "Laravel"
                },
                "includes": ["shared/**"],
                "allowed_drift": {},
                "retired": []
            }))
            .unwrap(),
        );
        write_file(&root, "PHP Core/shared/doc.md", b"Shared doc\n");
        write_file(&root, "Laravel/shared/doc.md", b"Stale doc\n");

        // `--write` never short-circuits.
        let write_res = core_sync_with_options(
            &root,
            SyncOptions {
                write: true,
                chain: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!write_res.cache_hit);
        assert_eq!(write_res.report["passed"], true);

        // First `--check` misses and populates receipt; second `--check` hits receipt.
        let cold = core_sync_with_options(
            &root,
            SyncOptions {
                write: false,
                chain: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!cold.cache_hit);
        assert_eq!(cold.report["passed"], true);

        let warm = core_sync_with_options(
            &root,
            SyncOptions {
                write: false,
                chain: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(warm.cache_hit);
        assert_eq!(warm.report, cold.report);

        // Explicit bypass & thread-local strict bypass do not hit receipt.
        let bypassed = core_sync_with_options(
            &root,
            SyncOptions {
                write: false,
                chain: false,
                use_cache: false,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!bypassed.cache_hit);
        let strict_bypass = check_receipts::with_cache_bypass(true, || {
            core_sync_with_options(
                &root,
                SyncOptions {
                    write: false,
                    chain: false,
                    use_cache: true,
                    cache_path: Some(cache_file.clone()),
                },
            )
        })
        .unwrap();
        assert!(!strict_bypass.cache_hit);

        // Target drift invalidates target_tree_sha256 and detects difference.
        write_file(&root, "Laravel/shared/doc.md", b"Drifted\n");
        let target_drift = core_sync_with_options(
            &root,
            SyncOptions {
                write: false,
                chain: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!target_drift.cache_hit);
        assert_eq!(target_drift.report["passed"], false);

        // Restoring target restores receipt hit.
        write_file(&root, "Laravel/shared/doc.md", b"Shared doc\n");
        let restored = core_sync_with_options(
            &root,
            SyncOptions {
                write: false,
                chain: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(restored.cache_hit);

        // Manifest rules drift invalidates rules_sha256.
        write_file(
            &root,
            "install/core-manifest.json",
            &common::json_bytes(&json!({
                "version": 1,
                "canonical": "PHP Core",
                "editions": {
                    "php-core": "PHP Core",
                    "laravel": "Laravel"
                },
                "includes": ["shared/**", "extra/**"],
                "allowed_drift": {},
                "retired": []
            }))
            .unwrap(),
        );
        write_file(&root, "PHP Core/extra/new.md", b"New\n");
        let rules_drift = core_sync_with_options(
            &root,
            SyncOptions {
                write: false,
                chain: false,
                use_cache: true,
                cache_path: Some(cache_file),
            },
        )
        .unwrap();
        assert!(!rules_drift.cache_hit);
        assert_eq!(rules_drift.report["passed"], false);
    }

    #[test]
    fn asset_sync_check_receipt_hits_when_unchanged_and_invalidates_on_contract_drift() {
        let temp = Temporary::new("infra-asset-sync-receipts").unwrap();
        let root = infra_core::fs::canonicalize(temp.path()).unwrap();
        let cache_dir = Temporary::new("infra-asset-sync-cache").unwrap();
        let cache_file = cache_dir.path().join("sync-receipts.json");

        write_file(&root, "PHP Core/memory-bank/MCP.md", b"# MCP\n");
        write_file(
            &root,
            "PHP Core/project-brain/config/runtime.json",
            br#"{"framework":"php-core","canonical_edition":".agents"}"#,
        );
        write_file(
            &root,
            "runtime-rust/crates/infra-generator/resources/native-runtime-contract.json",
            br#"{"native_engine":{"unix_path":"memory-bank/bin/infra-engine","windows_path":"memory-bank/bin/infra-engine.exe","capability_profile":"infra-engine-v1"}}"#,
        );
        std::fs::create_dir_all(
            root.join("Infrastructure-Creator/.agents/skills/memory-seed/assets"),
        )
        .unwrap();

        let write_res = asset_sync_with_options(
            &root,
            SyncOptions {
                write: true,
                chain: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!write_res.cache_hit);

        let cold = asset_sync_with_options(
            &root,
            SyncOptions {
                write: false,
                chain: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!cold.cache_hit);
        assert_eq!(cold.report["passed"], true);

        let warm = asset_sync_with_options(
            &root,
            SyncOptions {
                write: false,
                chain: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(warm.cache_hit);
        assert_eq!(warm.report, cold.report);

        // Editing canonical source invalidates receipt.
        write_file(&root, "PHP Core/memory-bank/MCP.md", b"# MCP v2\n");
        let after_edit = asset_sync_with_options(
            &root,
            SyncOptions {
                write: false,
                chain: false,
                use_cache: true,
                cache_path: Some(cache_file),
            },
        )
        .unwrap();
        assert!(!after_edit.cache_hit);
        assert_eq!(after_edit.report["passed"], false);
    }
}
