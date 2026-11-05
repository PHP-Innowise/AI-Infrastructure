//! Strict own-entry MCP registration and grouped rooted rollback.
use crate::common;
use infra_core::{Result, RuntimeError, bank::publication::PublicationFiles, fs as safe};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
const LIMIT: usize = 8 * 1024 * 1024;
pub const BEGIN: &str = "# BEGIN HARNESS MEMORY MCP";
pub const END: &str = "# END HARNESS MEMORY MCP";
const ENGINE: &str = if cfg!(windows) {
    "memory-bank/bin/infra-engine.exe"
} else {
    "memory-bank/bin/infra-engine"
};
pub fn host_path(host: &str) -> Result<&'static str> {
    match host {
        "claude" => Ok(".mcp.json"),
        "cursor" => Ok(".cursor/mcp.json"),
        "codex" => Ok(".codex/config.toml"),
        _ => Err(RuntimeError::invalid("Unknown MCP host")),
    }
}
fn host_for_path(path: &str) -> Result<&'static str> {
    match path {
        ".mcp.json" => Ok("claude"),
        ".cursor/mcp.json" => Ok("cursor"),
        ".codex/config.toml" => Ok("codex"),
        _ => Err(RuntimeError::invalid("Unknown project MCP configuration")),
    }
}
struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|byte| b" \r\n\t".contains(byte))
        {
            self.at += 1;
        }
    }
    fn take(&mut self, value: u8) -> bool {
        if self.bytes.get(self.at) == Some(&value) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn string(&mut self) -> Result<String> {
        let start = self.at;
        if !self.take(b'"') {
            return Err(RuntimeError::invalid("MCP object key must be text"));
        }
        while let Some(byte) = self.bytes.get(self.at).copied() {
            self.at += 1;
            if byte == b'\\' {
                if self.at >= self.bytes.len() {
                    break;
                }
                self.at += 1;
            } else if byte == b'"' {
                return Ok(serde_json::from_slice(&self.bytes[start..self.at])?);
            }
        }
        Err(RuntimeError::invalid("Unterminated MCP JSON string"))
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > 128 {
            return Err(RuntimeError::invalid(
                "MCP configuration nesting limit exceeded",
            ));
        }
        self.space();
        match self.bytes.get(self.at).copied() {
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b'{') => {
                self.at += 1;
                self.space();
                let mut object = Map::new();
                if self.take(b'}') {
                    return Ok(Value::Object(object));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    if object.contains_key(&key) {
                        return Err(RuntimeError::invalid("Duplicate MCP configuration key"));
                    }
                    self.space();
                    if !self.take(b':') {
                        return Err(RuntimeError::invalid("Invalid MCP object delimiter"));
                    }
                    object.insert(key, self.value(depth + 1)?);
                    self.space();
                    if self.take(b'}') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(RuntimeError::invalid("Invalid MCP object delimiter"));
                    }
                }
                Ok(Value::Object(object))
            }
            Some(b'[') => {
                self.at += 1;
                self.space();
                let mut items = Vec::new();
                if self.take(b']') {
                    return Ok(json!([]));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.space();
                    if self.take(b']') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(RuntimeError::invalid("Invalid MCP array delimiter"));
                    }
                }
                Ok(Value::Array(items))
            }
            Some(_) => {
                let start = self.at;
                while self
                    .bytes
                    .get(self.at)
                    .is_some_and(|byte| !b" \r\n\t,]}:".contains(byte))
                {
                    self.at += 1;
                }
                if start == self.at {
                    return Err(RuntimeError::invalid("Invalid MCP value"));
                }
                let raw = std::str::from_utf8(&self.bytes[start..self.at])
                    .map_err(|_| RuntimeError::invalid("Invalid MCP UTF-8"))?;
                if ["NaN", "Infinity", "-Infinity"].contains(&raw) {
                    return Err(RuntimeError::invalid("Nonfinite MCP value"));
                }
                let value: Value = serde_json::from_str(raw)?;
                if value.is_number()
                    && raw.contains(['.', 'e', 'E'])
                    && !raw.parse::<f64>().is_ok_and(f64::is_finite)
                {
                    return Err(RuntimeError::invalid("Nonfinite MCP value"));
                }
                Ok(value)
            }
            None => Err(RuntimeError::invalid("Incomplete MCP JSON")),
        }
    }
}
pub fn strict_json(bytes: &[u8]) -> Result<Value> {
    if bytes.len() > LIMIT {
        return Err(RuntimeError::invalid(
            "MCP configuration exceeds its size limit",
        ));
    }
    std::str::from_utf8(bytes)
        .map_err(|_| RuntimeError::invalid("MCP configuration is not UTF-8"))?;
    let mut parser = Parser { bytes, at: 0 };
    let value = parser.value(0)?;
    parser.space();
    if parser.at != bytes.len()
        || !value.is_object()
        || value
            .get("mcpServers")
            .is_some_and(|value| !value.is_object())
    {
        return Err(RuntimeError::invalid(
            "MCP configuration must contain exactly one server object document",
        ));
    }
    Ok(value)
}
/// The registration an installation or generation writes for `root`: portable
/// (see `crate::launch`), or bound to the absolute engine on native Windows.
pub fn entry(host: &str, root: &Path) -> Result<Value> {
    host_path(host)?;
    let root = long_path(&safe::absolute(root)?)?;
    if crate::launch::enabled()
        && let Ok(Some(prefix)) = crate::launch::prefix(&root)
    {
        return crate::launch::mcp_entry(host, &prefix);
    }
    Ok(
        json!({"type":"stdio","command":root.join(ENGINE),"args":["mcp","--root",root,"--host",host]}),
    )
}
fn normalized(value: &str) -> String {
    value.replace('\\', "/").to_lowercase()
}
fn launcher(command: &str, args: &[Value], host: &str) -> bool {
    if crate::launch::owned_mcp(command, args, host) {
        return true;
    }
    // Historical Python signatures are recognized as migration ownership data;
    // they are never executed or returned as an active server configuration.
    let legacy = match command {
        "python3" | "python" => Some(0),
        "py" => Some(1),
        _ => None,
    };
    if let Some(prefix) = legacy {
        if prefix == 1 && args.first().and_then(Value::as_str) != Some("-3") {
            return false;
        }
        let Some(args) = args.get(prefix..) else {
            return false;
        };
        if host == "cursor" {
            return args
                == [
                    json!("${workspaceFolder}/memory-bank/scripts/mcp_server.py"),
                    json!("--root"),
                    json!("${workspaceFolder}"),
                    json!("--host"),
                    json!("cursor"),
                ];
        }
        if args.len() != 3 || args[0] != "-c" || args[2] != host {
            return false;
        }
        let Some(code) = args[1].as_str() else {
            return false;
        };
        let Some(tail) = code.strip_prefix("# Harness memory bootstrap v") else {
            return false;
        };
        let Some((version, _)) = tail.split_once('\n') else {
            return false;
        };
        return !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit());
    }
    if args.len() != 5
        || args[0] != "mcp"
        || args[1] != "--root"
        || args[3] != "--host"
        || args[4] != host
    {
        return false;
    }
    let Some(root) = args[2].as_str() else {
        return false;
    };
    let root = normalized(root);
    let root = infra_core::domain::rstrip(&root);
    let root = root.strip_suffix('/').unwrap_or(root);
    let command = normalized(command);
    command == format!("{root}/memory-bank/bin/infra-engine")
        || command == format!("{root}/memory-bank/bin/infra-engine.exe")
}
pub fn owned(value: &Value, host: &str) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 3
        && object.contains_key("type")
        && object.contains_key("command")
        && object.contains_key("args")
        && value["type"] == "stdio"
        && value["command"]
            .as_str()
            .zip(value["args"].as_array())
            .is_some_and(|(command, args)| launcher(command, args, host))
}
fn owned_block(block: &str) -> Result<bool> {
    // The earlier wording is accepted so an older shipped template still merges.
    const SOURCE_NOTES: [&str; 2] = [
        "# Source template: installation writes the portable launch (a bound one on native Windows).",
        "# Source template: native install/generation binds the final absolute executable and root.",
    ];
    let mut lines = block
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    // This exact source-only annotation belongs to the shipped relative
    // template. Other comments and edits still fail closed below.
    let source_template = lines
        .first()
        .is_some_and(|line| SOURCE_NOTES.contains(line));
    if source_template {
        lines.remove(0);
    }
    if lines.len() != 4
        || lines[0] != "[mcp_servers.harness_memory]"
        || !lines[1].starts_with("command = ")
        || !lines[2].starts_with("args = ")
        || lines[3] != "enabled = true"
    {
        return Ok(false);
    }
    let document = block
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| RuntimeError::invalid("Invalid managed MCP TOML block"))?;
    let table = &document["mcp_servers"]["harness_memory"];
    if table.as_table().is_none_or(|table| table.len() != 3) {
        return Ok(false);
    }
    let Some(command) = table["command"].as_str() else {
        return Ok(false);
    };
    let Some(args) = table["args"].as_array() else {
        return Ok(false);
    };
    let mut arguments = Vec::new();
    for value in args {
        let Some(value) = value.as_str() else {
            return Ok(false);
        };
        arguments.push(json!(value));
    }
    if source_template {
        return Ok(table["enabled"].as_bool() == Some(true)
            && command == "./memory-bank/bin/infra-engine"
            && arguments
                == [
                    json!("mcp"),
                    json!("--root"),
                    json!("."),
                    json!("--host"),
                    json!("codex"),
                ]);
    }
    Ok(table["enabled"].as_bool() == Some(true) && launcher(command, &arguments, "codex"))
}
pub fn codex_block(root: &Path) -> Result<String> {
    let value = entry("codex", root)?;
    Ok(format!(
        "{BEGIN}\n[mcp_servers.harness_memory]\ncommand = {}\nargs = {}\nenabled = true\n{END}\n",
        serde_json::to_string(&value["command"])?,
        serde_json::to_string(&value["args"])?
    ))
}
pub fn template(host: &str, root: &Path) -> Result<Vec<u8>> {
    if host == "codex" {
        Ok(codex_block(root)?.into_bytes())
    } else {
        common::json_bytes(&json!({"mcpServers":{"harness-memory":entry(host,root)?}}))
    }
}
pub fn merge(path: &str, previous: Option<&[u8]>, source: &[u8], root: &Path) -> Result<Vec<u8>> {
    let host = host_for_path(path)?;
    if host != "codex" {
        let desired = strict_json(source)?;
        if !owned(&desired["mcpServers"]["harness-memory"], host) {
            return Err(RuntimeError::invalid(
                "Shipped memory MCP entry is not accelerator-owned",
            ));
        }
        let mut base = if let Some(previous) = previous {
            strict_json(previous)?
        } else {
            desired
        };
        let servers = base
            .as_object_mut()
            .ok_or_else(|| RuntimeError::invalid("MCP document must be an object"))?
            .entry("mcpServers")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| RuntimeError::invalid("MCP servers must be an object"))?;
        if servers
            .get("harness-memory")
            .is_some_and(|old| !owned(old, host))
        {
            return Err(RuntimeError::conflict(
                "Memory MCP name belongs to another server",
            ));
        }
        servers.insert("harness-memory".into(), entry(host, root)?);
        return common::json_bytes(&base);
    }
    let text = std::str::from_utf8(previous.unwrap_or(source))
        .map_err(|_| RuntimeError::invalid("Codex configuration is not UTF-8"))?;
    let retired = retire_legacy_codex(text);
    let text = retired.as_str();
    if text.len() > LIMIT
        || text.matches(BEGIN).count() != text.matches(END).count()
        || text.matches(BEGIN).count() > 1
    {
        return Err(RuntimeError::conflict("Malformed managed memory MCP block"));
    }
    let result = if let Some((before, rest)) = text.split_once(BEGIN) {
        let (block, after) = rest
            .split_once(END)
            .ok_or_else(|| RuntimeError::conflict("Managed MCP block is unfinished"))?;
        if !owned_block(block)? {
            return Err(RuntimeError::conflict(
                "Managed memory MCP block was edited; reconcile it before registration",
            ));
        }
        let outside = format!("{before}{after}");
        let parsed = outside
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| RuntimeError::invalid("Invalid Codex TOML configuration"))?;
        if parsed
            .get("mcp_servers")
            .and_then(|servers| servers.get("harness_memory"))
            .is_some()
        {
            return Err(RuntimeError::conflict(
                "Memory MCP name exists outside the managed block",
            ));
        }
        format!(
            "{before}{}{after}",
            codex_block(root)?.strip_suffix('\n').unwrap_or("")
        )
    } else {
        let parsed = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| RuntimeError::invalid("Invalid Codex TOML configuration"))?;
        if parsed
            .get("mcp_servers")
            .and_then(|servers| servers.get("harness_memory"))
            .is_some()
        {
            return Err(RuntimeError::conflict(
                "Memory MCP name belongs to an unmanaged Codex server",
            ));
        }
        format!(
            "{}\n\n{}",
            infra_core::domain::rstrip(text),
            codex_block(root)?
        )
    };
    let mut document = result
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| RuntimeError::invalid("Merged Codex configuration is invalid"))?;
    // The editions let Codex read the whole AGENTS.md chain in the root. A
    // configuration an older registration wrote has no limit of its own, and
    // Codex cut a long policy at its 32 KiB default, past the memory rules at
    // its end. The release's limit is added only where the project set none.
    let limit = std::str::from_utf8(source)
        .ok()
        .and_then(|source| source.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|source| {
            source
                .get("project_doc_max_bytes")
                .and_then(toml_edit::Item::as_integer)
        });
    if let Some(limit) = limit
        && !document.contains_key("project_doc_max_bytes")
    {
        document["project_doc_max_bytes"] = toml_edit::value(limit);
        let limited = document.to_string();
        if limited.matches(BEGIN).count() != 1 || limited.matches(END).count() != 1 {
            return Err(RuntimeError::conflict("Malformed managed memory MCP block"));
        }
        return Ok(limited.into_bytes());
    }
    Ok(result.into_bytes())
}
/// A Codex registration from before the managed block: the Python bootstrap
/// under `[mcp_servers.harness-memory]`, written into installed projects with
/// either host's argument. A project brought to the native runtime retires the
/// script it starts, so Codex reported the server failing in every session.
/// The table is recognized by the bootstrap's signature, as the JSON
/// registrations are, and only its own text is cut; a server of that name
/// with any other command stays. Text that does not parse is left to the
/// caller, which refuses it.
fn retire_legacy_codex(text: &str) -> String {
    let Ok(document) = toml_edit::ImDocument::parse(text) else {
        return text.into();
    };
    let Some(table) = document
        .get("mcp_servers")
        .and_then(|servers| servers.get("harness-memory"))
        .and_then(toml_edit::Item::as_table)
    else {
        return text.into();
    };
    let expected = if table.contains_key("enabled") { 3 } else { 2 };
    if table.len() != expected
        || table
            .get("enabled")
            .is_some_and(|enabled| enabled.as_bool() != Some(true))
    {
        return text.into();
    }
    let Some(command) = table.get("command").and_then(toml_edit::Item::as_str) else {
        return text.into();
    };
    let Some(arguments) = table
        .get("args")
        .and_then(toml_edit::Item::as_array)
        .and_then(|args| {
            args.iter()
                .map(|value| value.as_str().map(|value| json!(value)))
                .collect::<Option<Vec<_>>>()
        })
    else {
        return text.into();
    };
    if !["codex", "claude"]
        .into_iter()
        .any(|host| launcher(command, &arguments, host))
    {
        return text.into();
    }
    let Some(start) = table.span().map(|span| span.start) else {
        return text.into();
    };
    let Some(end) = table
        .iter()
        .filter_map(|(key, _)| table.get_key_value(key))
        .filter_map(|(_, item)| item.span().map(|span| span.end))
        .max()
    else {
        return text.into();
    };
    let end = text[end..]
        .find('\n')
        .map_or(text.len(), |offset| end + offset + 1);
    format!("{}{}", &text[..start], &text[end..])
}
pub fn long_path(path: &Path) -> Result<PathBuf> {
    #[cfg(not(windows))]
    {
        Ok(path.to_path_buf())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetLongPathNameW(short: *const u16, long: *mut u16, capacity: u32) -> u32;
        }
        let source = path
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>();
        let length = unsafe { GetLongPathNameW(source.as_ptr(), std::ptr::null_mut(), 0) };
        if length == 0 {
            return Ok(path.to_path_buf());
        }
        let mut buffer = vec![0u16; length as usize];
        let actual = unsafe { GetLongPathNameW(source.as_ptr(), buffer.as_mut_ptr(), length) };
        if actual == 0 || actual >= length {
            return Err(RuntimeError::unavailable(
                "Project long path is unavailable",
            ));
        }
        buffer.truncate(actual as usize);
        Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer)))
    }
}
#[derive(Clone)]
struct Plan {
    path: String,
    previous: Option<Vec<u8>>,
    content: Vec<u8>,
    mode: Option<u32>,
}
pub fn register(root: &Path, tools: &[String], dry_run: bool) -> Result<Value> {
    register_at(root, root, tools, dry_run)
}
/// Publish configurations into staging while binding commands to the final
/// destination. Neither registration nor staging grants client trust.
pub fn register_at(
    root: &Path,
    delivery_root: &Path,
    tools: &[String],
    dry_run: bool,
) -> Result<Value> {
    let root = long_path(&safe::absolute(root)?)?;
    let delivery_root = long_path(&safe::absolute(delivery_root)?)?;
    // Rooted admission rejects links without comparing platform-specific path
    // spellings (Windows canonicalize adds an extended-path prefix).
    let delivery = PublicationFiles::new(&delivery_root)?;
    delivery.check()?;
    let mut files = PublicationFiles::new(&root)?;
    let mut selected = BTreeSet::new();
    let hosts = if tools.is_empty() {
        vec!["claude".into(), "cursor".into(), "codex".into()]
    } else {
        tools.to_vec()
    };
    let mut plans = Vec::new();
    for host in hosts {
        host_path(&host)?;
        if !selected.insert(host.clone()) {
            continue;
        }
        let path = host_path(&host)?.to_string();
        let previous = files.read(&path, LIMIT)?;
        let content = merge(
            &path,
            previous.as_deref(),
            &template(&host, &delivery_root)?,
            &delivery_root,
        )?;
        let mode = files.read_mode(&path)?;
        plans.push(Plan {
            path,
            previous,
            content,
            mode,
        });
    }
    let changed = plans
        .iter()
        .filter(|plan| plan.previous.as_deref() != Some(plan.content.as_slice()))
        .map(|plan| plan.path.clone())
        .collect::<Vec<_>>();
    if dry_run {
        return Ok(
            json!({"status":"planned","changed":changed,"dry_run":true,"delivery_root":delivery_root}),
        );
    }
    let engine = root.join(ENGINE);
    let image = safe::read_regular(&engine)?;
    if image.is_empty() {
        return Err(RuntimeError::not_found("Project native engine is absent"));
    }
    let mut applied = Vec::<Plan>::new();
    let outcome = (|| -> Result<()> {
        for plan in &plans {
            if plan.previous.as_deref() == Some(plan.content.as_slice()) {
                continue;
            }
            files.write_expected_mode_checked(
                &plan.path,
                &plan.content,
                plan.previous.as_deref(),
                plan.mode,
                plan.mode.unwrap_or(0o600),
                LIMIT,
            )?;
            applied.push(plan.clone());
        }
        Ok(())
    })();
    if let Err(original) = outcome {
        let mut partial = false;
        for plan in applied.iter().rev() {
            let restored = if let Some(previous) = &plan.previous {
                files.write_expected_mode(
                    &plan.path,
                    previous,
                    Some(&plan.content),
                    plan.mode.unwrap_or(0o600),
                    LIMIT,
                )
            } else {
                files.unlink_expected_limit(&plan.path, &plan.content, LIMIT)
            };
            if restored.is_err() {
                partial = true;
            }
        }
        return Err(RuntimeError::conflict(if partial {
            format!("MCP registration is partial; inspect client configurations ({original})")
        } else {
            format!("MCP registration failed and was rolled back ({original})")
        }));
    }
    Ok(
        json!({"status":"complete","changed":changed,"dry_run":false,"engine_sha256":common::sha256(&image),"delivery_root":delivery_root}),
    )
}
pub fn execute(request: &Value) -> Result<Value> {
    let root = Path::new(request["root"].as_str().unwrap_or("."));
    let delivery_root = request["delivery_root"]
        .as_str()
        .map(Path::new)
        .unwrap_or(root);
    let selected = request.get("tools").or_else(|| request.get("tool"));
    let tools = match selected {
        None | Some(Value::Null) => vec![],
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(String::from)
                    .ok_or_else(|| RuntimeError::invalid("MCP tool names must be text"))
            })
            .collect::<Result<Vec<_>>>()?,
        _ => {
            return Err(RuntimeError::invalid(
                "MCP tool selection must be text or a text array",
            ));
        }
    };
    register_at(
        root,
        delivery_root,
        &tools,
        request["dry_run"].as_bool().unwrap_or(false),
    )
}

