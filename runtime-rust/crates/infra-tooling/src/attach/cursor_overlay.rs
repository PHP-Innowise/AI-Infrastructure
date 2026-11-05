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
    time::{SystemTime, UNIX_EPOCH},
};

const FILE_LIMIT: usize = 64 * 1024 * 1024;
const IMAGE_LIMIT: usize = 256 * 1024 * 1024;
const NODE_LIMIT: usize = 20_000;
const MANIFEST: &str = ".cursor-plugin/plugin.json";
const HOOKS: &str = ".cursor/hooks.json";
const MCP: &str = ".cursor/mcp.json";
/// Each image is `launch/cursor-plugin-<digest>`, named for its content. The
/// MCP configuration it carries names the launch folder, so every folder of
/// the project - its root and each worktree - and every version of the
/// edition's Cursor files has an image of its own.
const IMAGE: &str = "cursor-plugin-";
/// When each image was last handed to a launch, in seconds since 1970, and
/// the image after which the next sweep starts:
/// `{"handed":{"cursor-plugin-<digest>":<seconds>},"next":"cursor-plugin-<digest>"}`.
const HANDOUTS: &str = "launch/cursor-plugins.json";
const HANDOUTS_LIMIT: usize = 4 * 1024 * 1024;
/// An image's MCP configuration, read for the launch it was made for.
const DESCRIPTION_LIMIT: usize = 1024 * 1024;
/// Images one launch looks at besides its own, and removes if no launch can
/// still be handed them. Every launch of the project walks every image to
/// keep the state private, which costs far more than removing some: with 21
/// PHP Core images a Cursor launch took 32 s, and one that removed 16 of them
/// 37 s; the next, with 5 left, took 3 s, and with one 0.6 s.
const SWEEP: usize = 16;
/// A Cursor host reads its plugin for as long as its session runs, not only
/// when it starts, so an image this launcher no longer hands its folder is
/// kept for a day after a launch was last handed it.
const SUPERSEDED_KEPT: u64 = 24 * 60 * 60;
/// A launch rewrites its own image's hand-out once the record is an hour
/// old, so a record trails the image's last hand-out by less than an hour.
const HANDOUT_REFRESH: u64 = 60 * 60;
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
    let mcp: Value = serde_json::from_slice(&merged)?;
    let engine_text = project_text(engine)?;
    image.insert(
        MCP.into(),
        (
            memory_configuration(&mcp, &engine_text, &state, environment)?,
            0o600,
        ),
    );
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
    let digest = image_digest(&identity)?;
    let name = format!("{IMAGE}{digest}");
    let prefix = format!("launch/{name}");
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
    let launcher = Launcher {
        state: &state,
        engine: &engine_text,
        environment,
        mcp: &mcp,
        identity: &identity,
    };
    sweep(output, &name, &launcher, seconds())?;
    Ok(destination)
}

