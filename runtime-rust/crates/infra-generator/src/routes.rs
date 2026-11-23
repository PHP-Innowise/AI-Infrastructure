use crate::{
    assets, commands, common::*, content, coverage, flows, generated, gitignore, mutations,
    ownership, publication, quality, reviews,
};
use infra_core::{Result, RuntimeError, fs};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

const COMMAND_ANALYZER_CACHE_CAP: usize = 64;
const COMMAND_ANALYZER_RACY_NS: i128 = 2_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManifestFileStamp {
    exists: bool,
    regular: bool,
    len: u64,
    mtime_ns: i128,
}

impl ManifestFileStamp {
    fn capture(path: &Path) -> Self {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return Self {
                exists: false,
                regular: true,
                len: 0,
                mtime_ns: 0,
            };
        };
        let regular = !meta.file_type().is_symlink() && meta.is_file();
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|d| i128::try_from(d.as_nanos()).ok())
            .unwrap_or(0);
        Self {
            exists: true,
            regular,
            len: meta.len(),
            mtime_ns,
        }
    }

    fn content_sha(&self, path: &Path) -> Option<String> {
        if !self.exists || !self.regular {
            return None;
        }
        std::fs::read(path).ok().map(|bytes| sha(&bytes))
    }

    fn is_racy(&self, now_ns: i128) -> bool {
        self.exists
            && (!self.regular
                || self.mtime_ns == 0
                || (now_ns - self.mtime_ns).abs() < COMMAND_ANALYZER_RACY_NS)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkspaceManifestPair {
    composer: ManifestFileStamp,
    package: ManifestFileStamp,
}

struct CachedAnalyzerEntry {
    analyzer: Arc<commands::CommandAnalyzer>,
    stamps: WorkspaceManifestPair,
    composer_hash: Option<String>,
    package_hash: Option<String>,
    last_used: u64,
}

#[derive(Default)]
struct CommandAnalyzerCache {
    seq: u64,
    entries: HashMap<PathBuf, CachedAnalyzerEntry>,
}

fn command_analyzer_cache() -> &'static Mutex<CommandAnalyzerCache> {
    static CACHE: OnceLock<Mutex<CommandAnalyzerCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(CommandAnalyzerCache::default()))
}

#[cfg(test)]
fn command_analyzer_build_counts() -> &'static Mutex<HashMap<PathBuf, usize>> {
    static COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn current_unix_ns() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i128::try_from(d.as_nanos()).ok())
        .unwrap_or(0)
}

