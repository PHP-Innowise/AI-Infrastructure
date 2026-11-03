//! Native repository-aware code2prompt orchestration; no model calls.
use crate::{common, process, temp::Temporary};
use infra_core::{Result, RuntimeError, bank::publication::PublicationFiles, fs as safe};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::Path,
    time::Duration,
};
const ALWAYS: &[&str] = &[
    ".git/**",
    "**/.git/**",
    "**/__pycache__/**",
    "**/*.pyc",
    ".venv/**",
    "**/.venv/**",
    ".worktrees/**",
    ".superpowers/**",
    "**/memory-bank/local/**",
    "**/project-brain/local/**",
    "**/*.db",
    "**/*.sqlite",
    "**/*.sqlite3",
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/id_rsa*",
    "runtime-rust/target/**",
    "**/*.exe",
    "**/*.so",
    "**/*.dll",
    "**/*.dylib",
    "**/*.rlib",
    "**/*.rmeta",
    "**/bin/infra-engine",
];
fn strings(value: &Value, key: &str) -> Result<Vec<String>> {
    match value.get(key) {
        None => Ok(vec![]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(String::from)
                    .ok_or_else(|| RuntimeError::invalid(format!("{key} must contain text")))
            })
            .collect(),
        _ => Err(RuntimeError::invalid(format!("{key} must be an array"))),
    }
}
fn flag(request: &Value, key: &str) -> Result<bool> {
    match request.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(RuntimeError::invalid(format!("{key} must be boolean"))),
    }
}
fn scopes() -> Value {
    json!({
     "edition":{"edition":true,"description":"One edition canonical policy,skills,schemas and durable docs"},
     "skills":{"edition":true,"description":"One edition canonical skills"},
     "core":{"edition":true,"description":"Native shared runtime source plus edition data contracts"},
     "hooks":{"edition":true,"description":"Host hook wiring and direct native dispatch definitions"},
     "tooling":{"edition":false,"description":"Native tooling,reference tests,CI and distribution metadata"},
     "docs":{"edition":false,"description":"Repository documentation"},
     "harness":{"edition":false,"description":"Native orchestration and retained reference sources"},
     "diff":{"edition":false,"description":"Changed files against a merge base"},
     "custom":{"edition":false,"description":"Explicit include patterns with safety exclusions"}
    })
}
fn diff(root: &Path, base: &str) -> Result<Vec<String>> {
    let mut merge = common::git(root, &["merge-base", base, "HEAD"], None)?;
    if merge.code != 0 && base == "origin/main" {
        merge = common::git(root, &["merge-base", "main", "HEAD"], None)?;
    }
    if merge.code != 0 {
        return Err(RuntimeError::unavailable(
            "Cannot resolve collection merge base",
        ));
    }
    let merge = common::text(merge.stdout)?.trim().to_string();
    let raw = common::git_ok(
        root,
        &["diff", "--name-only", "--diff-filter=d", "-z", &merge, "--"],
        None,
    )?;
    let mut paths = Vec::new();
    for raw in raw.split(|byte| *byte == 0).filter(|raw| !raw.is_empty()) {
        let path = std::str::from_utf8(raw)
            .map_err(|_| RuntimeError::invalid("Changed collection path is not UTF-8"))?;
        common::relative(path)?;
        paths.push(format!("./{path}"));
    }
    if paths.is_empty() {
        return Err(RuntimeError::not_found(
            "No files changed against collection base",
        ));
    }
    Ok(paths)
}
fn invoke(
    binary: &OsStr,
    target: &Path,
    includes: &[String],
    excludes: &[String],
    format: &str,
    encoding: &str,
    sandbox: &Path,
) -> Result<Vec<u8>> {
    let mut arguments = vec![
        target.as_os_str().into(),
        "--hidden".into(),
        "--quiet".into(),
        "--output-format".into(),
        format.into(),
        "--output-file".into(),
        "-".into(),
        "--encoding".into(),
        encoding.into(),
    ];
    for pattern in includes {
        arguments.extend([OsString::from("--include"), pattern.into()]);
    }
    for pattern in excludes {
        arguments.extend([OsString::from("--exclude"), pattern.into()]);
    }
    let environment = BTreeMap::from([
        (
            OsString::from("XDG_CONFIG_HOME"),
            Some(sandbox.as_os_str().into()),
        ),
        (OsString::from("NO_COLOR"), None),
    ]);
    let output = process::run_with_env(
        binary,
        &arguments,
        sandbox,
        None,
        Duration::from_secs(120),
        256 * 1024 * 1024,
        &environment,
    )?;
    if output.code != 0 {
        return Err(RuntimeError::unavailable(format!(
            "code2prompt exited {}",
            output.code
        )));
    }
    Ok(output.stdout)
}
pub fn execute(request: &Value) -> Result<Value> {
    if flag(request, "list")? || request.get("scope").is_none() {
        return Ok(json!({"scopes":scopes(),"always_exclude":ALWAYS}));
    }
    let root = std::fs::canonicalize(request["root"].as_str().unwrap_or("."))?;
    let scope = request["scope"]
        .as_str()
        .ok_or_else(|| RuntimeError::invalid("Collection scope must be text"))?;
    let catalog = scopes();
    let definition = catalog
        .get(scope)
        .ok_or_else(|| RuntimeError::invalid("Unknown collection scope"))?;
    let needs_edition = definition["edition"].as_bool().unwrap_or(false);
    let edition = if needs_edition {
        Some(common::edition(request["edition"].as_str().ok_or_else(
            || RuntimeError::invalid("Collection scope requires edition"),
        )?)?)
    } else {
        if request.get("edition").is_some_and(|value| !value.is_null()) {
            return Err(RuntimeError::invalid(
                "Collection scope does not accept edition",
            ));
        }
        None
    };
    let target = if scope == "core" {
        root.clone()
    } else {
        edition
            .map(|edition| root.join(edition))
            .unwrap_or_else(|| root.clone())
    };
    let mut includes = match scope {
        "edition" => vec![
            ".agents/skills/**",
            "memory-bank/**",
            "project-brain/**",
            "specs/**",
            "examples/**",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        "skills" => vec![".agents/skills/**".into()],
        "core" => vec![
            "runtime-rust/crates/infra-core/**".into(),
            format!("{}/memory-bank/config/**", edition.unwrap_or("PHP Core")),
            format!("{}/project-brain/schemas/**", edition.unwrap_or("PHP Core")),
            format!(
                "{}/project-brain/PROTOCOL.md",
                edition.unwrap_or("PHP Core")
            ),
        ],
        "hooks" => vec![".claude/hooks/**", ".cursor/hooks/**", ".codex/hooks/**"]
            .into_iter()
            .map(String::from)
            .collect(),
        "tooling" => vec![
            "runtime-rust/crates/infra-tooling/**",
            "tests/**",
            ".github/**",
            "install/**",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        "docs" => vec!["docs/**".into()],
        "harness" => vec![
            "runtime-rust/crates/infra-harness/**".into(),
            "runtime-rust/crates/infra-host/**".into(),
            "runtime-rust/crates/infra-system/**".into(),
            "runtime-rust/crates/infra-adapters/**".into(),
            "runtime-rust/crates/infra-ui/**".into(),
            "harness/**".into(),
        ],
        "diff" => diff(&root, request["base"].as_str().unwrap_or("origin/main"))?,
        "custom" => vec![],
        _ => return Err(RuntimeError::invalid("Unknown collection scope")),
    };
    if ["edition", "docs"].contains(&scope) {
        for entry in std::fs::read_dir(&target)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| RuntimeError::invalid("Collection path is not UTF-8"))?;
            if entry.file_type()?.is_file() && (name.ends_with(".md") || name == "VERSION") {
                includes.push(format!("./{name}"));
            }
        }
    } else if scope == "skills" {
        includes.push("./AGENTS.md".into());
    }
    let with_mirrors = flag(request, "with_mirrors")? || scope == "hooks";
    let with_task = flag(request, "with_task")?;
    if with_mirrors && ["edition", "skills"].contains(&scope) {
        for host in [".claude", ".cursor", ".codex"] {
            includes.push(if scope == "skills" {
                format!("{host}/skills/**")
            } else {
                format!("{host}/**")
            });
        }
    }
    includes.extend(strings(request, "include")?);
    includes.extend(strings(request, "paths")?);
    if includes.is_empty() {
        return Err(RuntimeError::invalid(
            "Collection requires at least one include; refusing whole-tree fallback",
        ));
    }
    let mut excludes = ALWAYS
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    if !with_task {
        excludes.push("**/Task/**".into());
    }
    if !with_mirrors {
        excludes.extend(
            ["**/.claude/**", "**/.cursor/**", "**/.codex/**"]
                .iter()
                .map(|value| value.to_string()),
        );
    }
    excludes.extend(strings(request, "exclude")?);
    let binary = request["binary"]
        .as_str()
        .map(OsString::from)
        .or_else(|| std::env::var_os("CODE2PROMPT_BIN"))
        .unwrap_or_else(|| "code2prompt".into());
    let version = process::run(
        &binary,
        &["--version".into()],
        &root,
        None,
        Duration::from_secs(15),
        65536,
    )?;
    if version.code != 0 {
        return Err(RuntimeError::unavailable(
            "code2prompt version probe failed",
        ));
    }
    let version = String::from_utf8_lossy(&version.stdout);
    let expression = regex::Regex::new(r"(\d+)\.(\d+)\.(\d+)")
        .map_err(|_| RuntimeError::invalid("Invalid version expression"))?;
    let found = expression
        .captures(&version)
        .ok_or_else(|| RuntimeError::invalid("code2prompt version is unrecognizable"))?;
    let parsed = (
        found[1]
            .parse::<u32>()
            .map_err(|_| RuntimeError::invalid("Invalid code2prompt major"))?,
        found[2]
            .parse::<u32>()
            .map_err(|_| RuntimeError::invalid("Invalid code2prompt minor"))?,
        found[3]
            .parse::<u32>()
            .map_err(|_| RuntimeError::invalid("Invalid code2prompt patch"))?,
    );
    if !((4, 2, 0)..(5, 0, 0)).contains(&parsed) {
        return Err(RuntimeError::unavailable(
            "code2prompt version is outside the supported [4.2.0,5.0.0) contract",
        ));
    }
    let encoding = request["encoding"].as_str().unwrap_or("cl100k");
    if !["cl100k", "p50k", "p50k_edit", "r50k"].contains(&encoding) {
        return Err(RuntimeError::invalid("Unsupported collection encoding"));
    }
    let format = request["format"].as_str().unwrap_or("markdown");
    if !["markdown", "json", "xml"].contains(&format) {
        return Err(RuntimeError::invalid(
            "Unsupported collection output format",
        ));
    }
    let temporary = Temporary::new("infra-collect")?;
    let raw = invoke(
        &binary,
        &target,
        &includes,
        &excludes,
        "json",
        encoding,
        temporary.path(),
    )?;
    let payload: Value = serde_json::from_slice(&raw)?;
    let files = payload["files"]
        .as_array()
        .filter(|files| !files.is_empty())
        .ok_or_else(|| RuntimeError::not_found("Collection patterns matched no files"))?;
    let token_count = payload["token_count"]
        .as_u64()
        .ok_or_else(|| RuntimeError::invalid("Collection token count is invalid"))?;
    let mut largest = Vec::new();
    for file in files {
        let name = file
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Collection file names must be strings"))?;
        let name = name.strip_prefix("./").unwrap_or(name);
        let path = common::under(&target, name)?;
        if !path.is_file() {
            return Err(RuntimeError::conflict("Collected file no longer exists"));
        }
        for pattern in ALWAYS {
            if common::matches(pattern, name, false)? {
                return Err(RuntimeError::policy(
                    "Collection tool returned a forbidden source",
                ));
            }
        }
        largest.push((std::fs::metadata(path)?.len(), name.to_string()));
    }
    largest.sort_by(|a, b| b.cmp(a));
    let top = request["top"].as_u64().unwrap_or(0) as usize;
    largest.truncate(top);
    let bundle = match format {
        "json" => common::text(raw)?,
        "xml" => payload["prompt"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Collection XML prompt is missing"))?
            .to_string(),
        _ => common::text(invoke(
            &binary,
            &target,
            &includes,
            &excludes,
            "markdown",
            encoding,
            temporary.path(),
        )?)?,
    };
    let label = edition
        .map(|edition| {
            format!(
                "{scope}-{}",
                edition.replace([' ', '/'], "-").to_lowercase()
            )
        })
        .unwrap_or_else(|| scope.into());
    let report = json!({"scope":scope,"edition":edition,"root":target,"include":includes,"exclude":excludes,"format":format,"encoding":encoding,"files":files.len(),"bytes":bundle.len(),"tokens":token_count,"with_task":with_task,"with_mirrors":with_mirrors,"code2prompt":format!("{}.{}.{}",parsed.0,parsed.1,parsed.2),"largest":largest});
    let summary = format!(
        "bundle {label}\nfiles {}\nsize {} B\ntokens {token_count} ({encoding}; relative OpenAI BPE unit)\n",
        files.len(),
        bundle.len()
    );
    if flag(request, "dry_run")? {
        return Ok(json!({"_cli":{"code":0,"stdout":"","stderr":summary},"result":report}));
    }
    if flag(request, "stdout")? {
        return Ok(json!({"_cli":{"code":0,"stdout":bundle,"stderr":summary},"result":report}));
    }
    let extension = if format == "markdown" { "md" } else { format };
    let output = if let Some(path) = request["output"].as_str() {
        safe::absolute(Path::new(path))?
    } else {
        root.join(".c2p").join(format!("{label}.{extension}"))
    };
    let parent = output
        .parent()
        .ok_or_else(|| RuntimeError::invalid("Collection output lacks a parent"))?;
    safe::ensure_directory(parent, false)?;
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| RuntimeError::invalid("Collection output name is not UTF-8"))?;
    let mut custody = PublicationFiles::new(parent)?;
    let old = custody.read(name, 256 * 1024 * 1024)?;
    custody.write_expected_mode(
        name,
        bundle.as_bytes(),
        old.as_deref(),
        0o600,
        256 * 1024 * 1024,
    )?;
    let manifest = format!("{name}.manifest.json");
    let previous = custody.read(&manifest, 4 * 1024 * 1024)?;
    custody.write_expected_mode(
        &manifest,
        &common::json_bytes(&report)?,
        previous.as_deref(),
        0o600,
        4 * 1024 * 1024,
    )?;
    Ok(json!({"output":output,"manifest":parent.join(manifest),"result":report}))
}
