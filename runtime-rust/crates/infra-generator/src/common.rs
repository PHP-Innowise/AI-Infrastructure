use infra_core::{Result, RuntimeError, domain::json, fs};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: String,
    pub code: String,
    pub message: String,
}
pub fn diag(items: &mut Vec<Diagnostic>, code: &str, message: impl Into<String>) {
    items.push(Diagnostic {
        severity: "error".into(),
        code: code.into(),
        message: message.into(),
    });
}
pub fn warn(items: &mut Vec<Diagnostic>, code: &str, message: impl Into<String>) {
    items.push(Diagnostic {
        severity: "warning".into(),
        code: code.into(),
        message: message.into(),
    });
}
pub fn sorted(items: Vec<Diagnostic>) -> Vec<Diagnostic> {
    items
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
pub fn report(items: Vec<Diagnostic>) -> Value {
    let items = sorted(items);
    object!({"valid":!items.iter().any(|item|item.severity=="error"),"diagnostics":items})
}
pub fn nonempty(value: &Value) -> bool {
    value.as_str().is_some_and(|value| !value.trim().is_empty())
}
pub fn texts(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
pub fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
pub fn string_list(value: &Value, empty: bool) -> bool {
    value
        .as_array()
        .is_some_and(|items| (empty || !items.is_empty()) && items.iter().all(nonempty))
}
pub fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}
pub fn read_json(path: &Path) -> Result<Value> {
    json::parse(&fs::read_text(path)?)
}
pub fn read_object(path: &Path) -> Result<Value> {
    let value = read_json(path)?;
    if !value.is_object() {
        return Err(RuntimeError::invalid(format!(
            "{} must contain a JSON object",
            path.display()
        )));
    }
    Ok(value)
}
pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::atomic_json(path, value)
}
pub fn sha(bytes: &[u8]) -> String {
    infra_core::query::sha256(bytes)
}
pub fn hex(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
}
pub fn expression(pattern: &str) -> Result<regex::Regex> {
    regex::Regex::new(pattern)
        .map_err(|error| RuntimeError::invalid(format!("Invalid generator expression: {error}")))
}
pub fn matching(pattern: &str, text: &str) -> bool {
    regex::Regex::new(pattern)
        .map(|regex| regex.is_match(text))
        .unwrap_or(false)
}
pub fn path_text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| RuntimeError::invalid("Generator path must be UTF-8"))
}
pub fn walk_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    if !root.is_dir() {
        return Ok(result);
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut entries = std::fs::read_dir(&directory)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if fs::linked(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                result.push(entry.path());
            }
        }
    }
    result.sort();
    Ok(result)
}
pub fn relative(root: &Path, path: &Path) -> Result<String> {
    path.strip_prefix(root)
        .map_err(|_| RuntimeError::policy("Generator path escapes root"))?
        .to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| RuntimeError::invalid("Generator path must be UTF-8"))
}
pub fn flatten(value: &Value) -> Vec<String> {
    match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(items) => items.iter().flat_map(flatten).collect(),
        Value::Object(items) => items.values().flat_map(flatten).collect(),
        _ => vec![],
    }
}
pub fn word_tokens(value: &str) -> BTreeSet<String> {
    regex::Regex::new(r"[\p{L}\p{N}]+")
        .map(|regex| {
            regex
                .find_iter(&value.to_lowercase())
                .map(|hit| hit.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default()
}
pub fn slug(value: &str) -> String {
    let value = value.to_lowercase().replace(' ', "-");
    value
        .chars()
        .filter(|character| character.is_alphanumeric() || matches!(character, '_' | '-'))
        .collect()
}
pub fn load_diagnostic(path: &Path, label: &str, items: &mut Vec<Diagnostic>) -> Option<Value> {
    match read_object(path) {
        Ok(value) => Some(value),
        Err(error) => {
            diag(
                items,
                &format!("{label}_UNREADABLE"),
                format!("{}: {error}", path.display()),
            );
            None
        }
    }
}
pub fn strings_map(value: &Value) -> Result<BTreeMap<String, String>> {
    value
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Expected JSON object"))?
        .iter()
        .map(|(key, value)| {
            Ok((
                key.clone(),
                value
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("Expected string map value"))?
                    .to_owned(),
            ))
        })
        .collect()
}
pub fn safe_source(target: &Path, citation: &str) -> Result<PathBuf> {
    let relative = citation.split('#').next().unwrap_or(citation);
    crate::ownership::confined(target, relative)
}

fn glob_class(body: &[char]) -> String {
    // Match the class/range rules used by Python fnmatch, not regex class
    // negation or Rust's nested set operators.
    let mut chunks: Vec<Vec<char>> = Vec::new();
    if body.contains(&'-') {
        let mut start = 0;
        let mut search = if body.first() == Some(&'!') { 2 } else { 1 };
        while search < body.len() {
            let Some(offset) = body[search..].iter().position(|ch| *ch == '-') else {
                break;
            };
            let dash = search + offset;
            chunks.push(body[start..dash].to_vec());
            start = dash + 1;
            search = dash + 3;
        }
        if start < body.len() {
            chunks.push(body[start..].to_vec());
        } else if let Some(last) = chunks.last_mut() {
            last.push('-');
        }
        for index in (1..chunks.len()).rev() {
            if chunks[index - 1].last() > chunks[index].first() {
                let tail = chunks[index][1..].to_vec();
                chunks[index - 1].pop();
                chunks[index - 1].extend(tail);
                chunks.remove(index);
            }
        }
    } else {
        chunks.push(body.to_vec());
    }
    let empty = chunks.iter().all(Vec::is_empty);
    if empty {
        return r"[^\s\S]".into();
    }
    if chunks.len() == 1 && chunks[0] == ['!'] {
        return ".".into();
    }
    let mut output = String::from("[");
    for (index, chunk) in chunks.iter().enumerate() {
        if index > 0 {
            output.push('-');
        }
        for (position, ch) in chunk.iter().enumerate() {
            if index == 0 && position == 0 && *ch == '!' {
                output.push('^');
            } else {
                if matches!(*ch, '\\' | '[' | ']' | '^' | '&' | '~' | '|' | '-') {
                    output.push('\\');
                }
                output.push(*ch);
            }
        }
    }
    output.push(']');
    output
}
/// Apply the native Windows pathname normalization used by Python fnmatch.
/// Normalize before translating classes: e.g. [Z-a] becomes the empty [z-a].
#[cfg(windows)]
fn try_platform_normalize(value: &str) -> Result<String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LCMapStringEx(
            locale: *const u16,
            flags: u32,
            source: *const u16,
            source_length: i32,
            destination: *mut u16,
            destination_length: i32,
            version: *const std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
            sort_handle: isize,
        ) -> i32;
    }
    if value.is_empty() {
        return Ok(String::new());
    }
    let source = value.replace('/', "\\").encode_utf16().collect::<Vec<_>>();
    let source_length = i32::try_from(source.len())
        .map_err(|_| RuntimeError::invalid("Windows fnmatch input exceeds native length"))?;
    // LOCALE_NAME_INVARIANT is L""; NULL would select the user locale instead.
    let invariant = [0u16];
    const LOWERCASE: u32 = 0x0000_0100;
    let failure = || {
        let mut error = RuntimeError::unavailable("Windows fnmatch normalization failed");
        error.os_code = std::io::Error::last_os_error().raw_os_error();
        error
    };
    // Explicit lengths include any embedded NUL; no implicit terminator is read.
    let required = unsafe {
        LCMapStringEx(
            invariant.as_ptr(),
            LOWERCASE,
            source.as_ptr(),
            source_length,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        )
    };
    if required <= 0 {
        return Err(failure());
    }
    let mut mapped = vec![0u16; required as usize];
    let written = unsafe {
        LCMapStringEx(
            invariant.as_ptr(),
            LOWERCASE,
            source.as_ptr(),
            source_length,
            mapped.as_mut_ptr(),
            required,
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        )
    };
    if written != required {
        return Err(failure());
    }
    String::from_utf16(&mapped).map_err(|_| {
        RuntimeError::unavailable("Windows fnmatch normalization returned invalid Unicode")
    })
}

