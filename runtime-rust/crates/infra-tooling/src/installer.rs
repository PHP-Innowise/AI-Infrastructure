//! Closed-payload native installer. Reads/writes stay rooted in PublicationFiles.
use crate::{common, file_modes::FileModes};
use infra_core::{
    Result, RuntimeError,
    bank::publication::{
        PublicationFiles,
        commit::{ExpectedImage, POST_RENAME_IDENTITY_UNAVAILABLE, PublishedImage, WriteStage},
    },
    fs as safe,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
const FILE_LIMIT: usize = 512 * 1024 * 1024;
const RECORD: &str = "memory-bank/local/accelerator-install.json";
const ENGINE: &str = if cfg!(windows) {
    "memory-bank/bin/infra-engine.exe"
} else {
    "memory-bank/bin/infra-engine"
};
const AGENTS_BEGIN: &str = "<!-- BEGIN ACCELERATOR MANAGED POLICY -->";
const AGENTS_END: &str = "<!-- END ACCELERATOR MANAGED POLICY -->";
const ENTRIES_BEGIN: &str = "# BEGIN ACCELERATOR MANAGED ENTRIES";
const ENTRIES_END: &str = "# END ACCELERATOR MANAGED ENTRIES";
const SEED: &[&str] = &[
    "memory-bank/INDEX.md",
    "project-brain/config/runtime.json",
    "project-brain/indexes/active.json",
    "project-brain/indexes/archive.json",
    "specs/MANIFEST.md",
    "tasks/.task-counter",
];
const OVERRIDES: &[(&str, &str)] = &[
    (".gitattributes", ".install/gitattributes"),
    ("memory-bank/INDEX.md", "memory-bank/.install/INDEX.md"),
    (
        "project-brain/indexes/active.json",
        "project-brain/.install/active.json",
    ),
    (
        "project-brain/indexes/archive.json",
        "project-brain/.install/archive.json",
    ),
];
const SOURCE_ONLY: &[&str] = &[
    "Task/**",
    "examples/completed-task/**",
    "memory-bank/tests/**",
    "memory-bank/chunks/**",
    "project-brain/tests/**",
    "project-brain/control/handoffs/*.md",
    "project-brain/control/messages/*.jsonl",
    "project-brain/control/retrieval-manifests/*.json",
    "project-brain/control/result-attempts/*.json",
    "project-brain/control/result-learning-images/*.json",
    "project-brain/dynamic/*/*.md",
    "*/skills/skill-creator/tests/**",
    ".cursor-plugin/**",
    "memory-bank/scripts/*.py",
    "memory-bank/scripts/*.sh",
    "project-brain/scripts/*.py",
];
const SOURCE_EXACT: &[&str] = &[
    ".install/gitattributes",
    "CHANGELOG.md",
    "examples/context-summary.md",
    "examples/pr-description.md",
    "memory-bank/.install/INDEX.md",
    "memory-bank/.memory-counter",
    "project-brain/.install/active.json",
    "project-brain/.install/archive.json",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledFile {
    pub path: String,
    pub source: PathBuf,
    pub executable: bool,
    pub component: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inventory {
    pub edition: String,
    pub release: String,
    pub files: Vec<InstalledFile>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InstallOptions {
    pub dry_run: bool,
    pub overwrite: bool,
    pub merge_existing: bool,
    pub update_tracked: bool,
    pub rewire_codex: bool,
    pub allow_downgrade: bool,
}
pub struct Installer {
    pub source: PathBuf,
    engine: PathBuf,
}
#[derive(Clone)]
struct Effect {
    path: String,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    mode: u32,
    original_mode: u32,
    action: String,
    record: bool,
}
type PlannedEffects = (
    PathBuf,
    Inventory,
    Value,
    Vec<Effect>,
    Vec<Value>,
    Vec<Value>,
);
fn source_only(name: &str) -> Result<bool> {
    if name.ends_with(".py") {
        return Ok(true);
    }
    if SOURCE_EXACT.contains(&name) {
        return Ok(true);
    }
    for pattern in SOURCE_ONLY {
        if common::matches(pattern, name, false)? {
            return Ok(true);
        }
    }
    Ok(false)
}
fn component(name: &str) -> &'static str {
    if [
        ".agents/README.md",
        ".claude/README.md",
        ".cursor/README.md",
        ".codex/README.md",
    ]
    .contains(&name)
    {
        "shared"
    } else if name == ".mcp.json" || name.starts_with(".claude/") {
        "claude"
    } else if name.starts_with(".cursor/") {
        "cursor"
    } else if name.starts_with(".agents/") || name.starts_with(".codex/") {
        "codex"
    } else {
        "shared"
    }
}
fn slug(edition: &str) -> &str {
    match edition {
        "Laravel" => "laravel",
        "Symfony" => "symfony",
        "PHP Core" => "php-core",
        "Cms/wordpress" => "wordpress",
        _ => edition,
    }
}
fn legacy_blob(bytes: &[u8]) -> String {
    let mut digest = Sha1::new();
    digest.update(format!("blob {}\0", bytes.len()).as_bytes());
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}
fn identity_matches(known: Option<&str>, bytes: &[u8]) -> bool {
    known.is_some_and(|known| known == common::sha256(bytes) || known == legacy_blob(bytes))
}
fn remove_block(text: &str, begin: &str, end: &str) -> Result<String> {
    let starts = text.match_indices(begin).collect::<Vec<_>>();
    let ends = text.match_indices(end).collect::<Vec<_>>();
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => Ok(text.into()),
        ([(start, _)], [(finish, _)]) if finish > start => {
            let line_start = text[..*start]
                .rfind('\n')
                .map(|index| index + 1)
                .unwrap_or(0);
            let line_end = text[*finish + end.len()..]
                .find('\n')
                .map(|index| *finish + end.len() + index + 1)
                .unwrap_or(text.len());
            if !text[line_start..*start].trim().is_empty()
                || !text[*finish + end.len()..line_end].trim().is_empty()
            {
                return Err(RuntimeError::conflict(
                    "Managed markers must occupy whole lines",
                ));
            }
            Ok(format!("{}{}", &text[..line_start], &text[line_end..]))
        }
        _ => Err(RuntimeError::conflict("Managed policy block is malformed")),
    }
}
pub fn merge_policy(existing: &str, release: &str) -> Result<String> {
    let base = remove_block(existing, AGENTS_BEGIN, AGENTS_END)?;
    let base = infra_core::domain::rstrip(&base);
    let payload = infra_core::domain::rstrip(release);
    Ok(format!(
        "{}{AGENTS_BEGIN}\n{payload}\n{AGENTS_END}\n",
        if base.is_empty() {
            String::new()
        } else {
            format!("{base}\n\n")
        }
    ))
}
pub fn merge_additive(existing: &str, release: &str) -> Result<String> {
    let had_block = existing.contains(ENTRIES_BEGIN) || existing.contains(ENTRIES_END);
    let base = remove_block(existing, ENTRIES_BEGIN, ENTRIES_END)?;
    let lines = base
        .lines()
        .map(|line| line.trim())
        .collect::<BTreeSet<_>>();
    let additions = release
        .lines()
        .filter(|line| !line.trim().is_empty() && !lines.contains(line.trim()))
        .collect::<Vec<_>>();
    let base = infra_core::domain::rstrip(&base);
    if additions.is_empty() {
        return Ok(if had_block {
            format!("{base}\n")
        } else {
            existing.into()
        });
    }
    Ok(format!(
        "{}{ENTRIES_BEGIN}\n{}\n{ENTRIES_END}\n",
        if base.is_empty() {
            String::new()
        } else {
            format!("{base}\n\n")
        },
        additions.join("\n")
    ))
}
impl Installer {
    pub fn new(source: &Path) -> Result<Self> {
        let source = std::fs::canonicalize(source)?;
        if !source.join("install/inventories").is_dir() {
            return Err(RuntimeError::not_found(
                "Accelerator inventories are absent",
            ));
        }
        let engine = std::env::current_exe()?;
        Ok(Self { source, engine })
    }
    pub fn with_engine(mut self, engine: &Path) -> Result<Self> {
        self.engine = std::fs::canonicalize(engine)?;
        let image = std::fs::symlink_metadata(&self.engine)?;
        if safe::linked(&image) || !image.is_file() || image.len() > FILE_LIMIT as u64 {
            return Err(RuntimeError::policy(
                "Native engine is not an ordinary bounded artifact",
            ));
        }
        Ok(self)
    }
    pub fn inventory(&self, edition: &str, tools: &[String]) -> Result<Inventory> {
        let edition = common::edition(edition)?;
        if edition == "Infrastructure-Creator" {
            return Err(RuntimeError::invalid(
                "Generator is not an installable edition",
            ));
        }
        if tools.is_empty()
            || tools
                .iter()
                .any(|tool| !["claude", "cursor", "codex"].contains(&tool.as_str()))
        {
            return Err(RuntimeError::invalid("Select at least one supported host"));
        }
        let data = common::read_json(
            &self
                .source
                .join(format!("install/inventories/{}.json", slug(edition))),
        )?;
        if ![2, 3].contains(&data["schema_version"].as_u64().unwrap_or(0))
            || data["edition"] != edition
        {
            return Err(RuntimeError::invalid(
                "Unsupported inventory schema or edition",
            ));
        }
        if data["schema_version"] == 3 && data["native_engine"] != native_engine_descriptor() {
            return Err(RuntimeError::invalid(
                "Native inventory engine descriptor differs from the current-executable platform contract",
            ));
        }
        let exclusions = data["excluded_tracked_paths"]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("Inventory exclusions are absent"))?;
        let mut excluded = BTreeSet::new();
        let mut previous_exclusion = None::<String>;
        for value in exclusions {
            let value = value
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Inventory exclusion must be text"))?;
            common::relative(value)?;
            if previous_exclusion
                .as_deref()
                .is_some_and(|previous| previous >= value)
                || !excluded.insert(value.to_string())
            {
                return Err(RuntimeError::invalid(
                    "Inventory exclusions must be sorted and unique",
                ));
            }
            previous_exclusion = Some(value.into());
        }
        let overrides = data["source_overrides"]
            .as_object()
            .ok_or_else(|| RuntimeError::invalid("Inventory overrides are absent"))?;
        let components = data["installed"]
            .as_object()
            .ok_or_else(|| RuntimeError::invalid("Inventory components are absent"))?;
        if components.keys().map(String::as_str).collect::<Vec<_>>()
            != vec!["shared", "claude", "cursor", "codex"]
        {
            return Err(RuntimeError::invalid(
                "Inventory component order is invalid",
            ));
        }
        let base = self.source.join(edition);
        let modes = FileModes::read(&base)?;
        let mut seen = BTreeSet::new();
        let mut files = Vec::new();
        for (component, paths) in components {
            let paths = paths
                .as_array()
                .ok_or_else(|| RuntimeError::invalid("Inventory paths must be arrays"))?;
            let mut prior = None::<String>;
            for value in paths {
                let path = value
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("Inventory path must be text"))?;
                common::relative(path)?;
                if excluded.contains(path)
                    || prior.as_deref().is_some_and(|previous| previous >= path)
                    || !seen.insert(path.to_string())
                {
                    return Err(RuntimeError::invalid(
                        "Inventory paths must be sorted and unique",
                    ));
                }
                prior = Some(path.into());
                if source_only(path)? {
                    return Err(RuntimeError::invalid(format!(
                        "Native inventory still installs source-only material: {path}"
                    )));
                }
                if component != "shared" && !tools.contains(component) {
                    continue;
                }
                if path == "memory-bank/bin/infra-engine"
                    || path == "memory-bank/bin/infra-engine.exe"
                {
                    continue;
                }
                let source = data["source_overrides"]
                    .get(path)
                    .and_then(Value::as_str)
                    .unwrap_or(path);
                let source = common::under(&base, source)?;
                let executable = modes
                    .executable(
                        source
                            .strip_prefix(&base)
                            .map_err(|_| RuntimeError::policy("Source override escaped edition"))?
                            .to_str()
                            .ok_or_else(|| RuntimeError::invalid("Source override is not UTF-8"))?,
                    )?
                    .unwrap_or(path.ends_with(".sh"));
                files.push(InstalledFile {
                    path: path.into(),
                    source,
                    executable,
                    component: component.clone(),
                });
            }
        }
        let mut last_override = None::<String>;
        for (destination, source) in overrides {
            let source = source
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Source override must be text"))?;
            common::relative(destination)?;
            common::relative(source)?;
            if !seen.contains(destination)
                || !excluded.contains(source)
                || destination == source
                || last_override
                    .as_deref()
                    .is_some_and(|last| last >= destination.as_str())
            {
                return Err(RuntimeError::invalid("Invalid source override ownership"));
            }
            last_override = Some(destination.clone());
        }
        let release = data["release"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RuntimeError::invalid("Inventory release is missing"))?
            .to_string();
        files.push(InstalledFile {
            path: ENGINE.into(),
            source: self.engine.clone(),
            executable: true,
            component: "shared".into(),
        });
        Ok(Inventory {
            edition: edition.into(),
            release,
            files,
        })
    }
    pub fn detect_edition(&self, target: &Path) -> Result<Option<String>> {
        if !target.is_dir() {
            return Ok(None);
        }
        let mut files = PublicationFiles::new(target)?;
        if let Some(bytes) = files.read(RECORD, 4 * 1024 * 1024)?
            && let Ok(record) = serde_json::from_slice::<Value>(&bytes)
            && let Some(edition) = record["edition"].as_str()
        {
            return common::edition(edition).map(|value| Some(value.into()));
        }
        let Some(bytes) = files.read("project-brain/config/runtime.json", 1024 * 1024)? else {
            return Ok(None);
        };
        let config: Value = serde_json::from_slice(&bytes)?;
        let framework = config["framework"].as_str().unwrap_or("");
        Ok(match framework {
            "laravel" => Some("Laravel".into()),
            "symfony" => Some("Symfony".into()),
            "php-core" | "generic" => Some("PHP Core".into()),
            "wordpress" => Some("Cms/wordpress".into()),
            _ => None,
        })
    }
    pub fn overlay(&self, target: &Path, edition: &str, tools: &[String]) -> Result<Value> {
        self.install(
            target,
            edition,
            tools,
            &InstallOptions {
                merge_existing: true,
                ..Default::default()
            },
        )
    }
    fn plan_effects(
        &self,
        target: &Path,
        edition: &str,
        tools: &[String],
        options: &InstallOptions,
    ) -> Result<PlannedEffects> {
        if options.overwrite && options.merge_existing {
            return Err(RuntimeError::invalid(
                "overwrite and merge_existing are mutually exclusive",
            ));
        }
        if options.update_tracked || options.rewire_codex || options.allow_downgrade {
            return Err(RuntimeError::invalid(
                "update_tracked, rewire_codex and allow_downgrade require sync",
            ));
        }
        let target = safe::absolute(target)?;
        let inventory = self.inventory(edition, tools)?;
        let capabilities = self.engine_capabilities()?;
        let mut custody = if target.exists() {
            Some(PublicationFiles::new(&target)?)
        } else {
            None
        };
        let mut effects = Vec::new();
        let mut kept = Vec::new();
        let mut collisions = Vec::new();
        for item in &inventory.files {
            let raw_source = safe::read_regular(&item.source)?;
            let source = native_release_payload(&item.path, &raw_source, &target)?;
            if item.path == ENGINE
                && capabilities["artifact_sha256"].as_str()
                    != Some(common::sha256(&source).as_str())
            {
                return Err(RuntimeError::conflict(
                    "Native engine changed after capability admission",
                ));
            }
            if source.len() > FILE_LIMIT {
                return Err(RuntimeError::invalid(
                    "Distribution source exceeds its limit",
                ));
            }
            let mut name = item.path.clone();
            let mut previous = if let Some(files) = &mut custody {
                files.read(&name, FILE_LIMIT)?
            } else {
                None
            };
            if previous.as_deref() == Some(source.as_slice()) {
                kept.push(json!({"path":name,"reason":"unchanged"}));
                continue;
            }
            if previous.is_some() && SEED.contains(&name.as_str()) {
                kept.push(json!({"path":name,"reason":"seeded project-owned state"}));
                continue;
            }
            if name == "README.md" && previous.is_some() && options.merge_existing {
                name = "ACCELERATOR.md".into();
                previous = if let Some(files) = &mut custody {
                    files.read(&name, FILE_LIMIT)?
                } else {
                    None
                };
                if previous.is_some() && previous.as_deref() != Some(source.as_slice()) {
                    collisions.push(json!({"path":name,"reason":"accelerator-readme-exists"}));
                    continue;
                }
            }
            let mut payload = source.clone();
            let mut action = if previous.is_some() {
                "overwrite"
            } else {
                "copy"
            }
            .to_string();
            if [".mcp.json", ".cursor/mcp.json", ".codex/config.toml"].contains(&name.as_str()) {
                payload = crate::mcp_config::merge(&name, previous.as_deref(), &source, &target)?;
                action = "merge".into();
            } else if options.merge_existing
                && previous.is_some()
                && ["AGENTS.md", ".claude/CLAUDE.md"].contains(&name.as_str())
            {
                payload = merge_policy(
                    std::str::from_utf8(previous.as_deref().unwrap_or_default()).map_err(|_| {
                        RuntimeError::invalid("Managed project policy is not UTF-8")
                    })?,
                    std::str::from_utf8(&source)
                        .map_err(|_| RuntimeError::invalid("Release policy is not UTF-8"))?,
                )?
                .into_bytes();
                action = "merge".into();
            } else if options.merge_existing
                && previous.is_some()
                && [".gitignore", ".gitattributes"].contains(&name.as_str())
            {
                payload = merge_additive(
                    std::str::from_utf8(previous.as_deref().unwrap_or_default()).map_err(|_| {
                        RuntimeError::invalid("Managed project entries are not UTF-8")
                    })?,
                    std::str::from_utf8(&source)
                        .map_err(|_| RuntimeError::invalid("Release entries are not UTF-8"))?,
                )?
                .into_bytes();
                action = "merge".into();
            } else if previous.is_some()
                && !options.overwrite
                && !(is_wiring(&name) && previous.as_deref() == Some(raw_source.as_slice()))
            {
                collisions.push(json!({"path":name,"reason":"existing-file"}));
                continue;
            }
            if previous.as_deref() == Some(payload.as_slice()) {
                kept.push(json!({"path":name,"reason":"unchanged"}));
                continue;
            }
            let original_mode = if let Some(files) = &mut custody {
                files.read_mode(&name)?.unwrap_or(0o644)
            } else {
                0o644
            };
            let mode = if action == "merge" {
                original_mode
            } else if item.executable {
                0o755
            } else {
                0o644
            };
            let record = source == payload;
            effects.push(Effect {
                path: name,
                before: previous,
                after: payload,
                mode,
                original_mode,
                action,
                record,
            });
        }

        Ok((target, inventory, capabilities, effects, kept, collisions))
    }
    pub fn install(
        &self,
        target: &Path,
        edition: &str,
        tools: &[String],
        options: &InstallOptions,
    ) -> Result<Value> {
        let plan = self.plan(target, edition, tools, options)?;
        if options.dry_run {
            return Ok(plan.report("planned", true));
        }
        self.apply_plan(&plan)
    }
    pub fn engine_capabilities(&self) -> Result<Value> {
        let executable = safe::read_regular(&self.engine)?;
        let pin = common::sha256(&executable);
        let output = crate::process::run(
            self.engine.as_os_str(),
            &["capabilities".into(), "--json".into()],
            &self.source,
            None,
            std::time::Duration::from_secs(5),
            256 * 1024,
        )?;
        if output.code != 0 || !output.stderr.is_empty() {
            return Err(RuntimeError::unavailable(
                "Native engine capability descriptor is unavailable",
            ));
        }
        let mut descriptor: Value = serde_json::from_slice(&output.stdout)?;
        if descriptor["engine"] != "infra-engine"
            || descriptor["version"].as_str().is_none_or(str::is_empty)
        {
            return Err(RuntimeError::invalid(
                "Selected executable is not an identified native engine",
            ));
        }
        let capabilities = descriptor["capabilities"]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("Native engine capabilities must be an array"))?;
        for required in ["memory", "brain", "bank", "hooks", "mcp", "install"] {
            if !capabilities
                .iter()
                .any(|value| value.as_str() == Some(required))
            {
                return Err(RuntimeError::unavailable(format!(
                    "Native engine lacks capability: {required}"
                )));
            }
        }
        if common::sha256(&safe::read_regular(&self.engine)?) != pin {
            return Err(RuntimeError::conflict(
                "Native engine changed during capability admission",
            ));
        }
        descriptor["artifact_sha256"] = pin.into();
        Ok(descriptor)
    }
    fn source_commit(&self) -> Result<Option<String>> {
        let output = common::git(&self.source, &["rev-parse", "HEAD"], None)?;
        if output.code == 0 {
            Ok(Some(common::text(output.stdout)?.trim().into()))
        } else {
            Ok(None)
        }
    }
    pub fn inventory_document(&self, edition: &str) -> Result<Value> {
        let edition = common::edition(edition)?;
        if edition == "Infrastructure-Creator" {
            return Err(RuntimeError::invalid("Generator is not installable"));
        }
        let base = self.source.join(edition);
        let prefix = format!("{edition}/");
        let mut paths = common::tracked(&self.source, &[edition.into()])?
            .into_iter()
            .collect::<BTreeSet<_>>();
        for name in self.native_payload_paths(edition)? {
            paths.insert(format!("{prefix}{name}"));
        }
        let mut components = BTreeMap::<String, Vec<String>>::new();
        for name in ["shared", "claude", "cursor", "codex"] {
            components.insert(name.into(), Vec::new());
        }
        let mut excluded = Vec::new();
        let mut installed = BTreeSet::new();
        for path in paths {
            let relative = path
                .strip_prefix(&prefix)
                .ok_or_else(|| RuntimeError::invalid("Tracked inventory path escaped edition"))?;
            if source_only(relative)? {
                excluded.push(relative.into());
            } else {
                components
                    .get_mut(component(relative))
                    .ok_or_else(|| RuntimeError::invalid("Unknown inventory component"))?
                    .push(relative.into());
                installed.insert(relative.to_string());
            }
        }
        let mut object = serde_json::Map::new();
        for name in ["shared", "claude", "cursor", "codex"] {
            let mut values = components.remove(name).unwrap_or_default();
            values.sort();
            object.insert(name.into(), json!(values));
        }
        let overrides = OVERRIDES
            .iter()
            .filter(|(destination, source)| {
                installed.contains(*destination)
                    && excluded.iter().any(|path: &String| path == source)
            })
            .map(|(destination, source)| ((*destination).to_string(), json!(source)))
            .collect::<serde_json::Map<_, _>>();
        excluded.sort();
        let release = safe::read_text(&base.join("VERSION"))?.trim().to_string();
        let value = json!({"schema_version":3,"inventory_version":3,"edition":edition,"release":release,"scope":"closed tracked-file payload plus explicit native source additions and current engine artifact","installed":object,"excluded_tracked_paths":excluded,"source_overrides":overrides,"native_engine":native_engine_descriptor()});

        Ok(value)
    }
    pub fn write_inventory(&self, edition: &str, write: bool) -> Result<Value> {
        self.write_inventory_to(edition, write, None)
    }
    pub fn write_inventory_to(
        &self,
        edition: &str,
        write: bool,
        destination: Option<&Path>,
    ) -> Result<Value> {
        if destination.is_some() && !write {
            return Err(RuntimeError::invalid(
                "inventory_out requires inventory-write",
            ));
        }
        let value = self.inventory_document(edition)?;
        let name = format!("{}.json", slug(edition));
        let directory = if let Some(destination) = destination {
            safe::absolute(destination)?
        } else {
            self.source.join("install/inventories")
        };
        if write {
            safe::ensure_directory(&directory, false)?;
            let mut files = PublicationFiles::new(&directory)?;
            let previous = files.read(&name, 8 * 1024 * 1024)?;
            let mode = files.read_mode(&name)?.unwrap_or(0o644);
            files.write_expected_mode(
                &name,
                &common::json_bytes(&value)?,
                previous.as_deref(),
                mode,
                8 * 1024 * 1024,
            )?;
            Ok(json!({"edition":edition,"written":directory.join(name)}))
        } else {
            let stored = common::read_json(&directory.join(name))?;
            Ok(json!({"edition":edition,"passed":stored==value,"expected":value}))
        }
    }
}

