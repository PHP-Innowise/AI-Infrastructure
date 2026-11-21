//! Sealed driver configuration (`skillcache-pilot-driver-v1`) and path/overlap checks.
use infra_core::fs as core_fs;
use infra_harness::{HarnessError, Result, strict_json};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const SCHEMA: &str = "skillcache-pilot-driver-v1";
const CONFIG_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ModelConfig {
    pub provider: String,
    pub model: Option<String>,
    pub permissions: String,
    pub executable: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct TaskConfig {
    pub fixture: PathBuf,
    pub skills: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct DriverConfig {
    pub state_root: PathBuf,
    pub workspaces_root: PathBuf,
    pub engine: PathBuf,
    pub session_timeout_seconds: u64,
    pub models: BTreeMap<String, ModelConfig>,
    pub tasks: BTreeMap<String, TaskConfig>,
}

pub fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

pub fn check_outside(candidate: &Path, forbidden: &[PathBuf], label: &str) -> Result<()> {
    let resolved = core_fs::canonical_or_missing(candidate)?;
    for root in forbidden {
        let forbidden_resolved = core_fs::canonical_or_missing(root)?;
        if overlap(&resolved, &forbidden_resolved) {
            return Err(HarnessError::forbidden(format!(
                "{label} ({}) overlaps forbidden path ({}).",
                resolved.display(),
                forbidden_resolved.display()
            )));
        }
    }
    Ok(())
}

fn require_object<'a>(
    value: &'a Value,
    label: &str,
    known: &[&str],
) -> Result<&'a serde_json::Map<String, Value>> {
    let map = value
        .as_object()
        .ok_or_else(|| HarnessError::invalid(format!("{label} must be a JSON object.")))?;
    if let Some(unknown) = map.keys().find(|key| !known.contains(&key.as_str())) {
        return Err(HarnessError::invalid(format!(
            "{label} has unknown field `{unknown}`."
        )));
    }
    Ok(map)
}

fn require_text<'a>(value: &'a Value, label: &str, limit: usize) -> Result<&'a str> {
    value
        .as_str()
        .filter(|text| !text.trim().is_empty() && text.len() <= limit && !text.contains('\0'))
        .ok_or_else(|| HarnessError::invalid(format!("{label} must be non-empty UTF-8 text.")))
}

fn resolve_input_dir(inputs_root: &Path, raw: &str, label: &str) -> Result<PathBuf> {
    let candidate = Path::new(raw);
    let path = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        let joined = inputs_root.join(candidate);
        core_fs::admit_descendant(inputs_root, &joined)?;
        joined
    };
    if !core_fs::admit_directory(&path)? {
        return Err(HarnessError::invalid(format!(
            "{label} ({}) must be an existing non-symlink directory.",
            path.display()
        )));
    }
    Ok(core_fs::canonicalize(&path)?)
}

fn resolve_engine(explicit: Option<&str>) -> Result<PathBuf> {
    if let Some(raw) = explicit {
        let path = PathBuf::from(raw);
        if !path.is_absolute() || !path.is_file() {
            return Err(HarnessError::invalid(format!(
                "`engine` ({}) must be an existing absolute executable.",
                path.display()
            )));
        }
        return Ok(core_fs::canonicalize(&path)?);
    }
    if let Some(env_bin) = std::env::var_os("INFRA_ENGINE_BIN") {
        let path = PathBuf::from(env_bin);
        if path.is_file() {
            return Ok(core_fs::canonicalize(&path)?);
        }
    }
    let current = std::env::current_exe()?;
    if let Some(parent) = current.parent() {
        let sibling = parent.join(format!("infra-engine{}", std::env::consts::EXE_SUFFIX));
        if sibling.is_file() {
            return Ok(core_fs::canonicalize(&sibling)?);
        }
    }
    if let Some(found) = infra_host::provider::find_executable("infra-engine") {
        return Ok(core_fs::canonicalize(&found)?);
    }
    // Fall back to this binary itself, which serves the SkillCache and sandbox
    // internal subcommands when running without a separate `infra-engine` binary.
    Ok(core_fs::canonicalize(&current)?)
}