fn memory_configuration(
    mcp: &Value,
    engine: &str,
    state: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    let mut mcp = mcp.clone();
    mcp["mcpServers"]["harness-memory"] = json!({
        "type":"stdio","command":engine,
        "args":["mcp","--root",state,"--host","cursor"],"env":environment,
    });
    common::json_bytes(&mcp)
}
fn image_digest(identity: &BTreeMap<String, Value>) -> Result<String> {
    Ok(common::sha256(&serde_json::to_vec(identity)?))
}
fn seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
/// The digest an image directory is named for, if `name` is one.
fn named_digest(name: &str) -> Option<&str> {
    name.strip_prefix(IMAGE).filter(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}
/// Whether nothing is left at the absolute path `text`.
fn gone(text: &str) -> bool {
    let path = Path::new(text);
    path.is_absolute()
        && std::fs::symlink_metadata(path)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

/// What this launch hands a folder. A launcher's images for the folders of a
/// project differ in the folder its MCP environment names, and nowhere else.
struct Launcher<'a> {
    state: &'a Path,
    engine: &'a str,
    environment: &'a BTreeMap<String, String>,
    mcp: &'a Value,
    identity: &'a BTreeMap<String, Value>,
}
impl Launcher<'_> {
    /// The digest of the image this launcher hands a launch from `folder`.
    fn digest(&self, folder: &str) -> Result<String> {
        let mut environment = self.environment.clone();
        environment.insert("ACCELERATOR_PROJECT_DIR".into(), folder.into());
        let configuration = memory_configuration(self.mcp, self.engine, self.state, &environment)?;
        let mut identity = self.identity.clone();
        identity.insert(
            MCP.into(),
            json!({"sha256":common::sha256(&configuration),"mode":0o600}),
        );
        image_digest(&identity)
    }
    /// Whether this launcher made `image`, from this state, engine, clone and
    /// edition, if perhaps from other sources of the clone.
    fn made(&self, image: &Made) -> bool {
        image.engine == self.engine
            && self.environment.get("ACCELERATOR_HOME") == Some(&image.home)
            && self.environment.get("ACCELERATOR_EDITION") == Some(&image.edition)
    }
}

/// The launch an image was made for, as its MCP configuration names it.
struct Made {
    folder: String,
    home: String,
    edition: String,
    engine: String,
}
impl Made {
    fn read(bytes: &[u8], state: &Path) -> Option<Self> {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let server = value.get("mcpServers")?.get("harness-memory")?;
        // An image made for this state names it as its memory server's root.
        if server.get("args")? != &json!(["mcp", "--root", state, "--host", "cursor"]) {
            return None;
        }
        let environment = server.get("env")?;
        let text = |key: &str| Some(environment.get(key)?.as_str()?.to_string());
        Some(Self {
            folder: text("ACCELERATOR_PROJECT_DIR")?,
            home: text("ACCELERATOR_HOME")?,
            edition: text("ACCELERATOR_EDITION")?,
            engine: server.get("command")?.as_str()?.into(),
        })
    }
}

/// The hand-out record. One that cannot be read is an empty one: the images
/// it named are then first seen by the next sweeps, which keep them a day.
#[derive(Default)]
struct Handouts {
    handed: BTreeMap<String, u64>,
    next: Option<String>,
}
impl Handouts {
    fn read(bytes: &[u8]) -> Self {
        let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
            return Self::default();
        };
        Self {
            handed: value["handed"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(name, _)| named_digest(name).is_some())
                .filter_map(|(name, seconds)| Some((name.clone(), seconds.as_u64()?)))
                .collect(),
            next: value["next"]
                .as_str()
                .filter(|name| named_digest(name).is_some())
                .map(str::to_string),
        }
    }
    fn bytes(&self) -> Result<Vec<u8>> {
        let mut value = json!({"handed":self.handed});
        if let Some(next) = &self.next {
            value["next"] = json!(next);
        }
        common::json_bytes(&value)
    }
}