/// Returns a workspace-stamped, `RACY_NS`-guarded cached [`commands::CommandAnalyzer`]
/// (64-entry LRU) shared by generator routes and `infra-system`.
pub fn cached_command_analyzer(target: &Path) -> Result<Arc<commands::CommandAnalyzer>> {
    let ws_canon = ownership::resolve_target(target)?;
    if !ws_canon.is_dir() {
        return Err(RuntimeError::invalid("target is not a directory"));
    }
    let composer_path = ws_canon.join("composer.json");
    let package_path = ws_canon.join("package.json");
    let current = WorkspaceManifestPair {
        composer: ManifestFileStamp::capture(&composer_path),
        package: ManifestFileStamp::capture(&package_path),
    };
    let now_ns = current_unix_ns();
    if current.composer.regular && current.package.regular {
        let mut cache = command_analyzer_cache()
            .lock()
            .map_err(|_| RuntimeError::unavailable("CommandAnalyzer cache lock failed"))?;
        cache.seq = cache.seq.saturating_add(1);
        let use_seq = cache.seq;
        if let Some(entry) = cache.entries.get_mut(&ws_canon)
            && entry.stamps == current
        {
            let racy_match = (!current.composer.is_racy(now_ns)
                || entry.composer_hash == current.composer.content_sha(&composer_path))
                && (!current.package.is_racy(now_ns)
                    || entry.package_hash == current.package.content_sha(&package_path));
            if racy_match {
                entry.last_used = use_seq;
                return Ok(Arc::clone(&entry.analyzer));
            }
        }
    }

    let built = Arc::new(commands::CommandAnalyzer::new(&ws_canon)?);
    #[cfg(test)]
    if let Ok(mut counts) = command_analyzer_build_counts().lock() {
        *counts.entry(ws_canon.clone()).or_insert(0) += 1;
    }
    let after = WorkspaceManifestPair {
        composer: ManifestFileStamp::capture(&composer_path),
        package: ManifestFileStamp::capture(&package_path),
    };
    if after.composer.regular && after.package.regular {
        let composer_hash = after.composer.content_sha(&composer_path);
        let package_hash = after.package.content_sha(&package_path);
        let mut cache = command_analyzer_cache()
            .lock()
            .map_err(|_| RuntimeError::unavailable("CommandAnalyzer cache lock failed"))?;
        cache.seq = cache.seq.saturating_add(1);
        let use_seq = cache.seq;
        if cache.entries.len() >= COMMAND_ANALYZER_CACHE_CAP
            && !cache.entries.contains_key(&ws_canon)
            && let Some(evict_key) = cache
                .entries
                .iter()
                .min_by_key(|(_, v)| v.last_used)
                .map(|(k, _)| k.clone())
        {
            cache.entries.remove(&evict_key);
        }
        cache.entries.insert(
            ws_canon,
            CachedAnalyzerEntry {
                analyzer: Arc::clone(&built),
                stamps: after,
                composer_hash,
                package_hash,
                last_used: use_seq,
            },
        );
    }
    Ok(built)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeneratorOutput {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
impl GeneratorOutput {
    pub fn json(value: &Value, code: i32) -> Result<Self> {
        let mut bytes = infra_core::domain::json::sorted_pretty(value)?.into_bytes();
        bytes.push(b'\n');
        Ok(Self {
            code,
            stdout: bytes,
            stderr: vec![],
        })
    }
    pub fn text(value: impl AsRef<str>, code: i32) -> Self {
        Self {
            code,
            stdout: value.as_ref().as_bytes().to_vec(),
            stderr: vec![],
        }
    }
    pub fn error(error: impl std::fmt::Display, code: i32) -> Self {
        Self {
            code,
            stdout: vec![],
            stderr: format!("ERROR: {error}\n").into_bytes(),
        }
    }
}
fn field<'a>(request: &'a Value, key: &str) -> &'a Value {
    request
        .get(key)
        .or_else(|| request.get(key.replace('_', "-")))
        .unwrap_or(&Value::Null)
}
fn flag(request: &Value, key: &str) -> bool {
    field(request, key).as_bool() == Some(true)
}
fn optional(request: &Value, key: &str) -> Option<String> {
    field(request, key)
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
fn required(request: &Value, key: &str) -> Result<String> {
    optional(request, key)
        .ok_or_else(|| RuntimeError::invalid(format!("Missing generator argument: {key}")))
}
fn path(request: &Value, key: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(required(request, key)?))
}
fn optional_path(request: &Value, key: &str) -> Option<PathBuf> {
    optional(request, key).map(PathBuf::from)
}
fn list(request: &Value, key: &str) -> Vec<String> {
    if let Some(value) = field(request, key).as_str() {
        value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        texts(field(request, key))
    }
}
fn registry(request: &Value) -> Result<PathBuf> {
    if let Some(path) =
        optional_path(request, "registry").or_else(|| optional_path(request, "candidate_registry"))
    {
        return Ok(path);
    }
    let root = path(request, "generator_root")?;
    Ok(root.join(".agents/skills/skill-forge/references/candidate-registry.json"))
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
struct Violation {
    severity: String,
    code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    message: String,
}

fn classify_generated_violation(message: &str) -> Violation {
    let trimmed = message.trim();
    let (code, path) = if let Some(rest) = trimmed.strip_prefix("manifest sha256 mismatch for ") {
        ("MANIFEST_HASH_MISMATCH".into(), Some(rest.to_owned()))
    } else if let Some(rest) = trimmed.strip_prefix("manifest unsafe or missing path ") {
        (
            "MANIFEST_MISSING_FILE".into(),
            rest.split(':').next().map(str::trim).map(str::to_owned),
        )
    } else if trimmed.starts_with("manifest ")
        || trimmed.starts_with("manifest_version ")
        || trimmed.starts_with("generator_version ")
    {
        (
            "MANIFEST_INVALID".into(),
            Some(".infra-manifest.json".into()),
        )
    } else if trimmed.contains("AGENTS stamp") || trimmed.contains("AGENTS first line") {
        ("MANIFEST_AGENTS_STAMP".into(), Some("AGENTS.md".into()))
    } else if let Some(rest) =
        trimmed.strip_prefix("generated native hook policy resource must be manifest-owned: ")
    {
        ("HOOK_POLICY_UNOWNED".into(), Some(rest.trim().to_owned()))
    } else if trimmed.starts_with("hook policy ")
        || trimmed.starts_with("native hook policy:")
        || trimmed.starts_with("generated validation requires actual approved-profile")
    {
        (
            "HOOK_POLICY_INVALID".into(),
            Some(crate::hook_policy::PATH.into()),
        )
    } else if trimmed == "CLI editions do not match manifest editions" {
        (
            "EDITION_MISMATCH".into(),
            Some(".infra-manifest.json".into()),
        )
    } else if trimmed.starts_with("Native runtime artifact is not manifest-owned: ") {
        (
            "NATIVE_ARTIFACT_UNOWNED".into(),
            trimmed.split(": ").nth(1).map(|p| p.trim().to_owned()),
        )
    } else if trimmed == "Native runtime artifact is not executable" {
        (
            "NATIVE_ARTIFACT_NOT_EXECUTABLE".into(),
            crate::assets::required_artifact().ok(),
        )
    } else if let Some(rest) = trimmed
        .strip_prefix("runtime skeleton directory missing: ")
        .or_else(|| trimmed.strip_prefix("runtime skeleton file missing or not owned: "))
    {
        (
            "RUNTIME_SKELETON_MISSING".into(),
            Some(rest.trim().to_owned()),
        )
    } else if trimmed.starts_with("runtime framework ")
        || trimmed.starts_with("runtime canonical_edition ")
        || trimmed.starts_with("runtime configuration:")
    {
        (
            "RUNTIME_CONFIG_INVALID".into(),
            Some("project-brain/config/runtime.json".into()),
        )
    } else if trimmed.starts_with("memory-bank: ") {
        ("MEMORY_BANK_INVALID".into(), Some("memory-bank".into()))
    } else if let Some(rest) = trimmed.strip_prefix("project-brain source ") {
        (
            "PROJECT_BRAIN_INVALID".into(),
            rest.split(':').next().map(str::trim).map(str::to_owned),
        )
    } else if trimmed.starts_with("project-brain ") || trimmed.starts_with("project-brain:") {
        ("PROJECT_BRAIN_INVALID".into(), Some("project-brain".into()))
    } else if trimmed.starts_with("native runtime ")
        || trimmed.starts_with("status missing automatic_memory")
        || trimmed.contains("automatic_memory")
    {
        ("NATIVE_RUNTIME_STATUS".into(), None)
    } else if trimmed.contains(": unresolved placeholder matching ") {
        (
            "UNRESOLVED_PLACEHOLDER".into(),
            trimmed
                .split(": unresolved placeholder")
                .next()
                .map(str::to_owned),
        )
    } else if let Some((tag, body)) = trimmed.strip_prefix('[').and_then(|s| s.split_once("] ")) {
        if let Some(root) = body.strip_prefix("selected edition root missing: ") {
            ("EDITION_ROOT_MISSING".into(), Some(root.trim().to_owned()))
        } else if body.starts_with("unselected edition") {
            ("EDITION_UNSELECTED_FILES".into(), None)
        } else if body.starts_with("selected skills tree missing") {
            ("EDITION_SKILLS_MISSING".into(), None)
        } else if body.starts_with("runtime guide missing:") {
            ("RUNTIME_GUIDE_MISSING".into(), None)
        } else if body.starts_with("runtime agent wrapper not manifest-owned:") {
            ("AGENT_WRAPPER_UNOWNED".into(), None)
        } else if body.starts_with("runtime command wrapper not manifest-owned:") {
            ("COMMAND_WRAPPER_UNOWNED".into(), None)
        } else if body.starts_with("policy import missing:")
            || body.starts_with("CLAUDE.md must import")
        {
            (
                "CLAUDE_POLICY_IMPORT".into(),
                Some(".claude/CLAUDE.md".into()),
            )
        } else if body.contains("required hook")
            || body.contains("hook wiring")
            || body.contains("exec-only adapter")
        {
            let hook_path = body
                .split([':', ' '])
                .find(|tok| {
                    tok.ends_with(".sh") || tok.ends_with(".json") || tok.ends_with(".toml")
                })
                .map(|p| p.trim_end_matches(':').to_owned());
            ("HOST_HOOK_INVALID".into(), hook_path)
        } else if tag.starts_with("semantic[") {
            let sub = body.split(':').next().unwrap_or("SEMANTIC").trim();
            (format!("SEMANTIC_{sub}"), None)
        } else if tag.starts_with("routing[") {
            let sub = body.split(':').next().unwrap_or("ROUTING").trim();
            (format!("ROUTING_{sub}"), None)
        } else if tag.starts_with("flow-routing[") || tag.starts_with("flow-contract[") {
            ("FLOW_CONTRACT_INVALID".into(), None)
        } else {
            let candidate_path = body
                .split(": ")
                .next()
                .filter(|p| p.contains('/') || p.ends_with(".md"))
                .map(str::to_owned);
            ("EDITION_CONTRACT_INVALID".into(), candidate_path)
        }
    } else if let Some((file_part, rest)) = trimmed.split_once(": ") {
        let code = if rest.contains("not executable") {
            "HOST_HOOK_NOT_EXECUTABLE"
        } else if rest.contains("Memory MCP") {
            "HOST_MCP_INVALID"
        } else {
            "HOST_WIRING_INVALID"
        };
        (code.into(), Some(file_part.trim().to_owned()))
    } else {
        ("GENERATED_VALIDATION_OTHER".into(), None)
    };

    Violation {
        severity: "error".into(),
        code,
        path,
        message: trimmed.to_owned(),
    }
}

fn violation_report(items: Vec<Violation>) -> Value {
    let items: Vec<Violation> = items
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    object!({
        "valid": !items.iter().any(|item| item.severity == "error"),
        "diagnostics": items
    })
}

fn emit_diagnostics(
    items: Vec<Diagnostic>,
    as_json: bool,
    success: &str,
) -> Result<GeneratorOutput> {
    let errors = items.iter().any(|item| item.severity == "error");
    if as_json {
        return GeneratorOutput::json(&report(items), i32::from(errors));
    }
    let mut stdout = String::new();
    let mut stderr = String::new();
    for item in &items {
        let line = format!(
            "{}: {}: {}\n",
            item.severity.to_uppercase(),
            item.code,
            item.message
        );
        if item.severity == "error" {
            stderr.push_str(&line);
        } else {
            stdout.push_str(&line);
        }
    }
    if items.is_empty() {
        stdout.push_str(success);
        stdout.push('\n');
    }
    Ok(GeneratorOutput {
        code: i32::from(errors),
        stdout: stdout.into_bytes(),
        stderr: stderr.into_bytes(),
    })
}
fn emit_errors(errors: Vec<String>, as_json: bool, success: &str) -> Result<GeneratorOutput> {
    if as_json {
        return GeneratorOutput::json(
            &object!({"valid":errors.is_empty(),"errors":errors}),
            i32::from(!errors.is_empty()),
        );
    }
    if errors.is_empty() {
        Ok(GeneratorOutput::text(format!("{success}\n"), 0))
    } else {
        Ok(GeneratorOutput {
            code: 1,
            stdout: vec![],
            stderr: errors
                .iter()
                .map(|error| format!("ERROR: {error}\n"))
                .collect::<String>()
                .into_bytes(),
        })
    }
}
pub fn execute(
    operation: &str,
    request: &Value,
    services: &dyn generated::NativeGateServices,
) -> Result<GeneratorOutput> {
    if !request.is_object() {
        return Err(RuntimeError::invalid("Generator request must be an object"));
    }
    let operation = operation
        .strip_suffix(".py")
        .unwrap_or(operation)
        .replace('_', "-");
    match operation.as_str() {
        "analyze-commands" => {
            let analyzer = match cached_command_analyzer(&path(request, "target")?) {
                Ok(analyzer) => analyzer,
                Err(error) => {
                    return GeneratorOutput::json(&object!({"error":error.to_string()}), 2);
                }
            };
            let commands = list(request, "command");
            let verification = flag(request, "verification");
            let analysis =
                analyzer.analyze_target(&commands, verification, flag(request, "no_scripts"))?;
            let unsafe_command = analysis
                .scripts
                .values()
                .chain(analysis.commands.iter())
                .any(|analysis| !analysis.verification_safe);
            GeneratorOutput::json(
                &serde_json::to_value(analysis)?,
                i32::from(verification && unsafe_command),
            )
        }
        "ownership" | "infra-ownership" => ownership_route(request),
        "merge-gitignore" => gitignore_route(request),
        "publication" | "publish-staging" => publication_route(request),
        "validate-content-review" => emit_diagnostics(
            reviews::content_review(
                &path(request, "publication_plan")?,
                &path(request, "review")?,
            ),
            flag(request, "json"),
            "content review OK",
        ),
        "validate-plan-review" => emit_diagnostics(
            reviews::plan_review(
                &path(request, "plan")?,
                &path(request, "review")?,
                optional_path(request, "registry").as_deref(),
            ),
            flag(request, "json"),
            "plan review OK",
        ),
        "validate-flow-contracts" => emit_errors(
            flows::validate(
                &path(request, "plan")?,
                &path(request, "skill_flow")?,
                &path(request, "commands_dir")?,
            ),
            flag(request, "json"),
            "flow contracts OK",
        ),
        "validate-memory-content" => emit_diagnostics(
            content::memory_content(
                &path(request, "bank")?,
                optional_path(request, "target").as_deref(),
            ),
            flag(request, "json"),
            "memory content OK",
        ),
        "validate-reference-catalogs" => emit_errors(
            content::references(&path(request, "references_dir")?, &list(request, "forbid")),
            flag(request, "json"),
            "reference catalogs OK",
        ),
        "validate-scan-coverage" => emit_diagnostics(
            coverage::validate(
                &path(request, "target")?,
                &path(request, "task_dir")?,
                optional_path(request, "plan").as_deref(),
            ),
            flag(request, "json"),
            "discovery coverage OK",
        ),
        "validate-plan-mutations" => {
            let diagnostics = mutations::validate(
                &path(request, "plan")?,
                &path(request, "target")?,
                &registry(request)?,
                &assets::runtime_contract(),
            );
            let exercised = 9 - diagnostics
                .iter()
                .filter(|item| item.code == "MUTATION_NOT_APPLICABLE")
                .count();
            let success = format!("plan mutations OK ({exercised} of 9 exercised)");
            let mut output = emit_diagnostics(diagnostics, flag(request, "json"), &success)?;
            if !flag(request, "json")
                && output.code == 0
                && !output.stdout.ends_with(success.as_bytes())
            {
                output
                    .stdout
                    .extend_from_slice(format!("{success}\n").as_bytes());
            }
            Ok(output)
        }
        "validate-skill-quality" => quality_route(request),
        "partition-pre-generation-batches" => {
            let target = ownership::resolve_target(&path(request, "target")?)?;
            let manifest = ownership::load_manifest(&target)?;
            let changed = field(request, "changed_paths")
                .as_array()
                .map(|_| list(request, "changed_paths"));
            GeneratorOutput::json(
                &coverage::partition_pre_generation_batches(
                    &target,
                    &manifest,
                    changed.as_deref(),
                )?,
                0,
            )
        }
        "partition-update-slices" => {
            let target = ownership::resolve_target(&path(request, "target")?)?;
            let manifest = ownership::load_manifest(&target)?;
            let staging = ownership::resolve_target(&path(request, "staging")?)?;
            if !staging.is_dir() {
                return Err(RuntimeError::not_found("Staging directory not found"));
            }
            GeneratorOutput::json(
                &coverage::partition_update_slices(&target, &staging, &manifest)?,
                0,
            )
        }
        "validate-generated" => {
            let target = path(request, "target")?;
            let options = generated::GeneratedOptions {
                target,
                delivery_root: optional_path(request, "delivery_root"),
                approved_profile: optional_path(request, "approved_profile"),
                approved_profile_sha256: optional(request, "approved_profile_sha256"),
                profile_path: optional(request, "profile_path"),
                editions: optional(request, "editions")
                    .map(|value| {
                        value
                            .split(',')
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string)
                            .collect()
                    })
                    .or_else(|| {
                        field(request, "editions")
                            .as_array()
                            .map(|_| list(request, "editions"))
                    })
                    .unwrap_or_else(|| vec!["claude".into(), "cursor".into(), "codex".into()]),
                plan: optional_path(request, "skill_plan"),
                evidence_target: optional_path(request, "evidence_target"),
                candidate_registry: registry(request)?,
                generator_version: optional(request, "generator_version"),
                runtime_contract: assets::runtime_contract(),
            };
            let errors = generated::validate(&options, services)?;
            let want_stamp = field(request, "stamp_manifest").as_bool() == Some(true)
                || flag(request, "stamp_manifest");
            if errors.is_empty()
                && want_stamp
                && !flag(request, "read_only")
                && options.target.join(ownership::MANIFEST).is_file()
            {
                ownership::stamp_manifest_if_needed(
                    &options.target,
                    optional(request, "stamp_slice").as_deref(),
                )?;
            }
            if flag(request, "json") {
                let items: Vec<Violation> = errors
                    .iter()
                    .map(|message| classify_generated_violation(message))
                    .collect();
                let count = errors.len();
                let mut payload = violation_report(items);
                payload["error_count"] = object!(count);
                payload["errors"] = object!(errors);
                return GeneratorOutput::json(&payload, i32::from(count > 0));
            }
            if errors.is_empty() {
                Ok(GeneratorOutput::text("generated accelerator OK\n", 0))
            } else {
                let count = errors.len();
                let mut output = emit_errors(errors, false, "generated accelerator OK")?;
                output
                    .stderr
                    .extend_from_slice(format!("\n{count} problem(s) found.\n").as_bytes());
                Ok(output)
            }
        }
        "inspect-hook-policy-profile" => GeneratorOutput::json(
            &crate::hook_policy::inspect(
                &path(request, "profile")?,
                &required(request, "profile_path")?,
            )?,
            0,
        ),
        "materialize-hook-policy" => GeneratorOutput::json(
            &crate::hook_policy::materialize(
                &path(request, "profile")?,
                &required(request, "approved_profile_sha256")?,
                &required(request, "profile_path")?,
                &path(request, "staging")?,
            )?,
            0,
        ),
        "validate-hook-policy" => GeneratorOutput::json(
            &crate::hook_policy::validate(
                &path(request, "target")?,
                &path(request, "profile")?,
                &required(request, "approved_profile_sha256")?,
                &required(request, "profile_path")?,
            )?,
            0,
        ),
        "hook-policy-schema" => GeneratorOutput::json(&crate::hook_policy::schema(), 0),
        "seed-assets" => GeneratorOutput::json(
            &generated::seed_assets(
                &path(request, "assets")?,
                &path(request, "staging")?,
                &required(request, "framework")?,
                &list(request, "editions"),
                optional_path(request, "engine").as_deref(),
            )?,
            0,
        ),
        "seed-runtime" => {
            let staging = path(request, "staging")?;
            if let Some(delivery) = optional_path(request, "delivery_root")
                && ownership::resolve_target(&delivery)? != ownership::resolve_target(&staging)?
            {
                return Err(RuntimeError::invalid(
                    "seed-runtime binds ready-edition hooks and MCP to --staging; a different --delivery-root requires explicit final-target materialization. Use seed-assets for scoped Creator staging.",
                ));
            }
            GeneratorOutput::json(
                &generated::seed_runtime(
                    &path(request, "source")?,
                    &staging,
                    &required(request, "edition")?,
                    &list(request, "tools"),
                )?,
                0,
            )
        }
        "runtime-contract" => GeneratorOutput::json(&assets::runtime_contract(), 0),
        "mirror-rules" => GeneratorOutput::json(&assets::mirror_rules(), 0),
        _ => Err(RuntimeError::invalid(format!(
            "Unknown native generator operation: {operation}"
        ))),
    }
}
fn ownership_route(request: &Value) -> Result<GeneratorOutput> {
    let result = (|| -> Result<GeneratorOutput> {
        let target = ownership::resolve_target(&path(request, "target")?)?;
        if !target.is_dir() {
            return Err(RuntimeError::not_found("Target directory not found"));
        }
        match required(request, "command")?.as_str() {
            "refresh" => {
                let report = ownership::refresh_manifest(&target)?;
                let missing = texts(&report["missing"]);
                let code = i32::from(!missing.is_empty());
                if flag(request, "json") {
                    GeneratorOutput::json(&report, code)
                } else {
                    Ok(GeneratorOutput::text(
                        format!(
                            "refreshed {} hash(es){}\n",
                            report["listed_count"],
                            if missing.is_empty() {
                                String::new()
                            } else {
                                format!(
                                    "; STILL MISSING: {}",
                                    infra_core::domain::json::canonical(&object!(missing))?
                                )
                            }
                        ),
                        code,
                    ))
                }
            }
            "manifest" => {
                let plan = ownership::read_plan(&path(request, "write_plan")?)?;
                let decisions = optional_path(request, "decisions")
                    .map(|path| read_object(&path))
                    .transpose()?
                    .unwrap_or(object!({}));
                let source_target = optional_path(request, "source_target");
                let source_map = optional_path(request, "source_map");
                if source_target.is_some() != source_map.is_some() {
                    return Err(RuntimeError::invalid(
                        "--source-target and --source-map must be supplied together",
                    ));
                }
                let hashes = match (source_target, source_map) {
                    (Some(source), Some(map)) => Some(ownership::hash_sources(
                        &target,
                        &plan,
                        &ownership::resolve_target(&source)?,
                        &read_object(&map)?,
                        &decisions,
                    )?),
                    _ => None,
                };
                let manifest = ownership::build_manifest(
                    &target,
                    &plan,
                    &ownership::ManifestRequest {
                        generator: optional(request, "generator")
                            .unwrap_or_else(|| "Infrastructure-Creator".into()),
                        version: required(request, "version")?,
                        task: required(request, "task")?,
                        profile: required(request, "profile")?,
                        editions: list(request, "editions"),
                        mode: required(request, "mode")?,
                        decisions,
                        generated_at: optional(request, "generated_at"),
                    },
                    hashes,
                )?;
                let path = ownership::write_manifest(&target, &manifest)?;
                Ok(GeneratorOutput::text(
                    format!(
                        "wrote {} ({} files)\n",
                        path.display(),
                        manifest["files"]
                            .as_object()
                            .map(|files| files.len())
                            .unwrap_or(0)
                    ),
                    0,
                ))
            }
            "classify" => {
                let manifest = ownership::load_manifest(&target)?;
                let staging = ownership::resolve_target(&path(request, "staging")?)?;
                if !staging.is_dir() {
                    return Err(RuntimeError::not_found("Staging directory not found"));
                }
                if flag(request, "partition") {
                    GeneratorOutput::json(
                        &coverage::partition_update_slices(&target, &staging, &manifest)?,
                        0,
                    )
                } else {
                    GeneratorOutput::json(
                        &object!(ownership::classify_update(&target, &staging, &manifest)?),
                        0,
                    )
                }
            }
            "partition" => {
                let manifest = ownership::load_manifest(&target)?;
                let staging = ownership::resolve_target(&path(request, "staging")?)?;
                if !staging.is_dir() {
                    return Err(RuntimeError::not_found("Staging directory not found"));
                }
                GeneratorOutput::json(
                    &coverage::partition_update_slices(&target, &staging, &manifest)?,
                    0,
                )
            }
            _ => Err(RuntimeError::invalid(
                "Ownership command must be manifest, refresh, classify or partition",
            )),
        }
    })();
    Ok(result.unwrap_or_else(|error| GeneratorOutput::error(error, 2)))
}
fn gitignore_route(request: &Value) -> Result<GeneratorOutput> {
    let result = (|| -> Result<GeneratorOutput> {
        let requirements = list(request, "requirements");
        let documents = requirements
            .iter()
            .map(|path| fs::read_regular(Path::new(path)))
            .collect::<Result<Vec<_>>>()?;
        let required = gitignore::combine(&documents)?;
        let original = optional_path(request, "existing")
            .map(|path| fs::read_regular(&path))
            .transpose()?;
        let merged = gitignore::merge(original.as_deref(), &required)?;
        let metadata = infra_core::domain::json::sorted_pretty(&merged.metadata)? + "\n";
        let metadata_path = optional_path(request, "metadata");
        if let Some(path) = &metadata_path {
            fs::atomic_write(path, metadata.as_bytes())?;
        }
        let output = optional(request, "output").unwrap_or_else(|| "-".into());
        if output == "-" {
            Ok(GeneratorOutput {
                code: 0,
                stdout: merged.staged_bytes,
                stderr: if metadata_path.is_none() {
                    metadata.into_bytes()
                } else {
                    vec![]
                },
            })
        } else {
            fs::atomic_write(Path::new(&output), &merged.staged_bytes)?;
            Ok(GeneratorOutput {
                code: 0,
                stdout: if metadata_path.is_none() {
                    metadata.into_bytes()
                } else {
                    vec![]
                },
                stderr: vec![],
            })
        }
    })();
    Ok(result.unwrap_or_else(|error| GeneratorOutput::error(error, 2)))
}
fn publication_route(request: &Value) -> Result<GeneratorOutput> {
    let result = (|| -> Result<GeneratorOutput> {
        let target = ownership::resolve_target(&path(request, "target")?)?;
        if !target.is_dir() {
            return Err(RuntimeError::not_found("Target directory not found"));
        }
        let command = required(request, "command")?;
        if command == "rollback" {
            publication::restore(
                &target,
                &ownership::resolve_target(&path(request, "journal")?)?,
                None,
            )?;
            return Ok(GeneratorOutput::text("publication rolled back\n", 0));
        }
        let paths = publication::planned_paths(&path(request, "publication_plan")?)?;
        let removal_path = optional_path(request, "removal_plan");
        let removals = publication::removal_paths(removal_path.as_deref())?;
        let watch = optional_path(request, "watch_plan")
            .map(|path| ownership::read_plan(&path))
            .transpose()?
            .unwrap_or_default();
        if paths.iter().any(|path| removals.contains(path))
            || watch
                .iter()
                .any(|path| paths.contains(path) || removals.contains(path))
        {
            return Err(RuntimeError::policy(
                "Watch, publication and removal plans must not overlap",
            ));
        }
        match command.as_str() {
            "snapshot" => {
                let all = paths.iter().chain(&removals).cloned().collect::<Vec<_>>();
                let snapshot = publication::build_snapshot(&target, &all, &watch)?;
                write_json(&path(request, "output")?, &snapshot)?;
                Ok(GeneratorOutput::text(
                    format!(
                        "snapshot recorded for {} publication/removal and {} baseline-only path(s)\n",
                        all.len(),
                        watch.len()
                    ),
                    0,
                ))
            }
            "publish" => {
                let staging = ownership::resolve_target(&path(request, "staging")?)?;
                if !staging.is_dir() {
                    return Err(RuntimeError::not_found("Staging directory not found"));
                }
                let baseline = read_object(&path(request, "baseline")?)?;
                let journal = ownership::resolve_target(&path(request, "journal")?)?;
                publication::publish(
                    &target, &staging, &paths, &baseline, &journal, &removals, &watch,
                )?;
                Ok(GeneratorOutput::text(
                    format!(
                        "published {} and removed {} path(s); keep journal until verification\n",
                        paths.len(),
                        removals.len()
                    ),
                    0,
                ))
            }
            _ => Err(RuntimeError::invalid(
                "Publication command must be snapshot, publish or rollback",
            )),
        }
    })();
    Ok(result.unwrap_or_else(|error| GeneratorOutput::error(error, 1)))
}
fn quality_route(request: &Value) -> Result<GeneratorOutput> {
    let plan = path(request, "plan")?;
    let target = path(request, "target")?;
    let registry = registry(request)?;
    let plan_only = flag(request, "plan_only");
    let partial = flag(request, "allow_partial_skills");
    if plan_only && partial {
        return Err(RuntimeError::invalid(
            "plan-only and allow-partial-skills are mutually exclusive",
        ));
    }
    let mut items = if plan_only {
        quality::validate_plan(&plan, &target, &registry)
    } else {
        quality::validate_skills(
            &path(request, "skills_dir")?,
            &plan,
            &target,
            &registry,
            partial,
        )
    };
    if let Some(agents) = optional_path(request, "agents_dir") {
        items.extend(quality::validate_agent_routing(
            &agents, &plan, &target, &registry, true,
        ));
    }
    let mut baseline = None;
    if let Some(path) = optional_path(request, "baseline_plan") {
        let (additional, report) = quality::compare_coverage_baseline(&plan, &path);
        items.extend(additional);
        baseline = Some(report);
    }
    let items = sorted(items);
    let errors = items.iter().filter(|item| item.severity == "error").count();
    let warnings = items.len() - errors;
    let classes = quality::skill_class_split(&plan);
    if flag(request, "json") {
        let mut report = object!({"valid":errors==0,"error_count":errors,"warning_count":warnings,"diagnostics":items,"skill_classes":classes});
        if let Some(baseline) = baseline {
            report["coverage_baseline"] = baseline;
        }
        GeneratorOutput::json(&report, i32::from(errors > 0))
    } else {
        let mut stdout = String::new();
        let mut stderr = String::new();
        for item in items {
            let line = format!(
                "{} [{}] {}\n",
                item.severity.to_uppercase(),
                item.code,
                item.message
            );
            if item.severity == "error" {
                stderr.push_str(&line);
            } else {
                stdout.push_str(&line);
            }
        }
        stdout.push_str(&format!(
            "skill quality: {} ({errors} errors, {warnings} warnings)\n",
            if errors == 0 { "PASS" } else { "FAIL" }
        ));
        let project = classes.get("project").cloned().unwrap_or_default();
        let runtime = classes.get("runtime_fixed").cloned().unwrap_or_default();
        stdout.push_str(&format!(
            "skill inventory: {} project skills, {} runtime guides (project: {}; runtime: {})\n",
            project.len(),
            runtime.len(),
            if project.is_empty() {
                "none".into()
            } else {
                project.join(", ")
            },
            if runtime.is_empty() {
                "none".into()
            } else {
                runtime.join(", ")
            }
        ));
        if let Some(baseline) = baseline {
            stdout.push_str(&format!(
                "coverage baseline: {}\n",
                infra_core::domain::json::canonical(&baseline)?
            ));
        }
        Ok(GeneratorOutput {
            code: i32::from(errors > 0),
            stdout: stdout.into_bytes(),
            stderr: stderr.into_bytes(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "infra-generator-routes-test-{}-{seq}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct DummyServices;
    impl generated::NativeGateServices for DummyServices {
        fn validate_host(
            &self,
            _: &Path,
            _: &Path,
            _: &str,
            _: &BTreeMap<String, String>,
        ) -> Result<Vec<String>> {
            Ok(vec![])
        }
        fn runtime_command(
            &self,
            _: &Path,
            _: &[String],
            _: std::time::Duration,
        ) -> Result<generated::NativeOutput> {
            Ok(generated::NativeOutput {
                code: 0,
                stdout: b"{}".to_vec(),
                stderr: vec![],
            })
        }
    }

    #[test]
    fn validate_generated_respects_json_flag_with_coded_diagnostics() {
        let dir = TempDir::new();
        let registry = dir.0.join("registry.json");
        std::fs::write(&registry, b"{}").unwrap();
        let manifest = ownership::build_manifest(
            &dir.0,
            &[],
            &ownership::ManifestRequest {
                generator: "Infrastructure-Creator".into(),
                version: "1.0.0".into(),
                task: "TASK-001".into(),
                profile: "minimal".into(),
                editions: vec!["codex".into()],
                mode: "full".into(),
                decisions: object!({}),
                generated_at: None,
            },
            None,
        )
        .unwrap();
        ownership::write_manifest(&dir.0, &manifest).unwrap();
        let out = execute(
            "validate-generated",
            &object!({
                "target": &dir.0,
                "editions": ["codex"],
                "registry": &registry,
                "json": true,
            }),
            &DummyServices,
        )
        .unwrap();
        assert_ne!(out.code, 0);
        let payload: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(payload["valid"], false);
        assert!(payload["error_count"].as_u64().unwrap_or(0) > 0);
        assert!(
            payload["diagnostics"].as_array().is_some_and(|items| {
                !items.is_empty()
                    && items.iter().all(|item| {
                        item["code"]
                            .as_str()
                            .is_some_and(|c| !c.is_empty() && c != "GENERATED_VALIDATION")
                            && item["message"].as_str().is_some_and(|m| !m.is_empty())
                    })
            }),
            "{payload}"
        );
        let placeholder =
            classify_generated_violation(r"AGENTS.md: unresolved placeholder matching \bTODO\b");
        assert_eq!(placeholder.code, "UNRESOLVED_PLACEHOLDER");
        assert_eq!(placeholder.path.as_deref(), Some("AGENTS.md"));
        let skeleton = classify_generated_violation(
            "runtime skeleton file missing or not owned: specs/MANIFEST.md",
        );
        assert_eq!(skeleton.code, "RUNTIME_SKELETON_MISSING");
        assert_eq!(skeleton.path.as_deref(), Some("specs/MANIFEST.md"));
        let after = ownership::load_manifest(&dir.0).unwrap();
        assert!(after.get("validation_stamps").is_none());
    }

    #[test]
    fn cached_command_analyzer_reuses_entry_and_invalidates_on_racy_change() {
        let dir = TempDir::new();
        let ws = dir.0.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let pkg = ws.join("package.json");
        std::fs::write(&pkg, r#"{"scripts":{"test":"phpunit"}}"#).unwrap();
        let ws_canon = ownership::resolve_target(&ws).unwrap();

        let _ = cached_command_analyzer(&ws).unwrap();
        let first_count = command_analyzer_build_counts()
            .lock()
            .unwrap()
            .get(&ws_canon)
            .copied()
            .unwrap_or(0);
        assert_eq!(first_count, 1);

        let out = execute(
            "analyze-commands",
            &object!({"target": &ws, "command": ["npm run test"], "verification": true}),
            &DummyServices,
        )
        .unwrap();
        assert_eq!(out.code, 0);
        let parsed: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            parsed["scripts"]["package:test"]["expanded_commands"][0],
            "phpunit"
        );
        let reused_count = command_analyzer_build_counts()
            .lock()
            .unwrap()
            .get(&ws_canon)
            .copied()
            .unwrap_or(0);
        assert_eq!(reused_count, 1);

        // Mutate package.json within the 2s RACY_NS window (same size or same timestamp bucket).
        std::fs::write(&pkg, r#"{"scripts":{"test":"pest   "}}"#).unwrap();
        let rebuilt = cached_command_analyzer(&ws).unwrap();
        let third_count = command_analyzer_build_counts()
            .lock()
            .unwrap()
            .get(&ws_canon)
            .copied()
            .unwrap_or(0);
        assert_eq!(third_count, 2);
        let analyzed = rebuilt.analyze("npm run test", true, None).unwrap();
        assert_eq!(analyzed.expanded_commands, vec!["pest   "]);
    }

    #[test]
    fn partition_update_slices_route_and_ownership_partition_separate_stamped_slices() {
        let dir = TempDir::new();
        let target = dir.0.join("target");
        let staging = dir.0.join("staging");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(target.join("a.md"), b"same-content\n").unwrap();
        std::fs::write(target.join("b.md"), b"old-content\n").unwrap();
        std::fs::write(staging.join("a.md"), b"same-content\n").unwrap();
        std::fs::write(staging.join("b.md"), b"new-content\n").unwrap();

        let owned = vec!["a.md".to_string(), "b.md".to_string()];
        let manifest = ownership::build_manifest(
            &target,
            &owned,
            &ownership::ManifestRequest {
                generator: "Infrastructure-Creator".into(),
                version: "1.0.0".into(),
                task: "TASK-001".into(),
                profile: "minimal".into(),
                editions: vec!["codex".into()],
                mode: "merge".into(),
                decisions: object!({}),
                generated_at: None,
            },
            None,
        )
        .unwrap();
        ownership::write_manifest(&target, &manifest).unwrap();
        assert!(ownership::stamp_manifest_if_needed(&target, Some("wave3")).unwrap());
        assert!(!ownership::stamp_manifest_if_needed(&target, Some("wave3")).unwrap());

        let out = execute(
            "partition-update-slices",
            &object!({"target": &target, "staging": &staging}),
            &DummyServices,
        )
        .unwrap();
        assert_eq!(out.code, 0);
        let partitioned: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(partitioned["skipped_unchanged"], object!(["a.md"]));
        assert_eq!(partitioned["pending_slices"], object!(["b.md"]));

        let via_ownership = execute(
            "ownership",
            &object!({
                "command": "classify",
                "target": &target,
                "staging": &staging,
                "partition": true,
            }),
            &DummyServices,
        )
        .unwrap();
        assert_eq!(via_ownership.code, 0);
        let parsed: Value = serde_json::from_slice(&via_ownership.stdout).unwrap();
        assert_eq!(parsed, partitioned);
    }

    #[test]
    fn quality_runtime_and_contracts_reuse_cached_command_analyzer() {
        let dir = TempDir::new();
        let ws = dir.0.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let pkg = ws.join("package.json");
        std::fs::write(&pkg, r#"{"scripts":{"test":"phpunit"}}"#).unwrap();
        let ws_canon = ownership::resolve_target(&ws).unwrap();

        let before = command_analyzer_build_counts()
            .lock()
            .unwrap()
            .get(&ws_canon)
            .copied()
            .unwrap_or(0);

        let mut d1 = Vec::new();
        quality::test_runtime_body_commands(
            "test-skill",
            "Run:\n```bash\nnpm run test\n```\n",
            &ws,
            &mut d1,
        );
        let after_first = command_analyzer_build_counts()
            .lock()
            .unwrap()
            .get(&ws_canon)
            .copied()
            .unwrap_or(0);
        assert_eq!(after_first, before + 1);

        let mut d2 = Vec::new();
        quality::test_runtime_body_commands(
            "test-skill-2",
            "Run:\n```bash\nnpm run test\n```\n",
            &ws,
            &mut d2,
        );
        let mut d3 = Vec::new();
        let skill_val = object!({
            "kind": "workflow",
            "capability": {"mode": "check", "summary": "Verify suite"},
            "ownership": [],
            "path_contracts": [],
            "procedures": [],
            "required_procedure_roles": [],
            "verification": [{
                "id": "v1",
                "mode": "command",
                "instruction": "Run test suite",
                "command": "npm run test",
                "prerequisites": [],
                "safe_scope": "repo",
                "mutation_class": "none",
                "network_class": "none",
                "expected_result": "exit 0",
                "failure_result": "non-zero",
                "skip_condition": "never",
                "skip_reporting": "fail"
            }]
        });
        let ledger = quality::TestLedger::default();
        quality::test_contracts_skill("1.6", "test-skill", &skill_val, &ledger, &ws, &mut d3);

        let after_reused = command_analyzer_build_counts()
            .lock()
            .unwrap()
            .get(&ws_canon)
            .copied()
            .unwrap_or(0);
        assert_eq!(after_reused, before + 1);
    }
}