/// Python fnmatch.fnmatch: pathname case/separators follow the host platform.
pub fn glob_match(pattern: &str, value: &str) -> Result<bool> {
    #[cfg(windows)]
    {
        Ok(glob_match_case(
            &try_platform_normalize(pattern)?,
            &try_platform_normalize(value)?,
        ))
    }
    #[cfg(not(windows))]
    {
        Ok(glob_match_case(pattern, value))
    }
}

/// Python fnmatch.fnmatchcase: never normalize pathname case or separators.
pub fn glob_match_case(pattern: &str, value: &str) -> bool {
    glob_match_path(pattern, value, false).unwrap_or(false)
}

fn glob_case_expression(pattern: &str) -> String {
    let mut output = String::from(r"\A(?s:");
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        at += 1;
        match ch {
            '*' => {
                output.push_str(".*");
                while chars.get(at) == Some(&'*') {
                    at += 1;
                }
            }
            '?' => output.push('.'),
            '[' => {
                let mut end = at;
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
                    output.push_str(r"\[");
                } else {
                    output.push_str(&glob_class(&chars[at..end]));
                    at = end + 1;
                }
            }
            value => output.push_str(&regex::escape(&value.to_string())),
        }
    }
    output.push_str(r")\z");
    output
}

// Python re.IGNORECASE treats I/i/İ/ı as one character-equivalence set.
// Rust's Unicode simple fold has only I/i, so close the positive set explicitly
// before compiling a possibly negated class.
fn path_glob_class(body: &[char]) -> Result<String> {
    let class = glob_class(body);
    if class == "." || class == r"[^\s\S]" {
        return Ok(class);
    }
    let positive_body = if body.first() == Some(&'!') {
        &body[1..]
    } else {
        body
    };
    let positive = glob_class(positive_body);
    let probe = regex::Regex::new(&format!(r"\A(?:{positive})\z"))
        .map_err(|_| RuntimeError::invalid("Path glob class could not be compiled"))?;
    if ["I", "i", "İ", "ı"]
        .iter()
        .any(|value| probe.is_match(value))
    {
        let prefix = class
            .strip_suffix(']')
            .ok_or_else(|| RuntimeError::invalid("Path glob class could not be compiled"))?;
        Ok(format!("{prefix}Iiİı]"))
    } else {
        Ok(class)
    }
}

