//! Read-only runtime parity for installed projects and monorepo siblings.
use crate::{
    Result, Roots, RuntimeConfig, RuntimeContext, RuntimeError, fs, index, query, strict_json,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

// Embed the canonical sources directly; installed engines remain self-contained.
const MIRROR_DEFAULT: &str =
    include_str!("../../../../PHP Core/memory-bank/config/mirror-rules.json");
const CORE_DEFAULT: &str = include_str!("../../../../install/core-manifest.json");
const OPTIONAL_NATIVE: &[&str] = &[
    "memory-bank/bin/infra-engine",
    "memory-bank/bin/infra-engine.exe",
    "memory-bank/runtime-capabilities.json",
];
const EDITIONS: &[(&str, &str)] = &[
    ("Laravel", "Laravel"),
    ("Symfony", "Symfony"),
    ("PHP Core", "PHP Core"),
    ("WordPress", "Cms/wordpress"),
];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ParityRequest {
    pub skills_only: bool,
    pub cross_edition: bool,
}
#[derive(Clone, Debug)]
pub struct ParityService {
    context: RuntimeContext,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrossParity {
    pub drift: Vec<Value>,
    pub not_materialized: Vec<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MirrorRules {
    version: u32,
    classes: Vec<MirrorClass>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MirrorClass {
    name: String,
    canonical: String,
    mirrors: serde_json::Map<String, Value>,
    #[serde(default)]
    only: Option<Vec<String>>,
    #[serde(default)]
    skip: Vec<String>,
    #[serde(default)]
    skip_by_framework: BTreeMap<String, Vec<String>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MirrorSpec {
    pub transform: String,
    pub replacements: Vec<(String, String)>,
    pub skip: Vec<String>,
    pub description_overrides: BTreeMap<String, String>,
    pub quote_description: bool,
}
impl Default for MirrorSpec {
    fn default() -> Self {
        Self {
            transform: "copy".into(),
            replacements: Vec::new(),
            skip: Vec::new(),
            description_overrides: BTreeMap::new(),
            quote_description: true,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
struct CoreRules {
    version: u32,
    canonical: String,
    editions: serde_json::Map<String, Value>,
    includes: Vec<String>,
    #[serde(default)]
    allowed_drift: BTreeMap<String, String>,
}

impl ParityService {
    pub fn new(context: RuntimeContext) -> Self {
        Self { context }
    }
    pub fn run(&self, request: ParityRequest) -> Result<Value> {
        let repository = &self.context.roots.tooling;
        if request.cross_edition {
            return match cross_edition_report(repository)? {
                None => Ok(json!({"valid":true,"skipped":true,"reason":"standalone-edition"})),
                Some(report) => Ok(
                    json!({"valid":report.drift.is_empty(),"skipped":false,"drift":report.drift,"not_materialized":report.not_materialized}),
                ),
            };
        }
        let config = RuntimeConfig::load(&self.context.roots)?;
        let configured = config.value["canonical_edition"]
            .as_str()
            .unwrap_or(".agents");
        let canonical = index::effective_canonical_edition(repository, configured);
        let (drift, _) = index::skill_parity(&self.context.roots, &canonical)?;
        let mut result =
            json!({"valid":drift.is_empty(),"canonical_edition":canonical.clone(),"drift":drift});
        if canonical != configured {
            result["configured_canonical_edition"] = json!(configured);
        }
        if !request.skills_only {
            let mirror = full_mirror_drift_with_framework(
                repository,
                config.value["framework"].as_str().unwrap_or(""),
            )?;
            result["valid"] = json!(result["valid"] == true && mirror.is_empty());
            result["mirror_drift"] = json!(mirror);
        }
        Ok(result)
    }
}
fn relative(value: &str) -> Result<PathBuf> {
    if value.is_empty() || value.contains(['\0', '\\', ':']) {
        return Err(RuntimeError::policy("Invalid parity source key"));
    }
    let result = Path::new(value);
    if result
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(RuntimeError::policy("Parity source escapes its root"));
    }
    Ok(result.to_path_buf())
}
fn ignored(value: &str) -> bool {
    value
        .split('/')
        .any(|part| part == "__pycache__" || part == ".DS_Store")
        || value.ends_with(".pyc")
}
fn skipped(value: &str, skips: &[String]) -> bool {
    skips.iter().any(|entry| {
        if entry.ends_with('/') {
            value.starts_with(entry)
        } else {
            value == entry
        }
    })
}
fn read_resource(root: &Path, name: &str, fallback: &str) -> Result<Value> {
    let filename = root.join(relative(name)?);
    let raw = match std::fs::symlink_metadata(&filename) {
        Ok(_) => fs::read_regular(&filename)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fallback.as_bytes().to_vec(),
        Err(error) => return Err(error.into()),
    };
    strict_json::parse(&raw, 4 * 1024 * 1024, 96, 1_000_000)
}
fn rules(repository: &Path) -> Result<MirrorRules> {
    let rules: MirrorRules = serde_json::from_value(read_resource(
        repository,
        "memory-bank/config/mirror-rules.json",
        MIRROR_DEFAULT,
    )?)?;
    if rules.version != 1 || rules.classes.is_empty() {
        return Err(RuntimeError::invalid("Unsupported runtime mirror rules"));
    }
    for class in &rules.classes {
        if class.name.is_empty() || class.mirrors.is_empty() {
            return Err(RuntimeError::invalid("Empty runtime mirror class"));
        }
        relative(&class.canonical)?;
        if let Some(only) = &class.only {
            for name in only {
                relative(name)?;
            }
        }
        for (name, spec) in &class.mirrors {
            relative(name)?;
            let spec: MirrorSpec = serde_json::from_value(spec.clone())?;
            if !["copy", "cursor-command", "cursor-agent"].contains(&spec.transform.as_str()) {
                return Err(RuntimeError::invalid(
                    "Unknown runtime mirror transformation",
                ));
            }
        }
    }
    Ok(rules)
}
fn inventory(root: &Path, only: Option<&[String]>) -> Result<Vec<(String, PathBuf)>> {
    if let Some(only) = only {
        let mut found = Vec::new();
        for name in only {
            let file = root.join(relative(name)?);
            if file.is_file() {
                found.push((name.clone(), file));
            }
        }
        return Ok(found);
    }
    fn walk(base: &Path, folder: &Path, found: &mut Vec<(String, PathBuf)>) -> Result<()> {
        for entry in std::fs::read_dir(folder)? {
            let entry = entry?;
            let file = entry.path();
            let image = std::fs::symlink_metadata(&file)?;
            if fs::linked_at(&file, &image)? {
                continue;
            }
            let relative_path = file
                .strip_prefix(base)
                .map_err(|_| RuntimeError::policy("Parity inventory escapes its root"))?;
            let name = relative_path
                .to_str()
                .ok_or_else(|| RuntimeError::invalid("Parity source path is not UTF-8"))?;
            #[cfg(windows)]
            let name = name.replace('\\', "/");
            #[cfg(not(windows))]
            let name = name.to_string();
            relative(&name)?;
            if ignored(&name) {
                continue;
            }
            if image.is_dir() {
                walk(base, &file, found)?;
            } else if image.is_file() {
                found.push((name, file));
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    if !root.is_dir() {
        return Ok(found);
    }
    walk(root, root, &mut found)?;
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
}
fn python_space(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
}
fn python_strip(text: &str) -> &str {
    let start = text
        .char_indices()
        .find(|(_, character)| !python_space(*character))
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    crate::domain::rstrip(&text[start..])
}
fn python_lines(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if matches!(
            character,
            '\n' | '\r'
                | '\u{b}'
                | '\u{c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            result.push(&text[start..index]);
            start = index + character.len_utf8();
            if character == '\r' && characters.peek().is_some_and(|(_, next)| *next == '\n') {
                characters.next();
                start += 1;
            }
        }
    }
    if start < text.len() {
        result.push(&text[start..]);
    }
    result
}
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    if !text.starts_with("---\n") {
        return None;
    }
    let end = text[4..].find("\n---\n")? + 4;
    Some((&text[4..end + 1], &text[end + 5..]))
}
fn replacements(mut text: String, spec: &MirrorSpec) -> String {
    for (old, new) in &spec.replacements {
        text = text.replace(old, new);
    }
    text
}
/// The same byte-producing transformations used by the runtime mirror contract.
pub fn transform(relative: &str, text: &str, spec: &MirrorSpec) -> Result<String> {
    if spec.transform == "copy" {
        return Ok(replacements(text.to_string(), spec));
    }
    let Some((frontmatter, body)) = split_frontmatter(text) else {
        return Ok(replacements(text.to_string(), spec));
    };
    let body = replacements(body.to_string(), spec);
    if spec.transform == "cursor-agent" {
        let mut kept = String::new();
        for line in python_lines(frontmatter) {
            if !["model:", "invokes:", "phase:"]
                .iter()
                .any(|key| line.starts_with(key))
            {
                kept.push_str(line);
                kept.push('\n');
            }
        }
        return Ok(format!("---\n{kept}---\n{body}"));
    }
    if spec.transform != "cursor-command" {
        return Err(RuntimeError::invalid(
            "Unknown runtime mirror transformation",
        ));
    }
    if frontmatter
        .split('\n')
        .any(|line| line.starts_with("name:"))
    {
        return Ok(format!("---\n{frontmatter}---\n{body}"));
    }
    let name = relative.rsplit('/').next().unwrap_or(relative);
    let stem = name.strip_suffix(".md").unwrap_or(name);
    let description = if let Some(value) = spec.description_overrides.get(name) {
        value.clone()
    } else {
        python_lines(&body)
            .into_iter()
            .map(python_strip)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .ok_or_else(|| {
                RuntimeError::invalid(format!("{relative}: cannot derive a command description"))
            })?
            .to_string()
    };
    let description = if spec.quote_description {
        format!("\"{description}\"")
    } else {
        description
    };
    Ok(format!(
        "---\nname: {stem}\ndescription: {description}\n---\n{body}"
    ))
}
pub fn full_mirror_drift(repository: &Path) -> Result<Vec<Value>> {
    let roots = Roots::installed(repository)?;
    let config = RuntimeConfig::load(&roots)?;
    full_mirror_drift_with_framework(repository, config.value["framework"].as_str().unwrap_or(""))
}
pub fn full_mirror_drift_with_framework(repository: &Path, framework: &str) -> Result<Vec<Value>> {
    let rules = rules(repository)?;
    let mut drift = Vec::new();
    for class in &rules.classes {
        let canonical = repository.join(&class.canonical);
        if !canonical.is_dir() {
            continue;
        }
        for (mirror_name, spec) in &class.mirrors {
            let mirror = repository.join(mirror_name);
            if !mirror.is_dir() {
                continue;
            }
            let spec: MirrorSpec = serde_json::from_value(spec.clone())?;
            let mut skips = class.skip.clone();
            skips.extend(spec.skip.clone());
            if let Some(extra) = class.skip_by_framework.get(framework) {
                skips.extend(extra.clone());
            }
            let sources = inventory(&canonical, class.only.as_deref())?
                .into_iter()
                .filter(|(name, _)| !skipped(name, &skips))
                .collect::<Vec<_>>();
            // Reverse inventory is admitted before deciding that a class was not installed.
            let reverse = if class.only.is_none() {
                inventory(&mirror, None)?
            } else {
                Vec::new()
            };
            if !sources.is_empty() && !sources.iter().any(|(name, _)| mirror.join(name).is_file()) {
                continue;
            }
            let mut expected = BTreeSet::new();
            for (name, source) in sources {
                expected.insert(name.clone());
                let raw = fs::read_regular(&source)?;
                let derived = match std::str::from_utf8(&raw) {
                    Ok(text) => transform(&name, text, &spec)?.into_bytes(),
                    Err(_) if spec.transform == "copy" && spec.replacements.is_empty() => raw,
                    Err(_) => {
                        return Err(RuntimeError::invalid(format!(
                            "{}/{name}: binary file in a transformed mirror class",
                            class.canonical
                        )));
                    }
                };
                let target = mirror.join(&name);
                let actual = if target.is_file() {
                    Some(fs::read_regular(&target)?)
                } else {
                    None
                };
                if actual.as_deref() != Some(derived.as_slice()) {
                    drift.push(json!({"path":format!("{mirror_name}/{name}"),"class":class.name,"reason":if actual.is_none(){"missing from mirror".to_string()}else{format!("differs from canon ({}/{name})",class.canonical)}}));
                }
            }
            if class.only.is_none() {
                for (name, _) in reverse {
                    if expected.contains(&name) || skipped(&name, &skips) {
                        continue;
                    }
                    drift.push(json!({"path":format!("{mirror_name}/{name}"),"class":class.name,"reason":format!("no source in {}",class.canonical)}));
                }
            }
        }
    }
    Ok(drift)
}
fn core_rules(root: &Path) -> Result<CoreRules> {
    let rules: CoreRules = serde_json::from_value(read_resource(
        root,
        "install/core-manifest.json",
        CORE_DEFAULT,
    )?)?;
    if rules.version != 1 || rules.includes.is_empty() || rules.editions.len() < 2 {
        return Err(RuntimeError::invalid(
            "Unsupported cross-edition parity rules",
        ));
    }
    relative(&rules.canonical)?;
    for value in rules.editions.values() {
        relative(
            value
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Invalid cross-edition path"))?,
        )?;
    }
    for pattern in &rules.includes {
        relative(pattern)?;
    }
    Ok(rules)
}
fn glob(pattern: &str) -> Result<regex::Regex> {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut position = 0;
    let mut expression = String::from(r"\A(?s:");
    while position < chars.len() {
        match chars[position] {
            '*' if chars.get(position + 1) == Some(&'*') => {
                position += 1;
                if chars.get(position + 1) == Some(&'/') {
                    position += 1;
                    expression.push_str("(?:.*/)?");
                } else {
                    expression.push_str(".*");
                }
            }
            '*' => expression.push_str("[^/]*"),
            '?' => expression.push_str("[^/]"),
            '[' => {
                let mut end = position + 1;
                if chars.get(end) == Some(&'!') {
                    end += 1;
                }
                if chars.get(end) == Some(&']') {
                    end += 1;
                }
                while end < chars.len() && chars[end] != ']' {
                    end += 1;
                }
                if end == chars.len() {
                    expression.push_str(r"\[");
                } else {
                    expression.push('[');
                    let mut start = position + 1;
                    if chars.get(start) == Some(&'!') {
                        expression.push('^');
                        start += 1;
                    } else if chars.get(start) == Some(&'^') {
                        expression.push_str(r"\^");
                        start += 1;
                    }
                    for character in &chars[start..end] {
                        if matches!(*character, '\\' | '[' | ']' | '&') {
                            expression.push('\\');
                        }
                        expression.push(*character);
                    }
                    expression.push(']');
                    position = end;
                }
            }
            character => expression.push_str(&regex::escape(&character.to_string())),
        }
        position += 1;
    }
    expression.push_str(r")\z");
    regex::Regex::new(&expression).map_err(|_| RuntimeError::invalid("Invalid cross-edition glob"))
}
fn glob_inventory(base: &Path, pattern: &str) -> Result<Vec<(String, PathBuf)>> {
    relative(pattern)?;
    let components = pattern.split('/').collect::<Vec<_>>();
    let fixed = components
        .iter()
        .take_while(|part| !part.contains(['*', '?', '[']))
        .count();
    if fixed == components.len() {
        let filename = base.join(pattern);
        let image = match std::fs::symlink_metadata(&filename) {
            Ok(image) => image,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        return if image.is_file() && !fs::linked_at(&filename, &image)? && !ignored(pattern) {
            Ok(vec![(pattern.into(), filename)])
        } else {
            Ok(Vec::new())
        };
    }
    let prefix = components[..fixed].join("/");
    let root = if prefix.is_empty() {
        base.to_path_buf()
    } else {
        base.join(&prefix)
    };
    let pattern = glob(pattern)?;
    Ok(inventory(&root, None)?
        .into_iter()
        .map(|(name, file)| {
            (
                (if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                }),
                file,
            )
        })
        .filter(|(name, _)| pattern.is_match(name))
        .collect())
}
fn allowed(relative: &str, rules: &CoreRules) -> bool {
    rules.allowed_drift.keys().any(|entry| {
        if entry.ends_with('/') {
            relative.starts_with(entry)
        } else {
            relative == entry
        }
    })
}
pub fn monorepo_root(repository: &Path) -> Result<Option<PathBuf>> {
    let resolved = fs::python_realpath(repository)?;
    for candidate in resolved.ancestors().skip(1) {
        if EDITIONS
            .iter()
            .filter(|(_, path)| candidate.join(path).is_dir())
            .count()
            >= 2
        {
            return Ok(Some(candidate.to_path_buf()));
        }
    }
    Ok(None)
}
pub fn cross_edition_report(repository: &Path) -> Result<Option<CrossParity>> {
    let Some(root) = monorepo_root(repository)? else {
        return Ok(None);
    };
    let rules = core_rules(&root)?;
    let mut editions = Vec::new();
    for (name, value) in &rules.editions {
        let relative = value
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Invalid cross-edition path"))?;
        let base = root.join(relative);
        if base.is_dir() {
            editions.push((name.clone(), base));
        }
    }
    if editions.len() < 2 {
        return Err(RuntimeError::invalid(
            "Cross-edition rules name fewer than two present editions",
        ));
    }
    let mut patterns = rules.includes.clone();
    for pattern in OPTIONAL_NATIVE {
        if !patterns.iter().any(|value| value == pattern) {
            patterns.push((*pattern).to_string());
        }
    }
    let mut digests = BTreeMap::<String, BTreeMap<String, String>>::new();
    for (name, base) in &editions {
        for pattern in &patterns {
            for (relative, file) in glob_inventory(base, pattern)? {
                if allowed(&relative, &rules) {
                    continue;
                }
                let digest = query::sha256(&fs::read_regular(&file)?);
                digests
                    .entry(relative)
                    .or_default()
                    .insert(name.clone(), digest);
            }
        }
    }
    let mut drift = Vec::new();
    for (relative, copies) in &digests {
        let missing = editions
            .iter()
            .filter(|(name, _)| !copies.contains_key(name))
            .map(|(name, _)| name.clone())
            .collect::<BTreeSet<_>>();
        if !missing.is_empty() {
            drift.push(json!({"path":relative,"reason":"missing","editions":missing}));
        }
        if copies.values().collect::<BTreeSet<_>>().len() > 1 {
            drift.push(json!({"path":relative,"reason":"content differs","editions":copies.keys().collect::<Vec<_>>()}));
        }
    }
    let not_materialized = OPTIONAL_NATIVE
        .iter()
        .filter(|path| !digests.contains_key(**path))
        .map(|path| (*path).to_string())
        .collect();
    Ok(Some(CrossParity {
        drift,
        not_materialized,
    }))
}
pub fn cross_edition_drift(repository: &Path) -> Result<Option<Vec<Value>>> {
    Ok(cross_edition_report(repository)?.map(|report| report.drift))
}
fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}
pub fn format_skill_drift(drift: &[Value]) -> String {
    let mut lines = vec![format!(
        "Skill mirror parity drift ({} path(s)):",
        drift.len()
    )];
    for item in drift {
        lines.push(format!(
            "  {}: canonical {}, {} in {}",
            item["logical_path"].as_str().unwrap_or(""),
            item["canonical"].as_str().unwrap_or(""),
            item["reason"].as_str().unwrap_or("content differs"),
            strings(&item["mismatched"]).join(", ")
        ));
    }
    lines.join("\n")
}
pub fn format_mirror_drift(drift: &[Value]) -> String {
    let mut lines = vec![format!("Mirror parity drift ({} file(s)):", drift.len())];
    for item in drift {
        lines.push(format!(
            "  [{}] {}: {}",
            item["class"].as_str().unwrap_or(""),
            item["path"].as_str().unwrap_or(""),
            item["reason"].as_str().unwrap_or("")
        ));
    }
    lines.join("\n")
}
pub fn format_cross_drift(drift: &[Value]) -> String {
    let mut lines = vec![format!(
        "Cross-edition core drift ({} finding(s)):",
        drift.len()
    )];
    for item in drift {
        lines.push(format!(
            "  {}: {} ({})",
            item["path"].as_str().unwrap_or(""),
            item["reason"].as_str().unwrap_or(""),
            strings(&item["editions"]).join(", ")
        ));
    }
    lines.join("\n")
}
pub fn render_report(result: &Value, request: &ParityRequest) -> (String, String) {
    if request.cross_edition {
        if result["skipped"] == true {
            return (
                "Cross-edition parity skipped: standalone edition (no monorepo siblings found).\n"
                    .into(),
                String::new(),
            );
        }
        let drift = result["drift"].as_array().cloned().unwrap_or_default();
        if !drift.is_empty() {
            return (String::new(), format!("{}\n", format_cross_drift(&drift)));
        }
        let mut text = String::from("Cross-edition core parity passed.\n");
        let unmaterialized = strings(&result["not_materialized"]);
        if !unmaterialized.is_empty() {
            text.push_str(&format!(
                "Native artifacts not materialized: {}.\n",
                unmaterialized.join(", ")
            ));
        }
        return (text, String::new());
    }
    let drift = result["drift"].as_array().cloned().unwrap_or_default();
    let mirrors = result["mirror_drift"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut errors = Vec::new();
    if !drift.is_empty() {
        errors.push(format_skill_drift(&drift));
    }
    if !mirrors.is_empty() {
        errors.push(format_mirror_drift(&mirrors));
    }
    if result["valid"] != true {
        return (
            String::new(),
            if errors.is_empty() {
                String::new()
            } else {
                errors.join("\n") + "\n"
            },
        );
    }
    let canonical = result["canonical_edition"].as_str().unwrap_or("");
    let stand_in = result["configured_canonical_edition"]
        .as_str()
        .map(|configured| format!("; configured {configured} is not installed here"))
        .unwrap_or_default();
    (
        format!(
            "{} parity passed ({canonical} canonical{stand_in}).\n",
            if request.skills_only {
                "Skill mirror"
            } else {
                "Mirror"
            }
        ),
        String::new(),
    )
}
