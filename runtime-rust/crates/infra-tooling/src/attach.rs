//! Attached sessions keep client state outside the served project.
mod cursor_overlay;
use crate::{budget, common, process};
use infra_core::{Result, Roots, RuntimeError, bank::publication::PublicationFiles, fs as safe};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Overlay {
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub settings: Value,
    pub prompt_prefix: String,
}
fn project_text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| RuntimeError::invalid("Attached project path is not UTF-8"))
}
pub fn project_key(project: &Path) -> Result<String> {
    let canonical = infra_core::fs::canonicalize(project)?;
    let mut spelling = project_text(&canonical)?;
    if cfg!(windows) {
        spelling = spelling.replace('/', "\\").to_lowercase();
    }
    Ok(common::sha256(spelling.as_bytes())[..16].into())
}
pub fn state_directory(project: &Path, base: Option<&Path>) -> Result<PathBuf> {
    let selected = if let Some(base) = base {
        safe::absolute(base)?
    } else {
        let user = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_STATE_HOME").map(PathBuf::from)
        };
        let user = match user {
            Some(user) => user,
            None => {
                let user = std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .ok_or_else(|| RuntimeError::unavailable("User state directory unavailable"))?;
                PathBuf::from(user).join(if cfg!(windows) {
                    "AppData/Local"
                } else {
                    ".local/state"
                })
            }
        };
        safe::absolute(&user.join("ai-infrastructure-harness"))?
    };
    Ok(selected.join("attached").join(project_key(project)?))
}
fn read_optional(root: &Path, name: &str) -> Result<Value> {
    let path = common::under(root, name)?;
    if path.is_file() {
        common::read_json(&path)
    } else {
        Ok(json!({}))
    }
}
pub fn detect(project: &Path) -> Result<(Option<String>, String)> {
    let project = infra_core::fs::canonicalize(project)?;
    let composer = read_optional(&project, "composer.json")?;
    let lock = read_optional(&project, "composer.lock")?;
    let mut packages = BTreeSet::new();
    for key in ["require", "require-dev"] {
        if let Some(object) = composer[key].as_object() {
            packages.extend(object.keys().map(|value| value.to_lowercase()));
        }
    }
    for key in ["packages", "packages-dev"] {
        for package in lock[key].as_array().into_iter().flatten() {
            if let Some(name) = package["name"].as_str() {
                packages.insert(name.to_lowercase());
            }
        }
    }
    if packages.contains("laravel/framework") || project.join("artisan").is_file() {
        return Ok((
            Some("Laravel".into()),
            "Laravel Composer dependency or artisan entry".into(),
        ));
    }
    if packages.contains("symfony/framework-bundle")
        || project.join("bin/console").is_file() && project.join("config/bundles.php").is_file()
    {
        return Ok((
            Some("Symfony".into()),
            "Symfony framework bundle or native console/config".into(),
        ));
    }
    let wordpress = [
        "johnpbloch/wordpress",
        "johnpbloch/wordpress-core",
        "roots/wordpress",
        "roots/wordpress-no-content",
    ]
    .iter()
    .any(|name| packages.contains(*name));
    if wordpress
        || [
            "wordpress-plugin",
            "wordpress-theme",
            "wordpress-muplugin",
            "wordpress-core",
        ]
        .contains(&composer["type"].as_str().unwrap_or(""))
        || project.join("wp-config.php").is_file()
        || project.join("wp-content").is_dir()
    {
        return Ok((
            Some("Cms/wordpress".into()),
            "WordPress package, project type or installation layout".into(),
        ));
    }
    let mut php = std::fs::read_dir(&project)?.collect::<std::io::Result<Vec<_>>>()?;
    php.sort_by_key(|entry| entry.file_name());
    let mut php_seen = false;
    let mut headers = vec![project.join("style.css")];
    for entry in php {
        if entry.path().extension().is_some_and(|ext| ext == "php") {
            php_seen = true;
            if headers.len() < 41 {
                headers.push(entry.path());
            }
        }
    }
    for path in headers {
        if path.is_file() {
            let mut source = std::fs::File::open(&path)?;
            let mut bytes = Vec::new();
            Read::by_ref(&mut source)
                .take(8192)
                .read_to_end(&mut bytes)?;
            let text = String::from_utf8_lossy(&bytes);
            if text.contains("Theme Name:") || text.contains("Plugin Name:") {
                return Ok((
                    Some("Cms/wordpress".into()),
                    "WordPress theme or plugin header".into(),
                ));
            }
        }
    }
    if project.join("composer.json").is_file() || php_seen {
        return Ok((
            Some("PHP Core".into()),
            "PHP project without a framework-specific edition".into(),
        ));
    }
    Ok((None, "No PHP project evidence at the project root".into()))
}
fn environment(
    home: &Path,
    project: &Path,
    state: &Path,
    edition: &str,
) -> Result<BTreeMap<String, String>> {
    Ok(BTreeMap::from([
        ("ACCELERATOR_HOME".into(), project_text(home)?),
        ("ACCELERATOR_STATE_DIR".into(), project_text(state)?),
        ("ACCELERATOR_PROJECT_DIR".into(), project_text(project)?),
        ("ACCELERATOR_EDITION".into(), edition.into()),
    ]))
}
fn preamble(home: &Path, project: &Path, state: &Path, edition: &str) -> String {
    format!(
        "Accelerator: {edition}\nTooling: {}\nProject sources: {}\nPrivate session state: {}\nOperate on the project. Treat the shared accelerator clone as read-only.\n",
        home.display(),
        project.display(),
        state.display()
    )
}
fn toml(value: &Value) -> Result<String> {
    match value {
        Value::Null => Err(RuntimeError::invalid("Null cannot be a TOML override")),
        Value::Array(values) => Ok(format!(
            "[{}]",
            values
                .iter()
                .map(toml)
                .collect::<Result<Vec<_>>>()?
                .join(",")
        )),
        Value::Object(values) => Ok(format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| Ok(format!("{}={}", serde_json::to_string(key)?, toml(value)?)))
                .collect::<Result<Vec<_>>>()?
                .join(",")
        )),
        _ => Ok(serde_json::to_string(value)?),
    }
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn catalogue(home: &Path) -> Result<Vec<Value>> {
    let base = home.join(".agents/skills");
    let mut result = Vec::new();
    for path in common::walk(&base)? {
        if path.split('/').count() != 2 || !path.ends_with("/SKILL.md") {
            continue;
        }
        let absolute = common::under(&base, &path)?;
        let text = safe::read_text(&absolute)?;
        let (front, _) = budget::split(&text, &path)?;
        let name = budget::field(front, "name")
            .ok_or_else(|| RuntimeError::invalid("Canonical skill has no name"))?;
        let description = budget::field(front, "description")
            .ok_or_else(|| RuntimeError::invalid("Canonical skill has no description"))?;
        result.push(json!({"name":name,"description":description,"path":absolute}));
    }
    Ok(result)
}
fn hook_role(handler: &Value) -> Result<String> {
    if let Some(arguments) = handler["args"].as_array()
        && arguments.first().and_then(Value::as_str) == Some("hook")
        && let Some(role) = arguments.get(2).and_then(Value::as_str)
    {
        return Ok(role.into());
    }
    let command = handler["command"]
        .as_str()
        .ok_or_else(|| RuntimeError::invalid("Hook command must be text"))?;
    let expression = regex::Regex::new(r#"([A-Za-z0-9_.-]+)\.sh[\s'"]*$"#)
        .map_err(|_| RuntimeError::invalid("Invalid hook role expression"))?;
    if let Some(found) = expression.captures(command) {
        return Ok(found[1].into());
    }
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    if let Some(at) = tokens
        .iter()
        .position(|token| token.trim_matches(['\'', '"']) == "hook")
        && let Some(role) = tokens.get(at + 2)
    {
        return Ok(role.trim_matches(['\'', '"']).to_string());
    }
    if let Some((_, encoded)) =
        command.split_once(" -NoLogo -NoProfile -NonInteractive -EncodedCommand ")
    {
        let bytes = unbase64(encoded)?;
        if bytes.len() % 2 != 0 {
            return Err(RuntimeError::invalid(
                "Native PowerShell launch encoding is invalid",
            ));
        }
        let units = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        let script = String::from_utf16(&units)
            .map_err(|_| RuntimeError::invalid("Native PowerShell launch text is invalid"))?;
        let invocation = script
            .strip_prefix(POWERSHELL_SETUP)
            .and_then(|value| value.strip_prefix("& "))
            .and_then(|value| value.strip_suffix(";exit $LASTEXITCODE"))
            .ok_or_else(|| {
                RuntimeError::invalid("Native PowerShell launch has an unknown wrapper")
            })?;
        let argv = quoted_powershell_arguments(invocation)?;
        if argv.len() == 6
            && argv[1] == "hook"
            && ["claude", "cursor", "codex"].contains(&argv[2].as_str())
            && argv[4] == "--root"
        {
            return Ok(argv[3].clone());
        }
    }
    Err(RuntimeError::invalid(
        "Hook command has no native role mapping",
    ))
}
const POWERSHELL_SETUP: &str = "$ErrorActionPreference='Stop';[Console]::InputEncoding=[System.Text.UTF8Encoding]::new($false);[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false);$OutputEncoding=[Console]::OutputEncoding;";
fn quoted_powershell_arguments(value: &str) -> Result<Vec<String>> {
    let mut chars = value.chars().peekable();
    let mut arguments = Vec::new();
    loop {
        while chars.peek().is_some_and(|value| value.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            return Ok(arguments);
        }
        if chars.next() != Some('\'') {
            return Err(RuntimeError::invalid(
                "Native PowerShell argv is not literal",
            ));
        }
        let mut argument = String::new();
        let mut closed = false;
        while let Some(value) = chars.next() {
            if value == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    argument.push('\'');
                } else {
                    closed = true;
                    break;
                }
            } else {
                argument.push(value);
            }
        }
        if !closed || chars.peek().is_some_and(|value| !value.is_whitespace()) {
            return Err(RuntimeError::invalid(
                "Native PowerShell literal is incomplete",
            ));
        }
        arguments.push(argument);
    }
}
fn unbase64(value: &str) -> Result<Vec<u8>> {
    if value.len() > 1024 * 1024 || !value.len().is_multiple_of(4) {
        return Err(RuntimeError::invalid("Native launch encoding is invalid"));
    }
    let mut bytes = Vec::new();
    let count = value.len() / 4;
    for (index, chunk) in value.as_bytes().as_chunks::<4>().0.iter().enumerate() {
        let digit = |byte: u8| -> Result<u32> {
            match byte {
                b'A'..=b'Z' => Ok(u32::from(byte - b'A')),
                b'a'..=b'z' => Ok(u32::from(byte - b'a' + 26)),
                b'0'..=b'9' => Ok(u32::from(byte - b'0' + 52)),
                b'+' => Ok(62),
                b'/' => Ok(63),
                _ => Err(RuntimeError::invalid(
                    "Native launch encoding has an invalid digit",
                )),
            }
        };
        let a = digit(chunk[0])?;
        let b = digit(chunk[1])?;
        let padding = if chunk[2] == b'=' && chunk[3] == b'=' {
            2
        } else if chunk[2] != b'=' && chunk[3] == b'=' {
            1
        } else if chunk[2] != b'=' && chunk[3] != b'=' {
            0
        } else {
            return Err(RuntimeError::invalid("Native launch padding is invalid"));
        };
        if padding > 0 && index + 1 != count {
            return Err(RuntimeError::invalid("Native launch padding is misplaced"));
        }
        let c = if padding == 2 { 0 } else { digit(chunk[2])? };
        let d = if padding > 0 { 0 } else { digit(chunk[3])? };
        if padding == 2 && b & 15 != 0 || padding == 1 && c & 3 != 0 {
            return Err(RuntimeError::invalid(
                "Native launch encoding is not canonical",
            ));
        }
        let packed = (a << 18) | (b << 12) | (c << 6) | d;
        bytes.push((packed >> 16) as u8);
        if padding < 2 {
            bytes.push((packed >> 8) as u8);
        }
        if padding == 0 {
            bytes.push(packed as u8);
        }
    }
    Ok(bytes)
}
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();
    for chunk in bytes.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = chunk.get(1).copied().map(u32::from).unwrap_or(0);
        let third = chunk.get(2).copied().map(u32::from).unwrap_or(0);
        let packed = (first << 16) | (second << 8) | third;
        output.push(TABLE[((packed >> 18) & 63) as usize] as char);
        output.push(TABLE[((packed >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[((packed >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(packed & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}
fn absolute_launch(value: &str, windows: bool) -> bool {
    Path::new(value).is_absolute()
        || windows
            && (value.starts_with("\\\\")
                || value.as_bytes().get(1) == Some(&b':')
                    && value
                        .as_bytes()
                        .get(2)
                        .is_some_and(|byte| *byte == b'\\' || *byte == b'/'))
}
#[cfg(windows)]
fn system_powershell() -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, capacity: u32) -> u32;
    }
    let mut buffer = vec![0u16; 32768];
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return Err(RuntimeError::unavailable(
            "Trusted system directory is unavailable",
        ));
    }
    buffer.truncate(length as usize);
    let shell = PathBuf::from(std::ffi::OsString::from_wide(&buffer))
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    if !shell.is_absolute() || !shell.is_file() {
        return Err(RuntimeError::unavailable(
            "Trusted Windows PowerShell executable is unavailable",
        ));
    }
    Ok(shell)
}
#[cfg(not(windows))]
fn system_powershell() -> Result<PathBuf> {
    Err(RuntimeError::coded(
        infra_core::ErrorKind::Unavailable,
        "native-windows-shell-unavailable",
        "Windows shell handlers must be materialized on their target Windows system",
    ))
}
/// One launch authority for native hooks and explicitly registered dispatchers.
/// Shell wrappers only establish UTF-8 streams and exec the actual Rust engine.
pub fn native_handler(
    engine: &Path,
    arguments: &[String],
    host: &str,
    windows: bool,
) -> Result<Value> {
    if !["claude", "cursor", "codex"].contains(&host) {
        return Err(RuntimeError::invalid("Invalid native handler host"));
    }
    let engine = project_text(engine)?;
    if !absolute_launch(&engine, windows)
        || engine.contains(['\0', '\r', '\n'])
        || arguments
            .iter()
            .any(|argument| argument.contains(['\0', '\r', '\n']))
    {
        return Err(RuntimeError::invalid(
            "Native handler requires an explicit absolute engine and plain argv",
        ));
    }
    if windows && host == "claude" {
        return Ok(json!({"command":engine,"args":arguments}));
    }
    if windows {
        // PowerShell's documented EncodedCommand contract is UTF-16LE. Encoding
        // protects host-shell quoting only; every executable behavior stays in Rust.
        let mut invocation = vec![powershell_quote(&engine)];
        invocation.extend(arguments.iter().map(|argument| powershell_quote(argument)));
        let script = format!(
            "{POWERSHELL_SETUP}& {};exit $LASTEXITCODE",
            invocation.join(" ")
        );
        let bytes = script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let shell = project_text(&system_powershell()?)?;
        if shell.contains(['"', '\0', '\r', '\n']) {
            return Err(RuntimeError::invalid(
                "Trusted Windows shell path cannot be represented",
            ));
        }
        let launcher = if host == "cursor" {
            format!("& {}", powershell_quote(&shell))
        } else {
            format!("\"{shell}\"")
        };
        return Ok(
            json!({"command":format!("{launcher} -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",base64(&bytes))}),
        );
    }
    let mut invocation = vec![shell_quote(&engine)];
    invocation.extend(arguments.iter().map(|argument| shell_quote(argument)));
    Ok(json!({"command":format!("exec {}",invocation.join(" "))}))
}
/// Construct a hook's literal argv and delegate to the common native formatter.
pub fn hook_handler(
    engine: &Path,
    home: &Path,
    host: &str,
    role: &str,
    windows: bool,
) -> Result<Value> {
    if role.is_empty()
        || !role
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
    {
        return Err(RuntimeError::invalid("Invalid native hook role"));
    }
    let home = project_text(home)?;
    if !absolute_launch(&home, windows) || home.contains(['\0', '\r', '\n']) {
        return Err(RuntimeError::invalid(
            "Native hook root must be an explicit absolute path",
        ));
    }
    native_handler(
        engine,
        &[
            "hook".into(),
            host.into(),
            role.into(),
            "--root".into(),
            home,
        ],
        host,
        windows,
    )
}
/// How hooks are launched. Attached overlays and native Windows installations
/// bind one checkout's absolute engine and root; installed and generated
/// projects elsewhere commit the portable form of `crate::launch`.
#[derive(Clone, Debug)]
pub enum HookLaunch {
    Bound {
        engine: PathBuf,
        home: PathBuf,
        windows: bool,
    },
    Portable {
        prefix: String,
    },
}
impl HookLaunch {
    /// The launch an installation or generation writes into `home`.
    pub fn installed(home: &Path, engine: &Path, windows: bool) -> Self {
        if !windows && let Ok(Some(prefix)) = crate::launch::prefix(home) {
            return Self::Portable { prefix };
        }
        Self::Bound {
            engine: engine.into(),
            home: home.into(),
            windows,
        }
    }
    pub fn handler(&self, host: &str, role: &str) -> Result<Value> {
        match self {
            Self::Bound {
                engine,
                home,
                windows,
            } => hook_handler(engine, home, host, role, *windows),
            Self::Portable { prefix } => crate::launch::hook_handler(host, role, prefix),
        }
    }
    /// The launch an earlier installation of this form committed, which is
    /// still this installation's own.
    pub fn earlier_handler(&self, host: &str, role: &str) -> Result<Value> {
        match self {
            Self::Bound { .. } => self.handler(host, role),
            Self::Portable { prefix } => crate::launch::launcher_handler(host, role, prefix),
        }
    }
}
fn rewrite_handler(handler: &mut Value, host: &str, launch: &HookLaunch) -> Result<()> {
    if !handler.is_object()
        || handler.get("type").is_some_and(|value| value != "command")
        || host == "claude" && handler["type"] != "command"
    {
        return Err(RuntimeError::invalid(
            "Release hook handler must use the host command type",
        ));
    }
    let mut known = None;
    for role in native_roles(host) {
        let expected = launch.handler(host, role)?;
        if same_launch(handler, &expected) || source_hook_identity(handler, host, role).is_some() {
            known = Some(role.to_string());
            break;
        }
    }
    let role = if let Some(role) = known {
        role
    } else {
        let role = hook_role(handler)?;
        if !native_roles(host).contains(&role.as_str()) {
            return Err(RuntimeError::invalid(
                "Release hook has no supported native role",
            ));
        }
        role
    };
    let launch = launch.handler(host, &role)?;
    let handler = handler
        .as_object_mut()
        .ok_or_else(|| RuntimeError::invalid("Hook handler must be an object"))?;
    handler.remove("command");
    handler.remove("args");
    for (key, value) in launch
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Native hook launch must be an object"))?
    {
        handler.insert(key.clone(), value.clone());
    }
    Ok(())
}
fn rewrite_hook_groups(groups: &mut Value, host: &str, launch: &HookLaunch) -> Result<()> {
    let object = groups
        .as_object_mut()
        .ok_or_else(|| RuntimeError::invalid("Host hooks must be an object"))?;
    for groups in object.values_mut() {
        for group in groups
            .as_array_mut()
            .ok_or_else(|| RuntimeError::invalid("Host hook groups must be an array"))?
        {
            if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                for handler in handlers {
                    rewrite_handler(handler, host, launch)?;
                }
            } else if group["command"].is_string() {
                rewrite_handler(group, host, launch)?;
            }
        }
    }
    Ok(())
}
/// Materialize only the hooks field; all release metadata/permissions survive.
pub fn normalize_hook_configuration(
    bytes: &[u8],
    host: &str,
    launch: &HookLaunch,
) -> Result<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(bytes)?;
    let hooks = value
        .get_mut("hooks")
        .ok_or_else(|| RuntimeError::invalid("Release host configuration has no hook map"))?;
    rewrite_hook_groups(hooks, host, launch)?;
    common::json_bytes(&value)
}
fn hooks(home: &Path, host: &str, engine: &Path) -> Result<Value> {
    let path = home.join(if host == "claude" {
        ".claude/settings.json"
    } else if host == "codex" {
        ".codex/hooks.json"
    } else {
        ".cursor/hooks.json"
    });
    let value = common::read_json(&path)?;
    let mut hooks = value["hooks"].clone();
    let launch = HookLaunch::Bound {
        engine: engine.into(),
        home: home.into(),
        windows: cfg!(windows),
    };
    rewrite_hook_groups(&mut hooks, host, &launch)?;
    Ok(hooks)
}
fn validate_attach_operation(operation: &str) -> Result<()> {
    if ![
        "attach-detect",
        "attach-env",
        "attach-overlay",
        "attach-run",
        "attach-hook-trust",
        "attach-trust-codex-hooks",
        "attach-installed-hook-trust",
        "attach-trust-installed-codex-hooks",
    ]
    .contains(&operation)
    {
        return Err(RuntimeError::invalid("Unknown attach operation"));
    }
    Ok(())
}
fn validate_attach_host(host: &str) -> Result<()> {
    if !["claude", "codex", "cursor"].contains(&host) {
        return Err(RuntimeError::invalid("Unknown attached host"));
    }
    Ok(())
}
pub fn build_overlay(
    repository: &Path,
    project: &Path,
    state: &Path,
    edition: &str,
    host: &str,
) -> Result<Overlay> {
    let edition = common::edition(edition)?;
    if edition == "Infrastructure-Creator" {
        return Err(RuntimeError::invalid(
            "Generator is not an attached edition",
        ));
    }
    validate_attach_host(host)?;
    let project = independent_project(repository, project)?;
    let home = infra_core::fs::canonicalize(repository.join(edition))?;
    let state = safe::absolute(state)?;
    if state == project || state.starts_with(&project) {
        return Err(RuntimeError::policy(
            "Attached state must lie outside the project",
        ));
    }
    let (mut files, _) = prepare_state_custody(repository, &project, &state, edition)?;
    let engine = std::env::current_exe()?;
    let policy = safe::read_text(&home.join("AGENTS.md"))?;
    let prefix = preamble(&home, &project, &state, edition);
    let mut overlay = Overlay {
        arguments: vec![],
        environment: environment(&home, &project, &state, edition)?,
        settings: json!({}),
        prompt_prefix: String::new(),
    };
    match host {
        "claude" => {
            let relative = "launch/claude-system-prompt.md";
            let content = format!("{prefix}\n{policy}");
            let old = files.read(relative, 8 * 1024 * 1024)?;
            if old.as_deref() != Some(content.as_bytes()) {
                files.write_expected_mode(
                    relative,
                    content.as_bytes(),
                    old.as_deref(),
                    0o600,
                    8 * 1024 * 1024,
                )?;
            }
            files.secure_private_directory("launch", false)?;
            files.secure_private_file(relative)?;
            overlay.arguments = vec![
                "--add-dir".into(),
                project_text(&home)?,
                "--append-system-prompt-file".into(),
                project_text(&state.join(relative))?,
            ];
            let source = common::read_json(&home.join(".claude/settings.json"))?;
            let mut settings = json!({"permissions":{"allow":[],"deny":[]}});
            if source["env"].is_object() {
                settings["env"] = source["env"].clone();
            }
            let mut allow = source["permissions"]["allow"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut deny = source["permissions"]["deny"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            allow.push(
                format!(
                    "Read(//{}/**)",
                    project_text(&home)?.trim_start_matches('/')
                )
                .into(),
            );
            allow.push(
                format!(
                    "Read(//{}/**)",
                    project_text(&state)?.trim_start_matches('/')
                )
                .into(),
            );
            deny.push(
                format!(
                    "Edit(//{}/**)",
                    project_text(&home)?.trim_start_matches('/')
                )
                .into(),
            );
            // The save a Stop asks for runs the engine these hooks are bound
            // to, which no edition rule names: Claude Code asked after every
            // answer and refused the save under `claude -p`.
            allow.push(format!("Bash({} memory:*)", project_text(&engine)?).into());
            settings["permissions"]["allow"] = json!(allow);
            settings["permissions"]["deny"] = json!(deny);
            settings["hooks"] = hooks(&home, "claude", &engine)?;
            overlay.settings = settings;
        }
        "codex" => {
            let mut instructions = format!("{prefix}\n## Accelerator skills\n\n");
            for skill in catalogue(&home)? {
                instructions.push_str(&format!(
                    "- {}: {} (file: {})\n",
                    skill["name"].as_str().unwrap_or(""),
                    skill["description"].as_str().unwrap_or(""),
                    skill["path"].as_str().unwrap_or("")
                ));
            }
            instructions.push_str(&format!("\n## Accelerator policy\n\n{policy}"));
            overlay.arguments.extend([
                "-c".into(),
                format!("developer_instructions={}", toml(&json!(instructions))?),
            ]);
            let definitions = hooks(&home, "codex", &engine)?;
            for (event, groups) in definitions
                .as_object()
                .ok_or_else(|| RuntimeError::invalid("Codex hooks must be an object"))?
            {
                overlay
                    .arguments
                    .extend(["-c".into(), format!("hooks.{event}={}", toml(groups)?)]);
            }
            overlay
                .arguments
                .extend(["--add-dir".into(), project_text(&state)?]);
        }
        "cursor" => {
            if !home.join(".cursor-plugin/plugin.json").is_file() {
                return Err(RuntimeError::not_found(
                    "Selected edition has no Cursor plugin manifest",
                ));
            }
            let plugin = cursor_overlay::materialize(
                repository,
                &home,
                &state,
                &mut files,
                &engine,
                &overlay.environment,
            )?;
            overlay
                .arguments
                .extend(["--plugin-dir".into(), project_text(&plugin)?]);
            overlay.prompt_prefix = format!(
                "{prefix}\nRead and follow the accelerator policy at {} before acting.\n\n",
                home.join("AGENTS.md").display()
            );
        }
        _ => return Err(RuntimeError::invalid("Unknown attached host")),
    }
    files.check()?;
    Ok(overlay)
}
pub fn merge_claude(base: &Value, overlay: &Value) -> Result<Value> {
    let mut result = overlay
        .as_object()
        .cloned()
        .ok_or_else(|| RuntimeError::invalid("Claude overlay must be an object"))?;
    let base = base
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Claude settings must be an object"))?;
    for (key, value) in base {
        if !["env", "permissions", "hooks"].contains(&key.as_str()) {
            result.insert(key.clone(), value.clone());
        }
    }
    let mut environment = overlay["env"].as_object().cloned().unwrap_or_default();
    if let Some(values) = base.get("env").and_then(Value::as_object) {
        environment.extend(values.clone());
    }
    result.insert("env".into(), Value::Object(environment));
    let mut permissions = serde_json::Map::new();
    for source in [
        &overlay["permissions"],
        base.get("permissions").unwrap_or(&Value::Null),
    ] {
        if let Some(object) = source.as_object() {
            for (key, rules) in object {
                let entries = permissions
                    .entry(key.clone())
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .ok_or_else(|| RuntimeError::invalid("Permission rules must be arrays"))?;
                for rule in rules
                    .as_array()
                    .ok_or_else(|| RuntimeError::invalid("Permission rules must be arrays"))?
                {
                    if !entries.contains(rule) {
                        entries.push(rule.clone());
                    }
                }
            }
        }
    }
    result.insert("permissions".into(), Value::Object(permissions));
    let mut hooks = overlay["hooks"].as_object().cloned().unwrap_or_default();
    if let Some(values) = base.get("hooks").and_then(Value::as_object) {
        for (event, groups) in values {
            hooks
                .entry(event.clone())
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or_else(|| RuntimeError::invalid("Hook groups must be arrays"))?
                .extend(
                    groups
                        .as_array()
                        .ok_or_else(|| RuntimeError::invalid("Hook groups must be arrays"))?
                        .clone(),
                );
        }
    }
    result.insert("hooks".into(), Value::Object(hooks));
    Ok(Value::Object(result))
}
pub fn codex_config_arguments(command: &[String]) -> Result<Vec<String>> {
    let mut remaining = Vec::new();
    let mut configs = Vec::new();
    let mut first = None;
    let mut at = 0;
    while at < command.len() {
        let value = &command[at];
        if value == "--" {
            break;
        }
        let width = if value == "-c" || value == "--config" {
            if at + 1 >= command.len() || command[at + 1] == "--" {
                return Err(RuntimeError::invalid(
                    "Codex config argument is missing its value",
                ));
            }
            2
        } else if value.starts_with("--config=") || value.starts_with("-c") && value.len() > 2 {
            1
        } else {
            0
        };
        if width > 0 {
            first.get_or_insert(remaining.len());
            configs.extend_from_slice(&command[at..at + width]);
            at += width;
        } else {
            remaining.push(value.clone());
            at += 1;
        }
    }
    if configs.is_empty() {
        return Ok(command.to_vec());
    }
    let mut first = first.unwrap_or(remaining.len());
    if let Some(exec) = remaining.iter().position(|value| value == "exec") {
        first = first.min(exec);
    }
    remaining.splice(first..first, configs);
    remaining.extend_from_slice(&command[at..]);
    Ok(remaining)
}
pub fn apply_overlay(
    host: &str,
    command: &[String],
    overlay: &Overlay,
    first_turn: bool,
) -> Result<Vec<String>> {
    let mut command = command.to_vec();
    match host {
        "claude" => {
            let index = command
                .iter()
                .position(|value| value == "--settings")
                .ok_or_else(|| RuntimeError::invalid("Claude command lacks --settings"))?;
            let current: Value = serde_json::from_str(
                command
                    .get(index + 1)
                    .ok_or_else(|| RuntimeError::invalid("Claude settings are missing"))?,
            )?;
            command[index + 1] =
                serde_json::to_string(&merge_claude(&current, &overlay.settings)?)?;
            command.splice(index + 2..index + 2, overlay.arguments.clone());
        }
        "codex" => {
            let index = command
                .iter()
                .position(|value| value == "exec")
                .ok_or_else(|| RuntimeError::invalid("Codex command lacks exec"))?;
            command.splice(index..index, overlay.arguments.clone());
            command = codex_config_arguments(&command)?;
        }
        "cursor" => {
            let index = command
                .iter()
                .position(|value| value == "--")
                .ok_or_else(|| {
                    RuntimeError::invalid("Cursor command lacks literal -- separator")
                })?;
            command.splice(index..index, overlay.arguments.clone());
            if first_turn && !overlay.prompt_prefix.is_empty() {
                let prompt = command
                    .last_mut()
                    .ok_or_else(|| RuntimeError::invalid("Cursor prompt is missing"))?;
                *prompt = format!("{}{prompt}", overlay.prompt_prefix);
            }
        }
        _ => return Err(RuntimeError::invalid("Unknown attached host")),
    }
    Ok(command)
}
struct Rpc {
    child: process::OwnedChild,
    input: mpsc::SyncSender<Vec<u8>>,
    output: mpsc::Receiver<Result<Value>>,
    reader: Option<thread::JoinHandle<()>>,
    writer: Option<thread::JoinHandle<()>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    sequence: u64,
}
impl Rpc {
    fn open(executable: &str, arguments: &[String], cwd: &Path) -> Result<Self> {
        let mut arguments = arguments.iter().map(OsString::from).collect::<Vec<_>>();
        arguments.push("app-server".into());
        let mut child = process::spawn_child(
            std::ffi::OsStr::new(executable),
            &arguments,
            cwd,
            true,
            false,
            &BTreeMap::new(),
        )?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| RuntimeError::unavailable("Client input unavailable"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| RuntimeError::unavailable("Client output unavailable"))?;
        let (tx, input) = mpsc::sync_channel::<Vec<u8>>(8);
        let (output, rx) = mpsc::sync_channel(16);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writing = stop.clone();
        let writer = thread::spawn(move || {
            let mut stdin = stdin;
            for bytes in input {
                let mut offset = 0;
                while offset < bytes.len() && !writing.load(std::sync::atomic::Ordering::Acquire) {
                    match stdin.write(&bytes[offset..]) {
                        Ok(0) => return,
                        Ok(count) => offset += count,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2))
                        }
                        Err(_) => return,
                    }
                }
                if writing.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
            }
        });
        let reading = stop.clone();
        let reader = thread::spawn(move || {
            let mut pending = Vec::<u8>::new();
            let mut chunk = [0u8; 8192];
            loop {
                if reading.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                match stdout.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => {
                        for byte in &chunk[..count] {
                            if pending.len() >= 4 * 1024 * 1024 {
                                let _ = output.try_send(Err(RuntimeError::unavailable(
                                    "Client response exceeded its boundary",
                                )));
                                return;
                            }
                            pending.push(*byte);
                            if *byte == b'\n' {
                                if let Ok(value) = serde_json::from_slice::<Value>(&pending)
                                    && output.try_send(Ok(value)).is_err()
                                {
                                    return;
                                }
                                pending.clear();
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if reading.load(std::sync::atomic::Ordering::Acquire) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        let mut rpc = Self {
            child,
            input: tx,
            output: rx,
            reader: Some(reader),
            writer: Some(writer),
            stop,
            sequence: 0,
        };
        rpc.request(
            "initialize",
            json!({"clientInfo":{"name":"accelerator-attach","version":"1"}}),
        )?;
        rpc.send(&json!({"jsonrpc":"2.0","method":"initialized"}))?;
        Ok(rpc)
    }
    fn send(&self, value: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(RuntimeError::invalid("Client request exceeds its boundary"));
        }
        bytes.push(b'\n');
        self.input.try_send(bytes).map_err(|_| {
            RuntimeError::unavailable("Client request channel closed or exceeded its boundary")
        })
    }
    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.sequence;
        self.sequence += 1;
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let response = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| RuntimeError::unavailable("Client did not answer its request"))??;
            if response["id"].as_u64() != Some(id) {
                continue;
            }
            if response.get("error").is_some() {
                return Err(RuntimeError::unavailable(
                    "Client refused the requested operation",
                ));
            }
            return Ok(response["result"].clone());
        }
    }
    fn finish(&mut self) -> Result<()> {
        let cleanup = self.child.terminate();
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        let (empty, _) = mpsc::sync_channel(1);
        let old = std::mem::replace(&mut self.input, empty);
        drop(old);
        cleanup?;
        if let Some(writer) = self.writer.take() {
            writer
                .join()
                .map_err(|_| RuntimeError::unavailable("Client input worker failed"))?;
        }
        if let Some(reader) = self.reader.take() {
            reader
                .join()
                .map_err(|_| RuntimeError::unavailable("Client output worker failed"))?;
        }
        Ok(())
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
pub fn hook_trust(
    executable: &str,
    overlay: &Overlay,
    project: &Path,
    approve: bool,
) -> Result<Value> {
    let mut client = Rpc::open(executable, &overlay.arguments, project)?;
    let listing = client.request("hooks/list", json!({"cwds":[project]}))?;
    let mut entries = Vec::new();
    let mut pending = serde_json::Map::new();
    for scope in listing["data"].as_array().into_iter().flatten() {
        for hook in scope["hooks"].as_array().into_iter().flatten() {
            if hook["source"] != "sessionFlags" {
                continue;
            }
            let key = hook["key"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Client hook key missing"))?;
            let hash = hook["currentHash"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Client hook identity missing"))?;
            entries.push(json!({"key":key,"hash":hash,"status":hook["trustStatus"],"command":hook["command"]}));
            if hook["trustStatus"] != "trusted" {
                pending.insert(key.into(), json!({"trusted_hash":hash}));
            }
        }
    }
    if approve && !pending.is_empty() {
        // This branch is reachable only from the explicit trust command, never an
        // install, sync, session launch, background refresh, or inference.
        client.request(
            "config/batchWrite",
            json!({"edits":[{"keyPath":"hooks.state","mergeStrategy":"upsert","value":pending}]}),
        )?;
    }
    client.finish()?;
    Ok(json!({"entries":entries,"approval_requested":approve,"pending":pending.len()}))
}
pub fn execute(operation: &str, request: &Value) -> Result<Value> {
    validate_attach_operation(operation)?;
    let host = match request.get("tool") {
        None | Some(Value::Null) => "codex",
        Some(Value::String(host)) => host.as_str(),
        _ => return Err(RuntimeError::invalid("Attached host must be text")),
    };
    validate_attach_host(host)?;
    let source = request["root"].as_str().unwrap_or(".");
    let root = infra_core::fs::canonicalize(source)?;
    let project =
        independent_project(&root, Path::new(request["project"].as_str().unwrap_or(".")))?;
    let detected = detect(&project)?;
    if operation == "attach-detect" {
        return Ok(json!({"project":project,"edition":detected.0,"evidence":detected.1}));
    }
    let edition = if let Some(value) = request["edition"].as_str() {
        common::edition(value)?.to_string()
    } else {
        detected.0.ok_or_else(|| {
            RuntimeError::invalid("Cannot infer attached edition; choose it explicitly")
        })?
    };
    let state = state_directory(&project, request["state_base"].as_str().map(Path::new))?;
    let home = root.join(&edition);
    if operation == "attach-env" {
        if state.exists() {
            let mut files = PublicationFiles::new(&state)?;
            let _ = files.read("accelerator-attach.json", 1024 * 1024)?;
            files.check()?;
        }
        return Ok(
            json!({"environment":environment(&home,&project,&state,&edition)?,"project":project,"state":state}),
        );
    }
    if operation == "attach-installed-hook-trust"
        || operation == "attach-trust-installed-codex-hooks"
    {
        return installed_hook_trust(
            request["executable"].as_str().unwrap_or("codex"),
            &root,
            &edition,
            &project,
            operation == "attach-trust-installed-codex-hooks",
        );
    }
    let overlay = build_overlay(&root, &project, &state, &edition, host)?;
    if operation == "attach-overlay" {
        return Ok(serde_json::to_value(overlay)?);
    }
    if operation == "attach-hook-trust" || operation == "attach-trust-codex-hooks" {
        return hook_trust(
            request["executable"].as_str().unwrap_or("codex"),
            &overlay,
            &project,
            operation == "attach-trust-codex-hooks",
        );
    }
    if operation != "attach-run" {
        return Err(RuntimeError::invalid("Unknown attach operation"));
    }
    let executable = request["executable"]
        .as_str()
        .unwrap_or(if host == "cursor" { "agent" } else { host });
    let mut arguments = overlay.arguments.clone();
    if host == "claude" {
        arguments.extend([
            "--settings".into(),
            serde_json::to_string(&overlay.settings)?,
        ]);
    }
    for value in request["arguments"].as_array().into_iter().flatten() {
        arguments.push(
            value
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Client arguments must be text"))?
                .into(),
        );
    }
    if host == "cursor"
        && !overlay.prompt_prefix.is_empty()
        && request["arguments"].as_array().is_none_or(Vec::is_empty)
    {
        arguments.push(format!(
            "{}Say which accelerator is attached and wait for my task.",
            overlay.prompt_prefix
        ));
    }
    if host == "codex" {
        let mut full = vec![executable.to_string()];
        full.extend(arguments);
        arguments = codex_config_arguments(&full)?.into_iter().skip(1).collect();
    }
    let mut command = Command::new(executable);
    command
        .args(&arguments)
        .current_dir(&project)
        .envs(&overlay.environment);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec().into())
    }
    #[cfg(windows)]
    {
        let arguments = arguments.iter().map(OsString::from).collect::<Vec<_>>();
        let environment = overlay
            .environment
            .iter()
            .map(|(key, value)| (OsString::from(key), Some(OsString::from(value))))
            .collect();
        let code = process::run_inherited(
            std::ffi::OsStr::new(executable),
            &arguments,
            &project,
            &environment,
        )?;
        Ok(
            json!({"_cli":{"code":code,"stdout":"","stderr":""},"result":{"project":project,"state":state,"edition":edition}}),
        )
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(RuntimeError::unavailable(
            "Interactive native attach is unsupported on this platform",
        ))
    }
}

/// Compute the exact installed-hook trust selection without contacting a host or
/// writing any user trust configuration. The caller supplies its admitted engine.
pub fn installed_hook_trust_selection(
    repository: &Path,
    edition: &str,
    project: &Path,
    native: &Path,
    listing: &Value,
) -> Result<Value> {
    fn launch(event: &str, handler: &Value) -> Result<Option<(String, String)>> {
        if !handler.is_object()
            || handler["command"].as_str().is_none_or(str::is_empty)
            || handler.get("type").is_some_and(|value| value != "command")
        {
            return Ok(None);
        }
        // Compare effective launch fields, including explicit shell/argv
        // overrides. Host bookkeeping and the opaque currentHash are separate.
        let identity = json!({
            "command": handler["command"],
            "args": handler["args"],
            "shell": handler["shell"],
            "commandShell": handler["commandShell"],
            "commandWindows": handler["commandWindows"],
            "cwd": handler["cwd"],
            "env": handler["env"],
        });
        Ok(Some((
            event.to_lowercase(),
            serde_json::to_string(&identity)?,
        )))
    }
    fn launches(config: &Value) -> Result<BTreeSet<(String, String)>> {
        let mut result = BTreeSet::new();
        for (event, groups) in config["hooks"]
            .as_object()
            .ok_or_else(|| RuntimeError::invalid("Codex hooks must be an object"))?
        {
            for group in groups
                .as_array()
                .ok_or_else(|| RuntimeError::invalid("Codex hook groups must be arrays"))?
            {
                for handler in group["hooks"].as_array().into_iter().flatten() {
                    if let Some(identity) = launch(event, handler)? {
                        result.insert(identity);
                    }
                }
            }
        }
        Ok(result)
    }

    let project = safe::absolute(project)?;
    let home = repository.join(common::edition(edition)?);
    let binary = if cfg!(windows) {
        "memory-bank/bin/infra-engine.exe"
    } else {
        "memory-bank/bin/infra-engine"
    };
    let expected_engine = project.join(binary);
    let source = safe::read_regular(&home.join(".codex/hooks.json"))?;
    let materialized = normalize_hook_configuration(
        &source,
        "codex",
        &HookLaunch::installed(&project, &expected_engine, cfg!(windows)),
    )?;
    let shipped = launches(&serde_json::from_slice::<Value>(&materialized)?)?;
    let mut files = PublicationFiles::new(&project)?;
    let local = files
        .read(".codex/hooks.json", 4 * 1024 * 1024)?
        .ok_or_else(|| RuntimeError::not_found("Installed Codex hooks are absent"))?;
    let installed = launches(&serde_json::from_slice::<Value>(&local)?)?;
    let expected = infra_core::fs::canonicalize(project.join(".codex/hooks.json"))?;
    let native_digest = common::sha256(&safe::read_regular(native)?);
    let binary_matches = files
        .read(binary, 512 * 1024 * 1024)?
        .is_some_and(|bytes| common::sha256(&bytes) == native_digest);
    let mut pending = serde_json::Map::new();
    let mut left = Vec::new();
    let mut already = 0usize;
    for scope in listing["data"].as_array().into_iter().flatten() {
        for hook in scope["hooks"].as_array().into_iter().flatten() {
            if hook["source"] != "project" {
                continue;
            }
            let Some(location) = hook["sourcePath"].as_str() else {
                continue;
            };
            if infra_core::fs::canonicalize(location).ok().as_ref() != Some(&expected) {
                continue;
            }
            let key = hook["key"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Client hook key is missing"))?;
            let identity = launch(hook["eventName"].as_str().unwrap_or(""), hook)?;
            let mine = binary_matches
                && identity.as_ref().is_some_and(|identity| {
                    shipped.contains(identity) && installed.contains(identity)
                });
            if !mine {
                left.push(key.to_string());
            } else if hook["trustStatus"] == "trusted" {
                already += 1;
            } else {
                let hash = hook["currentHash"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("Client hook identity is missing"))?;
                pending.insert(key.into(), json!({"trusted_hash":hash}));
            }
        }
    }
    files.check()?;
    Ok(json!({"pending":pending,"already":already,"left":left}))
}

pub fn installed_hook_trust(
    executable: &str,
    repository: &Path,
    edition: &str,
    project: &Path,
    approve: bool,
) -> Result<Value> {
    let mut client = Rpc::open(executable, &[], project)?;
    let listing = client.request("hooks/list", json!({"cwds":[project]}))?;
    let selection = installed_hook_trust_selection(
        repository,
        edition,
        project,
        &std::env::current_exe()?,
        &listing,
    )?;
    let pending = selection["pending"]
        .as_object()
        .ok_or_else(|| RuntimeError::unavailable("Native hook selection is invalid"))?;
    if approve && !pending.is_empty() {
        client.request(
            "config/batchWrite",
            json!({"edits":[{"keyPath":"hooks.state","mergeStrategy":"upsert","value":pending}]}),
        )?;
    }
    client.finish()?;
    Ok(json!({
        "approved":if approve {pending.len()} else {0},
        "pending":pending.len(),
        "already":selection["already"],
        "left":selection["left"],
        "approval_requested":approve,
    }))
}

pub fn hook_definitions(
    repository: &Path,
    edition: &str,
    host: &str,
    engine: &Path,
) -> Result<Value> {
    let edition = common::edition(edition)?;
    hooks(&repository.join(edition), host, engine)
}

pub fn independent_project(repository: &Path, project: &Path) -> Result<PathBuf> {
    let repository = infra_core::fs::canonicalize(repository)?;
    let project = infra_core::fs::canonicalize(project)?;
    if !project.is_dir() {
        return Err(RuntimeError::invalid("Attached project is not a directory"));
    }
    if project == repository || project.starts_with(&repository) || repository.starts_with(&project)
    {
        return Err(RuntimeError::policy(
            "Attached project must be outside the accelerator clone and cannot contain it",
        ));
    }
    Ok(project)
}
pub fn read_record(state: &Path) -> Result<Option<Value>> {
    match std::fs::symlink_metadata(state) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut files = PublicationFiles::new(state)?;
    files
        .read("accelerator-attach.json", 1024 * 1024)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(RuntimeError::from))
        .transpose()
}
pub fn prepare_state(
    repository: &Path,
    project: &Path,
    state: &Path,
    edition: &str,
) -> Result<Value> {
    prepare_state_custody(repository, project, state, edition).map(|(_, record)| record)
}
fn prepare_state_custody(
    repository: &Path,
    project: &Path,
    state: &Path,
    edition: &str,
) -> Result<(PublicationFiles, Value)> {
    let edition = common::edition(edition)?;
    if edition == "Infrastructure-Creator" {
        return Err(RuntimeError::invalid(
            "Generator is not an attached edition",
        ));
    }
    let project = independent_project(repository, project)?;
    let home = infra_core::fs::canonicalize(repository.join(edition))?;
    let state = safe::absolute(state)?;
    if state == project || state.starts_with(&project) {
        return Err(RuntimeError::policy(
            "Attached state must lie outside the project",
        ));
    }
    // Admission precedes pathname-based chmod, mkdir, lock and seed effects.
    // Every effect below uses this same retained directory custody.
    let mut files = PublicationFiles::open_private_root(&state)?;
    let roots = Roots {
        tooling: home,
        project: project.clone(),
        state: state.clone(),
        shared: false,
    };
    roots.ensure_attached_state_in(&mut files)?;
    let previous = files.read("accelerator-attach.json", 1024 * 1024)?;
    let record = json!({"project":project,"edition":edition,"since":infra_core::domain::utc_now()});
    let same = previous
        .as_ref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .is_some_and(|old| {
            old["project"] == record["project"] && old["edition"] == record["edition"]
        });
    if !same {
        files.write_expected_mode(
            "accelerator-attach.json",
            &common::json_bytes(&record)?,
            previous.as_deref(),
            0o600,
            1024 * 1024,
        )?;
    }
    files.secure_private_file("accelerator-attach.json")?;
    let record = files
        .read("accelerator-attach.json", 1024 * 1024)?
        .ok_or_else(|| RuntimeError::conflict("Attached session record disappeared"))?;
    let value = serde_json::from_slice(&record)?;
    files.check()?;
    Ok((files, value))
}

const NATIVE_SCRIPT_ROLES: &[&str] = &[
    "bash-validator",
    "file-naming-validator",
    "loop-detection",
    "subagent-gate",
    "subagent-dispatch",
    "working-memory-read",
    "working-memory-write",
    "context-continuity",
    "local-context",
];
fn native_roles(host: &str) -> Vec<&'static str> {
    let mut roles = NATIVE_SCRIPT_ROLES.to_vec();
    if host == "claude" {
        roles.push("notification");
    }
    roles
}
fn same_launch(left: &Value, right: &Value) -> bool {
    left.get("command") == right.get("command") && left.get("args") == right.get("args")
}
fn source_hook_identity(handler: &Value, host: &str, role: &str) -> Option<NativeHookIdentity> {
    for engine in [
        "memory-bank/bin/infra-engine",
        "./memory-bank/bin/infra-engine",
        "memory-bank/bin/infra-engine.exe",
        "./memory-bank/bin/infra-engine.exe",
    ] {
        let command = format!("{engine} hook {host} {role} --root .");
        let shell_template = handler["command"] == command && handler.get("args").is_none();
        let exec_template = host == "claude"
            && handler["command"] == engine
            && handler["args"] == json!(["hook", host, role, "--root", "."]);
        if shell_template || exec_template {
            return Some(NativeHookIdentity {
                host: host.into(),
                role: role.into(),
                engine: engine.into(),
                root: ".".into(),
                source_template: true,
            });
        }
    }
    None
}
#[derive(Clone, Debug, Serialize)]
pub struct NativeHookIdentity {
    pub host: String,
    pub role: String,
    pub engine: String,
    pub root: String,
    pub source_template: bool,
}
/// Classify only the canonical source template or the exact materialized launch
/// produced by the shared formatter. No substring can establish ownership.
pub fn classify_native_hook(
    handler: &Value,
    host: &str,
    home: &Path,
) -> Result<Option<NativeHookIdentity>> {
    if !["claude", "cursor", "codex"].contains(&host) {
        return Err(RuntimeError::invalid("Native hook host is unsupported"));
    }
    if !handler.is_object() {
        return Err(RuntimeError::invalid(
            "Native hook handler must be an object",
        ));
    }
    if handler.get("type").is_some_and(|value| value != "command")
        || host == "claude" && handler["type"] != "command"
    {
        return Err(RuntimeError::invalid(
            "Native hook handler must use the host command type",
        ));
    }
    let roles = native_roles(host);
    for role in &roles {
        if let Some(identity) = source_hook_identity(handler, host, role) {
            return Ok(Some(identity));
        }
    }
    let home = safe::absolute(home)?;
    let engine = home.join(if cfg!(windows) {
        "memory-bank/bin/infra-engine.exe"
    } else {
        "memory-bank/bin/infra-engine"
    });
    if let HookLaunch::Portable { prefix } = HookLaunch::installed(&home, &engine, cfg!(windows)) {
        for role in &roles {
            if same_launch(handler, &crate::launch::hook_handler(host, role, &prefix)?)
                || same_launch(
                    handler,
                    &crate::launch::launcher_handler(host, role, &prefix)?,
                )
            {
                return Ok(Some(NativeHookIdentity {
                    host: host.into(),
                    role: (*role).into(),
                    engine: "memory-bank/bin/infra-engine".into(),
                    root: ".".into(),
                    source_template: false,
                }));
            }
        }
    }
    for role in roles {
        let expected = hook_handler(&engine, &home, host, role, cfg!(windows))?;
        if same_launch(handler, &expected) {
            return Ok(Some(NativeHookIdentity {
                host: host.into(),
                role: role.into(),
                engine: project_text(&engine)?,
                root: project_text(&home)?,
                source_template: false,
            }));
        }
    }
    Ok(None)
}

#[cfg(all(test, unix))]
mod overlay_permission_tests {
    use super::*;

    /// The save a Stop asks an attached Claude Code session for runs the
    /// engine its hooks are bound to, by its absolute path.
    #[test]
    fn claude_overlay_allows_the_memory_command_of_its_bound_engine() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .unwrap()
            .to_path_buf();
        let scratch = std::env::temp_dir().join(format!(
            "infra-attach-overlay-{}",
            infra_core::domain::uuid4()
        ));
        std::fs::create_dir_all(scratch.join("project")).unwrap();
        std::fs::write(
            scratch.join("project/composer.json"),
            br#"{"name":"fixture/project"}"#,
        )
        .unwrap();
        let scratch = infra_core::fs::canonicalize(&scratch).unwrap();
        let project = scratch.join("project");
        let built = state_directory(&project, Some(&scratch.join("state")))
            .and_then(|state| build_overlay(&repository, &project, &state, "PHP Core", "claude"));
        let _ = std::fs::remove_dir_all(&scratch);
        let overlay = built.unwrap();
        let rule = format!(
            "Bash({} memory:*)",
            std::env::current_exe().unwrap().display()
        );
        let allowed = overlay.settings["permissions"]["allow"].as_array().unwrap();
        assert!(
            allowed.iter().any(|value| value.as_str() == Some(&rule)),
            "{rule} missing from {allowed:?}"
        );
    }
}