pub fn load(inputs_raw: Option<&str>, protected: &[PathBuf]) -> Result<DriverConfig> {
    let inputs_str = inputs_raw.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
        HarnessError::invalid(
            "Pilot driver requires `driver.inputs.root` containing `driver.json`.",
        )
    })?;
    let inputs_path = Path::new(inputs_str);
    if !inputs_path.is_absolute() || !core_fs::admit_directory(inputs_path)? {
        return Err(HarnessError::invalid(
            "`inputs` must be an existing absolute non-symlink directory.",
        ));
    }
    let inputs_root = core_fs::canonicalize(inputs_path)?;
    let config_path = inputs_root.join("driver.json");
    core_fs::admit_descendant(&inputs_root, &config_path)?;
    let bytes = core_fs::read_regular(&config_path)?;
    let doc = strict_json::parse(&bytes, CONFIG_BYTES)?;
    require_object(
        &doc,
        "driver.json",
        &[
            "schema",
            "state_root",
            "workspaces_root",
            "engine",
            "session_timeout_seconds",
            "models",
            "tasks",
        ],
    )?;
    if doc["schema"] != SCHEMA {
        return Err(HarnessError::invalid(format!(
            "driver.json `schema` must be `{SCHEMA}`."
        )));
    }
    let state_raw = PathBuf::from(require_text(&doc["state_root"], "`state_root`", 4096)?);
    if !state_raw.is_absolute() {
        return Err(HarnessError::invalid(
            "`state_root` must be an absolute directory.",
        ));
    }
    let mut forbidden = protected.to_vec();
    forbidden.push(inputs_root.clone());
    check_outside(&state_raw, &forbidden, "`state_root`")?;
    infra_host::util::private_directory(&state_raw)?;
    let state_root = core_fs::canonicalize(&state_raw)?;
    check_outside(&state_root, &forbidden, "`state_root`")?;

    let workspaces_raw = match doc.get("workspaces_root") {
        None | Some(Value::Null) => {
            let parent = state_root.parent().ok_or_else(|| {
                HarnessError::invalid("`state_root` needs a parent for default `workspaces_root`.")
            })?;
            let stem = state_root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("skillcache");
            parent.join(format!("{stem}-workspaces"))
        }
        Some(value) => {
            let path = PathBuf::from(require_text(value, "`workspaces_root`", 4096)?);
            if !path.is_absolute() {
                return Err(HarnessError::invalid(
                    "`workspaces_root` must be an absolute directory.",
                ));
            }
            path
        }
    };
    let mut ws_forbidden = forbidden;
    ws_forbidden.push(state_root.clone());
    check_outside(&workspaces_raw, &ws_forbidden, "`workspaces_root`")?;
    infra_host::util::private_directory(&workspaces_raw)?;
    let workspaces_root = core_fs::canonicalize(&workspaces_raw)?;
    check_outside(&workspaces_root, &ws_forbidden, "`workspaces_root`")?;

    let engine_explicit = match doc.get("engine") {
        None | Some(Value::Null) => None,
        Some(value) => Some(require_text(value, "`engine`", 4096)?),
    };
    let engine = resolve_engine(engine_explicit)?;

    let session_timeout_seconds = match doc.get("session_timeout_seconds") {
        None | Some(Value::Null) => 1800,
        Some(value) => value
            .as_u64()
            .filter(|seconds| (1..=86_400).contains(seconds))
            .ok_or_else(|| {
                HarnessError::invalid("`session_timeout_seconds` must be between 1 and 86400.")
            })?,
    };

    let models_obj = doc["models"]
        .as_object()
        .filter(|map| !map.is_empty())
        .ok_or_else(|| HarnessError::invalid("`models` must be a non-empty object."))?;
    let mut models = BTreeMap::new();
    for (name, spec) in models_obj {
        require_object(
            spec,
            &format!("models.{name}"),
            &["provider", "model", "permissions", "executable"],
        )?;
        let provider = require_text(&spec["provider"], "provider", 32)?;
        if !["claude", "codex", "cursor"].contains(&provider) {
            return Err(HarnessError::invalid(format!(
                "models.{name}.provider `{provider}` must be claude, codex, or cursor."
            )));
        }
        let model = match spec.get("model") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let text = require_text(value, "model", 120)?;
                if text.starts_with('-') || text.chars().any(char::is_control) {
                    return Err(HarnessError::invalid("Invalid model identifier."));
                }
                Some(text.to_owned())
            }
        };
        let permissions = match spec.get("permissions") {
            None | Some(Value::Null) => "bypass".to_owned(),
            Some(value) => {
                let text = require_text(value, "permissions", 32)?;
                if !["auto", "bypass", "manual"].contains(&text) {
                    return Err(HarnessError::invalid(format!(
                        "models.{name}.permissions `{text}` must be auto, bypass, or manual."
                    )));
                }
                text.to_owned()
            }
        };
        let executable = match spec.get("executable") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let raw = PathBuf::from(require_text(value, "executable", 4096)?);
                let resolved = if raw.is_absolute() {
                    raw
                } else {
                    let joined = inputs_root.join(&raw);
                    core_fs::admit_descendant(&inputs_root, &joined)?;
                    joined
                };
                if !resolved.is_file() {
                    return Err(HarnessError::invalid(format!(
                        "models.{name}.executable ({}) does not exist.",
                        resolved.display()
                    )));
                }
                Some(core_fs::canonicalize(&resolved)?)
            }
        };
        models.insert(
            name.clone(),
            ModelConfig {
                provider: provider.to_owned(),
                model,
                permissions,
                executable,
            },
        );
    }

    let tasks_obj = doc["tasks"]
        .as_object()
        .filter(|map| !map.is_empty())
        .ok_or_else(|| HarnessError::invalid("`tasks` must be a non-empty object."))?;
    let mut tasks = BTreeMap::new();
    for (task_id, spec) in tasks_obj {
        require_object(spec, &format!("tasks.{task_id}"), &["fixture", "skills"])?;
        let fixture_raw =
            require_text(&spec["fixture"], &format!("tasks.{task_id}.fixture"), 4096)?;
        let fixture = resolve_input_dir(
            &inputs_root,
            fixture_raw,
            &format!("tasks.{task_id}.fixture"),
        )?;
        let mut skills = Vec::new();
        if let Some(raw_skills) = spec.get("skills").filter(|v| !v.is_null()) {
            let list = raw_skills.as_array().ok_or_else(|| {
                HarnessError::invalid(format!("tasks.{task_id}.skills must be an array."))
            })?;
            for (index, item) in list.iter().enumerate() {
                let skill_raw =
                    require_text(item, &format!("tasks.{task_id}.skills[{index}]"), 4096)?;
                let skill_dir = resolve_input_dir(
                    &inputs_root,
                    skill_raw,
                    &format!("tasks.{task_id}.skills[{index}]"),
                )?;
                if !skill_dir.join("SKILL.md").is_file() {
                    return Err(HarnessError::invalid(format!(
                        "Skill directory {} has no SKILL.md.",
                        skill_dir.display()
                    )));
                }
                skills.push(skill_dir);
            }
        }
        tasks.insert(task_id.clone(), TaskConfig { fixture, skills });
    }

    Ok(DriverConfig {
        state_root,
        workspaces_root,
        engine,
        session_timeout_seconds,
        models,
        tasks,
    })
}
