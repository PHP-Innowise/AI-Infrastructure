//! Display enrichment and activity counts never retain tool output or file contents.
use crate::graph_live::{CreatedEntity, GraphFocus, LiveFocusTracker};
use fancy_regex::Regex;
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
const ENRICH_BYTES: usize = 1024 * 1024;
const ENRICH_EVENTS: usize = 4000;
const STOP_COUNT: usize = 4500;
const STOP_BYTES: usize = 3670016;
const EXTRA_EVENTS: usize = 121;
const EXTRA_BYTES: usize = 256 * 1024;
const STORED_OVERHEAD: usize = 96;
fn limited() -> Value {
    object!({"kind":"status","targets":"limited","text":"Step details are not recorded after this point; steps are still counted."})
}
fn read(tool: &str) -> bool {
    tool == "read"
}
fn edit(tool: &str) -> bool {
    ["edit", "write", "multiedit", "notebookedit"].contains(&tool)
}
fn delete(tool: &str) -> bool {
    tool == "delete"
}
fn search(tool: &str) -> bool {
    ["grep", "glob", "ls"].contains(&tool)
}
fn command(tool: &str) -> bool {
    ["bash", "shell"].contains(&tool)
}
fn helper(tool: &str) -> bool {
    ["task", "agent"].contains(&tool)
}
fn replace(pattern: &Regex, value: &str, mode: u8, literal: Option<&str>) -> Option<String> {
    let mut result = String::new();
    let mut end = 0;
    for capture in pattern.captures_iter(value) {
        let capture = capture.ok()?;
        let matched = capture.get(0)?;
        result.push_str(&value[end..matched.start()]);
        if let Some(literal) = literal {
            result.push_str(literal);
        } else {
            match mode {
                1 => {
                    result.push_str(capture.get(1)?.as_str());
                    result.push_str("[redacted]");
                }
                2 => {
                    result.push_str(capture.get(1)?.as_str());
                    result.push_str(" [redacted]");
                }
                3 => {
                    result.push_str(capture.get(1)?.as_str());
                    result.push_str("[redacted]");
                    result.push_str(capture.get(2)?.as_str());
                }
                4 => result.push_str("-p[redacted]"),
                _ => result.push_str("[redacted]"),
            }
        }
        end = matched.end();
    }
    result.push_str(&value[end..]);
    Some(result)
}
fn redactions() -> Option<&'static Vec<(Regex, u8)>> {
    static PATTERNS: OnceLock<Option<Vec<(Regex, u8)>>> = OnceLock::new();
    PATTERNS.get_or_init(||{
 let value=r#"("[^"]*"|'[^']*'|[^\s"'`;|&)]+)"#;
 let sources=vec![
 (r#"-----BEGIN [A-Z ]*PRIVATE KEY-----|\bAKIA[A-Z0-9]{16}\b|\bsk-[A-Za-z0-9_-]{20,}|(?i:\b(?:password|api[_-]?key|access[_-]?token)\s*[:=]\s*['"]?[A-Za-z0-9_+/=-]{16,})"#.to_string(),0),
 (format!(r"(?i)((?<![\w-])--?(?:[a-z0-9]+[-_])*(?:password|passwd|pass|pwd|secret|token|api[-_]?key|access[-_]?key)(?:=|\s+))(?!-){value}"),1),
 (format!(r"(?<![\w-])([A-Z0-9_]*(?:TOKEN|SECRET|PASSWORD|PASSWD|PWD|PASS|KEY|CREDENTIALS?)=){value}"),1),
 (format!(r"(?i)(?<![\w-])([a-z0-9_]*(?:password|passwd|pwd|secret|token|api_?key|access_?key|private_?key)=){value}"),1),
 (r#"(?i)(\bauthorization:\s*(?:bearer|basic|token|digest)?\s*)([^\s"']+)"#.into(),1),
 (r#"(?i)((?<!\S)(?:-H|--header)[=\s]*["']?\s*(?:x-)?(?:api[-_]?key|auth[-_]?token|access[-_]?token|private[-_]?token):\s*)([^\s"']+)"#.into(),1),
 (r"(?i)(\bsshpass(?:\.exe)?(?:\s+-(?!p\b)\S+)*\s+-p)\s+(?!-)(\S+)".into(),2),
 (r"(?i)(\bredis-cli(?:\.exe)?\b[^|;&\n]*?\s-a)\s+(?!-)(\S+)".into(),2),
 (r"(?i)(\b(?:docker|podman)(?:\.exe)?\s+login\b[^|;&\n]*?\s-p)\s+(?!-)(\S+)".into(),2),
 (r"\b(Bearer\s+)[A-Za-z0-9._~+/=-]{12,}".into(),1),
 (r"(\b[A-Za-z][A-Za-z0-9+.-]*://[^/\s:@]+:)[^/\s@]+(@)".into(),3),
 (r"((?<!\S)(?:-u|--user)\s+[^\s:]+:)(\S+)".into(),1),
 (r"\b(?:gh[pousr]_[A-Za-z0-9]{16,}|github_pat_[A-Za-z0-9_]{20,}|(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{8,}|xox[abposr]-[A-Za-z0-9-]{8,}|glpat-[A-Za-z0-9_-]{16,})".into(),0),
 (r"\beyJ[A-Za-z0-9_-]{8,}(?:\.[A-Za-z0-9_-]+){0,2}".into(),0)];
 sources.into_iter().map(|(pattern,mode)|Regex::new(&pattern).ok().map(|regex|(regex,mode))).collect()
 }).as_ref()
}
pub fn redact_command(value: &str) -> String {
    let Some(patterns) = redactions() else {
        return "[redacted]".into();
    };
    let mut value = value.to_string();
    for (pattern, mode) in patterns {
        let Some(clean) = replace(pattern, &value, *mode, None) else {
            return "[redacted]".into();
        };
        value = clean;
    }
    static ATTACHED: OnceLock<Option<(Regex, Regex)>> = OnceLock::new();
    let attached = ATTACHED.get_or_init(|| {
        Some((
            Regex::new(r"(?:^|[\s/;&|(])(?:mysql[\w-]*|mariadb[\w-]*|sshpass)(?:\.exe)?\b").ok()?,
            Regex::new(r"(?<!\S)-p(?![\s-])\S+").ok()?,
        ))
    });
    match attached {
        Some((password, attached)) => match password.is_match(&value) {
            Ok(true) => replace(attached, &value, 4, None).unwrap_or_else(|| "[redacted]".into()),
            Ok(false) => value,
            Err(_) => "[redacted]".into(),
        },
        None => "[redacted]".into(),
    }
}
fn windows(root: &str) -> bool {
    cfg!(windows)
        || root.as_bytes().get(1) == Some(&b':')
        || root.starts_with("\\\\")
        || root.starts_with("//")
}
fn drive(value: &str) -> (String, &str) {
    if value.as_bytes().get(1) == Some(&b':') {
        return (value[..2].into(), &value[2..]);
    }
    if let Some(rest) = value.strip_prefix("//") {
        let mut parts = rest.splitn(3, '/');
        if let (Some(server), Some(share)) = (parts.next(), parts.next()) {
            let prefix = format!("//{server}/{share}");
            return (prefix.clone(), &value[prefix.len()..]);
        }
    }
    (String::new(), value)
}
fn norm(value: &str, win: bool) -> String {
    let value = if win {
        value.replace('\\', "/")
    } else {
        value.into()
    };
    let (prefix, rest) = if win {
        drive(&value)
    } else {
        (String::new(), value.as_str())
    };
    let absolute = rest.starts_with('/');
    let mut parts = vec![];
    for part in rest.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if parts.last().is_some_and(|value| *value != "..") {
                parts.pop();
            } else if !absolute {
                parts.push(part);
            }
        } else {
            parts.push(part);
        }
    }
    let body = parts.join("/");
    if absolute {
        format!("{prefix}/{body}")
    } else if body.is_empty() {
        if prefix.is_empty() {
            ".".into()
        } else {
            prefix
        }
    } else {
        format!("{prefix}{body}")
    }
}
pub fn full(path: &str, root: &str) -> String {
    let win = windows(root);
    let path = path.trim();
    let path = if win {
        path.replace('\\', "/")
    } else {
        path.into()
    };
    let root = if win {
        root.replace('\\', "/")
    } else {
        root.into()
    };
    if win {
        let (path_drive, path_rest) = drive(&path);
        let (root_drive, _) = drive(&root);
        if !path_drive.is_empty() {
            if !path_rest.starts_with('/') && path_drive.eq_ignore_ascii_case(&root_drive) {
                return norm(&format!("{root}/{path_rest}"), true);
            }
            return norm(&path, true);
        }
        if path.starts_with('/') {
            return norm(&format!("{root_drive}{path}"), true);
        }
    } else if path.starts_with('/') {
        return norm(&path, false);
    }
    norm(&format!("{root}/{path}"), win)
}
pub fn relative(path: &str, root: &str, alias: Option<&str>) -> (Option<String>, bool) {
    if path.trim().is_empty() {
        return (None, false);
    }
    let win = windows(root);
    let full = full(path, root);
    let key = if win {
        full.to_lowercase()
    } else {
        full.clone()
    };
    for base in std::iter::once(Some(root))
        .chain(std::iter::once(alias))
        .flatten()
        .filter(|value| !value.is_empty())
    {
        let base = norm(base, win);
        let prefix = if base.ends_with('/') {
            base.clone()
        } else {
            format!("{base}/")
        };
        let base_key = if win {
            base.to_lowercase()
        } else {
            base.clone()
        };
        let prefix_key = if win {
            prefix.to_lowercase()
        } else {
            prefix.clone()
        };
        if key == base_key {
            return (Some(String::new()), false);
        }
        if key.starts_with(&prefix_key) {
            return (
                Some(if win {
                    full.chars().skip(prefix.chars().count()).collect()
                } else {
                    full[prefix.len()..].to_string()
                }),
                false,
            );
        }
    }
    (
        Some(full.rsplit('/').next().unwrap_or("").to_string()),
        true,
    )
}
const MAX_STRIP_ROOT_REGEX_CACHE: usize = 128;