/// Path.glob component semantics, separate from pathname-normalizing fnmatch.
/// The selected case-insensitive contract is Python 3.14 re.IGNORECASE using
/// Unicode 16 simple character equivalences, never multi-character full folds.
/// Explicit mode permits the same Windows matching oracle on every test host.
pub(crate) fn glob_match_path(pattern: &str, value: &str, ignore_case: bool) -> Result<bool> {
    Ok(
        compile_path_component(pattern, ignore_case)?
            .is_some_and(|matcher| matcher.is_match(value)),
    )
}

/// Compile one reached component. Case-sensitive fnmatch treats a rejected
/// expression as a non-match; the case-insensitive Path.glob route reports it.
pub(crate) fn compile_path_component(
    pattern: &str,
    ignore_case: bool,
) -> Result<Option<regex::Regex>> {
    if !ignore_case {
        return Ok(regex::Regex::new(&glob_case_expression(pattern)).ok());
    }
    let mut output = String::from(r"\A(?s:");
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        at += 1;
        match ch {
            '*' => {
                output.push_str(".*");
                while chars.get(at) == Some(&'*') {
                    at += 1;
                }
            }
            '?' => output.push('.'),
            '[' => {
                let mut end = at;
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
                    output.push_str(r"\[");
                } else {
                    output.push_str(&path_glob_class(&chars[at..end])?);
                    at = end + 1;
                }
            }
            'I' | 'i' | 'İ' | 'ı' => output.push_str("[Iiİı]"),
            value => output.push_str(&regex::escape(&value.to_string())),
        }
    }
    output.push_str(r")\z");
    let matcher = regex::RegexBuilder::new(&output)
        .case_insensitive(true)
        .build()
        .map_err(|_| RuntimeError::invalid("Path glob expression could not be compiled"))?;
    Ok(Some(matcher))
}

pub fn trailing<'a>(value: &'a str, characters: &[char]) -> &'a str {
    let end = value
        .char_indices()
        .rev()
        .find(|(_, character)| !characters.contains(character))
        .map(|(at, character)| at + character.len_utf8())
        .unwrap_or(0);
    &value[..end]
}
