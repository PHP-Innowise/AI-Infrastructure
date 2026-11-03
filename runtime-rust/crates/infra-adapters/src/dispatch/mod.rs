//! Native subfolder/worktree dispatch; trust and import-approval records are never written.
pub mod permissions;
pub mod policy;
use crate::{hooks, output::Output, process};
use infra_core::{Result, RuntimeError, fs};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
pub const CLAUDE_VERSION: u64 = 6;
pub const CODEX_VERSION: u64 = 2;
pub fn user_home() -> PathBuf {
    (if cfg!(windows) {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    } else {
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    })
    .map(|value| native_path(&value.to_string_lossy()))
    .unwrap_or_else(std::env::temp_dir)
}
pub fn home(host: &str) -> PathBuf {
    std::env::var_os(if host == "codex" {
        "CODEX_HOME"
    } else {
        "CLAUDE_CONFIG_DIR"
    })
    .map(|value| native_path(&value.to_string_lossy()))
    .unwrap_or_else(|| user_home().join(if host == "codex" { ".codex" } else { ".claude" }))
}
pub fn native_path(value: &str) -> PathBuf {
    if cfg!(windows)
        && value.starts_with('/')
        && value.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic)
        && value.as_bytes().get(2).is_none_or(|byte| *byte == b'/')
    {
        PathBuf::from(format!(
            "{}:{}",
            value[1..2].to_ascii_uppercase(),
            if value.len() == 2 { "/" } else { &value[2..] }
        ))
    } else {
        PathBuf::from(value)
    }
}
pub fn installed(root: &Path) -> bool {
    hooks::installed_engine(root).is_file() || root.join("memory-bank/scripts/context.py").is_file()
}
fn own_session(host: &str, payload: &Value) -> bool {
    let Some(path) = payload["transcript_path"]
        .as_str()
        .map(native_path)
        .filter(|path| path.is_absolute())
    else {
        return false;
    };
    let config = home(host).join(if host == "codex" {
        "sessions"
    } else {
        "projects"
    });
    fs::canonical_or_missing(&path)
        .ok()
        .zip(fs::canonical_or_missing(&config).ok())
        .is_some_and(|(path, config)| path.starts_with(config))
}
fn linked_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| fs::linked(&meta))
}
fn toml_hooks(home: &Path) -> bool {
    let path = home.join("config.toml");
    let text = match fs::read_text(&path) {
        Ok(text) => text,
        Err(error) if error.kind == infra_core::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    let value = match text.parse::<toml::Value>() {
        Ok(value) => value,
        Err(_) => return true,
    };
    value
        .get("hooks")
        .and_then(toml::Value::as_table)
        .is_some_and(|hooks| hooks.keys().any(|key| key != "state"))
}
fn mode_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    #[cfg(not(unix))]
    let _ = mode;
    let resolved = if linked_file(path) {
        std::fs::canonicalize(path)?
    } else {
        path.into()
    };
    if fs::read_regular(&resolved).ok().as_deref() != Some(bytes) {
        if let Some(parent) = resolved.parent() {
            fs::ensure_directory(parent, false)?;
        }
        fs::atomic_write(&resolved, bytes)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&resolved)?.permissions().mode() & 0o777 != mode {
            std::fs::set_permissions(&resolved, std::fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}
const LAUNCH_FIELDS: &[&str] = &[
    "command",
    "args",
    "shell",
    "commandWindows",
    "command_windows",
];
fn launch(value: &Value) -> Value {
    let mut result = serde_json::Map::new();
    for field in LAUNCH_FIELDS {
        if let Some(value) = value.get(*field) {
            result.insert((*field).into(), value.clone());
        }
    }
    Value::Object(result)
}
fn global_handler(store: &Path, host: &str, name: &str) -> Result<Value> {
    let engine = store.join(if cfg!(windows) {
        "infra-engine.exe"
    } else {
        "infra-engine"
    });
    let args = vec![
        "dispatch".into(),
        host.into(),
        "--store".into(),
        store.display().to_string(),
        name.into(),
    ];
    let mut entry = infra_tooling::attach::native_handler(&engine, &args, host, cfg!(windows))?;
    entry["type"] = "command".into();
    Ok(entry)
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum ProjectLaunch {
    Native(&'static str),
    Script(String),
}
impl ProjectLaunch {
    fn name(&self) -> String {
        match self {
            Self::Native(role) => format!("{role}.sh"),
            Self::Script(name) => name.clone(),
        }
    }
}
#[derive(Clone)]
struct DeclaredHook {
    launch: ProjectLaunch,
    timeout: Duration,
}
fn safe_script_name(host: &str, name: &str) -> bool {
    name.ends_with(".sh")
        && !name.starts_with('.')
        && name.chars().all(|character| {
            character == '-'
                || character == '.'
                || if host == "claude" {
                    character.is_alphanumeric()
                } else {
                    character.is_ascii_alphanumeric()
                }
        })
}
fn project_launch(root: &Path, host: &str, hook: &Value) -> Option<ProjectLaunch> {
    if hook["type"].as_str().is_some_and(|kind| kind != "command") {
        return None;
    }
    // The committed launch of this installation, and the machine-bound one an
    // earlier installation wrote, both name a native role.
    let engine = hooks::installed_engine(root);
    let installed = infra_tooling::attach::HookLaunch::installed(root, &engine, cfg!(windows));
    for role in hooks::ROLES {
        if [
            installed.handler(host, role),
            infra_tooling::attach::hook_handler(&engine, root, host, role, cfg!(windows)),
        ]
        .into_iter()
        .flatten()
        .any(|expected| launch(hook) == expected)
        {
            return Some(ProjectLaunch::Native(role));
        }
    }
    let command = hook["command"].as_str()?;
    let name = if host == "claude" {
        command
            .strip_prefix(concat!("\"$", "{CLAUDE_PROJECT_DIR}\"/.claude/hooks/"))?
            .to_string()
    } else {
        // The previous Codex dispatcher selected the safe final script name,
        // then invoked that fixed file under the installed root's hook directory.
        let pattern = regex::Regex::new(r"([A-Za-z0-9][A-Za-z0-9.-]*\.sh)\W*$").ok()?;
        pattern.captures(command)?.get(1)?.as_str().to_string()
    };
    safe_script_name(host, &name).then_some(ProjectLaunch::Script(name))
}
fn hook_timeout(hook: &Value) -> Result<Duration> {
    let seconds = hook["timeout"].as_u64().unwrap_or(15);
    let timeout = Duration::from_secs(seconds);
    if timeout.is_zero() || std::time::Instant::now().checked_add(timeout).is_none() {
        return Err(RuntimeError::invalid(
            "Project hook timeout must be a positive bounded duration",
        ));
    }
    Ok(timeout)
}
fn configured_hooks(root: &Path, host: &str) -> Result<(Value, BTreeMap<String, DeclaredHook>)> {
    let path = root.join(if host == "codex" {
        ".codex/hooks.json"
    } else {
        ".claude/settings.json"
    });
    let source: Value = serde_json::from_str(&fs::read_text(&path)?)?;
    let groups = source["hooks"]
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Project hook wiring must be an object"))?;
    let mut configured = BTreeMap::<String, DeclaredHook>::new();
    for groups in groups.values() {
        for group in groups.as_array().into_iter().flatten() {
            for hook in group["hooks"].as_array().into_iter().flatten() {
                let Some(launch) = project_launch(root, host, hook) else {
                    continue;
                };
                let name = launch.name();
                let timeout = hook_timeout(hook)?;
                if let Some(prior) = configured.get_mut(&name) {
                    if prior.launch != launch {
                        return Err(RuntimeError::coded(
                            infra_core::ErrorKind::Conflict,
                            "dispatch-hook-ambiguous",
                            format!(
                                "Ambiguous native and legacy launch for project hook {name}; review its configuration before dispatch"
                            ),
                        ));
                    }
                    prior.timeout = prior.timeout.max(timeout);
                } else {
                    configured.insert(name, DeclaredHook { launch, timeout });
                }
            }
        }
    }
    Ok((source, configured))
}
#[cfg(not(windows))]
fn project_bash() -> Result<PathBuf> {
    Ok(std::env::var_os("BASH")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bash")))
}
#[cfg(any(windows, test))]
fn msys_bash_alias(value: &str) -> bool {
    [
        "/usr/bin/bash",
        "/usr/bin/bash.exe",
        "/bin/bash",
        "/bin/bash.exe",
    ]
    .iter()
    .any(|alias| value.eq_ignore_ascii_case(alias))
}
#[cfg(any(windows, test))]
fn legacy_script_spelling(
    original: &str,
    mut namespace: impl FnMut(&str) -> Result<Option<(u64, u128, u32)>>,
) -> Result<String> {
    let refused = || RuntimeError::policy("Git Bash cannot address the selected Windows hook path");
    if original.contains('\0') {
        return Err(refused());
    }
    let mut text = original.replace('\\', "/");
    if let Some(rest) = text.strip_prefix("//?/") {
        let ordinary = if rest
            .get(..4)
            .is_some_and(|value| value.eq_ignore_ascii_case("UNC/"))
        {
            format!("//{}", &rest[4..])
        } else if rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && rest.as_bytes().get(1) == Some(&b':')
            && rest.as_bytes().get(2) == Some(&b'/')
        {
            rest.to_string()
        } else {
            return Err(refused());
        };
        // Removing the extended namespace is permitted only after proving the
        // exact same volume, full 128-bit file ID and kind under ordinary lookup.
        let retained = namespace(original)?.ok_or_else(refused)?;
        let plain = namespace(&ordinary)?.ok_or_else(refused)?;
        if retained != plain {
            return Err(refused());
        }
        text = ordinary;
    }
    if text.starts_with("//./") || text.starts_with("//?/") {
        return Err(refused());
    }
    if text.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && text.as_bytes().get(1) == Some(&b':')
        && text.as_bytes().get(2) == Some(&b'/')
    {
        return Ok(format!("/{}{}", &text[..1], &text[2..]));
    }
    if let Some(unc) = text.strip_prefix("//") {
        let mut parts = unc.split('/');
        if parts
            .next()
            .is_none_or(|part| part.is_empty() || [".", "..", "?"].contains(&part))
            || parts
                .next()
                .is_none_or(|part| part.is_empty() || [".", ".."].contains(&part))
        {
            return Err(refused());
        }
        return Ok(text);
    }
    Err(refused())
}
#[cfg(windows)]
fn legacy_script_argument(path: &Path) -> Result<String> {
    let original = path
        .to_str()
        .ok_or_else(|| RuntimeError::invalid("Project hook script path is not UTF-8"))?;
    legacy_script_spelling(original, |spelling| {
        Ok(fs::native_image(Path::new(spelling))?.map(|image| image.namespace()))
    })
}
#[cfg(not(windows))]
fn legacy_script_argument(path: &Path) -> Result<String> {
    path.to_str()
        .filter(|value| !value.contains('\0'))
        .map(str::to_string)
        .ok_or_else(|| RuntimeError::invalid("Project hook script path is not UTF-8"))
}
#[cfg(windows)]
fn project_bash() -> Result<PathBuf> {
    fn unavailable() -> RuntimeError {
        RuntimeError::coded(
            infra_core::ErrorKind::Unavailable,
            "dispatch-hook-shell-unavailable",
            "The declared project hook needs Git Bash; set BASH or CLAUDE_CODE_GIT_BASH_PATH to its native bash.exe. WSL and cmd are not substituted.",
        )
    }
    fn admit(path: PathBuf) -> Result<PathBuf> {
        let text = path.to_str().ok_or_else(unavailable)?;
        let path = native_path(text);
        if !path.is_absolute()
            || !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("bash.exe"))
        {
            return Err(unavailable());
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_file() || fs::linked_at(&path, &metadata)? {
            return Err(unavailable());
        }
        let parent = path.parent().ok_or_else(unavailable)?;
        // Documented Git/MSYS direct usr/bin and bin/bash.exe-wrapper layouts.
        // This identifies an admitted shell layout, not binary authenticity.
        let wrapper_runtime = parent
            .parent()
            .map(|root| root.join("usr/bin/msys-2.0.dll"));
        if !parent.join("msys-2.0.dll").is_file()
            && !wrapper_runtime
                .as_ref()
                .is_some_and(|runtime| runtime.is_file())
        {
            return Err(unavailable());
        }
        Ok(path)
    }
    // An explicit dedicated host override must never silently fall through.
    if let Some(path) =
        std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").filter(|path| !path.is_empty())
    {
        return admit(path.into());
    }
    if let Some(path) = std::env::var_os("BASH").filter(|path| !path.is_empty()) {
        let text = path.to_str().ok_or_else(unavailable)?;
        if !msys_bash_alias(text) {
            return admit(path.into());
        }
        // Exported MSYS aliases are not native Windows executable paths.
        // Resolve the Git installation below; never hand the alias to WSL.
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let git = directory.join("git.exe");
            if !git.is_file() {
                continue;
            }
            let Ok(git) = fs::python_realpath(&git) else {
                continue;
            };
            for parent in git.ancestors().skip(1) {
                let bash = parent.join("bin/bash.exe");
                if bash.is_file()
                    && let Ok(bash) = admit(bash)
                {
                    return Ok(bash);
                }
            }
        }
    }
    Err(unavailable())
}

fn internal_policy_part(hook: &str) -> Option<usize> {
    let number = hook.strip_prefix("policy-")?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    number
        .parse::<usize>()
        .ok()
        .filter(|part| (1..=6).contains(part))
}
fn group(mut handler: Value, timeout: Option<u64>) -> Value {
    if let Some(timeout) = timeout {
        handler["timeout"] = timeout.into();
    }
    json!({"hooks":[handler]})
}
fn wanted(root: &Path, host: &str, store: &Path) -> Result<Vec<(String, Value)>> {
    let (source, _configured) = configured_hooks(root, host)?;
    let hooks = source["hooks"]
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Project hook wiring must be an object"))?;
    let mut wanted = vec![];
    for (event, groups) in hooks {
        for source_group in groups.as_array().into_iter().flatten() {
            for hook in source_group["hooks"].as_array().into_iter().flatten() {
                let Some(declared) = project_launch(root, host, hook) else {
                    continue;
                };
                let mut entry = global_handler(store, host, &declared.name())?;
                for field in ["timeout", "additionalContextLimit", "statusMessage"] {
                    if let Some(value) = hook.get(field)
                        && (matches!(field, "timeout" | "additionalContextLimit") && value.is_u64()
                            || field == "statusMessage" && value.is_string())
                    {
                        entry[field] = value.clone();
                    }
                }
                let mut one = group(entry, None);
                if let Some(matcher) = source_group["matcher"]
                    .as_str()
                    .filter(|matcher| !matcher.is_empty())
                {
                    one["matcher"] = matcher.into();
                }
                wanted.push((event.clone(), one));
            }
        }
    }
    if host == "claude" {
        wanted.push((
            "PreToolUse".into(),
            group(global_handler(store, host, "permissions")?, Some(10)),
        ));
        wanted.push((
            "SessionStart".into(),
            group(global_handler(store, host, "environment")?, Some(10)),
        ));
        if root.join("AGENTS.md").is_file() {
            for part in 1..=6 {
                wanted.push((
                    "SessionStart".into(),
                    group(
                        global_handler(store, host, &format!("policy-{part}"))?,
                        Some(10),
                    ),
                ));
            }
        }
    } else if !wanted.is_empty() && root.join("AGENTS.md").is_file() {
        let mut hook = global_handler(store, host, "policy")?;
        hook["additionalContextLimit"] = 32768.into();
        wanted.push(("SessionStart".into(), group(hook, Some(10))));
    }
    Ok(wanted)
}
fn native_global_name(hook: &Value, store: &Path, host: &str) -> Option<String> {
    let name = if let Some(arguments) = hook["args"].as_array() {
        let arguments = arguments
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()?;
        if arguments.len() != 5
            || arguments[0] != "dispatch"
            || arguments[1] != host
            || arguments[2] != "--store"
            || native_path(arguments[3]) != store
        {
            return None;
        }
        arguments[4].to_string()
    } else {
        let words = crate::shell::words(hook["command"].as_str()?).ok()?;
        let words = if words.first().is_some_and(|word| word == "exec") {
            &words[1..]
        } else {
            &words
        };
        if words.len() != 6
            || words[1] != "dispatch"
            || words[2] != host
            || words[3] != "--store"
            || native_path(&words[4]) != store
        {
            return None;
        }
        words[5].clone()
    };
    if !safe_script_name(host, &name) {
        return None;
    }
    let expected = global_handler(store, host, &name).ok()?;
    (launch(hook) == launch(&expected)).then_some(name)
}
fn entry_key(
    event: &str,
    group: &Value,
    store: &Path,
    host: &str,
) -> Option<(String, String, String)> {
    let hooks = group["hooks"].as_array().filter(|hooks| hooks.len() == 1)?;
    let hook = &hooks[0];
    let mut names = hooks::ROLES
        .iter()
        .map(|role| format!("{role}.sh"))
        .collect::<Vec<_>>();
    names.extend(
        ["permissions", "environment", "policy"]
            .into_iter()
            .map(String::from),
    );
    names.extend((1..=6).map(|part| format!("policy-{part}")));
    let launcher = store.join("ai-accelerator-dispatch.sh");
    let command = hook["command"].as_str().unwrap_or("");
    let legacy = crate::shell::words(command).ok().filter(|words| {
        words.len() == 2 && native_path(&words[0]) == launcher && hook.get("args").is_none()
    });
    if let Some(words) = &legacy
        && safe_script_name(host, &words[1])
    {
        names.push(words[1].clone());
    }
    if let Some(name) = native_global_name(hook, store, host) {
        names.push(name);
    }
    let name = names.into_iter().find(|name| {
        legacy.as_ref().is_some_and(|words| words[1] == *name)
            || global_handler(store, host, name)
                .is_ok_and(|expected| launch(hook) == launch(&expected))
    })?;
    Some((
        event.into(),
        group["matcher"].as_str().unwrap_or("").into(),
        name,
    ))
}
fn owned_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    if linked_file(path) {
        return Err(RuntimeError::policy(
            "Linked dispatcher-owned file is refused",
        ));
    }
    mode_write(path, bytes, mode)
}
pub fn install(root: &Path, host: &str, payload: Option<&Value>) -> Result<Value> {
    if !["claude", "codex"].contains(&host) || host == "codex" && cfg!(windows) {
        return Ok(json!({"state":"unsupported"}));
    }
    if payload.is_some_and(|payload| !own_session(host, payload)) {
        return Ok(json!({"state":"unrelated-session"}));
    }
    let root = std::fs::canonicalize(root)?;
    let home = home(host);
    let source = root.join(if host == "codex" {
        ".codex/hooks.json"
    } else {
        ".claude/settings.json"
    });
    if !source.is_file() || !home.is_dir() {
        return Ok(json!({"state":"unavailable"}));
    }
    let folder = home.join("ai-accelerator");
    if std::fs::symlink_metadata(folder.join("off")).is_ok() || host == "codex" && toml_hooks(&home)
    {
        return Ok(json!({"state":"disabled"}));
    }
    fs::ensure_directory(&folder, true)?;
    if !crate::counters::owned(&folder) {
        return Err(RuntimeError::policy("Foreign dispatcher directory"));
    }
    let Some(_guard) = infra_core::native::DrainGuard::acquire(&folder, "registration")? else {
        return Ok(json!({"state":"busy"}));
    };
    let launcher = folder.join("ai-accelerator-dispatch.sh");
    let wanted = wanted(&root, host, &folder)?;
    if wanted.is_empty() {
        return Ok(json!({"state":"unavailable"}));
    }
    let metadata = folder.join("native-dispatch.json");
    let prior = fs::read_text(&metadata)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let version = if host == "codex" {
        CODEX_VERSION
    } else {
        CLAUDE_VERSION
    };
    let legacy_version = fs::read_text(&folder.join("ai-accelerator-dispatch.py"))
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("DISPATCH_VERSION = ")
                    .and_then(|value| value.parse::<u64>().ok())
            })
        })
        .unwrap_or(0);
    let newer = prior
        .as_ref()
        .is_some_and(|prior| prior["version"].as_u64().unwrap_or(0) > version)
        || legacy_version > version;
    if newer {
        return Ok(json!({"state":"preserved","reason":"newer-dispatcher","host":host}));
    }
    if !newer {
        let binary = std::env::current_exe()?;
        let image = fs::read_regular(&binary)?;
        let engine = folder.join(if cfg!(windows) {
            "infra-engine.exe"
        } else {
            "infra-engine"
        });
        owned_write(&engine, &image, 0o755)?;
        let text = format!(
            "#!/bin/sh\nexec \"$(dirname -- \"$0\")/{}\" dispatch {host} --store \"$(dirname -- \"$0\")\" \"$@\"\n",
            if cfg!(windows) {
                "infra-engine.exe"
            } else {
                "infra-engine"
            }
        );
        owned_write(&launcher, text.as_bytes(), 0o755)?;
        let mut value = serde_json::to_vec_pretty(
            &json!({"profile":"infra-dispatch-v1","version":version,"engine":engine,"sha256":infra_core::query::sha256(&image)}),
        )?;
        value.push(b'\n');
        owned_write(&metadata, &value, 0o600)?;
    }
    let projects = folder.join("projects");
    let mut listed = fs::read_text(&projects)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .filter(|line| installed(Path::new(line)))
        .collect::<BTreeSet<_>>();
    listed.insert(root.display().to_string());
    owned_write(
        &projects,
        (listed.into_iter().collect::<Vec<_>>().join("\n") + "\n").as_bytes(),
        0o644,
    )?;
    let settings = home.join(if host == "codex" {
        "hooks.json"
    } else {
        "settings.json"
    });
    let target = if linked_file(&settings) {
        std::fs::canonicalize(&settings)?
    } else {
        settings
    };
    let mut user = if target.exists() {
        serde_json::from_str::<Value>(&fs::read_text(&target)?)?
    } else {
        json!({})
    };
    if !user.is_object() || user.get("hooks").is_some_and(|hooks| !hooks.is_object()) {
        return Ok(json!({"state":"preserved","reason":"invalid-user-wiring"}));
    }
    if user.get("hooks").is_none() {
        user["hooks"] = json!({});
    }
    for (event, group) in wanted {
        if user["hooks"].get(&event).is_none() {
            user["hooks"][&event] = json!([]);
        }
        let Some(present) = user["hooks"][&event].as_array_mut() else {
            return Ok(json!({"state":"preserved","reason":"invalid-user-wiring"}));
        };
        let key = entry_key(&event, &group, &folder, host);
        if let Some(index) = present
            .iter()
            .position(|item| entry_key(&event, item, &folder, host) == key)
        {
            if !newer {
                present[index] = group;
            }
        } else {
            present.push(group);
        }
    }
    let mut bytes = serde_json::to_vec_pretty(&user)?;
    bytes.push(b'\n');
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(&target)
            .map(|meta| meta.permissions().mode() & 0o777)
            .unwrap_or(0o644)
    };
    #[cfg(not(unix))]
    let mode = 0o644;
    mode_write(&target, &bytes, mode)?;
    Ok(json!({"state":"registered","host":host,"project":root,"trust":"not-written"}))
}
pub fn install_for_session(root: &Path, host: &str, payload: &Value) -> Result<Value> {
    install(root, host, Some(payload))
}

