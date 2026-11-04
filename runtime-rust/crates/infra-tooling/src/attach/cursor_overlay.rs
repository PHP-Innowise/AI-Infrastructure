//! Private Cursor plugin images. Manifest component paths remain plugin-relative.
//! Cursor's plugin/MCP contracts are documented at:
//! https://cursor.com/docs/reference/plugins
//! https://cursor.com/docs/mcp
use super::{normalize_hook_configuration, project_text};
use crate::{common, mcp_config};
use infra_core::{Result, RuntimeError, bank::publication::PublicationFiles, fs};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const FILE_LIMIT: usize = 64 * 1024 * 1024;
const IMAGE_LIMIT: usize = 256 * 1024 * 1024;
const NODE_LIMIT: usize = 20_000;
const MANIFEST: &str = ".cursor-plugin/plugin.json";
const HOOKS: &str = ".cursor/hooks.json";
const MCP: &str = ".cursor/mcp.json";
type SourceIdentity = (u64, u64, u32, u64, u64, i128, i128);
struct SourceReceipt {
    identity: SourceIdentity,
    sha256: String,
}
fn verify_sources(
    source: &mut PublicationFiles,
    receipts: &BTreeMap<String, SourceReceipt>,
    directories: &BTreeMap<String, Vec<String>>,
) -> Result<()> {
    for (path, expected) in directories {
        if source.audit_tree(path, 64, NODE_LIMIT)?.0 != *expected {
            return Err(RuntimeError::conflict(
                "Cursor plugin source directory membership changed",
            ));
        }
    }
    for (path, expected) in receipts {
        let (bytes, identity) = source
            .capture_source(path, FILE_LIMIT)?
            .ok_or_else(|| RuntimeError::conflict("Cursor plugin source asset disappeared"))?;
        if identity.3 != 1
            || identity != expected.identity
            || common::sha256(&bytes) != expected.sha256
        {
            return Err(RuntimeError::conflict(
                "Cursor plugin source identity or bytes changed",
            ));
        }
    }
    source.check()
}