enum Verdict {
    Keep,
    FirstSeen,
    Remove,
}
/// Whether a launch can still be handed the image `name`, or a host handed it
/// may still read it. The sweep removes an image only when neither holds:
/// - Launches publish an image's manifest last and the sweep removes it
///   first, so an image without one was never handed as it stands.
/// - A launch starts in its folder and its host runs hooks of its engine
///   against its edition in the clone. An image whose folder, engine or
///   clone is gone is handed to no launch, and a host handed it before is
///   left with nothing to serve.
/// - Otherwise only the launcher that made an image can tell whether it
///   still hands it. Once it no longer hands the image's folder that image -
///   the clone changed since - it keeps the image `SUPERSEDED_KEPT` after the
///   last hand-out, and from when it first sees an image the record does not
///   name.
///
/// An image the sweep cannot read, or whose launch it cannot tell, stays.
fn verdict(
    files: &mut PublicationFiles,
    name: &str,
    launcher: &Launcher,
    handouts: &Handouts,
    now: u64,
) -> Verdict {
    let prefix = format!("launch/{name}");
    match files.read_mode(&format!("{prefix}/{MANIFEST}")) {
        Ok(None) => return Verdict::Remove,
        Ok(Some(_)) => {}
        Err(_) => return Verdict::Keep,
    }
    let Some(made) = files
        .read(&format!("{prefix}/{MCP}"), DESCRIPTION_LIMIT)
        .ok()
        .flatten()
        .and_then(|bytes| Made::read(&bytes, launcher.state))
    else {
        return Verdict::Keep;
    };
    if [&made.folder, &made.engine, &made.home]
        .into_iter()
        .any(|path| gone(path))
    {
        return Verdict::Remove;
    }
    if !launcher.made(&made) {
        return Verdict::Keep;
    }
    match launcher.digest(&made.folder) {
        Ok(digest) if Some(digest.as_str()) != named_digest(name) => {}
        _ => return Verdict::Keep,
    }
    match handouts.handed.get(name) {
        None => Verdict::FirstSeen,
        Some(&handed)
            if now
                .checked_sub(handed)
                .is_some_and(|age| age >= SUPERSEDED_KEPT) =>
        {
            Verdict::Remove
        }
        Some(_) => Verdict::Keep,
    }
}

/// The order a removal takes an image's assets in: its manifest first, so
/// that a removal which stops leaves no plugin behind, and the next sweep
/// removes the rest of an image without one.
fn removal_order<'a>(assets: &'a [String], prefix: &str) -> Vec<&'a String> {
    let manifest = format!("{prefix}/{MANIFEST}");
    assets
        .iter()
        .filter(|asset| **asset == manifest)
        .chain(assets.iter().filter(|asset| **asset != manifest))
        .collect()
}
/// Remove an image's assets, then its folders.
fn remove(files: &mut PublicationFiles, name: &str) -> Result<()> {
    let prefix = format!("launch/{name}");
    let (assets, _) = files.audit_tree(&prefix, 64, NODE_LIMIT)?;
    for asset in removal_order(&assets, &prefix) {
        files.archive_remove_entry(asset)?;
    }
    // The audit holds every folder of the image; deepest first, each is
    // empty when its turn comes.
    let inside = format!("{prefix}/");
    let mut folders = files
        .directory_identities()?
        .into_keys()
        .filter(|folder| folder.starts_with(&inside))
        .collect::<Vec<_>>();
    folders.sort_by_key(|folder| std::cmp::Reverse(folder.matches('/').count()));
    folders.push(prefix);
    for folder in folders {
        if !files.archive_remove_empty_directory(&folder)? {
            return Err(RuntimeError::conflict(
                "Cursor plugin image folder is not empty",
            ));
        }
    }
    Ok(())
}