impl Installer {
    fn release_history(&self, edition: &str) -> Result<BTreeMap<String, BTreeSet<String>>> {
        if !self
            .source
            .ancestors()
            .any(|path| path.join(".git").exists())
        {
            let path = self.source.join("install/release-history.json");
            if !path.is_file() {
                return Ok(BTreeMap::new());
            }
            let data = common::read_json(&path)?;
            let section = data
                .get(edition)
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    RuntimeError::invalid("Packaged release history has no selected edition")
                })?;
            let mut result = BTreeMap::new();
            for (name, values) in section {
                common::relative(name)?;
                let mut hashes = BTreeSet::new();
                for value in values
                    .as_array()
                    .ok_or_else(|| RuntimeError::invalid("Release history hashes must be arrays"))?
                {
                    let hash = value
                        .as_str()
                        .filter(|value| {
                            value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                        .ok_or_else(|| RuntimeError::invalid("Malformed release history hash"))?;
                    hashes.insert(hash.into());
                }
                result.insert(name.clone(), hashes);
            }
            return Ok(result);
        }
        let output = common::git(
            &self.source,
            &[
                "log",
                "--all",
                "--format=",
                "--raw",
                "--no-abbrev",
                "--",
                edition,
            ],
            None,
        )?;
        if output.code != 0 {
            return Err(RuntimeError::unavailable(
                "Released file ownership history is unavailable",
            ));
        }
        let text = common::text(output.stdout)?;
        let prefix = format!("{edition}/");
        let mut result = BTreeMap::<String, BTreeSet<String>>::new();
        for line in text.lines().filter(|line| line.starts_with(':')) {
            let Some((meta, name)) = line.split_once('\t') else {
                continue;
            };
            let fields = meta.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 5 {
                continue;
            }
            let Some(name) = name.strip_prefix(&prefix) else {
                continue;
            };
            common::relative(name)?;
            for digest in &fields[2..4] {
                if digest.len() == 40 && !digest.bytes().all(|byte| byte == b'0') {
                    result
                        .entry(name.into())
                        .or_default()
                        .insert((*digest).into());
                }
            }
        }
        Ok(result)
    }
    fn downgrade_problem(&self, record: &Value) -> Result<Option<String>> {
        let Some(old) = record["source_commit"]
            .as_str()
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        if !old.bytes().all(|byte| byte.is_ascii_hexdigit()) || ![40, 64].contains(&old.len()) {
            return Err(RuntimeError::invalid(
                "Installed source commit identity is malformed",
            ));
        }
        let current = self.source_commit()?.ok_or_else(|| {
            RuntimeError::unavailable("Source release has no Git identity for downgrade protection")
        })?;
        if old == current {
            return Ok(None);
        }
        let output = common::git(
            &self.source,
            &["merge-base", "--is-ancestor", old, &current],
            None,
        )?;
        Ok(if output.code == 0 {
            None
        } else {
            Some("Installed release is newer than or outside this source history; explicit downgrade authorization is required".into())
        })
    }
    pub fn sync(&self, target: &Path, options: &InstallOptions) -> Result<Value> {
        if options.overwrite && options.merge_existing {
            return Err(RuntimeError::invalid(
                "overwrite and merge_existing are mutually exclusive",
            ));
        }
        let target = std::fs::canonicalize(target)?;
        let Some(edition) = self.detect_edition(&target)? else {
            return Err(RuntimeError::not_found(
                "Target is not an identified accelerator installation",
            ));
        };
        let mut custody = PublicationFiles::new(&target)?;
        if custody.read(ENGINE, FILE_LIMIT)?.is_none()
            && custody
                .read("memory-bank/scripts/context.py", FILE_LIMIT)?
                .is_none()
        {
            return Err(RuntimeError::not_found(
                "Target has no owned native or historical accelerator runtime marker",
            ));
        }
        let recorded = custody.read(RECORD, 4 * 1024 * 1024)?;
        let record = recorded
            .as_ref()
            .map(|bytes| serde_json::from_slice::<Value>(bytes))
            .transpose()?
            .unwrap_or_else(|| json!({}));
        if !record.is_object() {
            return Err(RuntimeError::invalid(
                "Install custody record must be an object",
            ));
        }
        if !options.allow_downgrade
            && let Some(problem) = self.downgrade_problem(&record)?
        {
            return Err(RuntimeError::conflict(problem));
        }
        let mut hosts = Vec::new();
        for (host, marker) in [
            ("claude", ".claude/settings.json"),
            ("cursor", ".cursor/hooks.json"),
            ("codex", ".codex/hooks.json"),
        ] {
            if custody.read(marker, 8 * 1024 * 1024)?.is_some() {
                hosts.push(host.to_string());
            }
        }
        if hosts.is_empty() {
            return Err(RuntimeError::not_found(
                "Installed accelerator has no selected host wiring",
            ));
        }
        let inventory = self.inventory(&edition, &hosts)?;
        let capabilities = self.engine_capabilities()?;
        let tracked = if options.update_tracked {
            BTreeSet::new()
        } else {
            let probe = common::git(&target, &["rev-parse", "--is-inside-work-tree"], None)?;
            if probe.code == 0 {
                common::tracked(&target, &[".".into()])?
                    .into_iter()
                    .collect()
            } else {
                BTreeSet::new()
            }
        };
        let known = record["files"].as_object().cloned().unwrap_or_default();
        let history = self.release_history(&edition)?;
        let stamp = uuid::Uuid::new_v4().simple().to_string();
        let mut effects = Vec::<Effect>::new();
        let mut removals = Vec::<(String, Vec<u8>, u32)>::new();
        let mut backups = Vec::<Effect>::new();
        let mut kept = Vec::<Value>::new();
        let mut active_paths = BTreeSet::new();
        let mut runtime_held = false;
        let mut held_wiring = BTreeSet::new();
        for item in &inventory.files {
            let mut name = item.path.clone();
            if name == "README.md" {
                name = "ACCELERATOR.md".into();
            }
            active_paths.insert(name.clone());
            let before = custody.read(&name, FILE_LIMIT)?;
            let runtime = is_runtime(&name);
            let wiring = hook_wiring(&name);
            if before.is_some() && SEED.contains(&name.as_str()) {
                kept.push(json!({"path":name,"reason":"seeded project-owned state"}));
                continue;
            }
            if tracked.contains(&name) {
                if runtime {
                    runtime_held = true;
                }
                if is_wiring(&name) {
                    held_wiring.insert(name.clone());
                } else if name.ends_with(".sh")
                    && let Some(wiring) = wiring
                {
                    held_wiring.insert(wiring.into());
                }
                kept.push(json!({"path":name,"reason":"tracked by project; explicit update_tracked is required"}));
                continue;
            }
            let raw_source = safe::read_regular(&item.source)?;
            let source = native_release_payload(&item.path, &raw_source, &target)?;
            if item.path == ENGINE
                && capabilities["artifact_sha256"].as_str()
                    != Some(common::sha256(&source).as_str())
            {
                return Err(RuntimeError::conflict(
                    "Native engine changed after capability admission",
                ));
            }
            if before.as_deref() == Some(source.as_slice()) {
                continue;
            }
            if name == ".codex/hooks.json" && before.is_some() && !options.rewire_codex {
                held_wiring.insert(name.clone());
                kept.push(json!({"path":name,"reason":"trust-bound wiring needs explicit rewire_codex; trust records are never written"}));
                continue;
            }
            let pristine = before.as_ref().is_some_and(|bytes| {
                identity_matches(known.get(&name).and_then(Value::as_str), bytes)
                    || history
                        .get(&item.path)
                        .is_some_and(|values| values.contains(&legacy_blob(bytes)))
            });
            let mut payload = source.clone();
            let mut action = if before.is_none() { "added" } else { "updated" }.to_string();
            if [".mcp.json", ".cursor/mcp.json", ".codex/config.toml"].contains(&name.as_str()) {
                payload = crate::mcp_config::merge(&name, before.as_deref(), &source, &target)?;
                action = "memory server updated".into();
            } else if before.is_some()
                && ["AGENTS.md", ".claude/CLAUDE.md"].contains(&name.as_str())
            {
                let previous = std::str::from_utf8(before.as_deref().unwrap_or_default())
                    .map_err(|_| RuntimeError::invalid("Project policy is not UTF-8"))?;
                let release = std::str::from_utf8(&source)
                    .map_err(|_| RuntimeError::invalid("Release policy is not UTF-8"))?;
                if previous.contains(AGENTS_BEGIN) || name == ".claude/CLAUDE.md" {
                    payload = merge_policy(previous, release)?.into_bytes();
                    action = "managed policy updated".into();
                } else if !pristine {
                    let first = release.lines().next().unwrap_or("");
                    if first.is_empty() || !previous.starts_with(first) {
                        kept.push(json!({"path":name,"reason":"project-owned policy"}));
                        continue;
                    }
                    action = "updated over local edit".into();
                }
            } else if before.is_some() && [".gitignore", ".gitattributes"].contains(&name.as_str())
            {
                let previous = std::str::from_utf8(before.as_deref().unwrap_or_default())
                    .map_err(|_| RuntimeError::invalid("Project entries are not UTF-8"))?;
                if !previous.contains(ENTRIES_BEGIN) {
                    kept.push(
                        json!({"path":name,"reason":"project-owned entries without managed block"}),
                    );
                    continue;
                }
                payload = merge_additive(
                    previous,
                    std::str::from_utf8(&source)
                        .map_err(|_| RuntimeError::invalid("Release entries are not UTF-8"))?,
                )?
                .into_bytes();
                action = "managed entries updated".into();
            } else if before.is_some() && !pristine && !runtime {
                if is_wiring(&name) {
                    held_wiring.insert(name.clone());
                } else if name.ends_with(".sh")
                    && let Some(wiring) = wiring
                {
                    held_wiring.insert(wiring.into());
                }
                kept.push(json!({"path":name,"reason":"edited in project"}));
                continue;
            }
            if before.as_deref() == Some(payload.as_slice()) {
                continue;
            }
            let original_mode = custody.read_mode(&name)?.unwrap_or(0o644);
            if let Some(bytes) = &before
                && (!pristine && runtime || action == "updated over local edit")
            {
                let backup = format!("memory-bank/local/accelerator-sync/{stamp}/{name}");
                if custody.read(&backup, FILE_LIMIT)?.is_some() {
                    return Err(RuntimeError::conflict("Sync backup unexpectedly exists"));
                }
                backups.push(Effect {
                    path: backup,
                    before: None,
                    after: bytes.clone(),
                    mode: original_mode,
                    original_mode,
                    action: "backup".into(),
                    record: false,
                });
            }
            let mode = if action.contains("managed") || action == "memory server updated" {
                original_mode
            } else if item.executable {
                0o755
            } else {
                0o644
            };
            effects.push(Effect {
                path: name,
                before,
                after: payload.clone(),
                mode,
                original_mode,
                action,
                record: payload == source,
            });
        }
        // Retire only previous runtime files whose ownership can be established.
        let legacy_candidates = known
            .keys()
            .chain(history.keys())
            .filter(|name| is_legacy_runtime(name) && !active_paths.contains(*name))
            .cloned()
            .collect::<BTreeSet<_>>();
        for name in &legacy_candidates {
            let Some(bytes) = custody.read(name, FILE_LIMIT)? else {
                continue;
            };
            if tracked.contains(name) {
                runtime_held = true;
                kept.push(json!({"path":name,"reason":"tracked legacy runtime cannot be retired without update_tracked"}));
                continue;
            }
            let mode = custody.read_mode(name)?.unwrap_or(0o644);
            if !identity_matches(known.get(name).and_then(Value::as_str), &bytes) {
                let backup = format!("memory-bank/local/accelerator-sync/{stamp}/{name}");
                backups.push(Effect {
                    path: backup,
                    before: None,
                    after: bytes.clone(),
                    mode,
                    original_mode: mode,
                    action: "backup legacy edit".into(),
                    record: false,
                });
            }
            removals.push((name.clone(), bytes, mode));
        }
        if !held_wiring.is_empty() {
            let legacy_present = !removals.is_empty();
            if legacy_present {
                runtime_held = true;
            }
            effects.retain(|effect| {
                let held = held_wiring.contains(&effect.path)
                    || hook_wiring(&effect.path).is_some_and(|wiring| held_wiring.contains(wiring));
                if held {
                    kept.push(
                        json!({"path":effect.path,"reason":"held back by unchanged host wiring"}),
                    );
                }
                !held
            });
        }
        if runtime_held {
            for effect in &effects {
                kept.push(json!({"path":effect.path,"reason":"held back because runtime migration is incomplete"}));
            }
            effects.clear();
            removals.clear();
            backups.clear();
        }
        let changes = effects
            .iter()
            .map(|effect| json!({"path":effect.path,"action":effect.action}))
            .chain(
                removals
                    .iter()
                    .map(|(path, _, _)| json!({"path":path,"action":"retire legacy runtime"})),
            )
            .collect::<Vec<_>>();
        if options.dry_run {
            return Ok(
                json!({"status":"planned","dry_run":true,"edition":edition,"release":inventory.release,"changed":changes,"kept":kept,"partial":runtime_held||!held_wiring.is_empty(),"requires_review":!held_wiring.is_empty(),"degraded":runtime_held||!held_wiring.is_empty(),"held_wiring":held_wiring}),
            );
        }
        let mut applied = Vec::<Effect>::new();
        let mut removed = Vec::<(String, Vec<u8>, u32)>::new();
        let outcome = (|| -> Result<()> {
            // Durable backups precede every replacement. All effects are expected-image CAS.
            for effect in &backups {
                custody.write_expected_mode(
                    &effect.path,
                    &effect.after,
                    None,
                    effect.mode,
                    FILE_LIMIT,
                )?;
                applied.push(effect.clone());
            }
            effects.sort_by_key(|effect| {
                if effect.path == ENGINE {
                    0
                } else if is_wiring(&effect.path) {
                    3
                } else if hook_wiring(&effect.path).is_some() {
                    1
                } else {
                    2
                }
            });
            for effect in &effects {
                custody.write_expected_mode_checked(
                    &effect.path,
                    &effect.after,
                    effect.before.as_deref(),
                    effect.before.as_ref().map(|_| effect.original_mode),
                    effect.mode,
                    FILE_LIMIT,
                )?;
                applied.push(effect.clone());
            }
            for (path, bytes, mode) in &removals {
                custody.unlink_expected_limit(path, bytes, FILE_LIMIT)?;
                removed.push((path.clone(), bytes.clone(), *mode));
            }
            let mut next = known.clone();
            for effect in &effects {
                if effect.record {
                    next.insert(effect.path.clone(), common::sha256(&effect.after).into());
                } else {
                    next.remove(&effect.path);
                }
            }
            for (path, _, _) in &removed {
                next.remove(path);
            }
            let mut value = record.clone();
            value["schema"] = 2.into();
            value["edition"] = edition.clone().into();
            value["release"] = inventory.release.clone().into();
            value["source_commit"] = json!(self.source_commit()?);
            value["synced_at"] = infra_core::domain::utc_now().into();
            value["files"] = Value::Object(next);
            value["engine_sha256"] = capabilities["artifact_sha256"].clone();
            value["engine_capabilities"] = capabilities.clone();
            if !effects.is_empty() || !removals.is_empty() {
                custody.write_expected_limit(
                    RECORD,
                    &common::json_bytes(&value)?,
                    recorded.as_deref(),
                    4 * 1024 * 1024,
                )?;
            }
            Ok(())
        })();
        if let Err(original) = outcome {
            let mut errors = Vec::new();
            for (path, bytes, mode) in removed.iter().rev() {
                if let Err(error) =
                    custody.write_expected_mode(path, bytes, None, *mode, FILE_LIMIT)
                {
                    errors.push(format!("{path}: {error}"));
                }
            }
            for effect in applied.iter().rev().filter(|effect| {
                !effect
                    .path
                    .starts_with("memory-bank/local/accelerator-sync/")
            }) {
                let result = if let Some(before) = &effect.before {
                    custody.write_expected_mode(
                        &effect.path,
                        before,
                        Some(&effect.after),
                        effect.original_mode,
                        FILE_LIMIT,
                    )
                } else {
                    custody.unlink_expected_limit(&effect.path, &effect.after, FILE_LIMIT)
                };
                if let Err(error) = result {
                    errors.push(format!("{}: {error}", effect.path));
                }
            }
            if errors.is_empty() {
                return Err(original);
            }
            return Err(RuntimeError::conflict(format!(
                "Sync failed ({original}); rollback requires attention: {}",
                errors.join("; ")
            )));
        }
        Ok(
            json!({"status":if runtime_held||!held_wiring.is_empty(){"partial"}else{"complete"},"edition":edition,"release":inventory.release,"changed":changes,"kept":kept,"backups":backups.iter().map(|effect|effect.path.clone()).collect::<Vec<_>>(),"dry_run":false,"partial":runtime_held||!held_wiring.is_empty(),"requires_review":!held_wiring.is_empty(),"degraded":runtime_held||!held_wiring.is_empty(),"held_wiring":held_wiring}),
        )
    }
}
fn is_legacy_runtime(path: &str) -> bool {
    (path.starts_with("memory-bank/scripts/") || path.starts_with("project-brain/scripts/"))
        && (path.ends_with(".py") || path.ends_with(".sh"))
}
fn is_runtime(path: &str) -> bool {
    path == ENGINE
        || path.starts_with("memory-bank/config/")
        || path.starts_with("project-brain/schemas/")
        || is_legacy_runtime(path)
}
fn is_wiring(path: &str) -> bool {
    [
        ".claude/settings.json",
        ".cursor/hooks.json",
        ".codex/hooks.json",
    ]
    .contains(&path)
}
fn hook_wiring(path: &str) -> Option<&'static str> {
    if path.starts_with(".claude/hooks/") {
        Some(".claude/settings.json")
    } else if path.starts_with(".cursor/hooks/") {
        Some(".cursor/hooks.json")
    } else if path.starts_with(".codex/hooks/") {
        Some(".codex/hooks.json")
    } else {
        None
    }
}

