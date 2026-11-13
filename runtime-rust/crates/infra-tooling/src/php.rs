use crate::{common, process, temp::Temporary};
use infra_core::{Result, RuntimeError, fs as safe};
use serde_json::{Value, json};
use std::{collections::BTreeMap, ffi::OsString, path::Path, time::Duration};
fn lint(root: &Path, file: &Path, source: &str) -> Result<Option<String>> {
    safe::atomic_write(file, source.as_bytes())?;
    let output = process::run(
        std::ffi::OsStr::new("php"),
        &[OsString::from("-l"), file.as_os_str().into()],
        root,
        None,
        Duration::from_secs(15),
        1024 * 1024,
    )?;
    if output.code == 0 {
        return Ok(None);
    }
    let joined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let message = joined
        .lines()
        .map(str::trim)
        .find(|line| line.contains("rror") && !line.contains("Errors parsing"))
        .or_else(|| joined.lines().map(str::trim).find(|line| !line.is_empty()))
        .unwrap_or("php -l failed");
    Ok(Some(
        message.replace(&file.to_string_lossy().to_string(), "<snippet>"),
    ))
}
pub fn check(root: &Path, require_php: bool) -> Result<Value> {
    let probe = process::run(
        std::ffi::OsStr::new("php"),
        &["--version".into()],
        root,
        None,
        Duration::from_secs(5),
        65536,
    );
    if probe.as_ref().is_err() || probe.as_ref().is_ok_and(|value| value.code != 0) {
        if require_php {
            return Err(RuntimeError::unavailable(
                "PHP is unavailable; no snippet was checked",
            ));
        }
        return Ok(json!({"status":"skipped","reason":"PHP unavailable","checked":0}));
    }
    let temporary = Temporary::new("infra-php-snippets")?;
    let file = temporary.path().join("snippet.php");
    let block = regex::Regex::new(r"(?ms)^```php([^\n]*)\n(.*?)^```")
        .map_err(|_| RuntimeError::invalid("Invalid PHP fence expression"))?;
    let files = common::tracked(root, &["*.md".into(), ":(glob,exclude)**/Task/**".into()])?;
    let mut modes = BTreeMap::<String, usize>::new();
    let mut failures = Vec::new();
    for name in files {
        let path = common::under(root, &name)?;
        if !path.is_file() {
            continue;
        }
        let text = safe::read_text(&path)?;
        for (index, capture) in block.captures_iter(&text).enumerate() {
            let source = &capture[2];
            if capture[1].split_whitespace().any(|word| word == "fragment") {
                *modes.entry("declared fragment".into()).or_default() += 1;
                continue;
            }
            if source.trim_start().starts_with("<?php") {
                if let Some(problem) = lint(root, &file, source)? {
                    failures.push(json!({"path":name,"block":index+1,"problem":problem}));
                } else {
                    *modes.entry("complete".into()).or_default() += 1;
                }
                continue;
            }
            let mut accepted = false;
            let mut first = None;
            for (mode, shape) in [
                ("statements", format!("<?php\n{source}\n")),
                (
                    "class members",
                    format!("<?php\nclass __Snippet\n{{\n{source}\n}}\n"),
                ),
            ] {
                match lint(root, &file, &shape)? {
                    None => {
                        *modes.entry(mode.into()).or_default() += 1;
                        accepted = true;
                        break;
                    }
                    Some(problem) => {
                        if first.is_none() {
                            first = Some(problem);
                        }
                    }
                }
            }
            if !accepted
                && (source.contains("<?php") || source.contains("<?="))
                && lint(root, &file, source)?.is_none()
            {
                *modes.entry("template".into()).or_default() += 1;
                accepted = true;
            }
            if !accepted {
                failures.push(json!({"path":name,"block":index+1,"problem":first.unwrap_or_else(||"No complete PHP shape parses".into())}));
            }
        }
    }
    Ok(
        json!({"status":if failures.is_empty(){"pass"}else{"fail"},"passed":failures.is_empty(),"counts":modes,"findings":failures}),
    )
}
