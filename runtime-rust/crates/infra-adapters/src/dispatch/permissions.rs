//! Claude's project-relative permission projection for a lent/subfolder installation.
use crate::TailTrim;
use infra_core::{Result, RuntimeError, fs};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::OnceLock,
};
type CompiledRegex = std::result::Result<regex::Regex, regex::Error>;
#[derive(Clone, Debug)]
pub enum Token {
    Word(String),
    Operator(String),
}
pub fn lex(text: &str) -> Result<Vec<Token>> {
    let chars = text.chars().collect::<Vec<_>>();
    let (mut i, mut word, mut have, mut quote) = (0, String::new(), false, None);
    let mut output = vec![];
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
                i += 1;
                continue;
            }
            if c == '\\'
                && q == '"'
                && chars
                    .get(i + 1)
                    .is_some_and(|c| ['$', '\u{60}', '"', '\\', '\n'].contains(c))
            {
                i += 1;
                if chars[i] != '\n' {
                    word.push(chars[i]);
                }
                i += 1;
                continue;
            }
            word.push(c);
            i += 1;
            continue;
        }
        match c {
            '\'' | '"' => {
                have = true;
                quote = Some(c);
                i += 1;
            }
            '\\' => {
                i += 1;
                if i >= chars.len() {
                    return Err(RuntimeError::invalid("Unterminated command escape"));
                }
                word.push(chars[i]);
                have = true;
                i += 1;
            }
            c if c.is_whitespace() => {
                if have {
                    output.push(Token::Word(std::mem::take(&mut word)));
                    have = false;
                }
                if c == '\n' {
                    output.push(Token::Operator("\n".into()));
                }
                i += 1;
            }
            ';' | '&' | '|' | '(' | ')' | '<' | '>' => {
                if have {
                    output.push(Token::Word(std::mem::take(&mut word)));
                    have = false;
                }
                let mut op = c.to_string();
                i += 1;
                while i < chars.len() && ";&|()<>".contains(chars[i]) {
                    op.push(chars[i]);
                    i += 1;
                }
                output.push(Token::Operator(op));
            }
            '#' if !have => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            _ => {
                word.push(c);
                have = true;
                i += 1;
            }
        }
    }
    if quote.is_some() {
        return Err(RuntimeError::invalid("Unterminated command quote"));
    }
    if have {
        output.push(Token::Word(word));
    }
    Ok(output)
}
pub fn plain(command: &str) -> bool {
    let chars = command.chars().collect::<Vec<_>>();
    let (mut quoted, mut i) = (false, 0);
    while i < chars.len() {
        if chars[i] == '\'' {
            quoted = !quoted;
        } else if !quoted && chars[i] == '\\' && chars.get(i + 1) == Some(&'\'') {
            i += 1;
        } else if !quoted && ['$', '\u{60}', '\n', '\r', '\\', '<', '>', '"'].contains(&chars[i]) {
            return false;
        }
        i += 1;
    }
    !quoted
}
fn token_word(token: &Token) -> Option<&str> {
    if let Token::Word(value) = token {
        Some(value)
    } else {
        None
    }
}
fn same(first: &Path, second: &Path) -> bool {
    let key = |path: &Path| {
        fs::absolute(path).ok().map(|path| {
            let text = path.to_string_lossy().into_owned();
            if cfg!(windows) {
                text.to_lowercase()
            } else {
                text
            }
        })
    };
    key(first).is_some() && key(first) == key(second)
}
pub fn memory_command(command: &str, root: &Path) -> bool {
    if command.chars().count() > 65536 || !plain(command) {
        return false;
    }
    let Ok(mut tokens) = lex(command) else {
        return false;
    };
    let mut relative = false;
    if tokens.first().and_then(token_word) == Some("cd") {
        if tokens.len() < 3
            || !tokens
                .get(1)
                .and_then(token_word)
                .is_some_and(|path| same(Path::new(path), root))
            || !matches!(&tokens[2],Token::Operator(op)if op=="&&")
        {
            return false;
        }
        tokens.drain(..3);
        relative = true;
    }
    if tokens
        .first()
        .and_then(token_word)
        .is_some_and(|name| ["echo", "printf"].contains(&name))
    {
        let Some(pipe) = tokens
            .iter()
            .position(|token| matches!(token,Token::Operator(op)if op=="|"))
        else {
            return false;
        };
        if pipe < 2
            || tokens[1..pipe]
                .iter()
                .any(|token| matches!(token, Token::Operator(_)))
        {
            return false;
        }
        tokens.drain(..=pipe);
    }
    let Some(program) = tokens.first().and_then(token_word) else {
        return false;
    };
    let engine = super::super::hooks::installed_engine(root);
    if !same(Path::new(program), &engine)
        && !(relative
            && program
                == if cfg!(windows) {
                    "memory-bank/bin/infra-engine.exe"
                } else {
                    "memory-bank/bin/infra-engine"
                })
    {
        return false;
    }
    if tokens.get(1).and_then(token_word) != Some("memory") {
        return false;
    }
    tokens.iter().all(|token| matches!(token, Token::Word(_)))
}
fn rules(root: &Path) -> BTreeMap<&'static str, Vec<String>> {
    let mut result = BTreeMap::from([("allow", vec![]), ("ask", vec![]), ("deny", vec![])]);
    for name in ["settings.json", "settings.local.json"] {
        if let Ok(data) = fs::read_text(&root.join(".claude").join(name))
            .and_then(|raw| serde_json::from_str::<Value>(&raw).map_err(Into::into))
        {
            for key in ["allow", "ask", "deny"] {
                result.get_mut(key).unwrap().extend(
                    data["permissions"][key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(String::from),
                );
            }
        }
    }
    result
}
fn own_rules() -> Vec<String> {
    let config = super::home("claude");
    let paths = [
        config.join("settings.json"),
        PathBuf::from("/etc/claude-code/managed-settings.json"),
        PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json"),
        std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Program Files"))
            .join("ClaudeCode/managed-settings.json"),
    ];
    let mut output = vec![];
    for path in paths {
        if let Ok(data) = fs::read_text(&path)
            .and_then(|raw| serde_json::from_str::<Value>(&raw).map_err(Into::into))
        {
            for key in ["deny", "ask"] {
                output.extend(
                    data["permissions"][key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(String::from),
                );
            }
        }
    }
    output
}
fn glob(pattern: &str) -> String {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut i = 0;
    let mut out = String::new();
    while i < chars.len() {
        if chars[i..].starts_with(&['*', '*', '/']) {
            out.push_str("(?:.*/)?");
            i += 3;
        } else if chars[i..].starts_with(&['*', '*']) {
            out.push_str(".*");
            i += 2;
        } else {
            match chars[i] {
                '*' => out.push_str("[^/]*"),
                '?' => out.push_str("[^/]"),
                c => out.push_str(&regex::escape(&c.to_string())),
            }
            i += 1;
        }
    }
    out
}
fn inside(path: &Path, root: &Path) -> bool {
    let (path, root) = (fs::absolute(path), fs::absolute(root));
    match (path, root) {
        (Ok(path), Ok(root)) if cfg!(windows) => {
            let (path, root) = (
                path.to_string_lossy().to_lowercase(),
                root.to_string_lossy().to_lowercase(),
            );
            path == root || path.starts_with(&format!("{root}{}", std::path::MAIN_SEPARATOR))
        }
        (Ok(path), Ok(root)) => path.starts_with(root),
        _ => false,
    }
}
fn path_matches(content: &str, path: &Path, root: &Path) -> bool {
    let mut text = content.trim();
    let (mut base, mut anchored) = (root.to_path_buf(), None);
    if let Some(rest) = text.strip_prefix("//") {
        base = if cfg!(windows) && rest.as_bytes().get(1) == Some(&b'/') {
            PathBuf::from(format!("{}:/", &rest[..1]))
        } else {
            PathBuf::from(std::path::MAIN_SEPARATOR.to_string())
        };
        text = if cfg!(windows) && rest.as_bytes().get(1) == Some(&b'/') {
            &rest[2..]
        } else {
            rest
        };
        anchored = Some(true);
    } else if let Some(rest) = text.strip_prefix("~/") {
        base = super::user_home();
        text = rest;
        anchored = Some(true);
    } else if let Some(rest) = text.strip_prefix('/') {
        text = rest;
        anchored = Some(true);
    } else if let Some(rest) = text.strip_prefix("./") {
        text = rest;
    }
    let folder = text.ends_with("/**") || text.ends_with('/');
    if let Some(rest) = text.strip_suffix("/**") {
        text = rest;
    } else {
        text = text.tail_matches('/');
    }
    let text = if text.is_empty() { "**" } else { text };
    let anchored = anchored.unwrap_or(text.contains('/'));
    let Ok(path) = fs::absolute(path) else {
        return false;
    };
    let Ok(base) = fs::absolute(&base) else {
        return false;
    };
    let relative = if cfg!(windows) {
        let p = path.to_string_lossy().replace('\\', "/").to_lowercase();
        let b = base.to_string_lossy().replace('\\', "/").to_lowercase();
        if !p.starts_with(&format!("{}/", b.tail_matches('/'))) {
            return false;
        }
        p[b.tail_matches('/').len() + 1..].into()
    } else {
        let Ok(relative) = path.strip_prefix(&base) else {
            return false;
        };
        relative.to_string_lossy().replace('\\', "/")
    };
    let names = relative
        .split('/')
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    let Ok(pattern) = regex::RegexBuilder::new(&format!("^(?:{})$", glob(text)))
        .case_insensitive(cfg!(windows))
        .build()
    else {
        return false;
    };
    for depth in 1..=names.len() {
        if folder && depth == names.len() && !path.is_dir() {
            continue;
        }
        let candidate = if anchored {
            names[..depth].join("/")
        } else {
            names[depth - 1].into()
        };
        if pattern.is_match(&candidate) {
            return true;
        }
    }
    false
}
fn file_paths(input: &Value, key: &str, start: &Path) -> Vec<PathBuf> {
    let Some(value) = input[key]
        .as_str()
        .filter(|value| !value.is_empty() && !value.contains('\0'))
    else {
        return vec![];
    };
    let value = super::native_path(value);
    let value = if value.is_absolute() {
        value
    } else {
        start.join(value)
    };
    let Ok(lexical) = fs::absolute(&value) else {
        return vec![];
    };
    let resolved = fs::canonical_or_missing(&lexical).unwrap_or_else(|_| lexical.clone());
    if resolved == lexical {
        vec![lexical]
    } else {
        vec![lexical, resolved]
    }
}
fn assignments(value: &str) -> bool {
    static ASSIGNMENT: OnceLock<CompiledRegex> = OnceLock::new();
    ASSIGNMENT
        .get_or_init(|| regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*\+?=.*$"))
        .as_ref()
        .is_ok_and(|pattern| pattern.is_match(value))
}
fn segments(command: &str) -> Vec<Vec<String>> {
    let Ok(segments) = crate::shell::split(command) else {
        return vec![];
    };
    segments
        .into_iter()
        .filter_map(|segment| {
            let mut words = segment.words;
            while words.first().is_some_and(|word| assignments(word)) {
                words.remove(0);
            }
            (!words.is_empty()).then_some(words)
        })
        .collect()
}
fn forms(words: &[String]) -> Vec<Vec<String>> {
    static DURATION: OnceLock<CompiledRegex> = OnceLock::new();
    let duration_pattern = DURATION
        .get_or_init(|| regex::Regex::new(r"^[0-9.]+[smhd]?$"))
        .as_ref()
        .ok();
    let mut current = words.to_vec();
    let mut output = vec![current.clone()];
    let wrappers = [
        "time", "nohup", "timeout", "nice", "stdbuf", "env", "command", "builtin", "noglob",
        "xargs", "sudo", "doas", "exec", "setsid", "ionice", "pkexec", "runuser", "eval",
    ];
    while let Some(first) = current.first() {
        if [
            "if", "then", "else", "elif", "do", "while", "until", "!", "{", "}",
        ]
        .contains(&first.as_str())
            || assignments(first)
        {
            current.remove(0);
            output.push(current.clone());
            continue;
        }
        let name = first.clone();
        if !wrappers.contains(&name.as_str()) {
            break;
        }
        current.remove(0);
        if name == "xargs" && current.first().is_none_or(|word| word.starts_with('-')) {
            break;
        }
        if name == "command"
            && current.first().is_some_and(|word| {
                word.starts_with('-') && (word.contains('v') || word.contains('V'))
            })
        {
            break;
        }
        while current.first().is_some_and(|word| {
            word.starts_with('-')
                || duration_pattern
                    .as_ref()
                    .is_some_and(|pattern| pattern.is_match(word))
                || name == "env" && assignments(word)
        }) {
            let option = current.remove(0);
            let takes = match name.as_str() {
                "env" => &["-u"][..],
                "sudo" => &["-u", "-g", "-C", "-h", "-p", "-U", "-r", "-t"][..],
                "doas" => &["-u", "-C"][..],
                "timeout" => &["-k", "-s"][..],
                "runuser" => &["-u", "-g"][..],
                _ => &[],
            };
            if takes.contains(&option.as_str()) && !current.is_empty() {
                current.remove(0);
            }
        }
        output.push(current.clone());
    }
    output
}
fn bash_matches(content: &str, words: &[String]) -> bool {
    if let Some(prefix) = content.strip_suffix(":*") {
        let Ok(prefix) = lex(prefix.trim()) else {
            return false;
        };
        let prefix = prefix.iter().filter_map(token_word).collect::<Vec<_>>();
        return !prefix.is_empty()
            && words.len() >= prefix.len()
            && words.iter().zip(prefix).all(|(left, right)| left == right);
    }
    if content.contains('*') {
        let expression = content
            .trim()
            .chars()
            .map(|c| {
                if c == '*' {
                    ".*".into()
                } else {
                    regex::escape(&c.to_string())
                }
            })
            .collect::<String>();
        regex::Regex::new(&format!("^(?:{expression})$"))
            .is_ok_and(|pattern| pattern.is_match(&words.join(" ")))
    } else {
        words.join(" ") == content.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}
fn matches(rule: &str, tool: &str, input: &Value, root: &Path, start: &Path, allow: bool) -> bool {
    static PERMISSION: OnceLock<CompiledRegex> = OnceLock::new();
    let Ok(pattern) = PERMISSION
        .get_or_init(|| regex::Regex::new(r"(?s)^\s*([A-Za-z_][A-Za-z0-9_-]*)(?:\((.*)\))?\s*$"))
        .as_ref()
    else {
        return false;
    };
    let Some(parts) = pattern.captures(rule) else {
        return false;
    };
    let name = &parts[1];
    let Some(content) = parts.get(2).map(|value| value.as_str()) else {
        return name == tool || name.starts_with("mcp__") && tool.starts_with(&format!("{name}__"));
    };
    let edit = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
    if ["Read", "Edit"].contains(&name) && (tool == "Read" || edit.contains(&tool)) {
        if name == "Edit" && !edit.contains(&tool)
            || name == "Read" && edit.contains(&tool) && allow
        {
            return false;
        }
        let paths = file_paths(
            input,
            if tool == "NotebookEdit" {
                "notebook_path"
            } else {
                "file_path"
            },
            start,
        );
        return if allow {
            !paths.is_empty()
                && paths
                    .iter()
                    .all(|path| inside(path, root) && path_matches(content, path, root))
        } else {
            paths.iter().any(|path| path_matches(content, path, root))
        };
    }
    if name == "Bash" && tool == "Bash" {
        let Some(command) = input["command"].as_str() else {
            return false;
        };
        let segments = segments(command);
        return if allow {
            plain(command) && segments.len() == 1 && bash_matches(content, &segments[0])
        } else {
            segments.iter().any(|segment| {
                forms(segment)
                    .iter()
                    .any(|form| bash_matches(content, form))
            })
        };
    }
    if ["Agent", "Task"].contains(&name) && ["Agent", "Task"].contains(&tool) && !allow {
        return input["subagent_type"].as_str() == Some(content.trim());
    }
    if name == "WebFetch" && tool == "WebFetch" {
        let Some(domain) = content.strip_prefix("domain:") else {
            return false;
        };
        let Some(host) = input["url"]
            .as_str()
            .and_then(|value| url::Url::parse(value).ok())
            .and_then(|url| url.host_str().map(str::to_string))
        else {
            return false;
        };
        let (domain, host) = (domain.trim().to_lowercase(), host.to_lowercase());
        return if let Some(suffix) = domain.strip_prefix('*') {
            host.ends_with(suffix)
        } else {
            domain == host
        };
    }
    false
}
pub fn permission(
    payload: &Value,
    root: &Path,
    start: &Path,
    project: Option<&Path>,
) -> Option<Value> {
    let (tool, input) = (
        payload["tool_name"].as_str()?,
        payload
            .get("tool_input")
            .filter(|value| value.is_object())?,
    );
    let rules = rules(root);
    let places = project
        .into_iter()
        .chain(std::iter::once(root))
        .collect::<Vec<_>>();
    let place = project.unwrap_or(root);
    for kind in ["deny", "ask", "allow"] {
        for rule in &rules[kind] {
            let found = if kind == "allow" {
                matches(rule, tool, input, place, start, true)
            } else {
                places
                    .iter()
                    .any(|base| matches(rule, tool, input, base, start, false))
            };
            if found {
                if kind == "allow"
                    && own_rules().iter().any(|own| {
                        places
                            .iter()
                            .copied()
                            .chain(std::iter::once(start))
                            .any(|base| matches(own, tool, input, base, start, false))
                    })
                {
                    return None;
                }
                let mut answer = json!({"hookEventName":"PreToolUse","permissionDecision":kind});
                if kind == "deny" {
                    answer["permissionDecisionReason"]=format!("The project's settings deny this ({rule}), as they would in the project root.").into();
                }
                return Some(json!({"hookSpecificOutput":answer}));
            }
        }
    }
    None
}
pub fn environment(root: &Path, project: Option<&Path>) -> Vec<String> {
    static ENVIRONMENT_NAME: OnceLock<CompiledRegex> = OnceLock::new();
    let environment_name = ENVIRONMENT_NAME
        .get_or_init(|| regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$"))
        .as_ref()
        .ok();
    let mut values = BTreeMap::new();
    for name in ["settings.json", "settings.local.json"] {
        if let Ok(data) = fs::read_text(&root.join(".claude").join(name))
            .and_then(|raw| serde_json::from_str::<Value>(&raw).map_err(Into::into))
        {
            for (key, value) in data["env"].as_object().into_iter().flatten() {
                if !environment_name
                    .as_ref()
                    .is_some_and(|pattern| pattern.is_match(key))
                    || key.starts_with("CLAUDE_")
                    || key.starts_with("ANTHROPIC_")
                {
                    continue;
                }
                let value = match value {
                    Value::String(value) => value.clone(),
                    Value::Number(value) => value.to_string(),
                    Value::Bool(value) => value.to_string(),
                    _ => continue,
                };
                values.insert(key.clone(), value);
            }
        }
    }
    if let Some(project) = project {
        values.insert("ACCELERATOR_HOME".into(), root.display().to_string());
        values.insert("ACCELERATOR_STATE_DIR".into(), root.display().to_string());
        values.insert(
            "ACCELERATOR_PROJECT_DIR".into(),
            project.display().to_string(),
        );
    }
    values
        .into_iter()
        .map(|(key, value)| format!("export {key}={}", infra_core::native::quote(&value)))
        .collect()
}

#[cfg(test)]
mod fixed_grammar_tests {
    use super::*;
    use std::fs as std_fs;

    #[test]
    fn assignments_keep_ascii_names_append_syntax_and_newline_boundaries() {
        for _ in 0..2 {
            for value in ["A=", "A+=value", "_2=value=other", "Name=значение"] {
                assert!(assignments(value), "{value:?}");
            }
            for value in [
                "",
                "1A=value",
                "é=value",
                "A++=value",
                "A-B=value",
                "A=line\nbreak",
            ] {
                assert!(!assignments(value), "{value:?}");
            }
        }
        let split = segments("A='line\nbreak' echo harmless");
        assert_eq!(
            split,
            vec![vec![
                "A=line\nbreak".to_string(),
                "echo".into(),
                "harmless".into()
            ]]
        );
    }

    #[test]
    fn duration_grammar_and_wrapper_unwrapping_keep_existing_forms() {
        for _ in 0..2 {
            for duration in [".", "1..2m", "0.5s", "42", "3h", "4d"] {
                let words = ["timeout", duration, "echo", "harmless"].map(String::from);
                assert_eq!(
                    forms(&words).last().unwrap(),
                    &["echo", "harmless"],
                    "{duration}"
                );
            }
            for duration in ["1ms", "1M", "1s2", "forever"] {
                let words = ["timeout", duration, "echo", "harmless"].map(String::from);
                assert!(
                    !forms(&words)
                        .iter()
                        .any(|form| form.first().is_some_and(|word| word == "echo")),
                    "{duration}"
                );
            }
        }
    }

    #[test]
    fn permission_grammar_keeps_bare_multiline_mcp_and_case_semantics() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for _ in 0..2 {
            for (rule, tool, input, expected) in [
                ("Bash", "Bash", json!({}), true),
                (" Bash\n", "Bash", json!({}), true),
                ("bash", "Bash", json!({}), false),
                ("1Bad", "1Bad", json!({}), false),
                (
                    "Bash(echo",
                    "Bash",
                    json!({"command":"echo harmless"}),
                    false,
                ),
                (
                    "Bash(echo\n harmless)",
                    "Bash",
                    json!({"command":"echo harmless"}),
                    true,
                ),
                ("mcp__files", "mcp__files__read", json!({}), true),
                ("mcp__files", "mcp__files2__read", json!({}), false),
                ("mcp__files(read)", "mcp__files__read", json!({}), false),
            ] {
                assert_eq!(
                    matches(rule, tool, &input, root, root, false),
                    expected,
                    "{rule:?}, {tool}"
                );
            }
        }
    }

    #[test]
    fn private_deny_projection_preserves_assignment_and_wrapper_decisions() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std_fs::create_dir(root.join(".claude")).unwrap();
        let settings = root.join(".claude/settings.json");
        let raw = serde_json::to_vec(
            &json!({"permissions":{"deny":["Read","Bash(unmatched:*)","Bash(echo:*)"]}}),
        )
        .unwrap();
        std_fs::write(&settings, &raw).unwrap();
        let expected = json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"The project's settings deny this (Bash(echo:*)), as they would in the project root."}});
        // Only deny rules are present: this never projects personal allow/ask rules.
        for _ in 0..2 {
            for (command, denied) in [
                ("A+=value echo harmless", true),
                ("A0=x A1=y env A2=z timeout 1s echo harmless", true),
                (
                    "env -u DROP A=x timeout -s TERM -k 1s 2m echo harmless",
                    true,
                ),
                ("timeout . echo harmless", true),
                ("timeout 1..2m echo harmless", true),
                ("command -- echo harmless", true),
                ("xargs echo harmless", true),
                ("command -v echo", false),
                ("command -V echo", false),
                ("xargs -n 1 echo harmless", false),
                ("1A=x echo harmless", false),
                ("A='line\nbreak' echo harmless", false),
            ] {
                let payload = json!({"tool_name":"Bash","tool_input":{"command":command}});
                assert_eq!(
                    permission(&payload, root, root, None),
                    denied.then(|| expected.clone()),
                    "{command:?}"
                );
            }
        }
        assert_eq!(std_fs::read(&settings).unwrap(), raw);
        std_fs::write(&settings, br#"{"permissions":{"deny":["Bash(other:*)"]}}"#).unwrap();
        assert_eq!(
            permission(
                &json!({"tool_name":"Bash","tool_input":{"command":"echo harmless"}}),
                root,
                root,
                None
            ),
            None,
            "fixed grammar reuse must not cache settings"
        );
    }

    #[test]
    fn environment_projection_keeps_name_filters_local_overrides_and_scalar_values() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std_fs::create_dir(root.join(".claude")).unwrap();
        std_fs::write(root.join(".claude/settings.json"), serde_json::to_vec(&json!({"env":{
            "BOOL":true,"NUMBER":7,"PLAIN":"base","QUOTED":"a'b","_ALLOWED":"yes",
            "CLAUDE_PRIVATE":"skip","ANTHROPIC_PRIVATE":"skip","1BAD":"skip","BAD-NAME":"skip","é":"skip","ARRAY":[],"NULL":null
        }})).unwrap()).unwrap();
        let local = root.join(".claude/settings.local.json");
        std_fs::write(&local, br#"{"env":{"PLAIN":"hello world"}}"#).unwrap();
        let expected = [
            "export BOOL='true'",
            "export NUMBER='7'",
            "export PLAIN='hello world'",
            "export QUOTED='a'\\''b'",
            "export _ALLOWED='yes'",
        ];
        assert_eq!(environment(root, None), expected);
        assert_eq!(environment(root, None), expected);
        std_fs::write(local, br#"{"env":{"PLAIN":"updated"}}"#).unwrap();
        assert!(environment(root, None).contains(&"export PLAIN='updated'".into()));
    }
}
