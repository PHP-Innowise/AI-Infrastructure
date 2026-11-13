//! Native platform integration, including the Symfony probe isolation contract.
use crate::{Result, RuntimeError, common};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Debug)]
pub struct PlatformConfig {
    pub key: &'static str,
    pub display_name: &'static str,
    pub project_marker: &'static str,
    pub skills_path: &'static str,
    pub executable: &'static str,
    pub executable_env: &'static str,
}
pub fn platform_config(request: &Value) -> Result<PlatformConfig> {
    let origin = common::text(request, "script_path")
        .or_else(|| common::text(request, "skill_path"))
        .unwrap_or("");
    let laravel = origin.split(['\\', '/']).any(|p| p == "Laravel")
        || common::text(request, "edition") == Some("Laravel");
    let native = origin.split(['\\', '/']).any(|p| p == "Symfony")
        || common::text(request, "edition") == Some("Symfony");
    let key = common::text(request, "platform").unwrap_or(if native {
        if origin.split(['\\', '/']).any(|p| p == ".cursor") {
            "cursor"
        } else {
            "codex"
        }
    } else {
        "claude"
    });
    Ok(match key {
        "codex" => PlatformConfig {
            key: "codex",
            display_name: "Codex",
            project_marker: ".agents",
            skills_path: ".agents/skills",
            executable: "codex",
            executable_env: "SKILL_CREATOR_CODEX_CLI",
        },
        "cursor" => PlatformConfig {
            key: "cursor",
            display_name: "Cursor",
            project_marker: ".cursor",
            skills_path: ".cursor/skills",
            executable: "cursor-agent",
            executable_env: "SKILL_CREATOR_CURSOR_CLI",
        },
        "claude" => PlatformConfig {
            key: "claude",
            display_name: "Claude",
            project_marker: if laravel { ".claude" } else { ".codex" },
            skills_path: if laravel {
                ".claude/commands"
            } else {
                ".codex/commands"
            },
            executable: "claude",
            executable_env: "SKILL_CREATOR_CLAUDE_CLI",
        },
        _ => {
            return Err(RuntimeError::invalid(
                "platform must be claude, codex, or cursor",
            ));
        }
    })
}
pub fn find_project_root(config: &PlatformConfig, start: &Path) -> Result<PathBuf> {
    let current = common::absolute(start)?;
    for candidate in current.ancestors() {
        if candidate.join(config.project_marker).is_dir() {
            return Ok(candidate.into());
        }
    }
    if config.key == "claude" {
        Ok(current)
    } else {
        Err(RuntimeError::not_found(format!(
            "Cannot find a project containing {}/ from {}.",
            config.project_marker,
            current.display()
        )))
    }
}
pub fn executable(
    config: &PlatformConfig,
    environment: &BTreeMap<String, String>,
) -> Result<String> {
    let name = environment
        .get(config.executable_env)
        .map(String::as_str)
        .unwrap_or(config.executable);
    let available = if name.contains(std::path::MAIN_SEPARATOR) {
        Path::new(name).is_file()
    } else {
        std::env::split_paths(environment.get("PATH").map(String::as_str).unwrap_or("")).any(|p| {
            #[cfg(windows)]
            {
                for ext in environment
                    .get("PATHEXT")
                    .map(String::as_str)
                    .unwrap_or(".EXE;.CMD;.BAT")
                    .split(';')
                {
                    if p.join(format!("{name}{ext}")).is_file() {
                        return true;
                    }
                }
            }
            p.join(name).is_file()
        })
    };
    if !available {
        return Err(RuntimeError::unavailable(format!(
            "{} CLI is unavailable. Install/authenticate it or set {} to a compatible executable.",
            config.display_name, config.executable_env
        )));
    }
    Ok(name.into())
}
pub fn write_probe_skill(
    config: &PlatformConfig,
    workspace: &Path,
    name: &str,
    description: &str,
) -> Result<(PathBuf, String)> {
    let regex = regex::Regex::new("[^a-z0-9]+").unwrap();
    let lower = name.to_lowercase();
    let raw = regex.replace_all(&lower, "-");
    let slug = common::chars(raw.trim_matches('-'), 40);
    let slug = if slug.is_empty() {
        "skill".into()
    } else {
        slug
    };
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let probe = format!("{slug}-trigger-probe-{}", &unique[..10]);
    let marker = format!(
        "SKILL_TRIGGERED_{}",
        uuid::Uuid::new_v4().simple().to_string().to_uppercase()
    );
    let directory = workspace.join(config.skills_path).join(&probe);
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("SKILL.md"),
        format!(
            "---\nname: {probe}\ndescription: {}\n---\n\n# Trigger Evaluation Probe\n\nWhen this skill is selected, respond with exactly \x60{marker}\x60 and stop.\nDo not call tools and do not complete the user's requested task.\n",
            serde_json::to_string(description)?
        ),
    )?;
    Ok((directory, marker))
}
#[expect(
    clippy::too_many_arguments,
    reason = "Preserve the explicit public provider-probe contract and caller-supplied execution scope without bundling or changing arguments."
)]
pub fn evaluate_trigger(
    config: &PlatformConfig,
    query: &str,
    name: &str,
    description: &str,
    timeout: Duration,
    _project: &Path,
    model: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> Result<bool> {
    let exe = executable(config, environment)?;
    let workspace =
        infra_eval::artifact::Scratch::new(&std::env::temp_dir(), "skill-trigger-probe")?;
    let (_, marker) = write_probe_skill(config, &workspace.0, name, description)?;
    let mut argv = vec![exe];
    if config.key == "codex" {
        argv.extend(
            [
                "exec",
                "--ephemeral",
                "--json",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--cd",
            ]
            .map(str::to_owned),
        );
        argv.push(workspace.0.to_string_lossy().into_owned());
    } else {
        argv.extend(["--print", "--output-format", "json"].map(str::to_owned));
    }
    if let Some(m) = model {
        argv.extend(["--model".into(), m.into()]);
    }
    argv.push(query.into());
    let output = common::invoke(
        argv,
        &workspace.0,
        environment.clone(),
        vec![],
        timeout,
        |_| None,
    )?;
    let text = common::success(output, &format!("{} trigger probe", config.display_name))?;
    Ok(text.contains(&marker))
}
pub fn call_model(
    config: &PlatformConfig,
    prompt: &str,
    model: Option<&str>,
    timeout: Duration,
) -> Result<String> {
    let environment = if config.key == "claude" {
        common::claude_env()
    } else {
        common::env()
    };
    let exe = executable(config, &environment)?;
    if config.key == "claude" {
        let mut argv = vec![exe, "-p".into(), "--output-format".into(), "text".into()];
        if let Some(m) = model {
            argv.extend(["--model".into(), m.into()]);
        }
        return common::success(
            common::invoke(
                argv,
                &std::env::current_dir()?,
                environment,
                prompt.as_bytes().to_vec(),
                timeout,
                |_| None,
            )?,
            "claude -p",
        );
    }
    let workspace =
        infra_eval::artifact::Scratch::new(&std::env::temp_dir(), "skill-description-improvement")?;
    let output_path = workspace.0.join("last-message.txt");
    let mut argv = vec![exe];
    if config.key == "codex" {
        argv.extend(
            [
                "exec",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--cd",
            ]
            .map(str::to_owned),
        );
        argv.push(workspace.0.to_string_lossy().into_owned());
        argv.extend([
            "--output-last-message".into(),
            output_path.to_string_lossy().into_owned(),
        ]);
    } else {
        argv.extend(["--print", "--output-format", "json"].map(str::to_owned));
    }
    if let Some(m) = model {
        argv.extend(["--model".into(), m.into()]);
    }
    argv.push(if config.key == "codex" {
        "-".into()
    } else {
        prompt.into()
    });
    let output = common::invoke(
        argv,
        &workspace.0,
        environment,
        if config.key == "codex" {
            prompt.as_bytes().to_vec()
        } else {
            vec![]
        },
        timeout,
        |_| None,
    )?;
    let stdout = common::success(
        output,
        &format!("{} description improvement", config.display_name),
    )?;
    if config.key == "codex" {
        let last = if output_path.is_file() {
            common::read(&output_path)?.trim().to_owned()
        } else {
            String::new()
        };
        return Ok(if last.is_empty() {
            stdout.trim().into()
        } else {
            last
        });
    }
    let payload: Value = serde_json::from_str(&stdout)
        .map_err(|_| RuntimeError::invalid("Cursor CLI returned invalid JSON output."))?;
    payload["result"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().into())
        .ok_or_else(|| {
            RuntimeError::invalid("Cursor CLI JSON output did not contain a non-empty result.")
        })
}
pub fn execute(v: &Value) -> Result<Value> {
    let config = platform_config(v)?;
    match common::text(v, "action").unwrap_or("detect") {
        "detect" => common::json_cli(
            json!({"key":config.key,"display_name":config.display_name,"project_marker":config.project_marker,"skills_path":config.skills_path,"executable":config.executable,"executable_env":config.executable_env,"project_root":find_project_root(&config,Path::new(common::text(v,"start").unwrap_or(".")))?}),
            String::new(),
        ),
        "evaluate-trigger" => {
            let found = evaluate_trigger(
                &config,
                common::required(v, "query")?,
                common::required(v, "skill_name")?,
                common::required(v, "description")?,
                Duration::from_secs(common::positive(v, "timeout", 60)? as u64),
                Path::new("."),
                common::text(v, "model"),
                &common::env(),
            )?;
            common::json_cli(json!({"triggered":found}), String::new())
        }
        "call-model" => common::json_cli(
            json!({"response":call_model(&config,common::required(v,"prompt")?,common::text(v,"model"),Duration::from_secs(common::positive(v,"timeout",300)?as u64))?}),
            String::new(),
        ),
        _ => Err(RuntimeError::invalid("Unknown platform action")),
    }
}
