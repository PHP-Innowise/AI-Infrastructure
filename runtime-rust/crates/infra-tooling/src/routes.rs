//! Native host routing, roster names, hook wiring and reachability.
use crate::common;
use infra_core::{Result, RuntimeError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
const TOOLS: [(&str, &str, &str); 3] = [
    ("claude", ".claude", ".claude/settings.json"),
    ("cursor", ".cursor", ".cursor/hooks.json"),
    ("codex", ".codex", ".codex/hooks.json"),
];
const NAME: &str = r"[a-z][a-z0-9]*(?:-[a-z0-9]+)*";
fn re(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|_| RuntimeError::invalid("Invalid routing expression"))
}
fn bare(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
        && value.as_bytes()[0].is_ascii_lowercase()
}
fn scalar(raw: &str) -> String {
    raw.trim().trim_matches(['\'', '"']).trim().into()
}
fn names(raw: &str) -> Vec<String> {
    let lines = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let items = if !lines.is_empty() && lines.iter().all(|line| line.starts_with('-')) {
        lines.iter().map(|line| line[1..].to_string()).collect()
    } else {
        let joined = lines.join(" ");
        let text = if joined.starts_with('[') && joined.ends_with(']') {
            &joined[1..joined.len() - 1]
        } else {
            &joined
        };
        text.split(',').map(String::from).collect::<Vec<_>>()
    };
    items
        .into_iter()
        .map(|value| scalar(&value))
        .filter(|value| !["", "null", "none", "~"].contains(&value.to_lowercase().as_str()))
        .collect()
}
fn frontmatter(lines: &[String]) -> (BTreeMap<String, (String, usize)>, usize) {
    if lines.first().is_none_or(|line| line.trim() != "---") {
        return (BTreeMap::new(), 0);
    }
    let mut fields: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut current: Option<String> = None;
    for (index, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            return (fields, index + 1);
        }
        if line.chars().next().is_some_and(|c| !c.is_whitespace())
            && let Some((key, value)) = line.split_once(':')
            && !key.is_empty()
            && key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        {
            fields.insert(key.into(), (value.trim_start().into(), index + 1));
            current = Some(key.into());
            continue;
        }
        if let Some(key) = &current
            && let Some((value, _)) = fields.get_mut(key)
        {
            value.push('\n');
            value.push_str(line);
        }
    }
    (BTreeMap::new(), 0)
}
fn roster(lines: &[String]) -> String {
    if lines.first().map(String::as_str) != Some("---") {
        return String::new();
    }
    for line in &lines[1..] {
        if line == "---" {
            break;
        }
        if let Some(name) = line.strip_prefix("name:") {
            return infra_core::domain::rstrip(name.trim_start_matches([' ', '\t']))
                .replace('"', "");
        }
    }
    String::new()
}
fn bodies(lines: &[String], start: usize) -> Vec<(usize, String)> {
    let mut fenced = false;
    let mut output = Vec::new();
    for (index, line) in lines.iter().enumerate().skip(start) {
        let text = line.trim_start();
        if text.starts_with("~~~") || text.starts_with("\x60\x60\x60") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            output.push((index + 1, line.clone()));
        }
    }
    output
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub edition: String,
    pub severity: String,
    pub check: String,
    pub path: String,
    pub line: Option<usize>,
    pub message: String,
    pub target: String,
}
#[derive(Clone)]
struct Agent {
    stem: String,
    name: String,
    path: PathBuf,
    lines: Vec<String>,
    fields: BTreeMap<String, (String, usize)>,
    body: usize,
    roster: String,
}
struct Edition {
    repo: PathBuf,
    name: String,
    root: PathBuf,
    skills: BTreeSet<String>,
    tool_skills: BTreeMap<String, BTreeSet<String>>,
    commands: BTreeMap<String, BTreeMap<String, PathBuf>>,
    agents: BTreeMap<String, BTreeMap<String, Agent>>,
    unreadable: RefCell<BTreeMap<PathBuf, (String, String)>>,
}
fn files(directory: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    if !directory.is_dir() {
        return Ok(vec![]);
    }
    let mut output = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.path().is_file()
            && entry.path().extension().and_then(|v| v.to_str()) == Some(extension)
            && entry.file_name() != "README.md"
        {
            output.push(entry.path());
        }
    }
    output.sort();
    Ok(output)
}
fn skills(directory: &Path) -> Result<BTreeSet<String>> {
    let mut output = BTreeSet::new();
    if directory.is_dir() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            if entry.path().is_dir() && entry.path().join("SKILL.md").is_file() {
                output.insert(
                    entry
                        .file_name()
                        .into_string()
                        .map_err(|_| RuntimeError::invalid("Routing name is not UTF-8"))?,
                );
            }
        }
    }
    Ok(output)
}
impl Edition {
    fn load(repo: &Path, name: &str) -> Result<Self> {
        let root = repo.join(name);
        let canonical = skills(&root.join(".agents/skills"))?;
        let mut value = Self {
            repo: repo.into(),
            name: name.into(),
            root,
            skills: canonical.clone(),
            tool_skills: BTreeMap::new(),
            commands: BTreeMap::new(),
            agents: BTreeMap::new(),
            unreadable: RefCell::new(BTreeMap::new()),
        };
        value.tool_skills.insert("agents".into(), canonical.clone());
        for (tool, directory, _) in TOOLS {
            let local = skills(&value.root.join(directory).join("skills"))?;
            value
                .tool_skills
                .insert(tool.into(), canonical.union(&local).cloned().collect());
            let mut commands = BTreeMap::new();
            for path in files(&value.root.join(directory).join("commands"), "md")? {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    commands.insert(stem.into(), path);
                }
            }
            value.commands.insert(tool.into(), commands);
            let mut agents = BTreeMap::new();
            for path in files(&value.root.join(directory).join("agents"), "md")? {
                let lines = value.read(&path, "routing");
                let (fields, body) = frontmatter(&lines);
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| RuntimeError::invalid("Agent name is not UTF-8"))?
                    .to_string();
                let name = fields
                    .get("name")
                    .map(|(raw, _)| scalar(raw))
                    .unwrap_or_default();
                agents.insert(
                    stem.clone(),
                    Agent {
                        stem,
                        name,
                        path,
                        roster: roster(&lines),
                        lines,
                        fields,
                        body,
                    },
                );
            }
            value.agents.insert(tool.into(), agents);
        }
        Ok(value)
    }
    fn read(&self, path: &Path, check: &str) -> Vec<String> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.unreadable
                    .borrow_mut()
                    .entry(path.into())
                    .or_insert((check.into(), format!("file cannot be read: {error}")));
                return vec![];
            }
        };
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => {
                self.unreadable.borrow_mut().entry(path.into()).or_insert((check.into(),format!("file is not valid UTF-8 (byte {}); it was checked with undecodable bytes replaced",error.utf8_error().valid_up_to())));
                String::from_utf8_lossy(error.as_bytes()).into_owned()
            }
        };
        py_lines(&text)
    }
    fn rel(&self, path: &Path) -> String {
        path.strip_prefix(&self.repo)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
    fn named(&self, tool: &str, name: &str) -> bool {
        self.agents
            .get(tool)
            .into_iter()
            .flat_map(|items| items.values())
            .any(|agent| agent.name == name && agent.roster == name)
    }
    fn alias(&self, tool: &str, name: &str) -> bool {
        self.agents
            .get(tool)
            .into_iter()
            .flat_map(|items| items.values())
            .any(|agent| {
                agent.stem == name
                    || agent.name == name
                    || !agent.name.is_empty() && format!("{}-agent", agent.name) == name
            })
    }
    fn namespace(&self, tool: &str) -> BTreeSet<String> {
        self.commands
            .get(tool)
            .into_iter()
            .flat_map(|items| items.keys())
            .chain(self.tool_skills.get(tool).into_iter().flatten())
            .cloned()
            .collect()
    }
}
struct Checker<'a> {
    edition: Edition,
    modes: &'a BTreeMap<String, String>,
    findings: Vec<Finding>,
    reached: BTreeSet<String>,
    seen: BTreeSet<(String, Option<usize>, String, String)>,
}
impl Checker<'_> {
    fn add(
        &mut self,
        severity: &str,
        check: &str,
        path: &Path,
        line: Option<usize>,
        message: String,
        target: &str,
    ) {
        let path = self.edition.rel(path);
        let key = (
            path.clone(),
            line,
            check.into(),
            if target.is_empty() {
                message.clone()
            } else {
                target.into()
            },
        );
        if self.seen.insert(key) {
            self.findings.push(Finding {
                edition: self.edition.name.clone(),
                severity: severity.into(),
                check: check.into(),
                path,
                line,
                message,
                target: target.into(),
            });
        }
    }
    fn reached(&mut self, name: &str) {
        if self.edition.skills.contains(name) {
            self.reached.insert(name.into());
        }
    }
    fn invocation(&mut self, tool: &str, path: &Path, line: usize, value: &str, label: &str) {
        let name = value.trim_start_matches('/');
        if self.edition.namespace(tool).contains(name) {
            self.reached(name);
            return;
        }
        let location = if tool == "agents" {
            "no skill in .agents/skills (Codex has no command layer)".into()
        } else {
            format!("no .{tool}/commands command and no skill")
        };
        self.add(
            "error",
            "routing",
            path,
            Some(line),
            format!("{label} resolves to {location}"),
            name,
        );
    }
    fn spawned(&mut self, tool: &str, path: &Path, line: usize, value: &str, label: &str) {
        if self.edition.named(tool, value) {
            return;
        }
        let hint = if self
            .edition
            .agents
            .get(tool)
            .into_iter()
            .flat_map(|items| items.values())
            .any(|agent| agent.name == value)
        {
            " (subagent-gate.sh reads that agent's name differently; see its name error)"
        } else if self.edition.alias(tool, value) {
            " (that is a file name or the spawns alias; spawn by the frontmatter name)"
        } else {
            ""
        };
        self.add(
            "error",
            "routing",
            path,
            Some(line),
            format!("{label} '{value}' names no .{tool}/agents agent{hint}"),
            value,
        );
    }
    fn skill(&mut self, path: &Path, line: usize, value: &str, label: &str) {
        if self.edition.skills.contains(value) {
            self.reached(value);
        } else {
            self.add(
                "error",
                "routing",
                path,
                Some(line),
                format!("{label} '{value}' has no .agents/skills/{value}/SKILL.md"),
                value,
            );
        }
    }
    fn references(&mut self, tool: &str, path: &Path, number: usize, line: &str) -> Result<()> {
        let invocation = re(&format!(r"^/({NAME})(?:[\s\[].*)?$"))?;
        for span in re(r"\x60([^\x60\n]+)\x60")?.captures_iter(line) {
            if let Some(found) = invocation.captures(&span[1]) {
                self.invocation(tool, path, number, &found[1], &format!("/{}", &found[1]));
            }
        }
        for found in re(r"\.(claude|cursor|agents)/skills/([A-Za-z0-9_-]+)/")?.captures_iter(line) {
            if self
                .edition
                .tool_skills
                .get(&found[1])
                .is_some_and(|items| items.contains(&found[2]))
            {
                self.reached(&found[2]);
            } else {
                self.add(
                    "error",
                    "routing",
                    path,
                    Some(number),
                    format!("path .{}/skills/{}/ names no skill", &found[1], &found[2]),
                    &found[2],
                );
            }
        }
        for found in re(&format!(
            r"(?i)\b(?:invoke|execute)\s+(?:the\s+)?(?:\x60({NAME})\x60|({NAME}-{NAME}))\s+skill\b"
        ))?
        .captures_iter(line)
        {
            let name = found
                .get(1)
                .or_else(|| found.get(2))
                .map(|m| m.as_str().to_lowercase())
                .unwrap_or_default();
            self.skill(path, number, &name, "invoked skill");
        }
        Ok(())
    }

    fn hooks(&mut self) -> Result<()> {
        let mut wired = BTreeSet::new();
        for (host, directory, wiring) in TOOLS {
            let path = self.edition.root.join(wiring);
            if !path.is_file() {
                continue;
            }
            let lines = self.edition.read(&path, "hook-wiring");
            let config = match serde_json::from_str::<Value>(&lines.join("\n")) {
                Ok(value) => value,
                Err(error) => {
                    self.add(
                        "error",
                        "hook-wiring",
                        &path,
                        Some(error.line()),
                        format!("invalid JSON: {error}"),
                        "",
                    );
                    continue;
                }
            };
            let Some(events) = config["hooks"].as_object() else {
                self.add(
                    "error",
                    "hook-wiring",
                    &path,
                    None,
                    "hooks must be an event object".into(),
                    "",
                );
                continue;
            };
            for (event, groups) in events {
                let Some(groups) = groups.as_array() else {
                    self.add(
                        "error",
                        "hook-wiring",
                        &path,
                        None,
                        format!("{event}: hook groups must be an array"),
                        "",
                    );
                    continue;
                };
                for group in groups {
                    let handlers = if let Some(value) = group.get("hooks") {
                        if let Some(handlers) = value.as_array() {
                            handlers.clone()
                        } else {
                            self.add(
                                "error",
                                "hook-wiring",
                                &path,
                                None,
                                format!("{event}: handlers must be an array"),
                                "",
                            );
                            continue;
                        }
                    } else {
                        vec![group.clone()]
                    };
                    for handler in handlers {
                        let identity = match crate::attach::classify_native_hook(
                            &handler,
                            host,
                            &self.edition.root,
                        ) {
                            Ok(Some(identity)) => identity,
                            Ok(None) => {
                                self.add(
                                    "error",
                                    "hook-wiring",
                                    &path,
                                    None,
                                    format!(
                                        "{event}: command is not a recognized native hook launch"
                                    ),
                                    "",
                                );
                                continue;
                            }
                            Err(error) => {
                                self.add(
                                    "error",
                                    "hook-wiring",
                                    &path,
                                    None,
                                    format!("{event}: {error}"),
                                    "",
                                );
                                continue;
                            }
                        };
                        // The direct Claude notification role has no compatibility script.
                        if identity.role == "notification" {
                            continue;
                        }
                        let relative = format!("{directory}/hooks/{}.sh", identity.role);
                        wired.insert(relative.clone());
                        let target = self.edition.root.join(&relative);
                        let mode = self.modes.get(&self.edition.rel(&target));
                        let message = if !target.is_file() && mode.is_none() {
                            Some(format!(
                                "wired native hook script {relative} does not exist"
                            ))
                        } else if mode.is_none() {
                            Some(format!(
                                "wired native hook script {relative} is not tracked in Git"
                            ))
                        } else if !target.is_file() {
                            Some(format!(
                                "wired native hook script {relative} is tracked but missing on disk"
                            ))
                        } else if mode.map(String::as_str) != Some("100755") {
                            Some(format!(
                                "wired native hook script {relative} has index mode {}, expected 100755 (git update-index --chmod=+x)",
                                mode.cloned().unwrap_or_default()
                            ))
                        } else {
                            None
                        };
                        if let Some(message) = message {
                            self.add("error", "hook-wiring", &path, None, message, &relative);
                        }
                    }
                }
            }
        }
        for (tool, directory, _) in TOOLS {
            for path in files(&self.edition.root.join(directory).join("hooks"), "sh")? {
                let relative = format!(
                    "{directory}/hooks/{}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                if !wired.contains(&relative) {
                    self.add(
                        "warning",
                        "hook-wiring",
                        &path,
                        None,
                        format!(
                            "hook script is present for {tool} but no native wiring references it"
                        ),
                        &relative,
                    );
                }
            }
        }
        Ok(())
    }
    fn agents(&mut self) -> Result<()> {
        for tool in ["claude", "cursor"] {
            let agents = self.edition.agents.get(tool).cloned().unwrap_or_default();
            let mut named = BTreeMap::<String, PathBuf>::new();
            for (stem, agent) in agents {
                if agent.name.is_empty() {
                    self.add(
                        "error",
                        "routing",
                        &agent.path,
                        Some(1),
                        "agent has no frontmatter name; commands and flows cannot route to it"
                            .into(),
                        &stem,
                    );
                } else if let Some(previous) = named.get(&agent.name) {
                    self.add(
                        "error",
                        "routing",
                        &agent.path,
                        agent.fields.get("name").map(|(_, line)| *line),
                        format!(
                            "agent name '{}' is also used by {}",
                            agent.name,
                            self.edition.rel(previous)
                        ),
                        &agent.name,
                    );
                } else {
                    named.insert(agent.name.clone(), agent.path.clone());
                }
                if !agent.name.is_empty() && (agent.roster != agent.name || !bare(&agent.name)) {
                    self.add("error","routing",&agent.path,agent.fields.get("name").map(|(_,line)|*line),format!("agent name '{}' must be a bare kebab-case name: subagent-gate.sh reads it as '{}', so no spawn would match it",agent.name,agent.roster),&agent.name);
                }
                if tool == "claude" {
                    if let Some((raw, line)) = agent.fields.get("invokes") {
                        for value in names(raw) {
                            self.skill(&agent.path, *line, &value, "invokes");
                        }
                    } else {
                        self.add(
                            "error",
                            "routing",
                            &agent.path,
                            Some(1),
                            "Claude agent has no 'invokes' skill".into(),
                            &stem,
                        );
                    }
                }
                for (number, line) in bodies(&agent.lines, agent.body) {
                    self.references(tool, &agent.path, number, &line)?;
                }
            }
        }
        Ok(())
    }
    fn commands(&mut self) -> Result<()> {
        let subagent = re(r"subagent_type[*:\s]*\x60([^\x60]+)\x60")?;
        let spawn = re(&format!(r"\b(?i:spawn)\s+(?:(?i:the)\s+)?\x60({NAME})\x60"))?;
        let bare_spawn = re(&format!(r"^\s*Spawn\s+(?:the\s+)?({NAME})\s+agent\b"))?;
        let negated = re(r"(?i)(?:\bnot|\bnever|n't)\s*$")?;
        let determiners = [
            "a", "all", "an", "another", "any", "both", "each", "every", "first", "its", "last",
            "new", "next", "no", "one", "only", "same", "several", "single", "that", "the",
            "their", "this",
        ];
        for tool in ["claude", "cursor"] {
            let commands = self.edition.commands.get(tool).cloned().unwrap_or_default();
            for (_, path) in commands {
                let lines = self.edition.read(&path, "routing");
                let (fields, start) = frontmatter(&lines);
                if tool == "claude" {
                    if let Some((raw, line)) = fields.get("spawns") {
                        for value in names(raw) {
                            if !self.edition.alias(tool, &value) {
                                self.add(
                                    "error",
                                    "routing",
                                    &path,
                                    Some(*line),
                                    format!("spawns '{value}' names no .claude/agents agent"),
                                    &value,
                                );
                            }
                        }
                    }
                    for key in ["flow-next", "flow-alternatives"] {
                        if let Some((raw, line)) = fields.get(key) {
                            for value in names(raw) {
                                self.invocation(
                                    tool,
                                    &path,
                                    *line,
                                    &value,
                                    &format!("{key} '{value}'"),
                                );
                            }
                        }
                    }
                    for (index, line) in lines
                        .iter()
                        .enumerate()
                        .take(start.saturating_sub(1))
                        .skip(1)
                    {
                        for capture in re(r"agents:\s*\[([^\]]*)\]")?.captures_iter(line) {
                            for value in names(&capture[1]) {
                                self.spawned(tool, &path, index + 1, &value, "flow stage agent");
                            }
                        }
                    }
                }
                for (number, line) in bodies(&lines, start) {
                    for found in subagent.captures_iter(&line) {
                        self.spawned(tool, &path, number, found[1].trim(), "subagent_type");
                    }
                    for found in spawn.captures_iter(&line) {
                        let begin = found.get(0).map(|m| m.start()).unwrap_or(0);
                        if !negated.is_match(&line[..begin]) {
                            self.spawned(tool, &path, number, &found[1], "spawn");
                        }
                    }
                    if let Some(found) = bare_spawn.captures(&line)
                        && !determiners.contains(&found.get(1).map(|m| m.as_str()).unwrap_or(""))
                    {
                        self.spawned(tool, &path, number, &found[1], "spawn");
                    }
                    self.references(tool, &path, number, &line)?;
                }
            }
        }
        Ok(())
    }
    fn flows(&mut self) -> Result<()> {
        for tool in ["agents", "claude", "cursor"] {
            let path = self
                .edition
                .root
                .join(format!(".{tool}/skills/SKILL FLOW.md"));
            if !path.is_file() {
                continue;
            }
            let lines = self.edition.read(&path, "routing");
            let (mut phase, mut fence) = (false, None::<String>);
            for (index, line) in lines.iter().enumerate() {
                let number = index + 1;
                let stripped = line.trim_start();
                if stripped.starts_with("\x60\x60\x60") || stripped.starts_with("~~~") {
                    fence = if fence.is_some() {
                        None
                    } else {
                        Some(stripped[3..].trim().to_lowercase())
                    };
                } else if fence
                    .as_deref()
                    .is_some_and(|kind| ["", "text", "txt", "plain", "plaintext"].contains(&kind))
                {
                    for value in flow_steps(line)? {
                        self.invocation(
                            tool,
                            &path,
                            number,
                            &value,
                            &format!("flow step '{value}'"),
                        );
                    }
                }
                if line.starts_with('#') && fence.is_none() {
                    phase = line
                        .trim_start_matches('#')
                        .trim()
                        .eq_ignore_ascii_case("Phase Map")
                        && line.starts_with("##");
                }
                for value in slash_tokens(line)? {
                    self.invocation(tool, &path, number, &value, &format!("/{value}"));
                }
                for found in
                    re(&format!(r"\x60({NAME})(?:[ \t]+[<\[][^\x60]*)?\x60"))?.captures_iter(line)
                {
                    self.invocation(tool, &path, number, &found[1], &format!("'{}'", &found[1]));
                }
                if phase && stripped.starts_with('|') {
                    let cells = stripped
                        .trim_matches('|')
                        .split('|')
                        .map(str::trim)
                        .collect::<Vec<_>>();
                    if cells.len() < 2 || cells.join("").chars().all(|c| "-: ".contains(c)) {
                        continue;
                    }
                    for cell in &cells[1..] {
                        for item in cell.split(',') {
                            let name = item
                                .trim()
                                .trim_matches(96 as char)
                                .trim()
                                .trim_start_matches('/');
                            if bare(name) {
                                self.invocation(
                                    tool,
                                    &path,
                                    number,
                                    name,
                                    &format!("Phase Map item '{name}'"),
                                );
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn policy(&mut self) -> Result<()> {
        let path = self.edition.root.join("AGENTS.md");
        if !path.is_file() {
            return Ok(());
        }
        let mut hooks = BTreeSet::new();
        for (_, directory, _) in TOOLS {
            for path in files(&self.edition.root.join(directory).join("hooks"), "sh")? {
                if let Some(stem) = path.file_stem().and_then(|v| v.to_str()) {
                    hooks.insert(stem.to_string());
                }
            }
        }
        let commands = self
            .edition
            .commands
            .get("claude")
            .into_iter()
            .chain(self.edition.commands.get("cursor"))
            .flat_map(|items| items.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let lines = self.edition.read(&path, "routing");
        for (number, line) in bodies(&lines, 0) {
            for name in code_slashes(&line)? {
                if !commands.contains(&name) && !self.edition.skills.contains(&name) {
                    self.add(
                        "error",
                        "routing",
                        &path,
                        Some(number),
                        format!("/{name} names no command and no skill"),
                        &name,
                    );
                }
            }
            for found in re(&format!(r"\x60({NAME})\x60\s+(skill|agent|command|hook)\b"))?
                .captures_iter(&line)
            {
                let (name, kind) = (&found[1], &found[2]);
                let exists = match kind {
                    "skill" => self.edition.skills.contains(name),
                    "agent" => {
                        self.edition.named("claude", name) || self.edition.named("cursor", name)
                    }
                    "command" => commands.contains(name),
                    _ => hooks.contains(name),
                };
                if !exists {
                    self.add(
                        "error",
                        "routing",
                        &path,
                        Some(number),
                        format!("'{name}' {kind} does not exist in this edition"),
                        name,
                    );
                }
            }
        }
        Ok(())
    }
    fn run(mut self) -> Result<Vec<Finding>> {
        self.hooks()?;
        self.agents()?;
        self.commands()?;
        self.flows()?;
        self.policy()?;
        let missing = self
            .edition
            .skills
            .difference(&self.reached)
            .cloned()
            .collect::<Vec<_>>();
        for skill in missing {
            let path = self
                .edition
                .root
                .join(".agents/skills")
                .join(&skill)
                .join("SKILL.md");
            self.add(
                "error",
                "reachability",
                &path,
                None,
                format!("skill '{skill}' is named by no command, agent or flow"),
                &skill,
            );
        }
        let errors = self.edition.unreadable.borrow().clone();
        for (path, (check, message)) in errors {
            let target = self.edition.rel(&path);
            self.add("error", &check, &path, None, message, &target);
        }
        Ok(self.findings)
    }
}
fn code_slashes(line: &str) -> Result<Vec<String>> {
    let mut output = vec![];
    let invocation = re(&format!(r"^/({NAME})(?:[\s\[].*)?$"))?;
    for span in re(r"\x60([^\x60\n]+)\x60")?.captures_iter(line) {
        if let Some(found) = invocation.captures(&span[1]) {
            output.push(found[1].into());
        }
    }
    Ok(output)
}
fn slash_tokens(line: &str) -> Result<Vec<String>> {
    let pattern = re(&format!(r"/({NAME})"))?;
    let mut output = vec![];
    for capture in pattern.captures_iter(line) {
        let whole = capture
            .get(0)
            .ok_or_else(|| RuntimeError::invalid("Invalid slash token"))?;
        let before = line[..whole.start()].chars().next_back();
        let after = line[whole.end()..].chars().next();
        let allowed = before.is_none_or(|c| {
            c.is_whitespace() || [96 as char, '(', '[', '"', ',', '\''].contains(&c)
        });
        let suffix = &line[whole.end()..];
        if allowed
            && !after.is_some_and(|c| c.is_ascii_alphanumeric() || "_/{<>-".contains(c))
            && !(suffix.starts_with('.')
                && suffix[1..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric()))
        {
            output.push(capture[1].into());
        }
    }
    Ok(output)
}
fn flow_steps(line: &str) -> Result<Vec<String>> {
    let starts = re(r"^[ \t]*(?:(?:->|→)[ \t]*)?|[├└]─+[ \t]*")?;
    let candidate = re(&format!(
        r"^({NAME}(?:[ \t]*,[ \t]*(?:or[ \t]+)?{NAME}|[ \t]+or[ \t]+{NAME})*)"
    ))?;
    let split = re(r"[ \t]*,[ \t]*(?:or[ \t]+)?|[ \t]+or[ \t]+")?;
    let mut output = vec![];
    for start in starts.find_iter(line) {
        if let Some(found) = candidate.captures(&line[start.end()..]) {
            let matched = found
                .get(1)
                .ok_or_else(|| RuntimeError::invalid("Invalid flow step"))?;
            let rest = &line[start.end() + matched.end()..];
            let stripped = rest.trim_start_matches([' ', '\t']);
            let spaces = rest.len() - stripped.len();
            let argument = spaces > 0
                && stripped.starts_with('<')
                && stripped.find('>').is_some_and(|end| {
                    end > 1
                        && !stripped[1..end]
                            .chars()
                            .any(|c| c.is_whitespace() || c == '<' || c == '>')
                        && stripped[end + 1..]
                            .chars()
                            .next()
                            .is_none_or(char::is_whitespace)
                });
            let accepted = stripped.is_empty()
                || stripped.starts_with('(')
                || argument
                || spaces >= 2
                    && stripped
                        .chars()
                        .next()
                        .is_some_and(|c| ('\u{2500}'..='\u{257f}').contains(&c) || c == '►');
            if accepted {
                output.extend(split.split(matched.as_str()).map(String::from));
            }
        }
    }
    Ok(output)
}

fn apply_allowlist(
    data: &Value,
    label: &str,
    editions: &[String],
    findings: Vec<Finding>,
    filtered: bool,
) -> (Vec<Finding>, Vec<Finding>) {
    let mut kept = Vec::new();
    let mut allowed = Vec::new();
    let mut entries = Vec::<(String, usize, Value)>::new();
    let known = common::EDITIONS
        .iter()
        .map(|name| name.to_string())
        .chain(editions.iter().cloned())
        .chain(std::iter::once("*".into()))
        .collect::<BTreeSet<_>>();
    let sections = [
        ("unwired_hooks", vec!["path"]),
        ("unreachable_skills", vec!["skill"]),
        ("references", vec!["file", "target"]),
    ];
    let finding = |severity: &str, message: String| Finding {
        edition: "(allowlist)".into(),
        severity: severity.into(),
        check: "allowlist".into(),
        path: label.into(),
        line: None,
        message,
        target: String::new(),
    };
    if let Some(map) = data.as_object() {
        for (section, values) in map {
            if section.starts_with('_') || section == "version" {
                continue;
            }
            let Some((_, fields)) = sections.iter().find(|(name, _)| *name == section) else {
                kept.push(finding("error", format!("unknown section '{section}'")));
                continue;
            };
            let Some(values) = values.as_array() else {
                kept.push(finding(
                    "error",
                    format!("section '{section}' must be a list"),
                ));
                continue;
            };
            for (index, entry) in values.iter().enumerate() {
                if !entry.is_object() {
                    kept.push(finding(
                        "error",
                        format!("{section}[{index}] must be an object"),
                    ));
                    continue;
                }
                let required = ["edition", "reason"]
                    .into_iter()
                    .chain(fields.iter().copied())
                    .collect::<Vec<_>>();
                let mut bad = false;
                for key in required {
                    if entry[key]
                        .as_str()
                        .is_none_or(|value| value.trim().is_empty())
                    {
                        kept.push(finding(
                            "error",
                            format!("{section}[{index}] needs a non-empty '{key}'"),
                        ));
                        bad = true;
                    }
                }
                if let Some(edition) = entry["edition"].as_str()
                    && !edition.trim().is_empty()
                    && !known.contains(edition)
                {
                    kept.push(finding(
                        "error",
                        format!("{section}[{index}] names unknown edition '{edition}'"),
                    ));
                    bad = true;
                }
                if !bad {
                    entries.push((section.clone(), index, entry.clone()));
                }
            }
        }
    }
    let mut used = BTreeSet::new();
    for item in findings {
        let mut matched = false;
        for (section, index, entry) in &entries {
            if entry["edition"] != "*" && entry["edition"] != item.edition {
                continue;
            }
            let matches = match section.as_str() {
                "unwired_hooks" => {
                    item.check == "hook-wiring"
                        && item.severity == "warning"
                        && entry["path"] == item.target
                }
                "unreachable_skills" => {
                    item.check == "reachability" && entry["skill"] == item.target
                }
                _ => {
                    item.check == "routing"
                        && entry["target"] == item.target
                        && entry["file"]
                            == item
                                .path
                                .strip_prefix(&format!("{}/", item.edition))
                                .unwrap_or(&item.path)
                }
            };
            if matches {
                used.insert((section.clone(), *index));
                matched = true;
            }
        }
        if matched {
            allowed.push(item);
        } else {
            kept.push(item);
        }
    }
    for (section, index, entry) in entries {
        if used.contains(&(section.clone(), index)) {
            continue;
        }
        let edition = entry["edition"].as_str().unwrap_or("");
        if edition == "*" && filtered
            || edition != "*" && !editions.iter().any(|name| name == edition)
        {
            continue;
        }
        let mut identity = entry.clone();
        if let Some(map) = identity.as_object_mut() {
            map.remove("reason");
        }
        kept.push(finding(
            "warning",
            format!(
                "stale entry {section}[{index}] matches nothing: {}",
                infra_core::domain::json::python_default(&identity, true, true, true)
                    .unwrap_or_default()
            ),
        ));
    }
    (kept, allowed)
}
pub fn check(root: &Path, editions: &[String], allowlist_path: Option<&Path>) -> Result<Value> {
    let filtered = !editions.is_empty();
    let editions = if editions.is_empty() {
        common::EDITIONS
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    } else {
        let mut names = Vec::new();
        for edition in editions {
            let edition = common::edition(edition)?.to_string();
            if !names.contains(&edition) {
                names.push(edition);
            }
        }
        names
    };
    let path = allowlist_path
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("scripts/check_routes_allowlist.json"));
    let allow = match std::fs::read(&path) {
        Ok(raw) => serde_json::from_slice::<Value>(&raw)
            .map_err(|_| RuntimeError::invalid("Allowlist must be a UTF-8 JSON object"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(error.into()),
    };
    if !allow.is_object() {
        return Err(RuntimeError::invalid(
            "Allowlist top level must be an object",
        ));
    }
    let label = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/");
    let mut arguments = vec![
        "ls-files".to_string(),
        "-s".into(),
        "-z".into(),
        "--".into(),
    ];
    for edition in &editions {
        for (_, directory, _) in TOOLS {
            arguments.push(format!("{edition}/{directory}/hooks"));
        }
    }
    let args = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    let raw = common::git_ok(root, &args, None)?;
    let mut modes = BTreeMap::new();
    for entry in raw
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let text = String::from_utf8_lossy(entry);
        if let Some((metadata, path)) = text.split_once('\t') {
            modes.insert(
                path.to_string(),
                metadata.split(' ').next().unwrap_or("").to_string(),
            );
        }
    }
    let mut findings = vec![];
    for name in &editions {
        if !root.join(name).is_dir() {
            findings.push(Finding {
                edition: name.clone(),
                severity: "error".into(),
                check: "routing".into(),
                path: name.clone(),
                line: None,
                message: "edition directory not found".into(),
                target: String::new(),
            });
            continue;
        }
        let edition = Edition::load(root, name)?;
        findings.extend(
            Checker {
                edition,
                modes: &modes,
                findings: vec![],
                reached: BTreeSet::new(),
                seen: BTreeSet::new(),
            }
            .run()?,
        );
    }
    let (kept, allowed) = apply_allowlist(&allow, &label, &editions, findings, filtered);
    Ok(
        json!({"editions":editions,"errors":kept.iter().filter(|item|item.severity=="error").count(),"warnings":kept.iter().filter(|item|item.severity=="warning").count(),"allowlisted":allowed.len(),"findings":kept,"allowlisted_findings":allowed}),
    )
}

fn py_lines(text: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut line = String::new();
    let mut characters = text.chars().peekable();
    while let Some(c) = characters.next() {
        if matches!(
            c,
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
            output.push(std::mem::take(&mut line));
            if c == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
            }
        } else {
            line.push(c);
        }
    }
    if !line.is_empty() {
        output.push(line);
    }
    output
}