/// Record that this launch hands the image `current`, and remove images no
/// launch can still be handed (see `verdict`). It runs under the preparation
/// lock, so no other launch writes an image or hands one meanwhile. A launch
/// records its hand-out before it hands the image, and one that cannot fails
/// as one that cannot write its image does. The sweep itself never fails a
/// launch: an image it cannot remove stays for a later one. It looks at
/// `SWEEP` images at most, from where the last sweep stopped.
fn sweep(files: &mut PublicationFiles, current: &str, launcher: &Launcher, now: u64) -> Result<()> {
    let recorded = files.read(HANDOUTS, HANDOUTS_LIMIT)?;
    let mut handouts = recorded.as_deref().map(Handouts::read).unwrap_or_default();
    if let Ok(names) = files.archive_entries("launch", 1_000_000) {
        let names = names.into_iter().collect::<BTreeSet<_>>();
        let mut images = names
            .iter()
            .filter(|name| named_digest(name).is_some() && name.as_str() != current)
            .cloned()
            .collect::<Vec<_>>();
        let start = handouts.next.clone().unwrap_or_else(|| current.into());
        let after = images.partition_point(|name| *name <= start);
        images.rotate_left(after);
        for name in images.into_iter().take(SWEEP) {
            match verdict(files, &name, launcher, &handouts, now) {
                Verdict::Keep => {}
                Verdict::FirstSeen => {
                    handouts.handed.insert(name.clone(), now);
                }
                Verdict::Remove => {
                    if remove(files, &name).is_ok() {
                        handouts.handed.remove(&name);
                    }
                }
            }
            handouts.next = Some(name);
        }
        handouts.handed.retain(|name, _| names.contains(name));
    }
    if handouts
        .handed
        .get(current)
        .is_none_or(|&handed| handed > now || now - handed >= HANDOUT_REFRESH)
    {
        handouts.handed.insert(current.into(), now);
    }
    let bytes = handouts.bytes()?;
    if recorded.as_deref() != Some(bytes.as_slice()) {
        files.write_expected_mode(HANDOUTS, &bytes, recorded.as_deref(), 0o600, HANDOUTS_LIMIT)?;
    }
    files.secure_private_file(HANDOUTS)?;
    files.check()
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
    /// An edition with Cursor plugin sources at `home`.
    fn edition(home: &Path) {
        for path in [
            ".cursor-plugin",
            ".cursor/rules",
            ".cursor/skills/sample/assets",
            ".cursor/agents",
            ".cursor/commands",
        ] {
            std::fs::create_dir_all(home.join(path)).unwrap();
        }
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
            serde_json::from_slice(&mcp_config::template("cursor", home).unwrap()).unwrap();
        mcp["mcpServers"]["independent-server"] = json!({"url":"https://example.invalid/mcp"});
        std::fs::write(
            home.join(".cursor/mcp.json"),
            common::json_bytes(&mcp).unwrap(),
        )
        .unwrap();
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("infra-cursor-overlay-{}", uuid::Uuid::new_v4()));
            let repository = root.join("tooling");
            let home = repository.join("PHP Core");
            let state = root.join("external state");
            edition(&home);
            std::fs::create_dir_all(&state).unwrap();
            std::fs::create_dir_all(root.join("served project")).unwrap();
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
            .map(|(files, _, _)| files)
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
        /// Another folder of the project, such as a worktree, which launches
        /// on the project's state.
        fn folder(&self, name: &str) -> PathBuf {
            let folder = self.root.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            folder
        }
        /// The image a launch from `folder` is handed.
        fn launch(&self, folder: &Path) -> Result<PathBuf> {
            self.launch_by(folder, &std::env::current_exe()?, &self.repository)
        }
        /// The image `engine` hands a launch from `folder`, with the edition
        /// in the clone `repository`.
        fn launch_by(&self, folder: &Path, engine: &Path, repository: &Path) -> Result<PathBuf> {
            let home = repository.join("PHP Core");
            let mut environment = self.environment();
            environment.insert("ACCELERATOR_PROJECT_DIR".into(), project_text(folder)?);
            environment.insert("ACCELERATOR_HOME".into(), project_text(&home)?);
            let mut output = self.prepared_state()?;
            materialize(
                repository,
                &home,
                &self.state,
                &mut output,
                engine,
                &environment,
            )
        }
        fn handouts(&self) -> Value {
            serde_json::from_slice(&std::fs::read(self.state.join(HANDOUTS)).unwrap()).unwrap()
        }
        fn record(&self, handouts: &Value) {
            std::fs::write(
                self.state.join(HANDOUTS),
                common::json_bytes(handouts).unwrap(),
            )
            .unwrap();
        }
        /// Record that a launch was last handed `image` at `seconds`.
        fn handed(&self, image: &Path, seconds: u64) {
            let mut handouts = self.handouts();
            handouts["handed"][name(image)] = json!(seconds);
            self.record(&handouts);
        }
        fn handed_at(&self, image: &Path) -> Option<u64> {
            self.handouts()["handed"][name(image)].as_u64()
        }
    }
    fn name(image: &Path) -> String {
        image.file_name().unwrap().to_str().unwrap().into()
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

    /// A worktree removed since its last launch leaves an image no launch is
    /// handed again: the next launch removes it, and the record forgets it.
    /// The images of folders still there stay.
    #[test]
    fn a_launch_removes_the_images_of_folders_that_are_gone() {
        let fixture = Fixture::new();
        let removed = ["removed worktree", "another removed worktree"].map(|name| {
            let folder = fixture.folder(name);
            (fixture.launch(&folder).unwrap(), folder)
        });
        let kept = fixture.launch(&fixture.folder("worktree")).unwrap();
        for (_, folder) in &removed {
            std::fs::remove_dir_all(folder).unwrap();
        }
        let own = fixture.materialize().unwrap();
        for (image, _) in &removed {
            assert!(
                !image.exists(),
                "the image of a folder that is gone stayed: {}",
                image.display()
            );
        }
        for image in [&kept, &own] {
            assert!(
                image.join(MANIFEST).is_file(),
                "{} was removed",
                image.display()
            );
        }
        for (image, _) in &removed {
            assert_eq!(fixture.handed_at(image), None, "the record kept {image:?}");
        }
    }

    /// Once the clone changes, launches are handed a new image, and a host
    /// handed the old one may still be reading it. The old image stays until
    /// no launch was handed it for a day, counted from its last hand-out.
    #[test]
    fn an_image_this_launcher_no_longer_hands_stays_a_day_after_its_last_launch() {
        let fixture = Fixture::new();
        let old = fixture.materialize().unwrap();
        fixture.handed(&old, 0);
        assert_eq!(fixture.materialize().unwrap(), old);
        std::fs::write(
            fixture.home.join(".cursor/skills/sample/assets/data.txt"),
            "changed asset",
        )
        .unwrap();
        let new = fixture.materialize().unwrap();
        assert_ne!(new, old);
        assert!(
            old.join(MANIFEST).is_file(),
            "an image handed a moment ago was removed"
        );
        fixture.handed(&old, seconds() - SUPERSEDED_KEPT + 60);
        fixture.materialize().unwrap();
        assert!(
            old.join(MANIFEST).is_file(),
            "an image handed within the day was removed"
        );
        fixture.handed(&old, seconds() - SUPERSEDED_KEPT);
        assert_eq!(fixture.materialize().unwrap(), new);
        assert!(
            !old.exists(),
            "an image no launch was handed for a day stayed"
        );
    }

    /// An image the record does not name - handed by a launcher that kept no
    /// record, or named by a record that cannot be read - may have been
    /// handed a moment ago, so its day starts when a launch first sees it.
    #[test]
    fn an_image_the_record_does_not_name_stays_a_day_from_when_it_is_first_seen() {
        let fixture = Fixture::new();
        let old = fixture.materialize().unwrap();
        std::fs::write(
            fixture.home.join(".cursor/skills/sample/assets/data.txt"),
            "changed asset",
        )
        .unwrap();
        let record = fixture.state.join(HANDOUTS);
        for unreadable in [false, true] {
            if unreadable {
                std::fs::write(&record, "{\"handed\":").unwrap();
            } else {
                std::fs::remove_file(&record).unwrap();
            }
            fixture.materialize().unwrap();
            assert!(
                old.join(MANIFEST).is_file(),
                "an image no record named was removed"
            );
            let seen = fixture
                .handed_at(&old)
                .expect("the first sight is recorded");
            assert!(seconds() - seen < 60);
        }
    }

    /// The image this launcher hands a folder is the one the next launch from
    /// there is handed, however long none came.
    #[test]
    fn an_image_this_launcher_still_hands_its_folder_stays_however_long_unused() {
        let fixture = Fixture::new();
        let worktree = fixture.launch(&fixture.folder("worktree")).unwrap();
        fixture.handed(&worktree, 0);
        fixture.materialize().unwrap();
        assert!(
            worktree.join(MANIFEST).is_file(),
            "the image a worktree's next launch is handed was removed"
        );
    }

    /// Images another engine, or the edition in another clone, made for this
    /// state are theirs to judge while they are there: this launcher cannot
    /// tell whether they still hand them. Once the engine or the clone is
    /// gone, no launch is handed them and no host can run their hooks.
    #[test]
    fn images_of_another_engine_or_clone_stay_until_it_is_gone() {
        let fixture = Fixture::new();
        let engine = fixture.root.join("retired engine");
        std::fs::write(&engine, "").unwrap();
        let clone = fixture.root.join("retired clone");
        edition(&clone.join("PHP Core"));
        let served = fixture.root.join("served project");
        let images = [
            fixture
                .launch_by(&served, &engine, &fixture.repository)
                .unwrap(),
            fixture
                .launch_by(&served, &std::env::current_exe().unwrap(), &clone)
                .unwrap(),
        ];
        let own = fixture.materialize().unwrap();
        for image in &images {
            assert_ne!(image, &own);
            fixture.handed(image, 0);
        }
        fixture.materialize().unwrap();
        for image in &images {
            assert!(
                image.join(MANIFEST).is_file(),
                "another launcher's image was removed: {}",
                image.display()
            );
        }
        std::fs::remove_file(&engine).unwrap();
        std::fs::remove_dir_all(&clone).unwrap();
        fixture.materialize().unwrap();
        for image in &images {
            assert!(
                !image.exists(),
                "an image whose engine or clone is gone stayed: {}",
                image.display()
            );
        }
    }

    /// Launches publish an image's manifest last, so an image left without
    /// one - by a launch that stopped while writing it, or by a removal that
    /// stopped - was never handed as it stands: the next launch removes it.
    #[test]
    fn a_launch_removes_an_image_left_without_its_manifest() {
        let fixture = Fixture::new();
        let partial = fixture.launch(&fixture.folder("worktree")).unwrap();
        std::fs::remove_file(partial.join(MANIFEST)).unwrap();
        std::fs::remove_file(partial.join(MCP)).unwrap();
        fixture.materialize().unwrap();
        assert!(!partial.exists(), "an image without its manifest stayed");
    }

    /// A removal takes an image's manifest first, so that one which stops
    /// leaves no plugin behind and the next sweep removes the rest - even
    /// when another asset's path sorts before the manifest's.
    #[test]
    fn a_removal_takes_the_manifest_first() {
        let prefix = format!("launch/{IMAGE}{}", "0".repeat(64));
        let assets = [".cursor-plugin/assets/logo.png", MANIFEST, MCP]
            .map(|path| format!("{prefix}/{path}"));
        assert_eq!(
            removal_order(&assets, &prefix),
            [&assets[1], &assets[0], &assets[2]]
        );
    }

    /// An image whose memory server serves another state was not made for
    /// this one: the sweep leaves it, as it leaves any image whose launch it
    /// cannot tell.
    #[test]
    fn an_image_made_for_another_state_stays() {
        let fixture = Fixture::new();
        let folder = fixture.folder("removed worktree");
        let image = fixture.launch(&folder).unwrap();
        std::fs::remove_dir_all(&folder).unwrap();
        let path = image.join(MCP);
        let mut mcp: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        mcp["mcpServers"]["harness-memory"]["args"][2] = json!(fixture.root.join("another state"));
        std::fs::write(&path, common::json_bytes(&mcp).unwrap()).unwrap();
        fixture.materialize().unwrap();
        assert!(
            image.join(MANIFEST).is_file(),
            "an image made for another state was removed"
        );
    }

    /// The sweep never stops a launch: an image it cannot remove - here one
    /// deeper than any image - stays as it is, and the launch goes on.
    #[test]
    fn an_image_the_sweep_cannot_remove_does_not_stop_the_launch() {
        let fixture = Fixture::new();
        let folder = fixture.folder("removed worktree");
        let orphan = fixture.launch(&folder).unwrap();
        std::fs::remove_dir_all(&folder).unwrap();
        let mut deep = orphan.join(".cursor");
        for _ in 0..64 {
            deep.push("d");
        }
        std::fs::create_dir_all(&deep).unwrap();
        fixture
            .materialize()
            .expect("an image the sweep could not remove stopped the launch");
        assert!(orphan.join(MANIFEST).is_file());
        assert!(deep.is_dir());
    }

    /// One launch looks at `SWEEP` images besides its own, and the next goes
    /// on where it stopped, so every image is looked at however many stay.
    #[test]
    fn a_launch_looks_at_a_bounded_number_of_images_and_the_next_goes_on() {
        let fixture = Fixture::new();
        let own = fixture.materialize().unwrap();
        let mut images = (0..=SWEEP)
            .map(|index| {
                let folder = fixture.folder(&format!("worktree {index}"));
                (fixture.launch(&folder).unwrap(), folder)
            })
            .collect::<Vec<_>>();
        // In the order a sweep goes that no earlier one left a place for:
        // from the launch's own image on.
        images.sort();
        let after = images.partition_point(|(image, _)| name(image) <= name(&own));
        images.rotate_left(after);
        let (last, folder) = images.last().unwrap();
        std::fs::remove_dir_all(folder).unwrap();
        let mut handouts = fixture.handouts();
        handouts.as_object_mut().unwrap().remove("next");
        fixture.record(&handouts);
        fixture.materialize().unwrap();
        assert!(last.exists(), "a launch looked at more than {SWEEP} images");
        fixture.materialize().unwrap();
        assert!(
            !last.exists(),
            "the next launch did not go on where the last stopped"
        );
    }

    /// A launch records that it was handed its image before it hands it. It
    /// rewrites the record once that is an hour old or ahead of the clock,
    /// not on every launch, and the record forgets images that are gone. A
    /// launch that cannot record its hand-out hands no image.
    #[test]
    fn a_launch_records_its_hand_out_before_it_hands_its_image() {
        let fixture = Fixture::new();
        let image = fixture.materialize().unwrap();
        let recent = |fixture: &Fixture| {
            fixture
                .handed_at(&image)
                .is_some_and(|handed| seconds().abs_diff(handed) < 60)
        };
        assert!(recent(&fixture), "the hand-out was not recorded");
        fixture.handed(&image, seconds() - 60);
        let record = std::fs::read(fixture.state.join(HANDOUTS)).unwrap();
        fixture.materialize().unwrap();
        assert_eq!(
            std::fs::read(fixture.state.join(HANDOUTS)).unwrap(),
            record,
            "a record a minute old was rewritten"
        );
        for handed in [seconds() - HANDOUT_REFRESH, seconds() + HANDOUT_REFRESH] {
            fixture.handed(&image, handed);
            fixture.materialize().unwrap();
            assert!(recent(&fixture), "the record of {handed} was kept");
        }
        let gone = format!("{IMAGE}{}", "0".repeat(64));
        let mut handouts = fixture.handouts();
        handouts["handed"][&gone] = json!(1);
        fixture.record(&handouts);
        fixture.materialize().unwrap();
        assert!(
            fixture.handouts()["handed"].get(&gone).is_none(),
            "the record kept an image that is gone"
        );
        std::fs::remove_file(fixture.state.join(HANDOUTS)).unwrap();
        std::fs::create_dir(fixture.state.join(HANDOUTS)).unwrap();
        assert!(
            fixture.materialize().is_err(),
            "a launch that could not record its hand-out handed its image"
        );
    }
}