#[cfg(test)]
mod native_source_template_tests {
    use super::*;

    #[test]
    fn codex_registration_accepts_empty_and_unrelated_configurations() {
        let root = std::env::temp_dir().join("infra-native-mcp-empty-contract");
        for previous in [
            "",
            "model = \"gpt-5\"\n",
            "[mcp_servers.other]\ncommand = \"other\"\n",
        ] {
            let merged =
                merge(".codex/config.toml", Some(previous.as_bytes()), b"", &root).unwrap();
            let merged = String::from_utf8(merged).unwrap();
            assert!(merged.contains(&codex_block(&root).unwrap()));
            assert!(merged.starts_with(previous));
        }
        let conflict = b"[mcp_servers.harness_memory]\ncommand = \"client-owned\"\n";
        assert!(merge(".codex/config.toml", Some(conflict), b"", &root).is_err());
    }

    #[test]
    fn a_legacy_python_bootstrap_registration_is_retired_from_codex() {
        let root = std::env::temp_dir().join("infra-native-mcp-legacy-contract");
        let bootstrap = "# Harness memory bootstrap v1\nimport os, sys, runpy\nfrom pathlib import Path\nhost = sys.argv[1]\nscript = root / 'memory-bank/scripts/mcp_server.py'\n";
        for host in ["claude", "codex"] {
            let legacy = format!(
                "[features]\nhooks = true\n\n[mcp_servers.harness-memory]\ncommand = \"python3\"\nargs = [\n    \"-c\",\n    \"\"\"\n{bootstrap}\"\"\",\n    \"{host}\",\n]\n\n[mcp_servers.other]\ncommand = \"other\"\n"
            );
            assert!(legacy.parse::<toml_edit::DocumentMut>().is_ok(), "{legacy}");
            let merged = merge(".codex/config.toml", Some(legacy.as_bytes()), b"", &root).unwrap();
            let merged = String::from_utf8(merged).unwrap();
            assert!(!merged.contains("harness-memory"), "{merged}");
            assert!(!merged.contains("bootstrap"), "{merged}");
            assert!(merged.contains("[features]\nhooks = true\n"), "{merged}");
            assert!(
                merged.contains("[mcp_servers.other]\ncommand = \"other\"\n"),
                "{merged}"
            );
            assert!(merged.contains(&codex_block(&root).unwrap()), "{merged}");
            let parsed = merged.parse::<toml_edit::DocumentMut>().unwrap();
            assert!(parsed["mcp_servers"].get("harness-memory").is_none());
        }
        // The release's Codex document limit reaches a configuration written
        // without one; a limit the project set stays.
        let release = include_bytes!("../../../../PHP Core/.codex/config.toml");
        let legacy = "[features]\nhooks = true\n";
        let merged = merge(
            ".codex/config.toml",
            Some(legacy.as_bytes()),
            release,
            &root,
        )
        .unwrap();
        let merged = String::from_utf8(merged).unwrap();
        let parsed = merged.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            parsed["project_doc_max_bytes"].as_integer(),
            Some(131072),
            "{merged}"
        );
        assert_eq!(
            parsed["features"]["hooks"].as_bool(),
            Some(true),
            "{merged}"
        );
        assert!(merged.contains(&codex_block(&root).unwrap()), "{merged}");
        let own = "project_doc_max_bytes = 65536\n";
        let merged = merge(".codex/config.toml", Some(own.as_bytes()), release, &root).unwrap();
        assert!(String::from_utf8(merged).unwrap().starts_with(own));
        // A server of that name with another command is the client's own.
        let foreign =
            "[mcp_servers.harness-memory]\ncommand = \"client-memory\"\nargs = [\"serve\"]\n";
        let merged = merge(".codex/config.toml", Some(foreign.as_bytes()), b"", &root).unwrap();
        assert!(String::from_utf8(merged).unwrap().starts_with(foreign));
    }

    #[test]
    fn shipped_relative_templates_bind_the_selected_native_engine() {
        let root = std::env::temp_dir().join(format!(
            "infra-native-mcp-source-contract-{}",
            std::process::id()
        ));
        for (host, path, source) in [
            (
                "claude",
                ".mcp.json",
                include_bytes!("../../../../PHP Core/.mcp.json").as_slice(),
            ),
            (
                "cursor",
                ".cursor/mcp.json",
                include_bytes!("../../../../PHP Core/.cursor/mcp.json").as_slice(),
            ),
        ] {
            let merged = merge(path, None, source, &root).unwrap();
            let value: Value = serde_json::from_slice(&merged).unwrap();
            assert_eq!(
                value["mcpServers"]["harness-memory"],
                entry(host, &root).unwrap()
            );
        }
        let source = include_bytes!("../../../../PHP Core/.codex/config.toml");
        let merged = merge(".codex/config.toml", None, source, &root).unwrap();
        let merged = String::from_utf8(merged).unwrap();
        assert!(merged.contains(&codex_block(&root).unwrap()));
        assert!(merged.contains("project_doc_max_bytes = 131072"));
    }

    #[test]
    fn a_managed_source_annotation_does_not_authorize_client_edits() {
        let root = std::env::temp_dir().join(format!(
            "infra-native-mcp-edited-contract-{}",
            std::process::id()
        ));
        let source = include_str!("../../../../PHP Core/.codex/config.toml");
        let note = "# Source template: installation writes the portable launch (a bound one on native Windows).";
        assert!(
            source.contains(note),
            "exercise the actual annotated source"
        );
        for edited in [
            source.replace(note, "# A client-edited managed annotation"),
            source.replace("./memory-bank/bin/infra-engine", "./client-owned-server"),
            source.replace("\"--root\", \".\"", "\"--root\", \"another-project\""),
        ] {
            let error = merge(
                ".codex/config.toml",
                Some(edited.as_bytes()),
                source.as_bytes(),
                &root,
            )
            .unwrap_err();
            assert!(error.message.contains("edited"));
        }
    }
}
