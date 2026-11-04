use infra_core::{Result, RuntimeError, fs};
use serde_json::Value;
use std::path::{Path, PathBuf};
fn width(value: &str) -> usize {
    value.encode_utf16().count()
}
fn take(value: &str, budget: usize) -> (&str, &str) {
    let mut size = 0;
    let mut end = 0;
    for (index, c) in value.char_indices() {
        let next = size + c.len_utf16();
        if next > budget {
            break;
        }
        size = next;
        end = index + c.len_utf8();
    }
    value.split_at(end)
}
fn pieces(text: &str, budget: usize) -> Vec<String> {
    let mut result = vec![];
    let mut lines = Vec::<(String, bool)>::new();
    let (mut size, mut fenced) = (0, false);
    for input in text.split_inclusive('\n') {
        let mut line = input;
        let mut heading = !fenced && line.starts_with('#');
        if line.starts_with("\u{60}\u{60}\u{60}") {
            fenced = !fenced;
        }
        while width(line) > budget {
            if !lines.is_empty() {
                result.push(lines.drain(..).map(|(line, _)| line).collect());
                size = 0;
            }
            let (first, rest) = take(line, budget);
            if first.is_empty() {
                break;
            }
            result.push(first.into());
            line = rest;
            heading = false;
        }
        while !lines.is_empty() && size + width(line) > budget {
            let (mut cut, mut prefix) = (lines.len(), 0);
            for (index, (earlier, starts)) in lines.iter().enumerate() {
                if index > 0 && *starts && prefix >= budget / 2 {
                    cut = index;
                }
                prefix += width(earlier);
            }
            result.push(lines.drain(..cut).map(|(line, _)| line).collect());
            size = lines.iter().map(|(line, _)| width(line)).sum();
        }
        if !line.is_empty() {
            size += width(line);
            lines.push((line.into(), heading));
        }
    }
    if !lines.is_empty() {
        result.push(lines.into_iter().map(|(line, _)| line).collect());
    }
    result
}
fn imports(root: &Path) -> bool {
    let Ok(expression) = regex::Regex::new(r"(?m)(?:^|\s)@(?:\./|\.\./)?AGENTS\.md(?:\s|$)") else {
        return false;
    };
    [".claude/CLAUDE.md", "CLAUDE.md"]
        .iter()
        .any(|name| fs::read_text(&root.join(name)).is_ok_and(|text| expression.is_match(&text)))
}
pub fn main_checkout(checkout: &Path) -> PathBuf {
    let operation = (|| -> Result<PathBuf> {
        let marker = checkout.join(".git");
        let raw = fs::read_text(&marker)?;
        let value = raw
            .trim()
            .strip_prefix("gitdir:")
            .ok_or_else(|| RuntimeError::invalid("Not a worktree marker"))?
            .trim();
        let private = infra_core::fs::canonicalize(checkout.join(value))?;
        let common = infra_core::fs::canonicalize(
            private.join(fs::read_text(&private.join("commondir"))?.trim()),
        )?;
        let back = infra_core::fs::canonicalize(
            private.join(fs::read_text(&private.join("gitdir"))?.trim()),
        )?;
        if private.parent() != Some(common.join("worktrees").as_path())
            || back != infra_core::fs::canonicalize(marker)?
        {
            return Err(RuntimeError::invalid("Unbound worktree marker"));
        }
        Ok(if common.file_name().is_some_and(|name| name == ".git") {
            common.parent().unwrap_or(&common).into()
        } else {
            common
        })
    })();
    operation.unwrap_or_else(|_| checkout.into())
}
pub fn project_key(start: &Path) -> PathBuf {
    for candidate in start.ancestors() {
        let marker = candidate.join(".git");
        if let Ok(meta) = std::fs::symlink_metadata(&marker) {
            if meta.file_type().is_symlink() {
                break;
            }
            if meta.is_dir() {
                return candidate.into();
            }
            if meta.is_file() {
                return main_checkout(candidate);
            }
        }
    }
    start.into()
}
fn external_approved(start: &Path) -> bool {
    let path = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .map(|path| path.join(".claude.json"))
        .unwrap_or_else(|| super::user_home().join(".claude.json"));
    let value = fs::read_text(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let Some(projects) = value
        .as_ref()
        .and_then(|value| value["projects"].as_object())
    else {
        return false;
    };
    let key = project_key(start).to_string_lossy().replace('\\', "/");
    projects
        .get(&key)
        .or_else(|| {
            cfg!(windows)
                .then(|| {
                    projects
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(&key))
                        .map(|(_, value)| value)
                })
                .flatten()
        })
        .is_some_and(|entry| entry["hasClaudeMdExternalIncludesApproved"] == true)
}
pub fn claude(root: &Path, start: &Path, part: usize, project: Option<&Path>) -> String {
    let path = root.join("AGENTS.md");
    if !(1..=6).contains(&part)
        || !path.is_file()
        || !imports(root)
        || project
            .map(|path| path.join("AGENTS.md").is_file())
            .unwrap_or_else(|| external_approved(start))
    {
        return String::new();
    }
    let Ok(text) = fs::read_text(&path) else {
        return String::new();
    };
    let header = |part, count| {
        format!(
            "Contents of {} (the project's policy), part {part} of {count}:\n\n",
            path.display()
        )
    };
    let budget = 10000usize.saturating_sub(width(&header(99, 99)) + 64);
    let parts = pieces(&text, budget);
    let count = parts.len().min(6);
    if part > count {
        return String::new();
    }
    let mut body = parts[part - 1].clone();
    if part == 6 && parts.len() > 6 {
        let done = parts[..5].iter().map(String::len).sum::<usize>();
        let reservation = format!(
            "\n\nThe policy goes on past what reaches this session: read {} from line 9999999 to its end.\n",
            path.display()
        );
        body = pieces(&text[done..], budget.saturating_sub(width(&reservation)))
            .into_iter()
            .next()
            .unwrap_or_default();
        let line = text[..done + body.len()]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        body.push_str(&format!("\n\nThe policy goes on past what reaches this session: read {} from line {line} to its end.\n",path.display()));
    }
    format!("{}{body}", header(part, count))
}
pub fn codex(root: &Path, project: &Path, payload: &Value) -> String {
    if !["startup", "clear", "compact"].contains(&payload["source"].as_str().unwrap_or("startup"))
        || project.join("AGENTS.md").is_file()
    {
        return String::new();
    }
    let path = root.join("AGENTS.md");
    fs::read_text(&path)
        .map(|text| {
            format!(
                "Contents of {} (the project's policy):\n\n{text}",
                path.display()
            )
        })
        .unwrap_or_default()
}
