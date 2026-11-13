use crate::{
    assets, commands, common::*, content, coverage, flows, generated, gitignore, mutations,
    ownership, publication, quality, reviews,
};
use infra_core::{Result, RuntimeError, fs};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::path::{Path, PathBuf};
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
            let analyzer = match commands::CommandAnalyzer::new(&path(request, "target")?) {
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
                GeneratorOutput::json(
                    &object!(ownership::classify_update(&target, &staging, &manifest)?),
                    0,
                )
            }
            _ => Err(RuntimeError::invalid(
                "Ownership command must be manifest, refresh or classify",
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
