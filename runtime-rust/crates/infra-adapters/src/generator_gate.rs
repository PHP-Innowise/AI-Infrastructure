//! Concrete native generator gates. Arbitrary generated shell is never executed.
use crate::{hooks, process};
use infra_core::{Result, RuntimeError, fs};
use infra_generator::generated::{NativeGateServices, NativeOutput};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
pub struct Gate {
    pub binary: PathBuf,
    pub digest: String,
    pub size: u64,
}
/// Owns a private executable copy for the complete native invocation lifetime.
/// Creator may place cwd in a disposable rooted target and run this argv in its
/// sandbox. Private custody prevents a mutable installed pathname being execed.
pub struct Invocation {
    pub binary: PathBuf,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub digest: String,
    pub environment: BTreeMap<std::ffi::OsString, Option<std::ffi::OsString>>,
    source: PathBuf,
    expected_argv: Vec<String>,
    _custody: tempfile::TempDir,
}
fn installed_environment() -> BTreeMap<std::ffi::OsString, Option<std::ffi::OsString>> {
    [
        "ACCELERATOR_HOME",
        "ACCELERATOR_STATE_DIR",
        "ACCELERATOR_PROJECT_DIR",
        "ACCELERATOR_EDITION",
    ]
    .into_iter()
    .map(|key| (key.into(), None))
    .collect()
}
fn engine_image(path: &Path, size: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let observed = std::fs::symlink_metadata(path)?;
    if fs::linked(&observed) || !observed.is_file() || observed.len() != size {
        return Err(RuntimeError::policy(
            "Native executable length or type differs from its pinned identity",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if fs::linked(&opened) || !opened.is_file() || opened.len() != size {
        return Err(RuntimeError::policy(
            "Native executable changed during custody open",
        ));
    }
    let limit = size
        .checked_add(1)
        .ok_or_else(|| RuntimeError::policy("Invalid native executable length"))?;
    let mut image = vec![];
    file.take(limit).read_to_end(&mut image)?;
    if image.len() as u64 != size {
        return Err(RuntimeError::policy(
            "Native executable changed during custody read",
        ));
    }
    Ok(image)
}
impl Gate {
    pub fn current() -> Result<Self> {
        let binary = std::fs::canonicalize(std::env::current_exe()?)?;
        let size = std::fs::metadata(&binary)?.len();
        let digest = infra_core::query::sha256(&engine_image(&binary, size)?);
        Ok(Self {
            binary,
            digest,
            size,
        })
    }
    fn engine(&self) -> Result<()> {
        if !self.binary.is_absolute()
            || infra_core::query::sha256(&engine_image(&self.binary, self.size)?) != self.digest
        {
            return Err(RuntimeError::policy(
                "Native generator engine identity changed",
            ));
        }
        Ok(())
    }
    pub fn runtime_invocation(&self, target: &Path, arguments: &[String]) -> Result<Invocation> {
        self.engine()?;
        if arguments != ["status".to_string(), "--json".to_string()]
            && arguments != ["validate".to_string()]
        {
            return Err(RuntimeError::policy(
                "Generated runtime gate accepts only status --json and validate",
            ));
        }
        let target = std::fs::canonicalize(target)?;
        let source = hooks::installed_engine(&target);
        let installed = engine_image(&source, self.size)?;
        if infra_core::query::sha256(&installed) != self.digest {
            return Err(RuntimeError::policy(
                "Generated native runtime differs from the trusted selected engine",
            ));
        }
        let selected = engine_image(&self.binary, self.size)?;
        if infra_core::query::sha256(&selected) != self.digest {
            return Err(RuntimeError::policy(
                "Selected native runtime changed before custody copy",
            ));
        }
        let custody = tempfile::Builder::new()
            .prefix("infra-native-gate-")
            .tempdir()?;
        let binary = custody.path().join(if cfg!(windows) {
            "infra-engine.exe"
        } else {
            "infra-engine"
        });
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o700)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut file = options.open(&binary)?;
        {
            use std::io::Write;
            file.write_all(&selected)?;
            file.sync_all()?;
        }
        drop(file);
        let mut argv = vec![
            "memory".into(),
            "--root".into(),
            target.display().to_string(),
        ];
        argv.extend_from_slice(arguments);
        // A generated target is an installed runtime. Attached-session roots
        // inherited from the host must not change its explicit --root context.
        let environment = installed_environment();
        let invocation = Invocation {
            binary,
            expected_argv: argv.clone(),
            argv,
            cwd: target,
            digest: self.digest.clone(),
            environment,
            source,
            _custody: custody,
        };
        self.verify_invocation(&invocation)?;
        Ok(invocation)
    }
    pub fn verify_invocation(&self, invocation: &Invocation) -> Result<()> {
        self.engine()?;
        if invocation.digest != self.digest
            || invocation.argv != invocation.expected_argv
            || invocation.environment != installed_environment()
            || invocation.binary.parent() != Some(invocation._custody.path())
            || std::fs::canonicalize(&invocation.cwd)? != invocation.cwd
        {
            return Err(RuntimeError::policy(
                "Native gate invocation custody changed",
            ));
        }
        for path in [&invocation.binary, &invocation.source] {
            if infra_core::query::sha256(&engine_image(path, self.size)?) != self.digest {
                return Err(RuntimeError::policy(
                    "Native gate executable identity changed",
                ));
            }
        }
        Ok(())
    }
}
fn directory(host: &str) -> Result<&'static str> {
    match host {
        "claude" => Ok(".claude/hooks"),
        "cursor" => Ok(".cursor/hooks"),
        "codex" => Ok(".codex/hooks"),
        _ => Err(RuntimeError::invalid("Unknown generated host")),
    }
}
fn wiring(host: &str) -> Result<&'static str> {
    match host {
        "claude" => Ok(".claude/settings.json"),
        "cursor" => Ok(".cursor/hooks.json"),
        "codex" => Ok(".codex/hooks.json"),
        _ => Err(RuntimeError::invalid("Unknown generated host")),
    }
}
pub fn handler(root: &Path, host: &str, role: &str) -> Result<Value> {
    if !hooks::ROLES.contains(&role) && !(host == "claude" && role == "notification") {
        return Err(RuntimeError::invalid("Unknown native hook role"));
    }
    infra_tooling::attach::hook_handler(
        &hooks::installed_engine(root),
        root,
        host,
        role,
        cfg!(windows),
    )
}
/// Shell-form callers use this only for hosts whose canonical helper has no args.
pub fn command(root: &Path, host: &str, role: &str) -> Result<String> {
    let value = handler(root, host, role)?;
    if value.get("args").is_some() {
        return Err(RuntimeError::invalid(
            "Native exec-form hook requires command and args together",
        ));
    }
    value["command"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| RuntimeError::invalid("Native hook has no command"))
}
const LAUNCH_FIELDS: &[&str] = &[
    "command",
    "args",
    "shell",
    "commandWindows",
    "command_windows",
];
fn launch(value: &Value) -> Value {
    let mut output = serde_json::Map::new();
    for field in LAUNCH_FIELDS {
        if let Some(value) = value.get(*field) {
            output.insert((*field).into(), value.clone());
        }
    }
    Value::Object(output)
}
fn handlers<'a>(value: &'a Value, output: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            if map.get("command").is_some_and(Value::is_string) {
                output.push(value);
            } else {
                for value in map.values() {
                    handlers(value, output);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                handlers(item, output);
            }
        }
        _ => {}
    }
}
fn event_handlers<'a>(document: &'a Value, event: &str) -> Vec<&'a Value> {
    let mut result = vec![];
    handlers(&document["hooks"][event], &mut result);
    result
}
fn matches_handler(value: &Value, expected: &Value, host: &str) -> bool {
    launch(value) == *expected
        && value["async"] != true
        && (host == "cursor" || value["type"] == "command")
}
fn events(host: &str) -> Vec<(&'static str, Vec<&'static str>)> {
    if host == "cursor" {
        vec![
            ("sessionStart", vec!["local-context", "context-continuity"]),
            (
                "beforeSubmitPrompt",
                vec!["working-memory-read", "context-continuity"],
            ),
            ("subagentStart", vec!["subagent-gate"]),
            ("subagentStop", vec!["subagent-dispatch"]),
            ("beforeShellExecution", vec!["bash-validator"]),
            (
                "afterFileEdit",
                vec!["file-naming-validator", "loop-detection"],
            ),
            ("afterAgentResponse", vec!["context-continuity"]),
            ("stop", vec!["working-memory-write"]),
        ]
    } else {
        let mut result = vec![
            ("SessionStart", vec!["local-context", "context-continuity"]),
            (
                "UserPromptSubmit",
                vec!["working-memory-read", "context-continuity"],
            ),
            ("Stop", vec!["working-memory-write", "context-continuity"]),
            (
                "PreToolUse",
                vec!["bash-validator", "file-naming-validator", "subagent-gate"],
            ),
            ("PostToolUse", vec!["loop-detection"]),
        ];
        if host == "claude" {
            result.push(("SubagentStop", vec!["subagent-dispatch"]));
            result.push(("PostToolUse", vec!["subagent-dispatch"]));
        }
        result
    }
}
impl NativeGateServices for Gate {
    fn validate_host(
        &self,
        target: &Path,
        delivery_root: &Path,
        host: &str,
        owned: &BTreeMap<String, String>,
    ) -> Result<Vec<String>> {
        self.engine()?;
        let target = std::fs::canonicalize(target)?;
        let delivery = fs::absolute(delivery_root)?;
        let hooks_dir = directory(host)?;
        let wiring_path = wiring(host)?;
        let mut errors = vec![];
        let framework = fs::read_text(&target.join("project-brain/config/runtime.json"))
            .ok()
            .and_then(|text| infra_core::domain::json::parse(&text).ok())
            .and_then(|value| value["framework"].as_str().map(String::from));
        if framework.is_none() || crate::hook_policy::load(&target, framework.as_deref()).is_err() {
            errors.push("Generated native hook policy is missing, malformed, names another framework or differs from manifest custody".into());
        }
        for role in hooks::ROLES {
            let path = format!("{hooks_dir}/{role}.sh");
            if !owned.contains_key(&path) {
                errors.push(format!(
                    "[{host}] required hook is not manifest-owned: {path}"
                ));
                continue;
            }
            let absolute = target.join(&path);
            let bytes = match fs::read_regular(&absolute) {
                Ok(bytes) => bytes,
                Err(_) => {
                    errors.push(format!("[{host}] required hook is unavailable: {path}"));
                    continue;
                }
            };
            let unix = hooks::shim(host, role, false)?;
            let windows = hooks::shim(host, role, true)?;
            if bytes != unix.as_bytes() && bytes != windows.as_bytes() {
                errors.push(format!("[{host}] {path} is not the exact native exec-only adapter; arbitrary shell, extra commands and Python forwarding are refused"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if std::fs::metadata(&absolute)?.permissions().mode() & 0o111 == 0 {
                    errors.push(format!("{path}: not executable (chmod +x needed)"));
                }
            }
        }
        if !owned.contains_key(wiring_path) {
            errors.push(format!(
                "[{host}] hook wiring is not manifest-owned: {wiring_path}"
            ));
            return Ok(errors);
        }
        let document = match fs::read_text(&target.join(wiring_path))
            .and_then(|text| serde_json::from_str::<Value>(&text).map_err(Into::into))
        {
            Ok(value) if value.is_object() => value,
            _ => {
                errors.push(format!("{wiring_path}: unreadable or invalid JSON"));
                return Ok(errors);
            }
        };
        let mut roles = hooks::ROLES.to_vec();
        if host == "claude" {
            roles.push("notification");
        }
        let expected = roles
            .iter()
            .map(|role| Ok(((*role).to_string(), handler(&delivery, host, role)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let mut wired = vec![];
        handlers(&document, &mut wired);
        if wired.is_empty() {
            errors.push(format!("{wiring_path}: no hook commands wired"));
        }
        for candidate in &wired {
            let command = candidate["command"].as_str().unwrap_or("");
            let belongs = command.contains(&format!(".{host}/hooks/"))
                || command.contains("infra-engine")
                || cfg!(windows)
                    && command.starts_with(
                        "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ",
                    );
            if belongs
                && !expected
                    .values()
                    .any(|expected| matches_handler(candidate, expected, host))
            {
                errors.push(format!("{wiring_path}: native hook launch fields are not in their exact shared generated form; altered args, shell overrides and trailing shell commands are refused"));
            }
        }
        for (event, roles) in events(host) {
            let wired = event_handlers(&document, event);
            for role in roles {
                let expected = expected
                    .get(role)
                    .ok_or_else(|| RuntimeError::invalid("Native hook registry is inconsistent"))?;
                if !wired
                    .iter()
                    .any(|candidate| matches_handler(candidate, expected, host))
                {
                    errors.push(format!("[{host}] {wiring_path} does not run {role} on {event} in its exact native form"));
                }
            }
        }
        if host == "codex" {
            let relative = ".codex/config.toml";
            if !owned.contains_key(relative) {
                errors.push(
                    "[codex] .codex/config.toml is not manifest-owned ([features] hooks = true)"
                        .into(),
                );
            } else {
                let data = fs::read_text(&target.join(relative))
                    .ok()
                    .and_then(|text| text.parse::<toml::Value>().ok());
                if data
                    .as_ref()
                    .and_then(|value| value.get("features"))
                    .and_then(|value| value.get("hooks"))
                    .and_then(toml::Value::as_bool)
                    != Some(true)
                {
                    errors.push("[codex] .codex/config.toml does not enable hooks = true".into());
                }
            }
        }
        // MCP configuration is generated data; validate native command/args directly,
        // without importing or executing generated code.
        let mcp = match host {
            "claude" => ".mcp.json",
            "cursor" => ".cursor/mcp.json",
            "codex" => ".codex/config.toml",
            _ => unreachable!(),
        };
        if owned.contains_key(mcp) {
            if host == "codex" {
                let data = fs::read_text(&target.join(mcp))
                    .ok()
                    .and_then(|text| text.parse::<toml::Value>().ok());
                let entry = data
                    .as_ref()
                    .and_then(|value| value.get("mcp_servers"))
                    .and_then(|value| value.get("harness_memory"));
                let good = entry
                    .and_then(|entry| entry.get("command"))
                    .and_then(toml::Value::as_str)
                    .is_some_and(|command| {
                        Path::new(command) == hooks::installed_engine(&delivery)
                    })
                    && entry
                        .and_then(|entry| entry.get("args"))
                        .and_then(toml::Value::as_array)
                        .is_some_and(|args| {
                            args.iter()
                                .filter_map(toml::Value::as_str)
                                .collect::<Vec<_>>()
                                == vec![
                                    "mcp",
                                    "--root",
                                    delivery.to_str().unwrap_or(""),
                                    "--host",
                                    host,
                                ]
                        });
                if !good {
                    errors.push(format!(
                        "{mcp}: portable Memory MCP registration is missing or changed"
                    ));
                }
            } else {
                let data = fs::read_text(&target.join(mcp))
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok());
                let entry = data
                    .as_ref()
                    .map(|value| &value["mcpServers"]["harness-memory"]);
                let expected_args = serde_json::json!(["mcp", "--root", delivery, "--host", host]);
                let good = entry.is_some_and(|entry| {
                    entry["command"].as_str().is_some_and(|command| {
                        Path::new(command) == hooks::installed_engine(&delivery)
                    }) && entry["args"] == expected_args
                });
                if !good {
                    errors.push(format!(
                        "{mcp}: portable Memory MCP registration is missing or changed"
                    ));
                }
            }
        }
        Ok(errors)
    }
    fn runtime_command(
        &self,
        target: &Path,
        arguments: &[String],
        timeout: Duration,
    ) -> Result<NativeOutput> {
        let invocation = self.runtime_invocation(target, arguments)?;
        let output = process::run_with_env(
            &invocation.binary,
            &invocation.argv,
            &invocation.cwd,
            &[],
            timeout,
            &invocation.environment,
        )?;
        self.verify_invocation(&invocation)?;
        Ok(NativeOutput {
            code: output.code,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[cfg(test)]
mod installed_context_tests {
    use super::*;
    #[test]
    fn generated_invocation_clears_attached_roots_and_custodies_that_environment() {
        let target = tempfile::tempdir().unwrap();
        let artifact = hooks::installed_engine(target.path());
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::copy(std::env::current_exe().unwrap(), &artifact).unwrap();
        let gate = Gate::current().unwrap();
        let mut invocation = gate
            .runtime_invocation(target.path(), &["status".into(), "--json".into()])
            .unwrap();
        assert_eq!(
            invocation.argv,
            vec![
                "memory".to_string(),
                "--root".into(),
                std::fs::canonicalize(target.path())
                    .unwrap()
                    .display()
                    .to_string(),
                "status".into(),
                "--json".into(),
            ]
        );
        assert_eq!(invocation.environment.len(), 4);
        for key in [
            "ACCELERATOR_HOME",
            "ACCELERATOR_STATE_DIR",
            "ACCELERATOR_PROJECT_DIR",
            "ACCELERATOR_EDITION",
        ] {
            assert_eq!(
                invocation.environment.get(std::ffi::OsStr::new(key)),
                Some(&None)
            );
        }
        gate.verify_invocation(&invocation).unwrap();
        invocation.environment.insert(
            "ACCELERATOR_HOME".into(),
            Some(invocation.cwd.as_os_str().into()),
        );
        assert!(
            gate.verify_invocation(&invocation).is_err(),
            "attached overrides must fail custody"
        );
        invocation.environment = installed_environment();
        invocation
            .environment
            .insert("UNDECLARED_OVERRIDE".into(), Some("value".into()));
        assert!(
            gate.verify_invocation(&invocation).is_err(),
            "unreviewed environment overrides must fail custody"
        );
    }
}
