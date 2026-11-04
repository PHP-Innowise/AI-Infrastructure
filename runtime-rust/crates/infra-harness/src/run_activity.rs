//! Display enrichment and activity counts never retain tool output or file contents.
use fancy_regex::Regex;
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::OnceLock,
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
        let mut variants = BTreeSet::new();
        variants.insert(root.clone());
        if win {
            variants.insert(root.replace('\\', "/"));
            variants.insert(root.replace('/', "\\"));
        }
        for variant in variants {
            if !(if win {
                result.to_lowercase().contains(&variant.to_lowercase())
            } else {
                result.contains(&variant)
            }) {
                continue;
            }
            let escaped = regex::escape(&variant);
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
                    (
                        format!(r#"{start}[\\/]+(?=[^\s"'`;|&<>()])"#),
                        inner.clone(),
                    ),
                    (format!(r"{start}[\\/]+"), format!("{name}/")),
                    (format!(r"{start}(?![\w./\\-])"), name.into()),
                ] {
                    let Ok(pattern) = Regex::new(&pattern) else {
                        return "[redacted]".into();
                    };
                    let Some(clean) = replace(&pattern, &result, 0, Some(&replacement)) else {
                        return "[redacted]".into();
                    };
                    result = clean;
                }
            }
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
    watch: Option<BTreeMap<String, String>>,
    watch_outside: bool,
    watched: BTreeSet<String>,
    reads: u64,
    outside_reads: u64,
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
            watch,
            watch_outside: outside,
            watched: BTreeSet::new(),
            reads: 0,
            outside_reads: 0,
        }
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
            entry.read = path;
        } else if kind == "skill"
            && fields["detail"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        {
            self.reads += 1;
            entry.read = Some(format!("skill:{}", fields["detail"].as_str().unwrap_or("")));
        } else if (edit(&kind) || delete(&kind))
            && fields["path"].as_str().is_some_and(|path| !path.is_empty())
        {
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
            self.command_id += 1;
            entry.command = Some(self.command_id);
            self.last_commands.push_back((self.command_id,object!({"command":fields["detail"].as_str().unwrap_or(""),"ok":null,"outcome":null,"exit_code":null})));
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
        entry
    }
    fn finish(&mut self, entry: Entry, fields: &Value, private: &Private) {
        let ok = fields["ok"] != false;
        let not_run = fields["outcome"] == "not_run";
        if let Some(id) = entry.command {
            if let Some((_, command)) = self.last_commands.iter_mut().find(|(key, _)| *key == id) {
                command["ok"] = ok.into();
                command["outcome"] = fields["outcome"].clone();
                command["exit_code"] = fields["exit_code"].clone();
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
            if !self.edited.iter().any(|(old, _)| old == &key) {
                self.edited.push((key.clone(), path));
            }
            if kind == "add" {
                self.created.insert(key);
            } else if kind == "delete" {
                self.deleted.insert(key);
            }
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
        let mut receipt = object!({"steps":self.steps,"opened":self.opened.len(),"opened_paths":listed(&self.opened),"edited":self.edited.len(),"edited_paths":listed(&self.edited),"created":self.created.len(),"deleted":self.deleted.len(),"searched":self.searched,"patterns":self.patterns.iter().take(12).collect::<Vec<_>>(),"commands":self.commands,"failed":self.failed,"not_run":self.not_run,"failed_steps":self.failed_steps,"last_commands":self.last_commands.iter().map(|(_,value)|value.clone()).collect::<Vec<_>>(),"plan":self.plan,"helpers":self.helpers,"limited":self.limited});
        if let Some(memory) = self.memory() {
            receipt["memory"] = memory;
        }
        receipt
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
            ledger: RunLedger::new(provider, watch, outside),
        }
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
