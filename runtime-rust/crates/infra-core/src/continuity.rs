//! Conversation metadata, recent files and scoped Cursor continuation topics.
use crate::{
    BrainRepository, Result, RuntimeConfig, RuntimeContext, RuntimeError, Store,
    bank::publication::PublicationFiles,
    domain::{self, clock, json},
    query, storage,
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json as object};
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub fn recent_key(context: &RuntimeContext) -> Result<String> {
    if !context.roots.attached() {
        return Ok("recent_files".into());
    }
    let path = crate::fs::canonicalize(&context.roots.project)?;
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().as_bytes().to_vec();
    Ok(format!("recent_files:{}", &query::sha256(&bytes)[..16]))
}
pub fn load_recent(store: &Store, key: &str) -> Vec<String> {
    store
        .conn
        .query_row("SELECT value FROM index_state WHERE key=?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .ok()
        .flatten()
        .and_then(|raw| json::parse(&raw).ok())
        .filter(|value| value["version"].as_u64() == Some(1))
        .and_then(|value| value["paths"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(String::from))
        .take(64)
        .collect()
}
pub fn record_recent(
    store: &mut Store,
    context: &RuntimeContext,
    paths: &[String],
) -> Result<usize> {
    if paths.is_empty() {
        return Ok(0);
    }
    let key = recent_key(context)?;
    let mut merged = paths.to_vec();
    merged.extend(load_recent(store, &key));
    merged = domain::unique_strings(merged);
    merged.truncate(64);
    let raw = json::python_default(&object!({"version":1,"paths":merged}), false, false, true)?;
    store.transaction(|conn|{conn.execute("INSERT INTO index_state(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,raw])?;Ok(())})?;
    Ok(paths.len())
}
pub fn kept(path: &str) -> bool {
    !path.starts_with('/')
        && !path.split('/').any(|part| part == "..")
        && ![
            "vendor/",
            "node_modules/",
            "var/",
            ".git/",
            "memory-bank/local/",
            "project-brain/dynamic/",
            "project-brain/control/",
            "project-brain/indexes/",
            "project-brain/archive/",
            "project-brain/local/",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix))
        && ![".md", ".mdx", ".rst", ".txt", ".adoc"]
            .iter()
            .any(|suffix| path.to_lowercase().ends_with(suffix))
        && domain::source_path_problem(path).is_none()
}
pub fn attach_recent(
    store: &Store,
    context: &RuntimeContext,
    capsule: &mut crate::Capsule,
) -> Result<()> {
    let files = load_recent(store, &recent_key(context)?)
        .into_iter()
        .filter(|path| kept(path) && context.roots.project.join(path).is_file())
        .take(10)
        .collect::<Vec<_>>();
    if !files.is_empty() {
        capsule.0["recent_files"] = object!(files);
    }
    Ok(())
}
fn project_file(word: &str, base: &Path, root: &Path) -> Option<String> {
    let word = word
        .trim()
        .trim_matches(|character| "'\"`,;()[]{}<>".contains(character));
    if word.is_empty() || word.starts_with('-') || word.chars().count() > 400 || word.contains('\0')
    {
        return None;
    }
    let mut candidates = vec![word.to_string()];
    if word.contains(':') && !Path::new(word).is_absolute() {
        candidates.push(word.split_once(':')?.1.into());
    }
    if let Ok(pattern) = regex::Regex::new(r"(:[0-9]+(:[0-9]+)?|#L[0-9]+(-L?[0-9]+)?)$") {
        candidates.push(pattern.replace(word, "").into_owned());
    }
    for candidate in candidates {
        let joined = if Path::new(&candidate).is_absolute() {
            PathBuf::from(candidate)
        } else {
            base.join(candidate)
        };
        let Ok(resolved) = crate::fs::canonicalize(joined) else {
            continue;
        };
        if resolved.is_file()
            && resolved.starts_with(root)
            && resolved != root
            && let Ok(relative) = resolved.strip_prefix(root)
        {
            return relative
                .to_str()
                .map(|relative| relative.replace('\\', "/"));
        }
    }
    None
}
#[derive(Clone, Debug)]
struct Token {
    text: String,
    operator: bool,
    quoted: bool,
}
fn shell_tokens(command: &str, powershell: bool) -> Vec<Token> {
    let mut output = Vec::new();
    let mut value = String::new();
    let mut quote = None;
    let mut quoted = false;
    let mut escaped = false;
    let mut characters = command.chars().peekable();
    while let Some(character) = characters.next() {
        if escaped {
            value.push(character);
            escaped = false;
            continue;
        }
        if character == if powershell { '`' } else { '\\' } && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(current) = quote {
            if character == current {
                if powershell && current == '\'' && characters.peek() == Some(&'\'') {
                    characters.next();
                    value.push('\'');
                } else {
                    quote = None;
                }
            } else {
                value.push(character);
            }
            continue;
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
            quoted = true;
            continue;
        }
        if character.is_whitespace() {
            if !value.is_empty() {
                output.push(Token {
                    text: std::mem::take(&mut value),
                    operator: false,
                    quoted,
                });
                quoted = false;
            }
            if character == '\n' {
                output.push(Token {
                    text: ";".into(),
                    operator: true,
                    quoted: false,
                });
            }
            continue;
        }
        if "();<>|&".contains(character) {
            if !value.is_empty() {
                output.push(Token {
                    text: std::mem::take(&mut value),
                    operator: false,
                    quoted,
                });
                quoted = false;
            }
            let mut operator = character.to_string();
            if characters.peek().is_some_and(|next| {
                *next == character
                    || character == '>' && *next == '&'
                    || character == '&' && *next == '>'
            }) {
                operator.push(characters.next().unwrap_or(character));
            }
            output.push(Token {
                text: operator,
                operator: true,
                quoted: false,
            });
        } else {
            value.push(character);
        }
    }
    if !value.is_empty() {
        output.push(Token {
            text: value,
            operator: false,
            quoted,
        });
    }
    output
}
fn without_heredocs(command: &str) -> String {
    let pattern = regex::Regex::new(r#"<<(-?)[ \t]*['\"]?([A-Za-z_][A-Za-z0-9_]*)['\"]?"#).ok();
    let mut waiting = Vec::<(String, bool)>::new();
    let mut output = Vec::new();
    for line in command.lines() {
        if let Some((delimiter, tabs)) = waiting.first() {
            let candidate = if *tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            if candidate == delimiter {
                waiting.remove(0);
            }
            continue;
        }
        if let Some(pattern) = &pattern {
            for captures in pattern.captures_iter(line) {
                waiting.push((captures[2].into(), &captures[1] == "-"));
            }
        }
        output.push(line);
    }
    output.join("\n")
}
fn shell_files(command: &str, base: &Path, powershell: bool) -> Vec<(String, bool, PathBuf)> {
    let tokens = shell_tokens(&without_heredocs(command), powershell);
    let mut output = Vec::new();
    let mut directory = base.to_path_buf();
    let mut segment = Vec::<Token>::new();
    let process = |segment: &mut Vec<Token>,
                   directory: &mut PathBuf,
                   output: &mut Vec<(String, bool, PathBuf)>| {
        if segment.is_empty() {
            return;
        }
        let mut at = 0;
        while at < segment.len() {
            let verb = segment[at].text.to_lowercase();
            if segment[at].text.contains('=') && !segment[at].quoted {
                at += 1;
                continue;
            }
            if [
                "sudo", "time", "nice", "nohup", "exec", "command", "env", "xargs", "timeout",
            ]
            .contains(&verb.as_str())
            {
                at += 1;
                while at < segment.len()
                    && (segment[at].text.starts_with('-')
                        || segment[at].text.parse::<f64>().is_ok()
                        || segment[at].text.contains('='))
                {
                    at += 1;
                }
                continue;
            }
            break;
        }
        if at >= segment.len() {
            segment.clear();
            return;
        }
        let verb = Path::new(&segment[at].text)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_lowercase();
        at += 1;
        if [
            "cd",
            "pushd",
            "chdir",
            "set-location",
            "sl",
            "push-location",
        ]
        .contains(&verb.as_str())
        {
            if let Some(path) = segment.get(at) {
                let joined = directory.join(&path.text);
                if let Ok(path) = crate::fs::canonicalize(joined) {
                    *directory = path;
                }
            }
            segment.clear();
            return;
        }
        let read = [
            "cat",
            "head",
            "tail",
            "sed",
            "awk",
            "nl",
            "less",
            "more",
            "bat",
            "grep",
            "egrep",
            "fgrep",
            "rg",
            "ag",
            "ack",
            "wc",
            "diff",
            "cmp",
            "jq",
            "yq",
            "xxd",
            "od",
            "strings",
            "cut",
            "sort",
            "uniq",
            "column",
            "get-content",
            "gc",
            "type",
            "select-string",
            "sls",
            "format-hex",
            "findstr",
        ]
        .contains(&verb.as_str());
        let mut write = [
            "tee",
            "rm",
            "touch",
            "truncate",
            "chmod",
            "set-content",
            "sc",
            "add-content",
            "ac",
            "clear-content",
            "clc",
            "out-file",
            "new-item",
            "ni",
            "remove-item",
            "ri",
            "del",
            "erase",
            "tee-object",
        ]
        .contains(&verb.as_str());
        let copy = [
            "cp",
            "ln",
            "install",
            "mv",
            "copy-item",
            "copy",
            "cpi",
            "move-item",
            "move",
            "mi",
            "rename-item",
            "rni",
            "ren",
        ]
        .contains(&verb.as_str());
        if ["sed", "perl"].contains(&verb.as_str())
            && segment.iter().any(|token| {
                token.text == "-i"
                    || token.text.starts_with("-i.")
                    || token.text.starts_with("-pi")
                    || token.text.starts_with("-0pi")
            })
        {
            write = true;
        }
        let mut git = false;
        if verb == "git" {
            while at < segment.len() && segment[at].text.starts_with('-') {
                if segment[at].text == "-C" && at + 1 < segment.len() {
                    let joined = directory.join(&segment[at + 1].text);
                    if let Ok(path) = crate::fs::canonicalize(joined) {
                        *directory = path;
                    }
                    at += 2;
                } else if segment[at].text == "-c" {
                    at += 2;
                } else {
                    at += 1;
                }
            }
            if let Some(subcommand) = segment.get(at) {
                let subcommand = subcommand.text.as_str();
                git = [
                    "show", "diff", "log", "blame", "grep", "cat-file", "add", "commit", "mv",
                    "rm", "checkout", "restore", "apply", "stash", "reset", "switch",
                ]
                .contains(&subcommand);
                write = [
                    "add", "commit", "mv", "rm", "checkout", "restore", "apply", "stash", "reset",
                    "switch",
                ]
                .contains(&subcommand);
                at += 1;
            }
        }
        let mut operands = Vec::new();
        let mut skip = false;
        let mut pattern_pending = [
            "grep", "egrep", "fgrep", "rg", "ag", "ack", "sed", "awk", "jq", "yq", "perl",
        ]
        .contains(&verb.as_str());
        while at < segment.len() {
            let token = &segment[at];
            if skip {
                skip = false;
                at += 1;
                continue;
            }
            if token.text.starts_with('-') {
                if [
                    "-e",
                    "--regexp",
                    "-f",
                    "--file",
                    "-g",
                    "--glob",
                    "--iglob",
                    "-t",
                    "--type",
                    "-n",
                    "--lines",
                    "-c",
                    "--bytes",
                    "--context",
                    "-A",
                    "-B",
                    "-C",
                ]
                .contains(&token.text.as_str())
                {
                    if ["-e", "--regexp", "-f", "--file"].contains(&token.text.as_str()) {
                        pattern_pending = false;
                    }
                    skip = true;
                }
                at += 1;
                continue;
            }
            if pattern_pending {
                pattern_pending = false;
                at += 1;
                continue;
            }
            operands.push(token.text.clone());
            at += 1;
        }
        if read || write || copy || git {
            let last = operands.len().saturating_sub(1);
            for (index, path) in operands.into_iter().enumerate() {
                output.push((
                    path,
                    if copy { index == last } else { write },
                    directory.clone(),
                ));
            }
        }
        segment.clear();
    };
    let mut at = 0;
    while at < tokens.len() {
        let token = &tokens[at];
        if token.operator
            && [">", ">>", ">|", "&>", "&>>", "<", "<>", ">&", "<&"].contains(&token.text.as_str())
            && let Some(target) = tokens.get(at + 1)
        {
            if !target.operator && !target.text.bytes().all(|byte| byte.is_ascii_digit()) {
                output.push((
                    target.text.clone(),
                    token.text.contains('>'),
                    directory.clone(),
                ));
            }
            at += 2;
            continue;
        }
        if token.operator {
            process(&mut segment, &mut directory, &mut output);
        } else {
            segment.push(token.clone());
        }
        at += 1;
    }
    process(&mut segment, &mut directory, &mut output);
    output
}
fn literal(code: &str, start: usize) -> Option<(String, usize)> {
    let bytes = code.as_bytes();
    let mut at = start;
    while bytes.get(at).is_some_and(|byte| byte.is_ascii_whitespace()) {
        at += 1;
    }
    let quote = *bytes.get(at)?;
    if !b"'\"`".contains(&quote) {
        return None;
    }
    at += 1;
    let mut value = String::new();
    while at < bytes.len() {
        if bytes[at] == quote {
            return Some((value, at + 1));
        }
        if quote == b'`' && bytes[at..].starts_with(b"${") {
            return None;
        }
        if bytes[at] == b'\\' {
            at += 1;
            let escaped = *bytes.get(at)?;
            match escaped {
                b'n' => value.push('\n'),
                b'r' => value.push('\r'),
                b't' => value.push('\t'),
                b'b' => value.push('\u{8}'),
                b'f' => value.push('\u{c}'),
                b'v' => value.push('\u{b}'),
                b'0' => value.push('\0'),
                b'\n' => {}
                b'u' | b'x' => {
                    let width = if escaped == b'u' { 4 } else { 2 };
                    let encoded = std::str::from_utf8(bytes.get(at + 1..at + 1 + width)?).ok()?;
                    let scalar = u32::from_str_radix(encoded, 16).ok()?;
                    value.push(char::from_u32(scalar)?);
                    at += width;
                }
                other => value.push(other as char),
            }
            at += 1;
        } else {
            let character = code[at..].chars().next()?;
            value.push(character);
            at += character.len_utf8();
        }
    }
    None
}
fn exec_calls(code: &str, root: &Path) -> Vec<(String, Value, PathBuf)> {
    let Ok(pattern) = regex::Regex::new(r"\btools\s*\.\s*(exec_command|apply_patch)\s*\(") else {
        return vec![];
    };
    let mut output = Vec::new();
    for captures in pattern.captures_iter(code) {
        let Some(found) = captures.get(0) else {
            continue;
        };
        let at = found.end();
        if &captures[1] == "apply_patch" {
            if let Some((patch, _)) = literal(code, at) {
                output.push(("apply_patch".into(), object!({"patch":patch}), root.into()));
            }
            continue;
        }
        let tail = &code[at..];
        let Some(begin) = tail.find('{') else {
            continue;
        };
        let mut fields = serde_json::Map::new();
        let mut cursor = at + begin + 1;
        while cursor < code.len() {
            while code
                .as_bytes()
                .get(cursor)
                .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b',')
            {
                cursor += 1;
            }
            if code.as_bytes().get(cursor) == Some(&b'}') {
                break;
            }
            let key = if let Some((key, end)) = literal(code, cursor) {
                cursor = end;
                key
            } else {
                let begin = cursor;
                while code
                    .as_bytes()
                    .get(cursor)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || b"_$".contains(byte))
                {
                    cursor += 1;
                }
                if cursor == begin {
                    break;
                }
                code[begin..cursor].into()
            };
            while code
                .as_bytes()
                .get(cursor)
                .is_some_and(u8::is_ascii_whitespace)
            {
                cursor += 1;
            }
            if code.as_bytes().get(cursor) != Some(&b':') {
                break;
            }
            cursor += 1;
            if let Some((value, end)) = literal(code, cursor) {
                fields.insert(key, Value::String(value));
                cursor = end;
            } else {
                while code
                    .as_bytes()
                    .get(cursor)
                    .is_some_and(|byte| *byte != b',' && *byte != b'}')
                {
                    cursor += 1;
                }
            }
        }
        if let Some(command) = fields
            .get("cmd")
            .or(fields.get("command"))
            .and_then(Value::as_str)
        {
            let base = fields
                .get("workdir")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_else(|| root.into());
            output.push(("Bash".into(), object!({"command":command}), base));
        }
    }
    output
}
fn calls(record: &Value, root: &Path) -> Vec<(String, Value, PathBuf)> {
    let mut output = Vec::new();
    if record["type"] == "assistant" && record["isSidechain"] != true {
        let base = record["cwd"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| root.into());
        for part in record["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if part["type"] == "tool_use" && part["input"].is_object() {
                output.push((
                    part["name"].as_str().unwrap_or("").into(),
                    part["input"].clone(),
                    base.clone(),
                ));
            }
        }
    } else if record["type"] == "response_item" {
        let payload = &record["payload"];
        let kind = payload["type"].as_str().unwrap_or("");
        if kind == "custom_tool_call" && payload["name"] == "exec" {
            if let Some(code) = payload["input"].as_str() {
                output.extend(exec_calls(code, root));
            }
        } else if kind == "custom_tool_call" && payload["name"] == "apply_patch" {
            output.push((
                "apply_patch".into(),
                object!({"patch":payload["input"]}),
                root.into(),
            ));
        } else if kind == "function_call" {
            let data = payload["arguments"]
                .as_str()
                .and_then(|raw| json::parse(raw).ok())
                .unwrap_or(object!({}));
            let base = data["workdir"]
                .as_str()
                .map(PathBuf::from)
                .unwrap_or_else(|| root.into());
            if payload["name"] == "apply_patch" {
                output.push(("apply_patch".into(), object!({"patch":data["input"]}), base));
            } else {
                let command = data["cmd"].as_str().or(data["command"].as_str());
                if let Some(command) = command {
                    output.push(("Bash".into(), object!({"command":command}), base));
                }
            }
        } else if kind == "local_shell_call" {
            let action = &payload["action"];
            let base = action["working_directory"]
                .as_str()
                .map(PathBuf::from)
                .unwrap_or_else(|| root.into());
            if let Some(command) = action["command"].as_array() {
                let parts = command.iter().filter_map(Value::as_str).collect::<Vec<_>>();
                let powershell = parts.first().is_some_and(|part| {
                    part.to_lowercase().contains("powershell")
                        || part.to_lowercase().contains("pwsh")
                });
                if let Some(script) = parts.last() {
                    output.push((
                        if powershell { "PowerShell" } else { "Bash" }.into(),
                        object!({"command":script}),
                        base,
                    ));
                }
            }
        }
    }
    output
}
fn prompt(record: &Value) -> bool {
    if record["type"] == "user" {
        if ["isMeta", "toolUseResult", "isCompactSummary", "isSidechain"]
            .iter()
            .any(|key| record[*key] == true)
        {
            return false;
        }
        let content = &record["message"]["content"];
        if content
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| part["type"] == "tool_result"))
        {
            return false;
        }
        let text = content.as_str().map(String::from).unwrap_or_else(|| {
            content
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|part| part["text"].as_str())
                .collect()
        });
        return !text.trim().is_empty()
            && !["INTERNAL_NATIVE_MEMORY_RESULT ", "<local-command"]
                .iter()
                .any(|prefix| text.trim_start().starts_with(prefix));
    }
    if record["type"] == "response_item"
        && record["payload"]["type"] == "message"
        && record["payload"]["role"] == "user"
    {
        let text = record["payload"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|part| part["text"].as_str())
            .collect::<String>();
        return !text.trim().is_empty()
            && !text.trim_start().starts_with("# AGENTS.md instructions")
            && !text.trim_start().starts_with("<environment_context>")
            && !text
                .trim_start()
                .starts_with("INTERNAL_NATIVE_MEMORY_RESULT ");
    }
    false
}
pub fn transcript_files(transcript: Option<&Path>, context: &RuntimeContext) -> Vec<String> {
    let operation = (|| -> Result<Vec<String>> {
        let Some(path) = transcript else {
            return Ok(vec![]);
        };
        if !path.is_absolute() {
            return Ok(vec![]);
        }
        let metadata = std::fs::symlink_metadata(path)?;
        if crate::fs::linked_at(path, &metadata)? || !metadata.is_file() {
            return Ok(vec![]);
        }
        let mut file = std::fs::File::open(path)?;
        let start = metadata.len().saturating_sub(4 * 1024 * 1024);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024).read_to_end(&mut bytes)?;
        let records = bytes
            .split(|byte| *byte == b'\n')
            .skip(usize::from(start > 0))
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .filter(Value::is_object)
            .collect::<Vec<_>>();
        let begin = records
            .iter()
            .rposition(|record| {
                record["type"] == "event_msg" && record["payload"]["type"] == "task_started"
            })
            .or_else(|| records.iter().rposition(prompt))
            .map(|index| index + 1)
            .unwrap_or(0);
        let root = crate::fs::canonicalize(&context.roots.project)?;
        let deadline = Instant::now() + Duration::from_secs(1);
        let patch_paths = regex::Regex::new(
            r"(?m)^\*\*\* (?:Update|Add|Delete) File: (.+)$|^\*\*\* Move to: (.+)$",
        )
        .map_err(|_| RuntimeError::invalid("Invalid patch metadata expression"))?;
        let mut changed = Vec::new();
        let mut read = Vec::new();
        for record in &records[begin..] {
            if Instant::now() > deadline {
                break;
            }
            for (name, data, base) in calls(record, &root) {
                let mut words = Vec::new();
                match name.as_str() {
                    "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => {
                        if let Some(path) = data["file_path"]
                            .as_str()
                            .or(data["notebook_path"].as_str())
                        {
                            words.push((path.into(), true, base.clone()));
                        }
                    }
                    "Read" | "Grep" => {
                        if let Some(path) = data["file_path"].as_str().or(data["path"].as_str()) {
                            words.push((path.into(), false, base.clone()));
                        }
                    }
                    "Bash" | "PowerShell" => {
                        if let Some(command) = data["command"].as_str() {
                            words.extend(shell_files(command, &base, name == "PowerShell"));
                        }
                    }
                    "apply_patch" => {
                        if let Some(patch) = data["patch"].as_str() {
                            for captures in patch_paths.captures_iter(patch) {
                                if let Some(path) = captures.get(1).or(captures.get(2)) {
                                    words.push((path.as_str().trim().into(), true, base.clone()));
                                }
                            }
                        }
                    }
                    _ => {}
                }
                for (word, write, base) in words {
                    if let Some(path) = project_file(&word, &base, &root)
                        && kept(&path)
                    {
                        if write {
                            changed.push(path);
                        } else {
                            read.push(path);
                        }
                    }
                }
            }
        }
        changed.extend(read);
        let mut paths = domain::unique_strings(changed);
        if let Ok(Some(frame)) = crate::git::GitFrame::discover(&root)
            && let Ok(ignored) = frame.ignored(&paths)
        {
            paths.retain(|path| !ignored.contains(path));
        }
        Ok(paths)
    })();
    operation.unwrap_or_default()
}

