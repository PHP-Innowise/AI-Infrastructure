use crate::{Result, RuntimeError};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
pub fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}
pub fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    text(v, key)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RuntimeError::invalid(format!("Missing {key}")))
}
pub fn flag(v: &Value, key: &str) -> bool {
    v.get(key).is_some_and(|x| x == true || x == "true")
}
pub fn number(v: &Value, key: &str, default: f64) -> Result<f64> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(x) => x
            .as_f64()
            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
            .filter(|n| n.is_finite())
            .ok_or_else(|| RuntimeError::invalid(format!("{key} must be finite"))),
    }
}
pub fn positive(v: &Value, key: &str, default: usize) -> Result<usize> {
    let n = number(v, key, default as f64)?;
    if n < 1.0 || n.fract() != 0.0 || n > usize::MAX as f64 {
        return Err(RuntimeError::invalid(format!(
            "{key} must be a positive integer"
        )));
    }
    Ok(n as usize)
}
pub fn positional(v: &Value, key: &str, index: usize) -> Result<String> {
    text(v, key)
        .or_else(|| v["_positionals"].get(index).and_then(Value::as_str))
        .map(str::to_owned)
        .ok_or_else(|| RuntimeError::invalid(format!("Missing {key}")))
}
pub fn absolute(path: impl AsRef<Path>) -> Result<PathBuf> {
    let p = path.as_ref();
    Ok(if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()?.join(p)
    })
}
pub fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}
pub fn read(path: &Path) -> Result<String> {
    let raw = std::fs::read(path)?;
    let text = std::str::from_utf8(&raw)
        .map_err(|_| RuntimeError::invalid("Text file is not valid UTF-8"))?;
    Ok(normalize(text))
}
pub fn json_file(path: &Path) -> Result<Value> {
    Ok(serde_json::from_str(&read(path)?)?)
}
pub fn pretty(v: &Value) -> Result<String> {
    Ok(serde_json::to_string_pretty(v)?)
}
pub fn cli(
    code: i32,
    stdout: impl Into<String>,
    stderr: impl Into<String>,
    result: Value,
) -> Value {
    json!({"_cli":{"code":code,"stdout":stdout.into(),"stderr":stderr.into()},"result":result})
}
pub fn json_cli(result: Value, stderr: String) -> Result<Value> {
    Ok(cli(0, format!("{}\n", pretty(&result)?), stderr, result))
}
pub fn rounded(n: f64) -> f64 {
    format!("{n:.4}").parse().unwrap_or(n)
}
pub fn env() -> BTreeMap<String, String> {
    std::env::vars().collect()
}
pub fn claude_env() -> BTreeMap<String, String> {
    env()
        .into_iter()
        .filter(|(k, _)| k != "CLAUDECODE")
        .collect()
}
pub fn sorted(path: &Path) -> Result<Vec<PathBuf>> {
    let mut p = std::fs::read_dir(path)?
        .map(|x| x.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    p.sort();
    Ok(p)
}
pub fn html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
pub fn display(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}
pub fn chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
pub fn title(s: &str) -> String {
    s.split(['-', '_', ' '])
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|a| a.to_uppercase().collect::<String>() + &c.as_str().to_lowercase())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn stdin_json() -> Result<Value> {
    const LIMIT: usize = 32 * 1024 * 1024;
    let mut input = Vec::new();
    std::io::stdin()
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut input)?;
    if input.len() > LIMIT {
        return Err(RuntimeError::invalid(
            "Standard input exceeds the 32 MiB JSON limit",
        ));
    }
    Ok(serde_json::from_slice(&input)?)
}
pub struct Capture {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub stopped: String,
    pub utf8_valid: bool,
}
pub fn invoke(
    argv: Vec<String>,
    cwd: &Path,
    env: BTreeMap<String, String>,
    input: Vec<u8>,
    timeout: Duration,
    mut observer: impl FnMut(&Value) -> Option<String> + Send,
) -> Result<Capture> {
    let scratch = infra_eval::artifact::Scratch::new(&std::env::temp_dir(), "skill-cli")?;
    let out = scratch.0.join("stdout");
    let err = scratch.0.join("stderr");
    let spec = infra_eval::process::CommandSpec {
        argv,
        cwd: cwd.into(),
        env,
        input,
    };
    let budget = infra_eval::process::Budget {
        timeout,
        cap: usize::MAX,
        max_bytes: 8 * 1024 * 1024,
        native: true,
        strict_events: false,
        error_diagnostics_terminal: false,
    };
    let session =
        infra_eval::process::execute(&spec, &budget, Some(&out), Some(&err), |event, _| {
            observer(event)
        })?;
    let stdout = std::fs::read(&out)?;
    let stderr = std::fs::read(&err)?;
    let utf8_valid = std::str::from_utf8(&stdout).is_ok() && std::str::from_utf8(&stderr).is_ok();
    Ok(Capture {
        code: session.exit.unwrap_or(-1),
        stdout: normalize(&String::from_utf8_lossy(&stdout)),
        stderr: normalize(&String::from_utf8_lossy(&stderr)),
        stopped: session.stopped,
        utf8_valid,
    })
}
pub fn success(output: Capture, label: &str) -> Result<String> {
    if !output.utf8_valid {
        return Err(RuntimeError::invalid(format!(
            "{label} returned invalid UTF-8"
        )));
    }
    if output.stopped != "finished" || output.code != 0 {
        return Err(RuntimeError::unavailable(format!(
            "{label} exited {} ({}): {}",
            output.code,
            output.stopped,
            output.stderr.trim()
        )));
    }
    Ok(output.stdout)
}
pub fn browser(url: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    let argv = vec![
        "cmd".into(),
        "/C".into(),
        "start".into(),
        String::new(),
        url.into(),
    ];
    #[cfg(target_os = "macos")]
    let argv = vec!["open".into(), url.into()];
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let argv = vec!["xdg-open".into(), url.into()];
    let output = invoke(
        argv,
        &std::env::current_dir()?,
        env(),
        vec![],
        Duration::from_secs(15),
        |_| None,
    )?;
    success(output, "browser opener").map(|_| ())
}
pub fn write_json(path: &Path, value: &Value, newline: bool) -> Result<()> {
    std::fs::write(path, pretty(value)? + if newline { "\n" } else { "" })?;
    Ok(())
}