struct Selection {
    root: PathBuf,
    project: PathBuf,
    borrowed: bool,
}
fn nearest_checkout(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|directory| directory.join(".git").exists())
        .map(Path::to_path_buf)
}
fn selection(store: &Path, start: &Path, host: &str) -> Result<Option<Selection>> {
    if std::env::var_os("ACCELERATOR_HOME").is_some() || store.join("off").exists() {
        return Ok(None);
    }
    let listed = fs::read_text(&store.join("projects"))?
        .lines()
        .map(native_path)
        .filter(|root| root.is_absolute())
        .collect::<Vec<_>>();
    let checkout = nearest_checkout(start);
    let main = checkout
        .as_ref()
        .map(|checkout| policy::main_checkout(checkout));
    let mut selected: Option<Selection> = None;
    if host == "claude" {
        for root in &listed {
            if start != root
                && start.starts_with(root)
                && checkout
                    .as_ref()
                    .is_none_or(|checkout| checkout == root || !checkout.starts_with(root))
                && selected
                    .as_ref()
                    .is_none_or(|current| root.as_os_str().len() > current.root.as_os_str().len())
            {
                selected = Some(Selection {
                    root: root.clone(),
                    project: root.clone(),
                    borrowed: false,
                });
            }
        }
    }
    if selected.is_none()
        && let (Some(checkout), Some(main)) = (&checkout, &main)
        && checkout != main
    {
        for root in &listed {
            if let Ok(relative) = root.strip_prefix(main) {
                let project = if relative.as_os_str().is_empty() {
                    checkout.clone()
                } else {
                    checkout.join(relative)
                };
                if start.starts_with(&project)
                    && selected.as_ref().is_none_or(|current| {
                        root.as_os_str().len() > current.root.as_os_str().len()
                    })
                {
                    selected = Some(Selection {
                        root: root.clone(),
                        project,
                        borrowed: true,
                    });
                }
            }
        }
    }
    if let Some(selected) = &selected {
        if !installed(&selected.root) {
            return Ok(None);
        }
        for ancestor in start.ancestors() {
            if ancestor == selected.project {
                break;
            }
            if installed(ancestor) {
                return Ok(None);
            }
            if !ancestor.starts_with(&selected.project) {
                break;
            }
        }
        if selected.borrowed && installed(&selected.project) {
            return Ok(None);
        }
    }
    Ok(selected)
}
pub fn run(host: &str, store: &Path, hook: &str, input: &[u8]) -> Result<Output> {
    if hook.is_empty()
        || hook.starts_with('.')
        || hook.contains('/')
        || !["claude", "codex"].contains(&host)
    {
        return Ok(Output::default());
    }
    let start = if host == "claude" {
        std::env::var_os("CLAUDE_PROJECT_DIR")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?)
    } else {
        std::env::current_dir()?
    };
    let start = std::fs::canonicalize(start)?;
    let payload = serde_json::from_slice::<Value>(input).unwrap_or_else(|_| json!({}));
    let Some(selected) = selection(store, &start, host)? else {
        if host == "claude" && hook == "bash-validator.sh" {
            let listed = fs::read_text(&store.join("projects")).unwrap_or_default();
            if listed.lines().any(|root| native_path(root) == start)
                && payload["cwd"].as_str().is_some_and(|_| {
                    hooks::Context::new(&start, "claude", input)
                        .is_ok_and(|context| context.entered)
                })
                && let Some(command) = payload["tool_input"]["command"].as_str()
                && permissions::memory_command(command, &start)
            {
                return Output::json(
                    &json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","permissionDecisionReason":"the accelerator's memory command, as the project allows it"}}),
                );
            }
        }
        return Ok(Output::default());
    };
    let project = selected.borrowed.then_some(selected.project.as_path());
    if hook == "environment" {
        if let Some(path) = std::env::var_os("CLAUDE_ENV_FILE").map(PathBuf::from) {
            let lines = permissions::environment(&selected.root, project);
            if !lines.is_empty() {
                let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
                writeln!(file, "{}", lines.join("\n"))?;
            }
        }
        return Ok(Output::default());
    }
    if hook == "permissions" {
        return permissions::permission(&payload, &selected.root, &start, project)
            .map(|value| Output::json(&value))
            .unwrap_or_else(|| Ok(Output::default()));
    }
    if host == "claude"
        && let Some(part) = internal_policy_part(hook)
    {
        return Ok(Output::text(policy::claude(
            &selected.root,
            &start,
            part,
            project,
        )));
    }
    if host == "codex" && hook == "policy" {
        let text = policy::codex(&selected.root, &selected.project, &payload);
        return if text.is_empty() {
            Ok(Output::default())
        } else {
            Output::json(
                &json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":text}}),
            )
        };
    }
    let requested = if hooks::ROLES.contains(&hook) {
        format!("{hook}.sh")
    } else {
        hook.to_string()
    };
    let (_, configured) = match configured_hooks(&selected.root, host) {
        Ok(configured) => configured,
        Err(error) if error.code.as_deref() == Some("dispatch-hook-ambiguous") => {
            return Ok(Output::error(2, error.message));
        }
        Err(error) => return Err(error),
    };
    let Some(declared) = configured.get(&requested) else {
        return Ok(Output::default());
    };
    let is_bash = requested == "bash-validator.sh";
    let mut environment = BTreeMap::<OsString, OsString>::new();
    if selected.borrowed {
        for (name, value) in [
            ("ACCELERATOR_HOME", &selected.root),
            ("ACCELERATOR_STATE_DIR", &selected.root),
            ("ACCELERATOR_PROJECT_DIR", &selected.project),
        ] {
            environment.insert(name.into(), value.as_os_str().into());
        }
    }
    if host == "claude" {
        environment.insert(
            "CLAUDE_PROJECT_DIR".into(),
            selected.project.as_os_str().into(),
        );
    }
    let mut output = match &declared.launch {
        ProjectLaunch::Native(role) => {
            let args = vec![
                "hook".into(),
                host.into(),
                (*role).into(),
                "--root".into(),
                selected.root.display().to_string(),
            ];
            process::engine(
                &args,
                &selected.project,
                input,
                declared.timeout,
                &environment,
            )?
        }
        ProjectLaunch::Script(name) => {
            let path = selected
                .root
                .join(if host == "claude" {
                    ".claude/hooks"
                } else {
                    ".codex/hooks"
                })
                .join(name);
            if !path.is_file() {
                return Err(RuntimeError::coded(
                    infra_core::ErrorKind::Unavailable,
                    "dispatch-hook-unavailable",
                    "The declared project hook script is unavailable",
                ));
            }
            let script = legacy_script_argument(&path)?;
            // This branch is chosen by the project's declaration, before native
            // execution. It never retries a failed native hook through a shell.
            process::run(
                &project_bash()?,
                &[script],
                &selected.project,
                input,
                declared.timeout,
                &environment,
            )?
        }
    };
    if is_bash
        && host == "claude"
        && output.code == 0
        && output.stdout.iter().all(u8::is_ascii_whitespace)
        && payload["tool_input"]["command"]
            .as_str()
            .is_some_and(|command| permissions::memory_command(command, &selected.root))
    {
        let allow = Output::json(&json!({"hookSpecificOutput":{"hookEventName":"PreToolUse",
            "permissionDecision":"allow",
            "permissionDecisionReason":"the accelerator's memory command, as the project allows it"}}))?;
        output.stdout.extend(allow.stdout);
    }
    Ok(output)
}