pub type FileIdentity = (u64, u64, u32, u64, u64, i128, i128);
pub type DirectoryIdentity = (u64, u64, u32);
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceImage {
    pub path: PathBuf,
    pub parent_identity: DirectoryIdentity,
    pub identity: FileIdentity,
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetImage {
    pub identity: FileIdentity,
    pub sha256: String,
    pub bytes: u64,
    pub mode: u32,
}
struct PublishedInstallFile {
    path: String,
    previous: Option<Vec<u8>>,
    previous_mode: Option<u32>,
    content: Vec<u8>,
    published: PublishedImage,
}

fn retain_post_rename_errno(selected: &mut Option<Option<i32>>, error: &RuntimeError) {
    if selected.is_none() && error.code.as_deref() == Some(POST_RENAME_IDENTITY_UNAVAILABLE) {
        // Outer Some records the typed failure even if its OS errno is absent.
        // The original error wins, otherwise use the first compensation failure.
        *selected = Some(error.os_code);
    }
}
fn rollback_attention_error(
    original: &RuntimeError,
    errors: &[String],
    post_rename_errno: Option<Option<i32>>,
) -> RuntimeError {
    let message = format!(
        "Reviewed install failed ({original}); rollback requires attention: {}",
        errors.join("; ")
    );
    if let Some(os_code) = post_rename_errno {
        let mut error = RuntimeError::coded(
            infra_core::ErrorKind::Conflict,
            POST_RENAME_IDENTITY_UNAVAILABLE,
            message,
        );
        error.os_code = os_code;
        error
    } else {
        RuntimeError::conflict(message)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PlannedPayload {
    Inline { content: Vec<u8>, sha256: String },
    Source { image: SourceImage },
}
// Internally tagged derived enums buffer values in Serde ContentDeserializer,
// which cannot deserialize i128. Decode the same strict wire fields directly.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlannedPayloadWire {
    kind: String,
    #[serde(default, deserialize_with = "present_payload_field")]
    content: Option<Vec<u8>>,
    #[serde(default, deserialize_with = "present_payload_field")]
    sha256: Option<String>,
    #[serde(default, deserialize_with = "present_payload_field")]
    image: Option<SourceImage>,
}
fn present_payload_field<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(T::deserialize(deserializer)?))
}
impl<'de> Deserialize<'de> for PlannedPayload {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PlannedPayloadWire::deserialize(deserializer)?;
        match wire.kind.as_str() {
            "inline" => {
                if wire.image.is_some() {
                    return Err(<D::Error as serde::de::Error>::unknown_field(
                        "image",
                        &["kind", "content", "sha256"],
                    ));
                }
                Ok(Self::Inline {
                    content: wire
                        .content
                        .ok_or_else(|| <D::Error as serde::de::Error>::missing_field("content"))?,
                    sha256: wire
                        .sha256
                        .ok_or_else(|| <D::Error as serde::de::Error>::missing_field("sha256"))?,
                })
            }
            "source" => {
                if wire.content.is_some() {
                    return Err(<D::Error as serde::de::Error>::unknown_field(
                        "content",
                        &["kind", "image"],
                    ));
                }
                if wire.sha256.is_some() {
                    return Err(<D::Error as serde::de::Error>::unknown_field(
                        "sha256",
                        &["kind", "image"],
                    ));
                }
                Ok(Self::Source {
                    image: wire
                        .image
                        .ok_or_else(|| <D::Error as serde::de::Error>::missing_field("image"))?,
                })
            }
            _ => Err(<D::Error as serde::de::Error>::unknown_variant(
                &wire.kind,
                &["inline", "source"],
            )),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedFile {
    pub path: String,
    pub before: Option<TargetImage>,
    pub payload: PlannedPayload,
    pub mode: u32,
    pub action: String,
    pub record: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetAnchor {
    pub path: PathBuf,
    pub identity: DirectoryIdentity,
}
fn anchor_relative(target: &Path, anchor: &Path) -> Result<String> {
    let suffix = target
        .strip_prefix(anchor)
        .map_err(|_| RuntimeError::conflict("Install anchor is outside the target ancestry"))?;
    let mut parts = Vec::new();
    for part in suffix.components() {
        let std::path::Component::Normal(name) = part else {
            return Err(RuntimeError::invalid(
                "Install target ancestry is not ordinary",
            ));
        };
        parts.push(
            name.to_str()
                .ok_or_else(|| RuntimeError::invalid("Install target ancestry is not UTF-8"))?,
        );
    }
    if parts.is_empty() {
        return Err(RuntimeError::conflict(
            "Missing target anchor must be a strict ancestor",
        ));
    }
    let relative = parts.join("/");
    common::relative(&relative)?;
    Ok(relative)
}
fn capture_target_anchor(target: &Path) -> Result<Option<TargetAnchor>> {
    match std::fs::symlink_metadata(target) {
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut current = target
        .parent()
        .ok_or_else(|| RuntimeError::invalid("Missing target has no parent"))?;
    loop {
        match std::fs::symlink_metadata(current) {
            Ok(_) => {
                let files = PublicationFiles::new(current)?;
                files.check()?;
                return Ok(Some(TargetAnchor {
                    path: current.into(),
                    identity: files.root_identity(),
                }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                current = current.parent().ok_or_else(|| {
                    RuntimeError::invalid("Missing target has no existing ancestor")
                })?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}
struct NewTargetDirectories {
    path: PathBuf,
    anchor: PublicationFiles,
    outer: Vec<(String, DirectoryIdentity)>,
    inner: Vec<(String, DirectoryIdentity)>,
}
impl NewTargetDirectories {
    fn open(expected: &TargetAnchor) -> Result<Self> {
        let anchor = PublicationFiles::new(&expected.path)?;
        if anchor.root_identity() != expected.identity {
            return Err(RuntimeError::conflict(
                "Install target ancestor changed after preview",
            ));
        }
        anchor.check()?;
        Ok(Self {
            path: expected.path.clone(),
            anchor,
            outer: vec![],
            inner: vec![],
        })
    }
    fn cleanup_outer(&mut self) -> Result<()> {
        let mut failures = Vec::new();
        for (name, identity) in self.outer.iter().rev() {
            match self.anchor.remove_empty_directory_expected(name, *identity) {
                Ok(true) => {}
                Ok(false) => failures.push(format!(
                    "{name}: created directory is not empty or is absent"
                )),
                Err(error) => failures.push(format!("{name}: {error}")),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(RuntimeError::conflict(format!(
                "Created install directories require attention: {}",
                failures.join("; ")
            )))
        }
    }
    fn create_target(&mut self, target: &Path) -> Result<PublicationFiles> {
        let outcome = (|| -> Result<PublicationFiles> {
            let relative = anchor_relative(target, &self.path)?;
            let parts = relative.split('/').collect::<Vec<_>>();
            let mut identity = None;
            for count in 1..=parts.len() {
                let name = parts[..count].join("/");
                let created = self.anchor.create_directory_exclusive(&name, 0o777)?;
                self.outer.push((name, created));
                identity = Some(created);
            }
            let files = PublicationFiles::new(target)?;
            if Some(files.root_identity()) != identity {
                return Err(RuntimeError::conflict("Created install target changed"));
            }
            self.anchor.check()?;
            Ok(files)
        })();
        match outcome {
            Ok(files) => Ok(files),
            Err(original) => match self.cleanup_outer() {
                Ok(()) => Err(original),
                Err(cleanup) => Err(RuntimeError::conflict(format!("{original}; {cleanup}"))),
            },
        }
    }
    fn create_payload_parents(
        &mut self,
        target: &mut PublicationFiles,
        planned: &[PlannedFile],
    ) -> Result<()> {
        let mut parents = BTreeSet::new();
        for path in planned
            .iter()
            .map(|file| file.path.as_str())
            .chain([RECORD])
        {
            let parts = path.split('/').collect::<Vec<_>>();
            for count in 1..parts.len() {
                parents.insert(parts[..count].join("/"));
            }
        }
        // Lexical order places each parent before all of its descendants.
        for name in parents {
            self.anchor.check()?;
            let identity = target.create_directory_exclusive(&name, 0o700)?;
            self.inner.push((name, identity));
        }
        Ok(())
    }
    fn rollback(&mut self, target: &mut PublicationFiles) -> Result<()> {
        let mut failures = Vec::new();
        for (name, identity) in self.inner.iter().rev() {
            match target.remove_empty_directory_expected(name, *identity) {
                Ok(true) => {}
                Ok(false) => failures.push(format!(
                    "{name}: created directory is not empty or is absent"
                )),
                Err(error) => failures.push(format!("{name}: {error}")),
            }
        }
        if let Err(error) = self.cleanup_outer() {
            failures.push(error.to_string());
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(RuntimeError::conflict(format!(
                "Created install directories require attention: {}",
                failures.join("; ")
            )))
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallPlan {
    pub schema_version: u32,
    pub nonce: String,
    pub source: PathBuf,
    pub source_identity: DirectoryIdentity,
    pub source_commit: Option<String>,
    pub target: PathBuf,
    pub target_identity: Option<DirectoryIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_anchor: Option<TargetAnchor>,
    pub directories: BTreeMap<String, DirectoryIdentity>,
    pub edition: String,
    pub release: String,
    pub tools: Vec<String>,
    pub options: InstallOptions,
    pub engine: Value,
    pub sources: Vec<SourceImage>,
    pub control: Option<TargetImage>,
    pub files: Vec<PlannedFile>,
    pub kept: Vec<Value>,
    pub collisions: Vec<Value>,
    pub sha256: String,
}
impl InstallPlan {
    fn digest(&self) -> Result<String> {
        let mut copy = self.clone();
        copy.sha256.clear();
        Ok(common::sha256(
            infra_core::domain::json::canonical(&serde_json::to_value(copy)?)?.as_bytes(),
        ))
    }
    pub fn verify(&self) -> Result<()> {
        if self.schema_version != 1
            || self.nonce.is_empty()
            || !self.source.is_absolute()
            || !self.target.is_absolute()
            || self.sha256 != self.digest()?
        {
            return Err(RuntimeError::conflict("Install preview seal differs"));
        }
        match (&self.target_identity, &self.target_anchor) {
            (None, Some(anchor)) if anchor.path.is_absolute() => {
                anchor_relative(&self.target, &anchor.path)?;
            }
            (Some(_), None) => {}
            _ => {
                return Err(RuntimeError::conflict(
                    "Install preview has no valid target ancestor",
                ));
            }
        }
        let mut names = BTreeSet::new();
        for file in &self.files {
            common::relative(&file.path)?;
            if !names.insert(&file.path) || file.mode & !0o777 != 0 {
                return Err(RuntimeError::invalid(
                    "Install preview has duplicate paths or invalid modes",
                ));
            }
        }
        Ok(())
    }
    pub fn report(&self, status: &str, dry_run: bool) -> Value {
        json!({"status":if self.collisions.is_empty(){status}else{"refused"},"edition":self.edition,"release":self.release,"changed":self.files.iter().map(|file|json!({"path":file.path,"action":file.action})).collect::<Vec<_>>(),"kept":self.kept,"collisions":self.collisions,"dry_run":dry_run,"plan_sha256":self.sha256,"nonce":self.nonce})
    }
}
fn source_image(path: &Path) -> Result<(SourceImage, Vec<u8>)> {
    let path = safe::absolute(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| RuntimeError::invalid("Release source has no parent"))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| RuntimeError::invalid("Release source name is not UTF-8"))?;
    let mut files = PublicationFiles::new(parent)?;
    let (content, identity) = files
        .capture_source(name, FILE_LIMIT)?
        .ok_or_else(|| RuntimeError::not_found("Reviewed release source disappeared"))?;
    let image = SourceImage {
        path: path.clone(),
        parent_identity: files.root_identity(),
        identity,
        sha256: common::sha256(&content),
        bytes: content.len() as u64,
    };
    Ok((image, content))
}
fn target_image(
    files: &mut PublicationFiles,
    name: &str,
    expected: Option<&[u8]>,
) -> Result<Option<TargetImage>> {
    let bytes = files.read(name, FILE_LIMIT)?;
    if bytes.as_deref() != expected {
        return Err(RuntimeError::conflict(
            "Target changed while preparing reviewed install bytes",
        ));
    }
    match bytes {
        None => Ok(None),
        Some(bytes) => {
            let identity = files
                .image_identity(name)?
                .ok_or_else(|| RuntimeError::conflict("Preview target image disappeared"))?;
            let mode = files
                .read_mode(name)?
                .ok_or_else(|| RuntimeError::conflict("Preview target mode disappeared"))?;
            Ok(Some(TargetImage {
                identity,
                sha256: common::sha256(&bytes),
                bytes: bytes.len() as u64,
                mode,
            }))
        }
    }
}
fn verified_source(expected: &SourceImage) -> Result<Vec<u8>> {
    let (actual, bytes) = source_image(&expected.path)?;
    if actual.parent_identity != expected.parent_identity
        || actual.identity != expected.identity
        || actual.sha256 != expected.sha256
        || actual.bytes != expected.bytes
    {
        return Err(RuntimeError::conflict(
            "Reviewed release source identity or bytes changed",
        ));
    }
    Ok(bytes)
}
fn verified_target(
    files: &mut PublicationFiles,
    name: &str,
    expected: Option<&TargetImage>,
) -> Result<Option<Vec<u8>>> {
    let bytes = files.read(name, FILE_LIMIT)?;
    let identity = files.image_identity(name)?;
    let mode = files.read_mode(name)?;
    match (expected, bytes) {
        (None, None) if identity.is_none() && mode.is_none() => Ok(None),
        (Some(expected), Some(bytes))
            if identity == Some(expected.identity)
                && mode == Some(expected.mode)
                && bytes.len() as u64 == expected.bytes
                && common::sha256(&bytes) == expected.sha256 =>
        {
            Ok(Some(bytes))
        }
        _ => Err(RuntimeError::conflict(format!(
            "Reviewed target baseline changed: {name}"
        ))),
    }
}
impl Installer {
    pub fn plan(
        &self,
        target: &Path,
        edition: &str,
        tools: &[String],
        options: &InstallOptions,
    ) -> Result<InstallPlan> {
        let target = safe::absolute(target)?;
        let target_anchor = capture_target_anchor(&target)?;
        // Freeze raw release inputs before deriving target-specific hook commands.
        // A source change during planning must not bless an earlier derived image.
        let initial = self.inventory(edition, tools)?;
        let mut snapshots = BTreeMap::<PathBuf, SourceImage>::new();
        for item in &initial.files {
            if !snapshots.contains_key(&item.source) {
                let (image, _) = source_image(&item.source)?;
                snapshots.insert(item.source.clone(), image);
            }
        }
        let (target, inventory, capabilities, effects, kept, collisions) =
            self.plan_effects(&target, edition, tools, options)?;
        if initial.files.len() != inventory.files.len()
            || initial.files.iter().zip(&inventory.files).any(|(a, b)| {
                a.path != b.path
                    || a.source != b.source
                    || a.executable != b.executable
                    || a.component != b.component
            })
        {
            return Err(RuntimeError::conflict(
                "Release inventory changed during reviewed install planning",
            ));
        }
        for image in snapshots.values() {
            let _ = verified_source(image)?;
        }
        let source_files = PublicationFiles::new(&self.source)?;
        let source_identity = source_files.root_identity();
        let mut target_files = if target.exists() {
            Some(PublicationFiles::new(&target)?)
        } else {
            None
        };
        let target_identity = target_files.as_ref().map(PublicationFiles::root_identity);
        let mut files = Vec::new();
        for effect in effects {
            let before = if let Some(target_files) = &mut target_files {
                target_image(target_files, &effect.path, effect.before.as_deref())?
            } else {
                None
            };
            let item = inventory.files.iter().find(|item| {
                item.path == effect.path
                    || item.path == "README.md" && effect.path == "ACCELERATOR.md"
            });
            let digest = common::sha256(&effect.after);
            let image = item
                .and_then(|item| snapshots.get(&item.source))
                .filter(|image| image.sha256 == digest);
            let payload = if let Some(image) = image {
                PlannedPayload::Source {
                    image: image.clone(),
                }
            } else {
                PlannedPayload::Inline {
                    sha256: digest,
                    content: effect.after,
                }
            };
            files.push(PlannedFile {
                path: effect.path,
                before,
                payload,
                mode: effect.mode,
                action: effect.action,
                record: effect.record,
            });
        }
        let control = if let Some(target_files) = &mut target_files {
            let current = target_files.read(RECORD, 4 * 1024 * 1024)?;
            target_image(target_files, RECORD, current.as_deref())?
        } else {
            None
        };
        let directories = target_files
            .as_ref()
            .map(PublicationFiles::directory_identities)
            .transpose()?
            .unwrap_or_default();
        if target_identity.is_none() != target_anchor.is_some() {
            return Err(RuntimeError::conflict(
                "Install target existence changed during preview",
            ));
        }
        if let Some(anchor) = &target_anchor {
            let _ = NewTargetDirectories::open(anchor)?;
        }
        let mut plan = InstallPlan {
            schema_version: 1,
            nonce: uuid::Uuid::new_v4().to_string(),
            source: self.source.clone(),
            source_identity,
            source_commit: self.source_commit()?,
            target,
            target_identity,
            target_anchor,
            directories,
            edition: inventory.edition,
            release: inventory.release,
            tools: tools.to_vec(),
            options: options.clone(),
            engine: capabilities,
            sources: snapshots.into_values().collect(),
            control,
            files,
            kept,
            collisions,
            sha256: String::new(),
        };
        plan.sha256 = plan.digest()?;
        Ok(plan)
    }
    pub fn apply_plan(&self, plan: &InstallPlan) -> Result<Value> {
        self.apply_plan_observed(plan, |_, _| Ok(()))
    }
    /// Reports each durable planned payload write. A later transactional rollback
    /// reports "rolled-back"; callers must remove it from confirmed changes.
    pub fn apply_plan_observed(
        &self,
        plan: &InstallPlan,
        mut observer: impl FnMut(&str, &str) -> Result<()>,
    ) -> Result<Value> {
        plan.verify()?;
        if plan.source != self.source
            || PublicationFiles::new(&self.source)?.root_identity() != plan.source_identity
            || self.source_commit()? != plan.source_commit
        {
            return Err(RuntimeError::conflict(
                "Install source checkout changed after preview",
            ));
        }
        if !plan.collisions.is_empty() {
            return Ok(plan.report("refused", false));
        }
        // Every source and every target is re-admitted before the first write.
        for source in &plan.sources {
            let _ = verified_source(source)?;
        }
        let engine = plan
            .sources
            .iter()
            .find(|source| source.path == self.engine)
            .ok_or_else(|| {
                RuntimeError::conflict("Current installer executable differs from reviewed engine")
            })?;
        if plan.engine["artifact_sha256"].as_str() != Some(engine.sha256.as_str()) {
            return Err(RuntimeError::conflict(
                "Reviewed engine metadata differs from its artifact",
            ));
        }
        let mut created_directories = plan
            .target_anchor
            .as_ref()
            .map(NewTargetDirectories::open)
            .transpose()?;
        let mut custody = match plan.target_identity {
            Some(identity) => {
                let mut files = PublicationFiles::new(&plan.target)?;
                if files.root_identity() != identity {
                    return Err(RuntimeError::conflict(
                        "Install target root identity changed",
                    ));
                }
                files.check_directory_identities(&plan.directories)?;
                Some(files)
            }
            None => {
                if std::fs::symlink_metadata(&plan.target).is_ok() {
                    return Err(RuntimeError::conflict(
                        "Previously absent install target now exists",
                    ));
                }
                None
            }
        };
        let mut payloads = Vec::new();
        for file in &plan.files {
            let previous = if let Some(custody) = &mut custody {
                verified_target(custody, &file.path, file.before.as_ref())?
            } else {
                if file.before.is_some() {
                    return Err(RuntimeError::conflict(
                        "Missing target has a reviewed existing file",
                    ));
                }
                None
            };
            let content = match &file.payload {
                PlannedPayload::Source { image } => verified_source(image)?,
                PlannedPayload::Inline { content, sha256 } => {
                    if content.len() > FILE_LIMIT || common::sha256(content) != *sha256 {
                        return Err(RuntimeError::conflict(
                            "Reviewed merged payload checksum differs",
                        ));
                    }
                    content.clone()
                }
            };
            payloads.push((file.clone(), previous, content));
        }
        let control = if let Some(custody) = &mut custody {
            verified_target(custody, RECORD, plan.control.as_ref())?
        } else {
            None
        };
        if custody.is_none() {
            let created = created_directories.as_mut().ok_or_else(|| {
                RuntimeError::conflict("Missing reviewed install target ancestor")
            })?;
            custody = Some(created.create_target(&plan.target)?);
        }
        let mut custody = custody
            .ok_or_else(|| RuntimeError::unavailable("Install target custody unavailable"))?;
        let mut applied = Vec::<PublishedInstallFile>::new();
        payloads.sort_by_key(|(file, _, _)| {
            if file.path == ENGINE {
                0
            } else if is_wiring(&file.path) {
                3
            } else if hook_wiring(&file.path).is_some() {
                1
            } else {
                2
            }
        });
        let outcome = (|| -> Result<()> {
            if let Some(created) = &mut created_directories {
                created.create_payload_parents(&mut custody, &plan.files)?;
            }
            for (file, previous, content) in &payloads {
                if let Some(created) = &created_directories {
                    created.anchor.check()?;
                }
                custody.write_reviewed_image(
                    &file.path,
                    content,
                    ExpectedImage {
                        bytes: previous.as_deref(),
                        mode: file.before.as_ref().map(|before| before.mode),
                        identity: file.before.as_ref().map(|before| before.identity),
                    },
                    file.mode,
                    FILE_LIMIT,
                    |stage| {
                        if let WriteStage::Published(published) = stage {
                            applied.push(PublishedInstallFile {
                                path: file.path.clone(),
                                previous: previous.clone(),
                                previous_mode: file.before.as_ref().map(|before| before.mode),
                                content: content.clone(),
                                published,
                            });
                        }
                        Ok(())
                    },
                )?;
                observer(&file.path, &file.action)?;
            }
            let parsed = control
                .as_ref()
                .map(|bytes| serde_json::from_slice::<Value>(bytes))
                .transpose();
            if let Ok(value) = parsed
                && value.as_ref().is_none_or(Value::is_object)
            {
                let mut value = value.unwrap_or_else(|| json!({}));
                let mut known = value["files"].as_object().cloned().unwrap_or_default();
                for (file, _, content) in &payloads {
                    if file.record {
                        known.insert(file.path.clone(), common::sha256(content).into());
                    }
                }
                value["schema"] = 2.into();
                value["edition"] = plan.edition.clone().into();
                value["release"] = plan.release.clone().into();
                value["source_commit"] = json!(plan.source_commit);
                value["synced_at"] = infra_core::domain::utc_now().into();
                value["last_install_plan"] = plan.nonce.clone().into();
                value["engine_sha256"] = engine.sha256.clone().into();
                value["engine_capabilities"] = plan.engine.clone();
                value["files"] = Value::Object(known);
                let bytes = common::json_bytes(&value)?;
                if let Some(created) = &created_directories {
                    created.anchor.check()?;
                }
                custody.write_reviewed_image(
                    RECORD,
                    &bytes,
                    ExpectedImage {
                        bytes: control.as_deref(),
                        mode: plan.control.as_ref().map(|before| before.mode),
                        identity: plan.control.as_ref().map(|before| before.identity),
                    },
                    plan.control
                        .as_ref()
                        .map(|before| before.mode)
                        .unwrap_or(0o600),
                    4 * 1024 * 1024,
                    |stage| {
                        if let WriteStage::Published(published) = stage {
                            applied.push(PublishedInstallFile {
                                path: RECORD.into(),
                                previous: control.clone(),
                                previous_mode: plan.control.as_ref().map(|before| before.mode),
                                content: bytes.clone(),
                                published,
                            });
                        }
                        Ok(())
                    },
                )?;
            }
            if let Some(created) = &created_directories {
                created.anchor.check()?;
            }
            Ok(())
        })();
        if let Err(original) = outcome {
            let unreceipted = original.code.as_deref() == Some(POST_RENAME_IDENTITY_UNAVAILABLE);
            let mut post_rename_errno = None;
            retain_post_rename_errno(&mut post_rename_errno, &original);
            let mut errors = Vec::new();
            if unreceipted {
                // This name was committed but no exact post-rename receipt was
                // obtainable. Preserve it; compensate only earlier receipted effects.
                errors.push(
                    "Unreceipted committed publication was preserved for recovery".to_string(),
                );
            }
            for file in applied.iter().rev() {
                let expected = ExpectedImage {
                    bytes: Some(file.content.as_slice()),
                    mode: Some(file.published.mode),
                    identity: Some(file.published.identity),
                };
                let result = if let Some(previous) = &file.previous {
                    custody
                        .write_reviewed_image(
                            &file.path,
                            previous,
                            expected,
                            file.previous_mode.unwrap_or(0o644),
                            FILE_LIMIT,
                            |_| Ok(()),
                        )
                        .map(|_| ())
                } else {
                    custody.remove_reviewed_image(&file.path, expected, FILE_LIMIT, |_| Ok(()))
                };
                if let Err(error) = result {
                    retain_post_rename_errno(&mut post_rename_errno, &error);
                    errors.push(format!("{}: {error}", file.path));
                    if file.path != RECORD
                        && let Err(error) = observer(&file.path, "rollback-failed")
                    {
                        retain_post_rename_errno(&mut post_rename_errno, &error);
                        errors.push(format!("{} progress: {error}", file.path));
                    }
                } else if file.path != RECORD
                    && let Err(error) = observer(&file.path, "rolled-back")
                {
                    retain_post_rename_errno(&mut post_rename_errno, &error);
                    errors.push(format!("{} progress: {error}", file.path));
                }
            }
            if let Some(created) = &mut created_directories
                && let Err(error) = created.rollback(&mut custody)
            {
                retain_post_rename_errno(&mut post_rename_errno, &error);
                errors.push(error.to_string());
            }
            if errors.is_empty() {
                return Err(original);
            }
            return Err(rollback_attention_error(
                &original,
                &errors,
                post_rename_errno,
            ));
        }
        Ok(plan.report("complete", false))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewImage {
    pub path: String,
    pub source_path: Option<PathBuf>,
    pub action: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub before_text: Option<String>,
    pub after_text: Option<String>,
    pub diff_truncated: bool,
    pub reason: Option<String>,
}
impl Installer {
    /// Materialize bounded review text from the frozen receipt, without merging
    /// again or granting approval. Large/binary artifacts keep exact sizes/digests.
    pub fn preview_images(
        &self,
        plan: &InstallPlan,
        max_text_bytes: usize,
    ) -> Result<Vec<PreviewImage>> {
        if max_text_bytes == 0 || max_text_bytes > 1024 * 1024 {
            return Err(RuntimeError::invalid("Invalid install preview text bound"));
        }
        plan.verify()?;
        if plan.source != self.source
            || PublicationFiles::new(&self.source)?.root_identity() != plan.source_identity
            || self.source_commit()? != plan.source_commit
        {
            return Err(RuntimeError::conflict("Reviewed install source changed"));
        }
        let mut custody = if let Some(identity) = plan.target_identity {
            let mut files = PublicationFiles::new(&plan.target)?;
            if files.root_identity() != identity {
                return Err(RuntimeError::conflict("Reviewed install target changed"));
            }
            files.check_directory_identities(&plan.directories)?;
            Some(files)
        } else {
            if std::fs::symlink_metadata(&plan.target).is_ok() {
                return Err(RuntimeError::conflict("Reviewed absent target now exists"));
            }
            None
        };
        let mut output = Vec::new();
        for file in &plan.files {
            let previous = if let Some(custody) = &mut custody {
                verified_target(custody, &file.path, file.before.as_ref())?
            } else {
                None
            };
            let (source_path, content) = match &file.payload {
                PlannedPayload::Source { image } => {
                    (Some(image.path.clone()), verified_source(image)?)
                }
                PlannedPayload::Inline { content, sha256 } => {
                    if common::sha256(content) != *sha256 {
                        return Err(RuntimeError::conflict("Reviewed merged payload changed"));
                    }
                    (None, content.clone())
                }
            };
            let before_bytes = previous
                .as_ref()
                .map(|bytes| bytes.len() as u64)
                .unwrap_or(0);
            let after_bytes = content.len() as u64;
            let truncated =
                before_bytes > max_text_bytes as u64 || after_bytes > max_text_bytes as u64;
            let binary = previous
                .as_ref()
                .is_some_and(|bytes| std::str::from_utf8(bytes).is_err())
                || std::str::from_utf8(&content).is_err();
            let before_text = if !truncated && !binary {
                Some(
                    String::from_utf8(previous.unwrap_or_default())
                        .map_err(|_| RuntimeError::invalid("Preview text is not UTF-8"))?,
                )
            } else {
                None
            };
            let after_text = if !truncated && !binary {
                Some(
                    String::from_utf8(content)
                        .map_err(|_| RuntimeError::invalid("Preview text is not UTF-8"))?,
                )
            } else {
                None
            };
            output.push(PreviewImage {
                path: file.path.clone(),
                source_path,
                action: file.action.clone(),
                before_bytes,
                after_bytes,
                before_text,
                after_text,
                diff_truncated: truncated,
                reason: if binary {
                    Some("Binary artifact; review its pinned checksum and size".into())
                } else if truncated {
                    Some(
                        "Text exceeds the bounded preview; review the source and target locally"
                            .into(),
                    )
                } else {
                    None
                },
            });
        }
        Ok(output)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativePayloadSources {
    schema_version: u32,
    editions: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    optional_files: Vec<String>,
}
impl Installer {
    fn native_payload_paths(&self, edition: &str) -> Result<Vec<String>> {
        let manifest: NativePayloadSources = serde_json::from_slice(&safe::read_regular(
            &self.source.join("install/native-payload.json"),
        )?)?;
        if manifest.schema_version != 1 {
            return Err(RuntimeError::invalid(
                "Unsupported native payload source manifest",
            ));
        }
        for name in manifest.editions.keys() {
            if !common::PHP_EDITIONS.contains(&name.as_str()) {
                return Err(RuntimeError::invalid(
                    "Native payload source manifest has an unknown edition",
                ));
            }
        }
        for name in common::PHP_EDITIONS {
            if !manifest.editions.contains_key(*name) {
                return Err(RuntimeError::invalid(
                    "Native payload source manifest omits an installable edition",
                ));
            }
        }
        let paths = manifest.editions.get(edition).ok_or_else(|| {
            RuntimeError::invalid("Native payload source manifest omits selected edition")
        })?;
        let mut previous = None::<&str>;
        let base = self.source.join(edition);
        let mut files = PublicationFiles::new(&base)?;
        for name in paths {
            common::relative(name)?;
            if previous.is_some_and(|value| value >= name.as_str())
                || source_only(name)?
                || name.starts_with("memory-bank/bin/")
                || name.ends_with("/infra-engine")
                || name.ends_with("/infra-engine.exe")
            {
                return Err(RuntimeError::invalid(
                    "Native payload additions must be sorted unique static production resources",
                ));
            }
            previous = Some(name.as_str());
            if files.capture_source(name, 8 * 1024 * 1024)?.is_none() {
                return Err(RuntimeError::not_found(format!(
                    "Declared native payload resource is absent: {edition}/{name}; synchronize canonical resources before regenerating inventories"
                )));
            }
        }
        let mut selected = paths.clone();
        let mut previous = None::<&str>;
        for name in &manifest.optional_files {
            common::relative(name)?;
            if previous.is_some_and(|value| value >= name.as_str())
                || source_only(name)?
                || name.starts_with("memory-bank/bin/")
            {
                return Err(RuntimeError::invalid(
                    "Optional native payload additions must be sorted unique static production resources",
                ));
            }
            previous = Some(name.as_str());
            // Only this explicit optional path is examined; no untracked directory is
            // enumerated. The generated project requires its policy via its own plan.
            let limit = if name == "project-brain/config/hook-policy.json" {
                64 * 1024
            } else {
                8 * 1024 * 1024
            };
            if files.capture_source(name, limit)?.is_some() {
                selected.push(name.clone());
            }
        }
        selected.sort();
        selected.dedup();
        Ok(selected)
    }
}

fn native_engine_descriptor() -> Value {
    json!({"unix_path":"memory-bank/bin/infra-engine","windows_path":"memory-bank/bin/infra-engine.exe","artifact":"current-executable","capability_profile":"infra-engine-v1"})
}

fn native_release_payload(path: &str, bytes: &[u8], target: &Path) -> Result<Vec<u8>> {
    let host = match path {
        ".claude/settings.json" => "claude",
        ".cursor/hooks.json" => "cursor",
        ".codex/hooks.json" => "codex",
        _ => return Ok(bytes.to_vec()),
    };
    crate::attach::normalize_hook_configuration(
        bytes,
        host,
        target,
        &target.join(ENGINE),
        cfg!(windows),
    )
}

#[cfg(test)]
mod planned_payload_wire_tests {
    use super::*;

    fn source() -> PlannedPayload {
        PlannedPayload::Source {
            image: SourceImage {
                path: PathBuf::from("/native release/source"),
                parent_identity: (11, 12, 0o40755),
                identity: (13, 14, 0o100644, 1, 3, i128::MAX, i128::MIN),
                sha256: common::sha256(b"abc"),
                bytes: 3,
            },
        }
    }

    #[test]
    fn source_and_inline_roundtrip_exactly_through_bytes_and_value() {
        for payload in [
            source(),
            PlannedPayload::Inline {
                content: b"reviewed inline\n".to_vec(),
                sha256: common::sha256(b"reviewed inline\n"),
            },
        ] {
            let value = serde_json::to_value(&payload).unwrap();
            let bytes = serde_json::to_vec(&payload).unwrap();
            for decoded in [
                serde_json::from_slice::<PlannedPayload>(&bytes).unwrap(),
                serde_json::from_value::<PlannedPayload>(value.clone()).unwrap(),
            ] {
                assert_eq!(serde_json::to_value(&decoded).unwrap(), value);
                if let PlannedPayload::Source { image } = decoded {
                    assert_eq!(image.identity.5, i128::MAX);
                    assert_eq!(image.identity.6, i128::MIN);
                }
            }
        }
    }

    #[test]
    fn unknown_cross_variant_missing_null_and_remove_fields_are_refused() {
        let source = serde_json::to_value(source()).unwrap();
        let image = source["image"].clone();
        for value in [
            json!({"kind":"remove","image":image}),
            json!({"kind":"inline","content":[],"sha256":"x","unexpected":true}),
            json!({"kind":"inline","content":[],"sha256":"x","image":image}),
            json!({"kind":"source","image":image,"content":[]}),
            json!({"kind":"source","image":image,"sha256":"x"}),
            json!({"kind":"source"}),
            json!({"kind":"inline","content":[]}),
            json!({"kind":"inline","content":null,"sha256":"x"}),
            json!({"kind":"inline","content":[],"sha256":null}),
            json!({"kind":"source","image":null}),
            json!({"kind":"source","image":image,"content":null}),
        ] {
            let bytes = serde_json::to_vec(&value).unwrap();
            assert!(
                serde_json::from_slice::<PlannedPayload>(&bytes).is_err(),
                "{value}"
            );
            assert!(
                serde_json::from_value::<PlannedPayload>(value.clone()).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn duplicate_wire_fields_are_refused_before_the_receipt_is_admitted() {
        for bytes in [
            r#"{"kind":"inline","kind":"source","content":[],"sha256":"x"}"#,
            r#"{"kind":"inline","content":[],"content":[1],"sha256":"x"}"#,
            r#"{"kind":"inline","content":[],"sha256":"x","sha256":"y"}"#,
            r#"{"kind":"source","image":{"path":"/x","path":"/y","parent_identity":[1,2,3],"identity":[1,2,3,1,0,1,2],"sha256":"x","bytes":0}}"#,
        ] {
            assert!(serde_json::from_slice::<PlannedPayload>(bytes.as_bytes()).is_err());
        }
    }
}

#[cfg(test)]
mod rollback_attention_tests {
    use super::*;

    fn failure(message: &str, typed: bool, os_code: Option<i32>) -> RuntimeError {
        let mut error = if typed {
            RuntimeError::coded(
                infra_core::ErrorKind::Io,
                POST_RENAME_IDENTITY_UNAVAILABLE,
                message,
            )
        } else {
            RuntimeError::conflict(message)
        };
        error.os_code = os_code;
        error
    }

    #[test]
    fn compensation_metadata_failure_retains_typed_code_errno_and_all_context() {
        for (original_typed, original_errno, compensation, expected_errno) in [
            (false, Some(22), vec![(true, Some(5))], Some(Some(5))),
            (false, Some(22), vec![(true, None)], Some(None)),
            (true, None, vec![(true, Some(5))], Some(None)),
            (
                false,
                None,
                vec![(true, Some(5)), (true, Some(13))],
                Some(Some(5)),
            ),
            (false, Some(22), vec![(false, Some(13))], None),
        ] {
            let original = failure("original install failure", original_typed, original_errno);
            let mut selected = None;
            retain_post_rename_errno(&mut selected, &original);
            let mut messages = Vec::new();
            for (ordinal, (typed, os_code)) in compensation.into_iter().enumerate() {
                let error = failure(&format!("compensation {ordinal}"), typed, os_code);
                retain_post_rename_errno(&mut selected, &error);
                messages.push(format!("value-{ordinal}.json: {error}"));
            }
            let error = rollback_attention_error(&original, &messages, selected);
            assert_eq!(error.kind, infra_core::ErrorKind::Conflict);
            assert_eq!(selected, expected_errno);
            assert_eq!(
                error.code.as_deref(),
                expected_errno.map(|_| POST_RENAME_IDENTITY_UNAVAILABLE),
            );
            assert_eq!(error.os_code, expected_errno.flatten());
            assert!(error.message.contains("original install failure"));
            assert!(error.message.contains("rollback requires attention"));
            for message in &messages {
                assert!(error.message.contains(message));
            }
        }
    }
}
