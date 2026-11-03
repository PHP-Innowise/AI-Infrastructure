//! Kit 3 catalog, advisory GitHub discovery, selection audit and pinned skills manager.
use crate::{common, process, registry};
use infra_core::{
    Result, RuntimeError,
    bank::publication::PublicationFiles,
    domain::{self, json as codec},
    fs,
};
use serde_json::{Value, json};
#[cfg(windows)]
use std::ffi::OsStr;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
const MANIFEST: &str = ".kit3-manifest.json";
const PACKAGE: &str = "skills@1.5.23";
const AGENTS: [&str; 3] = ["claude-code", "codex", "cursor"];
const TYPES: [&str; 6] = [
    "claude-marketplace",
    "mcp-server",
    "reference-clone",
    "npx-cli",
    "python-cli",
    "discovery-index",
];
fn required<'a>(request: &'a Value, key: &str) -> Result<&'a str> {
    request[key]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| RuntimeError::invalid(format!("{key} is required")))
}
fn text_list(request: &Value, key: &str) -> Result<Vec<String>> {
    match request.get(key) {
        None => Ok(vec![]),
        Some(value) => value
            .as_array()
            .ok_or_else(|| RuntimeError::invalid(format!("{key} must be a list")))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(String::from)
                    .ok_or_else(|| RuntimeError::invalid(format!("{key} must contain text")))
            })
            .collect(),
    }
}
pub fn safe_argument(value: &str) -> Result<String> {
    if value.trim().is_empty() || value.starts_with('-') || value.chars().any(|c| (c as u32) < 32) {
        return Err(RuntimeError::invalid(
            "arguments must be nonempty names, not options or control characters",
        ));
    }
    Ok(value.into())
}
fn expand(path: &str) -> PathBuf {
    if (path == "~" || path.starts_with("~/"))
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(path.strip_prefix("~/").unwrap_or(""));
    }
    PathBuf::from(path)
}
pub fn load_resources(root: &Path, override_path: Option<&Path>) -> Result<Vec<Value>> {
    let path = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("install/open-source-kit/resources.json"));
    let data = common::read_json(&path)?;
    let entries = data["resources"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| RuntimeError::invalid("Resource catalog has no resources"))?;
    let mut seen = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        if !entry.is_object() {
            return Err(RuntimeError::invalid(format!(
                "Resource #{index} must be an object"
            )));
        }
        for key in [
            "id",
            "name",
            "url",
            "category",
            "description",
            "install_type",
        ] {
            if !registry::stated(&entry[key]) {
                return Err(RuntimeError::invalid(format!(
                    "Resource #{index} has no usable {key}"
                )));
            }
        }
        let id = entry["id"].as_str().unwrap_or("");
        if !registry::safe_id(id) || !seen.insert(id.to_string()) {
            return Err(RuntimeError::invalid(format!(
                "Unsafe or duplicate resource ID: {id}"
            )));
        }
        if !TYPES.contains(&entry["install_type"].as_str().unwrap_or("")) {
            return Err(RuntimeError::invalid("Unknown resource install_type"));
        }
        if entry
            .get("verified_command")
            .is_some_and(|value| !value.is_null() && !registry::stated(value))
        {
            return Err(RuntimeError::invalid(
                "Resource verified_command must be nonempty text",
            ));
        }
        if entry.get("tool_compatibility").is_some_and(|value| {
            value
                .as_array()
                .is_none_or(|items| items.iter().any(|item| !item.is_string()))
        }) {
            return Err(RuntimeError::invalid(
                "tool_compatibility must be a list of strings",
            ));
        }
    }
    Ok(entries.clone())
}
pub fn guidance(resource: &Value) -> Result<String> {
    if let Some(command) = resource["verified_command"]
        .as_str()
        .filter(|command| !command.is_empty())
    {
        return Ok(command.into());
    }
    let url = required(resource, "url")?;
    let mut cleaned = infra_core::domain::rstrip(url);
    while let Some(previous) = cleaned.strip_suffix('/') {
        cleaned = previous;
    }
    let owner = cleaned
        .split_once("github.com/")
        .map(|(_, tail)| tail)
        .unwrap_or(url);
    let value=match resource["install_type"].as_str().unwrap_or(""){"claude-marketplace"=>format!("Inside a Claude Code session: /plugin marketplace add {owner}, then /plugin install <name>@<marketplace-slug> - read the repo README first for the exact plugin names it registers."),"mcp-server"=>"Register through Claude Code MCP config (claude mcp add ... or a manual .mcp.json entry); the repo README names the exact servers. Enable only what is needed.".into(),"reference-clone"=>format!("git clone --depth 1 {url} <local-path> and copy only specific reviewed files; do not bulk-copy the tree."),"npx-cli"=>"Run via npx; the repo README names the current package and flags. Review every proposed file before accepting.".into(),"python-cli"=>"Install the named external CLI with uv tool install or pipx install, then run its documented project-scoped setup command.".into(),"discovery-index"=>"This is a curated list, not an installable resource. Browse it, then vet the specific resource with the same checklist before adding it.".into(),_=>return Err(RuntimeError::invalid("Unknown install_type"))};
    Ok(value)
}
pub fn github_repo(url: &str) -> Option<(String, String)> {
    let prefix = url.get(..8)?;
    if !prefix.eq_ignore_ascii_case("https://") {
        return None;
    }
    let tail = &url[8..];
    let (host, path) = tail.split_once('/')?;
    if !host.eq_ignore_ascii_case("github.com") || url.contains('?') || url.contains('#') {
        return None;
    }
    let pattern = regex::Regex::new(r"^([A-Za-z0-9][A-Za-z0-9-]*)/([A-Za-z0-9_.-]+)/?$").ok()?;
    let capture = pattern.captures(path)?;
    let owner = capture[1].to_string();
    let repository = capture[2]
        .strip_suffix(".git")
        .unwrap_or(&capture[2])
        .to_string();
    if repository.is_empty() || [".", ".."].contains(&repository.as_str()) {
        return None;
    }
    Some((owner, repository))
}
fn get(url: &str, accept: &str) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .redirects(0)
        .build();
    let mut current = url.to_string();
    for _ in 0..=5 {
        if !current.starts_with("https://api.github.com/")
            && !current.starts_with("https://raw.githubusercontent.com/")
        {
            return Err(RuntimeError::policy(
                "GitHub redirect endpoint is not admitted",
            ));
        }
        let response = agent
            .get(&current)
            .set("User-Agent", "accelerator-php-kit3-fetcher")
            .set("Accept", accept)
            .call()
            .map_err(|error| {
                RuntimeError::unavailable(format!("GitHub request failed: {error}"))
            })?;
        if (300..400).contains(&response.status()) {
            let location = response
                .header("Location")
                .ok_or_else(|| RuntimeError::unavailable("GitHub redirect has no location"))?;
            current = if location.starts_with('/') && !location.starts_with("//") {
                let authority = current.split('/').take(3).collect::<Vec<_>>().join("/");
                format!("{authority}{location}")
            } else {
                location.to_string()
            };
            continue;
        }
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(RuntimeError::unavailable(
                "GitHub response exceeds its byte limit",
            ));
        }
        return Ok(bytes);
    }
    Err(RuntimeError::unavailable("GitHub redirect limit exceeded"))
}
pub fn extract_candidates(readme: &str, window: usize) -> Result<Vec<Value>> {
    let pattern =
        regex::Regex::new(r"(?s)\x60\x60\x60(?:bash|sh|shell|console|text)?\n(.*?)\x60\x60\x60")
            .map_err(|_| RuntimeError::invalid("Invalid README expression"))?;
    let mut output = Vec::new();
    for found in pattern.captures_iter(readme) {
        let start = found.get(0).map(|m| m.start()).unwrap_or(0);
        let prefix = readme[..start]
            .chars()
            .rev()
            .take(window)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
            .to_lowercase();
        if ![
            "install",
            "npx ",
            "npm ",
            "pip ",
            "pipx",
            "uv tool",
            "plugin",
            "clone",
            "curl",
            "marketplace",
            "setup",
        ]
        .iter()
        .any(|word| prefix.contains(word))
        {
            continue;
        }
        let command = found[1].trim();
        if command.is_empty() {
            continue;
        }
        let context = prefix
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("")
            .trim();
        output.push(json!({"context":context,"command":command}));
    }
    Ok(output)
}
pub fn refresh(url: &str) -> Value {
    let Some((owner, repo)) = github_repo(url) else {
        return json!({"metadata":null,"candidates":[],"errors":[format!("not a github.com URL: {url}")],"advisory":true});
    };
    let mut errors = Vec::new();
    let metadata = (|| -> Result<Value> {
        let raw = get(
            &format!("https://api.github.com/repos/{owner}/{repo}"),
            "application/vnd.github+json",
        )?;
        let data: Value = serde_json::from_slice(&raw)?;
        if !data.is_object() {
            return Err(RuntimeError::invalid(
                "Metadata response must be a JSON object",
            ));
        }
        for field in ["stargazers_count", "forks_count", "open_issues_count"] {
            if data[field].as_u64().is_none() {
                return Err(RuntimeError::invalid(format!(
                    "Invalid metadata field {field}"
                )));
            }
        }
        if !data["pushed_at"].is_string()
            || !data["license"].is_null()
                && (!data["license"].is_object()
                    || !data["license"]["spdx_id"].is_null()
                        && !data["license"]["spdx_id"].is_string())
        {
            return Err(RuntimeError::invalid("Invalid pushed_at/license metadata"));
        }
        Ok(
            json!({"stars":data["stargazers_count"],"forks":data["forks_count"],"pushed_at":data["pushed_at"],"open_issues":data["open_issues_count"],"license_spdx":data["license"]["spdx_id"]}),
        )
    })();
    let metadata = match metadata {
        Ok(data) => Some(data),
        Err(error) => {
            errors.push(error.to_string());
            None
        }
    };
    let readme = get(
        &format!("https://api.github.com/repos/{owner}/{repo}/readme"),
        "application/vnd.github.raw",
    )
    .and_then(|raw| extract_candidates(&String::from_utf8_lossy(&raw), 200));
    let candidates = match readme {
        Ok(data) => data,
        Err(error) => {
            errors.push(error.to_string());
            vec![]
        }
    };
    json!({"metadata":metadata,"candidates":candidates,"errors":errors,"advisory":true})
}
fn pin_values(request: &Value) -> Result<BTreeMap<String, String>> {
    let mut pins = BTreeMap::new();
    if let Some(map) = request["pins"].as_object() {
        for (key, value) in map {
            let value = value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| RuntimeError::invalid("Pin reference must be text"))?;
            if !registry::safe_id(key) {
                return Err(RuntimeError::invalid("Pin ID must be a safe slug"));
            }
            pins.insert(key.clone(), value.trim().into());
        }
    } else {
        for item in text_list(request, "pins")? {
            let (id, reference) = item
                .split_once('=')
                .ok_or_else(|| RuntimeError::invalid("Pin must use ID=REF"))?;
            if !registry::safe_id(id.trim()) || reference.trim().is_empty() {
                return Err(RuntimeError::invalid(
                    "Pin must have safe ID and nonempty REF",
                ));
            }
            pins.insert(id.trim().into(), reference.trim().into());
        }
    }
    Ok(pins)
}
pub fn load_manifest(files: &mut PublicationFiles) -> Result<(Value, Option<Vec<u8>>)> {
    let previous = files.read(MANIFEST, 8 * 1024 * 1024)?;
    let mut manifest = match &previous {
        Some(raw) => codec::parse(
            std::str::from_utf8(raw)
                .map_err(|_| RuntimeError::invalid("Manifest must be UTF-8"))?,
        )?,
        None => json!({"schema_version":1,"kit":"open-source-kit","entries":{}}),
    };
    if !manifest.is_object()
        || manifest["schema_version"].as_u64() != Some(1)
        || manifest["kit"] != "open-source-kit"
    {
        return Err(RuntimeError::invalid(
            "Existing manifest has unsupported schema or kit",
        ));
    }
    if manifest.get("entries").is_none() {
        manifest["entries"] = json!({});
    }
    if manifest["entries"]
        .as_object()
        .is_none_or(|map| map.values().any(|value| !value.is_object()))
    {
        return Err(RuntimeError::invalid(
            "Existing manifest entries must be objects",
        ));
    }
    Ok((manifest, previous))
}
fn target(request: &Value) -> Result<PathBuf> {
    let target = expand(request["target"].as_str().unwrap_or("."));
    let target = infra_core::fs::absolute(&target)?;
    if !target.is_dir() {
        return Err(RuntimeError::invalid("Target is not an existing directory"));
    }
    Ok(target)
}
fn selected<'a>(resources: &'a [Value], request: &Value) -> Result<Vec<&'a Value>> {
    let ids = if let Some(selection) = request["select"].as_str() {
        if selection == "all" {
            resources
                .iter()
                .filter_map(|resource| resource["id"].as_str())
                .map(String::from)
                .collect()
        } else {
            selection
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(String::from)
                .collect()
        }
    } else {
        text_list(request, "ids")?
    };
    ids.iter()
        .map(|id| {
            resources
                .iter()
                .find(|resource| resource["id"].as_str() == Some(id))
                .ok_or_else(|| RuntimeError::invalid(format!("Unknown resource id: {id}")))
        })
        .collect()
}
fn record(root: &Path, resources: &[Value], request: &Value) -> Result<Value> {
    let picks = selected(resources, request)?;
    let pins = pin_values(request)?;
    for id in pins.keys() {
        if !picks
            .iter()
            .any(|resource| resource["id"].as_str() == Some(id))
        {
            return Err(RuntimeError::invalid(format!(
                "Pin names unselected resource: {id}"
            )));
        }
    }
    if picks.is_empty() {
        return Ok(json!({"selected":[],"made":false,"reason":"No resources selected"}));
    }
    let target = target(request)?;
    let mut files = PublicationFiles::new(&target)?;
    let (mut manifest, previous) = load_manifest(&mut files)?;
    let previous_mode = files.read_mode(MANIFEST)?;
    let today = domain::clock::local_today().to_string();
    let mut reports = Vec::new();
    for resource in picks {
        let id = resource["id"].as_str().unwrap_or("");
        let old = manifest["entries"][id].clone();
        let reference = pins
            .get(id)
            .cloned()
            .or_else(|| {
                old["pinned_ref"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .map(String::from)
            })
            .or_else(|| resource["pinned_ref"].as_str().map(String::from));
        let status = registry::status_for(root, resource)?;
        let guide = guidance(resource)?;
        let source = if resource["verified_command"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
        {
            "verified_command"
        } else {
            "generic_template"
        };
        let mut entry = json!({"name":resource["name"],"url":resource["url"],"category":resource["category"],"license":resource["license"],"reviewed_date":resource["reviewed_date"],"selected_date":today,"pinned_ref":reference,"install_method":resource["install_type"],"install_guidance":guide,"install_guidance_source":source,"install_guidance_fetched_at":null,"risk_notes":resource["risk_notes"],"review":{"status":status,"reviewed":status.is_some()}});
        if request["refresh"] == true && resource["category"] != "discovery-index" {
            let refreshed = refresh(resource["url"].as_str().unwrap_or(""));
            entry["readme_refresh"] = json!({"attempted_at":today,"advisory":true,"candidates":refreshed["candidates"],"errors":refreshed["errors"]});
        }
        manifest["entries"][id] = entry.clone();
        reports.push(json!({"id":id,"entry":entry,"installation_verified":false}));
    }
    let dry = request["dry_run"] == true;
    if !dry {
        let mut payload = codec::serialize(&manifest, true, true, true, true)?.into_bytes();
        payload.push(b'\n');
        files.write_expected_mode_checked(
            MANIFEST,
            &payload,
            previous.as_deref(),
            previous_mode,
            previous_mode.unwrap_or(0o600),
            8 * 1024 * 1024,
        )?;
    }
    Ok(
        json!({"selected":reports,"manifest":manifest,"path":target.join(MANIFEST),"made":!dry,"dry_run":dry,"installation_verified":false}),
    )
}

fn percent_decode(text: &str) -> Result<String> {
    let mut bytes = Vec::new();
    let raw = text.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%' {
            if index + 2 >= raw.len() {
                return Err(RuntimeError::invalid("Invalid URI escape"));
            }
            let pair = std::str::from_utf8(&raw[index + 1..index + 3])
                .map_err(|_| RuntimeError::invalid("Invalid URI escape"))?;
            bytes.push(
                u8::from_str_radix(pair, 16)
                    .map_err(|_| RuntimeError::invalid("Invalid URI escape"))?,
            );
            index += 3;
        } else {
            bytes.push(raw[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).map_err(|_| RuntimeError::invalid("Local URI path must be UTF-8"))
}
fn https_source(value: &str) -> bool {
    let Some((scheme, tail)) = value.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let authority = tail.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty() && !authority.contains('@') && !authority.chars().any(char::is_whitespace)
}
pub fn resolve_source(root: &Path, value: &str, resources: &[Value]) -> Result<String> {
    let mut value = safe_argument(value)?;
    if let Some(resource) = resources
        .iter()
        .find(|resource| resource["id"].as_str() == Some(value.as_str()))
    {
        registry::status_for(root, resource)?;
        value = resource["skills_source"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                RuntimeError::invalid(
                    "This catalog resource has no Agent Skills install path; read its manual guide",
                )
            })?
            .into();
    }
    safe_argument(&value)?;
    if value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with('/')
        || value.starts_with("~/")
    {
        let path = expand(&value);
        if !path.is_dir() {
            return Err(RuntimeError::invalid("Local source is not a directory"));
        }
        return std::fs::canonicalize(path)?
            .into_os_string()
            .into_string()
            .map_err(|_| RuntimeError::invalid("Local source is not UTF-8"));
    }
    let owner_repo =
        regex::Regex::new(r"^[A-Za-z0-9_][A-Za-z0-9_.-]*/[A-Za-z0-9_][A-Za-z0-9_.-]*$")
            .map_err(|_| RuntimeError::invalid("Invalid source expression"))?;
    if owner_repo.is_match(&value) || https_source(&value) {
        return Ok(value);
    }
    if let Some(path) = value.strip_prefix("file://")
        && path.starts_with('/')
        && Path::new(&percent_decode(path)?).is_dir()
    {
        return Ok(value);
    }
    Err(RuntimeError::invalid(
        "Source must be a supported catalog ID, owner/repo, HTTPS URL, local Git file URI or local skill directory",
    ))
}
fn agents(request: &Value) -> Result<Vec<String>> {
    let agents = if let Some(agent) = request["agent"].as_str() {
        vec![agent.into()]
    } else {
        text_list(request, "agent")?
    };
    if agents.iter().any(|agent| !AGENTS.contains(&agent.as_str())) {
        return Err(RuntimeError::invalid("Unsupported skills agent"));
    }
    Ok(agents)
}
pub fn native_command(
    root: &Path,
    operation: &str,
    request: &Value,
    resources: &[Value],
) -> Result<Vec<String>> {
    let operation = match operation {
        "install" => "add",
        "uninstall" => "remove",
        other => other,
    };
    if !["find", "add", "remove", "update", "list"].contains(&operation) {
        return Err(RuntimeError::invalid("Unknown native skills operation"));
    }
    let mut command = vec![
        "npx".into(),
        "--yes".into(),
        PACKAGE.into(),
        operation.into(),
    ];
    if operation == "find" {
        if let Some(query) = request["query"].as_str().filter(|query| !query.is_empty()) {
            command.push(safe_argument(query)?);
        }
        return Ok(command);
    }
    if operation == "add" {
        command.push(resolve_source(
            root,
            required(request, "source")?,
            resources,
        )?);
        for skill in text_list(request, "skill")? {
            command.extend(["--skill".into(), safe_argument(&skill)?]);
        }
        if request["list"] == true {
            command.push("--list".into());
        }
        if request["copy"] == true {
            command.push("--copy".into());
        }
    } else if ["remove", "update"].contains(&operation) {
        for name in text_list(request, "names")? {
            command.push(safe_argument(&name)?);
        }
    }
    if operation == "list" && request["json"] == true {
        command.push("--json".into());
    }
    for agent in agents(request)? {
        command.extend(["--agent".into(), agent]);
    }
    if request["global"] == true || request["global_scope"] == true {
        command.push("--global".into());
    } else if operation == "update" {
        command.push("--project".into());
    }
    if request["yes"] == true {
        command.push("--yes".into());
    }
    Ok(command)
}
#[cfg(windows)]
fn native_launch(command: &[String], cwd: &Path) -> Result<(OsString, Vec<OsString>)> {
    fn ordinary(path: &Path) -> bool {
        std::fs::symlink_metadata(path)
            .is_ok_and(|image| image.is_file() && !infra_core::fs::linked(&image))
    }
    let paths = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        cwd.join(path)
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let suffixes = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|suffix| {
            [".com", ".exe", ".bat", ".cmd"].contains(&suffix.to_ascii_lowercase().as_str())
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut selected = None;
    'outer: for directory in &paths {
        for suffix in &suffixes {
            let candidate = directory.join(format!("npx{suffix}"));
            if ordinary(&candidate) {
                selected = Some(infra_core::fs::absolute(&candidate)?);
                break 'outer;
            }
        }
    }
    let selected = selected.ok_or_else(|| {
        RuntimeError::unavailable("Node.js with npx is required for skill management")
    })?;
    let extension = selected
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if ["exe", "com"].contains(&extension.as_str()) {
        return Ok((
            selected.into_os_string(),
            command[1..].iter().map(OsString::from).collect(),
        ));
    }
    // Admit the standard npm layout; never parse or execute a command shim.
    let directory = std::fs::canonicalize(
        selected
            .parent()
            .ok_or_else(|| RuntimeError::unavailable("npx installation root unavailable"))?,
    )?;
    let mut files = PublicationFiles::new(&directory)?;
    let raw = files
        .read("node_modules/npm/package.json", 1024 * 1024)?
        .ok_or_else(|| {
            RuntimeError::unavailable("Native npx requires the standard npm installation layout")
        })?;
    let package = codec::parse(
        std::str::from_utf8(&raw)
            .map_err(|_| RuntimeError::invalid("npm package metadata is not UTF-8"))?,
    )?;
    if package["name"] != "npm"
        || !package["bin"]["npx"]
            .as_str()
            .is_some_and(|name| name.strip_prefix("./").unwrap_or(name) == "bin/npx-cli.js")
    {
        return Err(RuntimeError::policy(
            "npm npx entry point is not the admitted standard layout",
        ));
    }
    let script = directory.join("node_modules/npm/bin/npx-cli.js");
    if files
        .read("node_modules/npm/bin/npx-cli.js", 8 * 1024 * 1024)?
        .is_none()
    {
        return Err(RuntimeError::unavailable(
            "npm npx JavaScript entry point absent",
        ));
    }
    let adjacent = directory.join("node.exe");
    let node = if ordinary(&adjacent) {
        adjacent
    } else {
        paths
            .iter()
            .map(|directory| directory.join("node.exe"))
            .find(|path| ordinary(path))
            .ok_or_else(|| {
                RuntimeError::unavailable("node.exe is required for native npx transport")
            })?
    };
    let node = std::fs::canonicalize(&node)?;
    let mut arguments = vec![script.into_os_string()];
    arguments.extend(command[1..].iter().map(OsString::from));
    files.check()?;
    Ok((node.into_os_string(), arguments))
}
#[cfg(not(windows))]
fn native_launch(command: &[String], cwd: &Path) -> Result<(OsString, Vec<OsString>)> {
    let _ = cwd;
    let program = command
        .first()
        .ok_or_else(|| RuntimeError::invalid("Native manager command is empty"))?;
    Ok((
        program.into(),
        command[1..].iter().map(OsString::from).collect(),
    ))
}
fn run_native(root: &Path, operation: &str, request: &Value, resources: &[Value]) -> Result<Value> {
    let target = std::fs::canonicalize(target(request)?)?;
    let command = native_command(root, operation, request, resources)?;
    if request["dry_run"] == true {
        return Ok(json!({"dry_run":true,"made":false,"target":target,"command":command}));
    }
    let (program, arguments) = native_launch(&command, &target)?;
    if request["capture"] == true || request["json"] == true {
        let output = process::run(
            &program,
            &arguments,
            &target,
            None,
            Duration::from_secs(request["timeout"].as_u64().unwrap_or(600)),
            8 * 1024 * 1024,
        )?;
        return Ok(
            json!({"exit_code":output.code,"command":command,"target":target,"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr),"external_manager":true,"installation_verified":false}),
        );
    }
    #[cfg(windows)]
    let code = process::run_inherited(&program, &arguments, &target, &BTreeMap::new())?;
    #[cfg(not(windows))]let code=std::process::Command::new(&program).args(&arguments).current_dir(&target).status().map_err(|_|RuntimeError::unavailable("Node.js with npx is required for skill management; catalog browsing works without it"))?.code().unwrap_or(1);
    Ok(
        json!({"exit_code":code,"command":command,"target":target,"stdio_inherited":true,"external_manager":true,"installation_verified":false}),
    )
}
fn catalog_rows(root: &Path, resources: &[Value], request: &Value) -> Result<Vec<Value>> {
    let query = infra_core::query::folded(request["query"].as_str().unwrap_or(""));
    let terms = query.split_whitespace().collect::<Vec<_>>();
    let agent = request["agent"].as_str();
    if agent.is_some_and(|agent| !AGENTS.contains(&agent)) {
        return Err(RuntimeError::invalid("Unsupported catalog agent"));
    }
    let category = request["category"].as_str();
    let review = request["review"].as_str();
    if review.is_some_and(|value| {
        !["unreviewed", "clear", "open_questions", "known_risks"].contains(&value)
    }) {
        return Err(RuntimeError::invalid("Unsupported review filter"));
    }
    let mut output = Vec::new();
    for resource in resources {
        if agent.is_some_and(|agent| {
            !resource["tool_compatibility"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|value| value.as_str() == Some(agent))
        }) || category.is_some_and(|category| resource["category"].as_str() != Some(category))
        {
            continue;
        }
        let text = ["id", "name", "description", "category"]
            .iter()
            .map(|key| resource[key].as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            .join(" ");
        let haystack = infra_core::query::folded(&text);
        if !terms.iter().all(|term| haystack.contains(term)) {
            continue;
        }
        let status = registry::status_for(root, resource)?.unwrap_or_else(|| "unreviewed".into());
        if review.is_some_and(|review| review != status) {
            continue;
        }
        let mut item = resource.clone();
        item["review_status"] = status.into();
        output.push(item);
    }
    Ok(output)
}
fn catalog_html(root: &Path, resources: &[Value]) -> Result<Vec<u8>> {
    let mut rows = Vec::new();
    for resource in resources {
        if !https_source(required(resource, "url")?) {
            return Err(RuntimeError::policy("Catalog URL must use HTTPS"));
        }
        let status = registry::status_for(root, resource)?;
        let dossier = if status.is_some() {
            Some(common::read_json(
                &root
                    .join("install/open-source-kit/registry")
                    .join(format!("{}.json", required(resource, "id")?)),
            )?)
        } else {
            None
        };
        let source = match resource.get("skills_source") {
            Some(value) if !value.is_null() => Some(
                value
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| RuntimeError::invalid("skills_source must be nonempty text"))?,
            ),
            _ => None,
        };
        let command = source
            .map(|source| {
                format!(
                    "./kit3 add {} --agent codex",
                    infra_core::native::quote(source)
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "./kit3 show {}",
                    infra_core::native::quote(resource["id"].as_str().unwrap_or(""))
                )
            });
        let mut row = resource.clone();
        row["review_status"] = json!(status);
        row["dossier"] = json!(dossier);
        row["command"] = command.into();
        row["install_guidance"] = guidance(resource)?.into();
        rows.push(row);
    }
    let data = codec::serialize(&json!(rows), false, true, false, false)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let template = common::text(std::fs::read(
        root.join("install/open-source-kit/web/index.html"),
    )?)?;
    if template.matches("__KIT3_DATA__").count() != 1 {
        return Err(RuntimeError::invalid(
            "Catalog template must contain exactly one data placeholder",
        ));
    }
    Ok(template.replace("__KIT3_DATA__", &data).into_bytes())
}
pub fn http_catalog(root: &Path) -> Result<Vec<u8>> {
    let resources = load_resources(root, None)?;
    catalog_html(root, &resources)
}
fn site_assets(
    root: &Path,
    resources: &[Value],
) -> Result<BTreeMap<String, (Vec<u8>, &'static str)>> {
    let mut files = PublicationFiles::new(root)?;
    let verified = crate::wasm_assets::read_verified(&mut files)?;
    let boot = files
        .read("harness/web/boot.js", crate::wasm_assets::ARTIFACT_LIMIT)?
        .ok_or_else(|| RuntimeError::unavailable("UI loader is absent"))?;
    let html = catalog_html(root, resources)?;
    verified.recheck(&mut files)?;
    files.check()?;
    let mut assets = BTreeMap::new();
    assets.insert("/".into(), (html, "text/html; charset=utf-8"));
    assets.insert("/boot.js".into(), (boot, "text/javascript; charset=utf-8"));
    assets.insert(
        "/assets/infra_ui.js".into(),
        (verified.javascript, "text/javascript; charset=utf-8"),
    );
    assets.insert(
        "/assets/infra_ui_bg.wasm".into(),
        (verified.wasm, "application/wasm"),
    );
    Ok(assets)
}
fn build(root: &Path, resources: &[Value], request: &Value) -> Result<Value> {
    let output = request["output"]
        .as_str()
        .map(expand)
        .unwrap_or_else(|| root.join("output/kit3-site"));
    let mut assets = site_assets(root, resources)?;
    let standalone = {
        let html = assets
            .get("/")
            .ok_or_else(|| RuntimeError::unavailable("Catalog HTML snapshot absent"))?;
        let javascript = assets
            .get("/assets/infra_ui.js")
            .ok_or_else(|| RuntimeError::unavailable("UI module snapshot absent"))?;
        let wasm = assets
            .get("/assets/infra_ui_bg.wasm")
            .ok_or_else(|| RuntimeError::unavailable("UI binary snapshot absent"))?;
        crate::wasm_assets::standalone_html(&html.0, &javascript.0, &wasm.0)?
    };
    assets.insert("/".into(), (standalone, "text/html; charset=utf-8"));
    fs::ensure_directory(&output, false)?;
    let mut files = PublicationFiles::new(&output)?;
    let mut previous = BTreeMap::new();
    for (route, (bytes, _)) in &assets {
        let name = if route == "/" {
            "index.html"
        } else {
            route.strip_prefix('/').unwrap_or(route)
        };
        let limit = if name == "index.html" {
            crate::wasm_assets::HTML_LIMIT
        } else {
            crate::wasm_assets::ARTIFACT_LIMIT
        };
        if bytes.len() > limit {
            return Err(RuntimeError::invalid("Site artifact limit exceeded"));
        }
        previous.insert(name.to_string(), files.read(name, limit)?);
    }
    let mut made = vec![];
    for route in [
        "/boot.js",
        "/assets/infra_ui.js",
        "/assets/infra_ui_bg.wasm",
        "/",
    ] {
        let (bytes, _) = assets
            .get(route)
            .ok_or_else(|| RuntimeError::unavailable("Site artifact snapshot incomplete"))?;
        let name = if route == "/" {
            "index.html"
        } else {
            route.strip_prefix('/').unwrap_or(route)
        };
        let limit = if name == "index.html" {
            crate::wasm_assets::HTML_LIMIT
        } else {
            crate::wasm_assets::ARTIFACT_LIMIT
        };
        files.write_expected_limit(
            name,
            bytes,
            previous.get(name).and_then(|bytes| bytes.as_deref()),
            limit,
        )?;
        made.push(name);
    }
    Ok(
        json!({"output":output.join("index.html"),"bytes":assets["/"].0.len(),"resources":resources.len(),"assets":made,"ui":"rust-wasm"}),
    )
}
fn serve(root: &Path, resources: &[Value], request: &Value) -> Result<Value> {
    let port = request
        .get("port")
        .map(|value| {
            value
                .as_u64()
                .filter(|port| *port <= 65535)
                .ok_or_else(|| RuntimeError::invalid("port must be between 0 and 65535"))
        })
        .transpose()?
        .unwrap_or(8765);
    let assets = site_assets(root, resources)?;
    let listener = std::net::TcpListener::bind(("127.0.0.1", port as u16))?;
    println!(
        "Kit 3 catalog: http://127.0.0.1:{}",
        listener.local_addr()?.port()
    );
    loop {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut header = Vec::new();
        let mut reader = std::io::BufReader::new(&stream);
        (&mut reader).take(8193).read_until(b'\n', &mut header)?;
        let line = String::from_utf8_lossy(&header);
        let words = line.split_whitespace().collect::<Vec<_>>();
        let method = words.first().copied().unwrap_or("");
        let path = words
            .get(1)
            .copied()
            .unwrap_or("")
            .split('?')
            .next()
            .unwrap_or("");
        let path = if path == "/index.html" { "/" } else { path };
        let (status, body, content_type) = if header.len() > 8192 {
            (
                "414 URI Too Long",
                b"Request line too long".as_slice(),
                "text/plain",
            )
        } else if !["GET", "HEAD"].contains(&method) {
            (
                "501 Not Implemented",
                b"Unsupported method".as_slice(),
                "text/plain",
            )
        } else if let Some((bytes, kind)) = assets.get(path) {
            ("200 OK", bytes.as_slice(), *kind)
        } else {
            ("404 Not Found", b"Not found".as_slice(), "text/plain")
        };
        drop(reader);
        let headers = format!(
            "HTTP/1.0 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(headers.as_bytes());
        if method != "HEAD" {
            let _ = stream.write_all(body);
        }
        let _ = stream.flush();
    }
}
pub fn execute(op: &str, request: &Value) -> Result<Value> {
    let root = PathBuf::from(required(request, "root")?);
    let override_path = request["resources"].as_str().map(Path::new);
    let resources = load_resources(&root, override_path)?;
    match op {
        "catalog" | "search" => Ok(json!(catalog_rows(&root, &resources, request)?)),
        "find" if request["remote"] != true => Ok(json!(catalog_rows(&root, &resources, request)?)),
        "find" => {
            if ["agent", "category", "review"]
                .iter()
                .any(|key| request.get(*key).is_some())
                || request["json"] == true
            {
                return Err(RuntimeError::invalid(
                    "Local catalog filters and JSON cannot be combined with remote find",
                ));
            }
            run_native(&root, "find", request, &resources)
        }
        "show" => {
            let id = required(request, "id")?;
            let mut item = resources
                .iter()
                .find(|resource| resource["id"] == id)
                .cloned()
                .ok_or_else(|| RuntimeError::not_found("Unknown resource id"))?;
            item["review_status"] = registry::status_for(&root, &item)?
                .unwrap_or_else(|| "unreviewed".into())
                .into();
            item["install_guidance"] = guidance(&item)?.into();
            Ok(item)
        }
        "record" | "select" => record(&root, &resources, request),
        "recorded" => {
            if request.get("agent").is_some_and(|value| {
                !value.is_null() && value.as_array().is_none_or(|items| !items.is_empty())
            }) || request["global"] == true
                || request["global_scope"] == true
            {
                return Err(RuntimeError::invalid(
                    "Recorded project selection cannot be combined with native agent/global filters",
                ));
            }
            let mut files = PublicationFiles::new(&target(request)?)?;
            Ok(load_manifest(&mut files)?.0)
        }
        "list" if request["recorded"] == true => {
            let mut forwarded = request.clone();
            if let Some(map) = forwarded.as_object_mut() {
                map.remove("recorded");
            }
            execute("recorded", &forwarded)
        }
        "add" | "install" | "remove" | "uninstall" | "update" | "list" => {
            run_native(&root, op, request, &resources)
        }
        "fetch" | "refresh" => {
            let url = if let Some(url) = request["url"].as_str() {
                url.to_string()
            } else {
                let id = required(request, "id")?;
                required(
                    resources
                        .iter()
                        .find(|resource| resource["id"] == id)
                        .ok_or_else(|| RuntimeError::not_found("Unknown resource id"))?,
                    "url",
                )?
                .to_string()
            };
            let mut report = refresh(&url);
            report["url"] = url.into();
            report["exit_code"] = (if report["metadata"].is_null()
                && report["candidates"].as_array().is_none_or(Vec::is_empty)
                && report["errors"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
            {
                1
            } else {
                0
            })
            .into();
            Ok(report)
        }
        "build" => build(&root, &resources, request),
        "serve" => serve(&root, &resources, request),
        _ => Err(RuntimeError::invalid("Unknown Kit 3 operation")),
    }
}
