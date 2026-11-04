use crate::common;
use infra_core::{Result, RuntimeError};
use serde_json::{Value, json};
use std::path::Path;
pub fn check(root: &Path, base: Option<&str>, strict: bool) -> Result<Value> {
    let default = std::env::var("GITHUB_BASE_REF")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|value| format!("origin/{value}"))
        .unwrap_or_else(|| "origin/main".into());
    let mut selected = base.unwrap_or(&default).to_string();
    let mut found = common::git(root, &["merge-base", &selected, "HEAD"], None)?;
    if found.code != 0 && selected == "origin/main" {
        selected = "main".into();
        found = common::git(root, &["merge-base", &selected, "HEAD"], None)?;
    }
    if found.code != 0 || found.stdout.is_empty() {
        if strict || base.is_some() {
            return Err(RuntimeError::unavailable(
                "Changelog base cannot be resolved",
            ));
        }
        return Ok(json!({"status":"skipped","reason":"merge base unavailable","base":selected}));
    }
    let merge_base = common::text(found.stdout)?.trim().to_string();
    let changed = common::git_ok(
        root,
        &["diff", "--name-only", "-z", &merge_base, "--"],
        None,
    )?;
    let names = changed
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(|raw| {
            std::str::from_utf8(raw)
                .map(str::to_string)
                .map_err(|_| RuntimeError::invalid("Changed source path is not UTF-8"))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut findings = Vec::new();
    let mut shared = Vec::new();
    for name in &names {
        let edition = common::PHP_EDITIONS
            .iter()
            .find_map(|edition| name.strip_prefix(&format!("{edition}/")));
        if name.starts_with("scripts/")
            || name.starts_with("runtime-rust/")
            || edition.is_some_and(|path| {
                path.starts_with("memory-bank/")
                    || path.starts_with("project-brain/")
                    || [".claude/hooks/", ".cursor/hooks/", ".codex/hooks/"]
                        .iter()
                        .any(|prefix| path.starts_with(prefix))
            })
        {
            shared.push(name.clone());
        }
    }
    if !shared.is_empty() && !names.iter().any(|name| name == "CHANGELOG.md") {
        findings.push(json!({"changelog":"CHANGELOG.md","reason":"shared executable core changed","paths":shared}));
    }
    for edition in ["Laravel", "Symfony", "PHP Core"] {
        let prefix = format!("{edition}/");
        let surface = names
            .iter()
            .filter(|name| {
                name.strip_prefix(&prefix).is_some_and(|name| {
                    [
                        "AGENTS.md",
                        "CLAUDE.md",
                        ".claude/DOD.md",
                        ".claude/GOLDEN-PRINCIPLES.md",
                        ".claude/STABILIZATION.md",
                        ".claude/settings.json",
                    ]
                    .contains(&name)
                        || name.starts_with(".agents/skills/")
                        || name.starts_with(".claude/agents/")
                        || name.starts_with(".claude/commands/")
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        let changelog = format!("{edition}/CHANGELOG.md");
        if !surface.is_empty() && !names.contains(&changelog) {
            findings.push(json!({"changelog":changelog,"reason":"model-facing surface changed","paths":surface}));
        }
    }
    Ok(
        json!({"status":if findings.is_empty(){"pass"}else{"fail"},"passed":findings.is_empty(),"base":selected,"merge_base":merge_base,"findings":findings}),
    )
}