fn relative(value: &str) -> Result<String> {
    let value = value.strip_prefix("./").unwrap_or(value);
    let value = value.strip_suffix('/').unwrap_or(value);
    if value.chars().any(char::is_control) {
        return Err(RuntimeError::invalid(
            "Control characters in Cursor plugin path",
        ));
    }
    common::relative(value)?;
    Ok(value.into())
}
fn component_paths(value: &Value, paths: &mut BTreeSet<String>) -> Result<()> {
    match value {
        Value::String(path) => {
            paths.insert(relative(path)?);
        }
        Value::Array(values) => {
            if values.len() > 128 {
                return Err(RuntimeError::invalid(
                    "Too many Cursor plugin component paths",
                ));
            }
            for value in values {
                component_paths(value, paths)?;
            }
        }
        _ => {
            return Err(RuntimeError::invalid(
                "Cursor plugin component paths must be strings",
            ));
        }
    }
    Ok(())
}
fn capture(
    source: &mut PublicationFiles,
    home: &Path,
    path: &str,
    image: &mut BTreeMap<String, (Vec<u8>, u32)>,
    total: &mut usize,
    receipts: &mut BTreeMap<String, SourceReceipt>,
    directories: &mut BTreeMap<String, Vec<String>>,
) -> Result<()> {
    let metadata = std::fs::symlink_metadata(home.join(path))?;
    if fs::linked(&metadata) {
        return Err(RuntimeError::policy(
            "Linked Cursor plugin asset is refused",
        ));
    }
    let paths = if metadata.is_dir() {
        let paths = source.audit_tree(path, 64, NODE_LIMIT)?.0;
        directories.insert(path.into(), paths.clone());
        paths
    } else if metadata.is_file() {
        vec![path.to_string()]
    } else {
        return Err(RuntimeError::policy(
            "Cursor plugin assets must be ordinary files",
        ));
    };
    for path in paths {
        if image.contains_key(&path) {
            continue;
        }
        if image.len() >= NODE_LIMIT && !image.contains_key(&path) {
            return Err(RuntimeError::invalid(
                "Cursor plugin image exceeds the file bound",
            ));
        }
        let (bytes, identity) = source
            .capture_source(&path, FILE_LIMIT)?
            .ok_or_else(|| RuntimeError::conflict("Cursor plugin source asset disappeared"))?;
        if identity.3 != 1 {
            return Err(RuntimeError::policy(
                "Hard-linked Cursor plugin source is refused",
            ));
        }
        *total = total
            .checked_add(bytes.len())
            .filter(|value| *value <= IMAGE_LIMIT)
            .ok_or_else(|| RuntimeError::invalid("Cursor plugin image exceeds the byte bound"))?;
        let mode = if identity.2 & 0o111 != 0 {
            0o700
        } else {
            0o600
        };
        receipts.insert(
            path.clone(),
            SourceReceipt {
                identity,
                sha256: common::sha256(&bytes),
            },
        );
        image.insert(path, (bytes, mode));
    }
    Ok(())
}
pub(super) fn materialize(
    repository: &Path,
    home: &Path,
    state: &Path,
    output: &mut PublicationFiles,
    engine: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<PathBuf> {
    let repository = fs::absolute(repository)?;
    let home = fs::absolute(home)?;
    let state = fs::absolute(state)?;
    if output.root_path() != state.as_path() {
        return Err(RuntimeError::policy(
            "Cursor plugin custody names another state root",
        ));
    }
    output.check()?;
    if state == repository || state.starts_with(&repository) || repository.starts_with(&state) {
        return Err(RuntimeError::policy(
            "Cursor plugin state must be separate from the accelerator clone",
        ));
    }
    if !engine.is_absolute() {
        return Err(RuntimeError::invalid(
            "Cursor plugin requires an absolute native engine",
        ));
    }
    let mut source = PublicationFiles::new(&home)?;
    let mut image = BTreeMap::<String, (Vec<u8>, u32)>::new();
    let mut total = 0usize;
    let mut receipts = BTreeMap::<String, SourceReceipt>::new();
    let mut directories = BTreeMap::<String, Vec<String>>::new();
    capture(
        &mut source,
        &home,
        ".cursor",
        &mut image,
        &mut total,
        &mut receipts,
        &mut directories,
    )?;
    capture(
        &mut source,
        &home,
        ".cursor-plugin",
        &mut image,
        &mut total,
        &mut receipts,
        &mut directories,
    )?;
    let manifest_bytes = image
        .get(MANIFEST)
        .ok_or_else(|| RuntimeError::not_found("Selected edition has no Cursor plugin manifest"))?
        .0
        .clone();
    let mut manifest = infra_core::strict_json::parse(&manifest_bytes, FILE_LIMIT, 64, 100_000)?;
    if !manifest.is_object() {
        return Err(RuntimeError::invalid(
            "Cursor plugin manifest must be an object",
        ));
    }
    let mut paths = BTreeSet::new();
    for (field, fallback) in [
        ("rules", "rules"),
        ("skills", "skills"),
        ("agents", "agents"),
        ("commands", "commands"),
    ] {
        if let Some(value) = manifest.get(field) {
            component_paths(value, &mut paths)?;
        } else if home.join(fallback).exists() {
            paths.insert(fallback.into());
        }
    }
    if let Some(Value::String(logo)) = manifest.get("logo")
        && !logo.starts_with("https://")
        && !logo.starts_with("http://")
    {
        paths.insert(relative(logo)?);
    }
    for path in paths {
        capture(
            &mut source,
            &home,
            &path,
            &mut image,
            &mut total,
            &mut receipts,
            &mut directories,
        )?;
    }
    // Ready editions intentionally have one canonical hook and MCP source.
    // Refuse an unknown routing shape rather than discarding any plugin config.
    if manifest.get("hooks").is_some_and(|value| {
        value
            .as_str()
            .and_then(|path| relative(path).ok())
            .as_deref()
            != Some(HOOKS)
    }) {
        return Err(RuntimeError::invalid(
            "Attached Cursor plugins require their canonical .cursor/hooks.json routing",
        ));
    }
    if manifest.get("mcpServers").is_some_and(|value| {
        value
            .as_str()
            .and_then(|path| relative(path).ok())
            .as_deref()
            != Some(MCP)
    }) {
        return Err(RuntimeError::invalid(
            "Attached Cursor plugins require their canonical .cursor/mcp.json routing",
        ));
    }
    let hooks = image
        .get(HOOKS)
        .ok_or_else(|| RuntimeError::not_found("Cursor hook configuration is absent"))?;
    // An attached overlay lives outside the project: it binds this machine's engine.
    let launch = super::HookLaunch::Bound {
        engine: engine.into(),
        home: home.clone(),
        windows: cfg!(windows),
    };
    let hooks = normalize_hook_configuration(&hooks.0, "cursor", &launch)?;
    image.insert(HOOKS.into(), (hooks, 0o600));
    let mcp = image
        .get(MCP)
        .ok_or_else(|| RuntimeError::not_found("Cursor memory MCP configuration is absent"))?;
    // The shared merger admits only the accelerator-owned memory registration,
    // while preserving every unrelated server and document field.
    let merged = mcp_config::merge(MCP, Some(&mcp.0), &mcp.0, &home)?;
    let mut mcp: Value = serde_json::from_slice(&merged)?;
    mcp["mcpServers"]["harness-memory"] = json!({
        "type":"stdio","command":project_text(engine)?,
        "args":["mcp","--root",state,"--host","cursor"],"env":environment,
    });
    image.insert(MCP.into(), (common::json_bytes(&mcp)?, 0o600));
    manifest["hooks"] = format!("./{HOOKS}").into();
    // Plugin discovery otherwise looks for mcp.json at the plugin root.
    manifest["mcpServers"] = format!("./{MCP}").into();
    image.insert(MANIFEST.into(), (common::json_bytes(&manifest)?, 0o600));

    image.values().try_fold(0usize, |total, (bytes, _)| {
        total
            .checked_add(bytes.len())
            .filter(|total| *total <= IMAGE_LIMIT)
            .ok_or_else(|| RuntimeError::invalid("Cursor plugin image exceeds the byte bound"))
    })?;
    let identity = image
        .iter()
        .map(|(path, (bytes, mode))| {
            (
                path.clone(),
                json!({"sha256":common::sha256(bytes),"mode":mode}),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let digest = common::sha256(&serde_json::to_vec(&identity)?);
    let prefix = format!("launch/cursor-plugin-{digest}");
    let destination = state.join(&prefix);
    // Bind the complete raw source receipt set before writing private assets.
    verify_sources(&mut source, &receipts, &directories)?;
    output.check()?;
    // Publish the manifest last; callers receive a directory only after the
    // complete image has been verified. A changed source gets a new image.
    for (path, (bytes, mode)) in image
        .iter()
        .filter(|(path, _)| path.as_str() != MANIFEST)
        .chain(image.iter().filter(|(path, _)| path.as_str() == MANIFEST))
    {
        if path.as_str() == MANIFEST {
            // Recheck again at the activation boundary, after the private assets
            // were written and before a manifest can expose their frozen image.
            verify_sources(&mut source, &receipts, &directories)?;
        }
        let relative = format!("{prefix}/{path}");
        let previous = output.read(&relative, FILE_LIMIT)?;
        if let Some(previous) = previous {
            if previous.as_slice() != bytes.as_slice()
                || output.read_mode(&relative)?
                    != Some(infra_core::bank::publication::kept_mode(*mode))
            {
                return Err(RuntimeError::conflict(
                    "Private Cursor plugin image was changed",
                ));
            }
        } else {
            output.write_expected_mode(&relative, bytes, None, *mode, FILE_LIMIT)?;
        }
    }
    source.check()?;
    let (actual, _) = output.audit_tree(&prefix, 64, NODE_LIMIT)?;
    let expected = image
        .keys()
        .map(|path| format!("{prefix}/{path}"))
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(RuntimeError::conflict(
            "Private Cursor plugin image contains unexpected assets",
        ));
    }
    output.check()?;
    output.secure_private_directory("launch", false)?;
    output.secure_private_directory(&prefix, false)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        repository: PathBuf,
        home: PathBuf,
        state: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("infra-cursor-overlay-{}", uuid::Uuid::new_v4()));
            let repository = root.join("tooling");
            let home = repository.join("PHP Core");
            let state = root.join("external state");
            for path in [
                ".cursor-plugin",
                ".cursor/rules",
                ".cursor/skills/sample/assets",
                ".cursor/agents",
                ".cursor/commands",
            ] {
                std::fs::create_dir_all(home.join(path)).unwrap();
            }
            std::fs::create_dir_all(&state).unwrap();
            std::fs::create_dir_all(root.join("served project")).unwrap();
            std::fs::write(home.join(".cursor-plugin/plugin.json"),common::json_bytes(&json!({
                "name":"fixture-accelerator","rules":["./.cursor/rules/"],
                "skills":"./.cursor/skills/","agents":"./.cursor/agents/","commands":"./.cursor/commands/",
                "hooks":"./.cursor/hooks.json","description":"fixture metadata",
            })).unwrap()).unwrap();
            for (path, bytes) in [
                (
                    ".cursor/rules/policy.mdc",
                    "---\nalwaysApply: true\n---\nPolicy\n",
                ),
                (
                    ".cursor/skills/sample/SKILL.md",
                    "---\nname: sample\ndescription: Fixture skill.\n---\n",
                ),
                (".cursor/skills/sample/assets/data.txt", "original asset"),
                (".cursor/agents/sample.md", "Agent fixture"),
                (".cursor/commands/sample.md", "Command fixture"),
            ] {
                std::fs::write(home.join(path), bytes).unwrap();
            }
            std::fs::write(
                home.join(".cursor/hooks.json"),
                common::json_bytes(&json!({
                    "version":1,"hooks":{"sessionStart":[{
                        "command":"memory-bank/bin/infra-engine hook cursor local-context --root .",
                        "timeout":15,"matcher":"retained-metadata",
                    }]}
                }))
                .unwrap(),
            )
            .unwrap();
            let mut mcp: Value =
                serde_json::from_slice(&mcp_config::template("cursor", &home).unwrap()).unwrap();
            mcp["mcpServers"]["independent-server"] = json!({"url":"https://example.invalid/mcp"});
            std::fs::write(
                home.join(".cursor/mcp.json"),
                common::json_bytes(&mcp).unwrap(),
            )
            .unwrap();
            Self {
                root,
                repository,
                home,
                state,
            }
        }
        fn environment(&self) -> BTreeMap<String, String> {
            BTreeMap::from([
                ("ACCELERATOR_HOME".into(), project_text(&self.home).unwrap()),
                (
                    "ACCELERATOR_STATE_DIR".into(),
                    project_text(&self.state).unwrap(),
                ),
                (
                    "ACCELERATOR_PROJECT_DIR".into(),
                    project_text(&self.root.join("served project")).unwrap(),
                ),
                ("ACCELERATOR_EDITION".into(), "PHP Core".into()),
            ])
        }
        fn prepared_state(&self) -> Result<PublicationFiles> {
            super::super::prepare_state_custody(
                &self.repository,
                &self.root.join("served project"),
                &self.state,
                "PHP Core",
            )
            .map(|(files, _)| files)
        }
        fn materialize_prepared(&self, prepared: &mut PublicationFiles) -> Result<PathBuf> {
            materialize(
                &self.repository,
                &self.home,
                &self.state,
                prepared,
                &std::env::current_exe()?,
                &self.environment(),
            )
        }
    }
    impl Fixture {
        fn materialize(&self) -> Result<PathBuf> {
            let mut output = self.prepared_state()?;
            self.materialize_prepared(&mut output)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn private_image_preserves_assets_and_binds_native_hooks_and_memory_to_attached_state() {
        let fixture = Fixture::new();
        let old_manifest = std::fs::read(fixture.home.join(MANIFEST)).unwrap();
        let old_hooks = std::fs::read(fixture.home.join(HOOKS)).unwrap();
        let old_mcp = std::fs::read(fixture.home.join(MCP)).unwrap();
        let image = fixture.materialize().unwrap();
        assert!(image.starts_with(&fixture.state));
        assert!(!image.starts_with(&fixture.home));
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(image.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(manifest["description"], "fixture metadata");
        assert_eq!(manifest["rules"], json!(["./.cursor/rules/"]));
        assert_eq!(manifest["mcpServers"], "./.cursor/mcp.json");
        for asset in [
            ".cursor/rules/policy.mdc",
            ".cursor/skills/sample/SKILL.md",
            ".cursor/skills/sample/assets/data.txt",
            ".cursor/agents/sample.md",
            ".cursor/commands/sample.md",
        ] {
            assert_eq!(
                std::fs::read(image.join(asset)).unwrap(),
                std::fs::read(fixture.home.join(asset)).unwrap()
            );
        }
        let hooks: Value =
            serde_json::from_slice(&std::fs::read(image.join(HOOKS)).unwrap()).unwrap();
        let handler = &hooks["hooks"]["sessionStart"][0];
        let expected = super::super::hook_handler(
            &std::env::current_exe().unwrap(),
            &fixture.home,
            "cursor",
            "local-context",
            cfg!(windows),
        )
        .unwrap();
        assert_eq!(handler["command"], expected["command"]);
        assert_eq!(handler["timeout"], 15);
        assert_eq!(handler["matcher"], "retained-metadata");
        assert_eq!(hooks["version"], 1);
        let mcp: Value = serde_json::from_slice(&std::fs::read(image.join(MCP)).unwrap()).unwrap();
        let memory = &mcp["mcpServers"]["harness-memory"];
        assert_eq!(memory["command"], json!(std::env::current_exe().unwrap()));
        assert_eq!(
            memory["args"],
            json!(["mcp", "--root", fixture.state, "--host", "cursor"])
        );
        assert_eq!(memory["env"], json!(fixture.environment()));
        assert_eq!(
            mcp["mcpServers"]["independent-server"]["url"],
            "https://example.invalid/mcp"
        );
        assert_eq!(
            std::fs::read(fixture.home.join(MANIFEST)).unwrap(),
            old_manifest
        );
        assert_eq!(std::fs::read(fixture.home.join(HOOKS)).unwrap(), old_hooks);
        assert_eq!(std::fs::read(fixture.home.join(MCP)).unwrap(), old_mcp);
        assert_eq!(fixture.materialize().unwrap(), image);
        std::fs::write(
            fixture.home.join(".cursor/skills/sample/assets/data.txt"),
            "changed asset",
        )
        .unwrap();
        let changed = fixture.materialize().unwrap();
        assert_ne!(changed, image);
        assert_eq!(
            std::fs::read(image.join(".cursor/skills/sample/assets/data.txt")).unwrap(),
            b"original asset"
        );
        std::fs::write(
            changed.join(".cursor/skills/sample/assets/data.txt"),
            "unowned edit",
        )
        .unwrap();
        assert!(
            fixture.materialize().is_err(),
            "never overwrite an edited private image"
        );
    }

    #[test]
    fn replaced_source_with_identical_bytes_is_not_the_captured_source() {
        let fixture = Fixture::new();
        let name = ".cursor/skills/sample/assets/data.txt";
        let mut source = PublicationFiles::new(&fixture.home).unwrap();
        let mut image = BTreeMap::new();
        let mut receipts = BTreeMap::new();
        let mut directories = BTreeMap::new();
        capture(
            &mut source,
            &fixture.home,
            name,
            &mut image,
            &mut 0,
            &mut receipts,
            &mut directories,
        )
        .unwrap();
        let path = fixture.home.join(name);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::rename(&path, fixture.root.join("retired-source")).unwrap();
        std::fs::write(&path, bytes).unwrap();
        assert!(verify_sources(&mut source, &receipts, &directories).is_err());
        assert!(!fixture.state.join("launch").exists());
    }

    #[cfg(unix)]
    #[test]
    fn source_mode_is_captured_with_the_bytes_and_rechecked() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        let name = ".cursor/skills/sample/assets/data.txt";
        let path = fixture.home.join(name);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut source = PublicationFiles::new(&fixture.home).unwrap();
        let mut image = BTreeMap::new();
        let mut receipts = BTreeMap::new();
        let mut directories = BTreeMap::new();
        capture(
            &mut source,
            &fixture.home,
            name,
            &mut image,
            &mut 0,
            &mut receipts,
            &mut directories,
        )
        .unwrap();
        assert_eq!(image[name].1, 0o700);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(verify_sources(&mut source, &receipts, &directories).is_err());
    }

    #[test]
    fn hardlinked_sources_are_refused_even_for_direct_component_files() {
        let fixture = Fixture::new();
        let name = ".cursor/skills/sample/assets/data.txt";
        std::fs::hard_link(fixture.home.join(name), fixture.root.join("second-name")).unwrap();
        let mut source = PublicationFiles::new(&fixture.home).unwrap();
        assert!(
            capture(
                &mut source,
                &fixture.home,
                name,
                &mut BTreeMap::new(),
                &mut 0,
                &mut BTreeMap::new(),
                &mut BTreeMap::new()
            )
            .is_err()
        );
        assert!(!fixture.state.join("launch").exists());
    }

    #[test]
    fn new_component_assets_after_capture_require_a_new_selection() {
        let fixture = Fixture::new();
        let mut source = PublicationFiles::new(&fixture.home).unwrap();
        let mut image = BTreeMap::new();
        let mut receipts = BTreeMap::new();
        let mut directories = BTreeMap::new();
        capture(
            &mut source,
            &fixture.home,
            ".cursor",
            &mut image,
            &mut 0,
            &mut receipts,
            &mut directories,
        )
        .unwrap();
        std::fs::write(
            fixture.home.join(".cursor/skills/sample/assets/new.txt"),
            "new asset",
        )
        .unwrap();
        assert!(verify_sources(&mut source, &receipts, &directories).is_err());
        assert!(!fixture.state.join("launch").exists());
    }

    #[cfg(unix)]
    #[test]
    fn linked_plugin_assets_are_refused_before_publication() {
        let fixture = Fixture::new();
        std::os::unix::fs::symlink(
            "/etc/passwd",
            fixture.home.join(".cursor/skills/sample/assets/linked"),
        )
        .unwrap();
        assert!(fixture.materialize().is_err());
        assert!(!fixture.state.join("launch").exists());
    }

    #[test]
    fn replaced_prepared_state_is_refused_before_cursor_plugin_writes() {
        let fixture = Fixture::new();
        let mut prepared = fixture.prepared_state().unwrap();
        let record = std::fs::read(fixture.state.join("accelerator-attach.json")).unwrap();
        let retired = fixture.root.join("retained prepared state");
        let moved = std::fs::rename(&fixture.state, &retired);
        if cfg!(windows) {
            // The prepared state holds its root open without delete sharing,
            // so Windows refuses to move it: no replacement can be admitted.
            let error = moved.expect_err("Windows moved a held prepared state root");
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
            return;
        }
        moved.unwrap();
        std::fs::create_dir(&fixture.state).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fixture.state, std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let error = fixture
            .materialize_prepared(&mut prepared)
            .expect_err("Cursor plugin publisher re-admitted a replacement state root");
        assert_eq!(error.kind, infra_core::ErrorKind::Conflict);
        assert!(
            std::fs::read_dir(&fixture.state).unwrap().next().is_none(),
            "replacement state received Cursor assets"
        );
        assert!(
            !retired.join("launch").exists(),
            "refusal wrote through the retained retired directory"
        );
        assert_eq!(
            std::fs::read(retired.join("accelerator-attach.json")).unwrap(),
            record
        );
    }
}