fn phrase(text: &str) -> String {
    query::folded(text)
        .split(|character: char| character.is_whitespace() || ",.!?;".contains(character))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn continuation(text: &str, persistent: bool) -> bool {
    if text.chars().count() > 128 {
        return false;
    }
    let phrase = phrase(text);
    if [
        "go on",
        "continue",
        "ok",
        "okay",
        "thanks",
        "thank you",
        "ok thanks",
        "okay thanks",
        "ok go on",
        "дальше",
        "продолжай",
        "продолжай дальше",
        "хорошо",
        "спасибо",
    ]
    .contains(&phrase.as_str())
    {
        return true;
    }
    let words = phrase.split_whitespace().collect::<Vec<_>>();
    persistent
        && !words.is_empty()
        && words.len() <= 6
        && words.iter().all(|word| confirmation(word))
}
fn confirmation(word: &str) -> bool {
    "yes yeah yep yup ok okay k sure please thanks thank you go on ahead continue proceed do it that this next keep going carry sounds good great fine perfect lgtm alright right cool nice and then now so let's lets why fix да ок окей ага угу хорошо отлично давай давайте дальше продолжай продолжи продолжайте делай сделай делайте так и спасибо верно согласен согласна поехали вперёд вперед можно ну пожалуйста это ладно годится норм супер почему исправь".split_whitespace().any(|value|value==word)
}
pub fn task_fallback(text: &str) -> bool {
    let phrase = phrase(text);
    if continuation(&phrase, false) {
        return false;
    }
    let words = phrase.split_whitespace().collect::<Vec<_>>();
    let action = |word: &str| {
        [
            "fix",
            "do",
            "implement",
            "update",
            "change",
            "add",
            "remove",
            "refactor",
            "test",
            "run",
            "check",
            "review",
            "apply",
            "finish",
            "исправь",
            "сделай",
            "делай",
            "добавь",
            "обнови",
            "измени",
            "проверь",
            "запусти",
            "реализуй",
            "поправь",
            "заверши",
        ]
        .contains(&word)
    };
    !words.is_empty()
        && words.len() <= 6
        && words.iter().all(|word| confirmation(word) || action(word))
        && words.iter().any(|word| action(word))
}
pub fn host_loaded(context: &RuntimeContext, host: &str) -> BTreeSet<String> {
    let mut loaded = BTreeSet::new();
    if host == "codex" {
        loaded.insert("AGENTS.md".into());
    }
    if host == "claude" {
        let names = ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"];
        loaded.extend(names.iter().map(|name| name.to_string()));
        if !names
            .iter()
            .any(|name| context.roots.project.join(name).is_file())
        {
            loaded.insert("AGENTS.md".into());
        }
        let mut pending = names
            .into_iter()
            .map(|name| (context.roots.project.join(name), 0))
            .collect::<Vec<_>>();
        let pattern = regex::Regex::new(
            r"@((?:\.{1,2}/)?[\p{L}\p{N}_-][\p{L}\p{N}_\u{0300}-\u{036f}./#+-]*)",
        )
        .ok();
        let inline_code = regex::Regex::new(r"`+[^`]*`+").ok();
        while let Some((path, depth)) = pending.pop() {
            if depth >= 5 {
                continue;
            }
            let Ok(text) = crate::fs::read_text(&path) else {
                continue;
            };
            let mut fenced = false;
            for line in text.lines() {
                if ["```", "~~~"]
                    .iter()
                    .any(|prefix| line.trim_start().starts_with(prefix))
                {
                    fenced = !fenced;
                    continue;
                }
                if fenced {
                    continue;
                }
                let plain = inline_code
                    .as_ref()
                    .map(|pattern| pattern.replace_all(line, "").into_owned())
                    .unwrap_or_else(|| line.into());
                if let Some(pattern) = &pattern {
                    for captures in pattern.captures_iter(&plain) {
                        let Some(found) = captures.get(0) else {
                            continue;
                        };
                        if plain[..found.start()]
                            .chars()
                            .next_back()
                            .is_some_and(|character| {
                                character == '@' || character.is_alphanumeric() || character == '_'
                            })
                        {
                            continue;
                        }
                        let relative =
                            captures[1].trim_matches(|character| ".,;:".contains(character));
                        let Ok(target) = crate::fs::canonicalize(
                            path.parent()
                                .unwrap_or(&context.roots.project)
                                .join(relative),
                        ) else {
                            continue;
                        };
                        if !target.is_file() || !target.starts_with(&context.roots.project) {
                            continue;
                        }
                        if let Ok(relative) = target.strip_prefix(&context.roots.project) {
                            let key = relative.to_string_lossy().replace('\\', "/");
                            if loaded.insert(key) {
                                pending.push((target, depth + 1));
                            }
                        }
                    }
                }
            }
        }
    }
    if context.roots.attached() {
        loaded.insert(
            context
                .roots
                .tooling
                .join("AGENTS.md")
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    loaded
}

pub fn transcript_prompt(record: &Value) -> bool {
    prompt(record)
}
pub fn transcript_calls(record: &Value, root: &Path) -> Vec<(String, Value, PathBuf)> {
    calls(record, root)
}
pub fn shell_file_words(command: &str, base: &Path, dialect: &str) -> Vec<(String, bool, PathBuf)> {
    shell_files(command, base, dialect == "powershell")
}
pub fn transcript_project_file(word: &str, base: &Path, root: &Path) -> Option<String> {
    project_file(word, base, root)
}
pub fn recent_file_kept(path: &str) -> bool {
    kept(path)
}

#[derive(Clone, Debug)]
pub struct TopicPlan {
    pub scope: Option<Value>,
    pub expected: Option<String>,
    pub seed: Option<Value>,
}
pub fn topic_scope(store: &Store, context: &RuntimeContext, task_id: &str) -> Option<Value> {
    let operation = (|| -> Result<Value> {
        if task_id.is_empty() || task_id.chars().count() > 200 {
            return Err(RuntimeError::invalid("Invalid topic identity"));
        }
        if [
            "GIT_DIR",
            "GIT_COMMON_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CONFIG",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_COUNT",
        ]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
        {
            return Err(RuntimeError::unavailable("Topic scope unavailable"));
        }
        let branch = crate::cursor_rule::scope_branch(context)?;
        let mut files = PublicationFiles::new(&context.roots.state)?;
        let config_image = files.read("project-brain/config/runtime.json", 65536)?;
        let config = RuntimeConfig::load(&context.roots)?;
        let mut alias = task_id.to_string();
        let mut uuid = Value::Null;
        let mut generation = Value::Null;
        let mut goal = Value::Null;
        let mut kind = "warming";
        if context.mode == "governed" {
            let bindings = store.conn.query_row(
                "SELECT COUNT(*) FROM task_bindings WHERE external_id=?1 OR task_uuid=?1",
                [task_id],
                |row| row.get::<_, usize>(0),
            )?;
            if bindings > 1 {
                return Err(RuntimeError::unavailable("Topic task binding is ambiguous"));
            }
            if let Some(binding) = store.binding(task_id)? {
                let task = BrainRepository::new(context.clone())?.get_task(&binding.task_uuid)?;
                if task["external_id"] != binding.external_id
                    || domain::is_terminal("task", task["status"].as_str().unwrap_or(""))
                    || !["public", "team"].contains(&task["privacy"].as_str().unwrap_or(""))
                    || !config
                        .strings("allowed_privacy")?
                        .iter()
                        .any(|privacy| task["privacy"].as_str() == Some(privacy))
                {
                    return Err(RuntimeError::unavailable("Topic scope unavailable"));
                }
                let pointer = store
                    .working(&binding.external_id)?
                    .ok_or_else(|| RuntimeError::unavailable("Topic pointer unavailable"))?;
                if pointer.goal != binding.task_uuid {
                    return Err(RuntimeError::unavailable("Topic pointer unavailable"));
                }
                alias = binding.external_id;
                uuid = object!(binding.task_uuid);
                generation = object!(pointer.created_at);
                goal = object!(query::sha256(
                    task["goal"].as_str().unwrap_or("").as_bytes()
                ));
                kind = "bound";
            } else {
                if store.working(task_id)?.is_some() {
                    return Err(RuntimeError::unavailable("Topic scope unavailable"));
                }
                let inventory = BrainRepository::new(context.clone())?.inventory(true)?;
                if inventory.records.iter().any(|image| {
                    image.record["id"] == task_id || image.record["external_id"] == task_id
                }) || inventory.problems.iter().any(|problem| {
                    problem.id.as_deref() == Some(task_id)
                        || problem.external_id.as_deref() == Some(task_id)
                }) {
                    return Err(RuntimeError::unavailable("Topic identity is not absent"));
                }
            }
        } else if let Some(task) = store.working(task_id)? {
            generation = object!(task.created_at);
            goal = object!(query::sha256(task.goal.as_bytes()));
            kind = "bound";
        }
        Ok(
            object!({"mode":context.mode,"branch":branch,"task_id":alias,"task_uuid":uuid,"generation":generation,"goal":goal,"kind":kind,"root":topic_root_digest(&context.roots.project)?,"config":config_image.as_ref().map(|image|query::sha256(image)).unwrap_or("absent".into()),"secret_policy":domain::secret_policy_fingerprint()?}),
        )
    })();
    operation.ok()
}
pub fn topic_begin(store: &Store, context: &RuntimeContext, task: &str) -> TopicPlan {
    let scope = topic_scope(store, context, task);
    let raw=store.conn.query_row("SELECT CASE WHEN typeof(value)='text' AND length(CAST(value AS BLOB))<=6144 THEN value ELSE NULL END FROM index_state WHERE key='cursor-topic-v1'",[],|row|row.get::<_,Option<String>>(0)).optional().ok().flatten().flatten();
    let now = clock::proof_now().timestamp_millis() as f64 / 1000.0;
    let seed = raw
        .as_ref()
        .and_then(|raw| json::parse(raw).ok())
        .filter(|value| {
            value.as_object().is_some_and(|object| object.len() == 4)
                && value["profile"] == "cursor-topic-v1"
                && Some(&value["scope"]) == scope.as_ref()
                && scope.is_some()
                && value["query"].as_str().is_some_and(|query| {
                    !query.is_empty()
                        && query.chars().count() <= 1024
                        && domain::sensitive_label(query).ok().flatten().is_none()
                })
                && value["written_at"].as_f64().is_some_and(|at| {
                    at.is_finite() && at >= 0.0 && at <= now && now - at <= 86400.0
                })
        });
    TopicPlan {
        scope,
        expected: raw,
        seed,
    }
}
pub fn topic_write(store: &mut Store, expected: Option<&str>, value: Option<&Value>) -> bool {
    if !store.conn.is_autocommit() || store.read_only {
        return false;
    }
    let timeout = store
        .conn
        .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, u64>(0))
        .unwrap_or(5000);
    let operation = (|| -> Result<bool> {
        store.conn.busy_timeout(Duration::ZERO)?;
        store.transaction(|conn|{if let Some(value)=value{let raw=json::python_default(value,true,false,true)?;if raw.len()>6144{return Ok(false);}let changed=if let Some(expected)=expected{conn.execute("UPDATE index_state SET value=?1 WHERE key='cursor-topic-v1' AND value=?2",params![raw,expected])?}else{conn.execute("DELETE FROM index_state WHERE key='cursor-topic-v1' AND (typeof(value)!='text' OR length(CAST(value AS BLOB))>6144)",[])?;conn.execute("INSERT INTO index_state(key,value) VALUES('cursor-topic-v1',?1) ON CONFLICT(key) DO NOTHING",[raw])?};Ok(changed==1)}else{if let Some(expected)=expected{conn.execute("DELETE FROM index_state WHERE key='cursor-topic-v1' AND value=?1",[expected])?;}Ok(true)}})
    })();
    let _ = store.conn.busy_timeout(Duration::from_millis(timeout));
    operation.unwrap_or(false)
}
pub fn topic_query(
    store: &Store,
    context: &RuntimeContext,
    task_id: &str,
    plan: &TopicPlan,
) -> Option<(String, String)> {
    let scope = plan.scope.as_ref()?;
    let task = if context.mode == "governed" {
        scope["task_uuid"].as_str().and_then(|id| {
            BrainRepository::new(context.clone())
                .and_then(|brain| brain.get_task(id))
                .ok()
        })
    } else {
        store
            .working(task_id)
            .ok()
            .flatten()
            .and_then(|task| serde_json::to_value(task).ok())
    };
    if let Some(task) = task
        && let Some(goal) = task["goal"]
            .as_str()
            .filter(|goal| !goal.trim().is_empty() && *goal != storage::derive_goal(task_id))
        && (context.mode != "governed"
            || ["public", "team"].contains(&task["privacy"].as_str().unwrap_or("")))
    {
        return query::build_capsule_query(goal, None)
            .ok()
            .map(|query| (query, "task".into()));
    }
    plan.seed
        .as_ref()
        .and_then(|seed| seed["query"].as_str())
        .map(|query| (query.into(), "continuation".into()))
}
#[expect(
    clippy::too_many_arguments,
    reason = "Topic continuity commits bind identity, phase, eligibility, warnings and selection from one refresh outcome"
)]
pub fn topic_finish(
    store: &mut Store,
    context: &RuntimeContext,
    task_id: &str,
    plan: &TopicPlan,
    query: &str,
    eligible: bool,
    warnings: &[String],
    continued: bool,
    acknowledgement: bool,
    selected: bool,
) {
    if !eligible
        || !warnings.is_empty()
        || plan.scope.is_none()
        || query.is_empty()
        || query.chars().count() > 1024
        || topic_scope(store, context, task_id) != plan.scope
    {
        topic_write(store, plan.expected.as_deref(), None);
        return;
    }
    if continued
        && plan
            .seed
            .as_ref()
            .is_some_and(|seed| seed["query"] == query)
    {
        return;
    }
    if acknowledgement {
        if plan.seed.is_none() {
            topic_write(store, plan.expected.as_deref(), None);
        }
        return;
    }
    if !selected {
        topic_write(store, plan.expected.as_deref(), None);
        return;
    }
    let value = object!({"profile":"cursor-topic-v1","scope":plan.scope,"query":query,"written_at":clock::proof_now().timestamp_millis()as f64/1000.0});
    topic_write(store, plan.expected.as_deref(), Some(&value));
}
pub fn topic_handoff(
    store: &mut Store,
    context: &RuntimeContext,
    task_id: &str,
    plan: &TopicPlan,
    created_uuid: Option<&str>,
    created_generation: &str,
) {
    let (Some(previous), Some(seed)) = (&plan.scope, &plan.seed) else {
        return;
    };
    if previous["kind"] != "warming" {
        return;
    }
    let Some(current) = topic_scope(store, context, task_id).filter(|current| {
        current["kind"] == "bound"
            && current["task_uuid"].as_str() == created_uuid
            && current["generation"] == created_generation
            && current["goal"] == query::sha256(storage::derive_goal(task_id).as_bytes())
    }) else {
        return;
    };
    if [
        "mode",
        "branch",
        "task_id",
        "root",
        "config",
        "secret_policy",
    ]
    .iter()
    .any(|key| current[*key] != previous[*key])
    {
        return;
    }
    let mut seed = seed.clone();
    seed["scope"] = current;
    topic_write(store, plan.expected.as_deref(), Some(&seed));
}
pub fn ticket_files(
    store: &Store,
    context: &RuntimeContext,
    text: &str,
    task: Option<&str>,
    shown: &[String],
) -> Vec<String> {
    let config = RuntimeConfig::load(&context.roots).ok();
    let enabled = std::env::var("CONTEXT_TICKET_FILES")
        .ok()
        .map(|value| value == "1")
        .unwrap_or_else(|| {
            config
                .as_ref()
                .is_some_and(|config| config.enabled("ticket_files"))
        });
    if !enabled {
        return vec![];
    }
    let known = crate::history::known_prefixes(store, None);
    let tickets =
        domain::unique_strings(crate::history::ticket_ids(text, &known).into_iter().chain(
            crate::history::ticket_ids(&task.unwrap_or("").to_uppercase(), &known),
        ));
    if tickets.is_empty() {
        return vec![];
    }
    let mut arguments = crate::git::strings(&[
        "log",
        "--all",
        "--no-merges",
        "--no-renames",
        "--relative",
        "-z",
        "--regexp-ignore-case",
        "--extended-regexp",
        "--format=%x1e",
        "--name-only",
        "--max-count=200",
    ]);
    for ticket in &tickets {
        arguments.push(format!("--grep=(^|[^A-Za-z0-9]){ticket}([^0-9]|$)").into());
    }
    let Ok(result) = crate::git::run(
        &context.roots.project,
        &arguments,
        None,
        Duration::from_secs(2),
        8 * 1024 * 1024,
    ) else {
        return vec![];
    };
    if result.code != 0 {
        return vec![];
    }
    let mut output = Vec::new();
    let folded = query::folded(text);
    for raw in result.stdout.split(|byte| *byte == 0) {
        let Ok(path) = std::str::from_utf8(raw) else {
            continue;
        };
        let path = path.trim_start_matches('\n');
        if path.is_empty()
            || path == "\u{1e}"
            || shown.iter().any(|shown| shown == path)
            || output.iter().any(|shown| shown == path)
            || folded.contains(&query::folded(path))
            || Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| folded.contains(&query::folded(name)))
        {
            continue;
        }
        if kept(path) && context.roots.project.join(path).is_file() {
            output.push(path.into());
            if output.len() >= 10 {
                break;
            }
        }
    }
    output
}

fn topic_root_digest(project: &Path) -> Result<String> {
    let project = crate::fs::canonicalize(project)?;
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        project.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = project.to_string_lossy().into_owned().into_bytes();
    Ok(query::sha256(&bytes))
}
