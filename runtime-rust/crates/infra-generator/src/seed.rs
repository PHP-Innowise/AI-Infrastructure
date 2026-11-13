//! Seed only the native memory/Brain asset closure; project policy and skills are authored elsewhere.
use crate::{assets, common::*, ownership};
use infra_core::bank::publication::PublicationFiles;
use infra_core::{Result, RuntimeError};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
const DATA_LIMIT: usize = 2 * 1024 * 1024;
const ENGINE_LIMIT: usize = 512 * 1024 * 1024;
const DIRECTORIES: &[&str] = &[
    "archive",
    "local",
    "control/handoffs",
    "control/messages",
    "control/promotions",
    "control/retrieval-manifests",
    "dynamic/bugs",
    "dynamic/decisions",
    "dynamic/events",
    "dynamic/findings",
    "dynamic/incidents",
    "dynamic/tasks",
];
struct Image {
    source: String,
    bytes: Vec<u8>,
    mode: u32,
}
fn input(
    files: &mut PublicationFiles,
    name: &str,
    limit: usize,
    identities: &mut BTreeMap<String, Value>,
) -> Result<Vec<u8>> {
    let before = files.image_identity(name)?;
    let raw = files
        .read(name, limit)?
        .ok_or_else(|| RuntimeError::not_found(format!("Native seed asset missing: {name}")))?;
    if files.image_identity(name)? != before {
        return Err(RuntimeError::conflict("Seed asset changed during capture"));
    }
    identities.insert(name.into(), object!(before));
    Ok(raw)
}
fn json(raw: &[u8]) -> Result<Value> {
    infra_core::domain::json::parse(
        std::str::from_utf8(raw)
            .map_err(|_| RuntimeError::invalid("Seed asset JSON is not UTF-8"))?,
    )
}
fn native_magic(raw: &[u8]) -> bool {
    if cfg!(windows) {
        raw.starts_with(b"MZ")
    } else if cfg!(target_os = "macos") {
        [
            [0xcf, 0xfa, 0xed, 0xfe],
            [0xfe, 0xed, 0xfa, 0xcf],
            [0xca, 0xfe, 0xba, 0xbe],
            [0xbe, 0xba, 0xfe, 0xca],
        ]
        .iter()
        .any(|magic| raw.starts_with(magic))
    } else {
        raw.starts_with(b"\x7fELF")
    }
}
pub fn assets(
    assets_root: &Path,
    staging: &Path,
    framework: &str,
    editions: &[String],
    engine: Option<&Path>,
) -> Result<Value> {
    if !matching(r"^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$", framework)
        || editions.is_empty()
        || editions
            .iter()
            .any(|edition| !["claude", "cursor", "codex"].contains(&edition.as_str()))
        || editions.iter().collect::<BTreeSet<_>>().len() != editions.len()
    {
        return Err(RuntimeError::invalid(
            "Seed assets require a confirmed framework slug and selected editions",
        ));
    }
    let assets_root = ownership::resolve_target(assets_root)?;
    let staging = ownership::resolve_target(staging)?;
    if assets_root.starts_with(&staging) || staging.starts_with(&assets_root) {
        return Err(RuntimeError::policy(
            "Seed assets and staging must be disjoint",
        ));
    }
    let mut source = PublicationFiles::new(&assets_root)?;
    let mut target = PublicationFiles::new(&staging)?;
    let mut identities = BTreeMap::new();
    let mut output = BTreeMap::<String, Image>::new();
    let descriptor = json(&input(
        &mut source,
        "native-artifacts.json",
        DATA_LIMIT,
        &mut identities,
    )?)?;
    let artifact = if cfg!(windows) {
        "bin/infra-engine.exe"
    } else {
        "bin/infra-engine"
    };
    let capabilities = texts(&descriptor["capabilities"]);
    let required = texts(&assets::runtime_contract()["native_engine"]["required_capabilities"]);
    if descriptor["schema_version"] != 1
        || descriptor["engine"] != "infra-engine"
        || descriptor["capability_profile"] != "infra-engine-v1"
        || !nonempty(&descriptor["version"])
        || descriptor["target"] != infra_tooling::artifacts::native_target()?
        || descriptor["path"] != artifact
        || !hex(text(&descriptor["sha256"]), 64)
        || descriptor["bytes"]
            .as_u64()
            .is_none_or(|bytes| bytes > ENGINE_LIMIT as u64)
        || capabilities.len()
            != descriptor["capabilities"]
                .as_array()
                .map(Vec::len)
                .unwrap_or(0)
        || required
            .iter()
            .any(|required| !capabilities.contains(required))
    {
        return Err(RuntimeError::policy(
            "Native seed artifact descriptor does not match this platform and runtime capability contract",
        ));
    }
    let (raw, external) = if let Some(engine) = engine {
        let engine = ownership::resolve_target(engine)?;
        let parent = engine
            .parent()
            .ok_or_else(|| RuntimeError::policy("Explicit engine has no parent"))?;
        let name = engine
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| RuntimeError::policy("Explicit engine name is not UTF-8"))?;
        let mut file = PublicationFiles::new(parent)?;
        let before = file.image_identity(name)?;
        let raw = file
            .read(name, ENGINE_LIMIT)?
            .ok_or_else(|| RuntimeError::not_found("Explicit native engine missing"))?;
        if file.image_identity(name)? != before {
            return Err(RuntimeError::conflict(
                "Explicit native engine changed while capturing",
            ));
        }
        (raw, Some((file, name.to_string(), before)))
    } else {
        (
            input(&mut source, artifact, ENGINE_LIMIT, &mut identities)?,
            None,
        )
    };
    if descriptor["bytes"].as_u64() != Some(raw.len() as u64)
        || sha(&raw) != text(&descriptor["sha256"])
        || !native_magic(&raw)
    {
        return Err(RuntimeError::policy(
            "Selected native seed artifact differs from the explicit release descriptor",
        ));
    }
    let executable = if cfg!(windows) { 0o644 } else { 0o755 };
    output.insert(
        format!("memory-bank/{artifact}"),
        Image {
            source: artifact.into(),
            bytes: raw,
            mode: executable,
        },
    );
    output.insert(
        "memory-bank/bin/.gitignore".into(),
        Image {
            source: "native engine ignore rules".into(),
            bytes: infra_tooling::launch::ENGINE_IGNORE.as_bytes().to_vec(),
            mode: 0o644,
        },
    );
    for (from, to) in [
        ("runtime-contract.json", "memory-bank/runtime-contract.json"),
        ("MCP.md", "memory-bank/MCP.md"),
        ("templates/chunk.md", "memory-bank/templates/chunk.md"),
        (
            "config/mirror-rules.json",
            "memory-bank/config/mirror-rules.json",
        ),
    ] {
        let raw = input(&mut source, from, DATA_LIMIT, &mut identities)?;
        if from == "runtime-contract.json" && json(&raw)? != assets::runtime_contract() {
            return Err(RuntimeError::policy(
                "Seed runtime contract differs from the trusted native contract",
            ));
        }
        output.insert(
            to.into(),
            Image {
                source: from.into(),
                bytes: raw,
                mode: 0o644,
            },
        );
    }
    let (paths, omitted) = source.walk_files(12, 10000, |name| {
        !name
            .split('/')
            .any(|part| ["scripts", "bin"].contains(&part))
    })?;
    if omitted > 0 {
        return Err(RuntimeError::policy(
            "Native seed asset discovery exceeded the static asset bound",
        ));
    }
    for path in paths {
        let Some(relative) = path.strip_prefix("project-brain/") else {
            continue;
        };
        let data = [
            "README.md",
            "PROTOCOL.md",
            ".gitignore",
            "config/providers.json",
            "config/telemetry.json",
            "indexes/active.json",
            "indexes/archive.json",
        ]
        .contains(&relative)
            || (relative.starts_with("schemas/") && relative.ends_with(".schema.json"))
            || (relative.starts_with("templates/")
                && (relative.ends_with(".md") || relative.ends_with(".json")))
            || DIRECTORIES
                .iter()
                .any(|directory| relative == format!("{directory}/.gitkeep"));
        if !data {
            continue;
        }
        let raw = input(&mut source, &path, DATA_LIMIT, &mut identities)?;
        if ["indexes/active.json", "indexes/archive.json"].contains(&relative)
            && json(&raw)? != object!([])
        {
            return Err(RuntimeError::policy(
                "Brain seed indexes must be empty arrays",
            ));
        }
        output.insert(
            path.clone(),
            Image {
                source: path,
                bytes: raw,
                mode: 0o644,
            },
        );
    }
    for required in [
        "project-brain/schemas/dynamic-record.schema.json",
        "project-brain/schemas/handoff.schema.json",
        "project-brain/schemas/message.schema.json",
        "project-brain/schemas/promotion.schema.json",
        "project-brain/schemas/retrieval-manifest.schema.json",
        "project-brain/schemas/token-usage-event.schema.json",
        "project-brain/templates/task.md",
        "project-brain/templates/finding.md",
        "project-brain/templates/bug.md",
        "project-brain/templates/incident.md",
        "project-brain/templates/decision.md",
        "project-brain/templates/event.md",
        "project-brain/templates/handoff.md",
        "project-brain/templates/retrieval-manifest.json",
        "project-brain/templates/promotion.json",
        "project-brain/README.md",
        "project-brain/PROTOCOL.md",
        "project-brain/.gitignore",
        "project-brain/config/providers.json",
        "project-brain/config/telemetry.json",
        "project-brain/indexes/active.json",
        "project-brain/indexes/archive.json",
    ] {
        if !output.contains_key(required) {
            return Err(RuntimeError::not_found(format!(
                "Native seed skeleton asset missing: {required}"
            )));
        }
    }
    let raw = input(
        &mut source,
        "project-brain/config/runtime.json.template",
        DATA_LIMIT,
        &mut identities,
    )?;
    let template = std::str::from_utf8(&raw)
        .map_err(|_| RuntimeError::invalid("Brain runtime template is not UTF-8"))?;
    if template.matches("{{TARGET_FRAMEWORK}}").count() != 1
        || template.matches("{{CANONICAL_EDITION}}").count() != 1
    {
        return Err(RuntimeError::policy(
            "Brain template must contain exactly the two sanctioned substitutions",
        ));
    }
    let canonical = if editions.iter().any(|edition| edition == "codex") {
        ".agents"
    } else if editions.iter().any(|edition| edition == "claude") {
        ".claude"
    } else {
        ".cursor"
    };
    let runtime = template
        .replace("{{TARGET_FRAMEWORK}}", framework)
        .replace("{{CANONICAL_EDITION}}", canonical);
    let config = json(runtime.as_bytes())?;
    if config["framework"] != framework || config["canonical_edition"] != canonical {
        return Err(RuntimeError::policy(
            "Materialized runtime substitutions do not match selected profile",
        ));
    }
    output.insert(
        "project-brain/config/runtime.json".into(),
        Image {
            source: "project-brain/config/runtime.json.template".into(),
            bytes: runtime.into_bytes(),
            mode: 0o644,
        },
    );
    for directory in DIRECTORIES {
        let name = format!("project-brain/{directory}/.gitkeep");
        output.entry(name).or_insert_with(|| Image {
            source: "prescribed empty runtime directory".into(),
            bytes: vec![],
            mode: 0o644,
        });
    }
    output.insert(
        "memory-bank/local/.gitkeep".into(),
        Image {
            source: "prescribed empty local directory".into(),
            bytes: vec![],
            mode: 0o644,
        },
    );
    // Creating chunks/.gitkeep establishes the chunk directory without inventing knowledge.
    output.insert(
        "memory-bank/chunks/.gitkeep".into(),
        Image {
            source: "prescribed empty chunks directory".into(),
            bytes: vec![],
            mode: 0o644,
        },
    );
    let mut unchanged = vec![];
    for (name, image) in &output {
        ownership::normalize(name)?;
        if let Some(existing) = target.read(name, ENGINE_LIMIT)? {
            if existing != image.bytes || target.read_mode(name)? != Some(image.mode) {
                return Err(RuntimeError::conflict(format!(
                    "Seed staging collision; preserve the existing file and use a fresh staging root: {name}"
                )));
            }
            unchanged.push(name.clone());
        }
    }
    for (name, identity) in &identities {
        if object!(source.image_identity(name)?) != *identity {
            return Err(RuntimeError::conflict(
                "Seed source changed before staged asset write",
            ));
        }
    }
    let mut external = external;
    if let Some((file, name, before)) = &mut external
        && file.image_identity(name)? != *before
    {
        return Err(RuntimeError::conflict(
            "Explicit native engine changed before staging",
        ));
    }
    let mut written = Vec::<(String, Value)>::new();
    let result = (|| -> Result<()> {
        for (name, image) in &output {
            if unchanged.contains(name) {
                continue;
            }
            target.write_expected_mode_checked(
                name,
                &image.bytes,
                None,
                None,
                image.mode,
                ENGINE_LIMIT,
            )?;
            written.push((name.clone(), object!(target.image_identity(name)?)));
        }
        source.check()?;
        target.check()?;
        Ok(())
    })();
    if let Err(error) = result {
        for (name, identity) in written.iter().rev() {
            if object!(target.image_identity(name)?) == *identity
                && let Some(bytes) = target.read(name, ENGINE_LIMIT)?
                && bytes == output[name].bytes
            {
                let _ = target.unlink_expected_limit(name, &bytes, ENGINE_LIMIT);
            }
        }
        return Err(error);
    }
    let files=output.iter().map(|(name,image)|(name.clone(),object!({"sha256":sha(&image.bytes),"source":image.source,"bytes":image.bytes.len(),"mode":image.mode}))).collect::<BTreeMap<_,_>>();
    Ok(
        object!({"framework":framework,"canonical_edition":canonical,"editions":editions,"artifact":descriptor,"files":files,"copied":written.iter().map(|(name,_)|name).collect::<Vec<_>>(),"unchanged":unchanged,"authored_elsewhere":["memory-bank/README.md","memory-bank/INDEX.md","memory-bank/.memory-counter","memory-bank/chunks/MEM-*.md","AGENTS.md","skills","hooks","agents","commands"],"staging_only":true}),
    )
}