type StripRootRegexKey = (String, String, bool);
type CompiledStripRoot = Option<Arc<Vec<(Regex, String)>>>;

#[derive(Default)]
struct StripRootRegexCache {
    entries: BTreeMap<StripRootRegexKey, CompiledStripRoot>,
    order: VecDeque<StripRootRegexKey>,
}

#[cfg(test)]
fn strip_root_regex_compile_counts() -> &'static Mutex<BTreeMap<String, usize>> {
    static COUNTS: OnceLock<Mutex<BTreeMap<String, usize>>> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn strip_root_regex_cache() -> &'static Mutex<StripRootRegexCache> {
    static CACHE: OnceLock<Mutex<StripRootRegexCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(StripRootRegexCache::default()))
}

fn compile_strip_root_patterns(variant: &str, name: &str, win: bool) -> CompiledStripRoot {
    let escaped = if win {
        variant
            .split('/')
            .map(regex::escape)
            .collect::<Vec<_>>()
            .join(r"[\\/]+")
    } else {
        regex::escape(variant)
    };
    let mut compiled = Vec::with_capacity(6);
    for (start, inner) in [
        (
            r"(?:(?<=\s-[A-Za-z])|(?<=^-[A-Za-z])|(?<=file://))",
            format!("{name}/"),
        ),
        (
            r"(?<![\w./\\-])",
            if name == "." {
                String::new()
            } else {
                format!("{name}/")
            },
        ),
    ] {
        let start = format!("{}{start}{escaped}", if win { "(?i)" } else { "" });
        for (pattern, replacement) in [
            (format!(r#"{start}[\\/]+(?=[^\s"'`;|&<>()])"#), inner),
            (format!(r"{start}[\\/]+"), format!("{name}/")),
            (format!(r"{start}(?![\w./\\-])"), name.into()),
        ] {
            #[cfg(test)]
            if let Ok(mut counts) = strip_root_regex_compile_counts().lock() {
                *counts.entry(variant.to_owned()).or_insert(0) += 1;
            }
            let regex = Regex::new(&pattern).ok()?;
            compiled.push((regex, replacement));
        }
    }
    Some(Arc::new(compiled))
}

fn cached_strip_root_patterns(variant: &str, name: &str, win: bool) -> CompiledStripRoot {
    let key = (variant.to_owned(), name.to_owned(), win);
    if let Ok(mut guard) = strip_root_regex_cache().lock()
        && let Some(found) = guard.entries.get(&key).cloned()
    {
        if let Some(pos) = guard.order.iter().position(|k| k == &key) {
            guard.order.remove(pos);
        }
        guard.order.push_back(key);
        return found;
    }
    let built = compile_strip_root_patterns(variant, name, win);
    if let Ok(mut guard) = strip_root_regex_cache().lock() {
        if let Some(pos) = guard.order.iter().position(|k| k == &key) {
            guard.order.remove(pos);
        } else if guard.entries.len() >= MAX_STRIP_ROOT_REGEX_CACHE
            && let Some(evict) = guard.order.pop_front()
        {
            guard.entries.remove(&evict);
        }
        guard.order.push_back(key.clone());
        guard.entries.insert(key, built.clone());
    }
    built
}

/// Normalizes a shell command into a canonical signature so trivial output paging/redirection
/// tweaks (`2>&1`, `| head -n 20`, extra whitespace) map to the same telemetry signature.
pub fn normalize_command_signature(command: &str) -> String {
    let mut trimmed = command.trim();
    loop {
        let next = trimmed.trim_end();
        let stripped = if let Some(idx) = next.rfind('|') {
            let tail = next[idx + 1..].trim_start();
            let head_word = tail.split_whitespace().next().unwrap_or("");
            if [
                "head", "tail", "cat", "less", "more", "wc", "grep", "rg", "tee",
            ]
            .contains(&head_word)
            {
                &next[..idx]
            } else {
                next
            }
        } else {
            next
        };
        let without_redir = stripped
            .trim_end()
            .strip_suffix("2>&1")
            .or_else(|| stripped.trim_end().strip_suffix("1>/dev/null"))
            .or_else(|| stripped.trim_end().strip_suffix("2>/dev/null"))
            .or_else(|| stripped.trim_end().strip_suffix(">/dev/null"))
            .map(str::trim_end);
        match without_redir {
            Some(shorter) if shorter.len() < next.len() => trimmed = shorter,
            _ if stripped.len() < next.len() => trimmed = stripped,
            _ => break,
        }
    }
    trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn strip_root(value: &str, roots: &[String], home: Option<&str>) -> String {
    let mut result = value.to_string();
    for (root, name) in roots
        .iter()
        .map(|root| (root.as_str(), "."))
        .chain(home.into_iter().map(|home| (home, "~")))
    {
        let win = windows(root);
        let mut root = norm(root, win);
        while root.ends_with('/') || root.ends_with('\\') {
            root.pop();
        }
        if root.chars().count() < 3 {
            continue;
        }
        let contains = if win {
            fn collapse_seps_lower(text: &str) -> String {
                let mut out = String::with_capacity(text.len());
                let mut prev_sep = false;
                for ch in text.chars() {
                    let mapped = if ch == '\\' { '/' } else { ch };
                    if mapped == '/' {
                        if !prev_sep {
                            out.push('/');
                            prev_sep = true;
                        }
                    } else {
                        prev_sep = false;
                        for lc in mapped.to_lowercase() {
                            out.push(lc);
                        }
                    }
                }
                out
            }
            let needle = collapse_seps_lower(&root);
            collapse_seps_lower(&result).contains(&needle)
        } else {
            result.contains(&root)
        };
        if !contains {
            continue;
        }
        let Some(patterns) = cached_strip_root_patterns(&root, name, win) else {
            return "[redacted]".into();
        };
        for (pattern, replacement) in patterns.iter() {
            let Some(clean) = replace(pattern, &result, 0, Some(replacement)) else {
                return "[redacted]".into();
            };
            result = clean;
        }
    }
    result
}
pub fn clean(value: &str, limit: usize) -> Option<String> {
    let value = value
        .chars()
        .filter(|value| !matches!(value,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
        .map(|value| {
            if value < ' ' || value == '\u{7f}' {
                ' '
            } else {
                value
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if value.is_empty() || limit == 0 {
        return None;
    }
    if value.chars().count() <= limit {
        Some(value)
    } else {
        Some(value.chars().take(limit - 1).chain(['…']).collect())
    }
}
fn call(value: &Value) -> Option<String> {
    let value = value.as_str()?;
    if value.is_empty()
        || value.len() > 128
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || b"_.:-".contains(&value))
    {
        None
    } else {
        Some(value.into())
    }
}
fn size(value: &Value) -> usize {
    infra_core::domain::json::python_default(value, false, false, false)
        .map(|value| value.len() + STORED_OVERHEAD)
        .unwrap_or(usize::MAX)
}
#[derive(Clone, Default)]
struct Private {
    key: Option<String>,
    changes: Vec<Option<String>>,
    existed: Option<bool>,
}
#[derive(Clone, Default)]
struct Entry {
    _kind: String,
    keys: Vec<(String, Option<String>, String)>,
    changes: Vec<(String, Option<String>, String)>,
    read: Option<String>,
    command: Option<u64>,
}
pub struct RunLedger {
    provider: String,
    pub plan: Option<Value>,
    pub limited: bool,
    steps: u64,
    helpers: u64,
    failed_steps: u64,
    searched: u64,
    commands: u64,
    /// Commands that asked the project's code graph.
    graph: u64,
    failed: u64,
    not_run: u64,
    opened: Vec<(String, Option<String>)>,
    edited: Vec<(String, Option<String>)>,
    created: BTreeSet<String>,
    deleted: BTreeSet<String>,
    patterns: Vec<String>,
    calls: BTreeMap<String, Entry>,
    uncalled: BTreeMap<String, VecDeque<Entry>>,
    last_commands: VecDeque<(u64, Value)>,
    command_id: u64,
    command_signature_counts: BTreeMap<String, u64>,
    command_signature_failures: BTreeMap<String, u64>,
    watch: Option<BTreeMap<String, String>>,
    watch_outside: bool,
    watched: BTreeSet<String>,
    reads: u64,
    outside_reads: u64,
    focus_tracker: LiveFocusTracker,
    focus_events: VecDeque<Value>,
}
impl RunLedger {
    pub fn new(provider: &str, watch: Option<BTreeMap<String, String>>, outside: bool) -> Self {
        Self {
            provider: provider.into(),
            plan: None,
            limited: false,
            steps: 0,
            helpers: 0,
            failed_steps: 0,
            searched: 0,
            commands: 0,
            graph: 0,
            failed: 0,
            not_run: 0,
            opened: vec![],
            edited: vec![],
            created: BTreeSet::new(),
            deleted: BTreeSet::new(),
            patterns: vec![],
            calls: BTreeMap::new(),
            uncalled: BTreeMap::new(),
            last_commands: VecDeque::new(),
            command_id: 0,
            command_signature_counts: BTreeMap::new(),
            command_signature_failures: BTreeMap::new(),
            watch,
            watch_outside: outside,
            watched: BTreeSet::new(),
            reads: 0,
            outside_reads: 0,
            focus_tracker: LiveFocusTracker::new(PathBuf::new()),
            focus_events: VecDeque::new(),
        }
    }
    pub fn set_focus_root(&mut self, root: PathBuf) {
        self.focus_tracker.set_root(root);
    }
    pub fn seed_graph_focus(&mut self, focus: GraphFocus) {
        self.focus_tracker.seed_focus(focus);
    }
    pub fn graph_focus(&self) -> &GraphFocus {
        self.focus_tracker.focus()
    }
    pub fn created_entities(&self) -> &[CreatedEntity] {
        &self.focus_tracker.focus().created_entities
    }
    pub fn target_paths(&self) -> &[String] {
        &self.focus_tracker.focus().target_paths
    }
    pub fn target_folders(&self) -> &[String] {
        &self.focus_tracker.focus().target_folders
    }
    pub fn take_focus_event(&mut self) -> Option<Value> {
        self.focus_events.pop_front()
    }
    pub fn take_focus_events(&mut self) -> Vec<Value> {
        self.focus_events.drain(..).collect()
    }
    fn changes(fields: &Value, private: &Private) -> Vec<(String, Option<String>, String)> {
        let changes = fields["changes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|change| {
                change.is_object() && change["path"].as_str().is_some_and(|path| !path.is_empty())
            })
            .collect::<Vec<_>>();
        let matching = private.changes.len() == changes.len();
        changes
            .iter()
            .enumerate()
            .map(|(index, change)| {
                let path = change["path"].as_str().unwrap_or("");
                let outside = change["outside"] == true;
                let key = if matching {
                    private.changes[index].clone()
                } else {
                    None
                }
                .unwrap_or_else(|| format!("{}{path}", if outside { "outside:" } else { "" }));
                (
                    key,
                    if outside { None } else { Some(path.into()) },
                    change["kind"].as_str().unwrap_or("update").into(),
                )
            })
            .collect()
    }
    fn start(&mut self, fields: &Value, private: &Private) -> Entry {
        let kind = fields["tool"].as_str().unwrap_or("").to_lowercase();
        let outside = fields["outside"] == true;
        let path = if outside {
            None
        } else {
            fields["path"].as_str().map(str::to_string)
        };
        let key = private.key.clone().unwrap_or_else(|| {
            format!(
                "{}{}",
                if outside { "outside:" } else { "" },
                fields["path"].as_str().unwrap_or("")
            )
        });
        let mut entry = Entry {
            _kind: kind.clone(),
            changes: Self::changes(fields, private),
            ..Default::default()
        };
        if read(&kind) && fields["path"].as_str().is_some_and(|path| !path.is_empty()) {
            if !self.opened.iter().any(|(old, _)| old == &key) {
                self.opened.push((key.clone(), path.clone()));
            }
            self.reads += 1;
            self.outside_reads += u64::from(outside);
            if let Some(rel) = path.as_deref() {
                self.focus_tracker.record_read(rel);
            }
            entry.read = path;
        } else if kind == "skill"
            && fields["detail"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        {
            self.reads += 1;
            entry.read = Some(format!("skill:{}", fields["detail"].as_str().unwrap_or("")));
        } else if (edit(&kind) || kind == "file_change" || delete(&kind))
            && fields["path"].as_str().is_some_and(|path| !path.is_empty())
        {
            if let Some(rel) = path.as_deref() {
                self.focus_tracker.record_mutation_start(rel);
            }
            let created = kind == "write"
                && private.existed.map(|value| !value).unwrap_or_else(|| {
                    !self.opened.iter().any(|(old, _)| old == &key)
                        && !self.edited.iter().any(|(old, _)| old == &key)
                });
            entry.keys.push((
                key,
                path,
                if delete(&kind) {
                    "delete"
                } else if created {
                    "add"
                } else {
                    "update"
                }
                .into(),
            ));
        } else if search(&kind) {
            self.searched += 1;
            if let Some(pattern) = fields["detail"].as_str().filter(|value| !value.is_empty())
                && self.patterns.len() < 10000
                && !self.patterns.iter().any(|old| old == pattern)
            {
                self.patterns.push(pattern.into());
            }
        } else if command(&kind) {
            self.commands += 1;
            let raw_command = fields["detail"].as_str().unwrap_or("");
            if crate::graph::asked(raw_command) {
                self.graph += 1;
            }
            self.command_id += 1;
            entry.command = Some(self.command_id);
            let signature = normalize_command_signature(raw_command);
            let repeats = if signature.is_empty() {
                0
            } else if self.command_signature_counts.len() < 512
                || self.command_signature_counts.contains_key(&signature)
            {
                let count = self
                    .command_signature_counts
                    .entry(signature.clone())
                    .or_insert(0);
                *count = count.saturating_add(1);
                *count
            } else {
                1
            };
            self.last_commands.push_back((
                self.command_id,
                object!({
                    "command": raw_command,
                    "signature": signature,
                    "repeats": repeats,
                    "ok": null,
                    "outcome": null,
                    "exit_code": null
                }),
            ));
            while self.last_commands.len() > 12 {
                self.last_commands.pop_front();
            }
        } else if helper(&kind)
            && if self.provider == "codex" {
                fields["detail"] == "spawn_agent"
            } else {
                self.provider == "claude"
            }
        {
            self.helpers += 1;
        }
        for (_, maybe_rel, _) in &entry.changes {
            if let Some(rel) = maybe_rel.as_deref() {
                self.focus_tracker.record_mutation_start(rel);
            }
        }
        entry
    }
    fn finish(&mut self, entry: Entry, fields: &Value, private: &Private) {
        let ok = fields["ok"] != false;
        let not_run = fields["outcome"] == "not_run";
        if let Some(id) = entry.command {
            let mut failed_sig = None;
            if let Some((_, command)) = self.last_commands.iter_mut().find(|(key, _)| *key == id) {
                command["ok"] = ok.into();
                command["outcome"] = fields["outcome"].clone();
                command["exit_code"] = fields["exit_code"].clone();
                if !ok && !not_run {
                    failed_sig = command["signature"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                }
            }
            if let Some(sig) = failed_sig
                && (self.command_signature_failures.len() < 512
                    || self.command_signature_failures.contains_key(&sig))
            {
                let count = self.command_signature_failures.entry(sig).or_insert(0);
                *count = count.saturating_add(1);
            }
            if !ok {
                self.not_run += u64::from(not_run);
                self.failed += u64::from(!not_run);
            }
        }
        if !ok {
            self.failed_steps += u64::from(!not_run);
            return;
        }
        if let (Some(watch), Some(read)) = (&self.watch, &entry.read)
            && let Some(id) = watch.get(read)
        {
            self.watched.insert(id.clone());
        }
        let changes = if fields["changes"]
            .as_array()
            .is_some_and(|changes| !changes.is_empty())
        {
            Self::changes(fields, private)
        } else {
            entry.changes
        };
        for (key, path, kind) in entry.keys.into_iter().chain(changes) {
            if let Some(rel) = path.as_deref() {
                self.focus_tracker
                    .record_mutation_complete(rel, Some(kind.as_str()));
            }
            if !self.edited.iter().any(|(old, _)| old == &key) {
                self.edited.push((key.clone(), path));
            }
            if kind == "add" {
                self.created.insert(key);
            } else if kind == "delete" {
                self.deleted.insert(key);
            }
        }
        if let Some(delta) = self.focus_tracker.take_delta_event() {
            self.focus_events.push_back(delta);
        }
    }
    fn observe_private(&mut self, label: &Value, fields: Option<&Value>, private: &Private) {
        let Some(fields) = fields else {
            if let Some(text) = label["text"]
                .as_str()
                .filter(|text| text.ends_with(": started"))
            {
                self.steps += 1;
                self.helpers += u64::from(text == "Agent activity: started");
            }
            if label["ok"] == false {
                self.failed_steps += 1;
            }
            return;
        };
        let state = fields["state"].as_str().unwrap_or("");
        let call = fields["call"].as_str().filter(|value| !value.is_empty());
        let kind = fields["tool"].as_str().unwrap_or("").to_lowercase();
        let entry =
            if state == "completed" && call.is_some_and(|call| self.calls.contains_key(call)) {
                self.calls.remove(call.unwrap_or("")).unwrap_or_default()
            } else if state == "completed"
                && call.is_none()
                && self
                    .uncalled
                    .get(&kind)
                    .is_some_and(|entries| !entries.is_empty())
            {
                self.uncalled
                    .get_mut(&kind)
                    .and_then(VecDeque::pop_front)
                    .unwrap_or_default()
            } else if state == "completed" && (call.is_none() || kind.is_empty()) {
                Entry::default()
            } else {
                self.steps += 1;
                let entry = self.start(fields, private);
                if state == "started" {
                    if let Some(delta) = self.focus_tracker.take_delta_event() {
                        self.focus_events.push_back(delta);
                    }
                    if let Some(call) = call {
                        self.calls.insert(call.into(), entry);
                    } else if !kind.is_empty() {
                        self.uncalled.entry(kind).or_default().push_back(entry);
                    }
                    return;
                }
                entry
            };
        self.finish(entry, fields, private)
    }
    pub fn observe(&mut self, label: &Value, fields: Option<&Value>) {
        self.observe_private(label, fields, &Private::default());
    }
    pub fn set_watch(&mut self, watch: Option<BTreeMap<String, String>>) -> bool {
        if self.steps > 0 || self.reads > 0 {
            return false;
        }
        self.watch = watch;
        self.watched.clear();
        true
    }
    pub fn set_watch_outside(&mut self, outside: bool) {
        self.watch_outside = outside;
    }
    pub fn memory(&self) -> Option<Value> {
        let watch = self.watch.as_ref()?;
        let mut unseen = vec![];
        if self.commands.saturating_sub(self.not_run) > 0 {
            unseen.push("shell");
        }
        if self.provider == "codex" && self.helpers > 0 {
            unseen.push("helpers");
        }
        if self.watch_outside && self.outside_reads > 0 {
            unseen.push("outside");
        }
        Some(
            object!({"delivered":watch.values().collect::<BTreeSet<_>>().len(),"opened":self.watched.len(),"observation":if unseen.is_empty(){"observed"}else if self.reads>0{"lower_bound"}else{"unknown"},"unseen":unseen}),
        )
    }
    pub fn receipt(&self) -> Value {
        let listed = |entries: &Vec<(String, Option<String>)>| {
            entries
                .iter()
                .filter_map(|(_, path)| path.clone())
                .take(40)
                .collect::<Vec<_>>()
        };
        let mut command_loops: Vec<(u64, u64, &str)> = self
            .command_signature_counts
            .iter()
            .filter(|(_, count)| **count >= 2)
            .map(|(sig, count)| {
                let failed = self
                    .command_signature_failures
                    .get(sig)
                    .copied()
                    .unwrap_or(0);
                (*count, failed, sig.as_str())
            })
            .collect();
        command_loops.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.cmp(b.2)));
        let loop_rows: Vec<Value> = command_loops
            .into_iter()
            .take(12)
            .map(|(count, failed, sig)| {
                object!({"signature": sig, "count": count, "failed": failed})
            })
            .collect();
        let max_command_repeats = self
            .command_signature_counts
            .values()
            .copied()
            .max()
            .unwrap_or(0);
        let mut receipt = object!({"steps":self.steps,"opened":self.opened.len(),"opened_paths":listed(&self.opened),"edited":self.edited.len(),"edited_paths":listed(&self.edited),"created":self.created.len(),"deleted":self.deleted.len(),"searched":self.searched,"patterns":self.patterns.iter().take(12).collect::<Vec<_>>(),"commands":self.commands,"graph":self.graph,"failed":self.failed,"not_run":self.not_run,"failed_steps":self.failed_steps,"last_commands":self.last_commands.iter().map(|(_,value)|value.clone()).collect::<Vec<_>>(),"max_command_repeats":max_command_repeats,"command_loops":loop_rows,"plan":self.plan,"helpers":self.helpers,"limited":self.limited});
        if let Some(memory) = self.memory() {
            receipt["memory"] = memory;
        }
        if !self.focus_tracker.focus().created_entities.is_empty() {
            receipt["created_entities"] = object!(self.focus_tracker.focus().created_entities);
        }
        receipt
    }
    /// The files of the project the run opened, and those it changed with
    /// what was done to each: `A` for a file it made, `D` for one it
    /// deleted, `M` for any other. All of them, where the receipt names the
    /// first few; a file outside the project is counted there and left out
    /// here.
    pub fn files(&self) -> (Vec<&str>, Vec<(&str, char)>) {
        let opened = self
            .opened
            .iter()
            .filter_map(|(_, path)| path.as_deref())
            .collect();
        let edited = self
            .edited
            .iter()
            .filter_map(|(key, path)| {
                let done = if self.deleted.contains(key) {
                    'D'
                } else if self.created.contains(key) {
                    'A'
                } else {
                    'M'
                };
                Some((path.as_deref()?, done))
            })
            .collect();
        (opened, edited)
    }
}
pub fn compaction_text(pre: Option<u64>, post: Option<u64>) -> String {
    fn grouped(value: u64) -> String {
        let text = value.to_string();
        let mut result = String::new();
        for (index, c) in text.chars().enumerate() {
            if index > 0 && (text.len() - index).is_multiple_of(3) {
                result.push(',');
            }
            result.push(c);
        }
        result
    }
    let size = match (pre, post) {
        (Some(pre), Some(post)) => format!(" · {} → {} tokens", grouped(pre), grouped(post)),
        (Some(pre), None) => format!(" at {} tokens", grouped(pre)),
        (None, Some(post)) => format!(" to {} tokens", grouped(post)),
        _ => String::new(),
    };
    format!("Context compacted{size}. The CLI replaced earlier conversation with a summary.")
}
pub struct Enricher {
    _provider: String,
    root: String,
    alias: Option<String>,
    home: Option<String>,
    roots: Vec<String>,
    events: usize,
    bytes: usize,
    extra_events: usize,
    extra_bytes: usize,
    plans: usize,
    compactions: usize,
    stopped: bool,
    noticed: bool,
    last_plan: Option<String>,
    plan_items: Option<Vec<Value>>,
    plan_total: u64,
    pub ledger: RunLedger,
}
impl Enricher {
    pub fn new(
        provider: &str,
        root: &str,
        alias: Option<&str>,
        strip: &[String],
        home: Option<&str>,
        watch: Option<BTreeMap<String, String>>,
        outside: bool,
    ) -> Self {
        let mut ledger = RunLedger::new(provider, watch, outside);
        if !root.is_empty() {
            ledger.set_focus_root(PathBuf::from(root));
        }
        Self {
            _provider: provider.into(),
            root: root.into(),
            alias: alias.map(str::to_string),
            home: home.map(str::to_string),
            roots: std::iter::once(Some(root.to_string()))
                .chain(std::iter::once(alias.map(str::to_string)))
                .chain(strip.iter().cloned().map(Some))
                .flatten()
                .filter(|value| !value.is_empty())
                .collect(),
            events: 0,
            bytes: 0,
            extra_events: 0,
            extra_bytes: 0,
            plans: 0,
            compactions: 0,
            stopped: false,
            noticed: false,
            last_plan: None,
            plan_items: None,
            plan_total: 0,
            ledger,
        }
    }
    pub fn take_focus_event(&mut self) -> Option<Value> {
        self.ledger.take_focus_event()
    }
    pub fn take_focus_events(&mut self) -> Vec<Value> {
        self.ledger.take_focus_events()
    }
    pub fn text(&self, value: &str, limit: usize) -> Option<String> {
        clean(
            &redact_command(&strip_root(value, &self.roots, self.home.as_deref())),
            limit,
        )
    }
    fn key(&self, raw: &str, path: &str, outside: bool) -> String {
        if !outside {
            return path.into();
        }
        let full = full(raw, &self.root);
        let full = if windows(&self.root) {
            full.to_lowercase()
        } else {
            full
        };
        format!(
            "outside:{}",
            &infra_core::query::sha256(full.as_bytes())[..20]
        )
    }
    fn read_fields(&self, targets: &Value) -> (Value, Private) {
        let mut fields = object!({"state":if targets["state"]=="completed"{"completed"}else{"started"},"ok":targets["ok"]!=false});
        let mut private = Private::default();
        let tool = targets["tool"].as_str().and_then(|value| clean(value, 120));
        if let Some(tool) = &tool {
            fields["tool"] = tool.clone().into();
        }
        for key in ["call", "parent"] {
            if let Some(value) = call(&targets[key]) {
                fields[key] = value.into();
            }
        }
        if let Some(raw) = targets["path"].as_str() {
            let (path, outside) = relative(raw, &self.root, self.alias.as_deref());
            if let Some(path) = path.and_then(|path| clean(&redact_command(&path), 1024)) {
                fields["path"] = path.clone().into();
                if outside {
                    fields["outside"] = true.into();
                }
                private.key = Some(self.key(raw, &path, outside));
                if tool
                    .as_deref()
                    .is_some_and(|tool| tool.eq_ignore_ascii_case("write"))
                    && fields["state"] == "started"
                    && windows(&self.root) == cfg!(windows)
                {
                    private.existed = match std::fs::symlink_metadata(full(raw, &self.root)) {
                        Ok(_) => Some(true),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
                        Err(_) => None,
                    };
                }
            }
        }
        if let Some(detail) = targets["detail"].as_str() {
            let detail = if tool
                .as_deref()
                .is_some_and(|tool| command(&tool.to_lowercase()))
            {
                infra_host::activity::command_head(detail)
            } else {
                Some(detail.into())
            };
            if let Some(detail) = detail.and_then(|detail| self.text(&detail, 300)) {
                fields["detail"] = detail.into();
            }
        }
        if targets["outcome"] == "not_run" {
            fields["outcome"] = "not_run".into();
        }
        if let Some(code) = targets["exit_code"].as_i64() {
            fields["exit_code"] = code.into();
        }
        if let Some(changes) = targets["changes"].as_array() {
            let mut cleaned = vec![];
            for change in changes.iter().take(20) {
                if let Some(raw) = change["path"].as_str() {
                    let (path, outside) = relative(raw, &self.root, self.alias.as_deref());
                    if let Some(path) = path.and_then(|path| clean(&redact_command(&path), 1024)) {
                        let kind = change["kind"]
                            .as_str()
                            .filter(|kind| ["add", "update", "delete"].contains(kind))
                            .unwrap_or("update");
                        let mut value = object!({"path":path,"kind":kind});
                        if outside {
                            value["outside"] = true.into();
                        }
                        cleaned.push(value);
                        private.changes.push(Some(self.key(raw, &path, outside)));
                    }
                }
            }
            fields["changes"] = object!(cleaned);
        }
        (fields, private)
    }
    pub fn fields(&self, targets: &Value) -> Option<Value> {
        if targets.is_object() {
            Some(self.read_fields(targets).0)
        } else {
            None
        }
    }
    pub fn take(&mut self, clean: &mut Value, count: usize, output_bytes: usize) -> Option<Value> {
        let targets = clean
            .as_object_mut()
            .and_then(|value| value.remove("targets"));
        let fields = targets
            .as_ref()
            .filter(|targets| targets.is_object())
            .map(|targets| self.read_fields(targets));
        match &fields {
            Some((fields, private)) => self.ledger.observe_private(clean, Some(fields), private),
            None => self
                .ledger
                .observe_private(clean, None, &Private::default()),
        }
        let (fields, _) = fields?;
        if self.stopped {
            return None;
        }
        let length = infra_core::domain::json::python_default(&fields, false, false, false)
            .ok()?
            .len();
        if self.events + 1 > ENRICH_EVENTS
            || self.bytes.saturating_add(length) > ENRICH_BYTES
            || count > STOP_COUNT
            || output_bytes > STOP_BYTES
        {
            self.stopped = true;
            self.ledger.limited = true;
            return None;
        }
        self.events += 1;
        self.bytes += length;
        Some(fields)
    }
    pub fn notice(&mut self) -> Option<Value> {
        if !self.stopped || self.noticed {
            return None;
        }
        self.noticed = true;
        let value = limited();
        self.allow_extra(&value);
        Some(value)
    }
    pub fn allow_extra(&mut self, event: &Value) -> bool {
        let reserve = usize::from(!self.noticed && event["targets"] != "limited");
        let length = size(event);
        if self.extra_events + 1 > EXTRA_EVENTS.saturating_sub(reserve)
            || self.extra_bytes.saturating_add(length)
                > EXTRA_BYTES.saturating_sub(reserve * size(&limited()))
        {
            return false;
        }
        self.extra_events += 1;
        self.extra_bytes += length;
        true
    }
    pub fn plan(&mut self, event: &Value) -> Option<Value> {
        let incoming=event["items"].as_array().into_iter().flatten().filter(|item|item.is_object()).map(|item|object!({"id":call(&item["id"]),"text":item["text"].as_str().and_then(|text|self.text(text,200)),"status":item["status"].as_str().filter(|value|["done","active","pending","cancelled"].contains(value))})).collect::<Vec<_>>();
        let (items, total) = if event["merge"] == true && self.plan_items.is_some() {
            let mut items = self.plan_items.clone().unwrap_or_default();
            let mut added = 0;
            for item in incoming {
                let id = item["id"].as_str();
                if let Some(index) = id.and_then(|id| items.iter().position(|old| old["id"] == id))
                {
                    for key in ["text", "status"] {
                        if item[key].as_str().is_some_and(|value| !value.is_empty()) {
                            items[index][key] = item[key].clone();
                        }
                    }
                } else if item["text"].as_str().is_some_and(|value| !value.is_empty()) {
                    let mut item = item;
                    if item["status"].is_null() {
                        item["status"] = "pending".into();
                    }
                    items.push(item);
                    added += 1;
                }
            }
            let total = (items.len() as u64).max(self.plan_total.saturating_add(added));
            (items, total)
        } else {
            let items = incoming
                .into_iter()
                .filter(|item| item["text"].as_str().is_some_and(|value| !value.is_empty()))
                .map(|mut item| {
                    if item["status"].is_null() {
                        item["status"] = "pending".into();
                    }
                    item
                })
                .collect::<Vec<_>>();
            let total = event["total"]
                .as_u64()
                .filter(|value| *value >= items.len() as u64)
                .unwrap_or(items.len() as u64);
            (items, total)
        };
        if items.is_empty() {
            return None;
        }
        self.plan_items = Some(items.iter().take(200).cloned().collect());
        self.plan_total = total;
        self.ledger.plan = Some(
            object!({"done":items.iter().filter(|item|item["status"]=="done").count(),"total":total}),
        );
        let shown = items
            .iter()
            .take(30)
            .map(|item| object!({"text":item["text"],"status":item["status"]}))
            .collect::<Vec<_>>();
        let mut plan = object!({"kind":"plan","items":shown,"total":total});
        if let Some(form) = event["active_form"]
            .as_str()
            .and_then(|form| self.text(form, 200))
        {
            plan["active_form"] = form.into();
        }
        let encoded = infra_core::domain::json::python_default(&plan, true, true, false).ok()?;
        let digest = infra_core::query::sha256(encoded.as_bytes());
        if self.last_plan.as_ref() == Some(&digest) || self.plans >= 100 || !self.allow_extra(&plan)
        {
            return None;
        }
        self.last_plan = Some(digest);
        self.plans += 1;
        Some(plan)
    }
    pub fn compaction(&mut self, compactions: &[Value]) -> Vec<Value> {
        let mut result = vec![];
        for item in compactions.iter().skip(self.compactions) {
            if self.compactions >= 20 {
                break;
            }
            self.compactions += 1;
            if !item.is_object() {
                break;
            }
            let pre = item["pre"].as_u64();
            let post = item["post"].as_u64();
            let event = object!({"kind":"status","compaction":{"pre":pre,"post":post},"text":compaction_text(pre,post)});
            if self.allow_extra(&event) {
                result.push(event);
            }
        }
        result
    }
}
pub fn user_home() -> Option<String> {
    std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .ok()
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn enricher_tracks_pre_edit_baseline_and_emits_live_focus_deltas() {
        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();
        fs::create_dir_all(root.join("src/Auth")).expect("auth dir");
        let target_file = root.join("src/Auth/TokenService.php");
        fs::write(
            &target_file,
            "<?php\nclass TokenService {\n    public function verify(): bool { return true; }\n}\n",
        )
        .expect("initial php");

        let mut enricher = Enricher::new(
            "claude",
            root.to_str().expect("utf8"),
            None,
            &[],
            None,
            None,
            false,
        );

        // Started Edit captures pre-edit baseline before file is modified on disk.
        let mut start_edit = object!({
            "kind": "tool",
            "targets": {
                "state": "started",
                "tool": "Edit",
                "call": "c1",
                "path": target_file.to_str().expect("path"),
            }
        });
        assert!(enricher.take(&mut start_edit, 0, 0).is_some());
        let start_deltas = enricher.take_focus_events();
        assert_eq!(start_deltas.len(), 1);
        assert_eq!(start_deltas[0]["kind"], "graph_focus");
        assert_eq!(start_deltas[0]["phase"], "active");
        assert_eq!(
            start_deltas[0]["target_paths"],
            object!(["src/Auth/TokenService.php"])
        );
        assert!(
            start_deltas[0]["created_entities"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );

        // Simulate tool writing new method on disk before completed event.
        fs::write(
            &target_file,
            "<?php\nclass TokenService {\n    public function verify(): bool { return true; }\n    public function rotateKey(): string { return 'secret-body'; }\n}\n",
        )
        .expect("updated php");

        let mut done_edit = object!({
            "kind": "tool",
            "targets": {
                "state": "completed",
                "ok": true,
                "tool": "Edit",
                "call": "c1",
                "path": target_file.to_str().expect("path"),
            }
        });
        assert!(enricher.take(&mut done_edit, 1, 100).is_some());
        let complete_deltas = enricher.take_focus_events();
        assert_eq!(complete_deltas.len(), 1);
        let delta = &complete_deltas[0];
        assert_eq!(delta["kind"], "graph_focus");
        assert_eq!(delta["phase"], "active");
        assert!(!delta.to_string().contains("secret-body"));
        assert_eq!(delta["created_entities"].as_array().map(Vec::len), Some(1));
        assert_eq!(delta["created_entities"][0]["kind"], "method");
        assert_eq!(delta["created_entities"][0]["name"], "rotateKey");
        assert_eq!(delta["created_entities"][0]["parent"], "TokenService");

        // Codex file_change adding a brand-new Rust file emits file + struct + impl method.
        fs::create_dir_all(root.join("src/Http")).expect("http dir");
        let guard_file = root.join("src/Http/guard.rs");
        fs::write(
            &guard_file,
            "pub struct RouteGuard;\nimpl RouteGuard {\n    pub fn allow(&self) -> bool { true }\n}\n",
        )
        .expect("write rust");
        let mut codex_change = object!({
            "kind": "tool",
            "targets": {
                "state": "completed",
                "ok": true,
                "tool": "file_change",
                "changes": [{"path": "src/Http/guard.rs", "kind": "add"}]
            }
        });
        assert!(enricher.take(&mut codex_change, 2, 200).is_some());
        let deltas = enricher.take_focus_events();
        assert_eq!(deltas.len(), 1);
        assert_eq!(enricher.ledger.created_entities().len(), 4);
        assert_eq!(enricher.ledger.target_paths().len(), 2);
        assert_eq!(enricher.ledger.target_folders().len(), 2);
        let receipt = enricher.ledger.receipt();
        assert_eq!(
            receipt["created_entities"].as_array().map(Vec::len),
            Some(4)
        );
        assert!(!receipt.to_string().contains("secret-body"));
    }

    #[test]
    fn strip_root_caches_compiled_regexes_and_ledger_normalizes_last_commands() {
        let dir = TempDir::new().expect("temp dir");
        let root_str = dir.path().to_string_lossy().into_owned();
        let unique_root = format!("{root_str}/unique-strip-root-cache-case");
        let cache_key = {
            let mut key = norm(&unique_root, windows(&unique_root));
            while key.ends_with('/') || key.ends_with('\\') {
                key.pop();
            }
            key
        };
        let compiles_for = |key: &str| -> usize {
            strip_root_regex_compile_counts()
                .lock()
                .unwrap()
                .get(key)
                .copied()
                .unwrap_or(0)
        };
        assert_eq!(compiles_for(&cache_key), 0);

        let enricher_a = Enricher::new("claude", &unique_root, None, &[], None, None, false);
        let first = enricher_a
            .text(&format!("cat {unique_root}/src/main.rs"), 200)
            .unwrap();
        assert_eq!(first, "cat src/main.rs");
        assert_eq!(compiles_for(&cache_key), 6);

        let second = enricher_a
            .text(&format!("grep foo {unique_root}/src/lib.rs"), 200)
            .unwrap();
        assert_eq!(second, "grep foo src/lib.rs");
        let enricher_b = Enricher::new("codex", &unique_root, None, &[], None, None, false);
        let third = enricher_b
            .text(&format!("ls {unique_root}/tests"), 200)
            .unwrap();
        assert_eq!(third, "ls tests");
        assert_eq!(
            compiles_for(&cache_key),
            6,
            "strip_root regexes must be cached across Enricher::text calls and instances"
        );

        let win_root = r"C:\Users\dev\repo-strip-root-win-mixed";
        assert_eq!(
            strip_root(
                r"cat C:\Users\dev\repo-strip-root-win-mixed/src/main.rs",
                &[win_root.to_owned()],
                None
            ),
            "cat src/main.rs"
        );
        assert_eq!(
            strip_root(
                "type C:/Users/dev/repo-strip-root-win-mixed\\src\\lib.rs",
                &[win_root.to_owned()],
                None
            ),
            r"type src\lib.rs"
        );
        assert_eq!(compiles_for("C:/Users/dev/repo-strip-root-win-mixed"), 6);

        assert_eq!(
            normalize_command_signature("  cargo test   -p infra-harness 2>&1 | tail -n 20 "),
            "cargo test -p infra-harness"
        );
        assert_eq!(
            normalize_command_signature("cargo test -p infra-harness | head -n 5 >/dev/null"),
            "cargo test -p infra-harness"
        );

        let mut ledger = RunLedger::new("claude", None, false);
        for (idx, cmd, ok, code) in [
            (
                "c1",
                "cargo test -p infra-harness 2>&1 | tail -n 20",
                false,
                101,
            ),
            (
                "c2",
                "cargo test   -p infra-harness | head -n 5",
                false,
                101,
            ),
            ("c3", "cargo test -p infra-harness >/dev/null", true, 0),
        ] {
            let start = object!({"tool": "Bash", "state": "started", "call": idx, "detail": cmd});
            ledger.observe(&object!({}), Some(&start));
            let done = object!({
                "tool": "Bash",
                "state": "completed",
                "call": idx,
                "ok": ok,
                "exit_code": code,
            });
            ledger.observe(&object!({}), Some(&done));
        }
        let receipt = ledger.receipt();
        let last = receipt["last_commands"].as_array().unwrap();
        assert_eq!(last.len(), 3);
        assert_eq!(last[0]["signature"], "cargo test -p infra-harness");
        assert_eq!(last[1]["signature"], "cargo test -p infra-harness");
        assert_eq!(last[2]["signature"], "cargo test -p infra-harness");
        assert_eq!(last[0]["repeats"], 1);
        assert_eq!(last[1]["repeats"], 2);
        assert_eq!(last[2]["repeats"], 3);
        assert_eq!(receipt["max_command_repeats"], 3);
        assert_eq!(
            receipt["command_loops"],
            object!([{"signature": "cargo test -p infra-harness", "count": 3, "failed": 2}])
        );
    }
}
