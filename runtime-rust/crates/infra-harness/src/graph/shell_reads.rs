//! Recover narrowly attributable reads from persisted shell summaries.
use serde_json::Value;
use std::{
    path::{Component, Path},
    sync::LazyLock,
};

static PRINT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"^sed[ \t]+-n[ \t]+(?:'[1-9][0-9]*(?:,[1-9][0-9]*)?p'|"[1-9][0-9]*(?:,[1-9][0-9]*)?p"|[1-9][0-9]*(?:,[1-9][0-9]*)?p)[ \t]+(?:--[ \t]+)?([A-Za-z0-9_./-]+)$"#,
    )
    .expect("literal sed print pattern")
});

fn operands(command: &str) -> Option<Vec<&str>> {
    // Each command must match independently. A failed or partial chain
    // cannot tell which commands ran successfully.
    command
        .split("&&")
        .map(|part| {
            let path = PRINT.captures(part.trim())?.get(1)?.as_str();
            (!path.starts_with('-')
                && !Path::new(path)
                    .components()
                    .any(|part| part == Component::ParentDir))
            .then_some(path)
        })
        .collect()
}

pub(crate) fn completed_command(
    id: u64,
    detail: &str,
    workspace: &Path,
) -> Option<(u64, Vec<String>)> {
    if id == 0 || workspace.as_os_str().is_empty() {
        return None;
    }
    let paths = operands(detail)?;
    let root = workspace.canonicalize().ok()?;
    let files: Vec<String> = paths
        .into_iter()
        .filter_map(|path| {
            let joined = workspace.join(path);
            let resolved = joined.canonicalize().ok()?;
            if !resolved.is_file() || !resolved.starts_with(&root) {
                return None;
            }
            let relative = joined.strip_prefix(workspace).ok()?;
            let canonical_relative = resolved.strip_prefix(&root).ok()?;
            // Do not attribute a symlink's contents to the lexical path.
            (relative == canonical_relative)
                .then(|| {
                    relative
                        .components()
                        .collect::<std::path::PathBuf>()
                        .to_str()
                        .map(|path| path.replace('\\', "/"))
                })
                .flatten()
        })
        .collect();
    (!files.is_empty()).then_some((id, files))
}

pub(crate) fn completed(event: &Value, workspace: &Path) -> Option<(u64, Vec<String>)> {
    let is_tool =
        event["kind"] == "tool" || (event["kind"] == "agent_activity" && event["type"] == "tool");
    if !is_tool
        || event["tool"] != "Shell"
        || event["state"] != "completed"
        || event["ok"] != true
        || event["exit_code"] != 0
        || event["outcome"] == "not_run"
        || workspace.as_os_str().is_empty()
    {
        return None;
    }
    let id = event["id"].as_u64().filter(|id| *id > 0)?;
    let detail = event["detail"].as_str()?;
    completed_command(id, detail, workspace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_literal_prints_in_a_successful_completion_are_evidence() {
        let project = tempfile::tempdir().unwrap();
        for name in ["a.php", "b.php"] {
            std::fs::write(project.path().join(name), "<?php").unwrap();
        }
        let mut step = json!({"kind":"tool","tool":"Shell","state":"completed","ok":true,"exit_code":0,"id":9,
            "detail":"sed -n '2p' ./a.php && sed -n '1,4p' b.php"});
        assert_eq!(
            completed(&step, project.path()),
            Some((9, vec!["a.php".into(), "b.php".into()]))
        );
        for command in [
            "sed -n '1p' *.php",
            "sed -n '1p' a.php | head",
            "sed -n '1p' a.php …",
            "cd src && sed -n '1p' a.php",
            "sed -n '1p' ../a.php",
            "sed -n '1p' a.php # comment",
            "sed -n '1p' a.php && echo done",
            "sed -n '1p' a.php >/dev/null",
        ] {
            assert!(operands(command).is_none(), "{command}");
        }
        step["exit_code"] = json!(1);
        assert!(completed(&step, project.path()).is_none());
        step["exit_code"] = json!(0);
        step["state"] = json!("started");
        assert!(completed(&step, project.path()).is_none());
    }
}
