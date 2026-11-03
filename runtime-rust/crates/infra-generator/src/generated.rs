use crate::{common::*, flows, ownership, quality};
use infra_core::{BankService, BrainRepository, Result, Roots, RuntimeContext, RuntimeError, fs};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct NativeOutput {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
/// Services are mandatory. Host adapters must validate their complete native
/// wiring/identity/capture/journal/continuity/MCP contracts, not just file presence.
pub trait NativeGateServices {
    fn validate_host(
        &self,
        generation_root: &Path,
        delivery_root: &Path,
        edition: &str,
        owned: &BTreeMap<String, String>,
    ) -> Result<Vec<String>>;
    fn runtime_command(
        &self,
        target: &Path,
        arguments: &[String],
        timeout: Duration,
    ) -> Result<NativeOutput>;
}
#[derive(Clone, Debug)]
pub struct GeneratedOptions {
    pub target: PathBuf,
    pub delivery_root: Option<PathBuf>,
    pub editions: Vec<String>,
    pub plan: Option<PathBuf>,
    pub approved_profile: Option<PathBuf>,
    pub approved_profile_sha256: Option<String>,
    pub profile_path: Option<String>,
    pub evidence_target: Option<PathBuf>,
    pub candidate_registry: PathBuf,
    pub generator_version: Option<String>,
    pub runtime_contract: Value,
}
pub fn validate_readiness(payload: &Value) -> Vec<String> {
    let mut errors = vec![];
    let readiness = &payload["automatic_memory"];
    if !payload.is_object() || !readiness.is_object() {
        return vec!["status missing automatic_memory readiness object".into()];
    }
    let allowed = ["active", "retrieval-only", "degraded"];
    if !allowed.contains(&text(&readiness["status"])) {
        errors.push("automatic_memory.status must be active, retrieval-only or degraded".into());
    }
    for component in [None, Some("task_identity"), Some("git_metadata")] {
        let report = component.map(|key| &readiness[key]).unwrap_or(readiness);
        let label = component.unwrap_or("automatic_memory");
        if !report.is_object()
            || !allowed.contains(&text(&report["status"]))
            || !nonempty(&report["reason"])
            || report.get("remediation").is_none()
            || (!report["remediation"].is_null() && !nonempty(&report["remediation"]))
        {
            errors.push(format!("{label}: invalid readiness shape"));
        }
    }
    let expected = match (
        text(&readiness["task_identity"]["status"]),
        text(&readiness["git_metadata"]["status"]),
    ) {
        ("active", "active") => "active",
        ("active", "degraded") => "retrieval-only",
        _ => "degraded",
    };
    if readiness["status"] != expected {
        errors.push(format!(
            "automatic_memory.status inconsistent with task/Git readiness: expected {expected}"
        ));
    }
    errors
}
pub fn validate_manifest(
    target: &Path,
    generator_version: Option<&str>,
) -> Result<(Value, Vec<String>)> {
    let manifest = ownership::load_manifest(target)?;
    let mut errors = vec![];
    if manifest["manifest_version"] != 1 {
        errors.push("manifest_version must be 1".into());
    }
    if !matching(r"^\d+\.\d+\.\d+$", text(&manifest["generator_version"])) {
        errors.push("generator_version must be semver".into());
    }
    if !matching(r"^TASK-\d+$", text(&manifest["task"])) {
        errors.push("manifest task must be TASK-<number>".into());
    }
    for key in ["generator", "profile"] {
        if !nonempty(&manifest[key]) {
            errors.push(format!("manifest {key} must be nonempty"));
        }
    }
    if !["full", "merge"].contains(&text(manifest.get("mode").unwrap_or(&object!("full")))) {
        errors.push("manifest mode must be full or merge".into());
    }
    let files = match strings_map(&manifest["files"]) {
        Ok(files) if !files.is_empty() => files,
        _ => {
            errors.push("manifest files must be non-empty object of hashes".into());
            BTreeMap::new()
        }
    };
    let editions = texts(&manifest["editions"]);
    if editions.is_empty()
        || editions
            .iter()
            .any(|edition| !["claude", "cursor", "codex"].contains(&edition.as_str()))
        || editions.iter().collect::<BTreeSet<_>>().len() != editions.len()
    {
        errors.push("manifest editions must be non-empty unique supported list".into());
    }
    if let Err(error) =
        ownership::validate_decisions(manifest.get("decisions").unwrap_or(&object!({})), &files)
    {
        errors.push(format!("manifest decisions: {error}"));
    }
    for (relative, digest) in &files {
        if relative.ends_with(".py")
            && (relative.starts_with("memory-bank/scripts/")
                || relative.starts_with("project-brain/scripts/"))
        {
            errors.push(format!(
                "manifest includes obsolete interpreted runtime logic: {relative}"
            ));
        }
        if ownership::normalize(relative).is_err() {
            errors.push(format!("manifest invalid relative path: {relative}"));
            continue;
        }
        if !ownership::ownable(relative) {
            errors.push(format!(
                "manifest owns itself, cache or runtime state: {relative}"
            ));
            continue;
        }
        if !hex(digest, 64) {
            errors.push(format!("manifest invalid sha256: {relative}"));
            continue;
        }
        match ownership::confined(target, relative).and_then(|path| fs::read_regular(&path)) {
            Ok(raw) if sha(&raw) == *digest => {}
            Ok(_) => errors.push(format!("manifest sha256 mismatch for {relative}")),
            Err(error) => errors.push(format!(
                "manifest unsafe or missing path {relative}: {error}"
            )),
        }
    }
    if files.contains_key("AGENTS.md") {
        let path = ownership::confined(target, "AGENTS.md")?;
        let content = fs::read_text(&path)?;
        let first = content.lines().next().unwrap_or("");
        let pattern =
            expression(r"^<!-- Generated by .+ v(\d+\.\d+\.\d+) \| (TASK-\d+) \| .+ -->$")?;
        match pattern.captures(first) {
            Some(captures) => {
                if captures[1] != *text(&manifest["generator_version"]) {
                    errors.push("AGENTS stamp version differs from manifest".into());
                }
                if captures[2] != *text(&manifest["task"]) {
                    errors.push("AGENTS stamp task differs from manifest".into());
                }
            }
            None => errors.push("AGENTS first line must be generation version stamp".into()),
        }
    }
    if let Some(version) = generator_version
        && text(&manifest["generator_version"]) != version
    {
        errors.push("manifest generator version differs from generator VERSION".into());
    }
    Ok((manifest, errors))
}
type EditionLayout = (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Vec<&'static str>,
);
fn layout(edition: &str) -> Option<EditionLayout> {
    match edition {
        "claude" => Some((
            ".claude/skills",
            Some(".claude/agents"),
            Some(".claude/commands"),
            vec![".claude"],
        )),
        "cursor" => Some((
            ".cursor/skills",
            Some(".cursor/agents"),
            Some(".cursor/commands"),
            vec![".cursor"],
        )),
        "codex" => Some((".agents/skills", None, None, vec![".agents", ".codex"])),
        _ => None,
    }
}
pub fn frontmatter(path: &Path) -> Result<(Value, String)> {
    let text = fs::read_text(path)?;
    let raw = text
        .strip_prefix("---\n")
        .ok_or_else(|| RuntimeError::invalid("Missing YAML frontmatter"))?;
    let (meta, body) = raw
        .split_once("\n---\n")
        .ok_or_else(|| RuntimeError::invalid("Missing closing frontmatter"))?;
    let mut output = object!({});
    for line in meta.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if key.is_empty() || key.starts_with('#') {
                continue;
            }
            let value = value.trim();
            let decoded = if value.starts_with('[') && value.ends_with(']') {
                object!(
                    value[1..value.len() - 1]
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| value.trim_matches(['\'', '"']))
                        .collect::<Vec<_>>()
                )
            } else {
                object!(value.trim_matches(['\'', '"']))
            };
            output[key] = decoded;
        }
    }
    Ok((output, body.into()))
}
fn collect_skills(prefix: &str, owned: &BTreeMap<String, String>) -> BTreeSet<String> {
    let prefix = format!("{prefix}/");
    owned
        .keys()
        .filter_map(|path| path.strip_prefix(&prefix))
        .filter_map(|tail| tail.strip_suffix("/SKILL.md"))
        .filter(|name| !name.contains('/'))
        .map(str::to_owned)
        .collect()
}
fn check_edition(
    target: &Path,
    edition: &str,
    files: &BTreeMap<String, String>,
    errors: &mut Vec<String>,
) -> Result<()> {
    let (skills, agents, commands, _) =
        layout(edition).ok_or_else(|| RuntimeError::invalid("Unknown generated edition"))?;
    let names = collect_skills(skills, files);
    if !target.join(skills).is_dir() || names.is_empty() {
        errors.push(format!(
            "[{edition}] selected skills tree missing or has no manifest-owned skills"
        ));
    }
    for required in [
        "memory-bank",
        "project-brain",
        "checkpoint",
        "memory",
        "context-save",
        "context-load",
    ] {
        if !names.contains(required) {
            errors.push(format!("[{edition}] runtime guide missing: {required}"));
        }
        if let Some(agents) = agents
            && names.contains(required)
            && !files.contains_key(&format!("{agents}/{required}-agent.md"))
        {
            errors.push(format!(
                "[{edition}] runtime agent wrapper not manifest-owned: {required}"
            ));
        }
        if let Some(commands) = commands
            && names.contains(required)
            && !files.contains_key(&format!("{commands}/{required}.md"))
        {
            errors.push(format!(
                "[{edition}] runtime command wrapper not manifest-owned: {required}"
            ));
        }
    }
    for name in &names {
        let path = ownership::confined(target, &format!("{skills}/{name}/SKILL.md"))?;
        match frontmatter(&path) {
            Ok((metadata, _)) => {
                if !nonempty(&metadata["name"]) || !nonempty(&metadata["description"]) {
                    errors.push(format!(
                        "[{edition}] {name}: missing skill name or description"
                    ));
                }
                for key in ["flow-next", "flow-alternatives", "related"] {
                    let refs = metadata[key]
                        .as_str()
                        .map(|value| vec![value.to_string()])
                        .unwrap_or_else(|| texts(&metadata[key]));
                    for referenced in refs {
                        if !["", "none", "null"].contains(&referenced.as_str())
                            && !names.contains(&referenced)
                        {
                            errors.push(format!(
                                "[{edition}] {name}: unresolved {key} -> {referenced}"
                            ));
                        }
                    }
                }
            }
            Err(error) => errors.push(format!("[{edition}] {name}: {error}")),
        }
    }
    let mut roster = BTreeSet::new();
    if let Some(agents) = agents {
        if !target.join(agents).is_dir() {
            errors.push(format!("[{edition}] agents tree missing"));
        }
        for path in files.keys().filter(|path| {
            path.starts_with(&format!("{agents}/"))
                && path.ends_with(".md")
                && !path.ends_with("/README.md")
        }) {
            match frontmatter(&ownership::confined(target, path)?) {
                Ok((meta, _)) => {
                    if !nonempty(&meta["name"]) {
                        errors.push(format!("[{edition}] {path}: agent name missing"));
                    } else {
                        roster.insert(text(&meta["name"]).to_string());
                    }
                    if nonempty(&meta["invokes"]) && !names.contains(text(&meta["invokes"])) {
                        errors.push(format!("[{edition}] {path}: invokes unknown skill"));
                    }
                }
                Err(error) => errors.push(format!("[{edition}] {path}: {error}")),
            }
        }
    }
    if let Some(commands) = commands {
        if !target.join(commands).is_dir() {
            errors.push(format!("[{edition}] commands tree missing"));
        }
        for path in files.keys().filter(|path| {
            path.starts_with(&format!("{commands}/"))
                && path.ends_with(".md")
                && !path.ends_with("/README.md")
        }) {
            match frontmatter(&ownership::confined(target, path)?) {
                Ok((meta, _)) => {
                    let spawned = text(&meta["spawns"]);
                    if !spawned.is_empty() {
                        let name = spawned.strip_suffix("-agent").unwrap_or(spawned);
                        if !names.contains(name) {
                            errors
                                .push(format!("[{edition}] {path}: spawns missing matching skill"));
                        }
                    }
                }
                Err(error) => errors.push(format!("[{edition}] {path}: {error}")),
            }
        }
    }
    Ok(())
}
fn placeholders(
    target: &Path,
    files: &BTreeMap<String, String>,
    errors: &mut Vec<String>,
) -> Result<()> {
    let patterns = [
        r"\{skill-name\}",
        r"\{NNNN\}",
        r"\bTODO\b",
        r"\bFIXME\b",
        r"\bYYYY-MM-DD\b",
        r"\[target[_ ]name\]",
        r"\bTASK-\{N\}",
        r"\{\{[A-Z_]+\}\}",
    ];
    for path in files.keys() {
        let raw = fs::read_regular(&ownership::confined(target, path)?)?;
        let Ok(text) = std::str::from_utf8(&raw) else {
            continue;
        };
        for pattern in patterns {
            if path == "memory-bank/templates/chunk.md" && pattern == r"\bYYYY-MM-DD\b" {
                continue;
            }
            if matching(pattern, text) {
                errors.push(format!("{path}: unresolved placeholder matching {pattern}"));
            }
        }
    }
    Ok(())
}
fn claude_policy(target: &Path, editions: &[String], errors: &mut Vec<String>) -> Result<()> {
    if !editions.iter().any(|edition| edition == "claude") {
        return Ok(());
    }
    let path = ownership::confined(target, ".claude/CLAUDE.md")?;
    let text = match fs::read_text(&path) {
        Ok(text) => text,
        Err(error) => {
            errors.push(format!("[claude] policy import missing: {error}"));
            return Ok(());
        }
    };
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if !fenced && line.trim() == "@../AGENTS.md" {
            return Ok(());
        }
    }
    errors.push("[claude] CLAUDE.md must import @../AGENTS.md outside a code block".into());
    Ok(())
}
pub fn validate(
    options: &GeneratedOptions,
    services: &dyn NativeGateServices,
) -> Result<Vec<String>> {
    if options.runtime_contract != crate::assets::runtime_contract() {
        return Err(RuntimeError::policy(
            "Generated validation requires the trusted bundled runtime contract",
        ));
    }
    let target = ownership::resolve_target(&options.target)?;
    if !target.is_dir() {
        return Err(RuntimeError::invalid("Target is not a directory"));
    }
    if options.editions.is_empty()
        || options
            .editions
            .iter()
            .any(|edition| layout(edition).is_none())
        || options.plan.is_some() != options.evidence_target.is_some()
    {
        return Err(RuntimeError::invalid(
            "Invalid editions or unpaired skill plan/evidence target",
        ));
    }
    let delivery_root = ownership::resolve_target(
        options
            .delivery_root
            .as_deref()
            .or(options.evidence_target.as_deref())
            .unwrap_or(&target),
    )?;
    let (manifest, mut errors) = validate_manifest(&target, options.generator_version.as_deref())?;
    if !errors.is_empty() {
        return Ok(errors);
    }
    let files = strings_map(&manifest["files"])?;
    for required in [crate::hook_policy::PATH, crate::hook_policy::SCHEMA_PATH] {
        if !files.contains_key(required) {
            errors.push(format!(
                "generated native hook policy resource must be manifest-owned: {required}"
            ));
        }
    }
    let profile_ref = options
        .profile_path
        .as_deref()
        .unwrap_or_else(|| text(&manifest["profile"]));
    if profile_ref != text(&manifest["profile"]) {
        errors.push("hook policy profile reference differs from manifest profile".into());
    }
    match(&options.approved_profile,&options.approved_profile_sha256){(Some(profile),Some(digest))=>if let Err(error)=crate::hook_policy::validate(&target,profile,digest,profile_ref){errors.push(format!("native hook policy: {error}"));},_=>errors.push("generated validation requires actual approved-profile path and its independently recorded review SHA256; self-hash is not approval".into())}
    if !errors.is_empty() {
        return Ok(errors);
    }
    if texts(&manifest["editions"])
        .into_iter()
        .collect::<BTreeSet<_>>()
        != options.editions.iter().cloned().collect()
    {
        errors.push("CLI editions do not match manifest editions".into());
    }
    for edition in ["claude", "cursor", "codex"] {
        let (_, _, _, roots) =
            layout(edition).ok_or_else(|| RuntimeError::invalid("Edition layout unavailable"))?;
        let selected = options.editions.iter().any(|value| value == edition);
        for root in roots {
            if selected && !target.join(root).is_dir() {
                errors.push(format!("[{edition}] selected edition root missing: {root}"));
            }
            if !selected
                && files
                    .keys()
                    .any(|path| path == root || path.starts_with(&format!("{root}/")))
            {
                errors.push(format!(
                    "[{edition}] unselected edition has manifest-owned files"
                ));
            }
        }
    }
    for edition in &options.editions {
        check_edition(&target, edition, &files, &mut errors)?;
        errors.extend(services.validate_host(&target, &delivery_root, edition, &files)?);
    }
    claude_policy(&target, &options.editions, &mut errors)?;
    placeholders(&target, &files, &mut errors)?;
    let artifact = crate::assets::required_artifact()?;
    if !files.contains_key(&artifact) {
        errors.push(format!(
            "Native runtime artifact is not manifest-owned: {artifact}"
        ));
    } else if let Ok(metadata) = std::fs::symlink_metadata(ownership::confined(&target, &artifact)?)
    {
        #[cfg(not(unix))]
        let _ = metadata;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                errors.push("Native runtime artifact is not executable".into());
            }
        }
    }
    let skeleton = texts(&options.runtime_contract["path_contracts"]["required_skeleton"]);
    if skeleton.is_empty() {
        return Err(RuntimeError::invalid(
            "Native runtime contract has no required skeleton",
        ));
    }
    for relative in skeleton {
        let directory = relative.ends_with('/');
        let relative = trailing(&relative, &['/']);
        let path = ownership::confined(&target, relative)?;
        if directory {
            if !path.is_dir() {
                errors.push(format!("runtime skeleton directory missing: {relative}"));
            }
        } else if !files.contains_key(relative) && (ownership::ownable(relative) || !path.is_file())
        {
            errors.push(format!(
                "runtime skeleton file missing or not owned: {relative}"
            ));
        }
    }
    let config_path = target.join("project-brain/config/runtime.json");
    match read_object(&config_path) {
        Ok(config) => {
            if !nonempty(&config["framework"]) {
                errors.push("runtime framework must be non-empty slug".into());
            }
            let canonical = text(&config["canonical_edition"]);
            if canonical.is_empty() || !target.join(canonical).join("skills").is_dir() {
                errors.push("runtime canonical_edition has no generated skills tree".into());
            }
        }
        Err(error) => errors.push(format!("runtime configuration: {error}")),
    }
    let source = options.evidence_target.as_ref().unwrap_or(&target);
    let context = RuntimeContext::new(
        Roots {
            tooling: target.clone(),
            project: ownership::resolve_target(source)?,
            state: target.clone(),
            shared: false,
        },
        "generator",
        "governed",
    )?;
    let bank = BankService::new(context.clone());
    match bank.validate() {
        Ok(problems) => errors.extend(
            problems
                .into_iter()
                .map(|error| format!("memory-bank: {error}")),
        ),
        Err(error) => errors.push(format!("memory-bank: {error}")),
    }
    match BrainRepository::new(context) {
        Ok(brain) => {
            let inventory = brain.inventory(true)?;
            for problem in inventory.problems {
                errors.push(format!(
                    "project-brain source {}: {}",
                    problem.path.display(),
                    problem.error
                ));
            }
            for image in inventory.records {
                if let Err(error) = infra_core::validate_record(&image.record, image.archived) {
                    errors.push(format!("project-brain {}: {error}", image.path.display()));
                }
            }
        }
        Err(error) => errors.push(format!("project-brain: {error}")),
    }
    for arguments in [
        vec!["status".to_string(), "--json".to_string()],
        vec!["validate".to_string()],
    ] {
        let output = services.runtime_command(&target, &arguments, Duration::from_secs(120))?;
        if output.code != 0 {
            let diagnostic = if output.stderr.is_empty() {
                &output.stdout
            } else {
                &output.stderr
            };
            errors.push(format!(
                "native runtime {} exited {}: {}",
                arguments[0],
                output.code,
                String::from_utf8_lossy(diagnostic)
                    .chars()
                    .take(400)
                    .collect::<String>()
            ));
            continue;
        }
        if arguments[0] == "status" {
            match std::str::from_utf8(&output.stdout)
                .map_err(|_| RuntimeError::invalid("Native status is not UTF-8"))
                .and_then(infra_core::domain::json::parse)
            {
                Ok(payload) => errors.extend(validate_readiness(&payload)),
                Err(error) => errors.push(format!("native runtime status JSON: {error}")),
            }
        }
    }
    if let (Some(plan), Some(evidence)) = (&options.plan, &options.evidence_target) {
        for edition in &options.editions {
            let (skills, agents, commands, _) = layout(edition)
                .ok_or_else(|| RuntimeError::invalid("Edition layout unavailable"))?;
            for diagnostic in quality::validate_skills(
                &target.join(skills),
                plan,
                evidence,
                &options.candidate_registry,
                false,
            ) {
                if diagnostic.severity == "error" {
                    errors.push(format!(
                        "semantic[{edition}] {}: {}",
                        diagnostic.code, diagnostic.message
                    ));
                }
            }
            if let Some(agents) = agents {
                for diagnostic in quality::validate_agent_routing(
                    &target.join(agents),
                    plan,
                    evidence,
                    &options.candidate_registry,
                    edition == "claude",
                ) {
                    if diagnostic.severity == "error" {
                        errors.push(format!(
                            "routing[{edition}] {}: {}",
                            diagnostic.code, diagnostic.message
                        ));
                    }
                }
            }
            if let Some(commands) = commands {
                for diagnostic in quality::validate_flow_routing(
                    &target.join(commands),
                    plan,
                    evidence,
                    &options.candidate_registry,
                ) {
                    if diagnostic.severity == "error" {
                        errors.push(format!(
                            "flow-routing[{edition}] {}: {}",
                            diagnostic.code, diagnostic.message
                        ));
                    }
                }
                errors.extend(
                    flows::validate(
                        plan,
                        &target.join(skills).join("SKILL FLOW.md"),
                        &target.join(commands),
                    )
                    .into_iter()
                    .map(|error| format!("flow-contract[{edition}] {error}")),
                );
            }
        }
    }
    Ok(errors)
}
/// Whole ready-edition overlay. Hooks and MCP bind to this staging root.
/// This helper is not a final-target materializer: use scoped seed_assets for
/// generated projects and materialize their wiring against the delivery root.
pub fn seed_runtime(
    source: &Path,
    staging: &Path,
    edition: &str,
    tools: &[String],
) -> Result<Value> {
    infra_tooling::installer::Installer::new(source)?.overlay(staging, edition, tools)
}

pub fn seed_assets(
    assets: &Path,
    staging: &Path,
    framework: &str,
    editions: &[String],
    engine: Option<&Path>,
) -> Result<Value> {
    crate::seed::assets(assets, staging, framework, editions, engine)
}