#[cfg(test)]
mod dispatch_contract_tests {
    use super::*;

    #[test]
    fn internal_policy_tokens_do_not_capture_custom_script_names() {
        for part in 1..=6 {
            assert_eq!(internal_policy_part(&format!("policy-{part}")), Some(part));
        }
        for hook in [
            "policy-1.sh",
            "policy-x",
            "policy-",
            "policy-0",
            "policy-7",
            "policy-1;other",
        ] {
            assert_eq!(internal_policy_part(hook), None);
        }
    }

    #[test]
    fn windows_legacy_script_arguments_keep_ordinary_paths_and_known_msys_aliases() {
        for (original, expected) in [
            (
                r"C:\space'apostrophe\guard.sh",
                "/C/space'apostrophe/guard.sh",
            ),
            ("c:/space name/guard.sh", "/c/space name/guard.sh"),
            (
                r"\\server\share\space name\guard.sh",
                "//server/share/space name/guard.sh",
            ),
        ] {
            assert_eq!(
                legacy_script_spelling(original, |_| panic!(
                    "ordinary spelling needs no NT alias proof"
                ))
                .unwrap(),
                expected
            );
        }
        for alias in [
            "/usr/bin/bash",
            "/usr/bin/bash.exe",
            "/bin/bash",
            "/bin/bash.exe",
        ] {
            assert!(msys_bash_alias(alias));
        }
        for other in [
            "bash",
            "/usr/bin/wsl",
            "C:/Windows/System32/bash.exe",
            "/usr/bin/bash-other",
        ] {
            assert!(!msys_bash_alias(other));
        }
        for invalid in [
            r"\\.\C:\guard.sh",
            r"\\?\Volume{fixture}\guard.sh",
            "C:relative.sh",
            "//server",
            "bad\0path",
        ] {
            assert!(
                legacy_script_spelling(invalid, |_| panic!(
                    "invalid spelling must not request identity proof"
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn extended_windows_hook_names_require_the_entire_native_namespace() {
        let original = r"\\?\C:\private\guard.sh";
        let ordinary = "C:/private/guard.sh";
        let held = (7u64, (1u128 << 120) | 42, 0o100000u32);
        let mut visited = Vec::new();
        let converted = legacy_script_spelling(original, |path| {
            visited.push(path.to_string());
            Ok(Some(held))
        })
        .unwrap();
        assert_eq!(converted, "/C/private/guard.sh");
        assert_eq!(visited, [original, ordinary]);
        for other in [
            (held.0, (1u128 << 121) | 42, held.2), // Equal low64 bits cannot bless a different file.
            (held.0 + 1, held.1, held.2),
            (held.0, held.1, 0o040000),
        ] {
            assert!(
                legacy_script_spelling(original, |path| {
                    Ok(Some(if path == original { held } else { other }))
                })
                .is_err()
            );
        }
        assert!(
            legacy_script_spelling(original, |path| {
                Ok(if path == original { Some(held) } else { None })
            })
            .is_err()
        );
        let unc = r"\\?\UNC\server\share\guard.sh";
        assert_eq!(
            legacy_script_spelling(unc, |_| Ok(Some(held))).unwrap(),
            "//server/share/guard.sh"
        );
    }
}
